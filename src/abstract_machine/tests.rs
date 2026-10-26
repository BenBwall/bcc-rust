//! Source-driven evaluator regressions; no fixed-source execution fixtures.

use super::{
    DEMO_SOURCE,
    Session,
    State,
    Value,
    compile,
};

fn run(source: &str) -> Session {
    let program = compile(source).expect("test source must compile");
    let mut session = program.session();
    for _ in 0..=super::model::MAX_STEPS + 1 {
        if session.state().terminal() {
            return session;
        }
        let _state = session.step();
    }
    panic!("session did not terminate at its resource bound");
}

fn returned(source: &str, expected: i32) {
    let session = run(source);
    assert_eq!(
        session.state().status,
        "finished",
        "{:?}",
        session.state().diagnostics
    );
    assert_eq!(
        session.state().return_value,
        Some(Value::Integer(expected)),
        "source-driven result"
    );
}

#[test]
fn demo_runs_calls_loop_pointer_and_array() {
    let program = compile(DEMO_SOURCE).expect("demo compiles");
    let mut session = program.session();
    let mut saw_call = false;
    let mut saw_indeterminate = false;
    let mut saw_effect = false;
    let mut saw_pointer = false;
    while !session.state().terminal() {
        let state = session.step();
        saw_call |= state.frames.len() == 2 && state.frames[1].function == "bump";
        saw_indeterminate |= state
            .objects
            .iter()
            .any(|object| object.name == "total" && object.value == Value::Indeterminate);
        saw_effect |= !state.pending_effects.is_empty();
        saw_pointer |= state.objects.iter().any(|object| {
            matches!(
                object.value,
                Value::Pointer {
                    target: Some(1),
                    offset: 1,
                }
            )
        });
    }
    assert_eq!(
        session.state().return_value,
        Some(Value::Integer(9)),
        "demo result"
    );
    assert!(
        saw_call && saw_indeterminate && saw_effect && saw_pointer,
        "all required observations appear"
    );
    assert!(
        session
            .state()
            .objects
            .iter()
            .all(|object| object.lifetime == "ended"),
        "return ends all automatic lifetimes"
    );
    returned(&DEMO_SOURCE.replace("{1, 2, 3}", "{4, 5, 6}"), 18);
}

#[test]
fn replay_restores_complete_states_and_does_not_reexecute() {
    let mut session = run(DEMO_SOURCE);
    let finished = session.state_json();
    let count = session.state().step;
    let trace = session.trace_json();
    let mut states: Vec<State> = Vec::new();
    for _ in 0..count {
        states.push(session.back().clone());
    }
    assert_eq!(session.state().step, 0, "rewound to initial state");
    let initial = session.back().clone();
    assert_eq!(initial, *session.state(), "back at start is stable");
    let _initial = states.pop();
    for expected in states.iter().rev() {
        assert_eq!(session.step(), expected, "replay restores each observation");
    }
    let _state = session.step();
    assert_eq!(
        session.state_json(),
        finished,
        "final snapshot replays exactly"
    );
    assert_eq!(
        session.trace_json(),
        trace,
        "replay does not append duplicate trace states"
    );
}

#[test]
fn loops_branches_recursion_and_argument_values_are_executed() {
    returned(
        "int sum(int n) { if (n == 0) return 0; int previous = sum(n - 1); return previous + n; } \
         int main(void) { return sum(5); }",
        15,
    );
    returned(
        "int main(void) { int n = 0; int total = 0; while (n < 5) { if (n % 2 == 0) total = total \
         + n; else total = total - 1; n++; } return total; }",
        4,
    );
    returned(
        "int add(int a, int b) { return a + b; } int main(void) { int result = add(3, 8); return \
         result; }",
        11,
    );
    returned(
        "int main(void) { int n = 0; for (; n < 4; ++n) ; return n; }",
        4,
    );
}

#[test]
fn dereference_increment_precedence_is_preserved_or_explicitly_rejected() {
    for operation in ["*p++", "*p--", "*(p)++", "*(p)--"] {
        let mut source = String::from("int main(void) { int a[2] = {3, 7}; int *p = a; ");
        source.push_str(operation);
        source.push_str("; return a[0]; }");
        let diagnostic =
            compile(&source).expect_err("unparenthesized postfix dereference must be rejected");
        assert!(
            diagnostic.message.contains("nested side effects"),
            "precise subset limitation"
        );
    }
    returned(
        "int main(void) { int a[2] = {3, 7}; int *p = a; (*p)++; return a[0]; }",
        4,
    );
    returned(
        "int main(void) { int a[2] = {3, 7}; int *p = a; ++*p; return a[0]; }",
        4,
    );
    returned(
        "int main(void) { int a[2] = {3, 7}; int *p = a; (*p)--; return a[0]; }",
        2,
    );
    returned(
        "int main(void) { int a[2] = {3, 7}; int *p = a; --*p; return a[0]; }",
        2,
    );
    returned(
        "int main(void) { int a[2] = {3, 7}; int *p = a; p++; return a[0]; }",
        3,
    );
}

#[test]
fn calls_require_a_prior_definition_or_the_current_function() {
    for source in [
        "int main(void) { return later(); } int later(void) { return 7; }",
        "int main(void) { if (0) return later(); return 0; } int later(void) { return 7; }",
    ] {
        let diagnostic =
            compile(source).expect_err("C99 requires a visible declaration before a call");
        assert!(
            diagnostic.message.contains("before its call"),
            "forward-call diagnostic explains the boundary"
        );
    }
    returned(
        "int earlier(void) { return 7; } int main(void) { return earlier(); }",
        7,
    );
    returned(
        "int self(int n) { if (n == 0) return 0; int result = self(n - 1); return result + 1; } \
         int main(void) { return self(3); }",
        3,
    );
}

#[test]
fn shadowing_ends_inner_lifetimes_and_preserves_outer_object() {
    let session = run("int main(void) { int x = 7; { int x = 9; x++; } return x; }");
    assert_eq!(
        session.state().return_value,
        Some(Value::Integer(7)),
        "outer binding survives"
    );
    assert_eq!(
        session.state().objects[1].value,
        Value::Integer(10),
        "inner binding was changed"
    );
    assert_ne!(
        session.state().objects[0].id,
        session.state().objects[1].id,
        "shadowed objects have distinct identities"
    );
}

#[test]
fn array_partial_initialization_and_scalar_pointer_arithmetic() {
    returned(
        "int main(void) { int a[2]; int *end = &a[2]; int *p = end - 1; *p = 6; return a[1]; }",
        6,
    );
    returned(
        "int main(void) { int *p = 0; int *q = &*p; return q == 0; }",
        1,
    );
    returned(
        "int main(void) { int a[3] = {4}; int *p = a; p = p + 2; *p = 8; return a[0] + a[1] + \
         a[2]; }",
        12,
    );
    returned(
        "int main(void) { int x = 4; int *p = &x; p++; p--; return *p; }",
        4,
    );
    returned(
        "int main(void) { int *p = 0; if (!p) return 7; return 2; }",
        7,
    );
}

#[test]
fn verified_ub_rules_stop_at_the_operation_and_are_cited() {
    for (source, citation) in [
        ("int main(void) { return 1 / 0; }", "6.5.5p5"),
        ("int main(void) { return 2147483647 + 1; }", "6.5p5"),
        ("int main(void) { int *p = 0; return *p; }", "6.5.3.2p4"),
        ("int main(void) { int a[2]; return a[2]; }", "6.5.6p8"),
        (
            "int main(void) { int a[2]; int *p = a + 3; return 0; }",
            "6.5.6p8",
        ),
        ("int main(void) { return (-2147483647 - 1) / -1; }", "6.5p5"),
        (
            "int main(void) { int a[2]; int *p = a - (-2147483647 - 1); return 0; }",
            "6.5.6p8",
        ),
    ] {
        let mut session = run(source);
        assert_eq!(session.state().status, "undefined_behavior", "{source}");
        let diagnostic = &session.state().diagnostics[0];
        assert!(
            diagnostic
                .citation
                .is_some_and(|rule| rule.contains(citation)),
            "verified clause for {source}"
        );
        assert!(
            diagnostic.source_range.end <= source.len(),
            "diagnostic range belongs to source"
        );
        let stopped = session.state_json();
        let _state = session.step();
        assert_eq!(session.state_json(), stopped, "UB is terminal");
    }
}

#[test]
fn indeterminate_policy_is_distinct_from_proven_ub() {
    for source in [
        "int main(void) { int x; return x; }",
        "int main(void) { int a[2]; return a[0]; }",
        "int main(void) { int *p; { int x = 1; p = &x; } return *p; }",
    ] {
        let session = run(source);
        assert_eq!(session.state().status, "error", "conservative policy stops");
        assert_eq!(
            session.state().diagnostics[0].kind,
            "indeterminate",
            "does not overclaim UB"
        );
        assert!(
            session.state().diagnostics[0].citation.is_some(),
            "policy cites the underlying rule"
        );
    }
}

#[test]
fn unsupported_constructs_are_rejected_even_in_dead_code() {
    for source in [
        "#define X 1\nint main(void) { return X; }",
        "int global; int main(void) { return 0; }",
        "int main(void) { if (0) { double x; } return 0; }",
        "int main(void) { int i = 0; return i++ + i; }",
        "int f(void) { return 1; } int main(void) { return f() + f(); }",
        "int main(void) { return missing(); }",
        "int main(void) { int x = 1; int **p; return x; }",
        "int f(int x) { return x; } int main(void) { return f(); }",
        "int main(void) { int *p; int x = 0; p = x; return 0; }",
        "int main(void) { int a[2]; a = 0; return 0; }",
        "int main(void) { int x; int x; return 0; }",
        "int f(int x) { int x; return 0; } int main(void) { return f(1); }",
        "int main(void) { return 010; }",
        "int main(void) { return 0 && 1 / 0; }",
        "int main(void) { return sizeof(int); }",
        "// comment ending in a splice\\\nint main(void) { return 0; }",
        "// trigraph ??/\nint main(void) { return 0; }",
    ] {
        let _diagnostic = compile(source).expect_err("unsupported input must be rejected");
    }
}

#[test]
fn limits_stop_source_compilation_execution_memory_and_recursion() {
    assert_eq!(
        compile(&" ".repeat(super::model::MAX_SOURCE + 1))
            .expect_err("source cap")
            .kind,
        "limit",
        "source size is bounded"
    );
    let nested = format_args_source(80);
    assert_eq!(
        compile(&nested).expect_err("depth cap").kind,
        "limit",
        "nesting is bounded"
    );
    for source in [
        "int main(void) { while (1) ; }",
        "int f(void) { return f(); } int main(void) { return f(); }",
        "int main(void) { while (1) { int x; } }",
        "int main(void) { int a[512]; int b; return 0; }",
    ] {
        assert_eq!(
            run(source).state().status,
            "limit",
            "bounded resource diagnostic for {source}"
        );
    }
}

#[test]
fn snapshots_use_schema_ranges_pointer_ids_and_escaped_source() {
    let source = "// quote \" and slash \\ and tab\t\nint main(void) { int a[1] = {2}; int *p = \
                  a; return *p; }";
    let session = run(source);
    let json = session.state_json();
    assert!(json.contains("\"schema_version\":1"), "versioned schema");
    assert!(
        json.contains("\"kind\":\"pointer\",\"target\":1,\"offset\":0"),
        "object-based pointer"
    );
    assert!(json.contains("\"line\":2"), "one-based line");
    let trace = session.trace_json();
    assert!(
        trace.contains("quote \\\" and slash \\\\ and tab\\t\\n"),
        "JSON source escaping"
    );
}

fn format_args_source(depth: usize) -> String {
    let mut source = String::from("int main(void) { return ");
    source.push_str(&"(".repeat(depth));
    source.push('1');
    source.push_str(&")".repeat(depth));
    source.push_str("; }");
    source
}
