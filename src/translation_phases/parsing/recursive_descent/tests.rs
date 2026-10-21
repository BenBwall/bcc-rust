//! Differential grammar checks for the opt-in parser study.

use std::path::PathBuf;

use super::{
    StudyMode,
    StudyStats,
};
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
        },
        preprocessing::Preprocessor,
    },
    util::{
        bump::Bump,
        shared::SharedVec,
    },
};

#[test]
fn expression_precedence_and_statement_forms_match_machine() {
    let sources = [
        "int f(void) { return a + b * c << 2 < d == e & f ^ g | h && i || j ? k, l : m ? n : o; }",
        "int f(void) { a = b = c += d *= e; a -= b; a /= b; a %= b; a <<= b; a >>= b; a &= b; a \
         ^= b; a |= b; return a, b, c; }",
        "int f(void) { factory()(x, y + z)[i].field->member++; --x; ++*p; return sizeof x + \
         sizeof(int) + (long)(int)x; }",
        "typedef int T; int f(void) { T x = 1; { int T; T = (x); } return (T)x; }",
        "typedef int T; int f(void) { label: T x = 1; last: }",
        "int f(void) { if (a) if (b) x(); else y(); else z(); while (a) { continue; } do { break; \
         } while (b); for (int i = 0; i < 3; ++i) x(i); for (;;) ; return 0; }",
        "int f(void) { label: switch (x) { case 1: x++; break; case 2: case 3: ; default: goto \
         label; } return; }",
        "struct S { int x; }; int f(void) { return ((struct S){ .x = 2 }).x + sizeof (int [2]) { \
         1, 2 }[0]; }",
        "enum E { A = 1 + 2 * 3 }; struct S { unsigned a : 1 + 2; }; int values[2 + 3] = { [1 + \
         1] = 4 };",
        "#define SUM(x, y) ((x) + (y))\nint f(void) { return SUM(1, 2) * SUM(3, 4); }",
        "int f(void) { return __builtin_choose_expr(1, (int) 2, 3) + ({ int x = 2; x; }); }",
    ];
    for source in sources {
        let (expected, _) = inspect(source, StudyMode::Machine, false);
        for mode in [
            StudyMode::Recursive,
            StudyMode::Growing,
            StudyMode::Hybrid { depth: 0 },
            StudyMode::Hybrid { depth: 1 },
            StudyMode::Hybrid { depth: 3 },
            StudyMode::Hybrid { depth: 8 },
        ] {
            let (actual, stats) = inspect(source, mode, false);
            assert_eq!(
                actual, expected,
                "differential mismatch for {mode:?}: {source}"
            );
            assert_eq!(
                stats.native_stack_span, 0,
                "timing mode does not sample stack addresses"
            );
            if let StudyMode::Hybrid { depth } = mode {
                assert!(
                    stats.max_depth <= depth,
                    "hybrid bounds recursive grammar entries"
                );
            }
        }
    }
}

#[test]
fn hybrid_fallback_preserves_binding_power_at_every_threshold() {
    let source = "int f(void) { return a = b ? c, d + e * f : g ? h : i, j + k * l; }";
    let (expected, _) = inspect(source, StudyMode::Machine, false);
    for depth in 0..16 {
        let (actual, _) = inspect(source, StudyMode::Hybrid { depth }, false);
        assert_eq!(actual, expected, "hybrid threshold {depth}");
    }
}

#[test]
fn hybrid_handles_deep_parentheses_without_native_recursion() {
    let source = format!(
        "int f(void) {{ return {}x{}; }}",
        "(".repeat(512),
        ")".repeat(512)
    );
    let (expected, _) = inspect(&source, StudyMode::Machine, false);
    let (actual, stats) = inspect(&source, StudyMode::Hybrid { depth: 4 }, true);
    assert_eq!(actual, expected);
    assert_eq!(stats.max_depth, 4);
    assert!(
        stats.fallback_count > 0,
        "deep subtree uses the frame machine"
    );
    assert!(
        stats.native_stack_span > 0,
        "explicit sampling records a native stack span"
    );
}

#[test]
fn limited_depth_returns_one_terminal_error_root() {
    let (text, stats) = inspect(
        "int f(void) { return ((((1)))); } int after;",
        StudyMode::Limited { depth: 3 },
        false,
    );
    assert_eq!(stats.depth_limit_hits, 1, "depth limit is reported once");
    assert_eq!(
        stats.max_depth, 3,
        "limited recursion respects the threshold"
    );
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
    for mode in [
        StudyMode::Machine,
        StudyMode::Recursive,
        StudyMode::Growing,
        StudyMode::Hybrid { depth: 4 },
    ] {
        let one = scratch_bytes(function, mode);
        let repeated = scratch_bytes(&function.repeat(128), mode);
        assert!(
            repeated <= one * 2,
            "{mode:?} scratch grew from {one} to {repeated}; function-local collections must \
             return to their pools"
        );
    }
}

fn scratch_bytes(source: &str, mode: StudyMode) -> usize {
    let tu = Bump::new();
    let pp = Bump::new();
    let parse = Bump::new();
    let mut context = Context::new(&tu);
    let preprocessor = Preprocessor::new(
        &pp,
        &mut context,
        PathBuf::from("<recursive-study-pool-test>").into_boxed_path(),
        source,
        SharedVec::default(),
        SharedVec::default(),
    );
    let (_unit, _stats) = Parser::new(preprocessor, &mut context, &parse)
        .with_study_mode(mode)
        .parse_study_translation_unit();
    assert_eq!(
        context.pending_error_count(),
        0,
        "pool workload parses without diagnostics"
    );
    parse.high_water()
}

fn inspect(source: &str, mode: StudyMode, stack_samples: bool) -> (String, StudyStats) {
    let tu = Bump::new();
    let pp = Bump::new();
    let parse = Bump::new();
    let mut context = Context::new(&tu);
    let preprocessor = Preprocessor::new(
        &pp,
        &mut context,
        PathBuf::from("<recursive-study-test>").into_boxed_path(),
        source,
        SharedVec::default(),
        SharedVec::default(),
    );
    let (unit, stats) = Parser::new(preprocessor, &mut context, &parse)
        .with_study_mode(mode)
        .with_study_stack_samples(stack_samples)
        .parse_study_translation_unit();
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
    for error in context.pending_errors.iter() {
        let diagnostic = error.diagnostic_in(&context, error.source_vectors(), &tu);
        text.push_str(renderer.render_text(&diagnostic, &context));
    }
    (text, stats)
}
