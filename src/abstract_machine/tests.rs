//! Source-driven evaluator regressions; no fixed-source execution fixtures.

use super::{
    DEMO_SOURCE,
    EventKind,
    Operand,
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
        let event = &session.state().event;
        assert!(
            !event.explanation.contains("prototype")
                && !session.state().diagnostics[0].message.contains("prototype"),
            "{}",
            event.explanation
        );
        let dangling = source.contains("&x");
        assert_eq!(
            event.title,
            if dangling {
                "Use of a dangling pointer"
            } else {
                "Read of an indeterminate value"
            }
        );
        // C99 §6.2.6.1p5: an indeterminate value may be a trap representation.
        assert_eq!(
            event.clauses.iter().any(|clause| clause.id == "6.2.6.1p5"),
            !dangling
        );
    }
}

#[test]
fn out_of_bounds_access_names_the_array_and_index() {
    let session = run("int main(void) { int a[3] = {2, 4, 6}; int *end = a + 3; return *end; }");
    let message = &session.state().diagnostics[0].message;
    assert!(
        message.starts_with("`a` has 3 elements, [0] to [2], and this access is at [3], one past"),
        "{message}"
    );
    // C99 §6.5.6p7: &x + 1 is the one-past position of an array of one.
    let session = run("int main(void) { int x = 1; int *p = &x + 1; return *p; }");
    let message = &session.state().diagnostics[0].message;
    assert!(message.contains("at [1], one past the end"), "{message}");
    let session = run("int main(void) { int x; return x; }");
    let message = &session.state().diagnostics[0].message;
    assert!(
        message.starts_with("nothing has been stored in `x` yet"),
        "{message}"
    );
}

#[test]
fn every_step_of_a_topic_carries_its_note() {
    let run = states("int main(void) { int a = 1; int b = 2; b = a + b; b = b + a; return b; }");
    let notes = |kind: EventKind| -> Vec<&str> {
        run.iter()
            .filter(|state| state.event.kind == kind)
            .map(|state| state.event.note.as_str())
            .collect()
    };
    // The site shows a note once; the engine attaches it to every step of
    // its topic, so skipping steps (Run, Run to line) loses none.
    let declarations = notes(EventKind::Declare);
    assert!(
        declarations[0].contains("automatic local variables"),
        "{}",
        declarations[0]
    );
    assert_eq!(declarations[0], declarations[1]);
    let sums = notes(EventKind::Arithmetic);
    assert!(sums[0].contains("2147483647"), "{}", sums[0]);
    assert_eq!(sums[0], sums[1]);
    let stores = notes(EventKind::Assign);
    assert!(stores[0].contains("a[i] = i++"), "{}", stores[0]);
    // Back and a replayed step see the same notes.
    let program = compile("int main(void) { int a = 1; int b = a; return b; }").expect("compiles");
    let mut session = program.session();
    let first = session.step().clone();
    let _ = session.step();
    assert_eq!(session.back(), &first);
    // C99 §6.7.8p17: without designators, elements are initialized in order.
    let array = states("int main(void) { int a[2] = {1, 2}; return a[0]; }");
    let select = array
        .iter()
        .find(|state| state.event.title.starts_with("Select"))
        .expect("an element is selected");
    assert_eq!(select.event.clauses[0].id, "6.7.8p17");
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
    assert!(json.contains("\"schema_version\":2"), "versioned schema");
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

/// Every state of a run, from the initial one.
fn states(source: &str) -> Vec<State> {
    let program = compile(source).expect("test source must compile");
    let mut session = program.session();
    let mut states = vec![session.state().clone()];
    while !session.state().terminal() {
        states.push(session.step().clone());
    }
    states
}

fn kinds(states: &[State]) -> Vec<&'static str> {
    states.iter().map(|state| state.event.kind.name()).collect()
}

#[test]
fn events_name_the_rule_source_values_and_clause() {
    let source = "int main(void) { int x = 2; x = x + 3; return x; }";
    let states = states(source);
    assert_eq!(
        kinds(&states),
        [
            "start",
            "enter_block",
            "declare",
            "designate",
            "constant",
            "initialize",
            "designate",
            "designate",
            "lvalue_conversion",
            "constant",
            "arithmetic",
            "assign",
            "designate",
            "lvalue_conversion",
            "return",
        ]
    );
    let read = &states[8].event;
    assert_eq!(
        &source[read.range.start..read.range.end],
        "x",
        "exact operand range"
    );
    assert_eq!(read.result, Some(Operand::Value(Value::Integer(2))));
    assert_eq!(
        read.clauses[0].id, "6.3.2.1p2",
        "lvalue conversion is cited"
    );
    assert!(read.explanation.contains('2'), "{}", read.explanation);
    let sum = &states[10].event;
    assert_eq!(&source[sum.range.start..sum.range.end], "x + 3");
    assert_eq!(
        sum.operands,
        [
            Operand::Value(Value::Integer(2)),
            Operand::Value(Value::Integer(3))
        ]
    );
    assert_eq!(sum.title, "Compute 2 + 3 = 5");
    let assign = &states[11].event;
    assert_eq!(&source[assign.range.start..assign.range.end], "x = x + 3");
    assert!(
        assign.sequence_point,
        "an expression statement ends a full expression"
    );
    assert!(
        assign
            .clauses
            .iter()
            .any(|clause| clause.id == "6.5.16.1p2")
    );
    assert!(
        assign.explanation.contains("previously 2"),
        "{}",
        assign.explanation
    );
    let done = &states[14].event;
    assert_eq!(done.title, "Return 5 from main");
    assert_eq!(done.clauses[0].id, "5.1.2.2.3p1");
    for state in &states {
        assert!(!state.event.title.is_empty() && !state.event.explanation.is_empty());
        assert!(
            !state.event.clauses.is_empty(),
            "{} cites a clause",
            state.event.kind.name()
        );
    }
}

#[test]
fn expression_trees_fill_in_as_evaluation_proceeds() {
    let source = "int main(void) { int a = 4; int b = a * (a - 1); return b; }";
    let program = compile(source).expect("compiles");
    let states = states(source);
    let product = states
        .iter()
        .position(|state| {
            state.event.kind == EventKind::Arithmetic && state.event.title.contains('*')
        })
        .expect("the product is computed");
    let evaluation = states[product]
        .evaluation
        .as_ref()
        .expect("a full expression is active");
    let nodes = program.expression(evaluation.function, evaluation.root);
    let labels: Vec<&str> = nodes.iter().map(|node| node.label.as_str()).collect();
    assert_eq!(
        labels,
        ["b", "a", "a", "1", "-", "*", "="],
        "children precede parents"
    );
    let first = nodes.first().map(|node| node.first).expect("nodes");
    let value = |label: &str| {
        let index = labels
            .iter()
            .rposition(|candidate| *candidate == label)
            .expect("label")
            + first;
        evaluation
            .values
            .iter()
            .find(|(node, _)| *node == index)
            .map(|(_, value)| value.clone())
    };
    assert_eq!(value("-"), Some(Operand::Value(Value::Integer(3))));
    assert_eq!(value("*"), Some(Operand::Value(Value::Integer(12))));
    assert_eq!(value("="), None, "the store has not run yet");
    assert_eq!(
        evaluation.last,
        Some(first + 5),
        "the product node just completed"
    );
    let stored = states[product + 1]
        .evaluation
        .as_ref()
        .expect("same expression");
    assert_eq!(stored.root, evaluation.root);
    assert!(
        stored
            .values
            .iter()
            .any(|(node, value)| *node == evaluation.root
                && *value == Operand::Value(Value::Integer(12)))
    );
}

#[test]
fn calls_returns_and_block_exits_explain_frames_and_lifetimes() {
    let source = "int twice(int n) { int r = n + n; return r; } int main(void) { int v = \
                  twice(3); { int t = v; } return v; }";
    let states = states(source);
    let call = states
        .iter()
        .find(|state| state.event.kind == EventKind::Call)
        .expect("a call");
    assert!(call.event.sequence_point);
    assert!(
        call.event.explanation.contains("`n` = 3"),
        "{}",
        call.event.explanation
    );
    assert!(
        call.event
            .clauses
            .iter()
            .any(|clause| clause.id == "6.5.2.2p4")
    );
    assert!(
        call.evaluation.is_none(),
        "the callee starts a fresh evaluation"
    );
    let waiting = call.frames[0].waiting.expect("main waits on the call");
    assert_eq!(&source[waiting.start..waiting.end], "twice(3)");
    assert_eq!(call.frames[1].waiting, None, "the callee waits on nothing");
    let returned = states
        .iter()
        .position(|state| state.event.kind == EventKind::Return)
        .expect("twice returns");
    let event = &states[returned].event;
    assert_eq!(event.title, "Return 6 from twice");
    assert!(
        event.explanation.contains("`twice(3)` now has the value 6"),
        "{}",
        event.explanation
    );
    assert_eq!(event.objects.len(), 2, "n and r end with the frame");
    let caller = states[returned]
        .evaluation
        .as_ref()
        .expect("the caller's expression resumes");
    assert!(
        caller
            .values
            .iter()
            .any(|(_, value)| *value == Operand::Value(Value::Integer(6))),
        "the call node holds the returned value"
    );
    let leave = states
        .iter()
        .find(|state| state.event.kind == EventKind::LeaveBlock && !state.event.objects.is_empty())
        .expect("the inner block ends t");
    assert_eq!(leave.event.title, "End the lifetime of `t`");
    let recursive = self::states(
        "int f(int n) { if (n == 0) return 0; int r = f(n - 1); return r + 1; } int main(void) { \
         return f(2); }",
    );
    assert!(
        recursive
            .iter()
            .any(|state| state.event.kind == EventKind::Call
                && state.event.explanation.contains("recursive"))
    );
}

#[test]
fn pointer_events_describe_decay_one_past_and_indirection() {
    let source =
        "int main(void) { int a[2] = {5}; int *p = a; int *end = p + 2; p = p + 1; return *p; }";
    let states = states(source);
    let find = |kind: EventKind| {
        states
            .iter()
            .filter(|state| state.event.kind == kind)
            .map(|state| &state.event)
            .collect::<Vec<_>>()
    };
    assert!(find(EventKind::ArrayDecay)[0].explanation.contains("a[0]"));
    assert!(
        find(EventKind::Constant)
            .iter()
            .any(|event| event.title == "Supply an implicit 0"),
        "partial initialization fills with zero"
    );
    let arithmetic = find(EventKind::PointerArithmetic);
    assert!(
        arithmetic[0].explanation.contains("one-past-the-end"),
        "{}",
        arithmetic[0].explanation
    );
    assert!(!arithmetic[1].explanation.contains("one-past-the-end"));
    let indirection = find(EventKind::Dereference);
    assert_eq!(indirection[0].title, "`*p` designates a[1]");
    let fault = run("int main(void) { int a[2] = {1, 2}; return a[2]; }");
    let event = &fault.state().event;
    assert_eq!(event.kind, EventKind::Fault);
    assert_eq!(event.title, "Undefined behavior: access one past the end");
    assert_eq!(event.clauses[0].id, "6.5.6p8");
}

#[test]
fn every_step_leads_with_one_sentence() {
    let sources = [
        DEMO_SOURCE,
        "int main(void) { char c = 100; char d = c + c; d++; int a[2] = {5}; int *p = a; int *q = \
         &a[1]; if (p == q) { return 1; } else { p = 0; } while (!p) { p = q; } return -d / 2 + \
         *p % 3; }",
        "int main(void) { int big = 2147483647; big = big + 1; return big; }",
        "int main(void) { int x; return x; }",
        "int main(void) { int zero = 0; return 1 / zero; }",
        "int main(void) { int *p = 0; return *p; }",
    ];
    for source in sources {
        for state in states(source) {
            let event = &state.event;
            let summary = &event.summary;
            assert!(
                summary.ends_with('.') && summary.len() <= 170,
                "{:?} {}: {summary:?}",
                event.kind,
                event.title
            );
            assert!(
                summary
                    .chars()
                    .next()
                    .is_some_and(|first| !first.is_lowercase()),
                "{summary:?}"
            );
            assert_ne!(summary, &event.explanation, "{summary}");
        }
    }
}

#[test]
fn names_are_narrated_by_what_their_operator_does() {
    let states = states("int main(void) { int x = 1; int *p = &x; x = x + 1; x++; return x; }");
    let titles = states
        .iter()
        .filter(|state| state.event.kind == EventKind::Designate)
        .map(|state| state.event.title.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        titles,
        [
            "`x` receives its initial value",
            "`p` receives its initial value",
            "`x` is the operand of `&`",
            "`x` is where the result goes",
            "`x` names an object to read",
            "`x` is the object to update",
            "`x` names an object to read",
        ]
    );
    let initializer = states
        .iter()
        .find(|state| state.event.kind == EventKind::Designate)
        .map(|state| &state.event)
        .expect("a designation");
    // A declarator is not an expression: no primary-expression citation.
    assert_eq!(initializer.clauses[0].id, "6.7.8p11");
    assert_eq!(initializer.note, "");
    let assigned = states
        .iter()
        .find(|state| state.event.title == "`x` is where the result goes")
        .map(|state| &state.event)
        .expect("an assignment target");
    assert!(
        assigned
            .clauses
            .iter()
            .any(|clause| clause.id == "6.5.16p4")
    );
}

#[test]
fn a_false_condition_says_whether_an_else_branch_runs() {
    let titles = |source: &str| {
        states(source)
            .into_iter()
            .filter(|state| state.event.kind == EventKind::Branch)
            .map(|state| state.event.title)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        titles("int main(void) { int x = 0; if (x) { x = 1; } else { x = 2; } return x; }"),
        ["Condition is false: take the else branch"]
    );
    assert_eq!(
        // The loop's backward jump ends the first branch; it is no else.
        titles("int main(void) { int x = 0; if (x) while (x) x = 0; return x; }"),
        ["Condition is false: skip the if branch"]
    );
}

#[test]
fn undefined_behavior_is_titled_by_its_cause() {
    let title = |source: &str| run(source).state().event.title.clone();
    assert_eq!(
        title("int main(void) { int big = 2147483647; return big + 1; }"),
        "Undefined behavior: signed overflow"
    );
    assert_eq!(
        title("int main(void) { int zero = 0; return 1 % zero; }"),
        "Undefined behavior: division by zero"
    );
    assert_eq!(
        title("int main(void) { int *p = 0; return *p; }"),
        "Undefined behavior: null pointer dereference"
    );
    assert_eq!(
        title("int main(void) { int a[2] = {1, 2}; int *p = a + 3; return *p; }"),
        "Undefined behavior: pointer outside its array"
    );
    // Nothing is accessed: the arithmetic itself has no meaning.
    assert_eq!(
        title("int main(void) { int *p = 0; p = p + 1; return 0; }"),
        "Undefined behavior: arithmetic on a null pointer"
    );
    // The remainder 0 would fit; the quotient is what does not.
    for operator in ["/", "%"] {
        let session = run(&format!(
            "int main(void) {{ int m = -2147483647 - 1; int n = -1; return m {operator} n; }}"
        ));
        let event = &session.state().event;
        assert_eq!(event.title, "Undefined behavior: the quotient does not fit");
        assert!(event.clauses.iter().any(|clause| clause.id == "6.5.5p6"));
    }
}

#[test]
fn storing_a_char_into_a_char_converts_nothing() {
    let states =
        states("int main(void) { char a = 5; char b = a; b = a; int i = 7; a = i; return b; }");
    let stores = states
        .iter()
        .filter(|state| matches!(state.event.kind, EventKind::Initialize | EventKind::Assign))
        .map(|state| &state.event)
        .collect::<Vec<_>>();
    let converts = stores
        .iter()
        .map(|event| event.explanation.contains("is converted to"))
        .collect::<Vec<_>>();
    // char a = 5 and a = i store ints; char b = a and b = a store chars.
    assert_eq!(converts, [true, false, false, false, true], "{stores:#?}");
}

#[test]
fn loop_conditions_and_exit_statuses_say_what_happens() {
    let states = states("int main(void) { int i = 0; while (i < 2) { i = i + 1; } return -1; }");
    let loops = states
        .iter()
        .filter(|state| state.event.kind == EventKind::Branch)
        .map(|state| state.event.summary.as_str())
        .collect::<Vec<_>>();
    assert!(
        loops.iter().all(|summary| !summary.contains("again")),
        "{loops:?}"
    );
    let returned = &states.last().expect("a final state").event;
    assert!(
        returned.summary.contains("a shell on Linux sees as 255"),
        "{}",
        returned.summary
    );
    // A declaration's lead does not claim the object starts there.
    let declared = &states
        .iter()
        .find(|state| state.event.kind == EventKind::Declare)
        .expect("a declaration")
        .event;
    assert!(
        declared.summary.contains("from the start of its block"),
        "{}",
        declared.summary
    );
}

#[test]
fn binary_operators_say_their_operand_order_is_the_machines_choice() {
    let states = states("int main(void) { int a = 1; int b = 2; return a + b < 4; }");
    for kind in [EventKind::Arithmetic, EventKind::Comparison] {
        let event = &states
            .iter()
            .find(|state| state.event.kind == kind)
            .expect("the operator's step")
            .event;
        assert!(
            event.explanation.contains("unspecified"),
            "{}",
            event.explanation
        );
        assert!(event.clauses.iter().any(|clause| clause.id == "6.5p3"));
    }
}

#[test]
fn subscripts_span_their_bracket_and_narration_agrees_in_number() {
    let source = "int f(void) { return 1; }\nint main(void) { int a[3] = {1, 2, 3}; int *p = \
                  &a[2]; { int t = a[1]; } int one = f(); return one + *p; }";
    let states = states(source);
    let event = |kind: EventKind| {
        states
            .iter()
            .map(|state| &state.event)
            .filter(|event| event.kind == kind)
            .collect::<Vec<_>>()
    };
    let text = |event: &super::Event| {
        source
            .get(event.range.start..event.range.end)
            .expect("event range")
            .to_owned()
    };
    // C99 §6.5.3.2p3: &a[2] is a + 2; its range and title include the `]`.
    let address = event(EventKind::PointerArithmetic)[0];
    assert_eq!(text(address), "&a[2]");
    assert_eq!(address.title, "Compute the address &a[2]");
    assert!(
        address.explanation.contains("moves 2 ints") && address.explanation.contains("a + i"),
        "{}",
        address.explanation
    );
    assert_eq!(address.clauses[0].id, "6.5.3.2p3");
    assert!(
        address
            .explanation
            .contains("the element itself is never read"),
        "{}",
        address.explanation
    );
    let subscript = event(EventKind::Subscript)[0];
    assert_eq!(text(subscript), "a[1]");
    assert!(
        subscript.explanation.contains("moves 1 element forward"),
        "{}",
        subscript.explanation
    );
    let ended = event(EventKind::LeaveBlock)[0];
    assert!(
        ended
            .explanation
            .contains("`t` reaches the end of its lifetime"),
        "{}",
        ended.explanation
    );
    let remainder = self::states("int main(void) { int x = 7; return x % 3; }")
        .into_iter()
        .find(|state| state.event.kind == EventKind::Arithmetic)
        .expect("the remainder is computed")
        .event;
    assert!(
        remainder
            .explanation
            .contains("takes the remainder of the division in int: 7 % 3 = 1"),
        "{}",
        remainder.explanation
    );
    let call = event(EventKind::Call)[0];
    assert!(
        call.explanation.starts_with("The call has no arguments"),
        "{}",
        call.explanation
    );
}

/// The pointer-arithmetic events of `main`'s `int *q = INITIALIZER;`, each
/// with the source text it covers and whether it is narrated as `&a[i]`.
fn pointer_steps(initializer: &str) -> Vec<(String, bool, String)> {
    let source = format!(
        "int main(void) {{ int a[3] = {{1, 2, 3}}; int x = 0; int *q = {initializer}; return x; }}"
    );
    self::states(&source)
        .into_iter()
        .filter(|state| state.event.kind == EventKind::PointerArithmetic)
        .map(|state| {
            let event = state.event;
            let text = source[event.range.start..event.range.end].to_owned();
            let element = event.clauses.iter().any(|clause| clause.id == "6.5.3.2p3");
            assert_eq!(
                element,
                event.explanation.contains("&a[i] as a + i"),
                "{}",
                event.explanation
            );
            (text, element, event.explanation)
        })
        .collect()
}

#[test]
fn only_an_element_address_is_narrated_as_one() {
    // C99 §6.5.3.2p3: the `&[]` node, not the source text, decides.
    let steps = pointer_steps("&a[1] + 1");
    assert_eq!(
        steps
            .iter()
            .map(|(text, element, _)| (text.as_str(), *element))
            .collect::<Vec<_>>(),
        [("&a[1]", true), ("&a[1] + 1", false)]
    );
    assert!(steps[1].2.contains("moves 1 int forward"), "{}", steps[1].2);
    let steps = pointer_steps("&x + 1");
    assert_eq!(steps.len(), 1);
    assert!(!steps[0].1, "{}", steps[0].2);
    let steps = pointer_steps("&a[2] - 1");
    assert_eq!(
        steps
            .iter()
            .map(|(text, element, _)| (text.as_str(), *element))
            .collect::<Vec<_>>(),
        [("&a[2]", true), ("&a[2] - 1", false)]
    );
    assert!(steps[1].2.contains("moves 1 int back"), "{}", steps[1].2);
    let steps = pointer_steps("(&a[2])");
    assert_eq!(steps.len(), 1);
    assert!(steps[0].1, "{}", steps[0].2);
    // A negative index moves back.
    let back =
        self::states("int main(void) { int a[3] = {1, 2, 3}; int *p = a + 2; return p[-1]; }")
            .into_iter()
            .find(|state| state.event.kind == EventKind::Subscript)
            .expect("p[-1] is a subscript")
            .event;
    assert!(
        back.explanation
            .contains("moves 1 element back and designates a[1]"),
        "{}",
        back.explanation
    );
}

#[test]
fn char_values_are_promoted_and_out_of_range_stores_wrap() {
    let source =
        "int main(void) { char c = 100; char d = c + c; int w = c + c; c++; return w + d; }";
    returned(source, 200 - 56);
    let states = states(source);
    let promotions = states
        .iter()
        .filter(|state| state.event.kind == EventKind::IntegerPromotion)
        .count();
    assert_eq!(promotions, 5, "each char operand of + is promoted");
    let wrapped = states
        .iter()
        .find(|state| state.event.title == "Initialize `d` = -56")
        .expect("200 converts to char");
    assert!(
        wrapped
            .event
            .clauses
            .iter()
            .any(|clause| clause.id == "6.3.1.3p3")
    );
    assert!(
        wrapped
            .event
            .explanation
            .starts_with("The value to store is the int 200.")
            && wrapped.event.explanation.contains("implementation-defined"),
        "{}",
        wrapped.event.explanation
    );
    let increment = states
        .iter()
        .find(|state| state.event.kind == EventKind::Increment)
        .expect("c++");
    assert!(
        increment.event.explanation.contains("promoted to int"),
        "{}",
        increment.event.explanation
    );
    returned("int main(void) { char c = 127; c++; return c; }", -128);
    for rejected in [
        "int main(void) { char c = 1; int *p = &c; return 0; }",
        "int main(void) { char s[2]; return 0; }",
    ] {
        let _diagnostic =
            compile(rejected).expect_err("char pointers and arrays are outside the subset");
    }
}

#[test]
fn char_reads_promote_only_where_an_operator_requires_it() {
    let promotions = |source: &str| {
        states(source)
            .iter()
            .filter(|state| state.event.kind == EventKind::IntegerPromotion)
            .count()
    };
    // Lvalue conversion keeps the char type: copying, discarding, testing,
    // indexing with, passing and returning a char promote nothing.
    for unpromoted in [
        "int main(void) { char c = 1; char d = c; return 0; }",
        "int main(void) { char c = 1; c; return 0; }",
        "int main(void) { char c = 1; if (c) { return 1; } return 0; }",
        "int main(void) { char c = 1; int a[2] = {0, 7}; int *p = a + c; return a[c] + *p; }",
        "int id(int x) { return x; } int main(void) { char c = 1; return id(c); }",
        "int main(void) { char c = 1; return !c; }",
        "int main(void) { char c = 1; return c; }",
    ] {
        assert_eq!(promotions(unpromoted), 0, "{unpromoted}");
    }
    // The operands of unary + and -, and of arithmetic, relational and
    // equality operators, are promoted.
    for (promoted, count) in [
        ("int main(void) { char c = 1; return -c; }", 1),
        ("int main(void) { char c = 1; return +c; }", 1),
        ("int main(void) { char c = 1; return c < 2; }", 1),
        (
            "int main(void) { char c = 1; char d = 2; return c == d; }",
            2,
        ),
    ] {
        assert_eq!(promotions(promoted), count, "{promoted}");
    }
}

#[test]
fn unary_plus_cites_its_own_paragraph() {
    let plus = states("int main(void) { int x = 1; return +x; }")
        .into_iter()
        .find(|state| state.event.kind == EventKind::Unary)
        .expect("unary plus is evaluated");
    assert_eq!(plus.event.clauses[0].id, "6.5.3.3p2");
}

#[test]
fn storing_zero_into_a_pointer_explains_the_null_pointer() {
    for source in [
        "int main(void) { int *p = 0; return 0; }",
        "int main(void) { int x = 1; int *p = &x; p = 0; return 0; }",
    ] {
        let store = states(source)
            .into_iter()
            .rfind(|state| state.event.title.contains("NULL"))
            .expect("0 is stored into p");
        let ids = store
            .event
            .clauses
            .iter()
            .map(|clause| clause.id)
            .collect::<Vec<_>>();
        assert!(ids.contains(&"6.3.2.3p3"), "{ids:?}");
        assert!(!ids.contains(&"6.3.1.3p3"), "{ids:?}");
        assert!(
            store.event.explanation.contains("null pointer")
                && !store.event.explanation.contains("char"),
            "{}",
            store.event.explanation
        );
    }
}

#[test]
fn schema_two_encodes_events_and_evaluations() {
    let session = run("int main(void) { int x = 1; return x; }");
    let json = session.state_json();
    assert!(json.contains("\"schema_version\":2"));
    assert!(
        json.contains("\"kind\":\"return\",\"category\":\"call\""),
        "{json}"
    );
    assert!(json.contains("\"id\":\"5.1.2.2.3p1\""), "{json}");
}

#[test]
fn overflow_names_the_arithmetic_that_did_not_fit() {
    let session = run("int main(void) { int x = 2147483647; x = x + 1; return x; }");
    let diagnostic = &session.state().diagnostics[0];
    assert!(
        diagnostic
            .message
            .starts_with("2147483647 + 1 = 2147483648"),
        "{}",
        diagnostic.message
    );
    assert_eq!(
        session.state().event.title,
        "Undefined behavior: signed overflow"
    );
    let negated = run("int main(void) { int x = -2147483647 - 1; return -x; }");
    assert!(
        negated.state().diagnostics[0]
            .message
            .starts_with("-(-2147483648) = 2147483648")
    );
}
