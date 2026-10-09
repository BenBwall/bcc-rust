//! Phase-7 function definitions and translation-unit completion.
//! C99: §6.9-§6.9.2, pp. 140-143; PDF pp. 152-155; inline definitions
//! §6.7.4p3, p. 112; PDF p. 124. This validates bodies without lowering them.

use super::{
    Analyzer,
    ArenaMap,
    ArenaVec,
    ArrayBound,
    BindingKind,
    Bump,
    CStandard,
    Declaration,
    DeclarationSpecifiers,
    Declarator,
    DirectDeclarator,
    Duration,
    Expression,
    FunctionDefinition,
    FxBuildHasher,
    Identifier,
    Linkage,
    Namespace,
    Parameter,
    Scalar,
    ScopeKind,
    SemanticErrorKind,
    SourceVectors,
    StorageClass,
    StringCacheId,
    TypeId,
    TypeKind,
    TypeQualifiers,
    TypeSpecifiers,
    Work,
};

/// Finalized definitions, separate from declaration occurrences. The implicit
/// function-name object retains its string contents for future lowering.
/// C99: §6.9.1-§6.9.2, pp. 141-143; PDF pp. 153-155; §6.4.2.2p1,
/// p. 52; PDF p. 64.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DefinitionKind {
    Object,
    Function,
    Inline,
    Tentative,
    FunctionName(StringCacheId),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Definition {
    pub(crate) binding: usize,
    pub(crate) kind:    DefinitionKind,
}

#[derive(Clone, Copy)]
pub(super) struct FunctionContext<'tu> {
    pub(super) id:         usize,
    pub(super) syntax:     &'tu FunctionDefinition<'tu>,
    pub(super) binding:    Option<usize>,
    pub(super) result:     TypeId,
    pub(super) parameters: &'tu [Parameter],
}

#[derive(Clone, Copy)]
pub(super) enum FunctionWork<'tu> {
    Finish(
        Option<FunctionContext<'tu>>,
        usize,
        Option<usize>,
        Option<usize>,
    ),
    RestoreSizeof(Option<usize>),
}

#[derive(Clone, Copy, Default)]
pub(super) struct Entity {
    latest:        usize,
    definition:    Option<SourceVectors>,
    tentative:     Option<usize>,
    non_inline:    bool,
    function_body: Option<SourceVectors>,
}

#[derive(Clone, Copy)]
struct Use {
    binding:         usize,
    name:            Identifier,
    sizeof:          Option<usize>,
    inline_function: Option<usize>,
}

#[derive(Clone, Copy)]
struct SizeofContext<'tu> {
    expression: &'tu Expression<'tu>,
    parent:     Option<usize>,
}

#[derive(Clone, Copy)]
struct InlineObject {
    function: usize,
    binding:  usize,
}

pub(super) struct State<'tu, 's> {
    pub(super) current: Option<FunctionContext<'tu>>,
    next_function:      usize,
    old_names:          ArenaMap<'s, (usize, StringCacheId), SourceVectors>,
    entities:           ArenaMap<'s, StringCacheId, Entity>,
    previous:           ArenaMap<'s, usize, SourceVectors>,
    uses:               ArenaVec<'s, Use>,
    sizeof_contexts:    ArenaVec<'s, SizeofContext<'tu>>,
    pub(super) sizeof:  Option<usize>,
    inline_objects:     ArenaVec<'s, InlineObject>,
}

impl<'s> State<'_, 's> {
    pub(super) fn new(scratch: &'s Bump) -> Self {
        Self {
            current:         None,
            next_function:   0,
            old_names:       ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            entities:        ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            previous:        ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            uses:            ArenaVec::new_in(scratch),
            sizeof_contexts: ArenaVec::new_in(scratch),
            sizeof:          None,
            inline_objects:  ArenaVec::new_in(scratch),
        }
    }
}

/// Finds the first derivation outward from the identifier, including grouping.
/// A function typedef supplies no such derivation. Identity is the immutable
/// suffix node, not the source span of its containing declarator.
/// C99: §6.7.5p4-6, pp. 114-115; PDF pp. 126-127; §6.9.1p2, p. 141;
/// PDF p. 153.
pub(super) fn function_derivation<'tu>(
    declarator: Declarator<'tu>,
    scratch: &Bump,
) -> Option<&'tu DirectDeclarator<'tu>> {
    enum Step<'tu> {
        Declarator(Declarator<'tu>),
        Direct(&'tu DirectDeclarator<'tu>),
        Pointer,
    }
    let mut work = ArenaVec::new_in(scratch);
    work.push(Step::Declarator(declarator));
    while let Some(step) = work.pop() {
        match step {
            | Step::Declarator(d) => {
                if !d.pointer.levels.is_empty() {
                    work.push(Step::Pointer);
                }
                for direct in d.kind.as_slice().iter().rev() {
                    work.push(Step::Direct(direct));
                }
            },
            | Step::Direct(direct) => match direct {
                | DirectDeclarator::Parenthesized(p) => work.push(Step::Declarator(p.declarator)),
                | DirectDeclarator::Function { .. }
                | DirectDeclarator::KAndRStyleFunction { .. } => return Some(direct),
                | DirectDeclarator::Array { .. } => return None,
                | _ => {},
            },
            | Step::Pointer => return None,
        }
    }
    None
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    pub(super) fn prepare_function(&mut self, f: &'tu FunctionDefinition<'tu>) {
        if let Some(name) = f.declarator.identifier()
            && let Some(entry) = self.lookup(Namespace::Ordinary, name.name)
        {
            _ = self.functions.previous.insert(
                std::ptr::from_ref(f).addr(),
                self.bindings[entry.binding].name.source_vectors,
            );
        }
    }

    /// C99: §6.9.1p2-7, pp. 141-142; PDF pp. 153-154.
    pub(super) fn function_body(&mut self, f: &'tu FunctionDefinition<'tu>) {
        let declared_type = self.take_type();
        let nested = self.scopes[self.scope].kind != ScopeKind::File;
        let name = f.declarator.identifier();
        if nested {
            // GNU nested functions denote lexical entities, not translation-
            // unit externals. Their parser owner reports the extension.
            if let Some(name) = name {
                self.bind(
                    name,
                    declared_type,
                    BindingKind::Function,
                    Linkage::None,
                    Duration::None,
                    None,
                );
            }
        } else {
            self.bind_declaration(f.declarator, f.declaration_specifiers, declared_type, false);
        }
        let binding = name
            .and_then(|n| self.lookup(Namespace::Ordinary, n.name))
            .map(|e| e.binding);
        let ty = binding.map_or_else(|| self.types.unknown(), |i| self.bindings[i].ty);
        let derivation = function_derivation(f.declarator, self.scratch);
        if derivation.is_none()
            || (!self.types.unanalyzed(ty)
                && !matches!(self.types.nodes[ty.index], TypeKind::Function { .. }))
        {
            self.error(
                SemanticErrorKind::InvalidFunctionDefinition,
                f.declarator.source_vectors,
                name.map(|n| n.name),
                None,
            );
        }
        if !nested && f.declaration_specifiers.storage_class == Some(StorageClass::Typedef) {
            self.error(
                SemanticErrorKind::FunctionDefinitionStorage,
                f.declaration_specifiers.source_vectors,
                name.map(|n| n.name),
                None,
            );
        }
        let mut result = match self.types.nodes[ty.index] {
            | TypeKind::Function { result, .. } => result,
            | _ => self.types.unknown(),
        };
        if !self.types.unanalyzed(result)
            && !matches!(
                self.types.nodes[result.index],
                TypeKind::Scalar(Scalar::Void)
            )
            && !self.complete_object(result)
        {
            self.error(
                SemanticErrorKind::IncompleteFunctionReturn,
                f.declarator.source_vectors,
                name.map(|n| n.name),
                None,
            );
            result = self.types.unknown();
        }
        if derivation.is_none()
            || matches!(
                self.types.nodes[result.index],
                TypeKind::Array(..) | TypeKind::Function { .. }
            )
        {
            result = self.types.unknown();
        }
        let (parameters, prototype_scope) = derivation
            .and_then(|d| self.parameters.get(&std::ptr::from_ref(d).addr()).copied())
            .unwrap_or((&[], 0));
        let old_style = derivation
            .is_some_and(|d| matches!(d, DirectDeclarator::KAndRStyleFunction { .. }))
            || (parameters.is_empty() && !f.declaration_list.is_empty());
        if !old_style && !f.declaration_list.is_empty() {
            self.error(
                SemanticErrorKind::InvalidDefinitionParameterList,
                f.source_vectors,
                name.map(|n| n.name),
                None,
            );
        }
        if let Some(index) = binding {
            self.record_function_definition(index, f);
        }
        self.work.push(Work::FunctionWork(FunctionWork::Finish(
            self.functions.current,
            self.statements.loops,
            self.statements.switch,
            self.functions.sizeof,
        )));
        self.enter(ScopeKind::Function);
        self.statements.vm = None;
        self.functions.sizeof = None;
        self.statements.loops = 0;
        self.statements.switch = None;
        let id = self.functions.next_function;
        self.functions.next_function += 1;
        self.functions.current = Some(FunctionContext {
            id,
            syntax: f,
            binding,
            result,
            parameters,
        });
        self.function_name = name;
        if old_style {
            for parameter in parameters {
                if let Some(name) = parameter.name {
                    _ = self
                        .functions
                        .old_names
                        .insert((id, name.name), name.source_vectors);
                }
            }
        }
        if prototype_scope != 0 {
            let entries = self.scope_entries[prototype_scope].finish(self.scratch, self.scratch);
            for &index in entries {
                let entry = self.entries[index];
                if entry.namespace == Namespace::Tag {
                    self.install(entry.name, entry.namespace, entry.binding);
                } else if entry.namespace == Namespace::Ordinary
                    && self.bindings[entry.binding].kind == BindingKind::Enumerator
                {
                    let b = self.bindings[entry.binding];
                    self.bind(b.name, b.ty, b.kind, b.linkage, b.duration, b.value);
                }
            }
        }
        if !old_style {
            let void = matches!(parameters, [Parameter { name: None, ty, .. }] if matches!(self.types.nodes[ty.index], TypeKind::Scalar(Scalar::Void)));
            for (parameter_index, parameter) in parameters.iter().enumerate() {
                if void {
                    continue;
                }
                if let Some(name) = parameter.name {
                    // Prototype construction already diagnosed duplicate
                    // parameter/enumerator names. Replay each binding once.
                    if self
                        .lookup(Namespace::Ordinary, name.name)
                        .is_some_and(|e| e.scope == self.scope)
                    {
                        continue;
                    }
                    // Void parameters were already rejected while the
                    // prototype was constructed. Failed parameter types must
                    // not generate dependent errors in the body.
                    let valid = !matches!(
                        self.types.nodes[parameter.ty.index],
                        TypeKind::Scalar(Scalar::Void)
                    ) && self.validate_definition_parameter(
                        parameter.ty,
                        name.source_vectors,
                        Some(name.name),
                    );
                    let parameter_type = if valid {
                        parameter.ty
                    } else {
                        self.types.unknown()
                    };
                    self.bind(
                        name,
                        parameter_type,
                        BindingKind::Parameter,
                        Linkage::None,
                        Duration::Automatic,
                        None,
                    );
                    self.record_vm(self.bindings.len() - 1);
                    // The body binding is distinct from its prototype binding.
                    // C99 §6.5.3.2p1, p. 78; PDF p. 90.
                    if matches!(derivation, Some(DirectDeclarator::Function { parameter_list, .. })
                        if parameter_list.as_slice().get(parameter_index).is_some_and(|p|
                            p.declaration_specifiers.storage_class == Some(StorageClass::Register)))
                    {
                        _ = self.register_bindings.insert(self.bindings.len() - 1, true);
                    }
                } else if self.context.configuration.standard() < CStandard::C23 {
                    let syntax = match derivation {
                        | Some(DirectDeclarator::Function { parameter_list, .. }) =>
                            parameter_list.as_slice().get(parameter_index),
                        | _ => None,
                    };
                    let parser_exempt = syntax.is_some_and(|p| {
                        parameters.len() == 1
                            && p.declarator.is_none()
                            && matches!(
                                p.declaration_specifiers.type_specifiers,
                                TypeSpecifiers::Void | TypeSpecifiers::TypedefName(_)
                            )
                    });
                    // The parser already reports the C23 spelling under a
                    // pedantic policy. Do not emit the same requirement twice.
                    if parser_exempt
                        || self.context.configuration.extension_policy()
                            == crate::configuration::ExtensionPolicy::Allow
                    {
                        self.error(
                            SemanticErrorKind::UnnamedDefinitionParameter,
                            syntax.map_or(f.declarator.source_vectors, |p| p.source_vectors),
                            None,
                            None,
                        );
                    }
                }
            }
            self.validate_definition_stars(derivation);
        }
        self.define_func_name(name);
        if !old_style {
            self.check_main(f, ty);
        }
        self.work.push(Work::PopScope);
        self.work.push(Work::Statement(f.body, false));
        self.work
            .push(Work::RestoreParameterMode(self.old_parameter_mode));
        if old_style {
            self.work.push(Work::OldSignature(f));
        }
        self.old_parameter_mode = old_style;
        for &d in f.declaration_list.iter().rev() {
            self.work.push(Work::Declaration(d));
        }
    }

    pub(super) fn function_work(&mut self, work: FunctionWork<'tu>) {
        match work {
            | FunctionWork::Finish(previous, loops, switch, sizeof) => {
                self.functions.current = previous;
                self.functions.sizeof = sizeof;
                self.function_name = previous.and_then(|f| f.syntax.declarator.identifier());
                self.statements.loops = loops;
                self.statements.switch = switch;
            },
            | FunctionWork::RestoreSizeof(previous) => self.functions.sizeof = previous,
        }
    }

    /// Definition parameters have complete adjusted object types.
    /// C99: §6.7.5.3p4, p. 118; PDF p. 130; §6.9.1p7, p. 142;
    /// PDF p. 154.
    fn validate_definition_parameter(
        &mut self,
        ty: TypeId,
        source: SourceVectors,
        name: Option<StringCacheId>,
    ) -> bool {
        if !self.types.unanalyzed(ty) && !self.complete_object(ty) {
            self.error(
                SemanticErrorKind::IncompleteDefinitionParameter,
                source,
                name,
                None,
            );
            return false;
        }
        true
    }

    /// `[*]` belongs to prototype scope, not a definition's parameter scope.
    /// C99: §6.7.5.2p4, p. 117; PDF p. 129; §6.9.1p7, p. 142; PDF p. 154.
    fn validate_definition_stars(&mut self, derivation: Option<&'tu DirectDeclarator<'tu>>) {
        let Some(DirectDeclarator::Function { parameter_list, .. }) = derivation else {
            return;
        };
        let mut work = ArenaVec::new_in(self.scratch);
        for parameter in *parameter_list {
            if let Some(d) = parameter.declarator {
                work.push(d);
            }
        }
        while let Some(d) = work.pop() {
            for direct in d.kind {
                match direct {
                    | DirectDeclarator::Parenthesized(p) => work.push(p.declarator),
                    | DirectDeclarator::Array {
                        is_pointer: true, ..
                    } => self.error(
                        SemanticErrorKind::DefinitionStarArray,
                        d.source_vectors,
                        d.identifier().map(|n| n.name),
                        None,
                    ),
                    // Nested function prototypes retain their own prototype scope.
                    | _ => {},
                }
            }
        }
    }

    pub(super) fn validate_declaration_list_item(&mut self, d: &'tu Declaration<'tu>) {
        if self.old_parameter_mode && d.init_declarators.is_empty() && d.assertion.is_none() {
            self.error(
                SemanticErrorKind::InvalidDefinitionParameterList,
                d.source_vectors,
                None,
                None,
            );
        }
    }

    /// C99: §6.9.1p6, p. 141; PDF p. 153.
    pub(super) fn validate_old_parameter(
        &mut self,
        name: Identifier,
        _spec: DeclarationSpecifiers<'tu>,
        ty: TypeId,
        initialized: bool,
    ) -> bool {
        let Some(function) = self.functions.current else {
            return true;
        };
        let mut valid = true;
        if !self
            .functions
            .old_names
            .contains_key(&(function.id, name.name))
            || initialized
        {
            valid = false;
            self.error(
                SemanticErrorKind::InvalidDefinitionParameterList,
                name.source_vectors,
                Some(name.name),
                None,
            );
        }
        if let Some(entry) = self.lookup(Namespace::Ordinary, name.name)
            && self.bindings[entry.binding].kind == BindingKind::Typedef
        {
            valid = false;
            self.error(
                SemanticErrorKind::InvalidDefinitionParameterList,
                name.source_vectors,
                Some(name.name),
                Some(entry.name.source_vectors),
            );
        }
        let adjusted = match self.types.nodes[ty.index] {
            | TypeKind::Array(element, _) => self.types.intern(TypeKind::Pointer(element)),
            | TypeKind::Function { .. } => self.types.intern(TypeKind::Pointer(ty)),
            | _ => ty,
        };
        self.validate_definition_parameter(adjusted, name.source_vectors, Some(name.name)) && valid
    }

    /// Missing K&R declarations use C89 implicit int through shared policy.
    /// Promoted signatures use the identical defining suffix as body bindings.
    /// C99: §6.7.5.3p15, pp. 119-120; PDF pp. 131-132; §6.9.1p6,
    /// p. 141; PDF p. 153. Implicit int is a pre-C99 extension.
    pub(super) fn old_signature(&mut self, function: &'tu FunctionDefinition<'tu>) {
        let Some(current) = self.functions.current else {
            return;
        };
        let mut seen = ArenaMap::with_hasher_in(FxBuildHasher, self.scratch);
        let mut parameters = ArenaVec::new_in(self.scratch);
        for parameter in current.parameters {
            let Some(name) = parameter.name else {
                continue;
            };
            if let Some(previous) = seen.insert(name.name, name.source_vectors) {
                self.error(
                    SemanticErrorKind::DuplicateDeclaration,
                    name.source_vectors,
                    Some(name.name),
                    Some(previous),
                );
                continue;
            }
            let entry = self
                .lookup(Namespace::Ordinary, name.name)
                .filter(|e| e.scope == self.scope);
            let declared = if let Some(entry) = entry {
                self.bindings[entry.binding].ty
            } else {
                let int = self.types.scalar(Scalar::Int);
                self.bind(
                    name,
                    int,
                    BindingKind::Parameter,
                    Linkage::None,
                    Duration::Automatic,
                    None,
                );
                if self.context.configuration.standard() >= CStandard::C99
                    && !self.context.configuration.gnu_extensions()
                {
                    self.error(
                        SemanticErrorKind::InvalidDefinitionParameterList,
                        name.source_vectors,
                        Some(name.name),
                        None,
                    );
                } else {
                    self.context.report_extension(
                        crate::configuration::Feature::ImplicitInt,
                        "implicit int parameter",
                        name.source_vectors,
                    );
                }
                int
            };
            let promoted = match self.types.nodes[declared.index] {
                | TypeKind::Scalar(
                    Scalar::Bool
                    | Scalar::Char
                    | Scalar::SignedChar
                    | Scalar::UnsignedChar
                    | Scalar::Short
                    | Scalar::UnsignedShort,
                ) => self.types.scalar(Scalar::Int),
                | TypeKind::Scalar(Scalar::Float) => self.types.scalar(Scalar::Double),
                | _ => declared.unqualified(),
            };
            parameters.push(promoted);
        }
        let Some(index) = current.binding else {
            return;
        };
        let binding = self.bindings[index];
        let TypeKind::Function { result, .. } = self.types.nodes[binding.ty.index] else {
            return;
        };
        let parameters = self.types.tu.alloc_slice_copy(&parameters);
        let defined = self.types.intern(TypeKind::Function {
            result,
            parameters,
            prototype: false,
            variadic: false,
        });
        if let Some(composite) = self.types.composite(binding.ty, defined) {
            self.bindings[index].ty = composite;
            self.check_main(function, composite);
        } else {
            let previous = self
                .functions
                .previous
                .get(&std::ptr::from_ref(function).addr())
                .copied();
            self.error(
                SemanticErrorKind::IncompatibleDeclaration,
                binding.name.source_vectors,
                Some(binding.name.name),
                previous,
            );
        }
    }

    /// The predefined object exists at function entry, including before its
    /// first use and when an outer object has the same spelling.
    /// C99: §6.4.2.2p1, p. 52; PDF p. 64.
    fn define_func_name(&mut self, function: Option<Identifier>) {
        let Some(function) = function else {
            return;
        };
        let name = self.context.string_cache.intern("__func__");
        let element = self
            .types
            .scalar(Scalar::Char)
            .qualified(TypeQualifiers::CONST);
        let count = self.context.string_cache.at(function.name).len() as u64 + 1;
        let ty = self
            .types
            .intern(TypeKind::Array(element, ArrayBound::Constant(count)));
        self.bind(
            Identifier {
                name,
                source_vectors: function.source_vectors,
            },
            ty,
            BindingKind::Object,
            Linkage::None,
            Duration::Static,
            None,
        );
        self.definitions.push(Definition {
            binding: self.bindings.len() - 1,
            kind:    DefinitionKind::FunctionName(function.name),
        });
    }

    /// Hosted `main` signatures are implementation-defined outside the two
    /// portable forms; warn without rejecting them. A freestanding startup
    /// function's name and type are implementation-defined, so nothing is
    /// checked there.
    /// C99: §5.1.2.2.1p1, p. 12; PDF p. 24; §5.1.2.1p1, p. 11; PDF p. 23.
    fn check_main(&mut self, f: &'tu FunctionDefinition<'tu>, ty: TypeId) {
        if !self.context.configuration.hosted() {
            return;
        }
        let Some(name) = f.declarator.identifier() else {
            return;
        };
        if self.context.string_cache.at(name.name) != "main"
            || self.scopes[self.scope]
                .parent
                .is_some_and(|s| self.scopes[s].kind != ScopeKind::File)
        {
            return;
        }
        let TypeKind::Function {
            result,
            parameters,
            variadic,
            ..
        } = self.types.nodes[ty.index]
        else {
            return;
        };
        if self.types.unanalyzed(ty) {
            return;
        }
        let int = self.types.scalar(Scalar::Int);
        let char = self.types.scalar(Scalar::Char);
        let pointer = self.types.intern(TypeKind::Pointer(char));
        let argv = self.types.intern(TypeKind::Pointer(pointer));
        if f.declaration_specifiers.storage_class == Some(StorageClass::Static)
            || result != int
            || variadic
            || !(parameters.is_empty() || parameters == [int, argv])
        {
            self.error(
                SemanticErrorKind::MainSignature,
                name.source_vectors,
                Some(name.name),
                None,
            );
        }
    }

    /// C99: §6.9.2p1-3, p. 143; PDF p. 155; §6.7.4p3, p. 112;
    /// PDF p. 124. Definition state is separate from declaration occurrences.
    pub(super) fn record_declaration(
        &mut self,
        index: usize,
        spec: DeclarationSpecifiers<'tu>,
        initialized: bool,
    ) {
        if self.tainted {
            return;
        }
        let binding = self.bindings[index];
        self.record_vm(index);
        if binding.kind == BindingKind::Object && (initialized || binding.linkage == Linkage::None)
        {
            self.definitions.push(Definition {
                binding: index,
                kind:    DefinitionKind::Object,
            });
        }
        if binding.kind == BindingKind::Object
            && binding.duration == Duration::Static
            && (initialized || binding.linkage == Linkage::None)
            && !self.types.unanalyzed(binding.ty)
            && self.inline_object_modifiable(binding.ty)
            && let Some(function) = self.functions.current
            && function
                .syntax
                .declaration_specifiers
                .function_specifiers
                .is_inline
            && let Some(function) = function.binding
            && self.bindings[function].linkage == Linkage::External
        {
            self.functions.inline_objects.push(InlineObject {
                function,
                binding: index,
            });
        }
        if binding.linkage == Linkage::None {
            return;
        }
        let mut entity = self
            .functions
            .entities
            .get(&binding.name.name)
            .copied()
            .unwrap_or_default();
        entity.latest = index;
        if self.scopes[binding.scope].kind == ScopeKind::File {
            if binding.kind == BindingKind::Function {
                entity.non_inline |= !spec.function_specifiers.is_inline
                    || spec.storage_class == Some(StorageClass::Extern);
            } else if binding.kind == BindingKind::Object {
                if initialized {
                    self.define_entity(index, &mut entity);
                } else if spec.storage_class.is_none()
                    || spec.storage_class == Some(StorageClass::Static)
                {
                    entity.tentative = Some(index);
                    if binding.linkage == Linkage::Internal
                        && !self.types.unanalyzed(binding.ty)
                        && !self.complete_object(binding.ty)
                    {
                        self.error(
                            SemanticErrorKind::IncompleteInternalTentative,
                            binding.name.source_vectors,
                            Some(binding.name.name),
                            None,
                        );
                    }
                }
            }
        }
        _ = self.functions.entities.insert(binding.name.name, entity);
    }

    /// Inline object restrictions concern the object's own qualification,
    /// not aggregate assignment: a const member does not protect other members.
    /// C99: §6.7.4p3, p. 112; PDF p. 124; array qualification §6.7.3p8,
    /// p. 109; PDF p. 121.
    fn inline_object_modifiable(&self, mut ty: TypeId) -> bool {
        loop {
            if ty.qualifiers.contains(TypeQualifiers::CONST) {
                return false;
            }
            if let TypeKind::Array(element, _) = self.types.nodes[ty.index] {
                ty = element;
            } else {
                return true;
            }
        }
    }

    fn define_entity(&mut self, index: usize, entity: &mut Entity) {
        let b = self.bindings[index];
        if let Some(previous) = entity.definition {
            self.error(
                SemanticErrorKind::DuplicateDefinition,
                b.name.source_vectors,
                Some(b.name.name),
                Some(previous),
            );
        } else {
            entity.definition = Some(b.name.source_vectors);
        }
    }

    fn record_function_definition(&mut self, index: usize, f: &'tu FunctionDefinition<'tu>) {
        if self.tainted {
            return;
        }
        let b = self.bindings[index];
        if b.linkage == Linkage::None {
            self.definitions.push(Definition {
                binding: index,
                kind:    DefinitionKind::Function,
            });
            return;
        }
        let mut entity = self
            .functions
            .entities
            .get(&b.name.name)
            .copied()
            .unwrap_or_default();
        entity.latest = index;
        if let Some(previous) = entity.function_body {
            self.error(
                SemanticErrorKind::DuplicateDefinition,
                b.name.source_vectors,
                Some(b.name.name),
                Some(previous),
            );
        } else {
            entity.function_body = Some(b.name.source_vectors);
        }
        if b.linkage == Linkage::Internal
            || !f.declaration_specifiers.function_specifiers.is_inline
            || f.declaration_specifiers.storage_class == Some(StorageClass::Extern)
        {
            entity.definition = entity.function_body;
            entity.non_inline = true;
        }
        _ = self.functions.entities.insert(b.name.name, entity);
        self.definitions.push(Definition {
            binding: index,
            kind:    DefinitionKind::Function,
        });
    }

    pub(super) fn record_binding_use(&mut self, binding: usize, name: Identifier) {
        if self.tainted {
            return;
        }
        let inline_function = self
            .functions
            .current
            .filter(|f| {
                f.syntax
                    .declaration_specifiers
                    .function_specifiers
                    .is_inline
            })
            .and_then(|f| f.binding)
            .filter(|&i| self.bindings[i].linkage == Linkage::External);
        self.functions.uses.push(Use {
            binding,
            name,
            sizeof: self.functions.sizeof,
            inline_function,
        });
    }

    /// Constant sizeof operands do not require an internal definition.
    /// C99: §6.9p3, p. 140; PDF p. 152.
    pub(super) fn enter_sizeof(&mut self, expression: &'tu Expression<'tu>) {
        self.work
            .push(Work::FunctionWork(FunctionWork::RestoreSizeof(
                self.functions.sizeof,
            )));
        let index = self.functions.sizeof_contexts.len();
        self.functions.sizeof_contexts.push(SizeofContext {
            expression,
            parent: self.functions.sizeof,
        });
        self.functions.sizeof = Some(index);
    }

    /// Translation-unit completion is deterministic in declaration order.
    /// C99: §6.9p3,p5, p. 140; PDF p. 152; §6.9.2p2-3, p. 143;
    /// PDF p. 155; inline definitions §6.7.4p3,p6, p. 112; PDF p. 124.
    pub(super) fn finish_translation_unit(&mut self) {
        self.finish_labels();
        let mut entities = ArenaVec::new_in(self.scratch);
        entities.extend(self.functions.entities.values().copied());
        entities.sort_unstable_by_key(|entity| entity.latest);
        for mut entity in entities {
            let index = entity.latest;
            let b = self.bindings[index];
            if entity.definition.is_none()
                && entity.tentative.is_some()
                && !matches!(self.types.nodes[b.ty.index], TypeKind::Scalar(Scalar::Void))
            {
                let mut ty = b.ty;
                if let TypeKind::Array(element, ArrayBound::Incomplete) = self.types.nodes[ty.index]
                    && b.linkage == Linkage::External
                    && !self.types.unanalyzed(ty)
                {
                    self.error(
                        SemanticErrorKind::TentativeArrayAssumedOne,
                        b.name.source_vectors,
                        Some(b.name.name),
                        None,
                    );
                    ty = self
                        .types
                        .intern(TypeKind::Array(element, ArrayBound::Constant(1)))
                        .qualified(ty.qualifiers);
                    self.bindings[index].ty = ty;
                }
                if !self.types.unanalyzed(ty) && !self.complete_object(ty) {
                    if b.linkage != Linkage::Internal {
                        self.error(
                            SemanticErrorKind::IncompleteTentativeDefinition,
                            b.name.source_vectors,
                            Some(b.name.name),
                            None,
                        );
                    }
                } else {
                    entity.definition = Some(b.name.source_vectors);
                    self.definitions.push(Definition {
                        binding: index,
                        kind:    DefinitionKind::Tentative,
                    });
                }
                _ = self.functions.entities.insert(b.name.name, entity);
            }
        }
        let mut ignored = ArenaVec::new_in(self.scratch);
        for context in &self.functions.sizeof_contexts {
            let info = self.expression_info(context.expression);
            let constant = info.ice || self.types.unanalyzed(info.ty);
            ignored.push(constant || context.parent.is_some_and(|p| ignored[p]));
        }
        let mut used = ArenaMap::with_hasher_in(FxBuildHasher, self.scratch);
        for i in 0..self.functions.uses.len() {
            let usage = self.functions.uses[i];
            let b = self.bindings[usage.binding];
            if let Some(function) = usage.inline_function
                && b.linkage == Linkage::Internal
                && self.is_inline_definition(function)
            {
                self.error(
                    SemanticErrorKind::InlineInternalReference,
                    usage.name.source_vectors,
                    Some(usage.name.name),
                    Some(b.name.source_vectors),
                );
            }
            if b.linkage == Linkage::Internal && !usage.sizeof.is_some_and(|s| ignored[s]) {
                _ = used.entry(b.name.name).or_insert(usage.name);
            }
        }
        for i in 0..self.functions.inline_objects.len() {
            let object = self.functions.inline_objects[i];
            if self.is_inline_definition(object.function) {
                let b = self.bindings[object.binding];
                self.error(
                    SemanticErrorKind::InlineStaticObject,
                    b.name.source_vectors,
                    Some(b.name.name),
                    None,
                );
            }
        }
        let mut entities = ArenaVec::new_in(self.scratch);
        entities.extend(self.functions.entities.values().copied());
        entities.sort_unstable_by_key(|entity| entity.latest);
        for entity in entities {
            let b = self.bindings[entity.latest];
            if b.linkage == Linkage::Internal
                && entity.definition.is_none()
                && !self.types.unanalyzed(b.ty)
            {
                if let Some(&usage) = used.get(&b.name.name) {
                    self.error(
                        SemanticErrorKind::UndefinedInternal,
                        usage.source_vectors,
                        Some(b.name.name),
                        Some(b.name.source_vectors),
                    );
                } else if b.kind == BindingKind::Function {
                    self.error(
                        SemanticErrorKind::UnusedStaticFunction,
                        b.name.source_vectors,
                        Some(b.name.name),
                        None,
                    );
                }
            }
        }
        for i in 0..self.definitions.len() {
            let definition = self.definitions[i];
            if definition.kind == DefinitionKind::Function
                && self.bindings[definition.binding].linkage == Linkage::External
                && self.is_inline_definition(definition.binding)
            {
                self.definitions[i].kind = DefinitionKind::Inline;
            }
        }
        self.definitions.sort_unstable_by_key(|d| d.binding);
    }

    /// C99: §6.7.4p6, pp. 112-113; PDF pp. 124-125, unless the target's ABI
    /// emits inline functions as external definitions.
    fn is_inline_definition(&self, binding: usize) -> bool {
        self.context.configuration.target().c99_inline_definitions()
            && self
                .functions
                .entities
                .get(&self.bindings[binding].name.name)
                .is_some_and(|e| !e.non_inline)
    }
}
