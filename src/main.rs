//! Binary for the bcc-rust crate.
use std::process::ExitCode;

fn main() -> ExitCode {
    // if let Err(e) = bcc_rust::run() {
    // eprintln!("{e}");
    // ExitCode::FAILURE
    // } else {
    // ExitCode::SUCCESS
    // }
    if let Err(e) = bcc_rust::run() {
        eprintln!("{e}");
        return ExitCode::FAILURE;
    };
    ExitCode::SUCCESS
}
