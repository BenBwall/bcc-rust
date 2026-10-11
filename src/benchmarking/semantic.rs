//! Runs semantic analysis and summarizes results for benchmark inputs.

use std::path::Path;

use super::{
    BenchmarkInput,
    ParseBenchmarkSummary,
    input::benchmark_context,
};
use crate::util::bump::Bump;

/// Runs the full pipeline including declaration semantic analysis.
#[doc(hidden)]
#[must_use]
pub fn sema(input: BenchmarkInput) -> ParseBenchmarkSummary {
    let tu = Bump::new();
    summarize_semantic(&tu, input.source())
}

/// Runs declaration semantic analysis over an arena copy of source.
#[doc(hidden)]
#[must_use]
pub fn sema_source(source: &str) -> ParseBenchmarkSummary {
    let tu = Bump::new();
    let source = tu.alloc_str(source);
    summarize_semantic(&tu, source)
}

pub(super) fn summarize_semantic<'tu>(tu: &'tu Bump, source: &'tu str) -> ParseBenchmarkSummary {
    let mut context = benchmark_context(tu);
    let unit = crate::pipeline::parse_translation_unit(
        &mut context,
        Path::new("<input>"),
        source,
        crate::headers::HeaderSearch::default(),
    );
    let _semantic = crate::pipeline::analyze_translation_unit(&mut context, &unit);
    ParseBenchmarkSummary {
        external_declarations: unit.external_declarations().len(),
        diagnostics:           context.pending_error_count(),
    }
}
