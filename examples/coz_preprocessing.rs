//! Preprocessing workload for the `coz` causal profiler.
//!
//! Preprocesses the generated one-million-line benchmark input `ITERATIONS`
//! times (by default until it is stopped) and marks a Unix `coz` progress
//! point after each pass, so the profile measures preprocessing, not parser
//! time:
//!
//! ```sh
//! cargo build --release --features benchmarking-internals --example coz_preprocessing
//! coz run --- target/release/examples/coz_preprocessing 100
//! ```
#![expect(
    unused_crate_dependencies,
    reason = "The workload uses the library's benchmark API and coz."
)]
#![expect(
    clippy::disallowed_methods,
    reason = "The harness reads its iteration count outside the measured workload."
)]

fn main() {
    let iterations = std::env::args().nth(1).map_or(u64::MAX, |count| {
        count
            .parse()
            .unwrap_or_else(|_| panic!("usage: coz_preprocessing [ITERATIONS], not {count:?}"))
    });
    for _ in 0..iterations {
        _ = std::hint::black_box(bcc_rust::preprocess_one_million());
        #[cfg(unix)]
        coz::progress!("preprocess_one_million");
    }
}
