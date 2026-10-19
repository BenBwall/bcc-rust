//! `f80` (`long double` on x86-64 Linux and MinGW) through the x87 unit.
//!
//! An `f80` value lives in a pair of general registers like an `i128`: the
//! eight significand bytes, then the sign and exponent in the low 16 bits of
//! the high register. Copies, spills, selects, loads, stores and `fneg`
//! (which flips bit 15 of the high register) need nothing else. Arithmetic,
//! comparisons and conversions store their operands to the function's
//! scratch bytes ([`Selector::scratch`]), push them with `fldt`, compute,
//! and pop the result with `fstpt`. Every sequence leaves the x87 stack as
//! empty as it found it, so the register stack is never allocated; only a
//! `return` leaves its value in `%st(0)`, as System V returns `long double`
//! (psABI §3.2.3, "Returning of Values", class X87).
//!
//! `fucomip` sets ZF, PF and CF as `ucomisd` does, so comparisons read the
//! flags as the SSE ones do. Conversions to integers store with `fistp`
//! under a control word whose rounding control is "toward zero" (Intel SDM
//! Vol. 1, §8.1.5.3, RC = 11b), restored afterwards; the SSE3 `fisttp` is
//! not in the x86-64 baseline. `fistp` and `fild` handle signed 16-, 32-
//! and 64-bit integers, so narrower integers widen, unsigned 32-bit ones
//! convert at 64 bits, and unsigned 64-bit ones add 2^64 after the signed
//! load of a negative value, or subtract 2^63 before the signed store of a
//! value at or above it, as Clang does: the value is compared first, so
//! only the adjusted one is stored and an in-range value raises no
//! "invalid" exception. `frem` calls `fmodl`, and conversions with `i128`
//! and `f128` call the runtime (`isel/quad.rs`).
//! Intel SDM Vol. 2: FLD, FILD, FSTP, FISTP, FUCOMI/FUCOMIP, FLDCW, FNSTCW.

use super::{
    Selector,
    ValueKind,
    calls::{
        LibArg,
        LibResult,
    },
    float::{
        float_test,
        float_width,
    },
    wide::Halves,
};
use crate::{
    backend::native::{
        mir::{
            AluOp,
            Base,
            Cond,
            FrameObject,
            FrameSlotData,
            MInst,
            Mem,
            Operand,
            Reg,
            Width,
            X87Format,
            X87Op,
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

/// 2^63 as an `f80`: biased exponent 16383 + 63 and the explicit integer
/// bit.
const TWO_TO_63: u128 = 0x403E_8000_0000_0000_0000;

/// 2^64 as `f32` bits.
const TWO_TO_64_F32: i32 = 0x5F80_0000;

/// The x87 control word's rounding control bits set to "toward zero".
const ROUND_TOWARD_ZERO: i32 = 0x0C00;

/// Where the scratch bytes hold what: the first `f80` operand and every
/// result at 0, the second operand at 16, integer and narrower float
/// conversions at 16 too, and the saved and truncating control words at 24
/// and 26.
const FIRST: i32 = 0;
const SECOND: i32 = 16;
const INTEGER: i32 = 16;
const SAVED_CONTROL: i32 = 24;
const TRUNCATING_CONTROL: i32 = 26;

impl Selector<'_, '_, '_> {
    /// `offset` bytes into the function's 32 scratch bytes, a 16-aligned
    /// frame slot made on first use. Sequences use them only between
    /// their own instructions, so one area serves the whole function.
    pub(super) fn scratch(&mut self, offset: i32) -> Mem {
        let slot = match self.scratch {
            | Some(slot) => slot,
            | None => {
                let slot = u32::try_from(self.func.slots.len()).expect("frame slots fit u32");
                self.func.slots.push(FrameSlotData {
                    size:  32,
                    align: 16,
                });
                self.scratch = Some(slot);
                slot
            },
        };
        Mem::new(Base::Frame(FrameObject::Slot(slot)), offset)
    }

    /// Pushes an `f80` value onto the x87 stack: a constant from the
    /// function's constants, a register pair through the scratch bytes at
    /// `offset`.
    pub(super) fn x87_push(&mut self, value: Value, offset: i32) {
        let mem = match self.kind(value) {
            | ValueKind::Wide(bits) => Mem::new(Base::Constant(self.func.constant(bits)), 0),
            | _ => {
                let mem = self.scratch(offset);
                self.pair_store(Type::F80, value, mem);
                mem
            },
        };
        self.emit(MInst::X87Load {
            format: X87Format::F80,
            mem,
        });
    }

    /// Pushes the `f80` constant `bits`.
    fn x87_push_constant(&mut self, bits: u128) {
        let index = self.func.constant(bits);
        self.emit(MInst::X87Load {
            format: X87Format::F80,
            mem:    Mem::new(Base::Constant(index), 0),
        });
    }

    /// Pops `%st(0)` into the pair `dst`.
    fn x87_pop_into(&mut self, dst: Halves) {
        let mem = self.scratch(FIRST);
        self.emit(MInst::X87Store {
            format: X87Format::F80,
            mem,
        });
        self.pair_load(Type::F80, dst, mem);
    }

    /// `fadd`, `fsub`, `fmul`, `fdiv` and `frem` of `f80` values.
    pub(super) fn x87_binary(
        &mut self,
        opcode: Opcode,
        dst: Halves,
        [lhs, rhs]: [Value; 2],
    ) -> Result<(), Construct> {
        let op = match opcode {
            | Opcode::Fadd => X87Op::Add,
            | Opcode::Fsub => X87Op::Sub,
            | Opcode::Fmul => X87Op::Mul,
            | Opcode::Fdiv => X87Op::Div,
            | Opcode::Frem => {
                // C's `fmodl` computes the IR's `frem`, as for `f64`.
                let args = [lhs, rhs].map(|value| LibArg::Wide(Type::F80, self.pair(value)));
                self.libcall("fmodl", &args, LibResult::Wide(Type::F80, dst));
                return Ok(());
            },
            | _ => return Err(Construct::Opcode(opcode)),
        };
        self.x87_push(rhs, SECOND);
        self.x87_push(lhs, FIRST);
        self.emit(MInst::X87Arith { op });
        self.x87_pop_into(dst);
        self.emit(MInst::X87Pop);
        Ok(())
    }

    /// `fneg` of an `f80`: the sign bit flipped.
    pub(super) fn x87_negate(&mut self, dst: Halves, arg: Value) {
        let src = self.pair(arg);
        self.copy_pair(dst, src);
        self.emit(MInst::Alu {
            op:    AluOp::Xor,
            width: Width::B32,
            dst:   dst.1,
            src:   Operand::Imm(0x8000),
        });
    }

    /// `fcmp` of `f80` values: `fucomip` with the left operand in
    /// `%st(0)`, then the same flag tests as `ucomis*`.
    pub(super) fn x87_compare(&mut self, cond: FloatCC, dst: Reg, [a, b]: [Value; 2]) {
        let (swapped, test) = float_test(cond);
        let (lhs, rhs) = if swapped { (b, a) } else { (a, b) };
        self.x87_push(rhs, SECOND);
        self.x87_push(lhs, FIRST);
        self.emit(MInst::X87Compare);
        // `fstp` leaves EFLAGS alone.
        self.emit(MInst::X87Pop);
        self.set_test(test, dst);
    }

    /// `fpext` from `f32` or `f64`, and `sitofp` or `uitofp` from integers
    /// of at most 64 bits, to `f80`.
    pub(super) fn convert_to_x87(&mut self, opcode: Opcode, dst: Halves, arg: Value) {
        let from = self.ty(arg);
        let mem = self.scratch(INTEGER);
        let format = match (opcode, from) {
            | (Opcode::Fpext, _) => {
                let src = self.reg(arg);
                self.emit(MInst::XStore {
                    width: float_width(from),
                    dst: mem,
                    src,
                });
                if from == Type::F32 {
                    X87Format::F32
                } else {
                    X87Format::F64
                }
            },
            | (Opcode::Uitofp, Type::I64) => {
                self.unsigned_to_x87(dst, arg);
                return;
            },
            | (Opcode::Sitofp, Type::I64) => {
                let src = self.reg(arg);
                self.store_integer(Width::B64, src, mem);
                X87Format::I64
            },
            | (Opcode::Uitofp, Type::I32) => {
                let src = self.reg(arg);
                let wide = self.vreg();
                self.emit(MInst::Extend {
                    signed: false,
                    from:   Width::B32,
                    to:     Width::B64,
                    dst:    wide,
                    src:    Operand::Reg(src),
                });
                self.store_integer(Width::B64, wide, mem);
                X87Format::I64
            },
            | (Opcode::Sitofp, Type::I1) => {
                // A canonical `i1` true is -1 as a signed value.
                let src = self.reg(arg);
                let negated = self.vreg();
                self.copy(negated, src);
                self.emit(MInst::Unary {
                    op:    crate::backend::native::mir::UnaryOp::Neg,
                    width: Width::B32,
                    dst:   negated,
                });
                self.store_integer(Width::B32, negated, mem);
                X87Format::I32
            },
            | _ => {
                let src = self.extended(arg, opcode == Opcode::Sitofp);
                self.store_integer(Width::B32, src, mem);
                X87Format::I32
            },
        };
        self.emit(MInst::X87Load { format, mem });
        self.x87_pop_into(dst);
    }

    /// Stores an integer register for `fild`.
    fn store_integer(&mut self, width: Width, src: Reg, mem: Mem) {
        self.emit(MInst::Store {
            width,
            dst: mem,
            src: Operand::Reg(src),
        });
    }

    /// `uitofp` from `i64`: the signed conversion, plus 2^64 when the top
    /// bit is set. The sum is exact, since 64 significand bits hold every
    /// result.
    fn unsigned_to_x87(&mut self, dst: Halves, arg: Value) {
        let src = self.reg(arg);
        let mem = self.scratch(INTEGER);
        self.store_integer(Width::B64, src, mem);
        self.emit(MInst::X87Load {
            format: X87Format::I64,
            mem,
        });
        let adjust = self.vreg();
        self.emit(MInst::Mov {
            width: Width::B32,
            dst:   adjust,
            src:   Operand::Imm(0),
        });
        let two_to_64 = self.vreg();
        self.emit(MInst::Mov {
            width: Width::B32,
            dst:   two_to_64,
            src:   Operand::Imm(TWO_TO_64_F32),
        });
        self.emit(MInst::Test {
            width: Width::B64,
            lhs:   src,
            rhs:   src,
        });
        self.emit(MInst::Cmov {
            cond:  Cond::S,
            width: Width::B32,
            dst:   adjust,
            src:   two_to_64,
        });
        let adjust_mem = self.scratch(SAVED_CONTROL);
        self.store_integer(Width::B32, adjust, adjust_mem);
        self.emit(MInst::X87Load {
            format: X87Format::F32,
            mem:    adjust_mem,
        });
        self.emit(MInst::X87Arith { op: X87Op::Add });
        self.x87_pop_into(dst);
        self.emit(MInst::X87Pop);
    }

    /// `fptrunc` to `f32` or `f64`, and `fptosi` or `fptoui` to integers of
    /// at most 64 bits, from `f80`.
    pub(super) fn convert_from_x87(&mut self, opcode: Opcode, ty: Type, dst: Reg, arg: Value) {
        let mem = self.scratch(INTEGER);
        if opcode == Opcode::Fptrunc {
            self.x87_push(arg, FIRST);
            let width = float_width(ty);
            self.emit(MInst::X87Store {
                format: if ty == Type::F32 {
                    X87Format::F32
                } else {
                    X87Format::F64
                },
                mem,
            });
            self.emit(MInst::XMov {
                width,
                dst,
                src: Operand::Mem(mem),
            });
            return;
        }
        let signed = opcode == Opcode::Fptosi;
        // Every in-range result fits the signed format chosen.
        let format = match (ty, signed) {
            | (Type::I64, false) => {
                self.x87_to_unsigned(dst, arg);
                return;
            },
            | (Type::I1 | Type::I8, _) | (Type::I16, true) => X87Format::I16,
            | (Type::I16, false) | (Type::I32, true) => X87Format::I32,
            | _ => X87Format::I64,
        };
        self.x87_push(arg, FIRST);
        self.store_truncated(format, mem);
        self.load_integer(format, dst, mem);
        if ty == Type::I1 {
            self.canonical_bool(dst);
        }
    }

    /// `fptoui` to `i64`: one truncating conversion of the value plus
    /// `adjust`, which is -2^63 when the value is at or above 2^63 and 0
    /// otherwise, then `adjust` xored back in to set the top bit. The sum
    /// is exact. Picking the adjustment before converting keeps an
    /// in-range value out of reach of the "invalid" exception, which only
    /// an out-of-range value raises; Clang picks it with `fcmov`.
    /// C99: §F.4 paragraph 1, p. 447; PDF p. 459.
    fn x87_to_unsigned(&mut self, dst: Reg, arg: Value) {
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
        self.x87_push(arg, FIRST);
        self.x87_push_constant(TWO_TO_63);
        // 2^63 <= value: CF or ZF set; `fucomip` pops 2^63.
        self.emit(MInst::X87Compare);
        self.emit(MInst::Cmov {
            cond:  Cond::Be,
            width: Width::B64,
            dst:   adjust,
            src:   top,
        });
        let mem = self.scratch(INTEGER);
        self.store_integer(Width::B64, adjust, mem);
        self.emit(MInst::X87Load {
            format: X87Format::I64,
            mem,
        });
        self.emit(MInst::X87Arith { op: X87Op::Add });
        self.store_truncated(X87Format::I64, mem);
        self.emit(MInst::X87Pop);
        self.load_integer(X87Format::I64, dst, mem);
        self.emit(MInst::Alu {
            op: AluOp::Xor,
            width: Width::B64,
            dst,
            src: Operand::Reg(adjust),
        });
    }

    /// Pops `%st(0)` to `mem` as an integer of `format`, rounded toward
    /// zero.
    fn store_truncated(&mut self, format: X87Format, mem: Mem) {
        let saved = self.scratch(SAVED_CONTROL);
        let truncating = self.scratch(TRUNCATING_CONTROL);
        self.emit(MInst::X87ControlWord {
            store: true,
            mem:   saved,
        });
        let control = self.vreg();
        self.emit(MInst::Extend {
            signed: false,
            from:   Width::B16,
            to:     Width::B32,
            dst:    control,
            src:    Operand::Mem(saved),
        });
        self.emit(MInst::Alu {
            op:    AluOp::Or,
            width: Width::B32,
            dst:   control,
            src:   Operand::Imm(ROUND_TOWARD_ZERO),
        });
        self.emit(MInst::Store {
            width: Width::B16,
            dst:   truncating,
            src:   Operand::Reg(control),
        });
        self.emit(MInst::X87ControlWord {
            store: false,
            mem:   truncating,
        });
        self.emit(MInst::X87Store { format, mem });
        self.emit(MInst::X87ControlWord {
            store: false,
            mem:   saved,
        });
    }

    /// Loads the integer `fistp` stored.
    fn load_integer(&mut self, format: X87Format, dst: Reg, mem: Mem) {
        match format {
            | X87Format::I16 => self.emit(MInst::Extend {
                signed: false,
                from: Width::B16,
                to: Width::B32,
                dst,
                src: Operand::Mem(mem),
            }),
            | X87Format::I32 => self.emit(MInst::Mov {
                width: Width::B32,
                dst,
                src: Operand::Mem(mem),
            }),
            | _ => self.emit(MInst::Mov {
                width: Width::B64,
                dst,
                src: Operand::Mem(mem),
            }),
        }
    }
}
