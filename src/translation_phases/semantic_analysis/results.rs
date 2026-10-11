//! Retained phase-7 output: types, declaration occurrences, lexical scopes,
//! definitions, expression results and conversions. Scratch lookup and
//! traversal state is not retained here. Validation happens in the analyzer.
//! C99: §6.2.1-§6.2.4, pp. 29-32; PDF pp. 41-44;
//! §6.7, pp. 97-124; PDF pp. 109-136.

use super::{
    Context,
    Conversion,
    ExpressionInfo,
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
    pub(crate) types:            Types<'tu>,
    pub(crate) bindings:         &'tu [Binding],
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Finalized definitions are retained for backend lowering."
        )
    )]
    pub(crate) definitions:      &'tu [Definition],
    pub(crate) scopes:           &'tu [Scope],
    pub(crate) type_names:       &'tu [(SourceVectors, TypeId)],
    pub(crate) parameters:       &'tu [(SourceVectors, &'tu [Parameter])],
    pub(crate) expressions:      &'tu [ExpressionInfo<'tu>],
    pub(crate) conversions:      &'tu [Conversion<'tu>],
    pub(crate) tag_declarations: &'tu [(usize, usize)],
    /// Error-severity diagnostics semantic analysis reported; warnings and
    /// diagnostics suppressed inside recovered syntax are not counted.
    pub(crate) errors:           usize,
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

impl SemanticTranslationUnit<'_> {
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
