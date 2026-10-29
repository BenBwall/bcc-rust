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
        ObjectSize,
        PredicateKind,
        ScopeKind,
        SemanticErrorKind,
        facts::{
            AliasFacts,
            AttributeRefusal,
            BindingId,
            CleanupKind,
            DllStorage,
            EntityAttributes,
            EntityId,
            InitFunctionFacts,
            InitFunctionKind,
            Inlining,
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
    dll::DllState,
    written::WrittenAttribute,
};
use crate::{
    translation_phases::{
        SourceVectors,
        preprocessing::{
            LiteralEncoding,
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

impl<'tu, 's> Analyzer<'_, 'tu, 's> {
    /// Applies one attribute list to `subject`. A redeclaration of a
    /// defined entity cannot add attributes: as in Clang, each attribute
    /// not already written before the definition is ignored with a warning.
    pub(super) fn entity_attribute_list(
        &mut self,
        subject: Subject,
        specifier: &'tu AttributeSpecifier<'tu>,
    ) {
        let linked = self.attribute_entity(subject);
        for attribute in self.written_attributes(specifier) {
            let Some(rule) = rule(
                specifier.syntax,
                attribute.namespace,
                attribute.name,
                self.context.configuration.target(),
            ) else {
                continue;
            };
            if follows_definition(rule, attribute.name)
                && self.ignored_after_definition(linked, attribute.name, attribute.source)
            {
                continue;
            }
            match rule {
                | Rule::CallDiagnostic => self.apply_call_diagnostic(subject, specifier, attribute),
                | Rule::Aligned => match subject {
                    | Subject::Function { binding, .. } => {
                        if let Some(alignment) = self
                            .function_alignment(specifier, attribute.index)
                            .and_then(|alignment| u32::try_from(alignment).ok())
                        {
                            let entity = self.entity_of(binding);
                            let attributes = self.attributes.entity(entity);
                            attributes.code_alignment = attributes.code_alignment.max(alignment);
                        }
                    },
                    | Subject::Object { binding, .. } => self.declares_alignment(binding),
                    | _ => {},
                },
                | Rule::Weak => self.weak_attribute(subject, attribute),
                | Rule::Alias => self.alias_attribute(subject, attribute),
                | Rule::Section => self.section_attribute(subject, attribute),
                | Rule::Used => self.used_attribute(subject, attribute),
                | Rule::Visibility => self.visibility_attribute(subject, attribute),
                | Rule::Constructor | Rule::Destructor =>
                    self.init_attribute(subject, specifier, attribute, rule == Rule::Constructor),
                | Rule::Cleanup => self.cleanup_attribute(subject, attribute),
                | Rule::ReturnsTwice => self.returns_twice_attribute(subject, attribute),
                | Rule::Nonnull => self.nonnull_attribute(subject, specifier, attribute),
                // On a typedef it names the type, which type construction
                // checks; anything else declared is not a union type.
                | Rule::TransparentUnion if !matches!(subject, Subject::Typedef) => self.error(
                    SemanticErrorKind::TransparentUnionNotUnion,
                    attribute.source,
                    None,
                    None,
                ),
                | Rule::SelectAny => self.select_any_attribute(subject, attribute),
                | Rule::Common(common) => self.common_attribute(subject, attribute, common),
                | Rule::DllImport => self.dll_attribute(subject, attribute, DllStorage::Import),
                | Rule::DllExport => self.dll_attribute(subject, attribute, DllStorage::Export),
                | Rule::NoInline => self.inline_attribute(subject, attribute, Inlining::Never),
                | Rule::AlwaysInline => self.inline_attribute(subject, attribute, Inlining::Always),
                | _ => {},
            }
        }
    }

    /// MSVC `__forceinline` on a function: the parser already records it as
    /// the `inline` function specifier, so the definition takes the inline
    /// linkage of the target (C99 inline definitions on GNU targets, the
    /// Microsoft ABI's COMDAT linkage on MSVC); this adds the
    /// `always_inline` request, as Clang's `AlwaysInlineAttr` spelled
    /// `__forceinline` does. Like that attribute it is ignored with a
    /// warning on a redeclaration after the definition, `noinline` still
    /// prevails, and on anything but a function it adds nothing to the
    /// diagnostic that `inline` there already gets.
    /// C99: §6.7.4 paragraphs 5-7, pp. 112-113; PDF pp. 124-125.
    /// MSVC extension: Microsoft Learn, "inline, __inline, __forceinline".
    /// <https://learn.microsoft.com/en-us/cpp/cpp/inline-functions-cpp>
    pub(super) fn forceinline(&mut self, subject: Subject, source: SourceVectors) {
        let Subject::Function { binding, .. } = subject else {
            return;
        };
        let linked = self.attribute_entity(subject);
        if self.ignored_after_definition(linked, "always_inline", source) {
            return;
        }
        let entity = self.entity_of(binding);
        let attributes = self.attributes.entity(entity);
        if attributes.inlining != Inlining::Never {
            attributes.inlining = Inlining::Always;
        }
    }

    /// The linked entity a function or object declaration names, with the
    /// source of its definition when an earlier declaration supplied one.
    fn attribute_entity(&mut self, subject: Subject) -> Option<(EntityId, Option<SourceVectors>)> {
        match subject {
            | Subject::Function { binding, .. } | Subject::Object { binding, .. }
                if self.bindings[binding].linkage != Linkage::None =>
                Some((self.entity_of(binding), self.earlier_definition(binding))),
            | _ => None,
        }
    }

    /// Whether an attribute named `name` (underscores trimmed) is ignored,
    /// with Clang's warning, because a redeclaration adds it after the
    /// definition; before the definition it is recorded as written there.
    fn ignored_after_definition(
        &mut self,
        linked: Option<(EntityId, Option<SourceVectors>)>,
        name: &str,
        source: SourceVectors,
    ) -> bool {
        let Some((entity, definition)) = linked else {
            return false;
        };
        let name = self.context.string_cache.intern(name);
        match definition {
            | Some(definition)
                if !self
                    .attributes
                    .before_definition
                    .contains_key(&(entity, name)) =>
            {
                self.error(
                    SemanticErrorKind::AttributeAfterDefinition,
                    source,
                    None,
                    Some(definition),
                );
                true
            },
            | Some(_) => false,
            | None => {
                _ = self.attributes.before_definition.insert((entity, name), ());
                false
            },
        }
    }

    /// Opens a cleanup variable's control region once its declarator and
    /// initializer are complete, so no exit from the initializer calls it.
    /// GNU extension: GCC manual, "Common Variable Attributes", cleanup.
    pub(in crate::translation_phases::semantic_analysis) fn open_cleanup(
        &mut self,
        declarator: usize,
    ) {
        let Some((variable, function, source, reference)) =
            self.attributes.pending_cleanups.remove(&declarator)
        else {
            return;
        };
        if self.functions.current.is_none() {
            return;
        }
        // The variable's scope calls the function, so the enclosing body
        // uses it where the attribute names it. C99: §6.9p3, p. 140; PDF p.
        // 152.
        let before = self.reference_count();
        self.record_binding_use(function, reference);
        let omitted = self.functions.dead || self.functions.elided;
        if omitted {
            self.elide_references(before..self.reference_count());
        }
        _ = self.control.open(
            self.scope,
            RegionKind::CleanupDeclaration(BindingId(variable)),
            if omitted {
                CleanupKind::None
            } else {
                CleanupKind::Call {
                    variable: BindingId(variable),
                    function: BindingId(function),
                }
            },
            source,
        );
    }

    /// Whether a declaration so far gives the entity of `binding` GNU
    /// `weak`, so that its address may be null.
    /// GNU extension: GCC manual, "Common Function Attributes", `weak`.
    pub(in crate::translation_phases::semantic_analysis) fn declared_weak(
        &mut self,
        binding: usize,
    ) -> bool {
        let entity = self.entity_of(binding);
        self.attributes
            .entities
            .get(&entity)
            .is_some_and(|attributes| attributes.weak)
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

    /// GCC `returns_twice` on a function: the entity's retained
    /// returns-twice fact, which lowering projects onto the IR function so
    /// the optimizer and back ends keep the caller's frame valid for a
    /// second return. Elsewhere it is ignored with a warning, as in Clang.
    /// GNU extension: GCC manual, "Common Function Attributes",
    /// `returns_twice`.
    fn returns_twice_attribute(&mut self, subject: Subject, attribute: WrittenAttribute<'_>) {
        if !self.no_arguments(attribute) {
            return;
        }
        if let Subject::Function { binding, .. } = subject {
            let entity = self.entity_of(binding);
            self.attributes.entity(entity).returns_twice = true;
        } else {
            self.ignored(attribute);
        }
    }

    /// Microsoft `selectany` on an object with external linkage; on a
    /// function or an object without external linkage it is Clang's error.
    /// A typedef keeps it silently, as in Clang.
    /// MSVC extension: Microsoft Learn, "selectany".
    /// <https://learn.microsoft.com/en-us/cpp/cpp/selectany>
    fn select_any_attribute(&mut self, subject: Subject, attribute: WrittenAttribute<'_>) {
        if !self.no_arguments(attribute) {
            return;
        }
        match subject {
            | Subject::Object {
                binding,
                definition,
                ..
            } if self.bindings[binding].linkage == Linkage::External => {
                let entity = self.entity_of(binding);
                self.attributes.entity(entity).select_any = true;
                // An `extern` declaration without an initializer defines it.
                if !definition {
                    self.select_any_definition(binding);
                }
            },
            | Subject::Typedef => {},
            | _ => self.error(
                SemanticErrorKind::SelectAnySubject,
                attribute.source,
                None,
                None,
            ),
        }
    }

    /// GCC `common` and `nocommon` on an object with external linkage decide
    /// whether its tentative definition is a common symbol, which lowering
    /// reads: `common` on any of its declarations wins, silently, as in
    /// Clang (GCC instead warns and keeps the attribute it met first).
    /// Elsewhere they are ignored with a warning.
    /// GNU extension: GCC manual, "Common Variable Attributes".
    fn common_attribute(
        &mut self,
        subject: Subject,
        attribute: WrittenAttribute<'_>,
        common: bool,
    ) {
        if !self.no_arguments(attribute) {
            return;
        }
        match subject {
            | Subject::Object { binding, .. }
                if self.bindings[binding].linkage == Linkage::External =>
            {
                let entity = self.entity_of(binding);
                let attributes = self.attributes.entity(entity);
                attributes.common = Some(common || attributes.common == Some(true));
            },
            | _ => self.ignored(attribute),
        }
    }

    /// GCC `noinline`, `noipa` and `always_inline` on a function; `never`
    /// prevails over `always`. Elsewhere they are ignored with a warning,
    /// as in Clang.
    /// GNU extension: GCC manual, "Common Function Attributes".
    fn inline_attribute(
        &mut self,
        subject: Subject,
        attribute: WrittenAttribute<'_>,
        inlining: Inlining,
    ) {
        if !self.no_arguments(attribute) {
            return;
        }
        let Subject::Function { binding, .. } = subject else {
            self.ignored(attribute);
            return;
        };
        let entity = self.entity_of(binding);
        let attributes = self.attributes.entity(entity);
        if attributes.inlining != Inlining::Never {
            attributes.inlining = inlining;
        }
    }

    /// Retains GNU `nonnull` parameter indices after checking each integer
    /// constant operand. The annotation affects diagnostics at later calls.
    /// GNU extension: GCC Common Function Attributes, nonnull.
    /// C99: §6.6p6, p. 95; PDF p. 107 (integer constant operands).
    fn nonnull_attribute(
        &mut self,
        subject: Subject,
        specifier: &'tu AttributeSpecifier<'tu>,
        attribute: WrittenAttribute<'_>,
    ) {
        use crate::translation_phases::parsing::syntax::{
            BinaryOperator as B,
            ExpressionType as E,
        };
        let Subject::Function { binding, .. } = subject else {
            self.ignored(attribute);
            return;
        };
        let TypeKind::Function {
            parameters,
            variadic,
            ..
        } = self.types.nodes[self.bindings[binding].ty.index]
        else {
            return;
        };
        let entity = self.entity_of(binding);
        if attribute.arguments.is_none_or(<[_]>::is_empty) {
            self.attributes.entity(entity).nonnull = Some(&[]);
            return;
        }
        let retained = self.attributes.entity(entity).nonnull;
        let mut positions = ArenaVec::new_in(self.scratch);
        positions.extend_from_slice(retained.unwrap_or_default());
        let mut work: ArenaVec<
            '_,
            &'tu crate::translation_phases::parsing::syntax::Expression<'tu>,
        > = ArenaVec::new_in(self.scratch);
        work.extend(
            specifier
                .arguments
                .iter()
                .filter(|argument| argument.index == attribute.index)
                .map(|argument| argument.expression),
        );
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
                .and_then(super::super::Integer::to_i128)
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
                continue;
            };
            if parameters
                .get(position)
                .is_some_and(|&parameter| self.pointer_target(parameter).is_none())
            {
                self.ignored(attribute);
                continue;
            }
            if !positions.contains(&position) {
                positions.push(position);
            }
        }
        if retained.is_none_or(|positions| !positions.is_empty()) && !positions.is_empty() {
            self.attributes.entity(entity).nonnull =
                Some(self.types.tu.alloc_slice_copy(&positions));
        }
    }

    /// Emits `-Wnonnull` only for calls whose visible declaration carries
    /// the annotation and whose argument is known to be a null pointer.
    /// GNU extension: GCC Common Function Attributes, nonnull.
    pub(in crate::translation_phases::semantic_analysis) fn check_nonnull_arguments(
        &mut self,
        binding: Option<usize>,
        arguments: crate::util::arena_list::ArenaList<
            'tu,
            &'tu crate::translation_phases::parsing::syntax::Expression<'tu>,
        >,
    ) {
        let Some(binding) = binding else {
            return;
        };
        let entity = self.entity_of(binding);
        let attributes = *self.attributes.entity(entity);
        let Some(positions) = attributes.nonnull else {
            return;
        };
        let parameters = match self.types.nodes[self.bindings[binding].ty.index] {
            | TypeKind::Function { parameters, .. } => parameters,
            | _ => &[],
        };
        for (index, &argument) in arguments.iter().enumerate() {
            let info = self.expression_info(argument);
            if ((positions.is_empty()
                && parameters
                    .get(index)
                    .is_some_and(|&parameter| self.pointer_target(parameter).is_some()))
                || positions.contains(&index))
                && self.null_pointer_constant(info)
            {
                self.pointer_diagnostic(
                    crate::configuration::DiagnosticGroup::Nonnull,
                    argument.source_vectors,
                );
            }
        }
    }

    /// Gives every linked function that the external-symbol contract says
    /// may return twice the same retained fact as GNU `returns_twice`:
    /// glibc declares `_setjmp` and `__sigsetjmp` without the attribute, and
    /// GCC and Clang recognize these names alone. Only the external
    /// spelling counts; an internal or local function of that name is
    /// another entity. C99: §7.13 paragraph 3, p. 243; PDF p. 255 (a program
    /// may not define `setjmp` itself); §6.2.2 paragraph 4, pp. 30-31; PDF
    /// pp. 42-43.
    fn external_returns_twice(&mut self) {
        for binding in 0..self.bindings.len() {
            let data = self.bindings[binding];
            if data.kind == BindingKind::Function
                && data.linkage == Linkage::External
                && crate::builtins::returns_twice(self.context.string_cache.at(data.name.name))
            {
                let entity = self.entity_of(binding);
                self.attributes.entity(entity).returns_twice = true;
            }
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
            // An alias names a symbol: Clang rejects it on anything else.
            | _ => {
                let name = self.context.string_cache.intern(attribute.name);
                self.error(
                    SemanticErrorKind::AttributeSubject,
                    attribute.source,
                    Some(name),
                    None,
                );
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
        let identity = self.function_identity(binding);
        if let Some(&previous) = self.attributes.alias_names.get(&identity) {
            if previous != binding {
                self.error(
                    SemanticErrorKind::DuplicateDefinition,
                    self.bindings[binding].name.source_vectors,
                    Some(self.bindings[binding].name.name),
                    Some(self.bindings[previous].name.source_vectors),
                );
            }
            return;
        }
        _ = self.attributes.alias_names.insert(identity, binding);
        let target = self.context.string_cache.intern(target);
        self.attributes.aliases.push(AliasRequest {
            binding,
            target,
            source: attribute.source,
            // Block aliases are semantic definitions for redeclaration
            // diagnostics, but Clang never emits or resolves their targets.
            emitted: self.scopes[self.scope].kind == ScopeKind::File,
        });
    }

    /// GCC `section("name")` on functions and static-storage objects.
    fn section_attribute(&mut self, subject: Subject, attribute: WrittenAttribute<'_>) {
        let Some(section) = self.string_argument(attribute) else {
            return;
        };
        let Some(binding) = self.static_subject(subject, attribute, true) else {
            return;
        };
        if !crate::ir::valid_section_name(section) {
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
            // Clang ignores visibility silently on members, parameters and
            // local variables, and with a warning on typedefs.
            | Subject::Typedef => self.ignored(attribute),
            | _ => {},
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
            let info = specifier
                .arguments
                .iter()
                .find(|argument| argument.index == attribute.index)
                .map(|argument| self.expression_info(argument.expression));
            // An operand that is already in error, such as an undeclared
            // identifier, has its diagnostic.
            if info.is_some_and(|info| self.types.unanalyzed(info.ty)) {
                return;
            }
            let value = info.and_then(|info| info.integer.filter(|_| info.ice));
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
        // Clang cleanup requires one function, rather than resolving the
        // overload set as an ordinary source call would. Redeclarations of
        // the same mangled identity remain a single function.
        let identity = self.function_identity(entry.binding);
        let mut candidate = self.overload_previous.get(&entry.binding).copied();
        while let Some(binding) = candidate {
            let overloaded = self
                .function_extensions
                .function(entry.binding)
                .is_some_and(|contract| contract.overloadable)
                || self
                    .function_extensions
                    .function(binding)
                    .is_some_and(|contract| contract.overloadable);
            if overloaded && self.function_identity(binding) != identity {
                self.error(
                    SemanticErrorKind::CleanupOverloadSet,
                    identifier.source_vectors,
                    Some(identifier.contents),
                    Some(self.bindings[binding].name.source_vectors),
                );
                return;
            }
            candidate = self.overload_previous.get(&binding).copied();
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
        if let Some(contract) = self.function_extensions.function(entry.binding)
            && let Some(size) = contract.sizes.first()
        {
            // The synthetic actual is &variable, so every mode bounds its
            // entire object. VLA bounds use the retained runtime size DAG.
            // C99: §6.5.3.4p2, p. 80; PDF p. 92.
            let value = if let Some(layout) = self.types.layout(variable.ty) {
                ObjectSize::Constant(layout.size)
            } else if size.dynamic && self.variably_modified(variable.ty) {
                ObjectSize::Dynamic {
                    ty:     variable.ty,
                    offset: 0,
                }
            } else {
                ObjectSize::Constant(if size.mode >= 2 { 0 } else { u64::MAX })
            };
            _ = self
                .function_extensions
                .cleanup_sizes
                .insert(binding, value);
        }
        if let Some(contract) = self.function_extensions.function(entry.binding) {
            if let Some(message) = contract.unavailable {
                self.call_attribute_diagnostic(identifier.source_vectors, message, false);
            }
            // Cleanup bypasses enable_if candidate filtering, but diagnoses
            // the call with its actual nonnull automatic-object address.
            for predicate in contract.predicates {
                if predicate.kind != PredicateKind::Enable
                    && self
                        .cleanup_predicate_value(
                            predicate.parameters,
                            predicate.expression,
                            binding,
                        )
                        .is_some_and(|value| !value.is_zero())
                {
                    self.call_attribute_diagnostic(
                        identifier.source_vectors,
                        predicate.message,
                        predicate.kind == PredicateKind::Warning,
                    );
                }
            }
        }
        _ = self.attributes.pending_cleanups.insert(
            declarator,
            (
                binding,
                entry.binding,
                variable.name.source_vectors,
                crate::translation_phases::parsing::syntax::Identifier {
                    name:           identifier.contents,
                    source_vectors: identifier.source_vectors,
                },
            ),
        );
    }

    /// The binding of a function or static- or thread-storage object (Clang
    /// places a thread-local object in a named section too). Anything else
    /// is an error for `section`, which names a symbol's section, and a
    /// warning for `used`.
    fn static_subject(
        &mut self,
        subject: Subject,
        attribute: WrittenAttribute<'_>,
        symbol_error: bool,
    ) -> Option<usize> {
        match subject {
            | Subject::Function { binding, .. } => Some(binding),
            | Subject::Object { binding, .. }
                if matches!(
                    self.bindings[binding].duration,
                    Duration::Static | Duration::Thread
                ) =>
                Some(binding),
            | _ if symbol_error => {
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

    pub(super) fn ignored(&mut self, attribute: WrittenAttribute<'_>) {
        let name = self.context.string_cache.intern(attribute.name);
        self.error(
            SemanticErrorKind::IgnoredAttribute,
            attribute.source,
            Some(name),
            None,
        );
    }

    pub(super) fn no_arguments(&mut self, attribute: WrittenAttribute<'_>) -> bool {
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

    /// The text of a single string-literal argument, or an error. An
    /// encoding prefix has no effect on the text, with Clang's warning.
    pub(super) fn string_argument(&mut self, attribute: WrittenAttribute<'_>) -> Option<&'tu str> {
        let arguments = attribute.arguments.unwrap_or_default();
        let mut text: Option<&str> = None;
        let mut valid = !arguments.is_empty();
        for token in arguments {
            let (id, wide) = match token.kind {
                | TokenType::String(StringTokenType::String(id)) => (id, false),
                | TokenType::String(StringTokenType::WideString(id)) => (id, true),
                | TokenType::String(StringTokenType::EncodedString(id, encoding)) =>
                    (id, encoding != LiteralEncoding::Utf8),
                | _ => {
                    valid = false;
                    break;
                },
            };
            if !matches!(token.kind, TokenType::String(StringTokenType::String(_))) {
                self.error(
                    SemanticErrorKind::UnevaluatedStringPrefix,
                    token.source_vectors,
                    None,
                    None,
                );
            }
            let Some(part) = self.context.literal_text_in(self.scratch, id, wide) else {
                valid = false;
                break;
            };
            text = Some(match text {
                | None => part,
                | Some(previous) =>
                    crate::diagnostics::format_in!(self.scratch, "{previous}{part}"),
            });
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

    /// A declaration of object `binding` specifies its alignment.
    pub(super) fn declares_alignment(&mut self, binding: usize) {
        let entity = self.entity_of(binding);
        self.attributes.entity(entity).declared_alignment = true;
    }

    /// Registers constructors and destructors in definition order and retains
    /// entity attributes. Alias targets are resolved after emitted ABI names.
    /// GNU extension: GCC manual, "Common Function Attributes", alias.
    pub(in crate::translation_phases::semantic_analysis) fn finish_attributes(
        &mut self,
    ) -> SymbolFacts<'tu> {
        let tu = self.types.tu;
        self.finish_call_diagnostics();
        self.external_returns_twice();
        let mut init_functions = ArenaVec::new_in(tu);
        for index in 0..self.definitions.len() {
            let definition = self.definitions[index];
            if !matches!(
                definition.kind,
                super::super::DefinitionKind::Function
                    | super::super::DefinitionKind::DiscardableFunction
            ) {
                continue;
            }
            let entity = self.entity_of(definition.binding);
            let attributes = self
                .attributes
                .entities
                .get(&entity)
                .copied()
                .unwrap_or_default();
            // Diagnose emitted definitions after redeclarations have supplied
            // their final section. Unused static and inline-only bodies may
            // never be emitted; the native emitter checks any it does emit.
            // GNU extension: section attributes on ELF.
            if matches!(
                self.context.configuration.target(),
                crate::target::Target::LinuxGnu | crate::target::Target::LinuxMusl
            ) && matches!(definition.kind, super::super::DefinitionKind::Function)
                && (self.bindings[definition.binding].linkage == Linkage::External
                    || attributes.used)
                && let Some(section) = attributes.section
                && crate::target::sections::ElfSectionKind::named(section).is_bss()
            {
                let name = self.context.string_cache.intern(section);
                self.error(
                    SemanticErrorKind::BssFunction,
                    self.bindings[definition.binding].name.source_vectors,
                    Some(name),
                    None,
                );
            }
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
            conventions: tu.alloc_slice_copy(&self.attributes.conventions),
            init_functions: init_functions.leak(),
            aliases: &[],
            refusals: tu.alloc_slice_copy(&self.attributes.refusals),
        }
    }

    /// Resolves target strings against emitted symbols after ABI finalization.
    /// Semantic overload identities are deliberately not linker spellings.
    pub(in crate::translation_phases::semantic_analysis) fn finish_aliases(
        &mut self,
        symbols: &mut SymbolFacts<'tu>,
    ) {
        if self.attributes.aliases.is_empty() {
            return;
        }
        let tu = self.types.tu;
        let mut aliases = ArenaVec::new_in(tu);
        let names = self.alias_symbols();
        for index in 0..self.attributes.aliases.len() {
            let request = self.attributes.aliases[index];
            let alias = self.bindings[request.binding];
            if self
                .linked_entity(self.function_identity(request.binding))
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
            if !request.emitted {
                continue;
            }
            let target = match names.resolve(request.target) {
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
            // An alias of a thread-local object is itself thread-local, as
            // Clang emits it. Clang also accepts an alias whose storage
            // duration differs from its target's and emits an alias that
            // addresses the wrong storage (one copy per thread or one shared
            // copy), so bcc refuses it.
            // C11: §6.2.4p4, p. 38; PDF p. 56.
            if (alias.duration == Duration::Thread) != (definition.duration == Duration::Thread) {
                self.refuse(UnsupportedAttribute::AliasThreadLocal, request.source);
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
        symbols.aliases = aliases.leak();
        symbols.refusals = tu.alloc_slice_copy(&self.attributes.refusals);
    }

    /// DLL storage depends on generated references in alias and initialization
    /// roots, which are known only after aliases resolve to emitted names.
    pub(in crate::translation_phases::semantic_analysis) fn finish_emitted_attributes(
        &mut self,
        symbols: &mut SymbolFacts<'tu>,
    ) {
        self.finish_dll_references(symbols);
        self.finish_dll();
        symbols
            .entities
            .extend(self.attributes.entities.iter().map(|(&k, &v)| (k, v)));
    }

    pub(super) fn alias_bindings(&self) -> impl Iterator<Item = usize> + '_ {
        self.attributes
            .aliases
            .iter()
            .filter(|request| request.emitted)
            .map(|request| request.binding)
    }

    /// Provisional targets make alias-reachable bodies participate in ABI
    /// numbering before unused declarations. Final validation follows naming.
    pub(super) fn alias_targets(&self) -> ArenaVec<'s, (usize, usize)> {
        let names = self.alias_symbols();
        let mut targets = ArenaVec::new_in(self.scratch);
        for request in &self.attributes.aliases {
            if request.emitted
                && let Ok(target) = names.resolve(request.target)
            {
                targets.push((request.binding, target));
            }
        }
        targets
    }

    fn alias_symbols(&self) -> AliasSymbols<'s> {
        let mut names = AliasSymbols {
            aliases:     ArenaMap::with_hasher_in(FxBuildHasher, self.scratch),
            definitions: ArenaMap::with_hasher_in(FxBuildHasher, self.scratch),
        };
        for request in &self.attributes.aliases {
            if request.emitted {
                _ = names
                    .aliases
                    .insert(self.alias_symbol(request.binding), request.target);
            }
        }
        for (binding, data) in self.bindings.iter().enumerate() {
            if data.linkage != Linkage::None
                && let Some((latest, defined)) = self.linked_entity(self.function_identity(binding))
            {
                _ = names
                    .definitions
                    .insert(self.alias_symbol(latest), (latest, defined));
            }
        }
        names
    }

    fn alias_symbol(&self, binding: usize) -> StringCacheId {
        let identity = self.function_identity(binding);
        self.function_extensions
            .abi_names
            .get(&identity)
            .copied()
            .unwrap_or(self.bindings[binding].name.name)
    }
}

/// Linker spellings, independent of the semantic identities that merge C
/// declarations. Following at most one edge per alias detects every cycle.
struct AliasSymbols<'s> {
    aliases:     ArenaMap<'s, StringCacheId, StringCacheId>,
    definitions: ArenaMap<'s, StringCacheId, (usize, bool)>,
}

impl AliasSymbols<'_> {
    fn resolve(&self, mut name: StringCacheId) -> Result<usize, SemanticErrorKind> {
        for _ in 0..=self.aliases.len() {
            if let Some(&next) = self.aliases.get(&name) {
                name = next;
            } else {
                return match self.definitions.get(&name) {
                    | Some(&(target, true)) => Ok(target),
                    | _ => Err(SemanticErrorKind::AliasTarget),
                };
            }
        }
        Err(SemanticErrorKind::AliasCycle)
    }
}

/// Whether an attribute that a redeclaration adds after the definition is
/// ignored, as Clang's `checkNewAttributesAfterDef` does. Type attributes
/// (layouts, conventions and `noreturn`, which Clang keeps in the function
/// type) still apply; a late `alias` is the alias-definition error.
fn follows_definition(rule: Rule, name: &str) -> bool {
    !matches!(
        rule,
        Rule::Representation
            | Rule::TransparentUnion
            | Rule::Convention(_)
            | Rule::RecordLayout { .. }
            | Rule::Alias
            | Rule::CallDiagnostic
    ) && name != "noreturn"
}

/// Attribute work in semantic scratch.
pub(in crate::translation_phases::semantic_analysis) struct AttributeState<'tu, 's> {
    /// Specifier lists already checked by name, by address.
    pub(in crate::translation_phases::semantic_analysis) checked: ArenaMap<'s, usize, ()>,
    pub(in crate::translation_phases::semantic_analysis) refusals: ArenaVec<'s, AttributeRefusal>,
    pub(in crate::translation_phases::semantic_analysis) conventions:
        ArenaVec<'s, super::super::ConventionFacts>,
    pub(super) entities: ArenaMap<'s, EntityId, EntityAttributes<'tu>>,
    pub(super) call_diagnostics:
        ArenaMap<'s, (usize, usize), super::call_diagnostics::ParsedDiagnostic>,
    sections: ArenaMap<'s, EntityId, SourceVectors>,
    /// Constructor and destructor priorities requested for an entity.
    initializers: ArenaMap<'s, EntityId, (Option<u16>, Option<u16>)>,
    aliases: ArenaVec<'s, AliasRequest>,
    /// Stable linked semantic identities with an alias request, one each.
    alias_names: ArenaMap<'s, StringCacheId, usize>,
    /// Validated cleanups by declarator identity, waiting for the end of
    /// their declarator's initializer, with the attribute's use of the
    /// function.
    pending_cleanups: ArenaMap<
        's,
        usize,
        (
            usize,
            usize,
            SourceVectors,
            crate::translation_phases::parsing::syntax::Identifier,
        ),
    >,
    /// Gotos written in the initializers of cleanup variables, by statement
    /// address, with each such variable.
    pub(in crate::translation_phases::semantic_analysis) initializing_jumps:
        ArenaVec<'s, (usize, usize)>,
    /// The attribute names written on an entity's declarations up to and
    /// including its definition.
    before_definition: ArenaMap<'s, (EntityId, StringCacheId), ()>,
    /// The DLL attributes of the declaration being processed (`dll.rs`).
    pub(super) pending_dll: ArenaVec<'s, (DllStorage, SourceVectors)>,
    /// Each entity's DLL storage so far.
    pub(super) dll: ArenaMap<'s, EntityId, DllState>,
}

impl<'tu, 's> AttributeState<'tu, 's> {
    /// A constructor or destructor is retained at its definition.
    pub(in crate::translation_phases::semantic_analysis) fn init_function(
        &self,
        entity: EntityId,
    ) -> bool {
        self.initializers.contains_key(&entity)
    }

    pub(in crate::translation_phases::semantic_analysis) fn new(scratch: &'s Bump) -> Self {
        Self {
            checked:            ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            refusals:           ArenaVec::new_in(scratch),
            conventions:        ArenaVec::new_in(scratch),
            entities:           ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            call_diagnostics:   ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            sections:           ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            initializers:       ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            aliases:            ArenaVec::new_in(scratch),
            alias_names:        ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            pending_cleanups:   ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            before_definition:  ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            initializing_jumps: ArenaVec::new_in(scratch),
            pending_dll:        ArenaVec::new_in(scratch),
            dll:                ArenaMap::with_hasher_in(FxBuildHasher, scratch),
        }
    }

    /// The cleanup variables whose initializers are being analyzed: their
    /// regions open once the initializer is complete.
    pub(in crate::translation_phases::semantic_analysis) fn initializing_cleanups(
        &self,
    ) -> impl Iterator<Item = usize> {
        self.pending_cleanups
            .values()
            .map(|&(variable, ..)| variable)
    }

    pub(super) fn entity(&mut self, entity: EntityId) -> &mut EntityAttributes<'tu> {
        self.entities.entry(entity).or_default()
    }
}

/// An alias declaration and the name of its target.
#[derive(Clone, Copy)]
struct AliasRequest {
    binding: usize,
    target:  StringCacheId,
    source:  SourceVectors,
    emitted: bool,
}
