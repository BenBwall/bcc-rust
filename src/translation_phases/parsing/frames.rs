//! Grammar frames suspend and resume on the parser's explicit control stack.
//! Each production's `step` method returns an owned action; child productions
//! use the same stack and return typed values. Frame delimiters and scope
//! changes stay with the production that owns them. Type checking belongs to
//! semantic analysis.
//!
//! For `int x = 1;`, the external-declaration frame pushes a declaration frame.
//! Its specifier, declarator, and initializer children return in turn. The
//! declaration consumes `;` and returns a root to the outer loop.
//!
//! Start with [`super::machine::ParseFrame::step`], then
//! [`external_declaration::ExternalDeclarationFrame::step`] and
//! [`declaration::DeclarationFrame::step`]. Read
//! [`expression::ExpressionFrame::step`] for precedence reduction.
//!
//! Files under `frames/` are grouped by production:
//!
//! - Declarations and types: `external_declaration.rs`, `declaration.rs`,
//!   `declaration_specifiers.rs`, `declarator.rs`, `parameter_list.rs`,
//!   `struct_or_union.rs`, `enum_specifier.rs`, and `type_name.rs`.
//! - Function bodies: `function_definition.rs`, `compound_statement.rs`, and
//!   `statement.rs`.
//! - Expressions and initialization: `expression.rs`,
//!   `expression_operators.rs`, `expression/lookahead.rs`, and
//!   `initializer.rs`.
//!
//! C99: translation phase 7, §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22.
//! C99: phrase-structure grammar, §6.5-§6.9, pp. 67-144; PDF pp. 79-156;
//! §A.2, pp. 409-416; PDF pp. 421-428.

// Declarations and types
pub(crate) mod declaration;
pub(crate) mod declaration_specifiers;
pub(crate) mod declarator;
pub(crate) mod enum_specifier;
pub(crate) mod external_declaration;
pub(crate) mod parameter_list;
pub(crate) mod struct_or_union;
pub(crate) mod type_name;

// Function bodies
pub(crate) mod compound_statement;
pub(crate) mod function_definition;
pub(crate) mod statement;

// Expressions and initialization
pub(crate) mod expression;
pub(crate) mod expression_operators;
pub(crate) mod initializer;

use super::{
    declaration_syntax,
    extensions::{
        gnu,
        modern,
        msvc,
    },
    machine,
    syntax,
};
