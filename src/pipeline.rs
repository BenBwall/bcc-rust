//! Runs the translation phases over one translation unit in order: each
//! source file is lexed completely when it is opened, the whole unit is
//! preprocessed, and only then is it parsed.
//!
//! C99: translation phases 1-7, §5.1.1.2, pp. 9-10; PDF pp. 21-22.

mod analysis;

mod storage;

use std::path::Path;

pub(crate) use analysis::analyze_translation_unit;
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

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod tests;
