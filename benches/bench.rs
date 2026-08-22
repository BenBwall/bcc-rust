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
    group.throughput(Throughput::ElementsAndBytes {
        elements: 1_000_000,
        bytes:    bcc_rust::one_million_input_bytes(),
    });
    _ = group.bench_function("one million lines", |b| {
        b.iter(bcc_rust::preprocess_one_million);
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
