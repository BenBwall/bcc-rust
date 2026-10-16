//! Clang's function contracts are checked in the declaring prototype scope.
//! C99: §6.7.5.3p7-8, p. 119; PDF p. 131 (adjusted parameters), extended
//! by <https://clang.llvm.org/docs/AttributeReference.html#pass-object-size-pass-dynamic-object-size>.

use super::super::{
    Analyzer,
    AttributeMessage,
    BinaryOperator,
    CallPredicate,
    Declarator,
    DirectDeclarator,
    Expression,
    ExpressionType,
    FunctionExtension,
    Integer,
    PredicateKind,
    ScopeKind,
    SemanticErrorKind,
    SizeParameter,
    SpecifierExtension,
    TypeKind,
    Work,
};
use crate::{
    translation_phases::{
        parsing::declaration_syntax::TypeQualifiers,
        preprocessing::StringTokenType,
    },
    util::{
        bump::ArenaVec,
        string_cache::StringCacheId,
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Predicate operands require the names of all prototype parameters. The
    /// temporary scope and unevaluated context are restored on the same stack
    /// before binding the function or entering its body.
    pub(in crate::translation_phases::semantic_analysis) fn prepare_function_attributes(
        &mut self,
        declarator: Declarator<'tu>,
        specifiers: Option<&'tu SpecifierExtension<'tu>>,
    ) {
        let lists = self.declaration_attribute_lists(specifiers, Some(declarator));
        let mut operands = ArenaVec::new_in(self.scratch);
        for &list in &lists {
            for attribute in self.written_attributes(list) {
                if super::catalog::rule(
                    list.syntax,
                    attribute.namespace,
                    attribute.name,
                    self.context.configuration.target(),
                )
                .is_none()
                {
                    continue;
                }
                if matches!(attribute.name, "enable_if" | "diagnose_if")
                    && let Some(argument) =
                        list.arguments.iter().find(|a| a.index == attribute.index)
                {
                    operands.push(argument.expression);
                }
            }
        }
        if operands.is_empty() {
            return;
        }
        let prototype = super::super::functions::function_derivation(declarator, self.scratch)
            .and_then(|d| self.parameters.get(&std::ptr::from_ref(d).addr()).copied());
        self.enter(ScopeKind::Prototype);
        self.work.push(Work::PopScope);
        if let Some((_, scope)) = prototype {
            for &index in self.scope_entries[scope].finish(self.scratch, self.scratch) {
                let entry = self.entries[index];
                self.install(entry.name, entry.namespace, entry.binding);
            }
        }
        self.enter_reference_context(None);
        for &expression in operands.iter().rev() {
            self.work.push(Work::Expression(expression));
        }
    }

    pub(in crate::translation_phases::semantic_analysis) fn function_contract(
        &mut self,
        declarator: Declarator<'tu>,
        specifiers: Option<&'tu SpecifierExtension<'tu>>,
        ty: super::super::TypeId,
        definition: bool,
        internal: bool,
    ) -> Option<&'tu FunctionExtension<'tu>> {
        if !matches!(self.types.nodes[ty.index], TypeKind::Function { .. }) {
            return None;
        }
        let name = declarator.identifier()?.name;
        let mut overloadable = false;
        let mut predicates = ArenaVec::new_in(self.scratch);
        let mut unavailable = None;
        for &list in &self.declaration_attribute_lists(specifiers, Some(declarator)) {
            for attribute in self.written_attributes(list) {
                if super::catalog::rule(
                    list.syntax,
                    attribute.namespace,
                    attribute.name,
                    self.context.configuration.target(),
                )
                .is_none()
                {
                    continue;
                }
                match attribute.name {
                    | "overloadable" => {
                        overloadable = true;
                        if attribute.arguments.is_some() {
                            self.contract_error(attribute.source, attribute.name);
                        }
                    },
                    | "unavailable" => {
                        let text = if attribute.arguments.is_none() {
                            self.context.string_cache.intern("function is unavailable")
                        } else if let Some(text) = self.string_argument(attribute) {
                            self.context.string_cache.intern(text)
                        } else {
                            continue;
                        };
                        unavailable = Some(AttributeMessage {
                            text,
                            source: attribute.source,
                        });
                    },
                    | "enable_if" | "diagnose_if" => {
                        let Some(argument) =
                            list.arguments.iter().find(|a| a.index == attribute.index)
                        else {
                            self.contract_error(attribute.source, attribute.name);
                            continue;
                        };
                        let parts = predicate_parts(argument.expression, self.scratch);
                        let count = if attribute.name == "enable_if" { 2 } else { 3 };
                        if parts.len() != count {
                            self.contract_error(attribute.source, attribute.name);
                            continue;
                        }
                        let Some(text) = self.contract_string(parts[1]) else {
                            self.contract_error(attribute.source, attribute.name);
                            continue;
                        };
                        let kind = if count == 2 {
                            PredicateKind::Enable
                        } else {
                            match self
                                .contract_string(parts[2])
                                .map(|text| self.context.string_cache.at(text))
                            {
                                | Some("warning") => PredicateKind::Warning,
                                | Some("error") => PredicateKind::Error,
                                | _ => {
                                    self.contract_error(attribute.source, attribute.name);
                                    continue;
                                },
                            }
                        };
                        let condition = self.expression_info(parts[0]);
                        if !self.types.unanalyzed(condition.ty)
                            && !self.arithmetic(condition.ty)
                            && self.pointer_target(condition.ty).is_none()
                        {
                            self.contract_error(attribute.source, attribute.name);
                            continue;
                        }
                        predicates.push(CallPredicate {
                            parameters: &[],
                            expression: parts[0],
                            kind,
                            message: AttributeMessage {
                                text,
                                source: attribute.source,
                            },
                        });
                    },
                    | _ => {},
                }
            }
        }
        let derivation = super::super::functions::function_derivation(declarator, self.scratch);
        let (parameters, _) = derivation
            .and_then(|d| self.parameters.get(&std::ptr::from_ref(d).addr()).copied())
            .unwrap_or((&[], 0));
        let mut sizes = ArenaVec::new_in(self.scratch);
        if let Some(DirectDeclarator::Function { parameter_list, .. }) = derivation {
            for (index, syntax) in parameter_list.iter().enumerate() {
                for &list in &self.declaration_attribute_lists(
                    syntax.declaration_specifiers.extensions,
                    syntax.declarator,
                ) {
                    for attribute in self.written_attributes(list) {
                        if super::catalog::rule(
                            list.syntax,
                            attribute.namespace,
                            attribute.name,
                            self.context.configuration.target(),
                        )
                        .is_none()
                        {
                            continue;
                        }
                        if !matches!(
                            attribute.name,
                            "pass_object_size" | "pass_dynamic_object_size"
                        ) {
                            continue;
                        }
                        let mode = list
                            .arguments
                            .iter()
                            .find(|a| a.index == attribute.index)
                            .and_then(|a| {
                                let info = self.expression_info(a.expression);
                                info.integer.filter(|_| info.ice)
                            })
                            .and_then(Integer::to_u64)
                            .and_then(|mode| u8::try_from(mode).ok())
                            .filter(|mode| *mode <= 3);
                        let Some(mode) = mode else {
                            self.contract_error(attribute.source, attribute.name);
                            continue;
                        };
                        let Some(parameter) = parameters.get(index) else {
                            continue;
                        };
                        if !matches!(self.types.nodes[parameter.ty.index], TypeKind::Pointer(_))
                            || (definition
                                && !parameter.ty.qualifiers.contains(TypeQualifiers::CONST))
                            || sizes
                                .iter()
                                .any(|size: &SizeParameter| size.index as usize == index)
                        {
                            self.contract_error(attribute.source, attribute.name);
                            continue;
                        }
                        let size = SizeParameter {
                            index: u32::try_from(index).expect("parameter ordinal fits in u32"),
                            mode,
                            dynamic: attribute.name == "pass_dynamic_object_size",
                            source: attribute.source,
                        };
                        sizes.push(size);
                        if let Some(binding) = parameter.binding {
                            _ = self.extension_parameters.insert(binding, size);
                        }
                    }
                }
            }
        }
        if !overloadable && sizes.is_empty() && predicates.is_empty() && unavailable.is_none() {
            return None;
        }
        let parameters = self
            .types
            .tu
            .alloc_slice_fill_iter(parameters.iter().map(|p| p.binding));
        let sizes = self.types.tu.alloc_slice_copy(&sizes);
        for predicate in &mut predicates {
            predicate.parameters = parameters;
        }
        let predicates = self.types.tu.alloc_slice_copy(&predicates);
        let size_type = self.types.scalar(self.types.target.size_t);
        let mut extension = FunctionExtension {
            overloadable,
            size_type,
            identity: name,
            sizes,
            parameters,
            predicates,
            unavailable,
        };
        if overloadable || predicates.iter().any(|p| p.kind == PredicateKind::Enable) {
            extension.identity = self.mangle_function(name, ty, extension, internal);
        }
        Some(self.types.tu.alloc(extension))
    }

    fn contract_error(&mut self, source: crate::translation_phases::SourceVectors, spelling: &str) {
        let name = self.context.string_cache.intern(spelling);
        self.error(
            SemanticErrorKind::AttributeArguments,
            source,
            Some(name),
            None,
        );
    }

    fn contract_string(&mut self, mut expression: &'tu Expression<'tu>) -> Option<StringCacheId> {
        while let ExpressionType::Parenthesized { expression: inner } = expression.kind {
            expression = inner;
        }
        let ExpressionType::StringLiteral(StringTokenType::String(id)) = expression.kind else {
            return None;
        };
        let text = self.context.literal_text_in(self.scratch, id, false)?;
        Some(self.context.string_cache.intern(text))
    }

    pub(in crate::translation_phases::semantic_analysis) fn function_identity(
        &self,
        binding: usize,
    ) -> StringCacheId {
        self.function_extensions
            .function(binding)
            .map_or(self.bindings[binding].name.name, |ext| ext.identity)
    }
}

fn predicate_parts<'a, 'tu>(
    expression: &'tu Expression<'tu>,
    arena: &'a crate::util::bump::Bump,
) -> ArenaVec<'a, &'tu Expression<'tu>> {
    let mut work = ArenaVec::new_in(arena);
    let mut parts = ArenaVec::new_in(arena);
    work.push(expression);
    while let Some(expression) = work.pop() {
        if let ExpressionType::Binary {
            operator: BinaryOperator::Comma,
            left_expression,
            right_expression,
        } = expression.kind
        {
            work.push(right_expression);
            work.push(left_expression);
        } else {
            parts.push(expression);
        }
    }
    parts
}

impl Analyzer<'_, '_, '_> {
    /// Call diagnostics accumulate on compatible redeclarations. Each retained
    /// predicate owns its declaring parameter environment, so a later prototype
    /// may rename its parameters without invalidating an earlier condition.
    pub(in crate::translation_phases::semantic_analysis) fn inherit_call_contract(
        &mut self,
        next: usize,
        name: StringCacheId,
    ) {
        let new = self.function_extensions.function(next);
        let identity = new.map_or(name, |extension| extension.identity);
        let mut previous = self
            .lookup(super::super::Namespace::Ordinary, name)
            .map(|entry| entry.binding);
        while let Some(binding) = previous {
            if self.function_identity(binding) == identity {
                if let Some(old) = self.function_extensions.function(binding) {
                    if let Some(new) = new {
                        let mut predicates = ArenaVec::new_in(self.scratch);
                        predicates.extend(
                            old.predicates
                                .iter()
                                .copied()
                                .filter(|predicate| predicate.kind != PredicateKind::Enable),
                        );
                        predicates.extend_from_slice(new.predicates);
                        let extension = self.types.tu.alloc(FunctionExtension {
                            predicates: self.types.tu.alloc_slice_copy(&predicates),
                            unavailable: new.unavailable.or(old.unavailable),
                            ..*new
                        });
                        _ = self.function_extensions.functions.insert(next, extension);
                    } else if !old.overloadable
                        && old.sizes.is_empty()
                        && !old
                            .predicates
                            .iter()
                            .any(|predicate| predicate.kind == PredicateKind::Enable)
                    {
                        _ = self.function_extensions.functions.insert(next, old);
                        self.correspondence.identify(next, old.identity);
                    }
                }
                break;
            }
            previous = self.overload_previous.get(&binding).copied();
        }
    }
}
