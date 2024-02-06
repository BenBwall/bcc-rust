use criterion::{
    criterion_group,
    criterion_main,
    Criterion,
};

fn bench(c: &mut Criterion) {
    c.bench_function("Preprocess hundred thousand", |b| {
        b.iter(bcc_rust::preprocess_hundred_thousand)
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
