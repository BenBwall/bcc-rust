//! Tests for the optimizer: each pass on textual IR, then the pipeline.
//!
//! A test parses IR text, verifies it, runs the optimizer, verifies the
//! result, and compares the printed result with the expected text. After a
//! pass, the printed values are numbered from 0 in listing order.

mod dce;
mod fold;
mod gvn;
mod options;
mod pipeline;
mod simplify_cfg;

use super::*;
use crate::ir::{
    Module,
    parse_module,
    verify_module,
};

/// Parses `text`, panicking with the error if it does not parse.
fn parse<'ir>(arena: &'ir Bump, text: &str) -> Module<'ir> {
    parse_module(arena, text).unwrap_or_else(|error| panic!("{error}\n{text}"))
}

/// Panics with the verifier's messages if `module` does not verify.
fn assert_verifies(module: &Module<'_>, context: &str) {
    let scratch = Bump::new();
    let errors: Vec<String> = verify_module(module, Profile::PreAbi, &scratch)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(errors.is_empty(), "{context}: {errors:?}\n{module}");
}

/// Optimizes the module `text` with `options` and returns its text and the
/// report.
fn optimize_text(text: &str, options: &OptimizerOptions<'_>) -> (String, OptimizationReport) {
    let arena = Bump::new();
    let scratch = Bump::new();
    let mut module = parse(&arena, text);
    assert_verifies(&module, "the input");
    let report = optimize(&mut module, options, &scratch);
    assert_verifies(&module, "the output");
    (module.to_string(), report)
}

/// The options that run `passes` once each.
fn only(passes: &[Pass]) -> OptimizerOptions<'static> {
    OptimizerOptions {
        passes: PassList::new(passes),
        verify_each: true,
        ..OptimizerOptions::default()
    }
}

/// Runs `pass` once over `input` and checks the printed result.
fn assert_pass(pass: Pass, input: &str, expected: &str) {
    let (actual, _) = optimize_text(input, &only(&[pass]));
    pretty_assertions::assert_eq!(actual, expected);
}

/// Runs `pass` once and checks that the module is unchanged.
fn assert_pass_leaves(pass: Pass, input: &str) {
    assert_pass(pass, input, input);
}

/// Runs the default pipeline over `input` and checks the printed result.
fn assert_pipeline(input: &str, expected: &str) {
    let options = OptimizerOptions {
        verify_each: true,
        ..OptimizerOptions::default()
    };
    let (actual, _) = optimize_text(input, &options);
    pretty_assertions::assert_eq!(actual, expected);
}
