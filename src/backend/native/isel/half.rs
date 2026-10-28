//! `f16` and `bf16` in SSE registers: the low 16 bits of an XMM register
//! hold the value, as the System V psABI passes `_Float16` and `__bf16`
//! (§3.2.3, "Classification": class SSE), and the rest are unspecified.
//!
//! These only move and convert: lowering computes `_Float16` and `__bf16`
//! arithmetic and comparisons in `f32`. Without F16C, SSE2 has no 16-bit
//! move between memory and an XMM register, so a load goes through a general
//! register (`movzwl`, then `movd`) and a store back (`movd`, then `movw`),
//! and a constant comes from the read-only constants as a 4-byte `movss`.
//! Conversions call libgcc's routines as GCC does for x86-64 without
//! `-mf16c`: `__extendhfsf2` widens to `f32` (and `__extendhfdf2`,
//! `__extendhfxf2` and `__extendhftf2` to the wider formats), and
//! `__truncsfhf2`, `__truncdfhf2`, `__truncxfhf2` and `__trunctfhf2` round
//! each format once, so a `double` is not rounded through `float` first.
//! `__floattihf`, `__floatuntihf`, `__fixhfti` and `__fixunshfti` convert
//! `i128`; lowering converts narrower integers through `f32`, which is exact.
//! A `bf16` narrows through `__truncsfbf2`, `__truncdfbf2`, `__truncxfbf2`,
//! `__trunctfbf2`, `__floattibf` and `__floatuntibf`, and widens to `f32` by
//! a shift; lowering takes every other conversion of one through `f32`.
//! Reference: GCC Internals, "Routines for floating point emulation".

use super::{
    Selector,
    calls::{
        LibArg,
        LibResult,
    },
};
use crate::{
    backend::native::{
        mir::{
            MInst,
            Mem,
            Operand,
            Reg,
            ShiftAmount,
            ShiftOp,
            Width,
        },
        support::Construct,
    },
    ir::{
        Inst,
        Opcode,
        Type,
        Value,
    },
};

impl Selector<'_, '_, '_> {
    /// A conversion to or from `f16` or `bf16`, or a `freeze` of one.
    pub(super) fn half_conversion(
        &mut self,
        inst: Inst,
        opcode: Opcode,
        ty: Type,
        arg: Value,
    ) -> Result<(), Construct> {
        let from = self.ty(arg);
        if matches!(ty, Type::F80 | Type::F128 | Type::I128) {
            let name = match (opcode, from, ty) {
                | (Opcode::Fpext, Type::F16, Type::F80) => "__extendhfxf2",
                | (Opcode::Fpext, Type::F16, Type::F128) => "__extendhftf2",
                | (Opcode::Fptosi, Type::F16, Type::I128) => "__fixhfti",
                | (Opcode::Fptoui, Type::F16, Type::I128) => "__fixunshfti",
                | _ => return Err(Construct::Opcode(opcode)),
            };
            let src = self.reg(arg);
            let dst = self.result_pair(inst);
            self.libcall(name, &[LibArg::Xmm(src)], LibResult::Wide(ty, dst));
            return Ok(());
        }
        let dst = self.result_reg(self.body.inst_result(inst));
        match (opcode, from, ty) {
            | (Opcode::Freeze, ..) => {
                let src = self.reg(arg);
                self.copy(dst, src);
            },
            | (Opcode::Bitcast, ..) => {
                let src = self.reg(arg);
                self.emit(MInst::MovGx {
                    width: Width::B32,
                    dst,
                    src,
                });
            },
            | (Opcode::Fpext, Type::F16, Type::F32 | Type::F64) => {
                let name = if ty == Type::F32 {
                    "__extendhfsf2"
                } else {
                    "__extendhfdf2"
                };
                let src = self.reg(arg);
                self.libcall(name, &[LibArg::Xmm(src)], LibResult::Xmm(dst));
            },
            | (Opcode::Fpext, Type::BF16, Type::F32) => {
                let src = self.reg(arg);
                self.bfloat_widen(dst, src);
            },
            | (Opcode::Fptrunc, Type::F32 | Type::F64, _) => {
                let name = match (from, ty) {
                    | (Type::F32, Type::F16) => "__truncsfhf2",
                    | (_, Type::F16) => "__truncdfhf2",
                    | (Type::F32, _) => "__truncsfbf2",
                    | _ => "__truncdfbf2",
                };
                let src = self.reg(arg);
                self.libcall(name, &[LibArg::Xmm(src)], LibResult::Xmm(dst));
            },
            | (Opcode::Fptrunc | Opcode::Sitofp | Opcode::Uitofp, ..)
                if matches!(from, Type::F80 | Type::F128 | Type::I128) =>
            {
                let bfloat = ty == Type::BF16;
                let name = match (opcode, from) {
                    | (Opcode::Fptrunc, Type::F80) if bfloat => "__truncxfbf2",
                    | (Opcode::Fptrunc, Type::F80) => "__truncxfhf2",
                    | (Opcode::Fptrunc, _) if bfloat => "__trunctfbf2",
                    | (Opcode::Fptrunc, _) => "__trunctfhf2",
                    | (Opcode::Sitofp, _) if bfloat => "__floattibf",
                    | (Opcode::Sitofp, _) => "__floattihf",
                    | _ if bfloat => "__floatuntibf",
                    | _ => "__floatuntihf",
                };
                let halves = self.pair(arg);
                self.libcall(name, &[LibArg::Wide(from, halves)], LibResult::Xmm(dst));
            },
            | _ => return Err(Construct::Opcode(opcode)),
        }
        Ok(())
    }

    /// `dst ← src` widened from `bf16` to `f32`: the encoding moves to the
    /// top half of a binary32 one (`movd`, `shl $16`, `movd`), as GCC and
    /// Clang widen it inline.
    fn bfloat_widen(&mut self, dst: Reg, src: Reg) {
        let bits = self.vreg();
        self.emit(MInst::MovGx {
            width: Width::B32,
            dst: bits,
            src,
        });
        self.emit(MInst::Shift {
            op:     ShiftOp::Shl,
            width:  Width::B32,
            dst:    bits,
            amount: ShiftAmount::Imm(16),
        });
        self.emit(MInst::MovGx {
            width: Width::B32,
            dst,
            src: bits,
        });
    }

    /// `dst ← [mem]` for an `f16` or `bf16`: `movzwl` into a general register,
    /// then `movd`.
    pub(super) fn half_load(&mut self, dst: Reg, mem: Mem) {
        let bits = self.vreg();
        self.emit(MInst::Extend {
            signed: false,
            from:   Width::B16,
            to:     Width::B32,
            dst:    bits,
            src:    Operand::Mem(mem),
        });
        self.emit(MInst::MovGx {
            width: Width::B32,
            dst,
            src: bits,
        });
    }

    /// `[mem] ← src` for an `f16` or `bf16` in an SSE register: `movd` into a
    /// general register, then `movw`.
    pub(super) fn half_store(&mut self, src: Reg, mem: Mem) {
        let bits = self.vreg();
        self.emit(MInst::MovGx {
            width: Width::B32,
            dst: bits,
            src,
        });
        self.emit(MInst::Store {
            width: Width::B16,
            dst:   mem,
            src:   Operand::Reg(bits),
        });
    }
}
