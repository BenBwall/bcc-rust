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
fn malformed_punctuation_and_assignment_flags_match_machine() {
    let sources = [
        "int f(void) { a + b = c; a = 1; return a; } int after;",
        "int f(void) { switch (a) { case 1 a++; default: break; } return a; } int after;",
        "int f(void) { a ? b; a = 1; return a; } int after;",
        "int f(void) { sizeof(int; a = 1; return a; } int after;",
        "int f(void) { a = (b + c; return a; } int after;",
        "int f(void) { a = p[b; return a; } int after;",
        "int f(void) { g(a, b; return a; } int after;",
        "int f(void) { return (a + b",
        "int f(void) { return a;",
    ];
    for source in sources {
        let (expected, _) = inspect(source, StudyMode::Machine, false);
        for mode in [
            StudyMode::Recursive,
            StudyMode::Growing,
            StudyMode::Hybrid { depth: 16 },
        ] {
            let (actual, _) = inspect(source, mode, false);
            assert_eq!(
                actual, expected,
                "malformed mismatch for {mode:?}: {source}"
            );
        }
    }
}

#[test]
fn recovery_preserves_delimiters_bodies_and_following_statements() {
    let bodies = [
        "a = (a b); a = 1;",
        "a = (b + c]; a = 1;",
        "call(a b, c); a = 1;",
        "if () a++; else a--;",
        "if (a b) { a++; } else a--;",
        "if (a) else a--;",
        "while (a b { a--; } a = 1;",
        "do a--; a = 1;",
        "for (a; b; c; a) a--;",
        "switch (a b) { default: a--; }",
        // Else ownership must survive intervening iterations and labels,
        // including a hybrid handoff while the then-body is still missing.
        "if (a) while (b) else a--;",
        "if (a) label: else a--;",
        "if (a) for (;;) else a--;",
        "if (a) do else a--;",
        "if (a) if (b) else a--; else b--;",
        "do a--; (a); a = 1;",
        "while () a--;",
        "switch () { default: a--; }",
        "do a--; while ();",
        "call(a b c); a = 1;",
    ];
    for body in bodies {
        let source =
            format!("typedef int T; int f(int a, int b, int c) {{ {body} return a; }} T after;");
        let (expected, _) = inspect(&source, StudyMode::Machine, false);
        for mode in [StudyMode::Recursive, StudyMode::Growing]
            .into_iter()
            .chain((0..=16).map(|depth| StudyMode::Hybrid { depth }))
        {
            let (actual, _) = inspect(&source, mode, false);
            assert_eq!(actual, expected, "recovery mismatch for {mode:?}: {body}");
        }
    }
}

#[test]
fn declarators_parameter_lists_and_type_names_match_machine() {
    let sources = [
        "int (((x))), *const *volatile p, (*(*f)(void))[3], a[static const 4], b[*];",
        "typedef int T; int g(T, int (*)(T x, ...), T [restrict], void (*)(), ...);",
        "int k(a, b, c) int a, b, c; { return sizeof(int (*)[2]) + (long (*)(void))0 != 0; }",
        "int v(...); int w(int, ...); int z(int x, int y[x]); struct S { int (*m)(int); };",
        "int f(void) { return (int (*[2])(int)) 0 == (void *) &(char [3]) { 1, 2 }; }",
        "int bad[3; int after;",
        "int (*missing(int; int after;",
        "int q[* x]; int r(int, ); int s(a b, c); int t(int ... x); int after;",
        "int *; int (); int (*)[2]; int after;",
        "int m(a, int b, ...); int n(int a b); int (x; int after;",
    ];
    for source in sources {
        let (expected, _) = inspect(source, StudyMode::Machine, false);
        for mode in [StudyMode::Recursive, StudyMode::Growing] {
            let (actual, _) = inspect(source, mode, false);
            assert_eq!(
                actual, expected,
                "declarator mismatch for {mode:?}: {source}"
            );
        }
    }
}

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
    let (expected, _) = inspect(&source, StudyMode::Machine, false);
    let (actual, stats) = inspect(&source, StudyMode::Recursive, false);
    assert_eq!(actual, expected);
    assert!(
        stats.max_depth > depth,
        "each parenthesized declarator is a recursive entry"
    );
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
    while let Some(error) = context.pop_pending_error() {
        let source = error.source_vectors(&mut context);
        let diagnostic = error.diagnostic_in(&context, source, &tu);
        text.push_str(renderer.render_text(&diagnostic, &context));
    }
    (text, stats)
}
