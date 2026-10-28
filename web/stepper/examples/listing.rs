//! Print a module's listing as the stepper shows it, with each
//! instruction's module offset and C source line:
//!
//! ```sh
//! cargo run --manifest-path web/Cargo.toml -p bcc-wasm-stepper --example listing -- web/wasm/recursion.wasm
//! ```

use std::process::ExitCode;

use wasm_stepper::Program;

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: listing <module.wasm>");
        return ExitCode::FAILURE;
    };
    let program = match std::fs::read(&path)
        .map_err(|error| error.to_string())
        .and_then(|bytes| Program::load(&bytes, None))
    {
        | Ok(program) => program,
        | Err(error) => {
            eprintln!("{path}: {error}");
            return ExitCode::FAILURE;
        },
    };
    for (text, line) in program.wat.text.lines().zip(&program.wat.lines) {
        match line.instr {
            | Some((func, pc, offset)) => {
                let source = program
                    .line_of(func, pc)
                    .map_or_else(|| "  ".to_owned(), |line| format!("{line:>2}"));
                println!("{offset:>6x} {source} | {text}");
            },
            | None => println!("          | {text}"),
        }
    }
    for import in &program.unresolved {
        println!("unresolved import: {import}");
    }
    ExitCode::SUCCESS
}
