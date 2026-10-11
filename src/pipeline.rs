//! The batch driver lexes each source file when it is opened, preprocesses
//! the whole translation unit, and then parses the collected tokens. The
//! preprocessing and parser work arenas end with their stages; the resulting
//! syntax tree and retained diagnostics stay in the translation-unit context.
//!
//! For `int x = 1;`, preprocessing produces the complete token array before
//! the parser constructs the declaration. [`analyze_translation_unit`] then
//! resolves its type and initializer from the finished tree.
//!
//! [`lower_translation_unit`] hands the analyzed unit to the middle end:
//! lowering into an IR arena created after semantic analysis, and the
//! optimizer when one is requested.
//!
//! Read [`parse_translation_unit`], [`with_preprocessor`], and
//! [`parse_with_arena`], then [`analyze_translation_unit`] and
//! [`lower_translation_unit`].
//!
//! Files by role:
//! - Phase storage and token collection: `storage.rs`.
//! - Semantic handoff: `analysis.rs`.
//! - Middle-end handoff: `middle_end.rs`.
//! - End-to-end fixtures: `tests.rs` and `tests/`.
//!
//! C99: translation phases 1-7, §5.1.1.2 paragraph 1, pp. 9-10;
//! PDF pp. 21-22.

// Phase storage
mod storage;

// Semantic analysis
mod analysis;

// Middle end
mod middle_end;

use std::path::Path;

pub(crate) use analysis::analyze_translation_unit;
pub(crate) use middle_end::lower_translation_unit;
pub(crate) use storage::{
    parse_with_arena,
    preprocess_with_diagnostics,
    with_preprocessor,
};

use crate::{
    headers::HeaderSearch,
    translation_phases::{
        Context,
        TranslationError,
        parsing::{
            ParsedTranslationUnit,
            Parser,
            PreprocessedTranslationUnit,
        },
        preprocessing::{
            Preprocessor,
            Token,
        },
    },
    util::{
        bump::Bump,
        region_vec::RegionVec,
    },
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
