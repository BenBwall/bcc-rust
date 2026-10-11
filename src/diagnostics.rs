//! Diagnostics combine fixed wording with labelled C source ranges, notes,
//! and help. [`Renderer::render_text`] prints those ranges as annotated source
//! snippets and reuses its scratch arena between diagnostics. Internal arena
//! indices and interned-string handles do not appear in the output.
//!
//! For `int x "abc";`, a parser diagnostic labels the unexpected string.
//! The renderer looks up its source line, prints a caret under that range,
//! and appends the diagnostic's notes and help.
//!
//! Read [`Renderer::render_text`], [`Renderer`], and
//! [`Renderer::render_in_scratch`], then [`Explanation::at`]
//! and [`ToDiagnostic::diagnostic_in`].
//!
//! Files by role:
//! - Diagnostic construction: `model.rs`.
//! - Rendering and color: `render.rs`, `color.rs`.
//! - Arena text and spelling: `formatting.rs`.
//! - Test views and fixtures: `owned.rs`, `tests.rs`.
//!
//! C99: required diagnostics, §5.1.1.3 paragraph 1, p. 11; PDF p. 23.
//! Source locations span phases 1-7, §5.1.1.2 paragraph 1, pp. 9-10;
//! PDF pp. 21-22.

// Diagnostic construction
mod model;

// Rendering
mod color;
mod render;

// Text formatting
mod formatting;

// Test views
#[cfg(test)]
mod owned;

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

// Tests
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod tests;
