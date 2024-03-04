#![allow(missing_docs)]
#![allow(unused_crate_dependencies)]
#![allow(clippy::cargo_common_metadata)]

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
