//! Binary for the bcc-rust crate.
use std::process::ExitCode;

fn main() -> ExitCode {
    /* 
    if let Err(e) = bcc_rust::run() {
        eprintln!("{e}");
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
    */
    bcc_rust::preprocess_hundred_thousand();
    ExitCode::SUCCESS
}
