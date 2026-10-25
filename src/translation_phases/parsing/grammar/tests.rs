//! Grammar checks for native recursion depth, the nesting-depth limit, and
//! scratch reuse.

use std::path::PathBuf;

use crate::{
    diagnostics::{
        ColorChoice,
        Renderer,
        ToDiagnostic,
    },
    translation_phases::{
        Context,
        GetSourceVectors,
        parsing::{
            InspectionOptions,
            Parser,
            limits::ParserLimits,
        },
        preprocessing::Preprocessor,
    },
    util::{
        bump::Bump,
        shared::SharedVec,
    },
};

#[test]
fn deep_parenthesized_declarators_recurse_natively() {
    let depth = 512;
    let source = format!(
        "int {}x{}; int {}(*y)(void){};",
        "(".repeat(depth),
        ")".repeat(depth),
        "(*".repeat(depth),
        ")".repeat(depth)
    );
    let (text, max_depth) = inspect(&source, ParserLimits::default());
    assert!(!text.contains("error"), "{text}");
    assert!(
        max_depth > depth,
        "each parenthesized declarator is a recursive entry"
    );
}

#[test]
fn limited_depth_returns_one_terminal_error_root() {
    let (text, max_depth) = inspect(
        "int f(void) { return ((((1)))); } int after;",
        ParserLimits {
            nesting_depth: 3,
            ..ParserLimits::default()
        },
    );
    assert_eq!(
        text.matches("nesting depth limit").count(),
        1,
        "depth limit is reported once"
    );
    assert_eq!(max_depth, 3, "limited recursion respects the threshold");
    assert!(
        text.contains("root[0] error"),
        "resource exhaustion retains an error root"
    );
    assert!(
        !text.contains("root[1]"),
        "terminal exhaustion abandons the remaining translation unit"
    );
}

#[test]
fn calls_and_compounds_reuse_scratch_between_functions() {
    let function = "int f(void) { g(1, 2, 3); { g(4); } }\n";
    let one = scratch_bytes(function);
    let repeated = scratch_bytes(&function.repeat(128));
    assert!(
        repeated <= one * 2,
        "scratch grew from {one} to {repeated}; function-local collections must return to their \
         pools"
    );
}

fn scratch_bytes(source: &str) -> usize {
    let tu = Bump::new();
    let pp = Bump::new();
    let parse = Bump::new();
    let mut context = Context::new(&tu);
    let preprocessor = Preprocessor::new(
        &pp,
        &mut context,
        PathBuf::from("<grammar-pool-test>").into_boxed_path(),
        source,
        SharedVec::default(),
        SharedVec::default(),
    );
    let _unit = Parser::new(preprocessor, &mut context, &parse).parse_translation_unit();
    assert_eq!(
        context.pending_error_count(),
        0,
        "pool workload parses without diagnostics"
    );
    parse.high_water()
}

/// Parses `source` under `limits` and returns its located inspection with
/// rendered diagnostics, and the deepest production nesting reached.
fn inspect(source: &str, limits: ParserLimits) -> (String, usize) {
    let tu = Bump::new();
    let pp = Bump::new();
    let parse = Bump::new();
    let mut context = Context::new(&tu);
    let preprocessor = Preprocessor::new(
        &pp,
        &mut context,
        PathBuf::from("<grammar-test>").into_boxed_path(),
        source,
        SharedVec::default(),
        SharedVec::default(),
    );
    let mut parser = Parser::new_with_limits(preprocessor, &mut context, limits, &parse);
    while parser.next_item().is_some() {}
    let max_depth = parser.max_depth;
    // The roots already streamed stay part of the aggregate result.
    let unit = parser.parse_translation_unit();
    let mut text = unit
        .inspect(
            &tu,
            &context,
            InspectionOptions {
                show_locations: true,
            },
        )
        .to_owned();
    let mut renderer = Renderer::new(ColorChoice::Plain);
    while let Some(error) = context.pop_pending_error() {
        let source = error.source_vectors(&mut context);
        let diagnostic = error.diagnostic_in(&context, source, &tu);
        text.push_str(renderer.render_text(&diagnostic, &context));
    }
    (text, max_depth)
}
