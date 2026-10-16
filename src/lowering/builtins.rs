//! Scalar GCC intrinsics scheduled on the lowering work stack. No intrinsic
//! is emitted as a runtime symbol, and operands are evaluated exactly once.
//! GNU extension: GCC "Other Builtins", "Bit Operation Builtins",
//! "Integer Overflow Builtins", "Object Size Checking", "Stack Allocation".
//! <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
//! <https://gcc.gnu.org/onlinedocs/gcc/Bit-Operation-Builtins.html>
//! <https://gcc.gnu.org/onlinedocs/gcc/Integer-Overflow-Builtins.html>
//! <https://gcc.gnu.org/onlinedocs/gcc/Object-Size-Checking.html>
//! <https://gcc.gnu.org/onlinedocs/gcc/Stack-Allocation.html>

mod floating;

use super::{
    FunctionLowerer,
    Item,
    LoweringError,
    Operand,
    Task,
    ssa::{
        Op,
        Terminator,
    },
    types,
};
use crate::{
    builtins::{
        ArgumentEvaluation,
        BitOperation,
        BuiltinOperation as CommonBuiltin,
        OverflowOperation,
        VaOperation,
    },
    ir::{
        InstData,
        InstFlags,
        IntCC,
        Opcode,
        Type,
        Value,
    },
    translation_phases::{
        parsing::syntax::Expression,
        semantic_analysis::{
            BuiltinCallFacts,
            BuiltinOperand,
            TypeId,
        },
    },
};

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    /// Only type-generic/vector special forms retain the explicit M6 syntax
    /// bridge. C99: §7.15.1, pp. 249-251; PDF pp. 261-263.
    pub(super) fn schedule_keyword(
        &mut self,
        e: &'tu Expression<'tu>,
        b: &'tu crate::translation_phases::parsing::Builtin<'tu>,
    ) -> Result<(), LoweringError> {
        use crate::translation_phases::preprocessing::KeywordTokenType as K;
        if b.keyword == K::BuiltinChooseExpr {
            let selected = self.info(e)?.selected_expression.ok_or_else(|| {
                LoweringError::missing("choose_expr has no selected operand", e.source_vectors)
            })?;
            self.tasks.push(Task::EvaluateRaw(selected));
            return Ok(());
        }
        if matches!(
            b.keyword,
            K::BuiltinVaStart
                | K::BuiltinVaEnd
                | K::BuiltinVaCopy
                | K::BuiltinVaArg
                | K::BuiltinOffsetof
        ) {
            let instance = self.unit.sema.builtin_call_facts(e).ok_or_else(|| {
                LoweringError::missing("checked keyword builtin is not retained", e.source_vectors)
            })?;
            return self.schedule_instance(e, instance);
        }
        Err(LoweringError::unsupported(
            super::Construct::Builtin,
            e.source_vectors,
        ))
    }

    /// The IR takes the address of `va_list` storage on both target families.
    /// Its pointer representation on Win64 is therefore not read as a value.
    /// C99: §7.15.1, pp. 249-251; PDF pp. 261-263.
    fn finish_varargs(
        &mut self,
        e: &'tu Expression<'tu>,
        instance: &'tu BuiltinCallFacts<'tu>,
    ) -> Result<(), LoweringError> {
        let CommonBuiltin::Varargs(operation) = instance.operation else {
            return Err(LoweringError::missing(
                "varargs operation is absent",
                e.source_vectors,
            ));
        };
        let source = e.source_vectors;
        let second = (operation == VaOperation::Copy).then(|| self.pop());
        let first = self.pop();
        let first = self.va_list_address(first, &instance.operands[0], source)?;
        let ty = instance.result_type;
        let operand = if operation == VaOperation::Arg {
            if types::repr(&self.unit.sema.types, ty).is_ok_and(|r| r == types::Repr::Aggregate) {
                let signature = self
                    .unit
                    .signature_of([ty], ty, false, self.scratch)
                    .map_err(|kind| super::module_items::at(kind, source))?;
                let signature = crate::ir::Signature {
                    params: &signature.params[1..],
                    aggregates: &[crate::ir::AggregateParam {
                        index: 0,
                        abi:   signature.param_abi(1).expect("aggregate argument"),
                    }],
                    noundef: &[],
                    ..signature
                };
                let sig = self.unit.module.intern(signature);
                let address = self.inst(InstData::VaArgAggregate { sig, list: first }, Type::Ptr);
                self.push_item(Operand::Aggregate(address), ty);
                return Ok(());
            }
            let result_type = self.value_type(ty, source)?;
            Operand::Value(self.convert(Opcode::VaArg, result_type, first))
        } else {
            let data = if let Some(second) = second {
                InstData::Binary {
                    opcode: Opcode::VaCopy,
                    ty:     Type::Ptr,
                    flags:  InstFlags::empty(),
                    args:   [
                        first,
                        self.va_list_address(second, &instance.operands[1], source)?,
                    ],
                }
            } else {
                InstData::Unary {
                    opcode: if operation == VaOperation::Start {
                        Opcode::VaStart
                    } else {
                        Opcode::VaEnd
                    },
                    ty:     Type::Ptr,
                    arg:    first,
                }
            };
            _ = self.draft.push(Op::Inst(data), None);
            Operand::Void
        };
        self.push_item(operand, ty);
        Ok(())
    }

    /// An array parameter has already decayed to the caller's list address.
    /// Win64's pointer list instead requires the address of its cursor.
    /// C99: §7.15.1p1, p. 249; PDF p. 261.
    fn va_list_address(
        &mut self,
        item: Item,
        operand: &BuiltinOperand<'tu>,
        source: crate::translation_phases::SourceVectors,
    ) -> Result<Value, LoweringError> {
        if operand.evaluation == ArgumentEvaluation::AdjustedPointer {
            let item = if let Operand::Place(place) = item.operand {
                self.read(place, item.ty, item.ty, source)?
            } else {
                item
            };
            return self.value(item, source);
        }
        match item.operand {
            | Operand::Place(super::Place::Memory(value)) | Operand::Value(value) => Ok(value),
            | _ => Err(LoweringError::missing("va_list has no storage", source)),
        }
    }

    /// Dispatch only the semantic instance; M6 families have no instance yet.
    pub(super) fn schedule_builtin(
        &mut self,
        e: &'tu Expression<'tu>,
    ) -> Result<bool, LoweringError> {
        let call = self.unit.sema.call_facts(e).ok_or_else(|| {
            LoweringError::missing("call facts are not retained", e.source_vectors)
        })?;
        let crate::translation_phases::semantic_analysis::Callee::Builtin {
            binding, instance, ..
        } = call.callee
        else {
            return Ok(false);
        };
        let Some(instance) = instance else {
            if binding.is_some_and(|binding| {
                self.unit.sema.bindings[binding]
                    .origin
                    .descriptor()
                    .is_some()
            }) {
                return Err(LoweringError::missing(
                    "checked named builtin is not retained",
                    e.source_vectors,
                ));
            }
            return Ok(false); // Explicit temporary M6/opaque-reserved bridge.
        };
        if instance.operation == CommonBuiltin::Library {
            return Ok(false);
        }
        self.schedule_instance(e, instance)?;
        Ok(true)
    }

    fn schedule_instance(
        &mut self,
        e: &'tu Expression<'tu>,
        instance: &'tu BuiltinCallFacts<'tu>,
    ) -> Result<(), LoweringError> {
        let descriptor = instance.descriptor.descriptor();
        if instance.operation != descriptor.operation || instance.effects != descriptor.effects {
            return Err(LoweringError::missing(
                "checked builtin contract is inconsistent",
                e.source_vectors,
            ));
        }
        if instance.operation == CommonBuiltin::Offsetof {
            return Err(LoweringError::unsupported(
                super::Construct::Builtin,
                e.source_vectors,
            ));
        }
        if !(descriptor.arity.0..=descriptor.arity.1).contains(&instance.operands.len()) {
            return Err(LoweringError::missing(
                "checked builtin operands are not retained",
                e.source_vectors,
            ));
        }
        self.tasks.push(Task::FinishBuiltin(e, instance));
        for operand in instance.operands.iter().rev() {
            let Some(expression) = operand.expression else {
                if operand.evaluation != ArgumentEvaluation::Type {
                    return Err(LoweringError::missing(
                        "a checked builtin operand is not retained",
                        e.source_vectors,
                    ));
                }
                continue;
            };
            match operand.evaluation {
                | ArgumentEvaluation::Value => self.tasks.push(Task::Evaluate(expression)),
                | ArgumentEvaluation::StorageAddress | ArgumentEvaluation::AdjustedPointer =>
                    self.tasks.push(Task::EvaluateRaw(expression)),
                | ArgumentEvaluation::EffectsOnly => self.tasks.push(Task::BuiltinEffects(
                    expression,
                    crate::translation_phases::semantic_analysis::EffectContext::Value,
                )),
                | ArgumentEvaluation::Unevaluated | ArgumentEvaluation::Type => {},
            }
        }
        Ok(())
    }

    /// Completes one intrinsic from its evaluated scalar operands.
    /// GNU extension: GCC manuals linked in this module's documentation.
    pub(super) fn finish_builtin(
        &mut self,
        e: &'tu Expression<'tu>,
        instance: &'tu BuiltinCallFacts<'tu>,
    ) -> Result<(), LoweringError> {
        let builtin = instance.operation;
        if matches!(builtin, CommonBuiltin::Varargs(_)) {
            return self.finish_varargs(e, instance);
        }
        let count = instance
            .operands
            .iter()
            .filter(|operand| {
                matches!(
                    operand.evaluation,
                    ArgumentEvaluation::Value
                        | ArgumentEvaluation::StorageAddress
                        | ArgumentEvaluation::AdjustedPointer
                )
            })
            .count();
        let start = self.operands.len() - count;
        // `__builtin_fpclassify` has the most operands, six.
        let mut items = [Item {
            operand: Operand::Void,
            ty:      instance.result_type,
        }; 6];
        items[..count].copy_from_slice(&self.operands[start..]);
        self.operands.truncate(start);
        let source = e.source_vectors;
        let result = match builtin {
            | CommonBuiltin::Varargs(_) | CommonBuiltin::Library | CommonBuiltin::Offsetof =>
                return Err(LoweringError::missing(
                    "varargs must use keyword scheduling",
                    source,
                )),
            | CommonBuiltin::Complex =>
                return Err(LoweringError::unsupported(
                    super::Construct::ComplexArithmetic,
                    source,
                )),
            | CommonBuiltin::Constant => {
                let value = self.iconst(
                    Type::I32,
                    instance.integer.expect("typed constant query").value,
                );
                Operand::Value(value)
            },
            | CommonBuiltin::ObjectSize(_) => {
                // Sema folded the query using the converted int mode.
                // C99 §6.5.2.2p7, p. 72; PDF p. 84.
                let value = instance.integer.ok_or_else(|| {
                    LoweringError::missing("object size query has no folded value", source)
                })?;
                let ty = self.value_type(instance.result_type, source)?;
                Operand::Value(self.iconst(ty, value.value))
            },
            | CommonBuiltin::Expect(_)
            | CommonBuiltin::Aligned
            | CommonBuiltin::ExtractReturnAddress => items[0].operand,
            // Only the current frame has a defined address everywhere; GCC
            // calls outer levels unpredictable. GNU extension: GCC "Return
            // Address". <https://gcc.gnu.org/onlinedocs/gcc/Return-Address.html>
            | CommonBuiltin::Address(return_address) => {
                let level = match instance.operands[0].expression {
                    | Some(level) => self.info(level)?.integer.map(|level| level.value),
                    | None => None,
                };
                if level != Some(0) {
                    return Err(LoweringError::unsupported(
                        super::Construct::Builtin,
                        source,
                    ));
                }
                Operand::Value(self.nullary(if return_address {
                    Opcode::ReturnAddress
                } else {
                    Opcode::FrameAddress
                }))
            },
            // Debug-information annotations do not execute.
            | CommonBuiltin::Prefetch | CommonBuiltin::Annotation => Operand::Void,
            // x86 `__rdtsc` reads the time-stamp counter, as Clang's
            // `llvm.x86.rdtsc`.
            | CommonBuiltin::ReadTimestamp => Operand::Value(self.inst(
                InstData::Nullary {
                    opcode: Opcode::ReadTimestamp,
                    ty:     Type::I64,
                },
                Type::I64,
            )),
            // Microsoft `__debugbreak` traps to a debugger and may continue.
            | CommonBuiltin::DebugTrap => {
                _ = self.draft.push(
                    Op::Inst(InstData::Nullary {
                        opcode: Opcode::DebugTrap,
                        ty:     Type::Ptr,
                    }),
                    None,
                );
                Operand::Void
            },
            | CommonBuiltin::Unreachable | CommonBuiltin::Trap => {
                if builtin == CommonBuiltin::Trap {
                    _ = self.draft.push(
                        Op::Inst(InstData::Nullary {
                            opcode: Opcode::Trap,
                            ty:     Type::Ptr,
                        }),
                        None,
                    );
                }
                self.draft
                    .terminate_and_continue_dead(Terminator::Unreachable);
                Operand::Void
            },
            | CommonBuiltin::Alloca => {
                let size = self.value(items[0], source)?;
                let size = self.extend(size, false, Type::I64);
                Operand::Value(self.dynamic_object(size, None))
            },
            | CommonBuiltin::Bswap(_) => {
                let arg = self.value(items[0], source)?;
                let ty = self.draft.value_type(arg);
                Operand::Value(self.convert(Opcode::Bswap, ty, arg))
            },
            | CommonBuiltin::Bit(op, _) => {
                let arg = self.value(items[0], source)?;
                let ty = self.draft.value_type(arg);
                let count = match op {
                    | BitOperation::Clz => self.convert(Opcode::Clz, ty, arg),
                    | BitOperation::Ctz => self.convert(Opcode::Ctz, ty, arg),
                    | BitOperation::Popcount => self.convert(Opcode::Popcount, ty, arg),
                    | BitOperation::Parity => {
                        let count = self.convert(Opcode::Popcount, ty, arg);
                        let one = self.iconst(ty, 1);
                        self.plain(Opcode::And, count, one)
                    },
                    | BitOperation::Ffs => {
                        let count = self.convert(Opcode::CtzDefined, ty, arg);
                        let one = self.iconst(ty, 1);
                        let next = self.plain(Opcode::Iadd, count, one);
                        let zero = self.iconst(ty, 0);
                        let nonzero = self.icmp(IntCC::Ne, arg, zero);
                        self.choose(nonzero, next, zero)
                    },
                    | BitOperation::Clrsb => {
                        let shift = self.iconst(ty, i128::from(ty.bits() - 1));
                        let sign = self.plain(Opcode::Ashr, arg, shift);
                        let bits = self.plain(Opcode::Xor, arg, sign);
                        let count = self.convert(Opcode::ClzDefined, ty, bits);
                        let one = self.iconst(ty, 1);
                        self.plain(Opcode::Isub, count, one)
                    },
                };
                Operand::Value(self.extend(count, false, Type::I32))
            },
            | CommonBuiltin::Float(operation, _) => self.floating_builtin(
                operation,
                &items[..count],
                instance.operation_type.ok_or_else(|| {
                    LoweringError::missing("a floating builtin has no operand type", source)
                })?,
                source,
            )?,
            | CommonBuiltin::Overflow(op, _, predicate) => {
                let destination = instance.operation_type.ok_or_else(|| {
                    LoweringError::missing("overflow destination is absent", source)
                })?;
                let width = if predicate {
                    instance.operands[2].bit_field
                } else {
                    None
                };
                let (result, overflow) =
                    self.overflow(op, items[0], items[1], destination, width, source)?;
                if !predicate {
                    let address = self.value(items[2], source)?;
                    self.store(result, address, destination, source)?;
                }
                Operand::Value(self.extend(overflow, false, Type::I8))
            },
        };
        self.push_item(result, instance.result_type);
        Ok(())
    }

    /// Infinite precision arithmetic uses unsigned magnitudes and sign bits.
    /// A carry means the magnitude exceeds 128 bits and cannot fit any C type.
    /// The wrapped result is nevertheless stored on overflow, as GCC requires.
    /// GNU extension: GCC "Integer Overflow Builtins".
    fn overflow(
        &mut self,
        op: OverflowOperation,
        a: Item,
        b: Item,
        destination: TypeId,
        bit_width: Option<u32>,
        source: crate::translation_phases::SourceVectors,
    ) -> Result<(Value, Value), LoweringError> {
        let (an, av) = self.magnitude(a, source)?;
        let (mut bn, bv) = self.magnitude(b, source)?;
        let zero = self.iconst(Type::I128, 0);
        let false_value = self.iconst(Type::I1, 0);
        let true_value = self.iconst(Type::I1, 1);
        if op == OverflowOperation::Sub {
            bn = self.plain(Opcode::Xor, bn, true_value);
        }
        let (negative, magnitude, carry) = if op == OverflowOperation::Mul {
            let sign = self.plain(Opcode::Xor, an, bn);
            let value = self.plain(Opcode::Imul, av, bv);
            let carry = self.overflow_inst(Opcode::UmulOverflow, av, bv);
            (sign, value, carry)
        } else {
            let same = self.icmp(IntCC::Eq, an, bn);
            let greater = self.icmp(IntCC::Uge, av, bv);
            let sum = self.plain(Opcode::Iadd, av, bv);
            let sum_carry = self.overflow_inst(Opcode::UaddOverflow, av, bv);
            let ab = self.plain(Opcode::Isub, av, bv);
            let ba = self.plain(Opcode::Isub, bv, av);
            let difference = self.choose(greater, ab, ba);
            let difference_sign = self.choose(greater, an, bn);
            (
                self.choose(same, an, difference_sign),
                self.choose(same, sum, difference),
                self.choose(same, sum_carry, false_value),
            )
        };
        let repr = self.repr(destination, source)?;
        let ty = self.value_type(destination, source)?;
        let bits = bit_width.unwrap_or(ty.bits());
        let limit = if repr.signed() {
            let negative_limit = self.iconst(Type::I128, i128::MIN >> (128 - bits));
            // Use unsigned magnitude, not the negative two's-complement limit.
            let negative_limit = self.plain(Opcode::Isub, zero, negative_limit);
            let one = self.iconst(Type::I128, 1);
            let positive_limit = self.plain(Opcode::Isub, negative_limit, one);
            self.choose(negative, negative_limit, positive_limit)
        } else {
            self.iconst(
                Type::I128,
                if bits == 128 {
                    -1
                } else {
                    i128::from_le_bytes((u128::MAX >> (128 - bits)).to_le_bytes())
                },
            )
        };
        let too_large = self.icmp(IntCC::Ugt, magnitude, limit);
        let mut overflow = self.plain(Opcode::Or, carry, too_large);
        if !repr.signed() {
            let nonzero = self.icmp(IntCC::Ne, magnitude, zero);
            let negative_nonzero = self.plain(Opcode::And, negative, nonzero);
            overflow = self.plain(Opcode::Or, overflow, negative_nonzero);
        }
        let negated = self.plain(Opcode::Isub, zero, magnitude);
        let value = self.choose(negative, negated, magnitude);
        Ok((self.extend(value, false, ty), overflow))
    }

    fn magnitude(
        &mut self,
        item: Item,
        source: crate::translation_phases::SourceVectors,
    ) -> Result<(Value, Value), LoweringError> {
        let signed = self.repr(item.ty, source)?.signed();
        let value = self.value(item, source)?;
        let value = self.extend(value, signed, Type::I128);
        let zero = self.iconst(Type::I128, 0);
        let negative = if signed {
            self.icmp(IntCC::Slt, value, zero)
        } else {
            self.iconst(Type::I1, 0)
        };
        let negated = self.plain(Opcode::Isub, zero, value);
        Ok((negative, self.choose(negative, negated, value)))
    }

    fn overflow_inst(&mut self, opcode: Opcode, a: Value, b: Value) -> Value {
        self.inst(
            InstData::Binary {
                opcode,
                ty: self.draft.value_type(a),
                flags: InstFlags::empty(),
                args: [a, b],
            },
            Type::I1,
        )
    }

    fn plain(&mut self, opcode: Opcode, a: Value, b: Value) -> Value {
        self.binary_inst(opcode, InstFlags::empty(), a, b)
    }

    fn choose(&mut self, condition: Value, yes: Value, no: Value) -> Value {
        let ty = self.draft.value_type(yes);
        self.inst(
            InstData::Select {
                ty,
                args: [condition, yes, no],
            },
            ty,
        )
    }
}
