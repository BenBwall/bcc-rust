//! Entry points for the Criterion and coz benchmarks.

mod input;

mod preprocessing;

mod measurements;

mod semantic;

mod inspection;

mod prepared;

use std::{
    path::Path,
    sync::OnceLock,
};

pub use input::BenchmarkInput;
use input::benchmark_context;
use inspection::summarize_parse;
pub use inspection::{
    parse_file,
    parse_file_with_library,
    parse_msvc_source,
    parse_source,
};
pub use measurements::{
    ArenaUsage,
    arena_usage,
};
pub use prepared::{
    PreparedParse,
    with_prepared_parse,
};
pub use preprocessing::{
    lex,
    preprocess,
    preprocess_one_million,
};
pub use semantic::{
    sema,
    sema_source,
};

use crate::{
    configuration::CompilerConfiguration,
    pipeline::{
        parse_with_arena,
        with_preprocessor,
    },
    translation_phases::{
        Context,
        TranslationPhase,
        parsing::{
            Parser,
            PreprocessedTranslationUnit,
        },
        preprocessor_tokenizer::TokenSource,
    },
    util::bump::Bump,
};

/// Runs translation phases 1 through 7 and summarizes the parse.
#[doc(hidden)]
#[must_use]
pub fn parse(input: BenchmarkInput) -> ParseBenchmarkSummary {
    let tu = Bump::new();
    summarize_parse(
        &tu,
        Path::new("<input>"),
        input.source(),
        crate::headers::HeaderSearch::default(),
    )
}

/// Summary of one benchmarked parse, returned so the work cannot be elided
/// and so benchmark setup can reject inputs that produce diagnostics.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseBenchmarkSummary {
    pub external_declarations: usize,
    pub diagnostics:           usize,
}
