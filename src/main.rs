#![allow(unused_crate_dependencies)]
#![allow(clippy::cargo_common_metadata)]
#![allow(missing_docs)]

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
