//! Tests for the LLVM back end: golden LLVM IR for textual bcc IR.

mod golden;

use super::*;
use crate::ir::{
    Profile,
    Type,
    parse_module,
    verify_module,
};

/// Parses and verifies `text`, then prints it as LLVM IR for `target`.
fn emit(text: &str, profile: Profile, target: Target) -> String {
    let arena = Bump::new();
    let module = parse_module(&arena, text).unwrap_or_else(|error| panic!("{error}\n{text}"));
    let scratch = Bump::new();
    let errors: Vec<String> = verify_module(&module, profile, &scratch)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(errors.is_empty(), "{errors:#?}\n{text}");
    let mut out = String::new();
    emit_module(&module, target, &mut out).unwrap();
    out
}
