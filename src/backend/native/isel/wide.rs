//! `i128` as pairs of 64-bit registers, low half first.
//!
//! Addition and subtraction carry between the halves (`add`/`adc`,
//! `sub`/`sbb`), bitwise operations work on each half, and multiplication
//! forms the low 128 bits from one widening `mul` and two cross products.
//! Shifts by a constant move or `shld`/`shrd` the halves; a variable count
//! shifts by its low six bits and then, if bit 6 is set, moves one half into
//! the other with `cmov` (counts of 128 or more are poison). Equality ORs the
//! XORs of the halves; ordering subtracts with borrow, whose flags give the
//! signed and unsigned conditions. Division calls the runtime's `__divti3`
//! family: under System V with each operand in a register pair and the
//! result in `%rdx:%rax` (psABI §3.2.3), under Win64 with pointers to
//! 16-byte copies and the result in `%xmm0`, as bundled Clang and GCC call
//! them.
//!
//! `uadd_overflow` is the carry out of the high half; `umul_overflow`
//! combines the five ways a 128 × 128-bit product can exceed 128 bits: both
//! high halves nonzero, either cross product exceeding 64 bits, their sum
//! carrying, or that sum carrying into the high half of the low product.

use super::{
    Selector,
    ValueKind,
    calls::{
        LibArg,
        LibResult,
    },
    mask,
    sign_extend,
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
            PReg,
            Reg,
            ShiftAmount,
            ShiftOp,
            UnaryOp,
            Width,
        },
        support::Construct,
    },
    ir::{
        Inst,
        IntCC,
        Opcode,
        Type,
        Value,
    },
};

/// A value as its low and high halves.
pub(super) type Halves = (Reg, Reg);

impl Selector<'_, '_, '_> {
    /// The halves of an `i128` value, materializing a constant.
    pub(super) fn pair(&mut self, value: Value) -> Halves {
        match self.kind(value) {
            | ValueKind::Pair(low, high) => (Reg::Virt(low), Reg::Virt(high)),
            | ValueKind::Wide(bits) => {
                let (low, high) = (self.vreg(), self.vreg());
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "The halves are split here."
                )]
                let (low_bits, high_bits) = (bits as u64, (bits >> 64) as u64);
                self.constant(low, low_bits, Type::I64);
                self.constant(high, high_bits, Type::I64);
                (low, high)
            },
            | _ => (self.vreg(), self.vreg()),
        }
    }

    /// The halves an instruction's `i128` result was classified into.
    pub(super) fn result_pair(&self, inst: Inst) -> Halves {
        match self.body.inst_result(inst).map(|value| self.kind(value)) {
            | Some(ValueKind::Pair(low, high)) => (Reg::Virt(low), Reg::Virt(high)),
            | _ => unreachable!("an i128 instruction with code has a pair result"),
        }
    }

    /// Copies both halves.
    pub(super) fn copy_pair(&mut self, (dst_low, dst_high): Halves, (low, high): Halves) {
        self.copy(dst_low, low);
        self.copy(dst_high, high);
    }

    /// The binary operations on `i128`.
    pub(super) fn wide_binary(
        &mut self,
        inst: Inst,
        opcode: Opcode,
        [lhs, rhs]: [Value; 2],
    ) -> Result<(), Construct> {
        match opcode {
            | Opcode::UaddOverflow => {
                let dst = self.result_reg(self.body.inst_result(inst));
                let (a, b) = (self.pair(lhs), self.pair(rhs));
                let (low, high) = (self.vreg(), self.vreg());
                self.copy_pair((low, high), a);
                self.alu2(AluOp::Add, low, b.0);
                self.alu2(AluOp::Adc, high, b.1);
                self.set_condition(Cond::B, dst);
                return Ok(());
            },
            | Opcode::UmulOverflow => {
                let dst = self.result_reg(self.body.inst_result(inst));
                self.wide_mul_overflow(dst, lhs, rhs);
                return Ok(());
            },
            | _ => {},
        }
        let dst = self.result_pair(inst);
        let (a, b) = (self.pair(lhs), self.pair(rhs));
        match opcode {
            | Opcode::Iadd | Opcode::Isub => {
                let (low_op, high_op) = if opcode == Opcode::Iadd {
                    (AluOp::Add, AluOp::Adc)
                } else {
                    (AluOp::Sub, AluOp::Sbb)
                };
                self.copy_pair(dst, a);
                self.alu2(low_op, dst.0, b.0);
                self.alu2(high_op, dst.1, b.1);
            },
            | Opcode::And | Opcode::Or | Opcode::Xor => {
                let op = match opcode {
                    | Opcode::And => AluOp::And,
                    | Opcode::Or => AluOp::Or,
                    | _ => AluOp::Xor,
                };
                self.copy_pair(dst, a);
                self.alu2(op, dst.0, b.0);
                self.alu2(op, dst.1, b.1);
            },
            | Opcode::Imul => self.wide_multiply(dst, a, b),
            | Opcode::Shl | Opcode::Lshr | Opcode::Ashr => self.wide_shift(opcode, dst, a, rhs),
            | Opcode::Sdiv | Opcode::Udiv | Opcode::Srem | Opcode::Urem => {
                let name = match opcode {
                    | Opcode::Sdiv => "__divti3",
                    | Opcode::Udiv => "__udivti3",
                    | Opcode::Srem => "__modti3",
                    | _ => "__umodti3",
                };
                self.wide_libcall(name, dst, a, b);
            },
            | _ => return Err(Construct::Opcode(opcode)),
        }
        Ok(())
    }

    /// `dst op= src` on 64-bit registers.
    fn alu2(&mut self, op: AluOp, dst: Reg, src: Reg) {
        self.emit(MInst::Alu {
            op,
            width: Width::B64,
            dst,
            src: Operand::Reg(src),
        });
    }

    /// The low 128 bits of a product: `al × bl` widened, plus both cross
    /// products into the high half.
    fn wide_multiply(&mut self, dst: Halves, (al, ah): Halves, (bl, bh): Halves) {
        let cross = self.vreg();
        self.copy(cross, al);
        self.alu2(AluOp::Imul, cross, bh);
        let other = self.vreg();
        self.copy(other, ah);
        self.alu2(AluOp::Imul, other, bl);
        self.emit(MInst::Mov {
            width: Width::B64,
            dst:   Reg::Phys(PReg::Rax),
            src:   Operand::Reg(al),
        });
        self.emit(MInst::MulWide {
            width: Width::B64,
            src:   bl,
        });
        self.copy(dst.0, Reg::Phys(PReg::Rax));
        self.copy(dst.1, Reg::Phys(PReg::Rdx));
        self.alu2(AluOp::Add, dst.1, cross);
        self.alu2(AluOp::Add, dst.1, other);
    }

    /// Shifts of an `i128`.
    fn wide_shift(&mut self, opcode: Opcode, dst: Halves, (low, high): Halves, amount: Value) {
        if let ValueKind::Wide(count) = self.kind(amount) {
            let count = (count & 127) as u8;
            self.wide_shift_constant(opcode, dst, (low, high), count);
            return;
        }
        let count = self.pair(amount).0;
        // `sar` sets the flags, so the sign comes before the test of bit 6.
        let fill = self.vreg();
        if opcode == Opcode::Ashr {
            self.copy(fill, high);
            self.shift_imm(ShiftOp::Sar, Width::B64, fill, 63);
        } else {
            self.constant(fill, 0, Type::I64);
        }
        self.copy_pair(dst, (low, high));
        self.emit(MInst::Mov {
            width: Width::B64,
            dst:   Reg::Phys(PReg::Rcx),
            src:   Operand::Reg(count),
        });
        let (into, from, op) = match opcode {
            | Opcode::Shl => (dst.1, dst.0, ShiftOp::Shl),
            | Opcode::Lshr => (dst.0, dst.1, ShiftOp::Shr),
            | _ => (dst.0, dst.1, ShiftOp::Sar),
        };
        self.emit(MInst::ShiftDouble {
            left:   opcode == Opcode::Shl,
            dst:    into,
            src:    from,
            amount: ShiftAmount::Cl,
        });
        self.emit(MInst::Shift {
            op,
            width: Width::B64,
            dst: from,
            amount: ShiftAmount::Cl,
        });
        let bit = self.vreg();
        self.copy(bit, count);
        self.emit(MInst::Alu {
            op:    AluOp::And,
            width: Width::B64,
            dst:   bit,
            src:   Operand::Imm(64),
        });
        // With bit 6 set, the shifted half moves over and the other fills.
        self.emit(MInst::Cmov {
            cond:  Cond::Ne,
            width: Width::B64,
            dst:   into,
            src:   from,
        });
        self.emit(MInst::Cmov {
            cond:  Cond::Ne,
            width: Width::B64,
            dst:   from,
            src:   fill,
        });
    }

    /// Shifts of an `i128` by a constant count below 128.
    fn wide_shift_constant(&mut self, opcode: Opcode, dst: Halves, (low, high): Halves, count: u8) {
        if count == 0 {
            self.copy_pair(dst, (low, high));
            return;
        }
        match opcode {
            | Opcode::Shl if count >= 64 => {
                self.copy(dst.1, low);
                self.shift_imm(ShiftOp::Shl, Width::B64, dst.1, count - 64);
                self.constant(dst.0, 0, Type::I64);
            },
            | Opcode::Shl => {
                self.copy_pair(dst, (low, high));
                self.emit(MInst::ShiftDouble {
                    left:   true,
                    dst:    dst.1,
                    src:    dst.0,
                    amount: ShiftAmount::Imm(count),
                });
                self.shift_imm(ShiftOp::Shl, Width::B64, dst.0, count);
            },
            | _ if count >= 64 => {
                let op = if opcode == Opcode::Ashr {
                    ShiftOp::Sar
                } else {
                    ShiftOp::Shr
                };
                self.copy(dst.0, high);
                self.shift_imm(op, Width::B64, dst.0, count - 64);
                if opcode == Opcode::Ashr {
                    self.copy(dst.1, high);
                    self.shift_imm(ShiftOp::Sar, Width::B64, dst.1, 63);
                } else {
                    self.constant(dst.1, 0, Type::I64);
                }
            },
            | _ => {
                let op = if opcode == Opcode::Ashr {
                    ShiftOp::Sar
                } else {
                    ShiftOp::Shr
                };
                self.copy_pair(dst, (low, high));
                self.emit(MInst::ShiftDouble {
                    left:   false,
                    dst:    dst.0,
                    src:    dst.1,
                    amount: ShiftAmount::Imm(count),
                });
                self.shift_imm(op, Width::B64, dst.1, count);
            },
        }
    }

    /// Emits an `i128` comparison and returns the condition that holds
    /// when it is true.
    pub(super) fn wide_compare(&mut self, cond: IntCC, [lhs, rhs]: [Value; 2]) -> Cond {
        let a = self.pair(lhs);
        let b = self.pair(rhs);
        if matches!(cond, IntCC::Eq | IntCC::Ne) {
            let (low, high) = (self.vreg(), self.vreg());
            self.copy_pair((low, high), a);
            self.alu2(AluOp::Xor, low, b.0);
            self.alu2(AluOp::Xor, high, b.1);
            self.alu2(AluOp::Or, low, high);
            return if cond == IntCC::Eq { Cond::E } else { Cond::Ne };
        }
        let (x, y, result) = match cond {
            | IntCC::Ult => (a, b, Cond::B),
            | IntCC::Uge => (a, b, Cond::Ae),
            | IntCC::Slt => (a, b, Cond::L),
            | IntCC::Sge => (a, b, Cond::Ge),
            | IntCC::Ugt => (b, a, Cond::B),
            | IntCC::Ule => (b, a, Cond::Ae),
            | IntCC::Sgt => (b, a, Cond::L),
            | _ => (b, a, Cond::Ge),
        };
        let high = self.vreg();
        self.copy(high, x.1);
        self.emit(MInst::Cmp {
            width: Width::B64,
            lhs:   x.0,
            rhs:   Operand::Reg(y.0),
        });
        self.alu2(AluOp::Sbb, high, y.1);
        result
    }

    /// `select` of `i128` values: both halves pick under one condition.
    pub(super) fn wide_select(&mut self, dst: Halves, [cond, then, otherwise]: [Value; 3]) {
        let then = self.pair(then);
        let otherwise = self.pair(otherwise);
        self.copy_pair(dst, otherwise);
        let cond = self.condition(cond);
        for (dst, src) in [(dst.0, then.0), (dst.1, then.1)] {
            self.emit(MInst::Cmov {
                cond,
                width: Width::B64,
                dst,
                src,
            });
        }
    }

    /// Conversions to and from `i128` that are not aliases.
    pub(super) fn wide_conversion(
        &mut self,
        inst: Inst,
        opcode: Opcode,
        ty: Type,
        arg: Value,
    ) -> Result<(), Construct> {
        let from = self.ty(arg);
        if from == Type::I128 {
            // To an `i1` (other truncations are aliases of the low half), or
            // a `freeze` in unreachable code.
            let (low, high) = self.pair(arg);
            if ty == Type::I128 {
                let dst = self.result_pair(inst);
                self.copy_pair(dst, (low, high));
                return Ok(());
            }
            let dst = self.result_reg(self.body.inst_result(inst));
            self.copy(dst, low);
            if ty == Type::I1 {
                self.canonical_bool(dst);
            }
            return Ok(());
        }
        let dst = self.result_pair(inst);
        match opcode {
            | Opcode::Zext | Opcode::Ptrtoint => {
                let src = self.reg(arg);
                match from {
                    | Type::I8 | Type::I16 | Type::I32 => self.emit(MInst::Extend {
                        signed: false,
                        from:   super::width(from),
                        to:     Width::B64,
                        dst:    dst.0,
                        src:    Operand::Reg(src),
                    }),
                    | _ => self.copy(dst.0, src),
                }
                self.constant(dst.1, 0, Type::I64);
            },
            | Opcode::Sext => {
                let src = self.reg(arg);
                match from {
                    | Type::I8 | Type::I16 | Type::I32 => self.emit(MInst::Extend {
                        signed: true,
                        from:   super::width(from),
                        to:     Width::B64,
                        dst:    dst.0,
                        src:    Operand::Reg(src),
                    }),
                    | Type::I1 => {
                        self.copy(dst.0, src);
                        self.emit(MInst::Unary {
                            op:    UnaryOp::Neg,
                            width: Width::B64,
                            dst:   dst.0,
                        });
                    },
                    | _ => self.copy(dst.0, src),
                }
                self.copy(dst.1, dst.0);
                self.shift_imm(ShiftOp::Sar, Width::B64, dst.1, 63);
            },
            | _ => return Err(Construct::Opcode(opcode)),
        }
        Ok(())
    }

    /// Loads a pair value of type `ty`: an `f80` as its eight significand
    /// bytes and two sign and exponent bytes, which is all the IR's `f80`
    /// load reads; an `i128` or `f128` as two eight-byte halves.
    pub(super) fn pair_load(&mut self, ty: Type, dst: Halves, mem: Mem) {
        if ty != Type::F80 {
            self.wide_load(dst, mem);
            return;
        }
        self.emit(MInst::Mov {
            width: Width::B64,
            dst:   dst.0,
            src:   Operand::Mem(mem),
        });
        self.emit(MInst::Extend {
            signed: false,
            from:   Width::B16,
            to:     Width::B32,
            dst:    dst.1,
            src:    Operand::Mem(mem.offset(8)),
        });
    }

    /// Loads an `i128` as two 8-byte halves.
    pub(super) fn wide_load(&mut self, dst: Halves, mem: Mem) {
        for (half, disp) in [(dst.0, 0), (dst.1, 8)] {
            self.emit(MInst::Mov {
                width: Width::B64,
                dst:   half,
                src:   Operand::Mem(mem.offset(disp)),
            });
        }
    }

    /// Stores a pair value of type `ty`: ten bytes of an `f80`, sixteen of
    /// an `i128` or `f128`.
    pub(super) fn pair_store(&mut self, ty: Type, value: Value, mem: Mem) {
        let high = if ty == Type::F80 {
            Width::B16
        } else {
            Width::B64
        };
        let halves = match self.kind(value) {
            | ValueKind::Wide(bits) => {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "The halves are split here."
                )]
                let halves = [(bits as u64, Width::B64), ((bits >> 64) as u64, high)];
                halves.map(|(half, width)| match super::immediate(half, width) {
                    | Some(imm) => Operand::Imm(imm),
                    | None => {
                        let reg = self.vreg();
                        self.constant(reg, half, Type::I64);
                        Operand::Reg(reg)
                    },
                })
            },
            | _ => {
                let (low, high) = self.pair(value);
                [Operand::Reg(low), Operand::Reg(high)]
            },
        };
        for ((src, disp), width) in halves.into_iter().zip([0, 8]).zip([Width::B64, high]) {
            self.emit(MInst::Store {
                width,
                dst: mem.offset(disp),
                src,
            });
        }
    }

    /// Calls a runtime `i128` division routine.
    fn wide_libcall(&mut self, name: &'static str, dst: Halves, a: Halves, b: Halves) {
        self.libcall(
            name,
            &[LibArg::Wide(Type::I128, a), LibArg::Wide(Type::I128, b)],
            LibResult::Wide(Type::I128, dst),
        );
    }

    /// A fresh 16-byte, 16-aligned frame slot.
    pub(super) fn wide_temporary(&mut self) -> Mem {
        let slot = u32::try_from(self.func.slots.len()).expect("frame slots fit u32");
        self.func.slots.push(FrameSlotData {
            size:  16,
            align: 16,
        });
        Mem::new(Base::Frame(FrameObject::Slot(slot)), 0)
    }

    /// `umul_overflow` of `i128` operands.
    fn wide_mul_overflow(&mut self, dst: Reg, lhs: Value, rhs: Value) {
        let (al, ah) = self.pair(lhs);
        let (bl, bh) = self.pair(rhs);
        let mut flags = [Reg::Phys(PReg::Rax); 6];
        // Both high halves nonzero.
        let (both, other) = (self.vreg(), self.vreg());
        self.emit(MInst::Test {
            width: Width::B64,
            lhs:   ah,
            rhs:   ah,
        });
        self.emit(MInst::Setcc {
            cond: Cond::Ne,
            dst:  both,
        });
        self.emit(MInst::Test {
            width: Width::B64,
            lhs:   bh,
            rhs:   bh,
        });
        self.emit(MInst::Setcc {
            cond: Cond::Ne,
            dst:  other,
        });
        self.emit(MInst::Alu {
            op:    AluOp::And,
            width: Width::B8,
            dst:   both,
            src:   Operand::Reg(other),
        });
        flags[0] = both;
        // The cross products, each overflowing 64 bits or not.
        let mut cross = [Reg::Phys(PReg::Rax); 2];
        for (index, (x, y)) in [(ah, bl), (al, bh)].into_iter().enumerate() {
            self.emit(MInst::Mov {
                width: Width::B64,
                dst:   Reg::Phys(PReg::Rax),
                src:   Operand::Reg(x),
            });
            self.emit(MInst::MulWide {
                width: Width::B64,
                src:   y,
            });
            let flag = self.vreg();
            self.emit(MInst::Setcc {
                cond: Cond::O,
                dst:  flag,
            });
            flags[1 + index] = flag;
            cross[index] = self.vreg();
            self.copy(cross[index], Reg::Phys(PReg::Rax));
        }
        // Their sum, and its carry into the low product's high half.
        self.alu2(AluOp::Add, cross[0], cross[1]);
        flags[3] = self.vreg();
        self.emit(MInst::Setcc {
            cond: Cond::B,
            dst:  flags[3],
        });
        self.emit(MInst::Mov {
            width: Width::B64,
            dst:   Reg::Phys(PReg::Rax),
            src:   Operand::Reg(al),
        });
        self.emit(MInst::MulWide {
            width: Width::B64,
            src:   bl,
        });
        let high = self.vreg();
        self.copy(high, Reg::Phys(PReg::Rdx));
        self.alu2(AluOp::Add, high, cross[0]);
        flags[4] = self.vreg();
        self.emit(MInst::Setcc {
            cond: Cond::B,
            dst:  flags[4],
        });
        for &flag in &flags[1..5] {
            self.emit(MInst::Alu {
                op:    AluOp::Or,
                width: Width::B8,
                dst:   flags[0],
                src:   Operand::Reg(flag),
            });
        }
        self.emit(MInst::Extend {
            signed: false,
            from: Width::B8,
            to: Width::B32,
            dst,
            src: Operand::Reg(flags[0]),
        });
    }
}

/// The `i128` constant a narrower constant extends to.
pub(super) fn widen(bits: u64, from: Type, signed: bool) -> u128 {
    if signed {
        i128::from(sign_extend(bits & mask(from), from).cast_signed()).cast_unsigned()
    } else {
        u128::from(bits & mask(from))
    }
}
