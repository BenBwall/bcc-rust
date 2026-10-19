//! `f32` and `f64` in SSE registers: arithmetic, comparisons, selects,
//! conversions and constants.
//!
//! Arithmetic uses the scalar SSE forms (`addss`/`addsd` …) in two-address
//! form; `frem` calls `fmodf`/`fmod`, whose results C's `fmod` defines as the
//! IR's `frem` does; `fneg` flips the sign bit with `xorps` against a
//! constant. Constants other than positive zero (`xorps`) are loaded from
//! the function's read-only constants.
//!
//! `ucomiss`/`ucomisd` set ZF, PF and CF, and an unordered pair (a NaN
//! operand) sets all three (Intel SDM Vol. 2, UCOMISD). Each IR condition
//! therefore maps to a flag test that is false (ordered conditions) or true
//! (unordered ones) when all three are set: `ogt` is `a` (CF=0 and ZF=0),
//! `oge` is `ae`, `ult` is `b`, `ule` is `be`, `one` is `ne`, `ueq` is `e`,
//! `ord` is `np` and `uno` is `p`, with the operands swapped for `olt`,
//! `ole`, `ugt` and `uge`. Only `oeq` (ZF=1 and PF=0) and `une` (ZF=0 or
//! PF=1) need two flags, combined with a byte `and` or `or`.
//!
//! Conversions use `cvtsi2s*` and `cvtts*2si`, which handle signed 32- and
//! 64-bit integers. Narrower integers are extended to 32 bits, unsigned
//! 32-bit ones to 64. An unsigned 64-bit integer with its top bit set is
//! halved (keeping the low bit, so rounding is unchanged), converted and
//! doubled; a float at or above 2^63 converts to unsigned after subtracting
//! 2^63 and setting the top bit again. Both pick with `cmov`, without
//! branches: the integer one picks its result, the float one the 2^63 to
//! subtract, before its only conversion, so that an in-range value raises
//! no "invalid" exception (C99 §F.4p1); Clang's code converts twice and
//! raises it for every value at or above 2^63. Out-of-range conversions are
//! poison in the IR, so the hardware's "integer indefinite" result refines
//! them.

use super::{
    Selector,
    ValueKind,
};
use crate::{
    backend::native::{
        mir::{
            AluOp,
            Base,
            Callee,
            Cond,
            MInst,
            Mem,
            Operand,
            PReg,
            Reg,
            RegClass,
            ShiftAmount,
            ShiftOp,
            SymbolRef,
            Width,
            XOp,
        },
        support::Construct,
    },
    ir::{
        FloatCC,
        Opcode,
        Type,
        Value,
    },
};

impl Selector<'_, '_, '_> {
    /// `dst ← bits` for an `f32` or `f64` constant.
    pub(super) fn float_constant(&mut self, dst: Reg, bits: u64, ty: Type) {
        if bits == 0 {
            self.emit(MInst::XZero { dst });
            return;
        }
        let index = self.func.constant(u128::from(bits));
        self.emit(MInst::XMov {
            width: float_width(ty),
            dst,
            src: Operand::Mem(Mem::new(Base::Constant(index), 0)),
        });
    }

    /// A source operand of a float operation: a register, or a constant
    /// read from memory.
    fn float_operand(&mut self, value: Value) -> Operand {
        match self.kind(value) {
            | ValueKind::Const(bits) if bits != 0 => {
                let index = self.func.constant(u128::from(bits));
                Operand::Mem(Mem::new(Base::Constant(index), 0))
            },
            | _ => Operand::Reg(self.reg(value)),
        }
    }

    /// `fadd`, `fsub`, `fmul`, `fdiv` and `frem`.
    pub(super) fn float_binary(
        &mut self,
        opcode: Opcode,
        ty: Type,
        dst: Reg,
        [lhs, rhs]: [Value; 2],
    ) -> Result<(), Construct> {
        let op = match opcode {
            | Opcode::Fadd => XOp::Add,
            | Opcode::Fsub => XOp::Sub,
            | Opcode::Fmul => XOp::Mul,
            | Opcode::Fdiv => XOp::Div,
            | Opcode::Frem => {
                let name = if ty == Type::F32 { "fmodf" } else { "fmod" };
                let args = [lhs, rhs]
                    .map(|value| super::calls::Passed::Scalar(Operand::Reg(self.reg(value))));
                self.call_sequence(Callee::Symbol(SymbolRef::Runtime(name)), &args, false);
                self.copy(dst, Reg::Phys(PReg::Xmm0));
                return Ok(());
            },
            | _ => return Err(Construct::Opcode(opcode)),
        };
        let lhs = self.reg(lhs);
        let rhs = self.float_operand(rhs);
        self.copy(dst, lhs);
        self.emit(MInst::XAlu {
            op,
            width: float_width(ty),
            dst,
            src: rhs,
        });
        Ok(())
    }

    /// `fneg`: the sign bit flipped.
    pub(super) fn float_negate(&mut self, ty: Type, dst: Reg, arg: Value) {
        let src = self.reg(arg);
        let sign = if ty == Type::F32 { 1 << 31 } else { 1 << 63 };
        let index = self.func.constant(sign);
        self.copy(dst, src);
        self.emit(MInst::XAlu {
            op: XOp::Xor,
            width: float_width(ty),
            dst,
            src: Operand::Mem(Mem::new(Base::Constant(index), 0)),
        });
    }

    /// `fcmp`: a canonical `i1` from the flags of `ucomis*`.
    pub(super) fn float_compare(&mut self, cond: FloatCC, ty: Type, dst: Reg, [a, b]: [Value; 2]) {
        let a = self.reg(a);
        let b = self.reg(b);
        let (swapped, test) = float_test(cond);
        let (lhs, rhs) = if swapped { (b, a) } else { (a, b) };
        self.emit(MInst::Ucomi {
            width: float_width(ty),
            lhs,
            rhs,
        });
        self.set_test(test, dst);
    }

    /// `dst ←` the canonical `i1` of a flag test, right after the
    /// comparison that set the flags.
    pub(super) fn set_test(&mut self, test: Test, dst: Reg) {
        let byte = self.vreg();
        match test {
            | Test::One(cond) => self.emit(MInst::Setcc { cond, dst: byte }),
            | Test::Both(first, second) | Test::Either(first, second) => {
                let other = self.vreg();
                self.emit(MInst::Setcc {
                    cond: first,
                    dst:  byte,
                });
                self.emit(MInst::Setcc {
                    cond: second,
                    dst:  other,
                });
                let op = if matches!(test, Test::Both(..)) {
                    AluOp::And
                } else {
                    AluOp::Or
                };
                self.emit(MInst::Alu {
                    op,
                    width: Width::B8,
                    dst: byte,
                    src: Operand::Reg(other),
                });
            },
        }
        self.emit(MInst::Extend {
            signed: false,
            from: Width::B8,
            to: Width::B32,
            dst,
            src: Operand::Reg(byte),
        });
    }

    /// `select` of floats: both arms move to general registers, where
    /// `cmov` picks one, and the result moves back.
    pub(super) fn float_select(&mut self, ty: Type, dst: Reg, [cond, then, otherwise]: [Value; 3]) {
        let width = float_width(ty);
        let then = self.reg(then);
        let otherwise = self.reg(otherwise);
        let then_bits = self.bits_in_gpr(width, then);
        let picked = self.bits_in_gpr(width, otherwise);
        let cond = self.condition(cond);
        self.emit(MInst::Cmov {
            cond,
            width: Width::B64,
            dst: picked,
            src: then_bits,
        });
        self.emit(MInst::MovGx {
            width,
            dst,
            src: picked,
        });
    }

    /// The bits of an SSE register in a fresh general register.
    fn bits_in_gpr(&mut self, width: Width, src: Reg) -> Reg {
        let dst = self.vreg();
        self.emit(MInst::MovGx { width, dst, src });
        dst
    }

    /// The conversions with a float on either side, and bit casts between
    /// the classes.
    pub(super) fn float_conversion(
        &mut self,
        opcode: Opcode,
        ty: Type,
        dst: Reg,
        arg: Value,
    ) -> Result<(), Construct> {
        let from = self.ty(arg);
        match opcode {
            | Opcode::Fpext | Opcode::Fptrunc => {
                let src = self.reg(arg);
                self.emit(MInst::FloatToFloat {
                    from: float_width(from),
                    dst,
                    src,
                });
            },
            | Opcode::Sitofp => {
                let (src, int) = self.widened_int(arg, true);
                self.emit(MInst::IntToFloat {
                    int,
                    float: float_width(ty),
                    dst,
                    src,
                });
            },
            | Opcode::Uitofp if from == Type::I64 => self.unsigned_to_float(ty, dst, arg),
            | Opcode::Uitofp => {
                let (src, int) = self.widened_int(arg, false);
                self.emit(MInst::IntToFloat {
                    int,
                    float: float_width(ty),
                    dst,
                    src,
                });
            },
            | Opcode::Fptosi => {
                let src = self.reg(arg);
                self.emit(MInst::FloatToInt {
                    float: float_width(from),
                    int: if ty.bits() > 32 {
                        Width::B64
                    } else {
                        Width::B32
                    },
                    dst,
                    src,
                });
            },
            | Opcode::Fptoui if ty == Type::I64 => self.float_to_unsigned(from, dst, arg),
            | Opcode::Fptoui => {
                // Every in-range value of 32 bits or fewer fits a signed
                // 64-bit conversion.
                let src = self.reg(arg);
                self.emit(MInst::FloatToInt {
                    float: float_width(from),
                    int: if ty.bits() >= 32 {
                        Width::B64
                    } else {
                        Width::B32
                    },
                    dst,
                    src,
                });
            },
            | Opcode::Bitcast => {
                let src = self.reg(arg);
                let width = if ty.bits() == 64 {
                    Width::B64
                } else {
                    Width::B32
                };
                self.emit(MInst::MovGx { width, dst, src });
            },
            | _ => return Err(Construct::Opcode(opcode)),
        }
        Ok(())
    }

    /// An integer for `cvtsi2s*`: extended by `signed` to 32 bits, or an
    /// unsigned 32-bit value zero-extended to 64.
    fn widened_int(&mut self, value: Value, signed: bool) -> (Reg, Width) {
        let ty = self.ty(value);
        match ty {
            | Type::I64 | Type::Ptr => (self.reg(value), Width::B64),
            | Type::I32 if signed => (self.reg(value), Width::B32),
            | Type::I32 => {
                let src = self.reg(value);
                let dst = self.vreg();
                self.emit(MInst::Extend {
                    signed: false,
                    from: Width::B32,
                    to: Width::B64,
                    dst,
                    src: Operand::Reg(src),
                });
                (dst, Width::B64)
            },
            | Type::I1 if signed => {
                // A canonical `i1` true is -1 as a signed value.
                let src = self.reg(value);
                let dst = self.vreg();
                self.copy(dst, src);
                self.emit(MInst::Unary {
                    op: crate::backend::native::mir::UnaryOp::Neg,
                    width: Width::B32,
                    dst,
                });
                (dst, Width::B32)
            },
            | _ => (self.extended(value, signed), Width::B32),
        }
    }

    /// `uitofp` from `i64`: a value with its top bit set is halved with its
    /// low bit kept, converted and doubled.
    fn unsigned_to_float(&mut self, ty: Type, dst: Reg, arg: Value) {
        let float = float_width(ty);
        let x = self.reg(arg);
        let direct = self.func.vreg_of(RegClass::Xmm);
        self.emit(MInst::IntToFloat {
            int: Width::B64,
            float,
            dst: Reg::Virt(direct),
            src: x,
        });
        let half = self.vreg();
        self.copy(half, x);
        self.emit(MInst::Shift {
            op:     ShiftOp::Shr,
            width:  Width::B64,
            dst:    half,
            amount: ShiftAmount::Imm(1),
        });
        let low = self.vreg();
        self.copy(low, x);
        self.emit(MInst::Alu {
            op:    AluOp::And,
            width: Width::B64,
            dst:   low,
            src:   Operand::Imm(1),
        });
        self.emit(MInst::Alu {
            op:    AluOp::Or,
            width: Width::B64,
            dst:   half,
            src:   Operand::Reg(low),
        });
        let halved = Reg::Virt(self.func.vreg_of(RegClass::Xmm));
        self.emit(MInst::IntToFloat {
            int: Width::B64,
            float,
            dst: halved,
            src: half,
        });
        self.emit(MInst::XAlu {
            op:    XOp::Add,
            width: float,
            dst:   halved,
            src:   Operand::Reg(halved),
        });
        let picked = self.bits_in_gpr(float, Reg::Virt(direct));
        let doubled = self.bits_in_gpr(float, halved);
        self.emit(MInst::Test {
            width: Width::B64,
            lhs:   x,
            rhs:   x,
        });
        self.emit(MInst::Cmov {
            cond:  Cond::S,
            width: Width::B64,
            dst:   picked,
            src:   doubled,
        });
        self.emit(MInst::MovGx {
            width: float,
            dst,
            src: picked,
        });
    }

    /// `fptoui` to `i64`: one truncating conversion of the value plus
    /// `adjust`, which is -2^63 when the value is at or above 2^63 and 0
    /// otherwise, then `adjust` xored back in to set the top bit. Both sums
    /// are exact. Picking the adjustment before converting keeps an
    /// in-range value out of reach of the "invalid" exception, which only
    /// an out-of-range value raises, and adds no inexact subtraction.
    /// C99: §F.4 paragraph 1, p. 447; PDF p. 459.
    fn float_to_unsigned(&mut self, from: Type, dst: Reg, arg: Value) {
        let float = float_width(from);
        let x = self.reg(arg);
        let limit_bits: u64 = if from == Type::F32 {
            0x5F00_0000
        } else {
            0x43E0_0000_0000_0000
        };
        let limit = Reg::Virt(self.func.vreg_of(RegClass::Xmm));
        self.float_constant(limit, limit_bits, from);
        let adjust = self.vreg();
        self.emit(MInst::Mov {
            width: Width::B32,
            dst:   adjust,
            src:   Operand::Imm(0),
        });
        let top = self.vreg();
        self.emit(MInst::MovImm64 {
            dst: top,
            imm: i64::MIN,
        });
        self.emit(MInst::Ucomi {
            width: float,
            lhs:   x,
            rhs:   limit,
        });
        self.emit(MInst::Cmov {
            cond:  Cond::Ae,
            width: Width::B64,
            dst:   adjust,
            src:   top,
        });
        let offset = Reg::Virt(self.func.vreg_of(RegClass::Xmm));
        self.emit(MInst::IntToFloat {
            int: Width::B64,
            float,
            dst: offset,
            src: adjust,
        });
        let reduced = Reg::Virt(self.func.vreg_of(RegClass::Xmm));
        self.copy(reduced, x);
        self.emit(MInst::XAlu {
            op:    XOp::Add,
            width: float,
            dst:   reduced,
            src:   Operand::Reg(offset),
        });
        self.emit(MInst::FloatToInt {
            float,
            int: Width::B64,
            dst,
            src: reduced,
        });
        self.emit(MInst::Alu {
            op: AluOp::Xor,
            width: Width::B64,
            dst,
            src: Operand::Reg(adjust),
        });
    }
}

/// The comparison a `setcc` sequence makes from the flags of `ucomis*` or
/// `fucomip`.
#[derive(Clone, Copy)]
pub(super) enum Test {
    One(Cond),
    Both(Cond, Cond),
    Either(Cond, Cond),
}

/// How a float comparison reads the flags of comparing its operands:
/// whether they are swapped first, and the flag test that is true exactly
/// when the condition holds.
pub(super) const fn float_test(cond: FloatCC) -> (bool, Test) {
    match cond {
        | FloatCC::Ogt => (false, Test::One(Cond::A)),
        | FloatCC::Oge => (false, Test::One(Cond::Ae)),
        | FloatCC::Olt => (true, Test::One(Cond::A)),
        | FloatCC::Ole => (true, Test::One(Cond::Ae)),
        | FloatCC::Ult => (false, Test::One(Cond::B)),
        | FloatCC::Ule => (false, Test::One(Cond::Be)),
        | FloatCC::Ugt => (true, Test::One(Cond::B)),
        | FloatCC::Uge => (true, Test::One(Cond::Be)),
        | FloatCC::One => (false, Test::One(Cond::Ne)),
        | FloatCC::Ueq => (false, Test::One(Cond::E)),
        | FloatCC::Ord => (false, Test::One(Cond::Np)),
        | FloatCC::Uno => (false, Test::One(Cond::P)),
        | FloatCC::Oeq => (false, Test::Both(Cond::E, Cond::Np)),
        | FloatCC::Une => (false, Test::Either(Cond::Ne, Cond::P)),
    }
}

/// The SSE width of a float type: `B32` for `f32`, `B64` for `f64`.
pub(super) const fn float_width(ty: Type) -> Width {
    match ty {
        | Type::F32 => Width::B32,
        | _ => Width::B64,
    }
}
