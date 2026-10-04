#![expect(
    missing_docs,
    reason = "We don't have a doc string here because it's obvious what our benchmarking module \
              does."
)]
#![expect(
    unused_crate_dependencies,
    reason = "We have a bunch of dependencies that are not used in our benchmarking module."
)]

use bcc_rust::BenchmarkInput;
use criterion::{
    BenchmarkId,
    Criterion,
    Throughput,
    criterion_group,
    criterion_main,
};

/// The benchmark identifier's function name. The pipeline once had several
/// scheduling strategies; keeping the batch name keeps results comparable.
const PIPELINE: &str = "batch";

fn throughput(input: BenchmarkInput) -> Throughput {
    Throughput::ElementsAndBytes {
        elements: input.lines(),
        bytes:    input.bytes(),
    }
}

/// Translation phases 1-3.
fn bench_lexer(c: &mut Criterion) {
    let mut group = c.benchmark_group("Lexer");
    for input in BenchmarkInput::ALL {
        _ = group.throughput(throughput(input));
        _ = group.bench_with_input(
            BenchmarkId::new(PIPELINE, input.name()),
            &input,
            |b, &input| {
                b.iter(|| bcc_rust::lex(input));
            },
        );
    }
    group.finish();
}

/// Translation phases 1-6.
fn bench_preprocessor(c: &mut Criterion) {
    let mut group = c.benchmark_group("Preprocessor");
    _ = group.sample_size(20);
    for input in BenchmarkInput::ALL {
        _ = group.throughput(throughput(input));
        _ = group.bench_with_input(
            BenchmarkId::new(PIPELINE, input.name()),
            &input,
            |b, &input| {
                b.iter(|| bcc_rust::preprocess(input));
            },
        );
    }
    group.finish();
}

/// Translation phases 1-7.
fn bench_parser(c: &mut Criterion) {
    let mut group = c.benchmark_group("Parser");
    _ = group.sample_size(20);
    for input in BenchmarkInput::ALL {
        _ = group.throughput(throughput(input));
        let summary = bcc_rust::parse(input);
        assert_eq!(
            summary.diagnostics,
            0,
            "{} must parse cleanly",
            input.name()
        );
        _ = group.bench_with_input(
            BenchmarkId::new(PIPELINE, input.name()),
            &input,
            |b, &input| {
                b.iter(|| bcc_rust::parse(input));
            },
        );
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
        let summary = bcc_rust::parse(input);
        assert_eq!(
            summary.diagnostics,
            0,
            "{} must parse cleanly",
            input.name()
        );
        _ = group.bench_with_input(
            BenchmarkId::new(PIPELINE, input.name()),
            &input,
            |b, &input| {
                b.iter_custom(|iterations| {
                    let mut elapsed = std::time::Duration::ZERO;
                    for _ in 0..iterations {
                        bcc_rust::with_prepared_parse(input, |prepared| {
                            let start = std::time::Instant::now();
                            _ = std::hint::black_box(prepared.parse());
                            elapsed += start.elapsed();
                        });
                    }
                    elapsed
                });
            },
        );
    }
    group.finish();
}

fn bench_preprocessor_allocations(c: &mut Criterion) {
    let mut group = c.benchmark_group("Preprocessor allocations");
    _ = group.sample_size(30);
    for input in BenchmarkInput::PREPROCESSOR_STRESS {
        _ = group.throughput(throughput(input));
        _ = group.bench_with_input(
            BenchmarkId::new(PIPELINE, input.name()),
            &input,
            |b, &input| b.iter(|| bcc_rust::preprocess(input)),
        );
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
