//! Retained phase-7 output: types, declaration occurrences, lexical scopes,
//! definitions, expression results and conversions. Scratch lookup and
//! traversal state is not retained here. Validation happens in the analyzer.
//! C99: §6.2.1-§6.2.4, pp. 29-32; PDF pp. 41-44;
//! §6.7, pp. 97-124; PDF pp. 109-136.

use super::{
    expressions::{
        Conversion,
        ExpressionInfo,
    },
    integer::Integer,
    types::{
        Field,
        TypeId,
        TypeKind,
        Types,
    },
};
use crate::{
    translation_phases::{
        Context,
        parsing::{
            SpecifierExtension,
            declaration_syntax::{
                Declaration,
                Declarator,
                TypeName,
            },
            syntax::{
                Expression,
                FunctionDefinition,
                Identifier,
            },
        },
    },
    util::{
        bump::ArenaMap,
        string_cache::StringCacheId,
    },
};

/// Durable semantic output. Working maps/stacks are gone when this is returned.
#[derive(Debug)]
pub(crate) struct SemanticTranslationUnit<'tu> {
    pub(crate) resolved:            super::ResolvedFacts<'tu>,
    pub(crate) correspondence:      super::CorrespondenceFacts<'tu>,
    pub(crate) sizes:               super::SizeFacts<'tu>,
    pub(crate) runtime_bounds:      &'tu [super::RuntimeBound<'tu>],
    pub(crate) control:             super::ControlFacts<'tu>,
    pub(crate) effects:             super::EffectFacts<'tu>,
    pub(super) expression_actions:  ArenaMap<'tu, usize, BoundSiteFacts<'tu>>,
    /// Declarators with vendor modifiers visited by the explicit type stack,
    /// including abstract declarators, retained for the lowering gate.
    /// C99: §6.7.5-6.7.6, pp. 114-122; PDF pp. 126-134 (extensions).
    pub(crate) declarators:         &'tu [Declarator<'tu>],
    /// Specifier extension chains, including those on type names and
    /// parameters. C99: §6.7, p. 97; PDF p. 109 (vendor extensions).
    pub(crate) spec_extensions:     &'tu [&'tu SpecifierExtension<'tu>],
    pub(crate) types:               Types<'tu>,
    pub(crate) bindings:            &'tu [Binding],
    pub(crate) definitions:         &'tu [Definition],
    /// Every function definition in traversal order, nested GNU definitions
    /// included, with its binding, body and parameter bindings.
    pub(crate) functions:           &'tu [FunctionRecord<'tu>],
    pub(crate) scopes:              &'tu [Scope],
    pub(crate) type_names:          &'tu [super::TypeNameFacts<'tu>],
    pub(super) type_name_indices:   ArenaMap<'tu, usize, super::TypeNameId>,
    pub(super) declaration_sites:   ArenaMap<'tu, usize, BoundSiteFacts<'tu>>,
    pub(super) size_query_sites:    ArenaMap<'tu, usize, super::SizeQueryFacts<'tu>>,
    /// Typed results in child-before-parent order; an expression's ordinal
    /// is its index here.
    pub(crate) expressions:         &'tu [ExpressionInfo<'tu>],
    /// Syntax identity (the node's address) to its ordinal in
    /// `expressions`; see [`Self::expression_index`].
    pub(super) expression_indices:  ArenaMap<'tu, usize, usize>,
    /// String initializer syntax to its destination element type, independent
    /// of array completion. C99: §6.7.8p14-15, p. 126; PDF p. 138.
    pub(super) string_initializers: ArenaMap<'tu, usize, TypeId>,
    /// Conversion records grouped by expression ordinal, each run in the
    /// order its conversions were applied; see [`Self::conversions_of`].
    pub(crate) conversions:         &'tu [Conversion<'tu>],
    /// `conversions[conversion_starts[i]..conversion_starts[i + 1]]` is the
    /// run of expression `i`. Records for an operand without a retained
    /// result follow the last run.
    pub(super) conversion_starts:   &'tu [usize],
    pub(crate) tag_declarations:    &'tu [(usize, usize)],
    /// Error-severity diagnostics semantic analysis reported; warnings and
    /// diagnostics suppressed inside recovered syntax are not counted.
    pub(crate) errors:              usize,
}

/// Execution attached to a declaration or type-name occurrence, rather than
/// inherited through a typedef. Saved bounds keep their existing identities;
/// these operand effects execute before the site's new declarator bounds.
/// GNU extension: GCC Typeof; C99: §6.5.3.4p2, p. 80; PDF p. 92.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BoundSiteFacts<'tu> {
    pub(crate) ty:                  TypeId,
    /// New runtime-bound identities at this written site, in type order.
    pub(crate) bounds:              &'tu [usize],
    pub(crate) operand_evaluations: &'tu [&'tu Expression<'tu>],
}

/// A resolved declaration occurrence, retained in lexical traversal order.
/// C99: §6.7p3-4, p. 97; PDF p. 109.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Binding {
    /// User, intrinsic or implicit-ordinary origin retained at lookup.
    /// C99: §6.2.1p4, pp. 29-30; PDF pp. 41-42 (shadowing).
    pub(crate) origin:   BindingOrigin,
    pub(crate) name:     Identifier,
    pub(crate) ty:       TypeId,
    pub(crate) scope:    usize,
    pub(crate) kind:     BindingKind,
    pub(crate) linkage:  Linkage,
    pub(crate) duration: Duration,
    pub(crate) value:    Option<Integer>,
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

/// A function definition with the facts lowering starts from, so that it
/// never matches bindings by source position. Recovered definitions also
/// have a record; lowering refuses their unit through the error gate.
/// C99: §6.9.1 paragraphs 2-10, pp. 141-142; PDF pp. 153-154.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FunctionRecord<'tu> {
    pub(crate) control:           super::FunctionControl,
    pub(crate) parameter_facts:   &'tu [super::ParameterFacts<'tu>],
    /// Source-ordered entry actions, before each parameter's new bounds.
    /// GNU extension: GCC Typeof; C99: §6.9.1p10, p. 142; PDF p. 154.
    pub(crate) old_style:         bool,
    pub(crate) parameter_sites:   &'tu [BoundSiteFacts<'tu>],
    pub(crate) syntax:            &'tu FunctionDefinition<'tu>,
    /// The function's binding; its type is the composite function type,
    /// which for an old-style definition lists the promoted parameter types
    /// (§6.9.1p7). `None` when the declarator names nothing.
    pub(crate) binding:           Option<usize>,
    /// The declared result type, `Unknown` when it is invalid; a return
    /// converts to its unqualified version (§6.8.6.4p3).
    pub(crate) result:            TypeId,
    /// The function scope that holds the parameters and the body's
    /// outermost declarations (§6.9.1p9).
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Retained lexical owner of the function parameters."
        )
    )]
    pub(crate) scope:             usize,
    /// Prototype bindings referenced by bounds and their corresponding body
    /// bindings, retained when replaying the definition's prototype scope.
    /// C99: §6.9.1p10, p. 142; PDF p. 154.
    pub(crate) parameter_aliases: &'tu [(usize, usize)],
}

/// A lexical scope and its enclosing scope.
/// C99: §6.2.1 paragraphs 1-4, pp. 29-30; PDF pp. 41-42.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Scope {
    pub(crate) parent: Option<usize>,
    pub(crate) kind:   ScopeKind,
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

/// A completed object or function definition and its declaration binding.
/// C99: §6.9 paragraph 5, p. 140; PDF p. 152.
/// C99: §6.9.2 paragraph 2, p. 143; PDF p. 155.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Definition {
    pub(crate) binding: usize,
    pub(crate) kind:    DefinitionKind,
}

impl<'tu> SemanticTranslationUnit<'tu> {
    /// Identity-indexed type-site execution; provenance is not a binding join.
    pub(crate) fn declaration_site(
        &self,
        declaration: &Declaration<'tu>,
    ) -> Option<BoundSiteFacts<'tu>> {
        self.declaration_sites
            .get(&std::ptr::from_ref(declaration).addr())
            .copied()
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Retained written type-site interface inspected by semantic tests."
        )
    )]
    pub(crate) fn type_name_site(&self, name: &TypeName<'tu>) -> Option<BoundSiteFacts<'tu>> {
        self.type_name_fact(name)?
            .size_query
            .map(|query| query.site)
    }

    pub(crate) fn type_name_fact(
        &self,
        name: &TypeName<'tu>,
    ) -> Option<&super::TypeNameFacts<'tu>> {
        let id = self
            .type_name_indices
            .get(&super::type_name_identity(name))?;
        let fact = self.type_names.get(id.0)?;
        debug_assert_eq!(fact.id, *id, "type-name indices select their own records");
        debug_assert!(
            std::ptr::eq(fact.syntax, name),
            "type-name lookup preserves syntax identity"
        );
        Some(fact)
    }

    /// Size-query execution includes the surrounding sizeof evaluation mode.
    pub(crate) fn size_query_site(
        &self,
        expression: &Expression<'tu>,
    ) -> Option<super::SizeQueryFacts<'tu>> {
        self.size_query_sites
            .get(&std::ptr::from_ref(expression).addr())
            .copied()
    }

    /// Whether analysis classified this expression as initializing the whole
    /// array, including braced literals and arrays completed from the string.
    /// C99: §6.7.8p14-15, p. 126; PDF p. 138.
    pub(crate) fn string_initializes(&self, expression: &Expression<'tu>, ty: TypeId) -> bool {
        let TypeKind::Array(element, _) = self.types.kind(ty) else {
            return false;
        };
        self.string_initializers
            .get(&std::ptr::from_ref(expression).addr())
            == Some(&element.unqualified())
    }

    /// The ordinal of an expression's record, in expected O(1). Parentheses
    /// have their own record, a copy of the inner one; an expression that
    /// analysis never typed, such as one inside unmodeled syntax, has none.
    /// C99: §6.5p1, p. 67; PDF p. 79.
    pub(crate) fn expression_index(&self, expression: &Expression<'tu>) -> Option<usize> {
        self.expression_indices
            .get(&std::ptr::from_ref(expression).addr())
            .copied()
    }

    /// The typed record of `expression`, in expected O(1).
    /// C99: §6.3.2.1p1-4, p. 46; PDF p. 58.
    pub(crate) fn expression_info(
        &self,
        expression: &Expression<'tu>,
    ) -> Option<&'tu ExpressionInfo<'tu>> {
        self.expression_index(expression)
            .map(|index| &self.expressions[index])
    }

    /// The conversions applied to expression `index`, in application order:
    /// a decay or lvalue conversion first, then any arithmetic, assignment
    /// or default-argument conversion. Each record attaches to the operand
    /// node its parent sees, so a parenthesized operand's run belongs to the
    /// parentheses.
    /// C99: §6.3, pp. 42-48; PDF pp. 54-60.
    pub(crate) fn conversions_of(&self, index: usize) -> &'tu [Conversion<'tu>] {
        &self.conversions[self.conversion_starts[index]..self.conversion_starts[index + 1]]
    }

    /// The member that `.` or `->` selects, in O(1), with its byte and bit
    /// offset from the record's start and the anonymous members on its path.
    /// `member` may be wrapped in parentheses, `__extension__` or a generic
    /// selection.
    /// C99: §6.5.2.3 paragraphs 3-4, p. 73; PDF p. 85.
    /// C11: §6.7.2.1 paragraph 13, p. 115; PDF p. 133 (anonymous members).
    pub(crate) fn selected_field(&self, member: &'tu Expression<'tu>) -> Option<&'tu Field<'tu>> {
        let facts = self.member_facts(member)?;
        self.types
            .tags
            .get(facts.tag)?
            .fields
            .get()
            .get(facts.field)
    }

    /// The conversions applied to `expression`, empty when it has no record.
    /// C99: §6.3, pp. 42-48; PDF pp. 54-60.
    pub(crate) fn expression_conversions(
        &self,
        expression: &Expression<'tu>,
    ) -> &'tu [Conversion<'tu>] {
        self.expression_index(expression)
            .map_or(&[], |index| self.conversions_of(index))
    }

    /// Whether code generation may consume this unit: no phase, this one
    /// included, has reported an error-severity diagnostic. Warnings do not
    /// block lowering. Types that analysis gave up on without a diagnostic
    /// remain possible, so lowering still rejects any type for which
    /// [`Types::unanalyzed`] holds.
    /// C99: §5.1.1.3 paragraph 1, p. 11; PDF p. 23.
    pub(crate) fn lowerable(&self, context: &Context<'_>) -> bool {
        self.errors == 0 && context.error_count() == 0
    }
}

impl<'tu> SemanticTranslationUnit<'tu> {
    pub(crate) fn expression_actions(
        &self,
        expression: &Expression<'_>,
    ) -> Option<BoundSiteFacts<'tu>> {
        self.expression_actions
            .get(&std::ptr::from_ref(expression).addr())
            .copied()
    }
}

/// Lookup origin, independent of spelling and user assembler labels.
/// C99: §6.2.1p4, pp. 29-30; PDF pp. 41-42 (user shadowing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BindingOrigin {
    User,
    Intrinsic(crate::builtins::BuiltinId),
    ImplicitOrdinary,
    /// Temporary M6 recognition bridge: atomic/x86/vector/type-generic
    /// families.
    LegacyIntrinsic,
    /// Unknown reserved names retain opaque signatures and bounded refusals.
    OpaqueIntrinsic,
}

impl BindingOrigin {
    pub(crate) fn is_intrinsic(self) -> bool {
        matches!(
            self,
            Self::Intrinsic(_) | Self::LegacyIntrinsic | Self::OpaqueIntrinsic
        )
    }

    pub(crate) fn descriptor(self) -> Option<&'static crate::builtins::BuiltinDescriptor> {
        match self {
            | Self::Intrinsic(id) => Some(id.descriptor()),
            | _ => None,
        }
    }

    pub(crate) fn library(self) -> Option<&'static str> {
        self.descriptor().and_then(|descriptor| descriptor.library)
    }
}
