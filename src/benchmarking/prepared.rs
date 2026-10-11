//! Separates preprocessing from repeated parsing while keeping each arena
//! alive for the work that borrows it.

use std::path::Path;

use super::{
    BenchmarkInput,
    ParseBenchmarkSummary,
    input::benchmark_context,
};
use crate::{
    pipeline::{
        parse_with_arena,
        with_preprocessor,
    },
    translation_phases::{
        Context,
        parsing::{
            Parser,
            PreprocessedTranslationUnit,
        },
    },
    util::bump::Bump,
};

/// Runs translation phases 1 through 6, then passes a prepared parser to a
/// callback while its translation context remains alive.
#[doc(hidden)]
pub fn with_prepared_parse<R>(
    input: BenchmarkInput,
    inspect: impl FnOnce(PreparedParse<'_, '_, '_>) -> R,
) -> R {
    let tu = Bump::new();
    let mut context = benchmark_context(&tu);
    let preprocessed = prepare_parse_in_context(&mut context, input);
    let parse = Bump::new();
    inspect(PreparedParse {
        context: &mut context,
        preprocessed,
        parse: &parse,
    })
}

impl PreparedParse<'_, '_, '_> {
    /// Runs translation phase 7 and summarizes the parse.
    #[must_use]
    pub fn parse(self) -> ParseBenchmarkSummary {
        let unit = parse_with_arena(self.preprocessed, self.context, self.parse);
        ParseBenchmarkSummary {
            external_declarations: unit.external_declarations().len(),
            diagnostics:           self.context.pending_error_count(),
        }
    }
}

/// A translation unit preprocessed through phase 6 and ready to parse, so a
/// benchmark can time phase 7 alone.
#[doc(hidden)]
pub struct PreparedParse<'a, 'tu, 'parse> {
    context:          &'a mut Context<'tu>,
    preprocessed:     PreprocessedTranslationUnit,
    pub(super) parse: &'parse Bump,
}

fn prepare_parse_in_context(
    context: &mut Context<'_>,
    input: BenchmarkInput,
) -> PreprocessedTranslationUnit {
    with_preprocessor(
        context,
        Path::new("<input>"),
        input.source(),
        crate::headers::HeaderSearch::default(),
        |preprocessor, context, _pp| Parser::preprocess(preprocessor, context),
    )
}

impl std::fmt::Debug for PreparedParse<'_, '_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedParse").finish_non_exhaustive()
    }
}
