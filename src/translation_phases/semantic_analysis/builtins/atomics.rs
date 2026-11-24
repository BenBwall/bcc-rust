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
    configuration::{
        DiagnosticGroup,
        RuleSet,
    },
    translation_phases::{
        Context,
        SourceVectors,
        parsing::{
            declaration_syntax::TypeQualifiers,
            syntax::Expression,
        },
        semantic_analysis::{
            Analyzer,
            SemanticErrorKind,
            compiler_wording::ConversionSite,
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
    util::{
        arena_list::ArenaList,
        string_cache::StringCacheId,
    },
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
        name: StringCacheId,
        builtin: crate::builtins::AtomicBuiltin,
        arguments: ArenaList<'tu, &'tu Expression<'tu>>,
    ) -> ExpressionInfo<'tu> {
        use Operation as O;
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
            return Self::expression_result(e, self.types.unknown());
        }
        let mut result = self.types.scalar(Scalar::Void);
        if matches!(operation, O::ThreadFence | O::SignalFence) {
            if builtin.count != 0 {
                self.atomic_integer_argument(args[0]);
            }
            return Self::expression_result(e, result);
        }
        if matches!(operation, O::AlwaysLockFree | O::IsLockFree) {
            let size_type = self.types.scalar(self.types.target.size_t);
            self.atomic_value_argument(args[0], size_type);
            let size = self
                .expression_info(args[0])
                .integer
                .zip(self.integer_type(size_type))
                // C99 §6.3.1.3p2: fold the converted unsigned size_t value.
                .and_then(|(value, (bits, signed))| {
                    self.wide.convert(value, bits, signed).to_u64()
                });
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
            return info;
        }
        let address = self.expression_info(args[0]);
        let address_ty = self.converted(address);
        if self.types.unanalyzed(address_ty) {
            return Self::expression_result(e, self.types.unknown());
        }
        let TypeKind::Pointer(object) = self.types.nodes[address_ty.index] else {
            self.error(
                SemanticErrorKind::InvalidAtomicOperand,
                args[0].source_vectors,
                None,
                None,
            );
            return Self::expression_result(e, self.types.unknown());
        };
        let atomic = matches!(self.types.nodes[object.index], TypeKind::Atomic(_));
        let value = self.types.non_atomic(object).unqualified();
        let integer = self.integer_type(value).is_some();
        let pointer = matches!(self.types.nodes[value.index], TypeKind::Pointer(_));
        let floating = matches!(
            self.types.nodes[value.index],
            TypeKind::Scalar(s) if matches!(s.representation(), Scalar::Float | Scalar::Double)
        );
        let modifying = operation != O::Load;
        let gcc = self.gcc_atomic_operands();
        // Both compilers refuse a `__sync` builtin on a `_BitInt` of more
        // than 16 bytes. Clang also refuses one in the `__atomic` value
        // builtins, and in the `__sync` builtins unless its width is a power
        // of two; GCC accepts those.
        let clang = self.context.configuration.bit_int_rules() == RuleSet::Clang;
        if let TypeKind::BitInt { width, .. } = self.types.nodes[value.index]
            && let Some(kind) = match builtin.family {
                | Family::Sync if width > 128 => Some(if clang {
                    SemanticErrorKind::AtomicOperandBytes
                } else {
                    SemanticErrorKind::InvalidAtomicOperand
                }),
                | Family::Gnu if clang && !matches!(operation, O::TestSet | O::Clear) =>
                    Some(SemanticErrorKind::AtomicBuiltinBitInt),
                | Family::Sync if clang && !width.is_power_of_two() =>
                    Some(SemanticErrorKind::AtomicOperandSize),
                | _ => None,
            }
        {
            self.error(kind, args[0].source_vectors, None, None);
            return Self::expression_result(e, self.types.unknown());
        }
        // Both compilers declare `__atomic_test_and_set` and `__atomic_clear`
        // with a `volatile void *` parameter, so any object will do. GCC only
        // pedwarns about a generic builtin's `const` object
        // ([`Self::gcc_generic_arguments`]).
        let valid = if builtin.family == Family::C11 {
            atomic
        } else if matches!(operation, O::TestSet | O::Clear) && builtin.family == Family::Gnu {
            true
        } else {
            // An incomplete type, such as an enumeration before its closing
            // brace, has no size to access.
            // C99 §6.2.5p1, p. 33; PDF p. 45; §6.7.2.2p4, p. 105; PDF p. 117.
            !atomic
                && self.complete_object(value)
                && (builtin.generic
                    || integer
                    || pointer
                    || (builtin.family == Family::Gnu && floating))
        } && (!modifying
            || !object.qualifiers.contains(TypeQualifiers::CONST)
            || (gcc && builtin.generic))
            && match operation {
                | O::FetchAdd | O::FetchSub =>
                    integer
                        || (pointer
                            && (builtin.family != Family::C11
                                || matches!(self.types.nodes[value.index],TypeKind::Pointer(target) if self.complete_object(target) || matches!(self.types.nodes[target.index],TypeKind::Function {..}))))
                        || (builtin.family != Family::Sync && floating),
                // GCC's `__sync` and `__atomic` builtins take a pointer
                // wherever they take an integer (`sync_resolve_size` in
                // GCC's `c-common.cc`).
                | O::FetchBitwise => integer || (gcc && pointer && builtin.family != Family::C11),
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
            return Self::expression_result(e, self.types.unknown());
        }
        if builtin.generic && gcc {
            self.gcc_generic_arguments(e, name, operation, args, object);
        } else {
            self.atomic_operands(e, builtin, args, value, pointer);
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
        info
    }

    /// Converts the operands after the address: generic GNU operands and
    /// compare-exchange expected values are pointer parameters, and the
    /// other values convert as by assignment, or under the GCC identity as
    /// by a cast ([`Self::atomic_operand`]).
    /// GNU extension: GCC manual, "__atomic Builtins".
    /// <https://gcc.gnu.org/onlinedocs/gcc/_005f_005fatomic-Builtins.html>
    /// C99: §6.5.2.2 paragraph 7, p. 72; PDF p. 84.
    fn atomic_operands(
        &mut self,
        e: &'tu Expression<'tu>,
        builtin: crate::builtins::AtomicBuiltin,
        args: &[&'tu Expression<'tu>],
        value: TypeId,
        pointer: bool,
    ) {
        use Operation as O;
        match builtin.operation {
            | O::Load if builtin.generic => self.atomic_buffer_argument(e, args[1], 2, value),
            | O::Store
            | O::Exchange
            | O::Init
            | O::FetchAdd
            | O::FetchSub
            | O::FetchBitwise
            | O::FetchMinMax => {
                if builtin.generic {
                    self.atomic_buffer_argument(e, args[1], 2, value);
                } else {
                    let operand = if pointer
                        && builtin.family != Family::Sync
                        && matches!(builtin.operation, O::FetchAdd | O::FetchSub)
                    {
                        self.types.scalar(self.types.target.ptrdiff_t)
                    } else {
                        value
                    };
                    self.atomic_operand(builtin, args[1], 2, operand);
                }
                if builtin.generic && builtin.operation == O::Exchange {
                    self.atomic_buffer_argument(e, args[2], 3, value);
                }
            },
            | O::Compare | O::ValueCompare => {
                if builtin.family == Family::Sync {
                    self.atomic_operand(builtin, args[1], 2, value);
                } else if builtin.family == Family::Gnu
                    && !builtin.generic
                    && self.gcc_atomic_operands()
                {
                    self.gcc_expected_argument(e, args[1], value);
                } else {
                    self.atomic_buffer_argument(e, args[1], 2, value);
                }
                if builtin.generic {
                    self.atomic_buffer_argument(e, args[2], 3, value);
                } else {
                    self.atomic_operand(builtin, args[2], 3, value);
                }
                if builtin.family == Family::Gnu {
                    let boolean = self.types.scalar(Scalar::Bool);
                    self.atomic_value_argument(args[3], boolean);
                }
            },
            | _ => {},
        }
    }

    /// Whether the atomic builtins' operands follow GCC's rules: the GCC
    /// identity's. GCC converts a value operand as by a cast, checks a
    /// generic builtin's pointers only for their size, and passes the
    /// expected value of `__atomic_compare_exchange_n` to a `void *`
    /// parameter (`resolve_overloaded_builtin` in GCC's `c-common.cc`).
    fn gcc_atomic_operands(&self) -> bool {
        self.context.configuration.compiler_identity()
            == crate::configuration::CompilerIdentity::Gcc
    }

    /// Converts the value operand `e`, argument `position`, of the atomic
    /// builtin `builtin` to `target`: as a prototype's argument, which
    /// Clang's words name, or for a GNU or `__sync` builtin under the GCC
    /// identity as by a cast, which GCC applies to it
    /// (`sync_resolve_params` in GCC's `c-common.cc`), so a value that
    /// discards qualifiers or converts between a pointer and an integer
    /// passes silently. Under GCC, a conversion a cast refuses, such as from
    /// a structure or between a pointer and a floating value, one between
    /// arithmetic types and a `__c11` builtin's operand are checked as by
    /// assignment, without GCC's assignment pedwarns.
    /// C99: §6.5.4 paragraphs 2-4, p. 81; PDF p. 93; §6.5.2.2 paragraph 7,
    /// p. 72; PDF p. 84; §6.5.16.1 paragraph 1, p. 92; PDF p. 104.
    fn atomic_operand(
        &mut self,
        builtin: crate::builtins::AtomicBuiltin,
        e: &'tu Expression<'tu>,
        position: usize,
        target: TypeId,
    ) {
        let info = self.expression_info(e);
        if !self.gcc_atomic_operands() {
            let site = ConversionSite::Argument {
                position,
                callee: None,
            };
            if !self.assignment_conversion_at(target, info, e.source_vectors, site, false) {
                self.error(
                    SemanticErrorKind::InvalidAtomicOperand,
                    e.source_vectors,
                    None,
                    None,
                );
            }
            self.convert(e, target, super::expressions::ConversionKind::Assignment);
            return;
        }
        if builtin.family == Family::C11 {
            self.atomic_value_argument(e, target);
            return;
        }
        let from = self.converted(info);
        let target_pointer = self.pointer_target(target).is_some();
        let from_pointer = self.pointer_target(from).is_some();
        let floating = |analyzer: &Self, ty: TypeId| {
            analyzer.arithmetic(ty) && analyzer.integer_type(ty).is_none()
        };
        if (target_pointer || from_pointer)
            && self.scalar_type(target)
            && self.scalar_type(from)
            && !(target_pointer && floating(self, from))
            && !(from_pointer && floating(self, target))
        {
            self.convert(e, target, super::expressions::ConversionKind::Assignment);
            return;
        }
        self.atomic_value_argument(e, target);
    }

    /// GCC's check of the expected operand `e` of the call `call` to
    /// `__atomic_compare_exchange_n`, whose object has type `value`: an
    /// argument passed to the `void *` parameter of the sized builtin GCC
    /// resolves the call to, such as `__atomic_compare_exchange_4`, which
    /// GCC names in its words. Any object pointer passes; one whose target
    /// is qualified discards the qualifiers, and an integer makes a pointer.
    /// GNU extension: GCC manual, "__atomic Builtins".
    /// <https://gcc.gnu.org/onlinedocs/gcc/_005f_005fatomic-Builtins.html>
    /// C99: §6.5.2.2 paragraph 7, p. 72; PDF p. 84; §6.5.16.1 paragraph 1,
    /// p. 92; PDF p. 104.
    fn gcc_expected_argument(
        &mut self,
        call: &'tu Expression<'tu>,
        e: &'tu Expression<'tu>,
        value: TypeId,
    ) {
        let size = self.types.layout(value).map_or(0, |layout| layout.size);
        let callee =
            crate::diagnostics::format_in!(self.scratch, "__atomic_compare_exchange_{size}");
        let callee = self.context.string_cache.intern(callee);
        let void = self.types.scalar(Scalar::Void);
        let target = self.types.intern(TypeKind::Pointer(void));
        let info = self.expression_info(e);
        // GCC reports the rebuilt call's argument at the call.
        let at = Context::first_token_vector(call.source_vectors);
        let parameter = ConversionSite::Argument {
            position: 2,
            callee:   Some(callee),
        };
        if !self.assignment_conversion_at(target, info, at, parameter, false) {
            self.error(
                SemanticErrorKind::InvalidAtomicOperand,
                e.source_vectors,
                None,
                None,
            );
        }
        self.convert(e, target, super::expressions::ConversionKind::Assignment);
    }

    /// GCC's checks of the pointer operands of the call `call` to the
    /// generic builtin `name`, whose first operand points to `object`. Each
    /// must point to an object of the first one's size, a `void` counting
    /// as one byte; one of another size is an error, and GCC checks no
    /// other compatibility. An operand the builtin writes through may not
    /// point to a `const` object, nor any operand after the first to a
    /// `volatile` one, which GCC reports as a pedantic warning
    /// (`get_atomic_generic_size` in GCC's `c-common.cc`).
    /// GNU extension: GCC manual, "__atomic Builtins".
    /// <https://gcc.gnu.org/onlinedocs/gcc/_005f_005fatomic-Builtins.html>
    fn gcc_generic_arguments(
        &mut self,
        call: &'tu Expression<'tu>,
        name: StringCacheId,
        operation: Operation,
        args: &[&'tu Expression<'tu>],
        object: TypeId,
    ) {
        use Operation as O;
        // The pointer operands, and the bits of those written through.
        let (count, outputs) = match operation {
            | O::Load => (2, 0b10),
            | O::Store => (2, 0b01),
            | O::Exchange => (3, 0b101),
            | O::Compare => (3, 0b011),
            | _ => return,
        };
        let at = Context::first_token_vector(call.source_vectors);
        let size = self.types.layout(object).map_or(0, |layout| layout.size);
        let value = self.types.non_atomic(object).unqualified();
        let builtin =
            crate::diagnostics::format_in!(self.scratch, "{}", self.context.string_cache.at(name));
        for (index, &arg) in args.iter().enumerate().take(count) {
            let position = index + 1;
            let target = if index == 0 {
                object
            } else {
                let ty = self.converted(self.expression_info(arg));
                let Some(target) = self.pointer_target(ty) else {
                    let words = crate::diagnostics::format_in!(
                        self.scratch,
                        "argument {position} of '{builtin}' must be a pointer type"
                    );
                    self.atomic_generic_error(words, at);
                    return;
                };
                let refusal = if matches!(self.types.nodes[target.index], TypeKind::Function { .. })
                {
                    Some("must not be a pointer to a function")
                } else if self.types.variably_modified(target) {
                    Some("must be a pointer to a constant size type")
                } else {
                    None
                };
                if let Some(refusal) = refusal {
                    let words = crate::diagnostics::format_in!(
                        self.scratch,
                        "argument {position} of '{builtin}' {refusal}"
                    );
                    self.atomic_generic_error(words, at);
                    return;
                }
                let bytes = if matches!(
                    self.types.nodes[target.index],
                    TypeKind::Scalar(Scalar::Void)
                ) {
                    1
                } else {
                    self.types.layout(target).map_or(0, |layout| layout.size)
                };
                if bytes != size {
                    let words = crate::diagnostics::format_in!(
                        self.scratch,
                        "size mismatch in argument {position} of '{builtin}'"
                    );
                    self.atomic_generic_error(words, at);
                    return;
                }
                target
            };
            let mut discarded = [None, None];
            if outputs & (1 << index) != 0 && target.qualifiers.contains(TypeQualifiers::CONST) {
                discarded[0] = Some("const");
            }
            if index > 0 && target.qualifiers.contains(TypeQualifiers::VOLATILE) {
                discarded[1] = Some("volatile");
            }
            for qualifier in discarded.into_iter().flatten() {
                let words = crate::diagnostics::format_in!(
                    self.scratch,
                    "argument {position} of '{builtin}' discards '{qualifier}' qualifier"
                );
                let words = self.context.string_cache.intern(words);
                let group = DiagnosticGroup::PointerQualifiers;
                self.group_diagnostic(
                    group,
                    SemanticErrorKind::PointerConstraint(group),
                    at,
                    Some(words),
                    None,
                );
            }
            if index > 0 {
                let target = self.types.intern(TypeKind::Pointer(value));
                self.convert(arg, target, super::expressions::ConversionKind::Assignment);
            }
        }
    }

    /// Reports GCC's `words` about a generic atomic builtin's operand at
    /// `site` as an error.
    fn atomic_generic_error(&mut self, words: &str, site: SourceVectors) {
        let words = self.context.string_cache.intern(words);
        self.error(
            SemanticErrorKind::AtomicGenericOperand,
            site,
            Some(words),
            None,
        );
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

    /// Converts the operand `e`, argument `position` of the call `call`, a
    /// pointer to a value, expected value or output buffer of the object
    /// type `value`, to a pointer to `value` as a prototype's argument
    /// converts, so a pointer to a qualified type discards the qualifiers.
    /// Clang also warns when it is null, since the builtin reads or writes
    /// through it (`BuildAtomicExpr` in Clang's `SemaChecking.cpp`).
    /// GNU extension: GCC manual, "__atomic Builtins".
    /// <https://gcc.gnu.org/onlinedocs/gcc/_005f_005fatomic-Builtins.html>
    /// Clang extension: Clang Language Extensions, "C11 atomic builtins".
    /// <https://clang.llvm.org/docs/LanguageExtensions.html#c11-atomic-builtins>
    /// C99: §6.5.2.2 paragraph 7, p. 72; PDF p. 84; §6.5.16.1 paragraph 1,
    /// p. 92; PDF p. 104.
    fn atomic_buffer_argument(
        &mut self,
        call: &'tu Expression<'tu>,
        e: &'tu Expression<'tu>,
        position: usize,
        value: TypeId,
    ) {
        if self.context.configuration.compiler_identity().rules() == RuleSet::Clang {
            let start = Context::first_token_vector(call.source_vectors);
            self.clang_null_argument(call, e, start);
        }
        let target = self.types.intern(TypeKind::Pointer(value));
        let info = self.expression_info(e);
        let site = ConversionSite::Argument {
            position,
            callee: None,
        };
        if !self.assignment_conversion_at(target, info, e.source_vectors, site, false) {
            self.error(
                SemanticErrorKind::InvalidAtomicOperand,
                e.source_vectors,
                None,
                None,
            );
        }
        self.convert(e, target, super::expressions::ConversionKind::Assignment);
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
                    || matches!(self.types.nodes[value.index], TypeKind::Atomic(_))))
            // Clang rejects an atomic `_BitInt` ("_Atomic cannot be applied
            // to integer type"); GCC accepts it.
            || (matches!(self.types.nodes[value.index], TypeKind::BitInt { .. })
                && self.context.configuration.bit_int_rules() == RuleSet::Clang);
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
