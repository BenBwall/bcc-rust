#![expect(
    missing_docs,
    reason = "We don't have a doc string here because it's obvious what our main module does."
)]
#![expect(
    unused_crate_dependencies,
    reason = "We have a bunch of dependencies that are not used in our main module."
)]
#![expect(
    clippy::cargo_common_metadata,
    reason = "We don't have any metadata in our main module, because it's not getting published."
)]

use std::process::ExitCode;

#[cfg(not(feature = "benchmarking-internals"))]
fn main() -> ExitCode {
    if let Err(e) = bcc_rust::run() {
        eprintln!("{e}");
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(feature = "benchmarking-internals")]
fn main() {
    for _ in 0..100_000_000_000_000i64 {
        bcc_rust::preprocess_one_million();
        coz::progress!("LOOOP!!");
    }
}
