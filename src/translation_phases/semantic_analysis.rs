//! Declaration semantic analysis, the second half of translation phase 7.
//! C99: §5.1.1.2p1, p. 10; PDF p. 22; scopes/linkage §6.2.1-§6.2.4,
//! pp. 29-32; PDF pp. 41-44; declarations §6.7, pp. 97-124;
//! PDF pp. 109-136. Expression and statement constraints remain later work.

mod declarations;
mod errors;
mod inspection;
mod integer;
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    reason = "Tests own generated inputs and expected values outside compilation."
)]
mod tests;
mod traversal;
mod types;

use std::cell::Cell;

pub(crate) use errors::{
    SemanticError,
    SemanticErrorKind,
};
use integer::Integer;
use rustc_hash::FxBuildHasher;
use types::{
    ArrayBound,
    Layout,
    Member,
    Parameter,
    Scalar,
    Tag,
    TagKind,
    TypeId,
    TypeInterner,
    TypeKind,
    Types,
    align_up,
};

use super::{
    Context,
    SourceVectors,
    TranslationError,
    parsing::{
        ExtendedType,
        ParsedTranslationUnit,
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
            StructOrUnion,
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
            ForInitializer,
            FunctionDefinition,
            Identifier,
            Statement,
            StatementType,
            StorageClass,
            UnaryOperator,
        },
    },
};
use crate::{
    configuration::CStandard,
    util::{
        bump::{
            ArenaMap,
            ArenaVec,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

/// The three C linkage states, distinct from lexical scope.
/// C99: §6.2.2, pp. 30-31; PDF pp. 42-43.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Linkage {
    None,
    Internal,
    External,
}
/// Object lifetime, separate from identifier visibility.
/// C99: §6.2.4, p. 32; PDF p. 44.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Duration {
    None,
    Automatic,
    Static,
}
/// Semantic ordinary binding category.
/// C99: §6.2.3, p. 31; PDF p. 43; typedefs §6.7.7, pp. 123-124; PDF pp.
/// 135-136.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BindingKind {
    Object,
    Function,
    Typedef,
    Enumerator,
    Parameter,
}

/// A resolved declaration occurrence, retained in lexical traversal order.
/// C99: §6.7p3-4, p. 97; PDF p. 109.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Binding {
    pub(crate) name:     Identifier,
    pub(crate) ty:       TypeId,
    pub(crate) scope:    usize,
    pub(crate) kind:     BindingKind,
    pub(crate) linkage:  Linkage,
    pub(crate) duration: Duration,
    pub(crate) value:    Option<Integer>,
}

/// Semantic scope kinds; members belong to nominal records, labels to
/// functions. C99: §6.2.1, pp. 29-30; PDF pp. 41-42.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScopeKind {
    File,
    Function,
    Block,
    Prototype,
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct Scope {
    pub(crate) parent: Option<usize>,
    pub(crate) kind:   ScopeKind,
}

/// Durable semantic output. Working maps/stacks are gone when this is returned.
#[derive(Debug)]
pub(crate) struct SemanticTranslationUnit<'tu> {
    pub(crate) types:            Types<'tu>,
    pub(crate) bindings:         &'tu [Binding],
    pub(crate) scopes:           &'tu [Scope],
    pub(crate) type_names:       &'tu [(SourceVectors, TypeId)],
    pub(crate) parameters:       &'tu [(SourceVectors, &'tu [Parameter])],
    pub(crate) tag_declarations: &'tu [(usize, usize)],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Namespace {
    Ordinary,
    Tag,
}
#[derive(Clone, Copy)]
struct Entry {
    name:      Identifier,
    namespace: Namespace,
    scope:     usize,
    binding:   usize,
    previous:  Option<usize>,
}

/// A scratch cons list builds nested parameter/member lists in linear space.
struct Collection<'s, T: Copy> {
    head: Cell<Option<&'s Link<'s, T>>>,
}
struct Link<'s, T: Copy> {
    value: T,
    next:  Option<&'s Link<'s, T>>,
}
impl<'s, T: Copy> Collection<'s, T> {
    fn new() -> Self {
        Self {
            head: Cell::new(None),
        }
    }

    fn push(&self, scratch: &'s Bump, value: T) {
        self.head.set(Some(scratch.alloc(Link {
            value,
            next: self.head.get(),
        })));
    }

    fn finish<'tu>(&self, tu: &'tu Bump, scratch: &Bump) -> &'tu [T] {
        let mut items = ArenaVec::new_in(scratch);
        let mut next = self.head.get();
        while let Some(link) = next {
            items.push(link.value);
            next = link.next;
        }
        items.reverse();
        tu.alloc_slice_copy(&items)
    }
}

// Continuations compose type construction, ICE evaluation and structural
// walking. No task invokes the driver recursively, including nested
// record/function types.
#[derive(Clone, Copy)]
enum Work<'tu, 's> {
    Root(ExternalDeclaration<'tu>),
    Declaration(&'tu Declaration<'tu>),
    DeclarationBase(&'tu Declaration<'tu>),
    Bind(Declarator<'tu>, DeclarationSpecifiers<'tu>, bool),
    Function(&'tu FunctionDefinition<'tu>),
    FunctionBase(&'tu FunctionDefinition<'tu>),
    FunctionBody(&'tu FunctionDefinition<'tu>),
    Statement(&'tu Statement<'tu>, bool),
    BlockItem(BlockItem<'tu>),
    Expression(&'tu Expression<'tu>),
    Slot(ExpressionSlot<'tu>),
    Initializer(&'tu Initializer<'tu>),
    PopScope,
    RestoreTaint(bool),
    RestoreParameterMode(bool),
    OldSignature(&'tu FunctionDefinition<'tu>),
    DiscardType,
    UnknownType,
    Spec(TypeSpecifiers<'tu>, TypeQualifiers, SourceVectors, bool),
    Qualify(TypeQualifiers, SourceVectors),
    TypeName(&'tu TypeName<'tu>),
    TypeNameBase(&'tu TypeName<'tu>),
    TypeNameDone(SourceVectors),
    Declarator(Declarator<'tu>, TypeId, bool),
    Direct(DirectDeclarator<'tu>, SourceVectors, bool),
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
        &'s Collection<'s, Parameter>,
        TypeId,
        bool,
        SourceVectors,
    ),
    ParameterBase(ParameterDeclaration<'tu>),
    ParameterDone(ParameterDeclaration<'tu>, &'s Collection<'s, Parameter>),
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
        &'s Collection<'s, Member>,
    ),
    MemberDone(usize, StructDeclarator<'tu>, &'s Collection<'s, Member>),
    MemberWidth(
        usize,
        StructDeclarator<'tu>,
        TypeId,
        &'s Collection<'s, Member>,
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
}

use crate::util::arena_list::ArenaList;

struct Analyzer<'a, 'tu, 's> {
    context:             &'a mut Context<'tu>,
    scratch:             &'s Bump,
    types:               TypeInterner<'tu, 's>,
    bindings:            ArenaVec<'tu, Binding>,
    scopes:              ArenaVec<'tu, Scope>,
    type_names:          ArenaVec<'tu, (SourceVectors, TypeId)>,
    parameter_lists:     ArenaVec<'tu, (SourceVectors, &'tu [Parameter])>,
    scope:               usize,
    entries:             ArenaVec<'s, Entry>,
    visible:             ArenaMap<'s, (Namespace, StringCacheId), usize>,
    external:            ArenaMap<'s, StringCacheId, usize>,
    scope_entries:       ArenaVec<'s, &'s Collection<'s, usize>>,
    parameters:          ArenaMap<'s, SourceVectors, (&'tu [Parameter], usize)>,
    work:                ArenaVec<'s, Work<'tu, 's>>,
    values:              ArenaVec<'s, TypeId>,
    integers:            ArenaVec<'s, Option<Integer>>,
    tainted:             bool,
    old_parameter_mode:  bool,
    semantic_errors:     usize,
    member_names:        ArenaMap<'s, (usize, StringCacheId), SourceVectors>,
    tag_declarations:    ArenaVec<'tu, (usize, usize)>,
    resolved_type_names: ArenaMap<'s, SourceVectors, TypeId>,
    integer_models:      ArenaMap<'s, usize, Option<(u32, bool)>>,
    ice_operands:        ArenaMap<'s, (usize, bool), bool>,
}

/// Runs declaration analysis only after the complete immutable syntax tree
/// exists. C99: §5.1.1.2p1, p. 10; PDF p. 22.
pub(crate) fn analyze<'tu>(
    context: &mut Context<'tu>,
    unit: &ParsedTranslationUnit<'tu>,
) -> SemanticTranslationUnit<'tu> {
    let scratch = Bump::new();
    let mut analyzer = Analyzer::new(context, &scratch);
    for &root in unit.external_declarations().iter().rev() {
        analyzer.work.push(Work::Root(root));
    }
    while let Some(work) = analyzer.work.pop() {
        analyzer.step(work);
    }
    SemanticTranslationUnit {
        types:            analyzer.types.finish(),
        bindings:         analyzer.bindings.leak(),
        scopes:           analyzer.scopes.leak(),
        type_names:       analyzer.type_names.leak(),
        parameters:       analyzer.parameter_lists.leak(),
        tag_declarations: analyzer.tag_declarations.leak(),
    }
}

impl<'c, 'tu, 's> Analyzer<'c, 'tu, 's> {
    fn new(context: &'c mut Context<'tu>, scratch: &'s Bump) -> Self {
        let tu = context.tu_arena();
        let mut analyzer = Self {
            context,
            scratch,
            types: TypeInterner::new(tu, scratch),
            bindings: ArenaVec::new_in(tu),
            scopes: ArenaVec::new_in(tu),
            type_names: ArenaVec::new_in(tu),
            parameter_lists: ArenaVec::new_in(tu),
            scope: 0,
            entries: ArenaVec::new_in(scratch),
            visible: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            external: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            scope_entries: ArenaVec::new_in(scratch),
            parameters: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            work: ArenaVec::new_in(scratch),
            values: ArenaVec::new_in(scratch),
            integers: ArenaVec::new_in(scratch),
            tainted: false,
            old_parameter_mode: false,
            semantic_errors: 0,
            member_names: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            tag_declarations: ArenaVec::new_in(tu),
            resolved_type_names: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            integer_models: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            ice_operands: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
        };
        analyzer.scopes.push(Scope {
            parent: None,
            kind:   ScopeKind::File,
        });
        analyzer
            .scope_entries
            .push(scratch.alloc(Collection::new()));
        analyzer
    }

    fn error(
        &mut self,
        kind: SemanticErrorKind,
        source_vectors: SourceVectors,
        name: Option<StringCacheId>,
        previous: Option<SourceVectors>,
    ) {
        if !self.tainted {
            self.semantic_errors += 1;
            self.context
                .append_pending_errors([TranslationError::Semantic(SemanticError {
                    kind,
                    source_vectors,
                    name,
                    previous,
                })]);
        }
    }

    /// C99: §6.2.1p4, p. 29; PDF p. 41.
    fn enter(&mut self, kind: ScopeKind) {
        let next = self.scopes.len();
        self.scopes.push(Scope {
            parent: Some(self.scope),
            kind,
        });
        self.scope_entries
            .push(self.scratch.alloc(Collection::new()));
        self.scope = next;
    }

    /// C99: §6.2.1p4, p. 29; PDF p. 41.
    fn leave(&mut self) {
        let mut next = self.scope_entries[self.scope].head.get();
        while let Some(link) = next {
            next = link.next;
            let entry = self.entries[link.value];
            if let Some(previous) = entry.previous {
                _ = self
                    .visible
                    .insert((entry.namespace, entry.name.name), previous);
            } else {
                _ = self.visible.remove(&(entry.namespace, entry.name.name));
            }
        }
        self.scope = self.scopes[self.scope].parent.unwrap_or(0);
    }

    /// C99: §6.2.1p2-4, p. 29; PDF p. 41; §6.2.3p1, p. 31; PDF p. 43.
    fn lookup(&self, namespace: Namespace, name: StringCacheId) -> Option<Entry> {
        self.visible
            .get(&(namespace, name))
            .map(|&i| self.entries[i])
    }

    /// C99: §6.2.1p7, p. 30; PDF p. 42; §6.2.3p1, p. 31; PDF p. 43.
    fn install(&mut self, name: Identifier, namespace: Namespace, binding: usize) {
        let previous = self.visible.get(&(namespace, name.name)).copied();
        let index = self.entries.len();
        self.entries.push(Entry {
            name,
            namespace,
            binding,
            scope: self.scope,
            previous,
        });
        _ = self.visible.insert((namespace, name.name), index);
        self.scope_entries[self.scope].push(self.scratch, index);
    }

    fn take_type(&mut self) -> TypeId {
        self.values.pop().unwrap_or_else(|| self.types.unknown())
    }

    fn taint(&mut self, recovered: bool) {
        self.work.push(Work::RestoreTaint(self.tainted));
        self.tainted |= recovered;
    }

    /// C99: §6.7.2p2-5, pp. 99-100; PDF pp. 111-112.
    fn spec(&mut self, spec: DeclarationSpecifiers<'tu>, force_tag: bool) {
        let mut extension = spec.extensions;
        let mut unmodeled = spec.auto_with_storage_class;
        while let Some(item) = extension {
            unmodeled |= !matches!(item.kind, SpecifierExtensionKind::ExtensionMarker);
            extension = item.next;
        }
        if unmodeled {
            self.taint(true);
            self.work.push(Work::UnknownType);
        }
        self.work.push(Work::Spec(
            spec.type_specifiers,
            spec.type_qualifiers,
            spec.source_vectors,
            force_tag,
        ));
        let mut extension = spec.extensions;
        while let Some(item) = extension {
            if let SpecifierExtensionKind::Alignment(operand) = item.kind {
                self.syntax_operand(operand);
            }
            extension = item.next;
        }
    }

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
            | Work::PopScope => self.leave(),
            | Work::DiscardType => {
                _ = self.values.pop();
            },
            | Work::UnknownType => {
                if let Some(ty) = self.values.pop()
                    && let TypeKind::Tag(id) = self.types.nodes[ty.index]
                {
                    self.types.tags[id].tainted.set(true);
                    self.types.tags[id].layout.set(None);
                }
                self.values.push(self.types.unknown());
            },
            | Work::Declaration(d) => {
                if let Some(assertion) = d.assertion {
                    self.work.push(Work::Expression(assertion.expression));
                    return;
                }
                self.taint(d.recovered);
                self.work.push(Work::DeclarationBase(d));
                self.spec(d.declaration_specifiers, d.init_declarators.is_empty());
            },
            | Work::DeclarationBase(d) => {
                let base = self.take_type();
                for init in d.init_declarators.iter().rev() {
                    if let Some(initializer) = init.initializer {
                        self.work.push(Work::Initializer(initializer));
                    }
                    self.work.push(Work::Bind(
                        init.declarator,
                        d.declaration_specifiers,
                        init.initializer.is_some(),
                    ));
                    self.work
                        .push(Work::Declarator(init.declarator, base, false));
                }
            },
            | Work::Bind(d, s, initialized) => {
                let ty = self.take_type();
                self.bind_declaration(d, s, ty, initialized);
            },
            | Work::Function(f) => {
                self.taint(f.recovered);
                self.work.push(Work::FunctionBase(f));
                self.spec(f.declaration_specifiers, false);
            },
            | Work::FunctionBase(f) => {
                let base = self.take_type();
                self.work.push(Work::FunctionBody(f));
                self.work
                    .push(Work::Bind(f.declarator, f.declaration_specifiers, false));
                self.work.push(Work::Declarator(f.declarator, base, false));
            },
            | Work::FunctionBody(f) => {
                self.enter(ScopeKind::Function);
                if let Some(&(params, prototype_scope)) =
                    self.parameters.get(&f.declarator.source_vectors)
                {
                    // Parameter-list tags/enumerators have function scope in
                    // definitions.
                    let entries: &[usize] = if prototype_scope == 0 {
                        &[]
                    } else {
                        self.scope_entries[prototype_scope].finish(self.scratch, self.scratch)
                    };
                    for &index in entries {
                        let entry = self.entries[index];
                        if entry.namespace == Namespace::Tag {
                            self.install(entry.name, entry.namespace, entry.binding);
                        } else if self.bindings[entry.binding].kind == BindingKind::Enumerator {
                            let binding = self.bindings[entry.binding];
                            self.bind(
                                binding.name,
                                binding.ty,
                                binding.kind,
                                binding.linkage,
                                binding.duration,
                                binding.value,
                            );
                        }
                    }
                    for param in params {
                        if !f.declaration_list.is_empty() {
                            continue;
                        }
                        if let Some(name) = param.name {
                            self.bind(
                                name,
                                param.ty,
                                BindingKind::Parameter,
                                Linkage::None,
                                Duration::Automatic,
                                None,
                            );
                        }
                    }
                }
                self.work.push(Work::PopScope);
                self.work.push(Work::Statement(f.body, false));
                self.work
                    .push(Work::RestoreParameterMode(self.old_parameter_mode));
                if !f.declaration_list.is_empty() {
                    self.work.push(Work::OldSignature(f));
                }
                self.old_parameter_mode = !f.declaration_list.is_empty();
                // Old-style parameter declarations precede the body.
                for &d in f.declaration_list.iter().rev() {
                    self.work.push(Work::Declaration(d));
                }
            },
            | Work::OldSignature(f) => self.old_signature(f),
            | Work::BlockItem(item) => match item {
                | BlockItem::Declaration(d) => self.work.push(Work::Declaration(d)),
                | BlockItem::Statement(s) => self.work.push(Work::Statement(s, true)),
                | BlockItem::FunctionDefinition(f) => self.work.push(Work::Function(f)),
            },
            | Work::Statement(statement, new_scope) => self.statement(statement, new_scope),
            | Work::Expression(expression) => self.expression(expression),
            | Work::Slot(slot) => self.expression_slot(slot),
            | Work::Initializer(initializer) => match initializer.kind {
                | InitializerType::AssignmentExpression(e) => self.work.push(Work::Expression(e)),
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
            },
            | Work::Spec(s, q, source, force) => self.resolve_spec(s, q, source, force),
            | Work::Qualify(q, source) => {
                let mut ty = self.take_type();
                if !(q
                    - (TypeQualifiers::CONST | TypeQualifiers::VOLATILE | TypeQualifiers::RESTRICT))
                    .is_empty()
                {
                    self.values.push(self.types.unknown());
                    return;
                }
                let mut arrays = ArenaVec::new_in(self.scratch);
                while let TypeKind::Array(element, bound) = self.types.nodes[ty.index] {
                    arrays.push(bound);
                    ty = element;
                }
                if matches!(self.types.nodes[ty.index], TypeKind::Function { .. }) && !q.is_empty()
                {
                    self.validate_qualifiers(ty.qualified(q), source);
                    self.error(SemanticErrorKind::QualifiedFunction, source, None, None);
                } else {
                    ty = ty.qualified(q);
                }
                while let Some(bound) = arrays.pop() {
                    ty = self.types.intern(TypeKind::Array(ty, bound));
                }
                self.validate_qualifiers(ty, source);
                self.values.push(ty);
            },
            | Work::TypeName(name) => {
                if let Some(&ty) = self.resolved_type_names.get(&name.source_vectors) {
                    self.values.push(ty);
                    return;
                }
                self.work.push(Work::TypeNameDone(name.source_vectors));
                self.work.push(Work::TypeNameBase(name));
                self.spec(name.declaration_specifiers, false);
            },
            | Work::TypeNameBase(name) => {
                let base = self.take_type();
                if let Some(d) = name.declarator {
                    self.work.push(Work::Declarator(d, base, false));
                } else {
                    self.values.push(base);
                }
            },
            | Work::TypeNameDone(source) =>
                if let Some(&ty) = self.values.last() {
                    self.type_names.push((source, ty));
                    _ = self.resolved_type_names.insert(source, ty);
                },
            | Work::Declarator(d, mut base, parameter) => {
                for level in d.pointer.levels {
                    if !(level.qualifiers
                        - (TypeQualifiers::CONST
                            | TypeQualifiers::VOLATILE
                            | TypeQualifiers::RESTRICT))
                        .is_empty()
                    {
                        base = self.types.unknown();
                        continue;
                    }
                    if level.attributes.is_some() {
                        base = self.types.unknown();
                        continue;
                    }
                    base = self
                        .types
                        .intern(TypeKind::Pointer(base))
                        .qualified(level.qualifiers);
                    self.validate_qualifiers(base, d.source_vectors);
                }
                self.values.push(base);
                for &direct in d.kind {
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
                    if value.value < 0 {
                        self.error(SemanticErrorKind::InvalidArrayBound, source, None, None);
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
                    u64::try_from(value.value)
                        .ok()
                        .map_or(ArrayBound::Variable, ArrayBound::Constant)
                } else {
                    ArrayBound::Variable
                };
                let ty = self
                    .types
                    .intern(TypeKind::Array(element, bound))
                    .qualified(q);
                self.values.push(ty);
            },
            | Work::FunctionParameters(list, index, params, result, variadic, source) =>
                if let Some(&p) = list.as_slice().get(index) {
                    self.work.push(Work::FunctionParameters(
                        list,
                        index + 1,
                        params,
                        result,
                        variadic,
                        source,
                    ));
                    self.work.push(Work::ParameterDone(p, params));
                    self.work.push(Work::ParameterBase(p));
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
                    _ = self.parameters.insert(source, (params, scope));
                    self.parameter_lists.push((source, params));
                    self.values.push(self.types.intern(TypeKind::Function {
                        result,
                        parameters,
                        prototype,
                        variadic,
                    }));
                },
            | Work::ParameterBase(p) => {
                let base = self.take_type();
                if let Some(d) = p.declarator {
                    self.validate_parameter_arrays(d);
                    self.work.push(Work::Declarator(d, base, true));
                } else {
                    self.values.push(base);
                }
            },
            | Work::ParameterDone(p, params) => {
                let mut ty = self.take_type();
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
                {
                    self.error(
                        SemanticErrorKind::InvalidParameter,
                        p.source_vectors,
                        name.map(|n| n.name),
                        None,
                    );
                }
                if let Some(name) = name {
                    self.bind(
                        name,
                        ty,
                        BindingKind::Parameter,
                        Linkage::None,
                        Duration::Automatic,
                        None,
                    );
                }
                params.push(
                    self.scratch,
                    Parameter {
                        name,
                        ty,
                        array_minimum,
                    },
                );
            },
            | Work::RecordMembers(tag, list, index, members) => {
                if let Some(&member) = list.as_slice().get(index) {
                    self.work
                        .push(Work::RecordMembers(tag, list, index + 1, members));
                    self.work.push(Work::RecordMemberBase(tag, member, members));
                    self.work.push(Work::Spec(
                        member.type_specifiers,
                        member.type_qualifiers,
                        member.source_vectors,
                        false,
                    ));
                } else {
                    self.complete_record(tag, members.finish(self.types.tu, self.scratch));
                }
            },
            | Work::RecordMemberBase(tag, member, members) => {
                let base = self.take_type();
                for &d in member.struct_declarator_list.iter().rev() {
                    self.work.push(Work::MemberBase(tag, d, base, members));
                }
                if member.extensions.is_some() {
                    self.types.tags[tag].tainted.set(true);
                }
            },
            | Work::MemberBase(tag, d, base, members) => {
                self.work.push(Work::MemberDone(tag, d, members));
                if let Some(decl) = d.declarator {
                    self.work.push(Work::Declarator(decl, base, false));
                } else {
                    self.values.push(base);
                }
            },
            | Work::MemberDone(tag, d, members) => {
                let ty = self.take_type();
                self.work.push(Work::MemberWidth(tag, d, ty, members));
                if let Some(width) = d.bitfield_width {
                    self.work.push(Work::Eval(width.expression()));
                    self.work.push(Work::Expression(width.expression()));
                } else {
                    self.integers.push(None);
                }
            },
            | Work::MemberWidth(tag, d, ty, members) => {
                let value = self.integers.pop().flatten();
                let width = if d.bitfield_width.is_some() {
                    let valid = value.and_then(|v| u32::try_from(v.value).ok());
                    let bits = self.integer_type(ty).map(|(bits, _)| bits);
                    let name = d.declarator.and_then(Declarator::identifier);
                    if valid.is_none()
                        || bits.is_none()
                        || valid > bits
                        || (valid == Some(0) && name.is_some())
                    {
                        if !self.types.unanalyzed(ty)
                            && !d
                                .bitfield_width
                                .is_some_and(|w| self.unanalyzed_constant(w.expression()))
                        {
                            self.error(
                                SemanticErrorKind::InvalidBitField,
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
                        ty,
                        width,
                        offset: 0,
                        bit_offset: 0,
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
                    self.types.tags[tag].complete.set(true);
                    if !self.types.tags[tag].tainted.get() {
                        self.types.tags[tag]
                            .layout
                            .set(self.types.target.scalar(Scalar::Int));
                    }
                }
            },
            | Work::EnumeratorDone(tag, list, index, errors_before, invalid_implicit) => {
                let value = self.integers.pop().flatten();
                let item = list.as_slice()[index];
                if value.is_none()
                    && self.semantic_errors == errors_before
                    && !invalid_implicit
                    && !item
                        .expression
                        .is_some_and(|e| self.unanalyzed_constant(e.expression()))
                {
                    self.error(
                        SemanticErrorKind::InvalidConstant,
                        item.source_vectors,
                        Some(item.name.name),
                        None,
                    );
                }
                if let Some(v) = value
                    && i32::try_from(v.value).is_err()
                {
                    if self.context.configuration.standard() < CStandard::C23
                        && !self.types.tags[tag].tainted.get()
                    {
                        self.error(
                            SemanticErrorKind::EnumeratorRange,
                            item.source_vectors,
                            Some(item.name.name),
                            None,
                        );
                    }
                    self.types.tags[tag].tainted.set(true);
                }
                if value.is_none() {
                    self.types.tags[tag].tainted.set(true);
                }
                let opaque = self.types.tags[tag].tainted.get();
                let ty = if opaque {
                    self.types.unknown()
                } else {
                    self.types.scalar(Scalar::Int)
                };
                self.bind(
                    item.name,
                    ty,
                    BindingKind::Enumerator,
                    Linkage::None,
                    Duration::None,
                    if opaque {
                        None
                    } else {
                        value.map(|v| Integer::int(v.value))
                    },
                );
                self.work
                    .push(Work::EnumMembers(tag, list, index + 1, value));
            },
            | Work::Eval(expression) => self.evaluate(expression),
            | Work::Unary(op, source) => {
                let value = self.integers.pop().flatten();
                let result = value.and_then(|v| v.unary(op));
                if value.is_some() && result.is_none() {
                    self.error(SemanticErrorKind::ConstantOverflow, source, None, None);
                }
                self.integers.push(result);
            },
            | Work::Binary(op, source) => {
                let right = self.integers.pop().flatten();
                let left = self.integers.pop().flatten();
                let result = left.zip(right).and_then(|(l, r)| l.binary(op, r));
                if left.is_some() && right.is_some() && result.is_none() {
                    self.error(SemanticErrorKind::ConstantOverflow, source, None, None);
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
                        self.error(SemanticErrorKind::ConstantOverflow, source, None, None);
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
                let layout = self
                    .types
                    .layout(ty)
                    .or_else(|| gnu_unit_size.then_some(Layout { size: 1, align: 1 }));
                if layout.is_none()
                    && !self.types.unanalyzed(ty)
                    && !matches!(
                        self.types.nodes[ty.index],
                        TypeKind::Unknown
                            | TypeKind::Array(_, ArrayBound::Variable | ArrayBound::Star)
                    )
                {
                    self.error(SemanticErrorKind::InvalidConstant, source, None, None);
                }
                self.integers.push(layout.map(|l| Integer {
                    value:  i128::from(if align { l.align } else { l.size }),
                    bits:   64,
                    signed: false,
                }));
            },
        }
    }
}
