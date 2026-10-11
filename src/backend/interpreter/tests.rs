//! Tests for the interpreter, written as textual IR: arithmetic, poison,
//! control flow and calls, memory, undefined behaviour, varargs and the
//! default host.

mod arithmetic;
mod calls;
mod control;
mod host;
mod memory;
mod poison;

use std::fmt::Write as _;

use super::*;

/// Runs `entry` of `text` and returns what it returned, panicking if it
/// trapped or ended otherwise.
fn returned(text: &str, entry: &str) -> RuntimeValue {
    returned_with(text, entry, &[])
}

/// [`returned`] with arguments.
fn returned_with(text: &str, entry: &str, args: &[RuntimeValue]) -> RuntimeValue {
    let run = run_text_with(text, entry, args, Limits::default());
    match run.result {
        | Ok(Outcome {
            termination: Termination::Returned(Some(value)),
            ..
        }) => value,
        | other => panic!("{other:?} {:?}\n{text}", run.message),
    }
}

/// Runs `entry` of `text` and returns the undefined behaviour it trapped on.
fn undefined(text: &str, entry: &str) -> UbKind {
    let run = run_text(text, entry);
    match run.result {
        | Err(Trap::UndefinedBehavior(kind, _)) => kind,
        | other => panic!("expected undefined behaviour, got {other:?}\n{text}"),
    }
}

/// An `i32` result.
fn int(value: u128) -> RuntimeValue {
    RuntimeValue::Int(value)
}

/// A null-terminated C string as a constant global named `name`.
fn c_string(name: &str, text: &str) -> String {
    let mut hex = String::new();
    for byte in text.bytes().chain([0]) {
        write!(hex, "{byte:02x}").unwrap();
    }
    format!(
        "global @{name} internal constant size {}, align 1 = bytes \"{hex}\"\n",
        text.len() + 1
    )
}
