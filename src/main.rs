#![allow(unused_crate_dependencies)]
#![allow(clippy::cargo_common_metadata)]
#![allow(missing_docs)]

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
