//! Entity attributes: validation at each declaration with Clang's
//! applicability diagnostics, merging over the declarations of one entity,
//! and translation-unit completion of aliases and constructor/destructor
//! registrations. `cleanup` resolves its function here; its control region
//! opens after the variable's initializer.
//! GNU extension: GCC manual, "Common Function Attributes" and "Common
//! Variable Attributes".
//! <https://gcc.gnu.org/onlinedocs/gcc/Common-Function-Attributes.html>
//! <https://gcc.gnu.org/onlinedocs/gcc/Common-Variable-Attributes.html>
//! Clang extension: Clang Attributes Reference.
//! <https://clang.llvm.org/docs/AttributeReference.html>
//! C99: §6.2.2p2, p. 30; PDF p. 42 (linked declarations denote one entity);
//! §6.2.4p5, p. 32; PDF p. 44 (automatic lifetime ends with the block).

use rustc_hash::FxBuildHasher;

use super::{
    super::{
        Analyzer,
        AttributeSpecifier,
        BindingKind,
        Duration,
        Linkage,
        Namespace,
        SemanticErrorKind,
        facts::{
            AliasFacts,
            AttributeRefusal,
            BindingId,
            CleanupKind,
            EntityAttributes,
            EntityId,
            InitFunctionFacts,
            InitFunctionKind,
            RegionKind,
            SymbolFacts,
            UnsupportedAttribute,
            Visibility,
        },
        types::{
            Scalar,
            TypeKind,
        },
    },
    catalog::{
        Rule,
        rule,
    },
    written::WrittenAttribute,
};
use crate::{
    translation_phases::{
        SourceVectors,
        preprocessing::{
            StringTokenType,
            TokenType,
        },
    },
    util::{
        bump::{
            ArenaMap,
            ArenaVec,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

/// What a declaration's attributes apply to.
#[derive(Clone, Copy, Debug)]
pub(in crate::translation_phases::semantic_analysis) enum Subject {
    Function {
        binding:    usize,
        definition: bool,
    },
    Object {
        binding:    usize,
        definition: bool,
        /// The declarator's suffix-list identity, keying its cleanup.
        declarator: usize,
    },
    Typedef,
    Parameter,
    Member,
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Applies one attribute list to `subject`.
    pub(super) fn entity_attribute_list(
        &mut self,
        subject: Subject,
        specifier: &'tu AttributeSpecifier<'tu>,
    ) {
        for attribute in self.written_attributes(specifier) {
            let Some(rule) = rule(specifier.syntax, attribute.namespace, attribute.name) else {
                continue;
            };
            match rule {
                | Rule::Aligned =>
                    if let Subject::Function { binding, .. } = subject
                        && let Some(alignment) = self
                            .function_alignment(specifier, attribute.index)
                            .and_then(|alignment| u32::try_from(alignment).ok())
                    {
                        let entity = self.entity_of(binding);
                        let attributes = self.attributes.entity(entity);
                        attributes.code_alignment = attributes.code_alignment.max(alignment);
                    },
                | Rule::Weak => self.weak_attribute(subject, attribute),
                | Rule::Alias => self.alias_attribute(subject, attribute),
                | Rule::Section => self.section_attribute(subject, attribute),
                | Rule::Used => self.used_attribute(subject, attribute),
                | Rule::Visibility => self.visibility_attribute(subject, attribute),
                | Rule::Constructor | Rule::Destructor =>
                    self.init_attribute(subject, specifier, attribute, rule == Rule::Constructor),
                | Rule::Cleanup => self.cleanup_attribute(subject, attribute),
                | _ => {},
            }
        }
    }

    /// Opens a cleanup variable's control region once its declarator and
    /// initializer are complete, so no exit from the initializer calls it.
    /// GNU extension: GCC manual, "Common Variable Attributes", cleanup.
    pub(in crate::translation_phases::semantic_analysis) fn open_cleanup(
        &mut self,
        declarator: usize,
    ) {
        let Some((variable, function, source)) =
            self.attributes.pending_cleanups.remove(&declarator)
        else {
            return;
        };
        if self.functions.current.is_none() {
            return;
        }
        _ = self.control.open(
            self.scope,
            RegionKind::CleanupDeclaration(BindingId(variable)),
            CleanupKind::Call {
                variable: BindingId(variable),
                function: BindingId(function),
            },
            source,
        );
    }

    /// GCC `weak`: external entities only; Clang rejects internal and
    /// absent linkage. Parameters are ignored silently, as in Clang.
    fn weak_attribute(&mut self, subject: Subject, attribute: WrittenAttribute<'_>) {
        if !self.no_arguments(attribute) {
            return;
        }
        match subject {
            | Subject::Function { binding, .. } | Subject::Object { binding, .. } => {
                if self.bindings[binding].linkage == Linkage::External {
                    let entity = self.entity_of(binding);
                    self.attributes.entity(entity).weak = true;
                } else {
                    self.error(
                        SemanticErrorKind::WeakInternal,
                        attribute.source,
                        None,
                        None,
                    );
                }
            },
            | Subject::Parameter => {},
            | Subject::Typedef | Subject::Member => self.ignored(attribute),
        }
    }

    /// GCC `alias("target")` on a declaration that is not a definition;
    /// the target is resolved when the translation unit is complete.
    fn alias_attribute(&mut self, subject: Subject, attribute: WrittenAttribute<'_>) {
        let Some(target) = self.string_argument(attribute) else {
            return;
        };
        let (binding, definition) = match subject {
            | Subject::Function {
                binding,
                definition,
            }
            | Subject::Object {
                binding,
                definition,
                ..
            } if self.bindings[binding].linkage != Linkage::None => (binding, definition),
            | Subject::Object { .. } => {
                let name = self.context.string_cache.intern(attribute.name);
                self.error(
                    SemanticErrorKind::AttributeSubject,
                    attribute.source,
                    Some(name),
                    None,
                );
                return;
            },
            | _ => {
                self.ignored(attribute);
                return;
            },
        };
        if definition {
            self.error(
                SemanticErrorKind::AliasDefinition,
                attribute.source,
                Some(self.bindings[binding].name.name),
                None,
            );
            return;
        }
        let target = self.context.string_cache.intern(target);
        if self
            .attributes
            .alias_names
            .insert(self.bindings[binding].name.name, ())
            .is_none()
        {
            self.attributes.aliases.push(AliasRequest {
                binding,
                target,
                source: attribute.source,
            });
        }
    }

    /// GCC `section("name")` on functions and static-storage objects.
    fn section_attribute(&mut self, subject: Subject, attribute: WrittenAttribute<'_>) {
        let Some(section) = self.string_argument(attribute) else {
            return;
        };
        let Some(binding) = self.static_subject(subject, attribute, true) else {
            return;
        };
        if section.is_empty() || section.contains(['"', '\n', '\r', '\0']) {
            self.refuse(UnsupportedAttribute::SectionName, attribute.source);
            return;
        }
        let entity = self.entity_of(binding);
        let previous = self.attributes.sections.get(&entity).copied();
        let written = self.attributes.entity(entity).section;
        if let (Some(written), Some(previous)) = (written, previous)
            && written != section
        {
            self.error(
                SemanticErrorKind::SectionMismatch,
                attribute.source,
                None,
                Some(previous),
            );
        }
        self.attributes.entity(entity).section = Some(section);
        _ = self.attributes.sections.insert(entity, attribute.source);
    }

    /// GCC `used` on functions and static-storage objects.
    fn used_attribute(&mut self, subject: Subject, attribute: WrittenAttribute<'_>) {
        if !self.no_arguments(attribute) {
            return;
        }
        if let Some(binding) = self.static_subject(subject, attribute, false) {
            let entity = self.entity_of(binding);
            self.attributes.entity(entity).used = true;
        }
    }

    /// GCC `visibility("default" | "hidden" | "internal" | "protected")`;
    /// `internal` is hidden, as in Clang. An entity with internal linkage
    /// has no symbol visibility: Clang accepts the attribute there silently
    /// and emits the symbol with default visibility.
    fn visibility_attribute(&mut self, subject: Subject, attribute: WrittenAttribute<'_>) {
        let Some(value) = self.string_argument(attribute) else {
            return;
        };
        let visibility = match value {
            | "default" => Visibility::Default,
            | "hidden" | "internal" => Visibility::Hidden,
            | "protected" => Visibility::Protected,
            | _ => {
                let value = self.context.string_cache.intern(value);
                self.error(
                    SemanticErrorKind::InvalidVisibility,
                    attribute.source,
                    Some(value),
                    None,
                );
                return;
            },
        };
        match subject {
            | Subject::Function { binding, .. } | Subject::Object { binding, .. }
                if self.bindings[binding].linkage != Linkage::None =>
                if self.bindings[binding].linkage == Linkage::External {
                    let entity = self.entity_of(binding);
                    self.attributes.entity(entity).visibility = Some(visibility);
                },
            | _ => self.ignored(attribute),
        }
    }

    /// GCC `constructor`/`destructor` with an optional priority: an integer
    /// constant from 0 to 65535, where 0-100 are reserved (GCC's
    /// `-Wprio-ctor-dtor`; bundled Clang accepts any priority silently).
    fn init_attribute(
        &mut self,
        subject: Subject,
        specifier: &'tu AttributeSpecifier<'tu>,
        attribute: WrittenAttribute<'_>,
        constructor: bool,
    ) {
        let name = self.context.string_cache.intern(attribute.name);
        if attribute.argument_count(self.context) > 1 {
            self.error(
                SemanticErrorKind::AttributeArguments,
                attribute.source,
                Some(name),
                None,
            );
            return;
        }
        let priority = if attribute.argument_count(self.context) == 0 {
            65_535
        } else {
            let value = specifier
                .arguments
                .iter()
                .find(|argument| argument.index == attribute.index)
                .map(|argument| self.expression_info(argument.expression))
                .and_then(|info| info.integer.filter(|_| info.ice));
            let Some(value) = value.and_then(super::super::Integer::to_i128) else {
                self.error(
                    SemanticErrorKind::AttributeRequiresConstant,
                    attribute.source,
                    Some(name),
                    None,
                );
                return;
            };
            let Ok(priority) = u16::try_from(value) else {
                self.error(
                    SemanticErrorKind::InvalidPriority,
                    attribute.source,
                    Some(name),
                    None,
                );
                return;
            };
            if priority <= 100 {
                self.error(
                    SemanticErrorKind::ReservedPriority,
                    attribute.source,
                    Some(name),
                    None,
                );
            }
            priority
        };
        let Subject::Function { binding, .. } = subject else {
            self.ignored(attribute);
            return;
        };
        let entity = self.entity_of(binding);
        let entry = self
            .attributes
            .initializers
            .entry(entity)
            .or_insert((None, None));
        if constructor {
            entry.0 = Some(priority);
        } else {
            entry.1 = Some(priority);
        }
    }

    /// GCC `cleanup(function)` on an automatic variable: the function takes
    /// one parameter to which a pointer to the variable converts, ignoring
    /// discarded qualifiers as Clang does.
    fn cleanup_attribute(&mut self, subject: Subject, attribute: WrittenAttribute<'_>) {
        let Subject::Object {
            binding,
            declarator,
            ..
        } = subject
        else {
            self.ignored(attribute);
            return;
        };
        let variable = self.bindings[binding];
        if variable.duration != Duration::Automatic || variable.kind != BindingKind::Object {
            self.ignored(attribute);
            return;
        }
        let name = self.context.string_cache.intern(attribute.name);
        let arguments = attribute.arguments.unwrap_or_default();
        if attribute.argument_count(self.context) != 1 {
            self.error(
                SemanticErrorKind::AttributeArguments,
                attribute.source,
                Some(name),
                None,
            );
            return;
        }
        let identifier = match arguments {
            | [token] if matches!(token.kind, TokenType::Identifier) => *token,
            | _ => {
                self.error(
                    SemanticErrorKind::CleanupNotFunction,
                    arguments
                        .first()
                        .map_or(attribute.source, |t| t.source_vectors),
                    None,
                    None,
                );
                return;
            },
        };
        let Some(entry) = self.lookup(Namespace::Ordinary, identifier.contents) else {
            self.error(
                SemanticErrorKind::UndeclaredIdentifier,
                identifier.source_vectors,
                Some(identifier.contents),
                None,
            );
            return;
        };
        let function = self.bindings[entry.binding];
        if function.kind != BindingKind::Function {
            self.error(
                SemanticErrorKind::CleanupNotFunction,
                identifier.source_vectors,
                Some(identifier.contents),
                None,
            );
            return;
        }
        if self.types.unanalyzed(function.ty) || self.types.unanalyzed(variable.ty) {
            return;
        }
        let TypeKind::Function { parameters, .. } = self.types.nodes[function.ty.index] else {
            return;
        };
        let [parameter] = parameters else {
            self.error(
                SemanticErrorKind::CleanupParameterCount,
                identifier.source_vectors,
                Some(identifier.contents),
                None,
            );
            return;
        };
        let parameter = self.types.non_atomic(*parameter);
        let compatible = match self.types.nodes[parameter.index] {
            | TypeKind::Pointer(target) =>
                matches!(
                    self.types.nodes[target.index],
                    TypeKind::Scalar(Scalar::Void)
                ) || self
                    .types
                    .composite(target.unqualified(), variable.ty.unqualified())
                    .is_some(),
            | _ => false,
        };
        if !compatible {
            self.error(
                SemanticErrorKind::CleanupParameterType,
                identifier.source_vectors,
                Some(identifier.contents),
                None,
            );
            return;
        }
        _ = self.attributes.pending_cleanups.insert(
            declarator,
            (binding, entry.binding, variable.name.source_vectors),
        );
    }

    /// The binding of a function or static-storage object; an automatic
    /// object is an error for `section` and a warning for `used`.
    fn static_subject(
        &mut self,
        subject: Subject,
        attribute: WrittenAttribute<'_>,
        automatic_error: bool,
    ) -> Option<usize> {
        match subject {
            | Subject::Function { binding, .. } => Some(binding),
            | Subject::Object { binding, .. }
                if self.bindings[binding].duration == Duration::Static =>
                Some(binding),
            | Subject::Object { .. } if automatic_error => {
                let name = self.context.string_cache.intern(attribute.name);
                self.error(
                    SemanticErrorKind::AttributeSubject,
                    attribute.source,
                    Some(name),
                    None,
                );
                None
            },
            | _ => {
                self.ignored(attribute);
                None
            },
        }
    }

    fn ignored(&mut self, attribute: WrittenAttribute<'_>) {
        let name = self.context.string_cache.intern(attribute.name);
        self.error(
            SemanticErrorKind::IgnoredAttribute,
            attribute.source,
            Some(name),
            None,
        );
    }

    fn no_arguments(&mut self, attribute: WrittenAttribute<'_>) -> bool {
        if attribute.arguments.is_none() {
            return true;
        }
        let name = self.context.string_cache.intern(attribute.name);
        self.error(
            SemanticErrorKind::AttributeArguments,
            attribute.source,
            Some(name),
            None,
        );
        false
    }

    /// The text of a single narrow string-literal argument, or an error.
    fn string_argument(&mut self, attribute: WrittenAttribute<'_>) -> Option<&'tu str> {
        let arguments = attribute.arguments.unwrap_or_default();
        let mut text: Option<&str> = None;
        let mut valid = !arguments.is_empty();
        for token in arguments {
            match token.kind {
                | TokenType::String(StringTokenType::String(id)) => {
                    let Some(part) = self.context.literal_text_in(self.scratch, id, false) else {
                        valid = false;
                        break;
                    };
                    text = Some(match text {
                        | None => part,
                        | Some(previous) =>
                            crate::diagnostics::format_in!(self.scratch, "{previous}{part}"),
                    });
                },
                | _ => {
                    valid = false;
                    break;
                },
            }
        }
        match text.filter(|_| valid) {
            | Some(text) => Some(self.types.tu.alloc_str(text)),
            | None => {
                let name = self.context.string_cache.intern(attribute.name);
                self.error(
                    SemanticErrorKind::AttributeArguments,
                    attribute.source,
                    Some(name),
                    None,
                );
                None
            },
        }
    }

    /// GCC `aligned` on a function: its requested code alignment, or the
    /// default maximum alignment without an operand.
    fn function_alignment(&self, specifier: &AttributeSpecifier<'tu>, index: usize) -> Option<u64> {
        let has_operand = specifier
            .tokens
            .get(index + 1)
            .is_some_and(|token| self.context.string_cache.at(token.contents) == "(");
        let value = if has_operand {
            self.attribute_alignment(specifier, index)?
        } else {
            16
        };
        (value.is_power_of_two() && value <= 1 << 31).then_some(value)
    }

    /// Whether a declaration of the linked name `name` has an `alias`
    /// attribute; its object takes storage from the target, so no tentative
    /// definition completes it. GNU extension: GCC manual, "Common Variable
    /// Attributes", alias. C99: §6.9.2p2, p. 143; PDF p. 155.
    pub(in crate::translation_phases::semantic_analysis) fn declares_alias(
        &self,
        name: StringCacheId,
    ) -> bool {
        self.attributes.alias_names.contains_key(&name)
    }

    pub(in crate::translation_phases::semantic_analysis) fn entity_of(
        &mut self,
        binding: usize,
    ) -> EntityId {
        self.correspondence.entity(binding, self.bindings[binding])
    }

    /// Resolves aliases to final definitions and registers constructor and
    /// destructor definitions in definition order, after every declaration
    /// and definition of the translation unit is known.
    /// GNU extension: GCC manual, "Common Function Attributes", alias.
    pub(in crate::translation_phases::semantic_analysis) fn finish_attributes(
        &mut self,
    ) -> SymbolFacts<'tu> {
        let tu = self.types.tu;
        let mut aliases = ArenaVec::new_in(tu);
        let mut by_name = ArenaMap::with_hasher_in(FxBuildHasher, self.scratch);
        for request in &self.attributes.aliases {
            _ = by_name.insert(self.bindings[request.binding].name.name, request.target);
        }
        for index in 0..self.attributes.aliases.len() {
            let request = self.attributes.aliases[index];
            let alias = self.bindings[request.binding];
            if self
                .linked_entity(alias.name.name)
                .is_some_and(|(_, defined)| defined)
            {
                self.error(
                    SemanticErrorKind::AliasDefinition,
                    request.source,
                    Some(alias.name.name),
                    None,
                );
                continue;
            }
            // Follow alias chains to the definition; a chain longer than
            // the number of aliases revisits one of them.
            let mut name = request.target;
            let mut steps = 0;
            let resolved = loop {
                if let Some(&next) = by_name.get(&name) {
                    steps += 1;
                    if steps > self.attributes.aliases.len() {
                        break Err(SemanticErrorKind::AliasCycle);
                    }
                    name = next;
                    continue;
                }
                match self.linked_entity(name) {
                    | Some((target, true)) => break Ok(target),
                    | _ => break Err(SemanticErrorKind::AliasTarget),
                }
            };
            let target = match resolved {
                | Ok(target) => target,
                | Err(kind) => {
                    self.error(kind, request.source, None, None);
                    continue;
                },
            };
            let definition = self.bindings[target];
            let function = alias.kind == BindingKind::Function;
            if function != (definition.kind == BindingKind::Function) {
                self.error(
                    SemanticErrorKind::AliasKind,
                    request.source,
                    None,
                    Some(definition.name.source_vectors),
                );
                continue;
            }
            if function
                && !self.types.unanalyzed(alias.ty)
                && !self.types.unanalyzed(definition.ty)
                && self.types.composite(alias.ty, definition.ty).is_none()
            {
                self.error(
                    SemanticErrorKind::AliasTypeMismatch,
                    request.source,
                    None,
                    Some(definition.name.source_vectors),
                );
            }
            aliases.push(AliasFacts {
                binding: request.binding,
                target,
                source: request.source,
            });
        }
        let mut init_functions = ArenaVec::new_in(tu);
        for index in 0..self.definitions.len() {
            let definition = self.definitions[index];
            if definition.kind != super::super::DefinitionKind::Function {
                continue;
            }
            let entity = self.entity_of(definition.binding);
            let Some(&(constructor, destructor)) = self.attributes.initializers.get(&entity) else {
                continue;
            };
            let binding = self.bindings[definition.binding];
            if let TypeKind::Function {
                parameters,
                variadic,
                ..
            } = self.types.nodes[binding.ty.index]
                && (!parameters.is_empty() || variadic)
            {
                self.refuse(
                    UnsupportedAttribute::InitFunctionParameters,
                    binding.name.source_vectors,
                );
            }
            for (priority, kind) in [
                (constructor, InitFunctionKind::Constructor),
                (destructor, InitFunctionKind::Destructor),
            ] {
                if let Some(priority) = priority {
                    init_functions.push(InitFunctionFacts {
                        kind,
                        priority,
                        binding: definition.binding,
                    });
                }
            }
        }
        let mut entities = ArenaMap::with_hasher_in(FxBuildHasher, tu);
        entities.extend(self.attributes.entities.iter().map(|(&k, &v)| (k, v)));
        SymbolFacts {
            entities,
            init_functions: init_functions.leak(),
            aliases: aliases.leak(),
            refusals: tu.alloc_slice_copy(&self.attributes.refusals),
        }
    }
}

/// Attribute work in semantic scratch.
pub(in crate::translation_phases::semantic_analysis) struct AttributeState<'tu, 's> {
    /// Specifier lists already checked by name, by address.
    pub(in crate::translation_phases::semantic_analysis) checked: ArenaMap<'s, usize, ()>,
    pub(in crate::translation_phases::semantic_analysis) refusals: ArenaVec<'s, AttributeRefusal>,
    entities: ArenaMap<'s, EntityId, EntityAttributes<'tu>>,
    sections: ArenaMap<'s, EntityId, SourceVectors>,
    /// Constructor and destructor priorities requested for an entity.
    initializers: ArenaMap<'s, EntityId, (Option<u16>, Option<u16>)>,
    aliases: ArenaVec<'s, AliasRequest>,
    /// The linked names with an alias request, one request each.
    alias_names: ArenaMap<'s, StringCacheId, ()>,
    /// Validated cleanups by declarator identity, waiting for the end of
    /// their declarator's initializer.
    pending_cleanups: ArenaMap<'s, usize, (usize, usize, SourceVectors)>,
    /// Indirect gotos and address-taken labels, checked against cleanup
    /// scopes when the translation unit is complete.
    pub(in crate::translation_phases::semantic_analysis) indirect_jumps:
        ArenaVec<'s, (super::super::facts::ControlPoint, SourceVectors)>,
    pub(in crate::translation_phases::semantic_analysis) address_labels:
        ArenaVec<'s, super::super::facts::LabelId>,
}

impl<'tu, 's> AttributeState<'tu, 's> {
    pub(in crate::translation_phases::semantic_analysis) fn new(scratch: &'s Bump) -> Self {
        Self {
            checked:          ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            refusals:         ArenaVec::new_in(scratch),
            entities:         ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            sections:         ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            initializers:     ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            aliases:          ArenaVec::new_in(scratch),
            alias_names:      ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            pending_cleanups: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            indirect_jumps:   ArenaVec::new_in(scratch),
            address_labels:   ArenaVec::new_in(scratch),
        }
    }

    fn entity(&mut self, entity: EntityId) -> &mut EntityAttributes<'tu> {
        self.entities.entry(entity).or_default()
    }
}

/// An alias declaration and the name of its target.
#[derive(Clone, Copy)]
struct AliasRequest {
    binding: usize,
    target:  StringCacheId,
    source:  SourceVectors,
}
