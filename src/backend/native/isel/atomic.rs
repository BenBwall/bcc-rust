//! Atomic operations: `_Atomic` objects, `<stdatomic.h>` and the GNU
//! `__atomic` and `__sync` builtins, which lowering turns into `atomic`
//! instructions.
//!
//! x86-64 orders memory almost sequentially already: every load acts as an
//! acquire, every store as a release, and only a store followed by a load
//! of another location may be reordered (Intel SDM Volume 3A, "Memory
//! Ordering in P6 and More Recent Processor Families"). Locked
//! instructions and `xchg` with memory, which is locked without a prefix,
//! are full barriers. The standard mapping of the C11 orders to x86-64
//! (Sewell and others, "C/C++11 mappings to processors", the x86 column,
//! <https://www.cl.cam.ac.uk/~pes20/cpp/cpp0xmappings.html>; GCC and LLVM
//! emit the same) follows:
//!
//! | operation | x86-64 |
//! | --- | --- |
//! | load, any order | `mov` |
//! | store, relaxed or release | `mov` |
//! | store, `seq_cst` | `xchg` |
//! | exchange | `xchg` |
//! | compare-exchange, strong or weak | `lock cmpxchg` |
//! | fetch-add, fetch-sub | `lock xadd` |
//! | add, sub, and, or, xor whose old value is unused | `lock add`, `sub`, `and`, `or`, `xor` |
//! | and, or, xor, nand, min, max, floating add and sub | a `lock cmpxchg` loop |
//! | `seq_cst` thread fence | `mfence` |
//! | other fences, signal fences | nothing |
//!
//! Sequential consistency puts the full barrier on the store, so loads stay
//! plain `mov`s. A fence weaker than `seq_cst` only forbids reordering that
//! x86 does not do, and the selector moves no memory access across another
//! instruction, so it emits nothing.
//!
//! `i128` operations call libatomic's `__atomic_*_16` routines and the
//! byte-buffer operations its generic `__atomic_load`, `__atomic_store`,
//! `__atomic_exchange` and `__atomic_compare_exchange`, as GCC does and as
//! LLVM does without `cmpxchg16b`, which the baseline x86-64 lacks; the
//! driver then links `-latomic`. The support check refuses what neither
//! form covers: `i128` minimum and maximum, floating minimum and maximum,
//! and objects aligned below their size.
//!
//! C11: §7.17.3, pp. 275-278; PDF pp. 293-296 (orders); §7.17.4,
//! pp. 278-279; PDF pp. 296-297 (fences); §7.17.7, pp. 282-285; PDF
//! pp. 300-303 (operations); §7.17.8, pp. 285-286; PDF pp. 303-304
//! (`atomic_flag`). Minimum, maximum and nand are GNU extensions (GCC
//! manual, "Built-in Functions for Memory Model Aware Atomic Operations").

use super::{
    Selector,
    calls::{
        LibArg,
        LibResult,
        Passed,
    },
    width,
};
use crate::{
    backend::native::mir::{
        AluOp,
        Callee,
        Cond,
        MInst,
        Mem,
        Operand,
        PReg,
        Reg,
        RegClass,
        SymbolRef,
        UnaryOp,
        Width,
        XOp,
    },
    ir::{
        AtomicOp,
        Entity,
        Inst,
        InstData,
        MemoryOrder,
        Type,
        Value,
    },
    util::bump::ArenaVec,
};

impl Selector<'_, '_, '_> {
    /// Selects one `atomic` instruction.
    pub(super) fn atomic(&mut self, inst: Inst, data: InstData) {
        let InstData::Atomic {
            op,
            ty,
            order,
            failure,
            args,
            ..
        } = data
        else {
            unreachable!("an atomic instruction");
        };
        let args = self.body.value_list(args);
        if op.is_fence() {
            if op == AtomicOp::ThreadFence && order == MemoryOrder::SeqCst {
                self.emit(MInst::Mfence);
            }
            return;
        }
        if op.is_bytes() {
            self.atomic_buffer(inst, op, [order, failure], args);
            return;
        }
        if ty == Type::I128 {
            self.wide_atomic(inst, op, [order, failure], args);
            return;
        }
        let result = self.body.inst_result(inst);
        let width = width(ty);
        let mem = self.address(args[0]);
        let unused = result.is_none_or(|value| self.uses[value.index()] == 0);
        match op {
            | AtomicOp::Load => {
                let dst = self.result_reg(result);
                self.load(ty, dst, mem);
            },
            | AtomicOp::Store if order == MemoryOrder::SeqCst => {
                let value = self.reg(args[1]);
                let swapped = self.vreg();
                self.copy(swapped, value);
                self.emit(MInst::Xchg {
                    width,
                    reg: swapped,
                    mem,
                });
            },
            | AtomicOp::Store => {
                let src = self.operand(args[1], width);
                self.emit(MInst::Store {
                    width,
                    dst: mem,
                    src,
                });
            },
            | AtomicOp::Exchange => {
                let dst = self.result_reg(result);
                let value = self.reg(args[1]);
                self.copy(dst, value);
                self.emit(MInst::Xchg {
                    width,
                    reg: dst,
                    mem,
                });
            },
            | AtomicOp::CompareExchange => {
                let desired = self.reg(args[1]);
                let expected = self.reg(args[2]);
                let rax = Reg::Phys(PReg::Rax);
                self.copy(rax, expected);
                self.emit(MInst::LockCmpxchg {
                    width,
                    mem,
                    src: desired,
                });
                let dst = self.result_reg(result);
                self.copy(dst, rax);
            },
            | AtomicOp::Add | AtomicOp::Sub | AtomicOp::And | AtomicOp::Or | AtomicOp::Xor
                if unused =>
            {
                let op = match op {
                    | AtomicOp::Add => AluOp::Add,
                    | AtomicOp::Sub => AluOp::Sub,
                    | AtomicOp::And => AluOp::And,
                    | AtomicOp::Or => AluOp::Or,
                    | _ => AluOp::Xor,
                };
                let src = self.operand(args[1], width);
                self.emit(MInst::LockAlu {
                    op,
                    width,
                    mem,
                    src,
                });
            },
            | AtomicOp::Add | AtomicOp::Sub => {
                let dst = self.result_reg(result);
                let value = self.reg(args[1]);
                self.copy(dst, value);
                if op == AtomicOp::Sub {
                    self.emit(MInst::Unary {
                        op: UnaryOp::Neg,
                        width,
                        dst,
                    });
                }
                self.emit(MInst::LockXadd {
                    width,
                    reg: dst,
                    mem,
                });
            },
            | _ => {
                let dst = self.result_reg(result);
                self.compare_exchange_loop(op, ty, dst, mem, args[1]);
            },
        }
    }

    /// A read-modify-write x86 has no instruction for: the new value is
    /// computed from the old one and stored with `lock cmpxchg`, which
    /// fails, and loads the current value into `%rax`, if another thread
    /// changed the object in between; the loop then tries again. `dst`
    /// receives the old value.
    fn compare_exchange_loop(&mut self, op: AtomicOp, ty: Type, dst: Reg, mem: Mem, value: Value) {
        let bits = match ty {
            | Type::F32 => Type::I32,
            | Type::F64 => Type::I64,
            | _ => ty,
        };
        let float = width(ty);
        let width = width(bits);
        // Minimum and maximum compare at 32 bits at least, which `cmov`
        // needs, after extending narrow operands by their signedness.
        let signed = matches!(op, AtomicOp::Min | AtomicOp::Max);
        let compare = if ty == Type::I64 {
            Width::B64
        } else {
            Width::B32
        };
        let operand = match op {
            | AtomicOp::Min | AtomicOp::Max | AtomicOp::Umin | AtomicOp::Umax =>
                self.extended(value, signed),
            | _ => self.reg(value),
        };
        let old = self.vreg();
        self.load(bits, old, mem);
        let retry = self.func.block(self.arena);
        let done = self.func.block(self.arena);
        self.emit(MInst::Jmp { target: retry });
        self.current = retry;
        let new = self.vreg();
        match op {
            | AtomicOp::And | AtomicOp::Or | AtomicOp::Xor | AtomicOp::Nand => {
                let alu = match op {
                    | AtomicOp::Or => AluOp::Or,
                    | AtomicOp::Xor => AluOp::Xor,
                    | _ => AluOp::And,
                };
                self.copy(new, old);
                self.emit(MInst::Alu {
                    op: alu,
                    width,
                    dst: new,
                    src: Operand::Reg(operand),
                });
                if op == AtomicOp::Nand {
                    self.emit(MInst::Alu {
                        op: AluOp::Xor,
                        width,
                        dst: new,
                        src: Operand::Imm(-1),
                    });
                }
            },
            | AtomicOp::Min | AtomicOp::Max | AtomicOp::Umin | AtomicOp::Umax => {
                let current = if ty.bits() < 32 {
                    let extended = self.vreg();
                    self.emit(MInst::Extend {
                        signed,
                        from: width,
                        to: Width::B32,
                        dst: extended,
                        src: Operand::Reg(old),
                    });
                    extended
                } else {
                    old
                };
                // Keep the old value where it already is the minimum or
                // maximum.
                let keep = match op {
                    | AtomicOp::Min => Cond::L,
                    | AtomicOp::Max => Cond::G,
                    | AtomicOp::Umin => Cond::B,
                    | _ => Cond::A,
                };
                self.copy(new, operand);
                self.emit(MInst::Cmp {
                    width: compare,
                    lhs:   current,
                    rhs:   Operand::Reg(operand),
                });
                self.emit(MInst::Cmov {
                    cond:  keep,
                    width: compare,
                    dst:   new,
                    src:   current,
                });
            },
            | _ => {
                // `fadd` and `fsub` in an SSE register.
                let sum = Reg::Virt(self.func.vreg_of(RegClass::Xmm));
                self.emit(MInst::MovGx {
                    width: float,
                    dst:   sum,
                    src:   old,
                });
                self.emit(MInst::XAlu {
                    op:    if op == AtomicOp::Fadd {
                        XOp::Add
                    } else {
                        XOp::Sub
                    },
                    width: float,
                    dst:   sum,
                    src:   Operand::Reg(operand),
                });
                self.emit(MInst::MovGx {
                    width: float,
                    dst:   new,
                    src:   sum,
                });
            },
        }
        let rax = Reg::Phys(PReg::Rax);
        self.copy(rax, old);
        self.emit(MInst::LockCmpxchg {
            width,
            mem,
            src: new,
        });
        self.copy(old, rax);
        self.emit(MInst::Jcc {
            cond:      Cond::Ne,
            taken:     retry,
            not_taken: done,
        });
        self.current = done;
        if ty.is_float() {
            self.emit(MInst::MovGx {
                width: float,
                dst,
                src: old,
            });
        } else {
            self.copy(dst, old);
        }
    }

    /// An `i128` operation through libatomic. Its routines take the memory
    /// order as C's `int` constants and the value as other `i128`
    /// arguments; compare-exchange takes the expected value through a
    /// pointer and leaves the observed one there.
    fn wide_atomic(&mut self, inst: Inst, op: AtomicOp, orders: [MemoryOrder; 2], args: &[Value]) {
        let address = self.reg(args[0]);
        let order = self.order(orders[0]);
        let name = match op {
            | AtomicOp::Load => "__atomic_load_16",
            | AtomicOp::Store => "__atomic_store_16",
            | AtomicOp::Exchange => "__atomic_exchange_16",
            | AtomicOp::CompareExchange => "__atomic_compare_exchange_16",
            | AtomicOp::Add => "__atomic_fetch_add_16",
            | AtomicOp::Sub => "__atomic_fetch_sub_16",
            | AtomicOp::And => "__atomic_fetch_and_16",
            | AtomicOp::Or => "__atomic_fetch_or_16",
            | AtomicOp::Xor => "__atomic_fetch_xor_16",
            | AtomicOp::Nand => "__atomic_fetch_nand_16",
            | _ => unreachable!("the support check refuses i128 `{op}`"),
        };
        match op {
            | AtomicOp::Load => {
                let dst = self.result_pair(inst);
                self.libcall(
                    name,
                    &[LibArg::Gpr(address), LibArg::Gpr(order)],
                    LibResult::Wide(Type::I128, dst),
                );
            },
            | AtomicOp::Store => {
                let value = self.pair(args[1]);
                let value = self.pass_wide(Type::I128, value);
                self.call_sequence(
                    Callee::Symbol(SymbolRef::Runtime(name)),
                    &[
                        Passed::Scalar(Operand::Reg(address)),
                        value,
                        Passed::Scalar(Operand::Reg(order)),
                    ],
                    false,
                );
            },
            | AtomicOp::CompareExchange => {
                let expected = self.wide_temporary();
                self.pair_store(Type::I128, args[2], expected);
                let pointer = self.vreg();
                self.emit(MInst::Lea {
                    dst:  pointer,
                    addr: expected,
                });
                let desired = self.pair(args[1]);
                let failure = self.order(orders[1]);
                let succeeded = self.vreg();
                self.libcall(
                    name,
                    &[
                        LibArg::Gpr(address),
                        LibArg::Gpr(pointer),
                        LibArg::Wide(Type::I128, desired),
                        LibArg::Gpr(order),
                        LibArg::Gpr(failure),
                    ],
                    LibResult::Gpr(succeeded),
                );
                let dst = self.result_pair(inst);
                self.wide_load(dst, expected);
            },
            | _ => {
                let value = self.pair(args[1]);
                let dst = self.result_pair(inst);
                self.libcall(
                    name,
                    &[
                        LibArg::Gpr(address),
                        LibArg::Wide(Type::I128, value),
                        LibArg::Gpr(order),
                    ],
                    LibResult::Wide(Type::I128, dst),
                );
            },
        }
    }

    /// A byte-buffer operation through libatomic's generic routines, whose
    /// size comes first (`__atomic_load (size, object, result, order)`).
    fn atomic_buffer(
        &mut self,
        inst: Inst,
        op: AtomicOp,
        orders: [MemoryOrder; 2],
        args: &[Value],
    ) {
        let order = self.order(orders[0]);
        let size = *args.last().expect("a buffer operation has a size");
        let mut passed = ArenaVec::with_capacity_in(args.len() + 2, self.arena);
        passed.push(Passed::Scalar(self.operand(size, Width::B64)));
        for &arg in &args[..args.len() - 1] {
            passed.push(Passed::Scalar(Operand::Reg(self.reg(arg))));
        }
        passed.push(Passed::Scalar(Operand::Reg(order)));
        let name = match op {
            | AtomicOp::LoadBytes => "__atomic_load",
            | AtomicOp::StoreBytes => "__atomic_store",
            | AtomicOp::ExchangeBytes => "__atomic_exchange",
            | _ => {
                let failure = self.order(orders[1]);
                passed.push(Passed::Scalar(Operand::Reg(failure)));
                "__atomic_compare_exchange"
            },
        };
        self.call_sequence(Callee::Symbol(SymbolRef::Runtime(name)), &passed, false);
        if let Some(result) = self.body.inst_result(inst) {
            let dst = self.result_reg(Some(result));
            self.copy(dst, Reg::Phys(PReg::Rax));
        }
    }

    /// A register holding libatomic's `int` for `order`, C11's
    /// `memory_order` value (`__ATOMIC_RELAXED` 0 to `__ATOMIC_SEQ_CST` 5).
    fn order(&mut self, order: MemoryOrder) -> Reg {
        let value = match order {
            | MemoryOrder::Relaxed => 0,
            | MemoryOrder::Acquire => 2,
            | MemoryOrder::Release => 3,
            | MemoryOrder::AcqRel => 4,
            | MemoryOrder::SeqCst => 5,
        };
        let reg = self.vreg();
        self.constant(reg, value, Type::I32);
        reg
    }
}
