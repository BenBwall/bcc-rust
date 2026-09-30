#![expect(
    missing_docs,
    reason = "We don't have a doc string here because it's obvious what our benchmarking module \
              does."
)]
#![expect(
    unused_crate_dependencies,
    reason = "We have a bunch of dependencies that are not used in our benchmarking module."
)]
#![expect(
    clippy::cargo_common_metadata,
    reason = "We don't have any metadata in our benchmarking module, because it's not getting \
              published."
)]

use criterion::{
    Criterion,
    Throughput,
    criterion_group,
    criterion_main,
};

fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("Preprocessor");
    _ = group.throughput(Throughput::ElementsAndBytes {
        elements: 1_000_000,
        bytes:    bcc_rust::one_million_input_bytes(),
    });
    _ = group.bench_function("one million lines", |b| {
        b.iter(bcc_rust::preprocess_one_million);
    });
    group.finish();
}

fn bench_parser(c: &mut Criterion) {
    let one_million = bcc_rust::parse_one_million();
    assert_eq!(
        one_million.diagnostics, 0,
        "one-million-line parser input must parse cleanly"
    );
    assert_eq!(
        one_million.external_declarations, 1_000_000,
        "one-million-line parser input must yield one root per line"
    );
    let mix = bcc_rust::parse_mix();
    assert_eq!(mix.diagnostics, 0, "mixed parser input must parse cleanly");

    let mut group = c.benchmark_group("Parser");
    _ = group.sample_size(20);
    _ = group.throughput(Throughput::ElementsAndBytes {
        elements: 1_000_000,
        bytes:    bcc_rust::one_million_input_bytes(),
    });
    _ = group.bench_function("one million lines", |b| {
        b.iter(bcc_rust::parse_one_million);
    });
    _ = group.throughput(Throughput::ElementsAndBytes {
        elements: bcc_rust::parser_mix_input_lines(),
        bytes:    bcc_rust::parser_mix_input_bytes(),
    });
    _ = group.bench_function("mixed C99 workload", |b| {
        b.iter(bcc_rust::parse_mix);
    });
    group.finish();
}

criterion_group!(benches, bench, bench_parser);
criterion_main!(benches);
