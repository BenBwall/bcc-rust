//! The batch driver lexes each source file when it is opened, preprocesses
//! the whole translation unit, and then parses the collected tokens. The
//! preprocessing and parser work arenas end with their stages; the resulting
//! syntax tree and retained diagnostics stay in the translation-unit context.
//!
//! For `int x = 1;`, preprocessing produces the complete token array before
//! the parser constructs the declaration. [`analyze_translation_unit`] then
//! resolves its type and initializer from the finished tree.
//!
//! Read [`parse_translation_unit`], [`with_preprocessor`], and
//! [`parse_with_arena`], then [`analyze_translation_unit`].
//!
//! Files by role:
//! - Stage drivers: `stages.rs` scopes the preprocessing and parser arenas to
//!   their stages and collects the preprocessed tokens.
//! - End-to-end fixtures: `tests.rs`.
//!
//! C99: translation phases 1-7, §5.1.1.2 paragraph 1, pp. 9-10;
//! PDF pp. 21-22.

// Stage drivers
mod stages;

use std::path::Path;

pub(crate) use stages::{
    parse_with_arena,
    preprocess_with_diagnostics,
    with_preprocessor,
};

use crate::{
    headers::HeaderSearch,
    translation_phases::{
        Context,
        parsing::{
            ParsedTranslationUnit,
            Parser,
        },
    },
    util::bump::Bump,
};

/// Completes preprocessing before constructing the parser. The parser-facing
/// token stream owns its region and lives until phase 7 ends.
pub(crate) fn parse_translation_unit<'tu>(
    context: &mut Context<'tu>,
    source_filename: &Path,
    source: &'tu str,
    search: HeaderSearch<'_>,
) -> ParsedTranslationUnit<'tu> {
    let preprocessed = with_preprocessor(
        context,
        source_filename,
        source,
        search,
        |preprocessor, context, _pp| Parser::preprocess(preprocessor, context),
    );
    let parse = Bump::new();
    parse_with_arena(preprocessed, context, &parse)
}

/// The semantic half of phase 7 starts after syntax parsing has completed.
/// Inspection callers may stop at `parse_translation_unit` to keep syntax modes
/// unchanged. C99: §5.1.1.2p1, p. 10; PDF p. 22.
pub(crate) fn analyze_translation_unit<'tu>(
    context: &mut Context<'tu>,
    unit: &ParsedTranslationUnit<'tu>,
) -> crate::translation_phases::semantic_analysis::SemanticTranslationUnit<'tu> {
    crate::translation_phases::semantic_analysis::analyze(context, unit)
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
