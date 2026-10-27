#![expect(
    missing_docs,
    reason = "We don't have a doc string here because it's obvious what our main module does."
)]
#![expect(
    unused_crate_dependencies,
    reason = "We have a bunch of dependencies that are not used in our main module."
)]
#[cfg(not(feature = "benchmarking-internals"))]
use std::io::Write as _;
#[cfg(not(feature = "benchmarking-internals"))]
use std::process::ExitCode;

#[cfg(not(feature = "benchmarking-internals"))]
fn main() -> ExitCode {
    match bcc_rust::run() {
        | Ok(0) => ExitCode::SUCCESS,
        // `ExitCode` holds only a byte; an interpreted program's status is
        // an `int`. `exit` hands it to the OS whole, so a POSIX parent sees
        // its low byte and Windows all 32 bits, as for a native build. A
        // driver personality that reported its own error exits with the
        // status it chose.
        | Ok(status) | Err(bcc_rust::MainError::Reported(status)) => std::process::exit(status),
        | Err(bcc_rust::MainError::ParseArgumentsError(error)) => error.exit(),
        | Err(error) => {
            drop(writeln!(std::io::stderr().lock(), "{error}"));
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
