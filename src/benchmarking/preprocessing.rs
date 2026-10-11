//! Benchmark entry points count lexer and preprocessor output tokens.

use super::{
    BenchmarkInput,
    Bump,
    Path,
    TokenSource,
    TranslationPhase,
    benchmark_context,
    with_preprocessor,
};

/// Runs translation phases 1 through 6 over the whole translation unit and
/// returns the number of parser-facing tokens. Their provenance is kept, as
/// the parser reading them would need.
#[doc(hidden)]
#[must_use]
pub fn preprocess(input: BenchmarkInput) -> usize {
    let tu = Bump::new();
    let mut context = benchmark_context(&tu);
    with_preprocessor(
        &mut context,
        Path::new("<input>"),
        input.source(),
        crate::headers::HeaderSearch::default(),
        |mut preprocessor, context, _pp| {
            let mut tokens = crate::util::region_vec::RegionVec::new();
            let _ = preprocessor.preprocess_into_arena(context, usize::MAX, &mut tokens);
            tokens.len()
        },
    )
}

/// Runs translation phases 1 through 3 only and returns the number of
/// preprocessing tokens.
#[doc(hidden)]
#[must_use]
pub fn lex(input: BenchmarkInput) -> usize {
    let tu = Bump::new();
    let mut context = benchmark_context(&tu);
    let file = context.intern_source_file(Path::new("<input>"));
    let pp = Bump::new();
    let mut tokens = TokenSource::new(&mut context, &pp, file, input.source());
    let mut count = 0;
    while tokens.next_item(&mut context).is_some() {
        count += 1;
        // Lexing alone never compacts provenance, so keep the arena from
        // dominating the measurement.
        context.source_vectors.0.clear();
    }
    count
}

#[doc(hidden)]
#[must_use]
pub fn preprocess_one_million() -> usize {
    preprocess(BenchmarkInput::OneMillionLines)
}
