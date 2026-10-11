//! User-facing diagnostics: a small builder for messages with labelled source
//! ranges, notes, and help, plus a renderer that prints them as annotated
//! source snippets in the style of `rustc`:
//!
//! ```text
//! error: expected `;` after the declarator, found string literal `"abc"`
//!  --> example.c:1:7
//!   |
//! 1 | int x "abc";
//!   |       ^^^^^ expected `,`, `=`, `;`, or a function body
//!   |
//!   = note: C99 §6.7: a declaration ends with `;`
//! ```
//!
//! Everything printed comes from the C source and fixed wording; no internal
//! representation (arena indices, interned-string handles, Rust `Debug`
//! output) may appear in a diagnostic.
//! C99: required diagnostics for syntax and constraint violations §5.1.1.3p1,
//! p. 11; PDF p. 23. This renderer carries locations from translation phases
//! 1-7 (§5.1.1.2, pp. 9-10; PDF pp. 21-22).

mod color;

mod formatting;

mod model;

#[cfg(test)]
mod owned;

mod render;

use std::fmt::{
    self,
    Display,
};

pub(crate) use color::ColorChoice;
#[cfg(test)]
use formatting::MAX_QUOTED_SPELLING;
pub(crate) use formatting::{
    closest_match,
    count_of,
    format_arguments_in,
    format_in,
    quote_spelling,
    write_c_quoted,
};
pub(crate) use model::{
    Diagnostic,
    Explanation,
    ToDiagnostic,
};
#[cfg(test)]
pub(crate) use owned::{
    OwnedDiagnostic,
    OwnedExplanation,
};
use owo_colors::Style;

use crate::{
    translation_phases::{
        Context,
        ErrorSeverity,
        SourceVector,
        SourceVectors,
    },
    util::bump::{
        ArenaString,
        ArenaVec,
        Bump,
    },
};

impl Renderer {
    /// Renders one diagnostic, ending with a blank line. The text lives in
    /// the renderer's scratch arena until the next diagnostic is rendered.
    pub(crate) fn render_text(
        &mut self,
        diagnostic: &Diagnostic<'_>,
        context: &Context<'_>,
    ) -> &str {
        // The previous text's borrow has ended; no raw scratch pointer is
        // retained in the renderer, diagnostic, or context.
        self.scratch.reset();
        self.render_in_scratch(diagnostic, context)
    }
}

/// Renders diagnostics with reusable scratch storage.
pub(crate) struct Renderer {
    color:   ColorChoice,
    scratch: Bump,
}

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod tests;
