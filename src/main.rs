#![expect(
    missing_docs,
    reason = "We don't have a doc string here because it's obvious what our main module does."
)]
#![expect(
    unused_crate_dependencies,
    reason = "We have a bunch of dependencies that are not used in our main module."
)]
#[cfg(not(feature = "benchmarking-internals"))]
use std::process::ExitCode;

#[cfg(not(feature = "benchmarking-internals"))]
fn main() -> ExitCode {
    match bcc_rust::run() {
        | Ok(0) => ExitCode::SUCCESS,
        // `ExitCode` holds only a byte; an interpreted program's status is
        // an `int`.
        | Ok(status) => std::process::exit(status),
        | Err(bcc_rust::MainError::ParseArgumentsError(error)) => error.exit(),
        | Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        },
    }
}

#[cfg(feature = "benchmarking-internals")]
fn main() {
    for _ in 0..100_000_000_000_000i64 {
        _ = bcc_rust::preprocess_one_million();
        #[cfg(unix)]
        coz::progress!("LOOOP!!");
    }
}
