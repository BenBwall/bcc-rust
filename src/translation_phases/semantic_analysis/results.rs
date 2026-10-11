//! Retained phase-7 output: types, declaration occurrences, lexical scopes,
//! definitions, expression results and conversions. Scratch lookup and
//! traversal state is not retained here. Validation happens in the analyzer.
//! C99: §6.2.1-§6.2.4, pp. 29-32; PDF pp. 41-44;
//! §6.7, pp. 97-124; PDF pp. 109-136.

use super::{
    Conversion,
    ExpressionInfo,
    Identifier,
    Integer,
    Parameter,
    SourceVectors,
    TypeId,
    Types,
    functions,
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
    pub(crate) definitions:      &'tu [functions::Definition],
    pub(crate) scopes:           &'tu [Scope],
    pub(crate) type_names:       &'tu [(SourceVectors, TypeId)],
    pub(crate) parameters:       &'tu [(SourceVectors, &'tu [Parameter])],
    pub(crate) expressions:      &'tu [ExpressionInfo<'tu>],
    pub(crate) conversions:      &'tu [Conversion<'tu>],
    pub(crate) tag_declarations: &'tu [(usize, usize)],
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
