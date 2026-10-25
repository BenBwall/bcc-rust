//! The phase-7 grammar as recursive-descent productions on [`Parser`].
//!
//! Each production is a `parse_*` method that consumes its tokens, calls the
//! productions it is built from, and returns its syntax. Every production
//! entry goes through [`Parser::enter_production`], which counts the nesting
//! depth against the configured limit and names the production that every
//! diagnostic captures. A production returns `None` only after a terminal
//! resource failure has been recorded; syntax errors are reported, recovered
//! through [`Parser::recover`], and return explicitly recovered syntax.
//! Productions own their delimiters and scope changes, and keep growing lists
//! in pooled parse-arena scratch vectors until they copy them into the tree.
//! Type checking and the remaining constraints belong to semantic analysis.
//!
//! For `int x = 1;`, [`Parser::drive`] calls
//! [`Parser::parse_external_declaration`], which calls
//! [`Parser::parse_declaration`]. That production calls
//! [`Parser::parse_declaration_specifiers`] for `int`,
//! [`Parser::parse_declarator`] for `x` (publishing `x` in the current scope),
//! and [`Parser::parse_initializer`] for `1`, whose expression is parsed by
//! [`Parser::parse_expression`]. The declaration consumes `;` and returns its
//! syntax, which becomes the root.
//!
//! Read [`Parser::enter_production`] and [`ProductionKind`] first, then
//! `declarations.rs` for the declaration/function-definition split,
//! `expressions.rs` for the Double-E precedence reducer, and `statements.rs`
//! for statements and blocks.
//!
//! Files under `grammar/` are grouped by production:
//!
//! - Declarations and types: `declarations.rs` (external declarations,
//!   declarations, and function definitions), `declaration_specifiers.rs`,
//!   `declarators.rs`, `parameter_lists.rs`, `struct_or_union.rs`,
//!   `enum_specifier.rs`, and `type_names.rs`.
//! - Function bodies: `statements.rs` (statements and compound statements).
//! - Expressions and initialization: `expressions.rs`,
//!   `expression_operators.rs`, `expressions/lookahead.rs`, and
//!   `initializers.rs`.
//! - Extensions: `modern.rs` (later ISO operands, generic selections,
//!   assertions, and attributes), `gnu.rs`, and `msvc.rs`.
//!
//! C99: translation phase 7, §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22.
//! C99: phrase-structure grammar, §6.5-§6.9, pp. 67-144; PDF pp. 79-156;
//! §A.2, pp. 409-416; PDF pp. 421-428. Nesting depth is bounded by a
//! configurable limit rather than the minimums of §5.2.4.1, pp. 20-21;
//! PDF pp. 32-33.

// Declarations and types
mod declaration_specifiers;
mod declarations;
mod declarators;
mod enum_specifier;
mod parameter_lists;
mod struct_or_union;
mod type_names;

// Function bodies
mod statements;

// Expressions and initialization
pub(super) mod expression_operators;
mod expressions;
mod initializers;

// Extensions
mod gnu;
mod modern;
mod msvc;

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "test workloads and owned inspection snapshots use the global allocator"
)]
mod tests;

pub(super) use expressions::{
    CallState,
    ExpressionOperand,
};
pub(super) use initializers::DesignationState;
pub(super) use statements::is_statement_keyword;

use super::{
    Parser,
    errors::ParserResource,
};

impl Parser<'_, '_, '_> {
    /// Enters a production of `kind`: counts its nesting depth and makes it
    /// the production that diagnostics name. Pair every successful entry with
    /// [`Self::leave_production`].
    ///
    /// Returns `None` after recording the terminal resource failure when the
    /// entry would exceed the nesting-depth limit, or when the syntax-node or
    /// source-segment limit has been crossed since the last entry.
    #[inline(always)]
    pub(super) fn enter_production(&mut self, kind: ProductionKind) -> Option<ProductionEntry> {
        if self.depth >= self.limits.nesting_depth
            || self.syntax_nodes > self.limits.syntax_nodes
            || self.context.source_segment_count() > self.limits.source_segments
        {
            self.exceed_production_limit();
            return None;
        }
        self.depth += 1;
        self.max_depth = self.max_depth.max(self.depth);
        let entry = ProductionEntry {
            active: self.active_production,
        };
        self.active_production = kind;
        Some(entry)
    }

    /// Leaves the production entered as `entry`.
    #[inline(always)]
    pub(super) fn leave_production(&mut self, entry: ProductionEntry) {
        self.depth -= 1;
        self.active_production = entry.active;
    }

    /// Records the resource failure [`Self::enter_production`] detected; the
    /// error root is returned by [`Parser::drive`].
    #[cold]
    #[inline(never)]
    fn exceed_production_limit(&mut self) {
        let (resource, limit) = if self.depth >= self.limits.nesting_depth {
            (ParserResource::NestingDepth, self.limits.nesting_depth)
        } else if self.syntax_nodes > self.limits.syntax_nodes {
            (ParserResource::SyntaxNodes, self.limits.syntax_nodes)
        } else {
            (ParserResource::SourceSegments, self.limits.source_segments)
        };
        self.failure = self.resource_failure(resource, limit);
    }
}

/// The production that was active before an entry, restored when it leaves.
#[derive(Clone, Copy)]
pub(super) struct ProductionEntry {
    active: ProductionKind,
}

/// Stable identity for a production family, used by recovery and diagnostics
/// instead of free-form strings.
///
/// C99: the families partition the clause-6 grammar described using the
/// notation of §6.1, p. 29; PDF p. 41. Their identity is an implementation
/// mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProductionKind {
    Modern,
    Gnu,
    Msvc,
    /// Translation-unit entry for one external declaration.
    ExternalDeclaration,
    /// Declaration shell and init-declarator list.
    Declaration,
    /// Declaration-specifier or specifier-qualifier sequence.
    DeclarationSpecifiers,
    /// Named, abstract, or maybe-abstract declarator.
    Declarator,
    /// Prototype or K&R parameter list.
    ParameterList,
    /// Struct or union tag specifier and optional member body.
    StructOrUnionSpecifier,
    /// Enum tag specifier and optional enumerator body.
    EnumSpecifier,
    /// Specifier-qualifier list and optional abstract declarator.
    TypeName,
    /// C99 expression with a caller-selected grammar entry and boundary.
    Expression,
    /// Scalar or brace-enclosed initializer.
    Initializer,
    /// Function definition continuation after a completed declaration head.
    FunctionDefinition,
    /// Brace-delimited ordered block-item sequence.
    CompoundStatement,
    /// One labeled, compound, expression, selection, iteration, or jump
    /// statement.
    Statement,
}

impl ProductionKind {
    /// Returns the stable kebab-case label that names the production in
    /// diagnostic text.
    #[expect(
        dead_code,
        reason = "No current diagnostic names its production; the labels stay stable for those \
                  that will."
    )]
    pub(super) fn label(self) -> &'static str {
        match self {
            | Self::Modern => "iso-construct",
            | Self::Gnu => "gnu-construct",
            | Self::Msvc => "msvc-construct",
            | Self::ExternalDeclaration => "external-declaration",
            | Self::Declaration => "declaration",
            | Self::DeclarationSpecifiers => "declaration-specifiers",
            | Self::Declarator => "declarator",
            | Self::ParameterList => "parameter-list",
            | Self::StructOrUnionSpecifier => "struct-or-union-specifier",
            | Self::EnumSpecifier => "enum-specifier",
            | Self::TypeName => "type-name",
            | Self::Expression => "expression",
            | Self::Initializer => "initializer",
            | Self::FunctionDefinition => "function-definition",
            | Self::CompoundStatement => "compound-statement",
            | Self::Statement => "statement",
        }
    }
}

/// Panics on a child result that the calling production never receives.
///
/// Takes `panic!`'s format arguments. Each use generates its own cold,
/// never-inlined function, instantiated for exactly the values that use
/// formats; the arguments are formatted inside it, and the caller keeps only
/// the call.
macro_rules! unexpected_return {
    ($($format:tt)+) => {{
        #[cold]
        #[inline(never)]
        fn unexpected_return<F: FnOnce() -> ::std::convert::Infallible>(report: F) -> ! {
            match report() {}
        }
        unexpected_return(|| -> ::std::convert::Infallible { panic!($($format)+) })
    }};
}

pub(super) use unexpected_return;
