//! Semantic analysis checks the finished syntax tree and retains its types,
//! bindings, scopes, expression results and definitions. Each external
//! declaration starts one explicit stack of continuations. The driver pops a
//! continuation, performs its step and pushes any remaining work. Scratch maps
//! and stacks are dropped after translation-unit checks; the retained results
//! stay in the translation-unit arena. Semantic lookup is independent of the
//! parser's typedef lookup.
//!
//! For `int x = 1;`, the loop first resolves `int`, constructs the declarator
//! and installs `x` in file scope. It then visits the initializer's literal,
//! records its type and checks the conversion to `x`'s type. Translation-unit
//! completion checks the definition before returning the retained graph.
//!
//! Read [`analyze`], [`Analyzer::step`], [`Work`] and [`Analyzer`] first. Then
//! follow the dispatched method into the file for that concern.
//! [`SemanticTranslationUnit`] describes what callers retain;
//! [`SemanticTranslationUnit::inspect`] shows it. The `pub(crate)` re-exports
//! below name the retained vocabulary that lowering matches on.
//!
//! - Driver storage and names: `state.rs` initializes the analyzer and checks
//!   stack balance; `collection.rs` builds scratch lists; `scopes.rs` installs,
//!   finds and restores names; `results.rs` defines the retained output.
//! - Syntax and constraints: `traversal.rs` schedules child syntax;
//!   `declarations.rs` constructs types and bindings; `expressions.rs` and
//!   `expressions/` type expressions and conversions; `initializers.rs` walks
//!   subobjects; `statements.rs` checks jumps and returns; `functions.rs`
//!   checks definitions and completes the translation unit.
//! - Retained facts: `facts/` records lexical/VM/block-expression regions and
//!   resolved destinations on the same work stack, then finalizes compact
//!   transitions using iterative heavy-light indexing. It also retains
//!   declaration/parameter correspondence, typed bound sites, size DAGs,
//!   expression effect summaries and effects-only evaluation recipes.
//! - Types and values: `types.rs` interns and compares types and computes
//!   layouts; `integer.rs` evaluates integer constants; `constants.rs` folds
//!   floating values.
//! - Language extensions: `generic.rs` selects C11 generic associations;
//!   `type_generic.rs` checks GNU type selections and math calls; `vectors.rs`
//!   constructs vector types and checks operators; `attributes.rs` retains
//!   packing, alignment and scalar machine modes; `bound_sites.rs` retains
//!   typeof operand execution at written type sites; `builtins.rs` and
//!   `builtins/` recognize and check header intrinsics, atomics and x86 calls.
//! - Diagnostics and inspection: `diagnostics.rs` reports constraints and
//!   extensions; `errors.rs` defines diagnostic kinds and renders them;
//!   `inspection.rs` formats retained results. `tests.rs` and `tests/` exercise
//!   these paths.
//!
//! C99: §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22 (translation phase 7).
//! C99: §6.2.1-§6.2.4, pp. 29-32; PDF pp. 41-44 (scopes, linkage and duration).
//! C99: §6.7-§6.7.7, pp. 97-124; PDF pp. 109-136 (declarations and type names).
//! C99: §6.3, pp. 42-48; PDF pp. 54-60; §6.5, pp. 67-94; PDF pp. 79-106
//! (conversions and expressions).
//! C99: §6.7.8, pp. 125-130; PDF pp. 137-142 (initializers).
//! C99: §6.8-§6.9.2, pp. 131-144; PDF pp. 143-156 (statements and definitions).
//! Backend control-flow and emitted code are not constructed.

// Driver storage and names
mod collection;
mod results;
mod scopes;
mod state;

// Syntax traversal and constraints
mod declarations;
mod expressions;
mod facts;
mod functions;
mod initializers;
mod predefined;
mod statements;
mod traversal;

// Types and constant values
mod constants;
mod integer;
mod types;

// Language extensions and builtin calls
mod attributes;
mod bound_sites;
mod builtins;
mod generic;
mod packing;
mod type_generic;
mod vectors;

// Diagnostics and inspection
mod diagnostics;
mod errors;
mod inspection;

#[cfg(test)]
use std::cell::Cell;

pub(crate) use builtins::{
    implemented_builtin,
    named_builtin,
};
use collection::Collection;
pub(crate) use constants::Floating;
use declarations::compatible_enum_type;
pub(crate) use errors::{
    SemanticError,
    SemanticErrorKind,
};
pub(crate) use expressions::{
    ConstantClass,
    Conversion,
    ConversionKind,
    ExpressionInfo,
    ValueCategory,
};
pub(crate) use facts::*;
pub(crate) use integer::Integer;
pub(crate) use results::{
    Binding,
    BindingKind,
    BindingOrigin,
    BoundSiteFacts,
    Definition,
    DefinitionKind,
    Duration,
    FunctionRecord,
    Linkage,
    Scope,
    ScopeKind,
    SemanticTranslationUnit,
};
use rustc_hash::FxBuildHasher;
use scopes::{
    Entry,
    Namespace,
};
use types::TypeInterner;
#[expect(
    unused_imports,
    reason = "The retained semantic vocabulary is the crate interface for lowering and inspection."
)]
pub(crate) use types::{
    ArrayBound,
    Field,
    FieldPath,
    Layout,
    Member,
    Parameter,
    Scalar,
    Tag,
    TagKind,
    TypeId,
    TypeKind,
    Types,
};

use super::{
    Context,
    DiagnosticPolicy,
    SourceVectors,
    parsing::{
        AttributeSpecifier,
        AttributeSyntax,
        ExtendedType,
        ParsedTranslationUnit,
        SpecifierExtension,
        SpecifierExtensionKind,
        SyntaxOperand,
        declaration_syntax::{
            Declaration,
            DeclarationSpecifiers,
            Declarator,
            DesignatorType,
            DirectDeclarator,
            Enumerator,
            Initializer,
            InitializerType,
            ParameterDeclaration,
            StructDeclaration,
            StructDeclarator,
            TypeName,
            TypeQualifiers,
            TypeSpecifiers,
        },
        syntax::{
            BinaryOperator,
            BlockItem,
            ConditionalExpression,
            Constant,
            Expression,
            ExpressionSlot,
            ExpressionType,
            ExternalDeclaration,
            FunctionDefinition,
            Identifier,
            Statement,
            StorageClass,
            UnaryOperator,
        },
    },
};
use crate::{
    configuration::CStandard,
    util::{
        arena_list::ArenaList,
        bump::{
            ArenaMap,
            ArenaVec,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

/// Runs declaration analysis only after the complete immutable syntax tree
/// exists. C99: §5.1.1.2p1, pp. 9-10; PDF pp. 21-22.
pub(crate) fn analyze<'tu>(
    context: &mut Context<'tu>,
    unit: &ParsedTranslationUnit<'tu>,
) -> SemanticTranslationUnit<'tu> {
    let scratch = Bump::new();
    let mut analyzer = Analyzer::new(context, &scratch);
    for &root in unit.external_declarations() {
        analyzer.assert_balanced();
        analyzer.work.push(Work::Root(root));
        while let Some(work) = analyzer.work.pop() {
            analyzer.step(work);
        }
    }
    analyzer.assert_balanced();
    analyzer.finish_translation_unit();
    analyzer.check_builtin_designators();
    let index = analyzer.retain_expression_index();
    let sizes = SizeFacts::retain(&analyzer.types, &scratch);
    analyzer.retain_runtime_bounds(&sizes);
    let type_names =
        analyzer
            .types
            .tu
            .alloc_slice_fill_iter(analyzer.type_names.iter().enumerate().map(
                |(index, &(syntax, resolved))| {
                    let id = TypeNameId(index);
                    _ = analyzer
                        .retained_type_names
                        .insert(type_name_identity(syntax), id);
                    let size_query = analyzer
                        .type_name_sites
                        .get(&type_name_identity(syntax))
                        .copied()
                        .map(|site| SizeQueryFacts {
                            site,
                            formula: sizes.formula(resolved),
                            operand: None,
                        });
                    TypeNameFacts {
                        id,
                        syntax,
                        resolved,
                        size_query,
                    }
                },
            ));
    SemanticTranslationUnit {
        resolved: analyzer.resolved.retain(),
        correspondence: analyzer.correspondence.retain(),
        sizes,
        runtime_bounds: analyzer.runtime_bounds.leak(),
        control: analyzer.control.retain(analyzer.types.tu),
        effects: analyzer.effects.retain(),
        expression_actions: analyzer.expression_actions,
        declarators: analyzer.declarators.leak(),
        spec_extensions: analyzer.spec_extensions.leak(),
        expressions: analyzer.expressions.leak(),
        expression_indices: index.expressions,
        string_initializers: analyzer.string_initializers,
        conversions: index.conversions,
        conversion_starts: index.conversion_starts,
        types: analyzer.types.finish(),
        bindings: analyzer.bindings.leak(),
        definitions: analyzer.definitions.leak(),
        functions: analyzer.function_records.leak(),
        scopes: analyzer.scopes.leak(),
        type_names,
        type_name_indices: analyzer.retained_type_names,
        declaration_sites: analyzer.declaration_sites,
        size_query_sites: analyzer.size_query_sites,
        tag_declarations: analyzer.tag_declarations.leak(),
        errors: analyzer.error_diagnostics,
    }
}

impl<'tu, 's> Analyzer<'_, 'tu, 's> {
    /// Iterative declaration/type/constant continuations.
    /// C99: §6.6, pp. 95-96; PDF pp. 107-108; §6.7, pp. 97-124; PDF pp.
    /// 109-136.
    #[expect(
        clippy::too_many_lines,
        reason = "The single iterative driver dispatches typed continuations; algorithms live in \
                  concern modules."
    )]
    fn step(&mut self, work: Work<'tu, 's>) {
        match work {
            | Work::Root(root) => match root {
                | ExternalDeclaration::Declaration(d)
                | ExternalDeclaration::RecoveredDeclaration(d) =>
                    self.work.push(Work::Declaration(d)),
                | ExternalDeclaration::FunctionDefinition(f)
                | ExternalDeclaration::RecoveredFunctionDefinition(f) =>
                    self.work.push(Work::Function(f)),
                | ExternalDeclaration::Error(_) | ExternalDeclaration::Asm(_) => {},
            },
            | Work::RestoreTaint(old) => self.tainted = old,
            | Work::RestoreParameterMode(old) => self.old_parameter_mode = old,
            | Work::RuntimeBound(runtime) => self.runtime_bound = runtime,
            | Work::PopScope => self.leave(),
            | Work::FunctionWork(work) => self.function_work(work),
            | Work::StatementWork(work) => self.statement_work(work),
            | Work::DiscardType => {
                _ = self.values.pop();
            },
            | Work::UnknownType => {
                _ = self.take_type();
                self.values.push(self.types.unknown());
            },
            | Work::Declaration(d) => {
                _ = self.declaration_sites.insert(
                    std::ptr::from_ref(d).addr(),
                    BoundSiteFacts {
                        bounds:              &[],
                        ty:                  self.types.unknown(),
                        operand_evaluations: &[],
                    },
                );
                if let Some(assertion) = d.assertion {
                    self.taint(assertion.recovered);
                    self.work.push(Work::RequireConstant(
                        assertion.expression,
                        Some(assertion),
                        self.semantic_errors,
                    ));
                    self.work.push(Work::Eval(assertion.expression));
                    self.work.push(Work::Expression(assertion.expression));
                    return;
                }
                self.taint(d.recovered);
                self.validate_declaration_list_item(d);
                self.work.push(Work::DeclarationBase(d));
                self.spec(d.declaration_specifiers, d.init_declarators.is_empty());
            },
            | Work::DeclarationBase(d) => {
                let base = self.take_type();
                if let Some(operand_evaluations) =
                    self.spec_evaluations(d.declaration_specifiers.type_specifiers)
                {
                    _ = self.declaration_sites.insert(
                        std::ptr::from_ref(d).addr(),
                        BoundSiteFacts {
                            bounds: self.spec_bounds(d.declaration_specifiers.type_specifiers),
                            ty: base,
                            operand_evaluations,
                        },
                    );
                } else {
                    _ = self.declaration_sites.remove(&std::ptr::from_ref(d).addr());
                }
                if d.init_declarators.is_empty()
                    || matches!(
                        d.declaration_specifiers.storage_class,
                        Some(StorageClass::Typedef | StorageClass::Register)
                    )
                {
                    self.reject_standard_alignment(d.declaration_specifiers.extensions);
                }
                for init in d.init_declarators.iter().rev() {
                    if let Some(initializer) = init.initializer {
                        self.work
                            .push(Work::InitializeDeclaration(init.declarator, initializer));
                    }
                    self.work.push(Work::DeclarationDeclarator(
                        init.declarator,
                        d.declaration_specifiers,
                        base,
                        init.initializer.is_some(),
                    ));
                }
            },
            | Work::DeclarationDeclarator(d, spec, base, initialized) => {
                self.work.push(Work::Bind(
                    d,
                    spec,
                    initialized,
                    self.types.runtime_bounds.len(),
                ));
                self.work.push(Work::DeclaredAlignment(
                    spec.extensions,
                    base,
                    spec.storage_class == Some(StorageClass::Typedef),
                ));
                self.work.push(Work::Declarator(d, base, false));
            },
            | Work::Bind(d, s, initialized, bound_start) => {
                let ty = self.take_type();
                let prior = self.bindings.len();
                let errors_before = self.semantic_errors;
                self.bind_declaration(d, s, ty, initialized);
                let bounds = self.bound_actions(ty, bound_start);
                let binding = (self.bindings.len() > prior).then(|| self.bindings.len() - 1);
                if let Some(binding) = binding {
                    _ = self.binding_bounds.insert(binding, bounds);
                }
                let accepted = binding.filter(|&binding| {
                    if self.semantic_errors != errors_before || self.tainted {
                        return false;
                    }

                    d.identifier().is_some_and(|name| {
                        self.lookup(Namespace::Ordinary, name.name)
                            .is_some_and(|entry| entry.binding == binding)
                    })
                });
                let parameters = self.prototype_parameter_facts(d);
                let id = self.correspondence.declaration(
                    d,
                    accepted.map(|i| (i, self.bindings[i])),
                    BoundSiteFacts {
                        ty,
                        bounds,
                        operand_evaluations: &[],
                    },
                    parameters,
                );
                self.record_bound_origins(bounds, BoundEvaluationSite::Declaration(id));
            },
            | Work::Function(f) => {
                self.taint(f.recovered);
                self.prepare_function(f);
                self.work.push(Work::FunctionBase(f));
                self.spec(f.declaration_specifiers, false);
            },
            | Work::FunctionBase(f) => {
                let base = self.take_type();
                self.work.push(Work::FunctionBody(f));
                self.work.push(Work::Declarator(f.declarator, base, false));
            },
            | Work::FunctionBody(f) => self.function_body(f),
            | Work::OldSignature(f) => self.old_signature(f),
            | Work::BlockItem(item) => match item {
                | BlockItem::Declaration(d) => {
                    self.unsummarized_block_item();
                    self.work.push(Work::Declaration(d));
                },
                | BlockItem::Statement(s) => self.work.push(Work::Statement(s, true)),
                | BlockItem::FunctionDefinition(f) => {
                    self.unsummarized_block_item();
                    self.work.push(Work::Function(f));
                },
            },
            | Work::Statement(statement, new_scope) => self.statement(statement, new_scope),
            | Work::Expression(expression) => self.expression(expression),
            | Work::ExpressionDone(e) => self.expression_done(e),
            | Work::ValueExpression(e) => {
                let info = self.expression_info(e);
                _ = self.converted(info);
            },
            | Work::CompoundInitializer(e, initializer) => {
                self.work.push(Work::ExpressionDone(e));
                self.work.push(Work::Initializer(initializer));
            },
            | Work::InitializeDeclaration(declarator, initializer) => {
                self.correspondence.initializer(declarator, initializer);
                if let Some(entry) = declarator
                    .identifier()
                    .and_then(|n| self.lookup(Namespace::Ordinary, n.name))
                {
                    self.work
                        .push(Work::InitializeDone(entry.binding, initializer));
                }
                self.work.push(Work::Initializer(initializer));
            },
            | Work::InitializeDone(binding, initializer) =>
                self.initialize_declaration(binding, initializer),
            | Work::Condition(slot, integer) => self.check_condition(slot, integer),
            | Work::RequireConstant(e, assertion, before) => {
                let value = self.integers.pop().flatten();
                if self.semantic_errors == before && !self.unanalyzed_constant(e) {
                    if value.is_none() {
                        self.error(
                            SemanticErrorKind::InvalidConstant,
                            e.source_vectors,
                            None,
                            None,
                        );
                    } else if let Some(assertion) = assertion
                        && value.is_some_and(|v| v.value == 0)
                    {
                        let message = assertion.message.map(|token| {
                            let text = match token.kind {
                                | super::preprocessing::TokenType::String(
                                    super::preprocessing::StringTokenType::String(id),
                                ) => self.context.literal_text_in(self.scratch, id, false),
                                | _ => None,
                            };
                            text.map_or(token.contents, |text| {
                                self.context.string_cache.intern(text)
                            })
                        });
                        self.error(
                            SemanticErrorKind::FailedAssertion,
                            e.source_vectors,
                            message,
                            None,
                        );
                    }
                }
            },
            | Work::Slot(slot) => self.expression_slot(slot),
            | Work::Initializer(initializer) => {
                self.taint(initializer.recovered);
                match initializer.kind {
                    | InitializerType::AssignmentExpression(e) =>
                        self.work.push(Work::Expression(e)),
                    | InitializerType::InitializerList(list) => {
                        for element in list.elements.iter().rev() {
                            self.work.push(Work::Initializer(element.initializer));
                            if let Some(designation) = element.designation {
                                for designator in designation.designators.iter().rev() {
                                    match designator.kind {
                                        | DesignatorType::Array(e) =>
                                            self.work.push(Work::Expression(e.expression())),
                                        | DesignatorType::Range(r) => {
                                            self.work.push(Work::Expression(r.upper.expression()));
                                            self.work.push(Work::Expression(r.lower.expression()));
                                        },
                                        | _ => {},
                                    }
                                }
                            }
                        }
                    },
                }
            },
            | Work::TypeofDone(syntax) => self.finish_typeof(syntax),
            | Work::VectorAttributes(chain, storage_alignment) => {
                self.work
                    .push(Work::VectorAttributesDone(chain, storage_alignment));
                self.attribute_chain_operands(chain);
            },
            | Work::VectorAttributesDone(chain, storage_alignment) => {
                let base = self.take_type();
                let ty = self.vector_attribute_chain(base, chain, storage_alignment);
                self.values.push(ty);
            },
            | Work::DeclaredAlignment(chain, base, can_weaken) => {
                let ty = self.take_type();
                let attributes = self.layout_attribute_chain(chain);
                let ty = self.align_declared_type(ty, attributes.alignment);
                let ty = self.minimum_declared_alignment(ty, base, can_weaken);
                let ty = self.standard_alignment(ty, chain);
                self.values.push(ty);
            },
            | Work::VectorDeclaratorAttributes(list) => {
                let mut ty = self.take_type();
                for direct in list {
                    if let DirectDeclarator::Attributes(a) = direct {
                        ty = self.mode_attribute(ty, a);
                    }
                }
                for construction in [true, false] {
                    for direct in list {
                        if let DirectDeclarator::Attributes(a) = direct
                            && vectors::constructs_vector(a, self.context) == construction
                        {
                            ty = if self.layout_attribute(a) {
                                self.types.unknown()
                            } else {
                                self.vector_attribute(ty, a, true)
                            };
                        }
                    }
                }
                self.values.push(ty);
            },
            | Work::Spec(s, q, source, force) => {
                self.work.push(Work::SpecDone(s, q, source, force));
                let attributes = match s {
                    | TypeSpecifiers::StructOrUnion(record) => record.attributes,
                    | TypeSpecifiers::Enum(enumeration) => enumeration.attributes,
                    | _ => None,
                };
                self.attribute_chain_operands(attributes);
            },
            | Work::SpecDone(s, q, source, force) => self.resolve_spec(s, q, source, force),
            | Work::Atomic(source) => {
                let value = self.take_type();
                let ty = self.atomic_type(value, source, true);
                self.values.push(ty);
            },
            | Work::Qualify(q, source) => {
                let mut ty = self.take_type();
                let q = if q.contains(TypeQualifiers::ATOMIC) {
                    ty = self.atomic_type(ty, source, false);
                    q - TypeQualifiers::ATOMIC
                } else {
                    q
                };
                if !(q
                    - (TypeQualifiers::CONST | TypeQualifiers::VOLATILE | TypeQualifiers::RESTRICT))
                    .is_empty()
                {
                    self.values.push(self.types.unknown());
                    return;
                }
                let mut arrays = ArenaVec::new_in(self.scratch);
                while let TypeKind::Array(element, bound) = self.types.nodes[ty.index] {
                    arrays.push((bound, ty.alignment));
                    ty = element;
                }
                if matches!(self.types.nodes[ty.index], TypeKind::Function { .. }) && !q.is_empty()
                {
                    self.validate_qualifiers(ty.qualified(q), source);
                    self.error(SemanticErrorKind::QualifiedFunction, source, None, None);
                } else {
                    ty = ty.qualified(q);
                    // C99 §6.7.3p2,p8: an array typedef qualifies its
                    // element, which must itself admit restrict.
                    self.validate_qualifiers(ty, source);
                }
                while let Some((bound, alignment)) = arrays.pop() {
                    ty = self
                        .types
                        .intern(TypeKind::Array(ty, bound))
                        .with_alignment(alignment);
                }
                self.values.push(ty);
            },
            | Work::TypeName(name) => {
                if let Some(site) = self.type_name_sites.get(&type_name_identity(name)) {
                    self.values.push(site.ty);
                    return;
                }
                self.work.push(Work::TypeNameBase(name));
                self.spec(name.declaration_specifiers, false);
            },
            | Work::TypeNameBase(name) => {
                let base = self.take_type();
                self.work
                    .push(Work::TypeNameDone(name, self.types.runtime_bounds.len()));
                self.reject_standard_alignment(name.declaration_specifiers.extensions);
                self.work.push(Work::DeclaredAlignment(
                    name.declaration_specifiers.extensions,
                    base,
                    true,
                ));
                if let Some(d) = name.declarator {
                    self.work.push(Work::Declarator(d, base, false));
                } else {
                    self.values.push(base);
                }
            },
            | Work::TypeNameDone(name, bound_start) =>
                if let Some(&ty) = self.values.last() {
                    let bounds = self.site_bounds(
                        ty,
                        bound_start,
                        name.declaration_specifiers.type_specifiers,
                    );
                    self.record_bound_origins(
                        bounds,
                        BoundEvaluationSite::TypeName(self.type_names.len()),
                    );
                    self.type_names.push((name, ty));
                    _ = self
                        .resolved_type_names
                        .insert(type_name_identity(name), ty);
                    if let Some(operand_evaluations) =
                        self.spec_evaluations(name.declaration_specifiers.type_specifiers)
                    {
                        _ = self.type_name_sites.insert(
                            type_name_identity(name),
                            BoundSiteFacts {
                                bounds: self.site_bounds(
                                    ty,
                                    bound_start,
                                    name.declaration_specifiers.type_specifiers,
                                ),
                                ty,
                                operand_evaluations,
                            },
                        );
                    }
                },
            | Work::Declarator(d, base, parameter) => {
                self.work.push(Work::DeclaratorDone(d, base, parameter));
                for level in d.pointer.levels {
                    self.attribute_chain_operands(level.attributes);
                }
                for direct in d.kind {
                    if let DirectDeclarator::Attributes(attribute) = direct {
                        self.attribute_operands(attribute);
                    }
                }
            },
            | Work::DeclaratorDone(d, mut base, parameter) => {
                if d.pointer
                    .levels
                    .iter()
                    .any(|level| level.attributes.is_some())
                    || d.kind.iter().any(|direct| {
                        matches!(
                            direct,
                            DirectDeclarator::Attributes(_)
                                | DirectDeclarator::MsModifier(..)
                                | DirectDeclarator::AsmLabel(_)
                        )
                    })
                {
                    self.declarators.push(d);
                }
                for level in d.pointer.levels {
                    if !(level.qualifiers
                        - (TypeQualifiers::CONST
                            | TypeQualifiers::VOLATILE
                            | TypeQualifiers::RESTRICT
                            | TypeQualifiers::ATOMIC))
                        .is_empty()
                    {
                        base = self.types.unknown();
                        continue;
                    }
                    if self.unmodeled_extension(level.attributes) {
                        base = self.types.unknown();
                        continue;
                    }
                    base = self.types.intern(TypeKind::Pointer(base));
                    if level.qualifiers.contains(TypeQualifiers::ATOMIC) {
                        base = self.atomic_type(base, d.source_vectors, false);
                    }
                    base = base.qualified(level.qualifiers - TypeQualifiers::ATOMIC);
                    let mut chain = level.attributes;
                    while let Some(item) = chain {
                        if let SpecifierExtensionKind::Attributes(attribute) = item.kind {
                            base = self.mode_attribute(base, attribute);
                            base = self.vector_attribute(base, attribute, true);
                        }
                        chain = item.next;
                    }
                    self.validate_qualifiers(base, d.source_vectors);
                }
                self.values.push(base);
                // GNU attributes extend C99 §6.7: an alignment suffix applies
                // to the declared array, not to its element type.
                let vector = d.kind.iter().any(|direct| {
                    matches!(direct, DirectDeclarator::Attributes(a)
                        if vectors::declared_type_attribute(a, self.context))
                });
                if vector {
                    self.work.push(Work::VectorDeclaratorAttributes(d.kind));
                }
                for direct in d.kind {
                    if vector && matches!(direct, DirectDeclarator::Attributes(_)) {
                        continue;
                    }
                    self.work
                        .push(Work::Direct(direct, d.source_vectors, parameter));
                }
            },
            | Work::Direct(d, source, parameter) => self.direct(d, source, parameter),
            | Work::ArrayDone(element, q, expression, source, errors_before) => {
                let value = self.integers.pop().flatten();
                if self.non_integer_bound(expression) {
                    self.error(SemanticErrorKind::InvalidArrayBound, source, None, None);
                    self.values.push(self.types.unknown());
                    return;
                }
                if value.is_none()
                    && (self.semantic_errors > errors_before
                        || self.unanalyzed_constant(expression))
                {
                    self.values.push(self.types.unknown());
                    return;
                }
                let bound = if let Some(value) = value {
                    if value.signed && value.value < 0 {
                        self.error(SemanticErrorKind::InvalidArrayBound, source, None, None);
                        self.values.push(self.types.unknown());
                        return;
                    }
                    // A bound beyond i128 is far beyond any object size.
                    if !self.validate_array_size(
                        element,
                        value.to_i128().unwrap_or(i128::MAX),
                        source,
                    ) {
                        self.values.push(self.types.unknown());
                        return;
                    }
                    if value.value == 0 {
                        let mut literal = expression;
                        while let ExpressionType::Parenthesized { expression, .. } = literal.kind {
                            literal = expression;
                        }
                        if !matches!(literal.kind, ExpressionType::Constant(Constant::Integer(_)))
                            && !self.tainted
                        {
                            self.context.report_extension(
                                crate::configuration::Feature::ZeroLengthArrays,
                                "zero-length array",
                                expression.source_vectors,
                            );
                        }
                    }
                    value.to_u64().map_or_else(
                        || self.types.variable_bound(expression),
                        ArrayBound::Constant,
                    )
                } else {
                    self.types.variable_bound(expression)
                };
                let ty = self
                    .types
                    .intern(TypeKind::Array(element, bound))
                    .qualified(q);
                self.values.push(ty);
            },
            | Work::FunctionParameters(list, index, params, result, variadic, source, identity) =>
                if let Some(&p) = list.as_slice().get(index) {
                    self.work.push(Work::FunctionParameters(
                        list,
                        index + 1,
                        params,
                        result,
                        variadic,
                        source,
                        identity,
                    ));
                    self.work.push(Work::ParameterBase(
                        p,
                        params,
                        std::ptr::from_ref(&list.as_slice()[index]).addr(),
                    ));
                    self.spec(p.declaration_specifiers, false);
                } else {
                    let scope = self.scope;
                    let params = params.finish(self.types.tu, self.scratch);
                    self.leave();
                    let mut parameters = ArenaVec::new_in(self.scratch);
                    let void = matches!(params,[Parameter {name:None,ty,..}] if matches!(self.types.nodes[ty.index],TypeKind::Scalar(Scalar::Void)) && ty.qualifiers.is_empty());
                    for p in params {
                        if !void {
                            if matches!(
                                self.types.nodes[p.ty.index],
                                TypeKind::Scalar(Scalar::Void)
                            ) {
                                self.error(
                                    SemanticErrorKind::InvalidParameter,
                                    source,
                                    p.name.map(|n| n.name),
                                    None,
                                );
                            }
                            parameters.push(p.ty.unqualified());
                        }
                    }
                    let prototype =
                        !list.is_empty() || self.context.configuration.standard() >= CStandard::C23;
                    let parameters = self.types.tu.alloc_slice_copy(&parameters);
                    _ = self.parameters.insert(identity, (params, scope));
                    self.values.push(self.types.intern(TypeKind::Function {
                        result,
                        parameters,
                        prototype,
                        variadic,
                    }));
                },
            | Work::ParameterBase(p, params, identity) => {
                let base = self.take_type();
                self.work.push(Work::ParameterDone(
                    p,
                    params,
                    self.types.runtime_bounds.len(),
                    identity,
                ));
                self.reject_standard_alignment(p.declaration_specifiers.extensions);
                self.work.push(Work::DeclaredAlignment(
                    p.declaration_specifiers.extensions,
                    base,
                    false,
                ));
                if let Some(d) = p.declarator {
                    self.validate_parameter_arrays(d);
                    self.work.push(Work::Declarator(d, base, true));
                } else {
                    self.values.push(base);
                }
            },
            | Work::ParameterDone(p, params, bound_start, _identity) => {
                let mut ty = self.take_type();
                let original = ty;
                let site = self
                    .spec_evaluations(p.declaration_specifiers.type_specifiers)
                    .map(|operand_evaluations| BoundSiteFacts {
                        ty: original,
                        operand_evaluations,
                        bounds: self.site_bounds(
                            original,
                            bound_start,
                            p.declaration_specifiers.type_specifiers,
                        ),
                    });
                let array_minimum = if let TypeKind::Array(_, bound) = self.types.nodes[ty.index] {
                    p.declarator
                        .filter(|d| {
                            d.kind.iter().any(|d| {
                                matches!(
                                    d,
                                    DirectDeclarator::Array {
                                        is_static: true,
                                        ..
                                    }
                                )
                            })
                        })
                        .map(|_| bound)
                } else {
                    None
                };
                ty = match self.types.nodes[ty.index] {
                    | TypeKind::Array(element, _) => self
                        .types
                        .intern(TypeKind::Pointer(element))
                        .qualified(ty.qualifiers),
                    | TypeKind::Function { .. } => self.types.intern(TypeKind::Pointer(ty)),
                    | _ => ty,
                };
                let name = p.declarator.and_then(Declarator::identifier);
                if p.declaration_specifiers
                    .storage_class
                    .is_some_and(|s| s != StorageClass::Register)
                    || self.thread_storage(p.declaration_specifiers.extensions)
                {
                    self.error(
                        SemanticErrorKind::InvalidParameter,
                        p.source_vectors,
                        name.map(|n| n.name),
                        None,
                    );
                }
                // C99 §6.7.4p2: parameters declare objects, including
                // declarators adjusted from function types.
                if p.declaration_specifiers.function_specifiers.is_inline {
                    self.error(
                        SemanticErrorKind::InlineParameter,
                        p.source_vectors,
                        name.map(|n| n.name),
                        None,
                    );
                }
                let binding = name.map(|name| {
                    self.bind(
                        name,
                        ty,
                        BindingKind::Parameter,
                        Linkage::None,
                        Duration::Automatic,
                        None,
                    );
                    let binding = self.bindings.len() - 1;
                    if let Some(minimum) = array_minimum {
                        _ = self
                            .correspondence
                            .parameter_minimums
                            .insert(binding, minimum);
                    }
                    binding
                });
                if let Some(site) = site {
                    self.record_bound_origins(
                        site.bounds,
                        BoundEvaluationSite::Parameter(self.next_parameter),
                    );
                }
                self.next_parameter += 1;
                params.push(
                    self.scratch,
                    Parameter {
                        site,
                        binding,
                        name,
                        ty,
                        array_minimum,
                        original,
                    },
                );
            },
            | Work::RecordMembers(tag, list, index, members) => {
                if let Some(&member) = list.as_slice().get(index) {
                    self.work
                        .push(Work::RecordMembers(tag, list, index + 1, members));
                    if let Some(assertion) = member.assertion {
                        self.taint(assertion.recovered);
                        self.work.push(Work::RequireConstant(
                            assertion.expression,
                            Some(assertion),
                            self.semantic_errors,
                        ));
                        self.work.push(Work::Eval(assertion.expression));
                        self.work.push(Work::Expression(assertion.expression));
                        return;
                    }
                    self.work.push(Work::RecordMemberBase(tag, member, members));
                    self.work
                        .push(Work::VectorAttributes(member.extensions, false));
                    self.work.push(Work::Spec(
                        member.type_specifiers,
                        member.type_qualifiers,
                        member.source_vectors,
                        false,
                    ));
                    let mut extension = member.extensions;
                    while let Some(item) = extension {
                        if let SpecifierExtensionKind::Alignment(operand) = item.kind {
                            self.syntax_operand(operand);
                        }
                        extension = item.next;
                    }
                } else {
                    self.complete_record(tag, members.finish(self.types.tu, self.scratch));
                }
            },
            | Work::RecordMemberBase(tag, member, members) => {
                let base = self.take_type();
                let attributes = self.layout_attribute_chain(member.extensions);
                if member.struct_declarator_list.is_empty() {
                    let anonymous_base = self.align_declared_type(base, attributes.alignment);
                    let anonymous_base = self.standard_alignment(anonymous_base, member.extensions);
                    if let Some(anonymous) = self.anonymous_member(tag, member, anonymous_base) {
                        members.push(self.scratch, anonymous);
                    }
                }
                for &d in member.struct_declarator_list.iter().rev() {
                    self.work.push(Work::MemberBase(
                        tag,
                        d,
                        base,
                        member.extensions,
                        attributes.packed,
                        members,
                    ));
                }
                if self.unmodeled_extension(member.extensions)
                    || member
                        .struct_declarator_list
                        .iter()
                        .any(|d| self.unmodeled_extension(d.attributes))
                {
                    self.types.tags[tag].tainted.set(true);
                }
            },
            | Work::MemberBase(tag, d, base, extensions, spec_packed, members) => {
                self.work.push(Work::MemberBaseDone(
                    tag,
                    d,
                    base,
                    extensions,
                    spec_packed,
                    members,
                ));
                self.attribute_chain_operands(d.attributes);
            },
            | Work::MemberBaseDone(tag, d, base, extensions, spec_packed, members) => {
                let packed = spec_packed
                    || self.layout_attribute_chain(d.attributes).packed
                    || d.declarator
                        .is_some_and(|decl| self.packed_declarator(decl));
                self.work.push(Work::MemberDone(
                    tag,
                    d,
                    members,
                    self.semantic_errors,
                    packed,
                    extensions,
                ));
                if d.bitfield_width.is_some() {
                    self.reject_standard_alignment(extensions);
                }
                self.work.push(Work::DeclaredAlignment(
                    extensions,
                    base,
                    packed || self.types.tags[tag].packed.get(),
                ));
                self.work.push(Work::VectorAttributes(d.attributes, true));
                if let Some(decl) = d.declarator {
                    self.work.push(Work::Declarator(decl, base, false));
                } else {
                    self.values.push(base);
                }
            },
            | Work::MemberDone(tag, d, members, errors_before, packed, extensions) => {
                let ty = self.take_type();
                let explicit_alignment = if self.semantic_errors == errors_before {
                    self.explicit_member_alignment(extensions, d.attributes, d.declarator)
                } else {
                    0
                };
                self.work.push(Work::MemberWidth(
                    tag,
                    d,
                    ty,
                    members,
                    errors_before,
                    packed,
                    explicit_alignment,
                ));
                if let Some(width) = d.bitfield_width {
                    self.work.push(Work::Eval(width.expression()));
                    self.work.push(Work::Expression(width.expression()));
                } else {
                    self.integers.push(None);
                }
            },
            | Work::MemberWidth(tag, d, ty, members, errors_before, packed, explicit_alignment) => {
                let value = self.integers.pop().flatten();
                // C99 §6.7.2.1p2-4: retain rejected members for recovery,
                // without repeating their constraints during completion.
                let mut invalid = self.semantic_errors != errors_before;
                let width = if d.bitfield_width.is_some() {
                    let valid = value.and_then(|v| v.to_u64().and_then(|n| u32::try_from(n).ok()));
                    let bits = self
                        .integer_type(ty)
                        .filter(|_| !matches!(self.types.nodes[ty.index], TypeKind::Atomic(_)))
                        .map(|(bits, _)| bits);
                    let name = d.declarator.and_then(Declarator::identifier);
                    if valid.is_none()
                        || bits.is_none()
                        || valid > bits
                        || (valid == Some(0) && name.is_some())
                    {
                        invalid = true;
                        if !self.types.unanalyzed(ty)
                            && self.semantic_errors == errors_before
                            && !d
                                .bitfield_width
                                .is_some_and(|w| self.unanalyzed_constant(w.expression()))
                        {
                            self.error(
                                if bits.is_none() {
                                    SemanticErrorKind::InvalidBitFieldType
                                } else if value.is_none() {
                                    SemanticErrorKind::InvalidConstant
                                } else if valid.is_none() || valid > bits {
                                    SemanticErrorKind::InvalidBitFieldWidth
                                } else {
                                    SemanticErrorKind::NamedZeroWidthBitField
                                },
                                d.source_vectors,
                                name.map(|n| n.name),
                                None,
                            );
                        }
                        self.types.tags[tag].tainted.set(true);
                    }
                    valid
                } else {
                    None
                };
                let name = d.declarator.and_then(Declarator::identifier);
                if self.variably_modified(ty) {
                    invalid = true;
                    self.error(
                        SemanticErrorKind::FileScopeVariableType,
                        name.map_or(d.source_vectors, |n| n.source_vectors),
                        name.map(|n| n.name),
                        None,
                    );
                    self.types.tags[tag].tainted.set(true);
                }
                if let Some(name) = name
                    && let Some(previous) = self
                        .member_names
                        .insert((tag, name.name), name.source_vectors)
                {
                    self.error(
                        SemanticErrorKind::DuplicateMember,
                        name.source_vectors,
                        Some(name.name),
                        Some(previous),
                    );
                }
                members.push(
                    self.scratch,
                    Member {
                        name,
                        source_vectors: d.source_vectors,
                        invalid,
                        ty,
                        width,
                        offset: 0,
                        bit_offset: 0,
                        anonymous: false,
                        packed,
                        explicit_alignment,
                    },
                );
            },
            | Work::EnumMembers(tag, list, index, previous) => {
                if let Some(item) = list.as_slice().get(index) {
                    self.work.push(Work::EnumeratorDone(
                        tag,
                        list,
                        index,
                        self.semantic_errors,
                        index != 0 && previous.is_none() && item.expression.is_none(),
                    ));
                    if let Some(expression) = item.expression {
                        self.work.push(Work::Eval(expression.expression()));
                        self.work.push(Work::Expression(expression.expression()));
                    } else {
                        self.integers.push(
                            previous
                                .and_then(|value| Integer { bits: 64, ..value }.increment())
                                .or_else(|| (index == 0).then_some(Integer::int(0))),
                        );
                    }
                } else {
                    _ = self.defining.remove(&tag);
                    let range = self.enum_ranges.remove(&tag).unwrap_or((0, 0));
                    let tag = self.types.tags[tag];
                    tag.complete.set(true);
                    if !tag.tainted.get() {
                        let compatible =
                            compatible_enum_type(range, &self.types.target, tag.packed.get());
                        tag.compatible.set(compatible);
                        tag.layout
                            .set(self.types.target.scalar(compatible).map(|layout| Layout {
                                align: layout.align.max(u64::from(tag.alignment.get())),
                                ..layout
                            }));
                    }
                }
            },
            | Work::EnumeratorDone(tag, list, index, errors_before, invalid_implicit) => {
                let mut value = self.integers.pop().flatten();
                let item = list.as_slice()[index];
                // The enum ABI still selects a standard integer type, at most
                // 64 bits. Do not truncate a full-width unsigned value here.
                // C99: implementation-defined §6.7.2.2p4, p. 105; PDF p. 117.
                if self.types.target.fixed_enum_type.is_none()
                    && value.is_some_and(|v| {
                        v.to_i128()
                            .is_none_or(|n| n < i128::from(i64::MIN) || n > i128::from(u64::MAX))
                    })
                {
                    self.error(
                        SemanticErrorKind::EnumeratorRange,
                        item.source_vectors,
                        Some(item.name.name),
                        None,
                    );
                    self.types.tags[tag].tainted.set(true);
                    value = None;
                }
                if let Some(v) = value
                    && v.bits == 128
                    && v.to_i128().is_some_and(|n| i32::try_from(n).is_err())
                {
                    value = Some(
                        v.cast(
                            64,
                            v.to_i128()
                                .is_some_and(|n| n < 0 || (v.signed && n <= i128::from(i64::MAX))),
                        ),
                    );
                }
                if value.is_none()
                    && self.semantic_errors == errors_before
                    && !invalid_implicit
                    && !item
                        .expression
                        .is_some_and(|e| self.unanalyzed_constant(e.expression()))
                {
                    // An implicit value past the previous one that no
                    // 64-bit type holds has no integer type at all.
                    self.error(
                        if item.expression.is_none() {
                            SemanticErrorKind::EnumeratorRange
                        } else {
                            SemanticErrorKind::InvalidConstant
                        },
                        item.source_vectors,
                        Some(item.name.name),
                        None,
                    );
                }
                // C99 §6.7.2.2p2 requires int values; GCC and Clang accept
                // wider ones, which C23 adopts.
                let wide =
                    value.is_some_and(|v| v.to_i128().is_none_or(|n| i32::try_from(n).is_err()));
                if wide && !self.tainted && !self.types.tags[tag].tainted.get() {
                    self.context.report_extension(
                        crate::configuration::Feature::WideEnumerators,
                        "enumerator value outside the range of int",
                        item.source_vectors,
                    );
                }
                if let Some(v) = value
                    && !self.types.tags[tag].tainted.get()
                {
                    let (low, high) = self
                        .enum_ranges
                        .get(&tag)
                        .copied()
                        .unwrap_or((v.value, v.value));
                    let (low, high) = (low.min(v.value), high.max(v.value));
                    if low < 0
                        && high > i128::from(i64::MAX)
                        && self.types.target.fixed_enum_type.is_none()
                    {
                        self.error(
                            SemanticErrorKind::EnumeratorRange,
                            item.name.source_vectors,
                            Some(item.name.name),
                            None,
                        );
                        self.types.tags[tag].tainted.set(true);
                    } else {
                        _ = self.enum_ranges.insert(tag, (low, high));
                    }
                }
                if value.is_none() {
                    self.types.tags[tag].tainted.set(true);
                }
                let opaque = self.types.tags[tag].tainted.get();
                let (ty, retained) = match value {
                    | _ if opaque => (self.types.unknown(), None),
                    | Some(v) if self.types.target.fixed_enum_type.is_some() =>
                        (self.types.scalar(Scalar::Int), Some(v.cast(32, true))),
                    | Some(v) if wide => (
                        self.types.scalar(match (v.bits, v.signed) {
                            | (64, true) => self.types.target.intmax_t,
                            | (64, false) => self.types.target.uintmax_t,
                            | _ => Scalar::UnsignedInt,
                        }),
                        Some(v),
                    ),
                    | _ => (
                        self.types.scalar(Scalar::Int),
                        value.map(|v| Integer::int(v.value)),
                    ),
                };
                self.bind(
                    item.name,
                    ty,
                    BindingKind::Enumerator,
                    Linkage::None,
                    Duration::None,
                    retained,
                );
                self.work.push(Work::EnumMembers(
                    tag,
                    list,
                    index + 1,
                    if self.types.target.fixed_enum_type.is_some() {
                        retained
                    } else {
                        value
                    },
                ));
            },
            | Work::Eval(expression) => self.evaluate(expression),
            | Work::Unary(op, source) => {
                let value = self.integers.pop().flatten();
                let result = value.and_then(|v| v.unary(op));
                if value.is_some() && result.is_none() {
                    self.exceptional_constant(source);
                }
                self.integers.push(result);
            },
            | Work::Binary(op, source) => {
                let right = self.integers.pop().flatten();
                let left = self.integers.pop().flatten();
                // Expression typing has already reported a sign-bit shift.
                let result = left
                    .zip(right)
                    .and_then(|(l, r)| l.binary(op, r).or_else(|| l.sign_bit_shift(op, r)));
                if left.is_some() && right.is_some() && result.is_none() {
                    self.exceptional_constant(source);
                }
                self.integers.push(result);
            },
            | Work::Logical(op, right, source) => {
                let left = self.integers.pop().flatten();
                if let Some(l) = left {
                    if (op == BinaryOperator::LogicalAnd && l.value == 0)
                        || (op == BinaryOperator::LogicalOr && l.value != 0)
                    {
                        self.integers
                            .push(Some(Integer::int(i128::from(l.value != 0))));
                    } else {
                        self.integers.push(left);
                        self.work.push(Work::Binary(op, source));
                        self.work.push(Work::Eval(right));
                    }
                } else {
                    self.integers.push(None);
                }
            },
            | Work::Conditional(c) =>
                if let Some(value) = self.integers.pop().flatten() {
                    if let Some((bits, signed)) = self.conditional_model(c) {
                        self.work.push(Work::ConvertInteger(bits, signed));
                    }
                    self.work.push(Work::Eval(if value.value != 0 {
                        c.then_expression
                    } else {
                        c.else_expression
                    }));
                } else {
                    self.integers.push(None);
                },
            | Work::ConvertInteger(bits, signed) => {
                let value = self.integers.pop().flatten();
                self.integers.push(value.map(|v| v.cast(bits, signed)));
            },
            | Work::CastBase(operand, source) => {
                let ty = self.take_type();
                let mut operand = operand;
                while let ExpressionType::Parenthesized { expression } = operand.kind {
                    operand = expression;
                }
                if let ExpressionType::Constant(Constant::Float(value)) = operand.kind {
                    let result = self.float_cast(ty, value);
                    if result.is_none()
                        && self.integer_type(ty).is_some()
                        && matches!(
                            value,
                            super::preprocessing::FloatTokenType::Float(_)
                                | super::preprocessing::FloatTokenType::Double(_)
                                | super::preprocessing::FloatTokenType::LongDouble(_)
                        )
                    {
                        self.exceptional_constant(source);
                    }
                    self.integers.push(result);
                    return;
                }
                self.work.push(Work::CastDone(ty, source));
                self.work.push(Work::Eval(operand));
            },
            | Work::CastDone(ty, _source) => {
                let value = self.integers.pop().flatten();
                let result = value.zip(self.integer_type(ty)).map(|(v, (bits, signed))| {
                    if matches!(self.types.nodes[ty.index], TypeKind::Scalar(Scalar::Bool)) {
                        Integer {
                            value:  i128::from(v.value != 0),
                            bits:   1,
                            signed: false,
                        }
                    } else {
                        v.cast(bits, signed)
                    }
                });
                self.integers.push(result);
            },
            | Work::SizeofDone(align, source) => {
                let ty = self.take_type();
                let gnu_unit_size = self.context.configuration.gnu_extensions()
                    && matches!(
                        self.types.nodes[ty.index],
                        TypeKind::Scalar(Scalar::Void) | TypeKind::Function { .. }
                    );
                // C11 §6.5.3.4p3: array alignment is its element alignment,
                // even when sizeof is runtime-valued (extension in C99).
                let value = if align {
                    self.types.alignment(ty)
                } else {
                    self.types.layout(ty).map(|layout| layout.size)
                }
                .or_else(|| gnu_unit_size.then_some(1));
                if value.is_none()
                    && !self.types.unanalyzed(ty)
                    // C99 §6.5.3.4p2: sizeof a VLA is not an integer
                    // constant, including when an inner dimension varies.
                    && !self.types.variably_modified(ty)
                {
                    self.error(SemanticErrorKind::InvalidConstant, source, None, None);
                }
                self.integers.push(value.map(|value| Integer {
                    value:  i128::from(value),
                    bits:   64,
                    signed: false,
                }));
            },
        }
    }
}

// Continuations compose type construction, ICE evaluation and structural
// walking. No task invokes the driver recursively, including nested
// record/function types.
#[derive(Clone, Copy)]
enum Work<'tu, 's> {
    DeclaredAlignment(Option<&'tu SpecifierExtension<'tu>>, TypeId, bool),
    Root(ExternalDeclaration<'tu>),
    Declaration(&'tu Declaration<'tu>),
    DeclarationBase(&'tu Declaration<'tu>),
    DeclarationDeclarator(Declarator<'tu>, DeclarationSpecifiers<'tu>, TypeId, bool),
    Bind(Declarator<'tu>, DeclarationSpecifiers<'tu>, bool, usize),
    Function(&'tu FunctionDefinition<'tu>),
    FunctionBase(&'tu FunctionDefinition<'tu>),
    FunctionBody(&'tu FunctionDefinition<'tu>),
    FunctionWork(functions::FunctionWork<'tu>),
    StatementWork(statements::StatementWork<'tu>),
    Statement(&'tu Statement<'tu>, bool),
    BlockItem(BlockItem<'tu>),
    Expression(&'tu Expression<'tu>),
    ExpressionDone(&'tu Expression<'tu>),
    ValueExpression(&'tu Expression<'tu>),
    CompoundInitializer(&'tu Expression<'tu>, &'tu Initializer<'tu>),
    InitializeDeclaration(Declarator<'tu>, &'tu Initializer<'tu>),
    InitializeDone(usize, &'tu Initializer<'tu>),
    Condition(ExpressionSlot<'tu>, bool),
    RequireConstant(
        &'tu Expression<'tu>,
        Option<&'tu super::parsing::StaticAssertion<'tu>>,
        usize,
    ),
    Slot(ExpressionSlot<'tu>),
    Initializer(&'tu Initializer<'tu>),
    PopScope,
    RestoreTaint(bool),
    RestoreParameterMode(bool),
    /// Sets whether exceptional evaluation makes an array bound variable
    /// rather than reporting an overflow.
    RuntimeBound(bool),
    OldSignature(&'tu FunctionDefinition<'tu>),
    DiscardType,
    VectorAttributes(Option<&'tu SpecifierExtension<'tu>>, bool),
    VectorAttributesDone(Option<&'tu SpecifierExtension<'tu>>, bool),
    VectorDeclaratorAttributes(ArenaList<'tu, DirectDeclarator<'tu>>),
    UnknownType,
    Spec(TypeSpecifiers<'tu>, TypeQualifiers, SourceVectors, bool),
    SpecDone(TypeSpecifiers<'tu>, TypeQualifiers, SourceVectors, bool),
    Qualify(TypeQualifiers, SourceVectors),
    TypeName(&'tu TypeName<'tu>),
    TypeNameBase(&'tu TypeName<'tu>),
    TypeNameDone(&'tu TypeName<'tu>, usize),
    Declarator(Declarator<'tu>, TypeId, bool),
    DeclaratorDone(Declarator<'tu>, TypeId, bool),
    Direct(&'tu DirectDeclarator<'tu>, SourceVectors, bool),
    ArrayDone(
        TypeId,
        TypeQualifiers,
        &'tu Expression<'tu>,
        SourceVectors,
        usize,
    ),
    FunctionParameters(
        ArenaList<'tu, ParameterDeclaration<'tu>>,
        usize,
        &'s Collection<'s, Parameter<'tu>>,
        TypeId,
        bool,
        SourceVectors,
        usize,
    ),
    ParameterBase(
        ParameterDeclaration<'tu>,
        &'s Collection<'s, Parameter<'tu>>,
        usize,
    ),
    ParameterDone(
        ParameterDeclaration<'tu>,
        &'s Collection<'s, Parameter<'tu>>,
        usize,
        usize,
    ),
    RecordMembers(
        usize,
        ArenaList<'tu, StructDeclaration<'tu>>,
        usize,
        &'s Collection<'s, Member>,
    ),
    RecordMemberBase(usize, StructDeclaration<'tu>, &'s Collection<'s, Member>),
    MemberBase(
        usize,
        StructDeclarator<'tu>,
        TypeId,
        Option<&'tu SpecifierExtension<'tu>>,
        bool,
        &'s Collection<'s, Member>,
    ),
    MemberBaseDone(
        usize,
        StructDeclarator<'tu>,
        TypeId,
        Option<&'tu SpecifierExtension<'tu>>,
        bool,
        &'s Collection<'s, Member>,
    ),
    MemberDone(
        usize,
        StructDeclarator<'tu>,
        &'s Collection<'s, Member>,
        usize,
        bool,
        Option<&'tu SpecifierExtension<'tu>>,
    ),
    MemberWidth(
        usize,
        StructDeclarator<'tu>,
        TypeId,
        &'s Collection<'s, Member>,
        usize,
        bool,
        u32,
    ),
    EnumMembers(
        usize,
        ArenaList<'tu, Enumerator<'tu>>,
        usize,
        Option<Integer>,
    ),
    EnumeratorDone(usize, ArenaList<'tu, Enumerator<'tu>>, usize, usize, bool),
    Eval(&'tu Expression<'tu>),
    Unary(UnaryOperator, SourceVectors),
    Binary(BinaryOperator, SourceVectors),
    Logical(BinaryOperator, &'tu Expression<'tu>, SourceVectors),
    Conditional(&'tu ConditionalExpression<'tu>),
    CastBase(&'tu Expression<'tu>, SourceVectors),
    CastDone(TypeId, SourceVectors),
    ConvertInteger(u32, bool),
    SizeofDone(bool, SourceVectors),
    Atomic(SourceVectors),
    TypeofDone(&'tu ExtendedType<'tu>),
}

struct Analyzer<'a, 'tu, 's> {
    // Deterministic scaling-test counts: VM ancestors, offset members,
    // initializer type nodes. They are absent from production compilation.
    #[cfg(test)]
    review_steps:            Cell<[usize; 3]>,
    context:                 &'a mut Context<'tu>,
    scratch:                 &'s Bump,
    types:                   TypeInterner<'tu, 's>,
    pack_alignments:         &'s [(usize, u32)],
    bindings:                ArenaVec<'tu, Binding>,
    declarators:             ArenaVec<'tu, Declarator<'tu>>,
    spec_extensions:         ArenaVec<'tu, &'tu SpecifierExtension<'tu>>,
    definitions:             ArenaVec<'tu, Definition>,
    /// Indexed by the function definition's traversal id.
    function_records:        ArenaVec<'tu, FunctionRecord<'tu>>,
    scopes:                  ArenaVec<'tu, Scope>,
    type_names:              ArenaVec<'s, (&'tu TypeName<'tu>, TypeId)>,
    retained_type_names:     ArenaMap<'tu, usize, TypeNameId>,
    declaration_sites:       ArenaMap<'tu, usize, BoundSiteFacts<'tu>>,
    type_name_sites:         ArenaMap<'s, usize, BoundSiteFacts<'tu>>,
    size_query_sites:        ArenaMap<'tu, usize, SizeQueryFacts<'tu>>,
    typeof_sites:            ArenaMap<'s, usize, BoundSiteFacts<'tu>>,
    scope:                   usize,
    entries:                 ArenaVec<'s, Entry>,
    visible:                 ArenaMap<'s, (Namespace, StringCacheId), usize>,
    external:                ArenaMap<'s, StringCacheId, usize>,
    scope_entries:           ArenaVec<'s, &'s Collection<'s, usize>>,
    parameters:              ArenaMap<'s, usize, (&'tu [Parameter<'tu>], usize)>,
    work:                    ArenaVec<'s, Work<'tu, 's>>,
    values:                  ArenaVec<'s, TypeId>,
    integers:                ArenaVec<'s, Option<Integer>>,
    tainted:                 bool,
    function_name:           Option<Identifier>,
    old_parameter_mode:      bool,
    /// Exceptional evaluation yields a variable array bound, not an error.
    runtime_bound:           bool,
    /// Reported constraint diagnostics, warnings included: local checks
    /// compare it with an earlier value to learn whether a check failed.
    semantic_errors:         usize,
    /// Reported diagnostics of error severity only, retained for lowering.
    error_diagnostics:       usize,
    member_indices:          ArenaMap<'s, (usize, StringCacheId), usize>,
    member_names:            ArenaMap<'s, (usize, StringCacheId), SourceVectors>,
    tag_declarations:        ArenaVec<'tu, (usize, usize)>,
    resolved_type_names:     ArenaMap<'s, usize, TypeId>,
    correspondence:          correspondence::CorrespondenceBuilder<'tu, 's>,
    runtime_bounds:          ArenaVec<'tu, RuntimeBound<'tu>>,
    bound_origins:           ArenaMap<'s, usize, BoundEvaluationSite>,
    bound_expression_starts: ArenaMap<'s, usize, usize>,
    integer_models:          ArenaMap<'s, usize, Option<(u32, bool)>>,
    resolved:                resolved::ResolvedBuilder<'tu, 's>,
    expressions:             ArenaVec<'tu, ExpressionInfo<'tu>>,
    effects:                 effects::EffectBuilder<'tu, 's>,
    expression_actions:      ArenaMap<'tu, usize, BoundSiteFacts<'tu>>,
    binding_bounds:          ArenaMap<'tu, usize, &'tu [usize]>,
    next_parameter:          usize,
    function_controls:       ArenaMap<'s, usize, (FunctionControl, Option<RegionId>)>,
    expression_indices:      ArenaMap<'s, usize, usize>,
    string_initializers:     ArenaMap<'tu, usize, TypeId>,
    /// Conversion records in the order they were applied, grouped by
    /// expression when analysis ends.
    conversions:             ArenaVec<'s, Conversion<'tu>>,
    const_members:           ArenaMap<'s, usize, bool>,
    register_bindings:       ArenaMap<'s, usize, bool>,
    ice_operands:            ArenaMap<'s, (usize, bool), bool>,
    functions:               functions::State<'tu, 's>,
    va_list_type:            Option<TypeId>,
    statements:              statements::State<'tu, 's>,
    control:                 builder::Builder<'tu, 's>,
    /// Tags whose member or enumerator list is open.
    defining:                ArenaMap<'s, usize, ()>,
    /// Least and greatest enumerator values of an open enumeration.
    enum_ranges:             ArenaMap<'s, usize, (i128, i128)>,
}

// Tests
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    reason = "Tests own generated inputs and expected values outside compilation."
)]
mod tests;
