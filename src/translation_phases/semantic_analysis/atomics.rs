//! Phase-7 atomic type constraints and type-generic intrinsics.
//! C11: §6.7.2.4p3 and §6.7.3p3, p. 121; PDF p. 139.
//! No backend operations are performed.

use super::{
    Analyzer,
    ArenaList,
    ConstantClass,
    Expression,
    ExpressionInfo,
    ExpressionType,
    Integer,
    Scalar,
    SemanticErrorKind,
    SourceVectors,
    TypeId,
    TypeKind,
    TypeQualifiers,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    C11,
    Gnu,
    Sync,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Operation {
    Init,
    ThreadFence,
    SignalFence,
    AlwaysLockFree,
    IsLockFree,
    Load,
    Store,
    Exchange,
    Compare,
    TestSet,
    Clear,
    FetchAdd,
    FetchSub,
    FetchBitwise,
    FetchMinMax,
    ValueCompare,
}

#[derive(Clone, Copy)]
struct Intrinsic {
    family:    Family,
    operation: Operation,
    generic:   bool,
    count:     usize,
}

fn intrinsic(name: &str) -> Option<Intrinsic> {
    use Family as F;
    use Operation as O;
    let (family, name) = if let Some(name) = name.strip_prefix("__c11_atomic_") {
        (F::C11, name)
    } else if let Some(name) = name.strip_prefix("__atomic_") {
        (F::Gnu, name)
    } else {
        (F::Sync, name.strip_prefix("__sync_")?)
    };
    let (operation, generic, count) = match (family, name) {
        | (F::C11, "init") => (O::Init, false, 2),
        | (F::C11 | F::Gnu, "thread_fence") => (O::ThreadFence, false, 1),
        | (F::C11 | F::Gnu, "signal_fence") => (O::SignalFence, false, 1),
        | (F::Sync, "synchronize") => (O::ThreadFence, false, 0),
        | (F::Gnu, "always_lock_free") => (O::AlwaysLockFree, false, 2),
        | (F::C11, "is_lock_free") => (O::IsLockFree, false, 1),
        | (F::Gnu, "is_lock_free") => (O::IsLockFree, false, 2),
        | (F::C11, "load") | (F::Gnu, "load_n") => (O::Load, false, 2),
        | (F::Gnu, "load") => (O::Load, true, 3),
        | (F::C11, "store") | (F::Gnu, "store_n") => (O::Store, false, 3),
        | (F::Gnu, "store") => (O::Store, true, 3),
        | (F::C11, "exchange") | (F::Gnu, "exchange_n") => (O::Exchange, false, 3),
        | (F::Gnu, "exchange") => (O::Exchange, true, 4),
        | (F::C11, "compare_exchange_strong" | "compare_exchange_weak") => (O::Compare, false, 5),
        | (F::Gnu, "compare_exchange_n") => (O::Compare, false, 6),
        | (F::Gnu, "compare_exchange") => (O::Compare, true, 6),
        | (F::Gnu, "test_and_set") => (O::TestSet, false, 2),
        | (F::Gnu, "clear") => (O::Clear, false, 2),
        | (F::Sync, "lock_test_and_set") => (O::Exchange, false, 2),
        | (F::Sync, "lock_release") => (O::Clear, false, 1),
        | (F::Sync, "bool_compare_and_swap") => (O::Compare, false, 3),
        | (F::Sync, "val_compare_and_swap") => (O::ValueCompare, false, 3),
        | (F::C11 | F::Gnu, "fetch_add")
        | (F::Gnu, "add_fetch")
        | (F::Sync, "fetch_and_add" | "add_and_fetch") =>
            (O::FetchAdd, false, if family == F::Sync { 2 } else { 3 }),
        | (F::C11 | F::Gnu, "fetch_sub")
        | (F::Gnu, "sub_fetch")
        | (F::Sync, "fetch_and_sub" | "sub_and_fetch") =>
            (O::FetchSub, false, if family == F::Sync { 2 } else { 3 }),
        | (F::C11 | F::Gnu, "fetch_and" | "fetch_or" | "fetch_xor" | "fetch_nand")
        | (F::Gnu, "and_fetch" | "or_fetch" | "xor_fetch" | "nand_fetch")
        | (
            F::Sync,
            "fetch_and_and" | "fetch_and_or" | "fetch_and_xor" | "fetch_and_nand" | "and_and_fetch"
            | "or_and_fetch" | "xor_and_fetch" | "nand_and_fetch",
        ) => (
            O::FetchBitwise,
            false,
            if family == F::Sync { 2 } else { 3 },
        ),
        | (F::C11 | F::Gnu, "fetch_min" | "fetch_max") | (F::Gnu, "min_fetch" | "max_fetch") =>
            (O::FetchMinMax, false, 3),
        | _ => return None,
    };
    Some(Intrinsic {
        family,
        operation,
        generic,
        count,
    })
}

/// Exact builtin recognition is shared with the preprocessing feature query.
pub(crate) fn modeled(name: &str) -> bool {
    intrinsic(name).is_some()
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Clang/GCC type-generic atomic builtin contract, independent of lowering.
    /// <https://clang.llvm.org/docs/LanguageExtensions.html#c11-atomic-builtins>
    /// <https://gcc.gnu.org/onlinedocs/gcc/_005f_005fatomic-Builtins.html>
    pub(super) fn atomic_call(
        &mut self,
        e: &'tu Expression<'tu>,
        mut callee: &'tu Expression<'tu>,
        arguments: ArenaList<'tu, &'tu Expression<'tu>>,
    ) -> Option<ExpressionInfo<'tu>> {
        use Operation as O;
        while let ExpressionType::Parenthesized { expression } = callee.kind {
            callee = expression;
        }
        let ExpressionType::Identifier(name) = callee.kind else {
            return None;
        };
        let builtin = intrinsic(self.context.string_cache.at(name.name))?;
        let operation = builtin.operation;
        let args = arguments.as_slice();
        if args.len() < builtin.count
            || ((builtin.family != Family::Sync || operation == O::ThreadFence)
                && args.len() != builtin.count)
        {
            self.error(
                SemanticErrorKind::InvalidArgumentCount,
                e.source_vectors,
                None,
                None,
            );
            return Some(Self::expression_result(e, self.types.unknown()));
        }
        let mut result = self.types.scalar(Scalar::Void);
        if matches!(operation, O::ThreadFence | O::SignalFence) {
            if builtin.count != 0 {
                self.atomic_integer_argument(args[0]);
            }
            return Some(Self::expression_result(e, result));
        }
        if matches!(operation, O::AlwaysLockFree | O::IsLockFree) {
            let size_type = self.types.scalar(self.types.target.size_t);
            self.atomic_value_argument(args[0], size_type);
            let size = self
                .expression_info(args[0])
                .integer
                .and_then(Integer::to_u64);
            let mut aligned = true;
            if let Some(&pointer) = args.get(1) {
                let info = self.expression_info(pointer);
                let ty = self.converted(info);
                let null = matches!(
                    self.address_parts(pointer, false),
                    Some((super::expressions::AddressBase::Absolute, 0))
                );
                if !self.null_pointer_constant(info) && !null {
                    if let TypeKind::Pointer(target) = self.types.nodes[ty.index] {
                        aligned = self
                            .types
                            .layout(target)
                            .zip(size)
                            .is_some_and(|(layout, size)| layout.align >= size);
                    } else if !self.types.unanalyzed(ty) {
                        self.error(
                            SemanticErrorKind::InvalidAtomicOperand,
                            pointer.source_vectors,
                            None,
                            None,
                        );
                    }
                }
            }
            result = self.types.scalar(Scalar::Bool);
            let mut info = Self::expression_result(e, result);
            let always = size.is_some_and(|size| matches!(size, 0 | 1 | 2 | 4 | 8)) && aligned;
            if size.is_some()
                && !self.expression_info(args[0]).atomic_cast()
                && (operation == O::AlwaysLockFree || always)
            {
                info.integer = Some(Integer::int(i128::from(always)).cast(1, false));
                info.ice = true;
                info.constant = ConstantClass::Arithmetic;
            }
            return Some(info);
        }
        let address = self.expression_info(args[0]);
        let address_ty = self.converted(address);
        if self.types.unanalyzed(address_ty) {
            return Some(Self::expression_result(e, self.types.unknown()));
        }
        let TypeKind::Pointer(object) = self.types.nodes[address_ty.index] else {
            self.error(
                SemanticErrorKind::InvalidAtomicOperand,
                args[0].source_vectors,
                None,
                None,
            );
            return Some(Self::expression_result(e, self.types.unknown()));
        };
        let atomic = matches!(self.types.nodes[object.index], TypeKind::Atomic(_));
        let value = self.types.non_atomic(object).unqualified();
        let integer = self.integer_type(value).is_some();
        let pointer = matches!(self.types.nodes[value.index], TypeKind::Pointer(_));
        let floating = matches!(
            self.types.nodes[value.index],
            TypeKind::Scalar(Scalar::Float | Scalar::Double)
        );
        let modifying = operation != O::Load;
        let valid = if builtin.family == Family::C11 {
            atomic
        } else if matches!(operation, O::TestSet | O::Clear) && builtin.family == Family::Gnu {
            operation == O::TestSet
                || matches!(
                    self.types.nodes[value.index],
                    TypeKind::Scalar(
                        Scalar::Bool | Scalar::Char | Scalar::SignedChar | Scalar::UnsignedChar
                    )
                )
        } else {
            !atomic
                && if builtin.generic {
                    self.complete_object(value)
                } else {
                    integer || pointer || (builtin.family == Family::Gnu && floating)
                }
        } && (!modifying || !object.qualifiers.contains(TypeQualifiers::CONST))
            && match operation {
                | O::FetchAdd | O::FetchSub =>
                    integer
                        || (pointer
                            && (builtin.family != Family::C11
                                || matches!(self.types.nodes[value.index],TypeKind::Pointer(target) if self.complete_object(target) || matches!(self.types.nodes[target.index],TypeKind::Function {..}))))
                        || (builtin.family != Family::Sync && floating),
                | O::FetchBitwise => integer,
                | O::FetchMinMax => integer || floating,
                | _ => true,
            };
        if !valid {
            self.error(
                SemanticErrorKind::InvalidAtomicOperand,
                args[0].source_vectors,
                None,
                None,
            );
            return Some(Self::expression_result(e, self.types.unknown()));
        }
        // Generic GNU operands and compare-exchange expected values are
        // pointer parameters; ordinary values use assignment conversion.
        match operation {
            | O::Load if builtin.generic => self.atomic_buffer_argument(args[1], value),
            | O::Store
            | O::Exchange
            | O::Init
            | O::FetchAdd
            | O::FetchSub
            | O::FetchBitwise
            | O::FetchMinMax => {
                if builtin.generic {
                    self.atomic_buffer_argument(args[1], value);
                } else {
                    let operand = if pointer
                        && builtin.family != Family::Sync
                        && matches!(operation, O::FetchAdd | O::FetchSub)
                    {
                        self.types.scalar(self.types.target.ptrdiff_t)
                    } else {
                        value
                    };
                    self.atomic_value_argument(args[1], operand);
                }
                if builtin.generic && operation == O::Exchange {
                    self.atomic_buffer_argument(args[2], value);
                }
            },
            | O::Compare | O::ValueCompare => {
                if builtin.family == Family::Sync {
                    self.atomic_value_argument(args[1], value);
                } else {
                    self.atomic_buffer_argument(args[1], value);
                }
                if builtin.generic {
                    self.atomic_buffer_argument(args[2], value);
                } else {
                    self.atomic_value_argument(args[2], value);
                }
                if builtin.family == Family::Gnu {
                    let boolean = self.types.scalar(Scalar::Bool);
                    self.atomic_value_argument(args[3], boolean);
                }
            },
            | _ => {},
        }
        if builtin.family != Family::Sync && operation != O::Init {
            let order_index = builtin.count - 1;
            if operation == O::Compare {
                self.atomic_order(args[order_index - 1], O::Exchange, false);
                self.atomic_order(args[order_index], O::Compare, true);
            } else {
                self.atomic_order(args[order_index], operation, false);
            }
        }
        result = match operation {
            | O::Compare | O::TestSet => self.types.scalar(Scalar::Bool),
            | O::Store | O::Init | O::Clear => result,
            | O::Load | O::Exchange if builtin.generic => result,
            | _ => value,
        };
        Some(Self::expression_result(e, result))
    }

    fn atomic_integer_argument(&mut self, e: &'tu Expression<'tu>) {
        // Memory-order/size parameters have integer formal types and accept
        // ordinary scalar assignment conversions (including floating values).
        let integer = self.types.scalar(Scalar::Int);
        self.atomic_value_argument(e, integer);
    }

    fn atomic_value_argument(&mut self, e: &'tu Expression<'tu>, target: TypeId) {
        if !self.assignment_compatible(target, self.expression_info(e)) {
            self.error(
                SemanticErrorKind::InvalidAtomicOperand,
                e.source_vectors,
                None,
                None,
            );
        }
        self.convert(e, target, super::expressions::ConversionKind::Assignment);
    }

    fn atomic_buffer_argument(&mut self, e: &'tu Expression<'tu>, value: TypeId) {
        let info = self.expression_info(e);
        let ty = self.converted(info);
        if let TypeKind::Pointer(source) = self.types.nodes[ty.index]
            && !source.qualifiers.is_empty()
            && (self
                .types
                .composite(source.unqualified(), value.unqualified())
                .is_some()
                || matches!(
                    self.types.nodes[source.index],
                    TypeKind::Scalar(Scalar::Void)
                ))
        {
            self.error(
                SemanticErrorKind::AtomicBufferQualifiers,
                e.source_vectors,
                None,
                None,
            );
            let target = self.types.intern(TypeKind::Pointer(value));
            self.convert(e, target, super::expressions::ConversionKind::Assignment);
            return;
        }
        let target = self.types.intern(TypeKind::Pointer(value));
        self.atomic_value_argument(e, target);
    }

    fn atomic_order(&mut self, e: &'tu Expression<'tu>, operation: Operation, failure: bool) {
        self.atomic_integer_argument(e);
        let info = self.expression_info(e);
        if let Some(order) = info.integer {
            let invalid = !(0..=5).contains(&order.value)
                || (matches!(operation, Operation::Load | Operation::Compare)
                    && matches!(order.value, 3 | 4))
                || (matches!(operation, Operation::Store | Operation::Clear)
                    && matches!(order.value, 1 | 2 | 4));
            if invalid {
                self.error(
                    if failure {
                        SemanticErrorKind::InvalidAtomicFailureOrder
                    } else {
                        SemanticErrorKind::InvalidAtomicOrder
                    },
                    e.source_vectors,
                    None,
                    None,
                );
            }
        }
    }
}

impl Analyzer<'_, '_, '_> {
    /// C11 §6.7.2.4p3 and §6.7.3p3; Clang also requires a complete object.
    pub(super) fn atomic_type(
        &mut self,
        value: TypeId,
        source: SourceVectors,
        specifier: bool,
    ) -> TypeId {
        if self.types.unanalyzed(value) {
            return self.types.unknown();
        }
        let invalid = !self.complete_object(value)
            || matches!(
                self.types.nodes[value.index],
                TypeKind::Array(..) | TypeKind::Function { .. }
            )
            || (specifier
                && (!value.qualifiers.is_empty()
                    || matches!(self.types.nodes[value.index], TypeKind::Atomic(_))));
        if invalid {
            self.error(SemanticErrorKind::InvalidAtomicType, source, None, None);
            self.types.unknown()
        } else if matches!(self.types.nodes[value.index], TypeKind::Atomic(_)) {
            value
        } else {
            self.types
                .intern(TypeKind::Atomic(value.unqualified()))
                .qualified(value.qualifiers)
        }
    }
}
