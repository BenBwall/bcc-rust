#![expect(missing_docs, reason = "We don't have a doc string here because it's obvious what our benchmarking module does.")]
#![expect(unused_crate_dependencies, reason = "We have a bunch of dependencies that are not used in our benchmarking module.")]
#![expect(clippy::cargo_common_metadata, reason = "We don't have any metadata in our benchmarking module, because it's not getting published.")]

use criterion::{
    criterion_group,
    criterion_main,
    Criterion,
};

fn bench(c: &mut Criterion) {
    _ = c.bench_function("Preprocess one million", |b| {
        b.iter(bcc_rust::preprocess_one_million);
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
