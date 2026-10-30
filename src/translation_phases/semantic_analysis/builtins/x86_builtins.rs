//! Checks of x86 builtin immediates and of Clang's operand-overloaded vector
//! builtins. Signatures and immediate limits come from the generated
//! `src/builtins/x86.rs` registry shard, whose generator independently
//! compiles every signature on four targets; an overload is identified by
//! its descriptor's [`VectorOverload`] rule.
//! Extensions: GCC x86 Built-in Functions and Clang Language Extensions.
//!
//! These extensions are checked during translation phase 7. Operand types and
//! constant requirements are validated here; backend instructions are not
//! emitted. C99: §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22;
//! §6.5.2.2, pp. 71-72; PDF pp. 83-84 (extended function calls).

use crate::{
    builtins::{
        ConstantConstraint,
        TypeRule,
        VectorOverload,
    },
    configuration::RuleSet,
    translation_phases::{
        parsing::{
            declaration_syntax::TypeQualifiers,
            syntax::Expression,
        },
        semantic_analysis::{
            Analyzer,
            SemanticErrorKind,
            expressions::ExpressionInfo,
            types::{
                Scalar,
                TypeId,
                TypeKind,
            },
        },
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Checks constant immediate arguments after prototype argument conversion.
    /// GNU extension: GCC manual, "x86 Built-in Functions".
    /// <https://gcc.gnu.org/onlinedocs/gcc/x86-Built-in-Functions.html>
    /// C99: §6.5.2.2 paragraph 7, p. 72; PDF p. 84.
    pub(in crate::translation_phases::semantic_analysis) fn x86_immediates(
        &mut self,
        function: ExpressionInfo<'tu>,
        args: super::ArenaList<'tu, &'tu Expression<'tu>>,
    ) {
        let Some(descriptor) = function
            .binding
            .and_then(|binding| self.bindings[binding].origin.descriptor())
        else {
            return;
        };
        if descriptor.type_rule != TypeRule::X86Signature {
            return;
        }
        // C99 §6.5.2.2p7, p. 72; PDF p. 84: prototype arguments convert
        // to parameter types before GNU intrinsic immediate constraints apply.
        let TypeKind::Function { parameters, .. } = self.types.nodes[function.ty.index] else {
            return;
        };
        for (index, rule) in descriptor.arguments.iter().enumerate() {
            let ConstantConstraint::Immediate(low, high, mask) = rule.constraint else {
                continue;
            };
            if let Some(&arg) = args.get(index) {
                let info = self.expression_info(arg);
                if self.types.unanalyzed(info.ty) {
                    continue;
                }
                let value = info.integer.filter(|_| info.ice).and_then(|value| {
                    let parameter = parameters.get(index)?;
                    let (bits, signed) = self.integer_type(*parameter)?;
                    self.wide.convert(value, bits, signed).to_i128()
                });
                if !value.is_some_and(|v| {
                    v >= i128::from(low)
                        && v <= i128::from(high)
                        && (mask == 0 || (v < 16 && (mask & (1 << v)) != 0))
                }) {
                    // Clang's `-Wargument-outside-range` is an error by
                    // default, which `-w` silences; a scale outside its
                    // set stays an error.
                    if let Some(value) = value.and_then(|value| i32::try_from(value).ok())
                        && mask == 0
                        && self.context.configuration.compiler_identity().rules() == RuleSet::Clang
                    {
                        self.error(
                            SemanticErrorKind::ImmediateOutOfRange { value, low, high },
                            arg.source_vectors,
                            None,
                            None,
                        );
                        continue;
                    }
                    self.error(
                        super::registry::diagnostic(rule.diagnostic),
                        arg.source_vectors,
                        None,
                        None,
                    );
                }
            }
        }
    }

    /// Clang overloads these builtins by their operands; they cannot be given
    /// a single C prototype. Retain the checked operand-dependent result.
    /// Clang extension: Clang Language Extensions, "Builtin Functions".
    /// <https://clang.llvm.org/docs/LanguageExtensions.html#builtin-functions>
    pub(in crate::translation_phases::semantic_analysis) fn vector_overload(
        &mut self,
        e: &'tu Expression<'tu>,
        callee: super::Identifier,
        operation: VectorOverload,
        args: super::ArenaList<'tu, &'tu Expression<'tu>>,
    ) -> ExpressionInfo<'tu> {
        use VectorOverload as O;
        let arity = operation.arity();
        let integer = matches!(operation, O::AddSat | O::SubSat);
        let floating = operation == O::Sqrt;
        let load = operation == O::NontemporalLoad;
        let store = operation == O::NontemporalStore;
        let absolute = operation == O::Abs;
        let min_max = matches!(operation, O::Min | O::Max);
        self.context.report_extension(
            crate::configuration::Feature::VectorBuiltins,
            "vector builtin",
            callee.source_vectors,
        );
        // GNU overload constraints cannot add errors for already failed
        // operands.
        if args
            .iter()
            .any(|&arg| self.types.unanalyzed(self.expression_info(arg).ty))
        {
            return Self::expression_result(e, self.types.unknown());
        }
        let input = args.first().map(|&arg| self.expression_info(arg));
        if args.len() == arity
            && let Some(input) = input
        {
            let mut ty = self.converted(input);
            if load {
                if let TypeKind::Pointer(element) = self.types.nodes[ty.index]
                    && self.nontemporal_element(element)
                {
                    return Self::expression_result(e, element.unqualified());
                }
            } else if store {
                let pointer = self.expression_info(args[1]);
                let pointer = self.converted(pointer);
                if let TypeKind::Pointer(to) = self.types.nodes[pointer.index]
                    && self.nontemporal_element(to)
                    && !to.qualifiers.contains(TypeQualifiers::CONST)
                    && self.assignment_compatible(to, input)
                {
                    ty = self.types.scalar(Scalar::Void);
                    return Self::expression_result(e, ty);
                }
            } else {
                let element = self.vector(ty).map_or(ty, |v| v.0);
                let valid = (self.integer_type(element).is_some()
                    || matches!(
                        self.types.nodes[element.index],
                        TypeKind::Scalar(Scalar::Float | Scalar::Double | Scalar::LongDouble)
                    ))
                    // Clang's GNU min/max overloads exclude C99 §6.2.5p2 _Bool.
                    && (!min_max || !matches!(self.types.nodes[element.index], TypeKind::Scalar(Scalar::Bool)))
                    && (!absolute || self.integer_type(element).is_none_or(|(_, signed)| signed))
                    && (!integer || self.integer_type(element).is_some())
                    && (!floating
                        || matches!(
                            self.types.nodes[element.index],
                            TypeKind::Scalar(Scalar::Float | Scalar::Double | Scalar::LongDouble)
                        ))
                    && args
                        .iter()
                        .all(|&arg| self.expression_info(arg).ty.unqualified() == ty.unqualified());
                if valid {
                    return Self::expression_result(e, ty.unqualified());
                }
            }
        }
        self.error(
            SemanticErrorKind::InvalidVectorBuiltin,
            e.source_vectors,
            None,
            None,
        );
        Self::expression_result(e, self.types.unknown())
    }

    /// Checks scalar element eligibility for non-temporal load and store
    /// builtins.
    /// Clang extension: Clang Language Extensions, "Non-temporal load/store
    /// builtins".
    /// <https://clang.llvm.org/docs/LanguageExtensions.html#non-temporal-load-store-builtins>
    fn nontemporal_element(&self, ty: TypeId) -> bool {
        let element = self.vector(ty).map_or(ty, |v| v.0);
        self.integer_type(element).is_some()
            || matches!(
                self.types.nodes[element.index],
                TypeKind::Pointer(_)
                    | TypeKind::Scalar(Scalar::Float | Scalar::Double | Scalar::LongDouble)
            )
    }
}
