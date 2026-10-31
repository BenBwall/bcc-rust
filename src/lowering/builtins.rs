//! Scalar GCC intrinsics scheduled on the lowering work stack. No intrinsic
//! is emitted as a runtime symbol, and operands are evaluated exactly once.
//! GNU extension: GCC "Other Builtins", "Bit Operation Builtins",
//! "Integer Overflow Builtins", "Object Size Checking", "Stack Allocation".
//! <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
//! <https://gcc.gnu.org/onlinedocs/gcc/Bit-Operation-Builtins.html>
//! <https://gcc.gnu.org/onlinedocs/gcc/Integer-Overflow-Builtins.html>
//! <https://gcc.gnu.org/onlinedocs/gcc/Object-Size-Checking.html>
//! <https://gcc.gnu.org/onlinedocs/gcc/Stack-Allocation.html>
//!
//! `atomic.rs` lowers the atomic builtins, `floating.rs` the floating
//! classifications and `bits.rs` the type-generic bit builtins.

mod atomic;
mod bits;
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
        ComplexPart,
        MemoryBuiltin,
        OverflowOperation,
        VaOperation,
    },
    ir::{
        Align,
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
            Integer,
            TypeId,
        },
    },
};

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    /// Varargs and offsetof keyword forms execute their checked instance;
    /// `choose_expr` evaluates its retained selection. The other type-generic
    /// and vector forms are unsupported. C99: §7.15.1, pp. 249-251;
    /// PDF pp. 261-263.
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
        if self.schedule_vector_keyword(e, b) {
            return Ok(());
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
            if types::repr(&self.unit.sema.types, ty)
                .is_ok_and(|r| matches!(r, types::Repr::Aggregate | types::Repr::Complex { .. }))
                || types::limb_argument(&self.unit.sema.types, ty)
            {
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
                let operand = if matches!(
                    types::repr(&self.unit.sema.types, ty),
                    Ok(types::Repr::Complex { .. })
                ) {
                    self.load_complex(address, ty, source)?
                } else if types::limb_argument(&self.unit.sema.types, ty) {
                    Operand::Value(self.load_limbs(address, ty, source)?)
                } else {
                    Operand::Aggregate(address)
                };
                self.push_item(operand, ty);
                return Ok(());
            }
            let representation = self.repr(ty, source)?;
            let result_type = self.value_type(ty, source)?;
            let value = self.convert(Opcode::VaArg, result_type, first);
            Operand::Value(self.normalize(value, representation))
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
                    opcode: if matches!(operation, VaOperation::Start | VaOperation::StartAt) {
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

    /// The refusal of a call of a reserved `__builtin_` name semantic
    /// analysis has no contract for, whose result type it could not
    /// analyze: by name where GCC has the builtin.
    pub(super) fn opaque_builtin(&self, mut e: &'tu Expression<'tu>) -> Option<LoweringError> {
        // The call may sit under parentheses and casts, as in
        // `(void) __builtin_apply_args ()`, whose cast has no type either.
        use crate::translation_phases::parsing::syntax::ExpressionType as E;
        loop {
            match e.kind {
                | E::Parenthesized { expression } => e = expression,
                | E::Cast {
                    operand_expression, ..
                } => e = operand_expression,
                | _ => break,
            }
        }
        let crate::translation_phases::semantic_analysis::Callee::Builtin {
            binding: Some(binding),
            instance: None,
            ..
        } = self.unit.sema.call_facts(e)?.callee
        else {
            return None;
        };
        let data = self.unit.sema.bindings[binding];
        (data.origin
            == crate::translation_phases::semantic_analysis::BindingOrigin::OpaqueIntrinsic)
            .then(|| {
                let name = self.unit.context.string_cache.at(data.name.name);
                super::module_items::at(
                    super::LoweringErrorKind::UnsupportedBuiltin(crate::builtins::unimplemented(
                        name,
                    )),
                    e.source_vectors,
                )
            })
    }

    /// Dispatch only the semantic instance. Semantic-only and unsupported
    /// family operations retain none and continue as unsupported calls.
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
                    .is_some_and(|descriptor| descriptor.operation.retains_instance())
            }) {
                return Err(LoweringError::missing(
                    "checked named builtin is not retained",
                    e.source_vectors,
                ));
            }
            return Ok(false); // A family or opaque reserved builtin.
        };
        if matches!(
            instance.operation,
            CommonBuiltin::Library | CommonBuiltin::FloatConstant(_, true)
        ) {
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
        for item in &mut items[..count] {
            *item = self.settle(*item);
        }
        let source = e.source_vectors;
        let result = match builtin {
            | CommonBuiltin::Atomic(atomic) =>
                self.finish_atomic(e, instance, atomic, &items[..count])?,
            | CommonBuiltin::Varargs(_) | CommonBuiltin::Library | CommonBuiltin::Offsetof =>
                return Err(LoweringError::missing(
                    "varargs must use keyword scheduling",
                    source,
                )),
            | CommonBuiltin::Semantic(_) | CommonBuiltin::Unsupported(_) =>
                return Err(LoweringError::missing(
                    "family builtins retain no checked instance",
                    source,
                )),
            // A pack that no call passes on, or its length outside a copy.
            // GNU extension: GCC manual, "Constructing Calls".
            | CommonBuiltin::ArgumentPack(false) =>
                return Err(super::module_items::at(
                    super::LoweringErrorKind::InvalidArgumentPack(false),
                    source,
                )),
            | CommonBuiltin::ArgumentPack(true) => self.pack_length(source)?,
            | CommonBuiltin::Complex => {
                let pair = [self.value(items[0], source)?, self.value(items[1], source)?];
                self.complex_pair(pair)
            },
            | CommonBuiltin::Constant => {
                let value = self.iconst(
                    Type::I32,
                    instance.integer.expect("typed constant query").value,
                );
                Operand::Value(value)
            },
            | CommonBuiltin::ObjectSize(_) => {
                let size = self
                    .unit
                    .sema
                    .function_extensions
                    .object_size(e)
                    .ok_or_else(|| {
                        LoweringError::missing("object size facts are not retained", source)
                    })?;
                let pointer = if count == 0 {
                    None
                } else {
                    Some(self.value(items[0], source)?)
                };
                Operand::Value(self.emit_object_size(size, pointer, source)?)
            },
            | CommonBuiltin::Expect(_)
            | CommonBuiltin::Aligned
            | CommonBuiltin::ExtractReturnAddress
            | CommonBuiltin::AssocBarrier => items[0].operand,
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
            // x86 keeps instruction fetch coherent with stores, so
            // `__builtin___clear_cache` needs nothing either.
            | CommonBuiltin::Prefetch | CommonBuiltin::Annotation | CommonBuiltin::ClearCache =>
                Operand::Void,
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
            | CommonBuiltin::Setjmp => {
                let buffer = self.value(items[0], source)?;
                Operand::Value(self.builtin_setjmp(buffer))
            },
            // The value operand is the constant 1, which the resumed
            // `__builtin_setjmp` returns.
            | CommonBuiltin::Longjmp => {
                let buffer = self.value(items[0], source)?;
                _ = self.draft.push(
                    Op::Inst(InstData::Unary {
                        opcode: Opcode::SjljLongjmp,
                        ty:     Type::Ptr,
                        arg:    buffer,
                    }),
                    None,
                );
                self.draft
                    .terminate_and_continue_dead(Terminator::Unreachable);
                Operand::Void
            },
            | CommonBuiltin::Alloca => {
                let size = self.value(items[0], source)?;
                let size = self.extend(size, false, Type::I64);
                Operand::Value(self.dynamic_object(size, None))
            },
            // The alignment operand is a constant power of two in bits;
            // GCC requires at least a byte's and gives at least `alloca`'s.
            | CommonBuiltin::AllocaAligned => {
                let bits = match instance.operands[1].expression {
                    | Some(bits) => self.info(bits)?.integer.and_then(Integer::to_u64),
                    | None => None,
                }
                .ok_or_else(|| {
                    LoweringError::missing("alloca alignment is not constant", source)
                })?;
                let align = Align::from_bytes((bits / 8).max(16))
                    .ok_or_else(|| LoweringError::unsupported(super::Construct::Builtin, source))?;
                let size = self.value(items[0], source)?;
                let size = self.extend(size, false, Type::I64);
                Operand::Value(self.dynamic_object(size, Some(align)))
            },
            | CommonBuiltin::ComplexPart(part) => {
                let [real, imaginary] = self.complex_parts(items[0], source)?;
                match part {
                    | ComplexPart::Real => Operand::Value(real),
                    | ComplexPart::Imaginary => Operand::Value(imaginary),
                    | ComplexPart::Conjugate => {
                        let ty = self.draft.value_type(imaginary);
                        let negated = self.convert(Opcode::Fneg, ty, imaginary);
                        self.complex_pair([real, negated])
                    },
                }
            },
            // As Clang lowers them: a move with the pointers swapped, a fill
            // with zero, and a copy returning the end of the destination.
            | CommonBuiltin::Memory(operation) => {
                let first = self.value(items[0], source)?;
                let second = self.value(items[1], source)?;
                let count = self.value(items[count - 1], source)?;
                let count = self.extend(count, false, Type::I64);
                let (opcode, flags, args) = match operation {
                    | MemoryBuiltin::Bcopy =>
                        (Opcode::Copy, InstFlags::MAY_OVERLAP, [second, first, count]),
                    | MemoryBuiltin::Bzero => {
                        let zero = self.iconst(Type::I8, 0);
                        (Opcode::Fill, InstFlags::empty(), [first, zero, count])
                    },
                    | MemoryBuiltin::Mempcpy =>
                        (Opcode::Copy, InstFlags::empty(), [first, second, count]),
                };
                _ = self.draft.push(
                    self.memory_op(InstData::MemoryRange {
                        opcode,
                        flags,
                        align: Align::BYTE,
                        args,
                    }),
                    None,
                );
                if operation == MemoryBuiltin::Mempcpy {
                    Operand::Value(self.ptr_add(first, count))
                } else {
                    Operand::Void
                }
            },
            // GCC's and Clang's expansion: the carry out of either addition
            // (or the borrow of either subtraction), so an incoming carry
            // other than 0 or 1 counts once.
            | CommonBuiltin::Carry(subtract, _) => {
                let a = self.value(items[0], source)?;
                let b = self.value(items[1], source)?;
                let carry = self.value(items[2], source)?;
                let (opcode, compare) = if subtract {
                    (Opcode::Isub, IntCC::Ugt)
                } else {
                    (Opcode::Iadd, IntCC::Ult)
                };
                let first = self.plain(opcode, a, b);
                let first_carry = self.icmp(compare, first, a);
                let result = self.plain(opcode, first, carry);
                let second_carry = self.icmp(compare, result, first);
                let carried = self.plain(Opcode::Or, first_carry, second_carry);
                let ty = self.draft.value_type(result);
                let carried = self.extend(carried, false, ty);
                let address = self.value(items[3], source)?;
                self.store(carried, address, instance.result_type, source)?;
                Operand::Value(result)
            },
            | CommonBuiltin::FloatConstant(_, nan) => {
                if nan {
                    return Err(LoweringError::missing(
                        "NaN constructor must use library scheduling",
                        source,
                    ));
                }
                let ty = self.value_type(instance.result_type, source)?;
                let bits = super::floats::constant_parts((false, 0x7FFF, 1 << 63), ty)
                    .ok_or_else(|| LoweringError::unsupported(super::Construct::Builtin, source))?;
                Operand::Value(self.fconst(ty, bits))
            },
            | CommonBuiltin::Rotate(width, right) => {
                let value = self.value(items[0], source)?;
                let count = self.value(items[1], source)?;
                let ty = self.draft.value_type(value);
                let mask = self.iconst(ty, i128::from(width - 1));
                let count = self.plain(Opcode::And, count, mask);
                let zero = self.iconst(ty, 0);
                let opposite = self.plain(Opcode::Isub, zero, count);
                let opposite = self.plain(Opcode::And, opposite, mask);
                let first =
                    self.plain(if right { Opcode::Lshr } else { Opcode::Shl }, value, count);
                let second = self.plain(
                    if right { Opcode::Shl } else { Opcode::Lshr },
                    value,
                    opposite,
                );
                Operand::Value(self.plain(Opcode::Or, first, second))
            },
            | CommonBuiltin::Bswap(_) => {
                let arg = self.value(items[0], source)?;
                let ty = self.draft.value_type(arg);
                if ty == Type::I128 {
                    // The swapped halves, exchanged: every back end swaps
                    // 64 bits.
                    let shift = self.iconst(Type::I128, 64);
                    let low = self.convert(Opcode::Trunc, Type::I64, arg);
                    let high = self.plain(Opcode::Lshr, arg, shift);
                    let high = self.convert(Opcode::Trunc, Type::I64, high);
                    let low = self.convert(Opcode::Bswap, Type::I64, low);
                    let high = self.convert(Opcode::Bswap, Type::I64, high);
                    let low = self.extend(low, false, Type::I128);
                    let high = self.extend(high, false, Type::I128);
                    let low = self.plain(Opcode::Shl, low, shift);
                    Operand::Value(self.plain(Opcode::Or, low, high))
                } else {
                    Operand::Value(self.convert(Opcode::Bswap, ty, arg))
                }
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
            | CommonBuiltin::GenericBit(operation) =>
                self.generic_bit(operation, instance, &items[..count], source)?,
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
                if [items[0].ty, items[1].ty, destination]
                    .into_iter()
                    .any(|ty| self.wide_bit_int(ty).is_some())
                {
                    let (result, overflow) =
                        self.wide_overflow(op, items[0], items[1], destination, source)?;
                    if !predicate {
                        let address = self.value(items[2], source)?;
                        let place = Item {
                            operand: Operand::Place(super::Place::Memory(address)),
                            ty:      destination,
                        };
                        _ = self.write(place, result, source)?;
                    }
                    let overflow = self.extend(overflow, false, Type::I8);
                    self.push_item(Operand::Value(overflow), instance.result_type);
                    return Ok(());
                }
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
            let carry = if self.value_type(a.ty, source)?.bits() <= 64
                && self.value_type(b.ty, source)?.bits() <= 64
            {
                // Two magnitudes below 2^64 multiply within 128 bits.
                false_value
            } else if self.unit.target.is_wasm() {
                // The WebAssembly back end has no 128-bit `umul_overflow`:
                // the product wrapped if dividing it by a nonzero `av`
                // does not give `bv` back.
                let one = self.iconst(Type::I128, 1);
                let zero_a = self.icmp(IntCC::Eq, av, zero);
                let divisor = self.choose(zero_a, one, av);
                let quotient = self.plain(Opcode::Udiv, value, divisor);
                let differs = self.icmp(IntCC::Ne, quotient, bv);
                let nonzero_a = self.plain(Opcode::Xor, zero_a, true_value);
                self.plain(Opcode::And, differs, nonzero_a)
            } else {
                self.overflow_inst(Opcode::UmulOverflow, av, bv)
            };
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
        // A narrow bit-precise destination holds only its width.
        let bits = bit_width.unwrap_or(repr.narrow().map_or(ty.bits(), |(_, _, width)| width));
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
        let value = self.extend(value, false, ty);
        Ok((self.normalize(value, repr), overflow))
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

    /// GCC's `__builtin_setjmp` as Clang emits it: the frame address in
    /// word 0 of the buffer and the stack position in word 2, then
    /// `sjlj_setjmp`, which saves the resume address in word 1 and returns
    /// 0, or 1 when `__builtin_longjmp` resumes it. GNU extension: GCC
    /// manual, "Nonlocal Gotos".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Nonlocal-Gotos.html>
    fn builtin_setjmp(&mut self, buffer: Value) -> Value {
        let frame = self.nullary(Opcode::FrameAddress);
        self.store_word(frame, buffer);
        let stack = self.nullary(Opcode::StackSave);
        let offset = self.iconst(Type::I64, 16);
        let stack_word = self.ptr_add(buffer, offset);
        self.store_word(stack, stack_word);
        self.inst(
            InstData::Unary {
                opcode: Opcode::SjljSetjmp,
                ty:     Type::I32,
                arg:    buffer,
            },
            Type::I32,
        )
    }

    /// Stores the pointer `value` in a buffer word at `address`.
    fn store_word(&mut self, value: Value, address: Value) {
        _ = self.draft.push(
            self.memory_op(InstData::Store {
                ty:    Type::Ptr,
                flags: InstFlags::empty(),
                align: Align::from_bytes(8).expect("a word's alignment"),
                tag:   None,
                args:  [value, address],
            }),
            None,
        );
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

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    pub(super) fn emit_object_size(
        &mut self,
        size: crate::translation_phases::semantic_analysis::ObjectSize<'tu>,
        pointer: Option<Value>,
        source: crate::translation_phases::SourceVectors,
    ) -> Result<Value, LoweringError> {
        use crate::translation_phases::semantic_analysis::ObjectSize;
        let (size, offset) = match size {
            | ObjectSize::Constant(value) => return Ok(self.iconst(Type::I64, i128::from(value))),
            | ObjectSize::Dynamic { ty, offset, .. } => (self.runtime_size(ty, source)?, offset),
            | ObjectSize::Remainder { base, ty } => {
                let info = *self.info(base)?;
                let item = if matches!(
                    base.kind,
                    crate::translation_phases::parsing::syntax::ExpressionType::StringLiteral(_)
                ) {
                    let global = self.unit.string(base)?;
                    let address = self.inst(InstData::GlobalAddr { global }, Type::Ptr);
                    Item {
                        operand: Operand::Place(super::work::Place::Memory(address)),
                        ty:      info.ty,
                    }
                } else {
                    self.identifier(&info, source)?
                };
                let Operand::Place(super::work::Place::Memory(base)) = item.operand else {
                    return Err(LoweringError::missing(
                        "an object-size origin has no address",
                        source,
                    ));
                };
                let pointer = pointer.ok_or_else(|| {
                    LoweringError::missing("an object-size pointer was not evaluated", source)
                })?;
                let pointer = self.convert(Opcode::Ptrtoint, Type::I64, pointer);
                let base = self.convert(Opcode::Ptrtoint, Type::I64, base);
                let offset = self.binary_inst(Opcode::Isub, InstFlags::empty(), pointer, base);
                let size = self.runtime_size(ty, source)?;
                let remainder = self.binary_inst(Opcode::Isub, InstFlags::empty(), size, offset);
                let within = self.icmp(IntCC::Ule, offset, size);
                let zero = self.iconst(Type::I64, 0);
                return Ok(self.choose(within, remainder, zero));
            },
            | ObjectSize::Incoming { binding, offset } => (
                *self.incoming_sizes.get(&binding).ok_or_else(|| {
                    LoweringError::missing("incoming object size is not retained", source)
                })?,
                offset,
            ),
            | ObjectSize::Unknown => return Ok(self.iconst(Type::I64, i128::from(u64::MAX))),
            | ObjectSize::Deferred {
                mode,
                dynamic,
                through,
            } => {
                let query = crate::ir::ObjectSizeQuery::new(mode, dynamic)
                    .ok_or_else(|| LoweringError::missing("an object-size mode above 3", source))?;
                // Modes 1 and 3 need a call site's `subobject` hint, which
                // holds only for a parameter offset by pointer arithmetic
                // ([`Self::subobject_hints_pass`]); a pointer formed here
                // by a member access may name a narrower subobject.
                if query.subobject() && !through.is_some_and(|p| self.subobject_hints_pass(p)) {
                    let unknown = query.unknown(Type::I64);
                    return Ok(self.iconst(
                        Type::I64,
                        i128::try_from(unknown).expect("an i64 answer fits"),
                    ));
                }
                let ptr = pointer.ok_or_else(|| {
                    LoweringError::missing("an object-size pointer was not evaluated", source)
                })?;
                return Ok(self.inst(
                    InstData::ObjectSize {
                        ty: Type::I64,
                        query,
                        ptr,
                    },
                    Type::I64,
                ));
            },
        };
        if offset == 0 {
            return Ok(size);
        }
        let offset = self.iconst(Type::I64, i128::from(offset));
        let remainder = self.binary_inst(Opcode::Isub, InstFlags::empty(), size, offset);
        let within = self.icmp(IntCC::Ule, offset, size);
        let zero = self.iconst(Type::I64, 0);
        Ok(self.choose(within, remainder, zero))
    }
}
