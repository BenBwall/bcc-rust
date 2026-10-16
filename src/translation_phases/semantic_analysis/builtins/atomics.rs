//! Phase-7 atomic type constraints and type-generic intrinsics.
//! C11: §6.7.2.4p3 and §6.7.3p3, p. 121; PDF p. 139.
//! No backend operations are performed.
//!
//! These extensions are checked during translation phase 7. Operand types and
//! constant requirements are validated here; backend instructions are not
//! emitted. C99: §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22;
//! §6.5.2.2, pp. 71-72; PDF pp. 83-84 (extended function calls).

use crate::{
    builtins::{
        AtomicFamily as Family,
        AtomicOperation as Operation,
    },
    translation_phases::{
        SourceVectors,
        parsing::{
            declaration_syntax::TypeQualifiers,
            syntax::{
                Expression,
                ExpressionType,
            },
        },
        semantic_analysis::{
            Analyzer,
            SemanticErrorKind,
            expressions::{
                ConstantClass,
                ExpressionInfo,
            },
            integer::Integer,
            types::{
                Scalar,
                TypeId,
                TypeKind,
            },
        },
    },
    util::arena_list::ArenaList,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Clang/GCC type-generic atomic builtin contract, independent of lowering.
    /// Clang extension: Clang Language Extensions, "C11 atomic builtins".
    /// <https://clang.llvm.org/docs/LanguageExtensions.html#c11-atomic-builtins>
    /// GNU extension: GCC manual, "__atomic Builtins".
    /// <https://gcc.gnu.org/onlinedocs/gcc/_005f_005fatomic-Builtins.html>
    /// GNU extension: GCC manual, "__sync Builtins".
    /// <https://gcc.gnu.org/onlinedocs/gcc/_005f_005fsync-Builtins.html>
    pub(in crate::translation_phases::semantic_analysis) fn atomic_call(
        &mut self,
        e: &'tu Expression<'tu>,
        callee: &'tu Expression<'tu>,
        arguments: ArenaList<'tu, &'tu Expression<'tu>>,
    ) -> Option<ExpressionInfo<'tu>> {
        use Operation as O;
        let callee = self.selected_designator(callee);
        let ExpressionType::Identifier(name) = callee.kind else {
            return None;
        };
        // C99 §6.5.2.2p1: shadowing bindings use ordinary call typing.
        if !self
            .expression_info(callee)
            .binding
            .is_some_and(|id| self.bindings[id].origin.is_intrinsic())
        {
            return None;
        }
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
                .zip(self.integer_type(size_type))
                // C99 §6.3.1.3p2: fold the converted unsigned size_t value.
                .and_then(|(value, (bits, signed))| value.cast(bits, signed).to_u64());
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
        let mut info = Self::expression_result(e, result);
        info.operation_type = Some(object);
        Some(info)
    }

    /// Applies the integer parameter conversion for an atomic order or size.
    /// GNU extension: GCC manual, "__atomic Builtins".
    /// <https://gcc.gnu.org/onlinedocs/gcc/_005f_005fatomic-Builtins.html>
    /// C99: §6.5.2.2 paragraph 7, p. 72; PDF p. 84.
    fn atomic_integer_argument(&mut self, e: &'tu Expression<'tu>) {
        // Memory-order/size parameters have integer formal types and accept
        // ordinary scalar assignment conversions (including floating values).
        let integer = self.types.scalar(Scalar::Int);
        self.atomic_value_argument(e, integer);
    }

    /// Checks and retains assignment conversion for an atomic value argument.
    /// The operand converts as a prototype argument does, so pointer
    /// constraint violations take their diagnostic groups, as in Clang.
    /// C99: §6.5.16.1 paragraph 1, p. 92; PDF p. 104.
    /// C99: §6.5.2.2 paragraph 7, p. 72; PDF p. 84.
    fn atomic_value_argument(&mut self, e: &'tu Expression<'tu>, target: TypeId) {
        let info = self.expression_info(e);
        if !self.assignment_conversion(target, info) {
            self.error(
                SemanticErrorKind::InvalidAtomicOperand,
                e.source_vectors,
                None,
                None,
            );
        }
        self.convert(e, target, super::expressions::ConversionKind::Assignment);
    }

    /// Checks atomic output/expected buffer types and discarded qualifiers.
    /// GNU extension: GCC manual, "__atomic Builtins".
    /// <https://gcc.gnu.org/onlinedocs/gcc/_005f_005fatomic-Builtins.html>
    /// C99: §6.5.16.1 paragraph 1, p. 92; PDF p. 104.
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

    /// Checks operation-specific constant memory orders, including failure
    /// orders.
    /// GNU extension: GCC manual, "__atomic Builtins".
    /// <https://gcc.gnu.org/onlinedocs/gcc/_005f_005fatomic-Builtins.html>
    /// C11: §7.17.7.2 paragraph 2, p. 282; PDF p. 300.
    /// C11: §7.17.7.1 paragraph 2, p. 282; PDF p. 300.
    /// C11: §7.17.7.4 paragraph 2, p. 283; PDF p. 301.
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
    /// Checks atomic specifier/qualifier eligibility; Clang also requires a
    /// complete object.
    /// C11: §6.7.2.4 paragraph 3, p. 121; PDF p. 139.
    /// C11: §6.7.3 paragraph 3, p. 121; PDF p. 139.
    /// Clang extension: Clang rejects `_Atomic` on an incomplete type with
    /// `err_atomic_specifier_bad_type` in `DiagnosticSemaKinds.td`.
    pub(in crate::translation_phases::semantic_analysis) fn atomic_type(
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

/// Recognition follows the neutral registry.
fn intrinsic(name: &str) -> Option<crate::builtins::AtomicBuiltin> {
    match crate::builtins::lookup(name)?.operation {
        | crate::builtins::BuiltinOperation::Atomic(atomic) => Some(atomic),
        | _ => None,
    }
}
pub(crate) fn modeled(name: &str) -> bool {
    intrinsic(name).is_some()
}
