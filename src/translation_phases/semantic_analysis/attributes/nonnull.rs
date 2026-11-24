//! GNU `nonnull`: the parameters of a function that may not be null, and
//! the warnings that depend on them. Both compilers warn of a call that
//! passes a null argument for one (`-Wnonnull`); Clang also warns, inside
//! the function, of comparing such a parameter with a null pointer or
//! converting it to `_Bool` (`always_nonnull.rs`), which needs each
//! attribute and where it was written, in the order Clang keeps them.
//!
//! The compilers read the attribute differently. Clang drops an attribute
//! whose index is out of range, ignores an index of a parameter that is no
//! pointer, and then takes an attribute left without indices for every
//! pointer parameter (`handleNonNullAttr` in `SemaDeclAttr.cpp`); it also
//! accepts the attribute on a parameter, which it then inherits by
//! position (`handleNonNullAttrParameter`, `mergeParamDeclAttributes`).
//! GCC drops an attribute with any index it refuses, and ignores one on a
//! parameter. For `void f(int *p, int n) __attribute__((nonnull(2)));`
//! Clang makes `p` nonnull and GCC nothing.
//! C99: §6.6p6, p. 95; PDF p. 107 (integer constant operands); §6.9.1p10,
//! p. 142; PDF p. 154 (arguments convert to the parameter types).
//! GNU extension: GCC manual, "Common Function Attributes", nonnull.

use super::{
    super::{
        Analyzer,
        AttributeSpecifier,
        BindingKind,
        ExpressionInfo,
        Integer,
        SemanticErrorKind,
        always_nonnull::NonnullNote,
        functions::function_derivation,
        types::TypeKind,
    },
    entities::Subject,
    written::WrittenAttribute,
};
use crate::{
    configuration::{
        DiagnosticGroup,
        RuleSet,
    },
    translation_phases::{
        Context,
        SourceVectors,
        parsing::{
            declaration_syntax::Declarator,
            syntax::{
                BinaryOperator as B,
                Expression,
                ExpressionType as E,
                UnaryOperator as U,
            },
        },
    },
    util::{
        arena_list::ArenaList,
        bump::ArenaVec,
    },
};

/// One GNU `nonnull` attribute as Clang keeps it (`NonNullAttr`): where it
/// was written, and the zero-based positions of the parameters it names,
/// none for every pointer parameter. One written on a parameter names that
/// parameter alone.
/// GNU extension: GCC manual, "Common Function Attributes", nonnull.
#[derive(Debug, Clone, Copy)]
pub(in crate::translation_phases::semantic_analysis) struct NonnullAttribute<'s> {
    site:      SourceVectors,
    positions: &'s [usize],
    parameter: bool,
}

impl<'tu, 's> Analyzer<'_, 'tu, 's> {
    /// Applies a `nonnull` written on a function or, for Clang, on a
    /// parameter. A function keeps the union of its attributes' positions,
    /// which GCC's checks and the sanitizer read, and each attribute,
    /// which Clang's read.
    pub(super) fn nonnull_attribute(
        &mut self,
        subject: Subject,
        specifier: &'tu AttributeSpecifier<'tu>,
        attribute: WrittenAttribute<'_>,
    ) {
        let clang = self.context.configuration.compiler_identity().rules() == RuleSet::Clang;
        let bare = attribute.arguments.is_none_or(<[_]>::is_empty);
        let binding = match subject {
            | Subject::Function { binding, .. } => binding,
            // An unnamed parameter's type is checked where it is known.
            | Subject::Parameter(binding)
                if clang
                    && bare
                    && binding.is_none_or(|binding| {
                        self.pointer_target(self.bindings[binding].ty).is_some()
                    }) =>
            {
                let site = &mut self.attributes.parameter_nonnull;
                *site = site.or(Some(attribute.source));
                return;
            },
            | _ => {
                self.ignored(attribute);
                return;
            },
        };
        let TypeKind::Function {
            parameters,
            variadic,
            ..
        } = self.types.nodes[self.bindings[binding].ty.index]
        else {
            return;
        };
        let mut positions = ArenaVec::new_in(self.scratch);
        let mut work: ArenaVec<'_, &'tu Expression<'tu>> = ArenaVec::new_in(self.scratch);
        work.extend(
            specifier
                .arguments
                .iter()
                .filter(|argument| argument.index == attribute.index)
                .map(|argument| argument.expression),
        );
        let mut refused = false;
        while let Some(expression) = work.pop() {
            if let E::Binary {
                operator: B::Comma,
                left_expression,
                right_expression,
            } = expression.kind
            {
                work.push(right_expression);
                work.push(left_expression);
                continue;
            }
            let info = self.expression_info(expression);
            let position = info
                .integer
                .filter(|_| info.ice)
                .and_then(Integer::to_i128)
                .and_then(|value| usize::try_from(value).ok())
                .and_then(|value| value.checked_sub(1));
            let Some(position) =
                position.filter(|&position| position < parameters.len() || variadic)
            else {
                self.error(
                    SemanticErrorKind::AttributeArguments,
                    expression.source_vectors,
                    None,
                    None,
                );
                // Clang stops at the first index out of range.
                if clang {
                    return;
                }
                refused = true;
                continue;
            };
            if parameters
                .get(position)
                .is_some_and(|&parameter| self.pointer_target(parameter).is_none())
            {
                self.ignored(attribute);
                refused = true;
                continue;
            }
            if !positions.contains(&position) {
                positions.push(position);
            }
        }
        // GCC drops the whole attribute.
        if refused && !clang {
            return;
        }
        positions.sort_unstable();
        let entity = self.entity_of(binding);
        let attributes = self.attributes.entity(entity);
        attributes.nonnull_site = attributes.nonnull_site.or(Some(attribute.source));
        let every = positions.is_empty() && (bare || clang);
        let retained = attributes.nonnull;
        if every {
            attributes.nonnull = Some(&[]);
        } else if retained.is_none_or(|retained| !retained.is_empty()) && !positions.is_empty() {
            let mut union = ArenaVec::new_in(self.scratch);
            union.extend_from_slice(retained.unwrap_or_default());
            for &position in &positions {
                if !union.contains(&position) {
                    union.push(position);
                }
            }
            let union = self.types.tu.alloc_slice_copy(&union);
            self.attributes.entity(entity).nonnull = Some(union);
        }
        if every || !positions.is_empty() {
            let positions = self.scratch.alloc_slice_copy(&positions);
            self.keep_nonnull(
                entity,
                NonnullAttribute {
                    site: attribute.source,
                    positions,
                    parameter: false,
                },
            );
        }
    }

    /// Keeps the `nonnull` written on each parameter of the function
    /// declaration `binding` that `declarator` declares, as Clang's
    /// parameters inherit theirs from the declaration before, by position.
    pub(in crate::translation_phases::semantic_analysis) fn keep_parameter_nonnull(
        &mut self,
        binding: usize,
        declarator: Declarator<'tu>,
    ) {
        let Some(derivation) = function_derivation(declarator, self.scratch) else {
            return;
        };
        let Some(&(parameters, _)) = self.parameters.get(&std::ptr::from_ref(derivation).addr())
        else {
            return;
        };
        let entity = self.entity_of(binding);
        for (position, parameter) in parameters.iter().enumerate() {
            if let Some(site) = parameter.nonnull {
                let positions = self.scratch.alloc_slice_copy(&[position]);
                self.keep_nonnull(
                    entity,
                    NonnullAttribute {
                        site,
                        positions,
                        parameter: true,
                    },
                );
            }
        }
    }

    fn keep_nonnull(&mut self, entity: super::super::facts::EntityId, kept: NonnullAttribute<'s>) {
        let earlier = self
            .attributes
            .nonnull
            .get(&entity)
            .copied()
            .unwrap_or_default();
        let mut all = ArenaVec::with_capacity_in(earlier.len() + 1, self.scratch);
        all.extend_from_slice(earlier);
        all.push(kept);
        _ = self.attributes.nonnull.insert(entity, all.leak());
    }

    /// Where the `nonnull` that makes the parameter `binding` nonnull for
    /// Clang was written, inside a function body, while Clang still assumes
    /// so (`DiagnoseAlwaysNonNullPointer` in `SemaChecking.cpp`). For a
    /// parameter of the function being defined: one on the parameter
    /// itself first, else the first of the function's that names it or
    /// every pointer parameter. A parameter of a prototype, such as the
    /// `p` of `void g(int *p, int a[p == 0]);` in a body, belongs to no
    /// function yet, so only one written on it counts.
    pub(in crate::translation_phases::semantic_analysis) fn nonnull_parameter(
        &mut self,
        binding: usize,
    ) -> Option<SourceVectors> {
        if self.attributes.modified_parameters.contains(&binding) {
            return None;
        }
        let function = self.functions.current?;
        // The function's parameters are bound in its own scope, where
        // their names are distinct.
        if self.bindings[binding].scope != function.scope {
            return self.attributes.own_nonnull.get(&binding).copied();
        }
        let name = self.bindings[binding].name.name;
        let position = function
            .parameters
            .iter()
            .position(|parameter| parameter.name.is_some_and(|written| written.name == name))?;
        let entity = self.entity_of(function.binding?);
        let kept = self.attributes.nonnull.get(&entity).copied()?;
        kept.iter()
            .find(|kept| kept.parameter && kept.positions == [position])
            .or_else(|| {
                kept.iter().find(|kept| {
                    !kept.parameter
                        && (kept.positions.is_empty() || kept.positions.contains(&position))
                })
            })
            .map(|kept| kept.site)
    }

    /// Where `returns_nonnull` was written on the function `binding`, if it
    /// was.
    pub(in crate::translation_phases::semantic_analysis) fn returns_nonnull_site(
        &mut self,
        binding: usize,
    ) -> Option<SourceVectors> {
        let entity = self.entity_of(binding);
        self.attributes
            .entities
            .get(&entity)
            .and_then(|attributes| attributes.returns_nonnull)
    }

    /// Records the parameter that `e`, an `&` or an assignment, names as
    /// its operand or left operand, as written, without parentheses: Clang
    /// no longer assumes it nonnull (`RecordModifiableNonNullParam` in
    /// `SemaExpr.cpp`). A compound assignment does not count.
    pub(in crate::translation_phases::semantic_analysis) fn note_nonnull_change(
        &mut self,
        e: &'tu Expression<'tu>,
    ) {
        let (E::Unary {
            operator: U::AddressOf,
            operand_expression: operand,
        }
        | E::Binary {
            operator: B::Assignment,
            left_expression: operand,
            ..
        }) = e.kind
        else {
            return;
        };
        if let E::Identifier(_) = operand.kind
            && let Some(binding) = self.expression_info(operand).binding
            && self.bindings[binding].kind == BindingKind::Parameter
        {
            _ = self.attributes.modified_parameters.insert(binding);
        }
    }

    /// `-Wnonnull` at the call `call` of `function`: an argument whose value
    /// is a null pointer, or zero, where the declaration's `nonnull` wants a
    /// pointer. Clang warns where control may reach the call
    /// (`CheckNonNullArguments` in `SemaChecking.cpp`, through
    /// `DiagRuntimeBehavior`); GCC as it builds it (`check_nonnull_arg` in
    /// `c-common.cc`).
    /// GNU extension: GCC manual, "Common Function Attributes", nonnull.
    pub(in crate::translation_phases::semantic_analysis) fn check_nonnull_arguments(
        &mut self,
        call: &'tu Expression<'tu>,
        function: ExpressionInfo<'tu>,
        arguments: ArenaList<'tu, &'tu Expression<'tu>>,
    ) {
        let Some(binding) = function.binding else {
            return;
        };
        if self.context.configuration.compiler_identity().rules() == RuleSet::Clang {
            self.clang_nonnull_arguments(call, binding, arguments);
        } else {
            self.gcc_nonnull_arguments(function, binding, arguments);
        }
    }

    /// Clang's check: an attribute for every pointer parameter checks the
    /// arguments of pointer type and reports at the call's `)`; otherwise
    /// each argument a `nonnull` names is reported where it is.
    fn clang_nonnull_arguments(
        &mut self,
        call: &'tu Expression<'tu>,
        binding: usize,
        arguments: ArenaList<'tu, &'tu Expression<'tu>>,
    ) {
        let entity = self.entity_of(binding);
        let Some(kept) = self.attributes.nonnull.get(&entity).copied() else {
            return;
        };
        let close = Context::last_source_vector(
            call.operator_source_vectors.unwrap_or(call.source_vectors),
        );
        let mut named = ArenaVec::new_in(self.scratch);
        named.resize(arguments.len(), false);
        for kept in kept.iter().filter(|kept| !kept.parameter) {
            if kept.positions.is_empty() {
                for (index, &argument) in arguments.iter().enumerate() {
                    if self.pointer_argument(binding, index, argument) {
                        self.clang_null_argument(call, argument, close);
                    }
                }
                return;
            }
            for &position in kept.positions {
                if let Some(slot) = named.get_mut(position) {
                    *slot = true;
                }
            }
        }
        for kept in kept.iter().filter(|kept| kept.parameter) {
            if let Some(slot) = named.get_mut(kept.positions[0]) {
                *slot = true;
            }
        }
        for (index, &argument) in arguments.iter().enumerate() {
            if named[index] {
                self.clang_null_argument(call, argument, Self::clang_location(argument));
            }
        }
    }

    /// Reports `argument` at `site`, with its range, where control may
    /// reach `call`, if its value is null (`CheckNonNullExpr`).
    pub(in crate::translation_phases::semantic_analysis) fn clang_null_argument(
        &mut self,
        call: &'tu Expression<'tu>,
        argument: &'tu Expression<'tu>,
        site: SourceVectors,
    ) {
        if !self.null_value(self.expression_info(argument)) {
            return;
        }
        let name = self
            .context
            .string_cache
            .intern("null passed to a callee that requires a non-null argument");
        let group = DiagnosticGroup::Nonnull;
        self.with_highlights(
            [argument.source_vectors, SourceVectors::default()],
            |analyzer| {
                analyzer.warn_if_reachable(
                    call,
                    group,
                    SemanticErrorKind::Nonnull(group, NonnullNote::Plain),
                    site,
                    Some(name),
                );
            },
        );
    }

    /// GCC's check: each argument of pointer type that the union of the
    /// function's attributes names, reported at the called function's name
    /// with a note at its latest declaration.
    fn gcc_nonnull_arguments(
        &mut self,
        function: ExpressionInfo<'tu>,
        binding: usize,
        arguments: ArenaList<'tu, &'tu Expression<'tu>>,
    ) {
        let entity = self.entity_of(binding);
        let Some(positions) = self
            .attributes
            .entities
            .get(&entity)
            .and_then(|attributes| attributes.nonnull)
        else {
            return;
        };
        let callee = Self::without_parentheses(function.expression);
        let site = match callee.kind {
            | E::Identifier(name) if callee.source_vectors == function.expression.source_vectors =>
                name.source_vectors,
            | _ => Context::first_token_vector(function.expression.source_vectors),
        };
        let declared = self.bindings[binding].name;
        for (index, &argument) in arguments.iter().enumerate() {
            if (positions.is_empty() || positions.contains(&index))
                && self.pointer_argument(binding, index, argument)
                && self.null_value(self.expression_info(argument))
            {
                let text = crate::diagnostics::format_in!(
                    self.scratch,
                    "argument {} null where non-null expected",
                    index + 1
                );
                let name = self.context.string_cache.intern(text);
                self.group_diagnostic(
                    DiagnosticGroup::Nonnull,
                    SemanticErrorKind::Nonnull(DiagnosticGroup::Nonnull, NonnullNote::GccCall),
                    site,
                    Some(name),
                    Some(declared.source_vectors),
                );
            }
        }
    }

    /// Whether argument `index` of a call to `binding` is a pointer once
    /// converted: to its parameter's type, or as itself past them.
    /// C99: §6.5.2.2p7, p. 72; PDF p. 84.
    fn pointer_argument(
        &mut self,
        binding: usize,
        index: usize,
        argument: &'tu Expression<'tu>,
    ) -> bool {
        let parameter = match self.types.nodes[self.bindings[binding].ty.index] {
            | TypeKind::Function { parameters, .. } => parameters.get(index).copied(),
            | _ => None,
        };
        let ty = match parameter {
            | Some(parameter) => parameter,
            | None => {
                let info = self.expression_info(argument);
                self.converted(info)
            },
        };
        self.pointer_target(ty).is_some()
    }

    /// Whether `info`'s value is known to be null or zero, as Clang's
    /// `EvaluateAsBooleanCondition` and GCC's `integer_zerop` find.
    fn null_value(&self, info: ExpressionInfo<'tu>) -> bool {
        self.null_pointer_constant(info)
            || info.integer.is_some_and(Integer::is_zero)
                && matches!(
                    info.constant,
                    super::super::ConstantClass::Arithmetic | super::super::ConstantClass::Address
                )
    }
}
