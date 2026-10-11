//! Retained phase-7 output: types, declaration occurrences, lexical scopes,
//! definitions, expression results and conversions. Scratch lookup and
//! traversal state is not retained here. Validation happens in the analyzer.
//! C99: §6.2.1-§6.2.4, pp. 29-32; PDF pp. 41-44;
//! §6.7, pp. 97-124; PDF pp. 109-136.

use super::{
    ArenaMap,
    Context,
    Conversion,
    Expression,
    ExpressionInfo,
    FunctionDefinition,
    Identifier,
    Integer,
    Parameter,
    SourceVectors,
    StringCacheId,
    TypeId,
    Types,
};

/// Durable semantic output. Working maps/stacks are gone when this is returned.
#[derive(Debug)]
pub(crate) struct SemanticTranslationUnit<'tu> {
    pub(crate) types:              Types<'tu>,
    pub(crate) bindings:           &'tu [Binding],
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Finalized definitions are retained for backend lowering."
        )
    )]
    pub(crate) definitions:        &'tu [Definition],
    /// Every function definition in traversal order, nested GNU definitions
    /// included, with its binding, body and parameter bindings.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Function records are retained for backend lowering."
        )
    )]
    pub(crate) functions:          &'tu [FunctionRecord<'tu>],
    pub(crate) scopes:             &'tu [Scope],
    pub(crate) type_names:         &'tu [(SourceVectors, TypeId)],
    pub(crate) parameters:         &'tu [(SourceVectors, &'tu [Parameter])],
    /// Typed results in child-before-parent order; an expression's ordinal
    /// is its index here.
    pub(crate) expressions:        &'tu [ExpressionInfo<'tu>],
    /// Syntax identity (the node's address) to its ordinal in
    /// `expressions`; see [`Self::expression_index`].
    pub(super) expression_indices: ArenaMap<'tu, usize, usize>,
    /// Conversion records grouped by expression ordinal, each run in the
    /// order its conversions were applied; see [`Self::conversions_of`].
    pub(crate) conversions:        &'tu [Conversion<'tu>],
    /// `conversions[conversion_starts[i]..conversion_starts[i + 1]]` is the
    /// run of expression `i`. Records for an operand without a retained
    /// result follow the last run.
    pub(super) conversion_starts:  &'tu [usize],
    pub(crate) tag_declarations:   &'tu [(usize, usize)],
    /// Error-severity diagnostics semantic analysis reported; warnings and
    /// diagnostics suppressed inside recovered syntax are not counted.
    pub(crate) errors:             usize,
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
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Lowering reads these records; no lowering exists yet."
    )
)]
pub(crate) struct FunctionRecord<'tu> {
    pub(crate) syntax:     &'tu FunctionDefinition<'tu>,
    /// The function's binding; its type is the composite function type,
    /// which for an old-style definition lists the promoted parameter types
    /// (§6.9.1p7). `None` when the declarator names nothing.
    pub(crate) binding:    Option<usize>,
    /// The declared result type, `Unknown` when it is invalid; a return
    /// converts to its unqualified version (§6.8.6.4p3).
    pub(crate) result:     TypeId,
    /// The function scope that holds the parameters and the body's
    /// outermost declarations (§6.9.1p9).
    pub(crate) scope:      usize,
    /// One entry per declared parameter in the declarator's order (the
    /// identifier list for an old-style definition), each the parameter's
    /// body binding. A lone `void` gives no entries. An unnamed parameter or
    /// a repeated name has `None`. A binding's own type is the declared
    /// type, which an old-style definition converts to from the promoted
    /// argument type on entry (§6.9.1p10).
    pub(crate) parameters: &'tu [Option<usize>],
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
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Lowering looks expressions up; no lowering exists yet."
        )
    )]
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

    /// The conversions applied to `expression`, empty when it has no record.
    /// C99: §6.3, pp. 42-48; PDF pp. 54-60.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Lowering looks expressions up; no lowering exists yet."
        )
    )]
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
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Lowering gates on it; no lowering exists yet.")
    )]
    pub(crate) fn lowerable(&self, context: &Context<'_>) -> bool {
        self.errors == 0 && context.error_count() == 0
    }
}
