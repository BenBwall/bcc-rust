//! ABI lowering tests: eightbyte classification and Win64 sizes
//! (`classify.rs`), signatures, callers and callees rewritten from textual
//! IR (`rewrite.rs`), `va_arg` expansion (`varargs.rs`), C programs run in
//! the interpreter before and after ABI lowering (`programs.rs`), and
//! conformance with Clang-compiled code through the LLVM back end
//! (`conformance.rs`).

mod classify;
mod conformance;
pub(crate) mod programs;
mod rewrite;
mod varargs;

use super::*;
use crate::ir::{
    parse_module,
    verify_module,
};

/// Parses and verifies the pre-ABI `text`, lowers it for `target`, verifies
/// the result as post-ABI and checks that its text round-trips. Returns the
/// lowered module's text.
#[track_caller]
fn lower_text(text: &str, target: Target) -> String {
    let arena = Bump::new();
    let mut module = parse_module(&arena, text).unwrap_or_else(|error| panic!("{error}\n{text}"));
    assert_verified(&module, Profile::PreAbi, text);
    let scratch = Bump::new();
    lower_abi(&mut module, target, &scratch).unwrap_or_else(|error| panic!("{error}\n{text}"));
    let lowered = module.to_string();
    assert_verified(&module, Profile::PostAbi, &lowered);
    let reparsed = Bump::new();
    let parsed =
        parse_module(&reparsed, &lowered).unwrap_or_else(|error| panic!("{error}\n{lowered}"));
    pretty_assertions::assert_eq!(parsed.to_string(), lowered, "the lowered IR round-trips");
    lowered
}

/// Panics with the verifier's messages if `module` does not verify for
/// `profile`.
#[track_caller]
fn assert_verified(module: &Module<'_>, profile: Profile, text: &str) {
    let scratch = Bump::new();
    let errors: Vec<String> = verify_module(module, profile, &scratch)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(errors.is_empty(), "{errors:#?}\n{text}");
}

/// The lowered signature of each function of `text`, one per line, as
/// `@name(params) -> result`.
#[track_caller]
fn lowered_signatures(text: &str, target: Target) -> String {
    let lowered = lower_text(text, target);
    lowered
        .lines()
        .filter_map(|line| line.strip_prefix("function "))
        .map(|line| {
            let end = line.rfind(" external").or_else(|| line.rfind(" internal"));
            line[..end.unwrap_or(line.len())].to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
