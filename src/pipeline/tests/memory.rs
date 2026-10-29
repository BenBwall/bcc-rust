//! Working storage per output token of macro-heavy input. Each token and
//! each expression it forms may keep a bounded number of bytes through
//! semantic analysis; tables that grow with the input must not keep the
//! copies they outgrew.

use std::path::Path;

use crate::{
    headers::HeaderSearch,
    pipeline::{
        analyze_translation_unit,
        parse_translation_unit,
    },
    test_support::{
        committed_peak,
        macro_heavy_source,
    },
    translation_phases::Context,
    util::bump::Bump,
};

/// The bytes a compilation of `lines` macro-heavy declarations through
/// semantic analysis commits at its peak, and its output tokens.
fn analysis_peak(lines: usize) -> (usize, usize) {
    let source = macro_heavy_source(lines);
    committed_peak(|| {
        let tu = Bump::new();
        let mut context = Context::new(&tu);
        let source = tu.alloc_str(&source);
        let unit = parse_translation_unit(
            &mut context,
            Path::new("<input>"),
            source,
            HeaderSearch::default(),
        );
        let _semantic = analyze_translation_unit(&mut context, &unit);
        context.preprocessed_tokens
    })
}

/// Marginal commit per output token, between 400 and 800 lines: enough
/// that the per-expression tables pass the arena's large-block threshold,
/// and independent of the fixed cost of each region.
#[test]
fn macro_heavy_analysis_commits_a_bounded_amount_per_token() {
    let (small, small_tokens) = analysis_peak(400);
    let (large, large_tokens) = analysis_peak(800);
    let per_token = (large - small) / (large_tokens - small_tokens);
    assert!(
        per_token <= MAX_ANALYSIS_BYTES_PER_TOKEN,
        "{per_token} bytes per token ({small} for {small_tokens} tokens, {large} for \
         {large_tokens})"
    );
}

/// Bytes per token semantic analysis may commit for this input, most of them
/// in the expressions' retained records (160 bytes each) and the tables
/// keyed by expression. Growing the record table by copying within the
/// arena, which kept each outgrown copy, took 682 bytes per token; growing
/// it in its own region takes 548.
const MAX_ANALYSIS_BYTES_PER_TOKEN: usize = 615;
