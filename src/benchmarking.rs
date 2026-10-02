//! Entry points for the Criterion and coz benchmarks.

use crate::{
    pipeline::PreprocessorIterator,
    translation_phases::{
        Context,
        box_path_from_str,
        parsing::Parser as LanguageParser,
        preprocessing::Preprocessor,
    },
    util::shared::SharedVec,
};

#[doc(hidden)]
pub fn preprocess_one_million() -> usize {
    let million_lines = one_million_lines();
    let iterator = PreprocessorIterator::new(
        box_path_from_str("<input>"),
        million_lines.to_owned().into(),
        SharedVec::default(),
        SharedVec::default(),
    );
    iterator.count()
}

#[doc(hidden)]
pub fn one_million_input_bytes() -> u64 {
    u64::try_from(one_million_lines().len()).expect("benchmark input length must fit in u64")
}

fn one_million_lines() -> &'static str {
    include_str!(concat!(env!("OUT_DIR"), "/one-million-lines.c"))
}

#[expect(
    clippy::large_include_file,
    reason = "The generated parser benchmark input is intentionally large."
)]
fn parser_mix() -> &'static str {
    include_str!(concat!(env!("OUT_DIR"), "/parser-mix.c"))
}

/// Summary of one benchmarked parse, returned so the work cannot be elided
/// and so benchmark setup can reject inputs that produce diagnostics.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseBenchmarkSummary {
    pub external_declarations: usize,
    pub diagnostics:           usize,
}

fn parse_benchmark_input(input: &'static str) -> ParseBenchmarkSummary {
    let mut context = Context::new();
    let preprocessor = Preprocessor::new(
        &mut context,
        box_path_from_str("<input>"),
        input.to_owned().into(),
        SharedVec::default(),
        SharedVec::default(),
    );
    let unit = LanguageParser::new(preprocessor).parse_translation_unit(&mut context);
    ParseBenchmarkSummary {
        external_declarations: unit.external_declarations().len(),
        diagnostics:           context.take_pending_errors().len(),
    }
}

#[doc(hidden)]
#[must_use]
pub fn parse_one_million() -> ParseBenchmarkSummary {
    parse_benchmark_input(one_million_lines())
}

#[doc(hidden)]
#[must_use]
pub fn parse_mix() -> ParseBenchmarkSummary {
    parse_benchmark_input(parser_mix())
}

#[doc(hidden)]
#[must_use]
pub fn parser_mix_input_bytes() -> u64 {
    u64::try_from(parser_mix().len()).expect("benchmark input length must fit in u64")
}

#[doc(hidden)]
#[must_use]
pub fn parser_mix_input_lines() -> u64 {
    u64::try_from(parser_mix().lines().count()).expect("benchmark line count must fit in u64")
}
