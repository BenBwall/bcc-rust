#![expect(
    missing_docs,
    reason = "We don't have a doc string here because it's obvious what our benchmarking module \
              does."
)]
#![expect(
    unused_crate_dependencies,
    reason = "We have a bunch of dependencies that are not used in our benchmarking module."
)]

use bcc_rust::{
    BenchmarkInput,
    PreprocessingStrategy,
};
use criterion::{
    BatchSize,
    BenchmarkId,
    Criterion,
    Throughput,
    criterion_group,
    criterion_main,
};

const STRATEGIES: [(&str, PreprocessingStrategy); 3] = [
    ("streaming", PreprocessingStrategy::Streaming),
    ("batch lexing", PreprocessingStrategy::BatchLexing),
    ("batch", PreprocessingStrategy::Batch),
];

fn throughput(input: BenchmarkInput) -> Throughput {
    Throughput::ElementsAndBytes {
        elements: input.lines(),
        bytes:    input.bytes(),
    }
}

/// Translation phases 1-3 only. "batch lexing" and "batch" lex identically
/// here, so only one batch variant is measured.
fn bench_lexer(c: &mut Criterion) {
    let mut group = c.benchmark_group("Lexer");
    for input in BenchmarkInput::ALL {
        _ = group.throughput(throughput(input));
        for (name, strategy) in &STRATEGIES[..2] {
            _ = group.bench_with_input(
                BenchmarkId::new(*name, input.name()),
                &input,
                |b, &input| {
                    b.iter(|| bcc_rust::lex(input, *strategy));
                },
            );
        }
    }
    group.finish();
}

/// Translation phases 1-6.
fn bench_preprocessor(c: &mut Criterion) {
    let mut group = c.benchmark_group("Preprocessor");
    _ = group.sample_size(20);
    for input in BenchmarkInput::ALL {
        let expected = bcc_rust::preprocess(input, PreprocessingStrategy::Streaming);
        _ = group.throughput(throughput(input));
        for (name, strategy) in STRATEGIES {
            assert_eq!(
                bcc_rust::preprocess(input, strategy),
                expected,
                "every strategy must yield the same tokens"
            );
            _ = group.bench_with_input(
                BenchmarkId::new(name, input.name()),
                &input,
                |b, &input| {
                    b.iter(|| bcc_rust::preprocess(input, strategy));
                },
            );
        }
    }
    group.finish();
}

/// Translation phases 1-7.
fn bench_parser(c: &mut Criterion) {
    let mut group = c.benchmark_group("Parser");
    _ = group.sample_size(20);
    for input in BenchmarkInput::ALL {
        _ = group.throughput(throughput(input));
        for (name, strategy) in STRATEGIES {
            let summary = bcc_rust::parse(input, strategy);
            assert_eq!(
                summary.diagnostics,
                0,
                "{} must parse cleanly",
                input.name()
            );
            _ = group.bench_with_input(
                BenchmarkId::new(name, input.name()),
                &input,
                |b, &input| {
                    b.iter(|| bcc_rust::parse(input, strategy));
                },
            );
        }
    }
    group.finish();
}

/// Translation phase 7 alone: each input is preprocessed in untimed setup.
fn bench_parser_only(c: &mut Criterion) {
    let mut group = c.benchmark_group("Parser only");
    _ = group.sample_size(20);
    for input in BenchmarkInput::ALL
        .into_iter()
        .chain(BenchmarkInput::PARSER_STRESS)
    {
        _ = group.throughput(throughput(input));
        let summary = bcc_rust::prepare_parse(input).parse();
        assert_eq!(
            summary.diagnostics,
            0,
            "{} must parse cleanly",
            input.name()
        );
        _ = group.bench_with_input(
            BenchmarkId::new("batch", input.name()),
            &input,
            |b, &input| {
                b.iter_batched(
                    || bcc_rust::prepare_parse(input),
                    bcc_rust::PreparedParse::parse,
                    BatchSize::PerIteration,
                );
            },
        );
    }
    group.finish();
}

fn bench_preprocessor_allocations(c: &mut Criterion) {
    let mut group = c.benchmark_group("Preprocessor allocations");
    _ = group.sample_size(30);
    for input in BenchmarkInput::PREPROCESSOR_STRESS {
        let expected = bcc_rust::preprocess(input, PreprocessingStrategy::Streaming);
        _ = group.throughput(throughput(input));
        for (name, strategy) in STRATEGIES {
            assert_eq!(
                bcc_rust::preprocess(input, strategy),
                expected,
                "every strategy must yield the same tokens for {}",
                input.name()
            );
            _ = group.bench_with_input(
                BenchmarkId::new(name, input.name()),
                &input,
                |b, &input| b.iter(|| bcc_rust::preprocess(input, strategy)),
            );
        }
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_lexer,
    bench_preprocessor,
    bench_parser,
    bench_parser_only,
    bench_preprocessor_allocations
);
criterion_main!(benches);
