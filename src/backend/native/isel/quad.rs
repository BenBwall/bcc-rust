//! `f128` through runtime routines, and every conversion with an `f80` or
//! `f128` on either side.
//!
//! An `f128` lives in a pair of general registers like an `i128`, so copies,
//! spills, selects, loads, stores, bit casts to and from `i128`, and `fneg`
//! (which flips bit 127) need nothing else. Arithmetic, comparisons and
//! conversions call the soft-float routines of libgcc and compiler-rt
//! (`__addtf3`, `__lttf2`, `__extenddftf2`, `__fixtfti` …) that LLVM calls
//! for them, with the arguments and results each convention gives those
//! types ([`Selector::libcall`]). The comparison routines return an `int`
//! whose sign answers the ordered question; an unordered pair makes
//! `__lttf2` and `__letf2` positive and `__gttf2` and `__getf2` negative, so
//! the unordered conditions are the negated ordered ones of the opposite
//! routine, and `ueq` and `one` combine `__eqtf2` with `__unordtf2`.
//! Reference: GCC Internals, "Routines for floating point emulation"
//! (`__unordtf2`, `__eqtf2` … and their return values).
//!
//! `f80` conversions other than with `i128` and `f128` stay on the x87 unit
//! (`isel/x87.rs`).

use super::{
    Selector,
    calls::{
        LibArg,
        LibResult,
    },
    wide::Halves,
};
use crate::{
    backend::native::{
        mir::{
            AluOp,
            Cond,
            MInst,
            Operand,
            Reg,
            UnaryOp,
            Width,
        },
        support::Construct,
    },
    ir::{
        FloatCC,
        Inst,
        Opcode,
        Type,
        Value,
    },
};

impl Selector<'_, '_, '_> {
    /// A unary operation that involves an `f80` or `f128`.
    pub(super) fn wide_float_unary(
        &mut self,
        inst: Inst,
        opcode: Opcode,
        ty: Type,
        arg: Value,
    ) -> Result<(), Construct> {
        let from = self.ty(arg);
        if matches!(ty, Type::I128 | Type::F80 | Type::F128) {
            let dst = self.result_pair(inst);
            return self.convert_to_pair(opcode, ty, dst, arg);
        }
        let dst = self.result_reg(self.body.inst_result(inst));
        match from {
            | Type::F80 => self.convert_from_x87(opcode, ty, dst, arg),
            | _ => {
                let (name, result) = Self::narrowing_routine(opcode, ty, from, dst)?;
                let halves = self.pair(arg);
                self.libcall(name, &[LibArg::Wide(from, halves)], result);
                if ty == Type::I1 {
                    self.canonical_bool(dst);
                }
            },
        }
        Ok(())
    }

    /// The routine, and where its result goes, for a conversion from an
    /// `f128` to a narrower type.
    fn narrowing_routine(
        opcode: Opcode,
        ty: Type,
        from: Type,
        dst: Reg,
    ) -> Result<(&'static str, LibResult), Construct> {
        let float = |name| Ok((name, LibResult::Xmm(dst)));
        let int = |name| Ok((name, LibResult::Gpr(dst)));
        match (opcode, from, ty) {
            | (Opcode::Fptrunc, Type::F128, Type::F32) => float("__trunctfsf2"),
            | (Opcode::Fptrunc, Type::F128, _) => float("__trunctfdf2"),
            // Narrower results come from the 32-bit signed routine, whose
            // range holds every in-range value, as LLVM promotes them.
            | (Opcode::Fptoui, Type::F128, Type::I32) => int("__fixunstfsi"),
            | (Opcode::Fptoui, Type::F128, Type::I64) => int("__fixunstfdi"),
            | (Opcode::Fptosi, Type::F128, Type::I64) => int("__fixtfdi"),
            | (Opcode::Fptosi | Opcode::Fptoui, Type::F128, _) => int("__fixtfsi"),
            | _ => Err(Construct::Opcode(opcode)),
        }
    }

    /// A unary operation whose result is an `i128`, `f80` or `f128`.
    fn convert_to_pair(
        &mut self,
        opcode: Opcode,
        ty: Type,
        dst: Halves,
        arg: Value,
    ) -> Result<(), Construct> {
        let from = self.ty(arg);
        let name = match (opcode, from, ty) {
            | (Opcode::Fneg, _, Type::F80) => {
                self.x87_negate(dst, arg);
                return Ok(());
            },
            | (Opcode::Fneg, _, _) => {
                self.quad_negate(dst, arg);
                return Ok(());
            },
            // Bit casts and freezes the selector could not alias.
            | (Opcode::Bitcast | Opcode::Freeze, _, _) => {
                let src = self.pair(arg);
                self.copy_pair(dst, src);
                return Ok(());
            },
            | (Opcode::Fpext, Type::F32 | Type::F64, Type::F80)
            | (
                Opcode::Sitofp | Opcode::Uitofp,
                Type::I1 | Type::I8 | Type::I16 | Type::I32 | Type::I64,
                Type::F80,
            ) => {
                self.convert_to_x87(opcode, dst, arg);
                return Ok(());
            },
            | (Opcode::Fptosi, Type::F80, _) => "__fixxfti",
            | (Opcode::Fptosi, Type::F128, _) => "__fixtfti",
            | (Opcode::Fptoui, Type::F80, _) => "__fixunsxfti",
            | (Opcode::Fptoui, Type::F128, _) => "__fixunstfti",
            | (Opcode::Sitofp, Type::I128, Type::F80) => "__floattixf",
            | (Opcode::Uitofp, Type::I128, Type::F80) => "__floatuntixf",
            | (Opcode::Sitofp, Type::I128, _) => "__floattitf",
            | (Opcode::Uitofp, Type::I128, _) => "__floatuntitf",
            | (Opcode::Fpext, Type::F32, _) => "__extendsftf2",
            | (Opcode::Fpext, Type::F64, _) => "__extenddftf2",
            | (Opcode::Fpext, Type::F80, _) => "__extendxftf2",
            | (Opcode::Fptrunc, Type::F128, _) => "__trunctfxf2",
            | (Opcode::Sitofp | Opcode::Uitofp, Type::I64, _) =>
                if opcode == Opcode::Sitofp {
                    "__floatditf"
                } else {
                    "__floatunditf"
                },
            | (Opcode::Uitofp, Type::I32, _) => "__floatunsitf",
            // Narrower integers widen to `int`, which holds them all.
            | (Opcode::Sitofp | Opcode::Uitofp, _, _) => {
                let src = self.int_argument(opcode == Opcode::Sitofp, arg);
                self.libcall("__floatsitf", &[LibArg::Gpr(src)], LibResult::Wide(ty, dst));
                return Ok(());
            },
            | _ => return Err(Construct::Opcode(opcode)),
        };
        let arg = match from {
            | Type::F32 | Type::F64 => LibArg::Xmm(self.reg(arg)),
            | Type::I64 | Type::I32 => LibArg::Gpr(self.reg(arg)),
            | _ => LibArg::Wide(from, self.pair(arg)),
        };
        self.libcall(name, &[arg], LibResult::Wide(ty, dst));
        Ok(())
    }

    /// An integer of at most 32 bits as the `int` a routine reads: extended
    /// by `signed`, a canonical `i1` true as -1 when signed.
    fn int_argument(&mut self, signed: bool, arg: Value) -> Reg {
        if signed && self.ty(arg) == Type::I1 {
            let src = self.reg(arg);
            let negated = self.vreg();
            self.copy(negated, src);
            self.emit(MInst::Unary {
                op:    UnaryOp::Neg,
                width: Width::B32,
                dst:   negated,
            });
            return negated;
        }
        self.extended(arg, signed)
    }

    /// `fadd`, `fsub`, `fmul`, `fdiv` and `frem` of `f128` values.
    pub(super) fn quad_binary(
        &mut self,
        opcode: Opcode,
        dst: Halves,
        [lhs, rhs]: [Value; 2],
    ) -> Result<(), Construct> {
        let name = match opcode {
            | Opcode::Fadd => "__addtf3",
            | Opcode::Fsub => "__subtf3",
            | Opcode::Fmul => "__multf3",
            | Opcode::Fdiv => "__divtf3",
            | Opcode::Frem => "fmodf128",
            | _ => return Err(Construct::Opcode(opcode)),
        };
        let args = [lhs, rhs].map(|value| LibArg::Wide(Type::F128, self.pair(value)));
        self.libcall(name, &args, LibResult::Wide(Type::F128, dst));
        Ok(())
    }

    /// `fneg` of an `f128`: the sign bit flipped.
    fn quad_negate(&mut self, dst: Halves, arg: Value) {
        let src = self.pair(arg);
        self.copy_pair(dst, src);
        let sign = self.vreg();
        self.emit(MInst::MovImm64 {
            dst: sign,
            imm: i64::MIN,
        });
        self.emit(MInst::Alu {
            op:    AluOp::Xor,
            width: Width::B64,
            dst:   dst.1,
            src:   Operand::Reg(sign),
        });
    }

    /// `fcmp` of `f128` values through the comparison routines.
    pub(super) fn quad_compare(&mut self, cond: FloatCC, dst: Reg, [a, b]: [Value; 2]) {
        let (first, second) = match cond {
            | FloatCC::Oeq => (("__eqtf2", Cond::E), None),
            | FloatCC::Une => (("__netf2", Cond::Ne), None),
            | FloatCC::Olt => (("__lttf2", Cond::L), None),
            | FloatCC::Ole => (("__letf2", Cond::Le), None),
            | FloatCC::Ogt => (("__gttf2", Cond::G), None),
            | FloatCC::Oge => (("__getf2", Cond::Ge), None),
            | FloatCC::Ult => (("__getf2", Cond::L), None),
            | FloatCC::Ule => (("__gttf2", Cond::Le), None),
            | FloatCC::Ugt => (("__letf2", Cond::G), None),
            | FloatCC::Uge => (("__lttf2", Cond::Ge), None),
            | FloatCC::Ord => (("__unordtf2", Cond::E), None),
            | FloatCC::Uno => (("__unordtf2", Cond::Ne), None),
            | FloatCC::Ueq => (("__eqtf2", Cond::E), Some((AluOp::Or, Cond::Ne))),
            | FloatCC::One => (("__eqtf2", Cond::Ne), Some((AluOp::And, Cond::E))),
        };
        let byte = self.quad_test(first.0, first.1, a, b);
        if let Some((op, cond)) = second {
            let other = self.quad_test("__unordtf2", cond, a, b);
            self.emit(MInst::Alu {
                op,
                width: Width::B8,
                dst: byte,
                src: Operand::Reg(other),
            });
        }
        self.emit(MInst::Extend {
            signed: false,
            from: Width::B8,
            to: Width::B32,
            dst,
            src: Operand::Reg(byte),
        });
    }

    /// Calls the comparison routine `name` and sets a byte to whether its
    /// `int` result compares to zero as `cond` says.
    fn quad_test(&mut self, name: &'static str, cond: Cond, a: Value, b: Value) -> Reg {
        let args = [a, b].map(|value| LibArg::Wide(Type::F128, self.pair(value)));
        let result = self.vreg();
        self.libcall(name, &args, LibResult::Gpr(result));
        self.emit(MInst::Cmp {
            width: Width::B32,
            lhs:   result,
            rhs:   Operand::Imm(0),
        });
        let byte = self.vreg();
        self.emit(MInst::Setcc { cond, dst: byte });
        byte
    }
}
