//! Atomic builtin execution uses retained operation and operand contracts.
//! Read `finish_atomic` first: queries and fences are direct; object operations
//! select scalar instructions or the runtime buffer path. `objects` implements
//! atomic lvalue access and CAS retry loops, `storage` packs object values and
//! calls the buffer runtime, and `instructions` emits ordered IR operations.
//!
//! C11: §7.17.3p6, pp. 275-276; PDF pp. 293-294;
//! §7.17.7.4p2, p. 283; PDF p. 301.
//! GNU extension: <https://gcc.gnu.org/onlinedocs/gcc/_005f_005fatomic-Builtins.html>

// Object access, representation and instruction emission.
mod instructions;
mod objects;
mod storage;

use super::super::{
    FunctionLowerer,
    Item,
    LoweringError,
    Operand,
    Place,
    ssa::Op,
    types,
};
use crate::{
    builtins::{
        AtomicBuiltin,
        AtomicFamily as F,
        AtomicOperation as O,
        AtomicRmw as R,
    },
    ir::{
        Align,
        AtomicOp as A,
        InstData,
        InstFlags,
        IntCC,
        MemoryOrder as M,
        Opcode,
        Type,
        Value,
        ValueList,
    },
    translation_phases::{
        SourceVectors,
        parsing::syntax::Expression,
        semantic_analysis::{
            BuiltinCallFacts,
            TypeId,
            TypeKind,
        },
    },
};

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    pub(super) fn finish_atomic(
        &mut self,
        e: &'tu Expression<'tu>,
        instance: &'tu BuiltinCallFacts<'tu>,
        builtin: AtomicBuiltin,
        items: &[Item],
    ) -> Result<Operand, LoweringError> {
        let source = e.source_vectors;
        let order_at = |index: usize| {
            instance.operands[index]
                .integer
                .map_or(M::SeqCst, |n| M::from_c(n.value))
        };
        if matches!(builtin.operation, O::AlwaysLockFree | O::IsLockFree) {
            if let Some(value) = instance.integer {
                return Ok(Operand::Value(self.iconst(Type::I8, value.value)));
            }
            let size = self.value(items[0], source)?;
            let ty = self.draft.value_type(size);
            let mut lock_free = self.iconst(Type::I1, 0);
            for size_value in [0, 1, 2, 4, 8] {
                let n = self.iconst(ty, size_value);
                let equal = self.icmp(IntCC::Eq, size, n);
                lock_free = self.plain(Opcode::Or, lock_free, equal);
            }
            if items.len() > 1 {
                let pointer = self.value(items[1], source)?;
                let pointer = if self.draft.value_type(pointer) == Type::Ptr {
                    self.convert(Opcode::Ptrtoint, Type::I64, pointer)
                } else {
                    // The query contract accepts integer null pointer
                    // constants without recording a pointer conversion.
                    self.extend(pointer, false, Type::I64)
                };
                let size = self.extend(size, false, Type::I64);
                let one = self.iconst(Type::I64, 1);
                let mask = self.plain(Opcode::Isub, size, one);
                let low = self.plain(Opcode::And, pointer, mask);
                let zero = self.iconst(Type::I64, 0);
                let aligned = self.icmp(IntCC::Eq, low, zero);
                let empty = self.icmp(IntCC::Eq, size, zero);
                let aligned = self.plain(Opcode::Or, aligned, empty);
                lock_free = self.plain(Opcode::And, lock_free, aligned);
            }
            return Ok(Operand::Value(self.convert(
                Opcode::Zext,
                Type::I8,
                lock_free,
            )));
        }
        let mut order = if builtin.family == F::Sync {
            match builtin.operation {
                | O::Exchange => M::Acquire,
                | O::Clear => M::Release,
                | _ => M::SeqCst,
            }
        } else if builtin.operation == O::Init {
            M::Relaxed
        } else {
            order_at(builtin.count - 1 - usize::from(builtin.operation == O::Compare))
        };
        if builtin.operation == O::Compare
            && builtin.family != F::Sync
            && instance.operands[builtin.count - 1].integer.is_none()
        {
            // GCC permits dynamic orders to use seq_cst. Strengthen success
            // too, so a dynamic failure never outranks the emitted success.
            order = M::SeqCst;
        }
        if matches!(builtin.operation, O::ThreadFence | O::SignalFence) {
            _ = self.atomic_inst(
                if builtin.operation == O::ThreadFence {
                    A::ThreadFence
                } else {
                    A::SignalFence
                },
                Type::I8,
                &[],
                [order, M::Relaxed],
                Align::BYTE,
                false,
            );
            return Ok(Operand::Void);
        }
        let object = instance
            .operation_type
            .ok_or_else(|| LoweringError::missing("atomic object type is not retained", source))?;
        let value_type = self.unit.sema.types.non_atomic(object).unqualified();
        let address = self.value(items[0], source)?;
        let layout = if builtin.operation == O::TestSet {
            crate::target::Layout { size: 1, align: 1 }
        } else {
            types::layout(&self.unit.sema.types, object)
                .map_err(|kind| super::super::module_items::at(kind, source))?
        };
        if builtin.generic
            || matches!(self.repr(value_type, source)?, types::Repr::Aggregate)
            || !matches!(layout.size, 1 | 2 | 4 | 8 | 16)
        {
            return self.finish_atomic_buffer(e, instance, builtin, items, object, order);
        }
        let floating_type = self
            .repr(value_type, source)?
            .value_type()
            .filter(|ty| ty.is_float());
        if floating_type == Some(Type::F80)
            && matches!(builtin.operation, O::FetchAdd | O::FetchSub)
            || floating_type.is_some() && builtin.operation == O::FetchMinMax
        {
            let initial = self.atomic_read(address, object, value_type, source)?;
            let right = items[1];
            let place = Item {
                ty:      object,
                operand: Operand::Place(Place::Memory(address)),
            };
            let (old, updated) = self.atomic_update(place, initial, source, |this, old| {
                let a = this.value(old, source)?;
                let b = this.value(right, source)?;
                let value = if matches!(builtin.operation, O::FetchAdd | O::FetchSub) {
                    this.plain(
                        if builtin.operation == O::FetchAdd {
                            Opcode::Fadd
                        } else {
                            Opcode::Fsub
                        },
                        a,
                        b,
                    )
                } else {
                    this.atomic_float_extreme(builtin.rmw == R::Min, a, b)
                };
                Ok(Item {
                    operand: Operand::Value(value),
                    ty:      value_type,
                })
            })?;
            return Ok(if builtin.return_new {
                updated.operand
            } else {
                old.operand
            });
        }
        let bits_ty = types::int_type(u32::try_from(layout.size * 8).expect("small atomic width"));
        let align = self.place_alignment(address, types::align(layout));
        let volatile = types::is_volatile(object);
        if builtin.operation == O::Init {
            let bits = self.atomic_item_bits(items[1], bits_ty, source)?;
            self.plain_store_bits(bits, address, align, volatile);
            return Ok(Operand::Void);
        }
        if builtin.operation == O::Load {
            let bits = self
                .atomic_inst(
                    A::Load,
                    bits_ty,
                    &[address],
                    [order, M::Relaxed],
                    align,
                    volatile,
                )
                .expect("atomic load result");
            if builtin.generic {
                let destination = self.value(items[1], source)?;
                self.store_atomic_buffer(bits, destination, value_type, source)?;
                return Ok(Operand::Void);
            }
            return self
                .atomic_bits_item(bits, value_type, source)
                .map(|item| item.operand);
        }
        if matches!(builtin.operation, O::Compare | O::ValueCompare) {
            let expected_address = if builtin.family == F::Sync {
                None
            } else {
                Some(self.value(items[1], source)?)
            };
            let expected = if let Some(expected) = expected_address {
                self.load_atomic_buffer(expected, value_type, bits_ty, source)?
            } else {
                self.atomic_item_bits(items[1], bits_ty, source)?
            };
            let desired = if builtin.generic {
                let desired = self.value(items[2], source)?;
                self.load_atomic_buffer(desired, value_type, bits_ty, source)?
            } else {
                self.atomic_item_bits(items[2], bits_ty, source)?
            };
            let failure = if builtin.family == F::Sync {
                M::SeqCst
            } else {
                order_at(builtin.count - 1)
            };
            let observed = self
                .atomic_inst(
                    A::CompareExchange,
                    bits_ty,
                    &[address, desired, expected],
                    [order, failure],
                    align,
                    volatile,
                )
                .expect("compare result");
            if builtin.operation == O::ValueCompare {
                return self
                    .atomic_bits_item(observed, value_type, source)
                    .map(|item| item.operand);
            }
            let success = self.icmp(IntCC::Eq, observed, expected);
            if let Some(expected_address) = expected_address {
                let failed = self.draft.create_block(&[]);
                let continuation = self.draft.create_block(&[]);
                self.draft.brif(success, continuation, failed);
                self.draft.seal(failed);
                self.draft.switch_to(failed);
                self.store_atomic_buffer(observed, expected_address, value_type, source)?;
                self.draft.jump(continuation, &[]);
                self.draft.seal(continuation);
                self.draft.switch_to(continuation);
            }
            return Ok(Operand::Value(self.convert(
                Opcode::Zext,
                Type::I8,
                success,
            )));
        }
        let mut operand = if matches!(builtin.operation, O::TestSet | O::Clear) {
            self.iconst(bits_ty, i128::from(builtin.operation == O::TestSet))
        } else if builtin.generic {
            let buffer = self.value(items[1], source)?;
            self.load_atomic_buffer(buffer, value_type, bits_ty, source)?
        } else if matches!(builtin.operation, O::FetchAdd | O::FetchSub)
            && matches!(self.unit.sema.types.kind(value_type), TypeKind::Pointer(_))
        {
            let value = self.value(items[1], source)?;
            let value = if self.draft.value_type(value) == Type::Ptr {
                self.convert(Opcode::Ptrtoint, bits_ty, value)
            } else {
                self.extend(value, true, bits_ty)
            };
            if builtin.family == F::C11 {
                let target =
                    types::target_type(&self.unit.sema.types, value_type).expect("pointer target");
                let size = self.runtime_size(target, source)?;
                let size = self.extend(size, false, bits_ty);
                self.plain(Opcode::Imul, value, size)
            } else {
                value
            }
        } else {
            self.atomic_item_bits(items[1], bits_ty, source)?
        };
        let mut op = match builtin.operation {
            | O::Store | O::Clear => A::Store,
            | O::Exchange | O::TestSet => A::Exchange,
            | _ => match builtin.rmw {
                | R::Exchange => A::Exchange,
                | R::Add => A::Add,
                | R::Sub => A::Sub,
                | R::And => A::And,
                | R::Or => A::Or,
                | R::Xor => A::Xor,
                | R::Nand => A::Nand,
                | R::Min =>
                    if self.repr(value_type, source)?.signed() {
                        A::Min
                    } else {
                        A::Umin
                    },
                | R::Max =>
                    if self.repr(value_type, source)?.signed() {
                        A::Max
                    } else {
                        A::Umax
                    },
            },
        };
        let floating = self
            .repr(value_type, source)?
            .value_type()
            .filter(|ty| ty.is_float());
        let atomic_ty = if let Some(float_ty) = floating {
            if matches!(
                builtin.operation,
                O::FetchAdd | O::FetchSub | O::FetchMinMax
            ) {
                op = match builtin.rmw {
                    | R::Add => A::Fadd,
                    | R::Sub => A::Fsub,
                    | R::Min => A::Fmin,
                    | _ => A::Fmax,
                };
                operand = self.convert(Opcode::Bitcast, float_ty, operand);
                float_ty
            } else {
                bits_ty
            }
        } else {
            bits_ty
        };
        let result = self.atomic_inst(
            op,
            atomic_ty,
            &[address, operand],
            [order, M::Relaxed],
            align,
            volatile,
        );
        if matches!(builtin.operation, O::Store | O::Clear) {
            return Ok(Operand::Void);
        }
        let old = result.expect("atomic RMW result");
        if builtin.generic {
            let destination = self.value(items[2], source)?;
            self.store_atomic_buffer(old, destination, value_type, source)?;
            return Ok(Operand::Void);
        }
        if builtin.operation == O::TestSet {
            let zero = self.iconst(bits_ty, 0);
            let nonzero = self.icmp(IntCC::Ne, old, zero);
            return Ok(Operand::Value(self.convert(
                Opcode::Zext,
                Type::I8,
                nonzero,
            )));
        }
        let value = if builtin.return_new {
            self.atomic_updated(op, old, operand)
        } else {
            old
        };
        let value = if atomic_ty.is_float() {
            self.convert(Opcode::Bitcast, bits_ty, value)
        } else {
            value
        };
        self.atomic_bits_item(value, value_type, source)
            .map(|item| item.operand)
    }
}
