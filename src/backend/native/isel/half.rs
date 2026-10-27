//! `f16` in SSE registers: the low 16 bits of an XMM register hold the
//! value, as the System V psABI passes `_Float16` (§3.2.3, "Classification":
//! class SSE), and the rest are unspecified.
//!
//! An `f16` only moves and converts: lowering computes `_Float16`
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
    /// A conversion to or from `f16`, or a `freeze` of one.
    pub(super) fn half_conversion(
        &mut self,
        inst: Inst,
        opcode: Opcode,
        ty: Type,
        arg: Value,
    ) -> Result<(), Construct> {
        let from = self.ty(arg);
        if matches!(ty, Type::F80 | Type::F128 | Type::I128) {
            let name = match (opcode, ty) {
                | (Opcode::Fpext, Type::F80) => "__extendhfxf2",
                | (Opcode::Fpext, Type::F128) => "__extendhftf2",
                | (Opcode::Fptosi, Type::I128) => "__fixhfti",
                | (Opcode::Fptoui, Type::I128) => "__fixunshfti",
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
            | (Opcode::Fptrunc, Type::F32 | Type::F64, Type::F16) => {
                let name = if from == Type::F32 {
                    "__truncsfhf2"
                } else {
                    "__truncdfhf2"
                };
                let src = self.reg(arg);
                self.libcall(name, &[LibArg::Xmm(src)], LibResult::Xmm(dst));
            },
            | (Opcode::Fptrunc | Opcode::Sitofp | Opcode::Uitofp, _, Type::F16)
                if matches!(from, Type::F80 | Type::F128 | Type::I128) =>
            {
                let name = match (opcode, from) {
                    | (Opcode::Fptrunc, Type::F80) => "__truncxfhf2",
                    | (Opcode::Fptrunc, _) => "__trunctfhf2",
                    | (Opcode::Sitofp, _) => "__floattihf",
                    | _ => "__floatuntihf",
                };
                let halves = self.pair(arg);
                self.libcall(name, &[LibArg::Wide(from, halves)], LibResult::Xmm(dst));
            },
            | _ => return Err(Construct::Opcode(opcode)),
        }
        Ok(())
    }

    /// `dst ← [mem]` for an `f16`: `movzwl` into a general register, then
    /// `movd`.
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

    /// `[mem] ← src` for an `f16` in an SSE register: `movd` into a general
    /// register, then `movw`.
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
