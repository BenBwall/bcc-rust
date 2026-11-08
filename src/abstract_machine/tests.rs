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

fn returned(source: &str, expected: i128) {
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
                    ..
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
        "#define X(a) a\nint main(void) { return X(1); }",
        "int global = global; int main(void) { return 0; }",
        "struct s { int x; }; int main(void) { struct s a[2]; return 0; }",
        "struct s { int x; } v; int main(void) { return 0; }",
        "struct s { struct s inner; }; int main(void) { return 0; }",
        "struct s { int x; }; int main(void) { struct s a; struct s b; return a == b; }",
        "struct s { int x; }; int main(void) { struct s a = {1}; return a.y; }",
        "struct s { int x; }; int main(void) { struct s a = {1, 2}; return a.x; }",
        "struct s; int main(void) { struct s a; return 0; }",
        "int main(void) { static int x = 1 + 1; return x; }",
        "int main(void) { return 1.5 % 2; }",
        "int main(void) { if (0) { _Bool x; } return 0; }",
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
        "int main(void) { return 1 & 2; }",
        "int main(void) { return ~1; }",
        "int main(void) { double _Complex x; return 0; }",
        "int main(void) { long float x; return 0; }",
        "int main(void) { unsigned double x; return 0; }",
        "int main(void) { double x = 1e999; return 0; }",
        "int main(void) { double x = 0x1.8; return 0; }",
        "int main(void) { double d = 1.0; d <<= 1; return 0; }",
        "int main(void) { double d = 1.0; return (int)d % 2.0; }",
        "int main(void) { return 0x10; }",
        "int main(void) { return 99999999999999999999; }",
        "int main(void) { return 1lul; }",
        "int main(void) { int i = 0; i++ > 0; return 0; }",
        "int main(void) { int i = 0; while (i++ < i) {} return 0; }",
        "int main(void) { int i = 0; int *p = &i; while (i++ < *p) {} return 0; }",
        "int main(void) { int i = 0; int j = 0; while (i++ < j++) {} return 0; }",
        "int main(void) { unsigned short long x; return 0; }",
        "int main(void) { signed unsigned x; return 0; }",
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
    assert!(json.contains("\"schema_version\":7"), "versioned schema");
    assert!(
        json.contains(
            "\"kind\":\"pointer\",\"target\":1,\"offset\":0,\"first\":0,\"length\":1,\"structure\"\
             :null"
        ),
        "object-based pointer: {json}"
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
fn an_omitted_for_condition_is_a_nonzero_constant() {
    let states = states("int main(void) { for (;;) { return 3; } }");
    let branch = &states
        .iter()
        .find(|state| state.event.kind == EventKind::Branch)
        .expect("the loop's test")
        .event;
    assert!(
        branch
            .summary
            .starts_with("This for statement has no condition"),
        "{}",
        branch.summary
    );
    assert_eq!(branch.title, "No condition: run the loop body");
    assert_eq!(branch.clauses[0].id, "6.8.5.3p2");
    assert!(!branch.sequence_point);
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
    // p[0] is *(p + 0): the subscript is pointer arithmetic first.
    let session = run("int main(void) { int *q = 0; int i = 1; return q[i + 1]; }");
    let event = &session.state().event;
    assert_eq!(
        event.title,
        "Undefined behavior: subscript of a null pointer"
    );
    // The lead quotes the program's own subscript.
    assert!(
        event.summary.starts_with("`q[i + 1]` means `*(q + i + 1)`"),
        "{}",
        event.summary
    );
    let cited = event
        .clauses
        .iter()
        .map(|clause| clause.id)
        .collect::<Vec<_>>();
    assert_eq!(cited, ["6.5.2.1p2", "6.5.6p8"]);
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
fn a_decrement_is_narrated_as_a_subtraction() {
    let stepped = states("int main(void) { char b = -128; b--; return b; }");
    let decrement = &stepped
        .iter()
        .find(|state| state.event.kind == EventKind::Increment)
        .expect("the decrement")
        .event;
    assert!(
        decrement
            .explanation
            .contains("The subtraction happens in int"),
        "{}",
        decrement.explanation
    );
    let cited = |source: &str| {
        states(source)
            .into_iter()
            .find(|state| state.event.kind == EventKind::Increment)
            .map(|state| state.event.clauses[0].id)
    };
    assert_eq!(
        cited("int main(void) { int c = 1; c--; return c; }"),
        Some("6.5.2.4p3")
    );
    assert_eq!(
        cited("int main(void) { int c = 1; --c; return c; }"),
        Some("6.5.3.1p3")
    );
    assert_eq!(
        cited("int main(void) { int c = 1; c++; return c; }"),
        Some("6.5.2.4p2")
    );
    assert_eq!(
        cited("int main(void) { int c = 1; ++c; return c; }"),
        Some("6.5.3.1p2")
    );
    let overflow = run("int main(void) { int m = -2147483647 - 1; m--; return m; }");
    let message = &overflow.state().diagnostics[0].message;
    assert!(
        message.starts_with("-2147483648 - 1 = -2147483649"),
        "{message}"
    );
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
        String::from(
            source
                .get(event.range.start..event.range.end)
                .expect("event range"),
        )
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
    states(&source)
        .into_iter()
        .filter(|state| state.event.kind == EventKind::PointerArithmetic)
        .map(|state| {
            let event = state.event;
            let text = String::from(&source[event.range.start..event.range.end]);
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
    let back = states("int main(void) { int a[3] = {1, 2, 3}; int *p = a + 2; return p[-1]; }")
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
        "int main(void) { char *s[2]; return 0; }",
    ] {
        let _diagnostic = compile(rejected)
            .expect_err("a char's address is no int pointer, and arrays of pointers are outside");
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
fn schema_three_encodes_events_and_evaluations() {
    let session = run("int main(void) { int x = 1; return x; }");
    let json = session.state_json();
    assert!(json.contains("\"schema_version\":7"));
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

/// The ids of every clause an event cites.
fn cited(state: &State) -> Vec<&'static str> {
    state.event.clauses.iter().map(|clause| clause.id).collect()
}

#[test]
fn static_objects_are_initialized_before_main_and_live_for_the_whole_run() {
    let source = "struct point { int x; int y; };\nint counter;\nint limit = -3;\nint table[3] = \
                  {4};\nstruct point origin;\nint *cursor;\nint main(void) {\n    counter = \
                  counter + 1;\n    return counter + limit + table[0] + table[2] + origin.y;\n}\n";
    let states = states(source);
    assert!(
        states[0].event.summary.contains("before main"),
        "{}",
        states[0].event.summary
    );
    assert!(cited(&states[0]).contains(&"5.1.2p1"));
    let initial: Vec<&State> = states[1..=5].iter().collect();
    for state in &initial {
        assert_eq!(
            state.event.kind,
            EventKind::StaticStorage,
            "{}",
            state.event.title
        );
        assert!(state.frames.is_empty(), "no call has begun before main");
        assert!(cited(state).contains(&"6.2.4p3"), "{:?}", cited(state));
        assert!(cited(state).contains(&"5.1.2p1"));
    }
    let objects = &initial[4].objects;
    assert!(
        objects
            .iter()
            .all(|object| object.storage_duration == "static" && object.frame_id == 0)
    );
    assert_eq!(
        objects[0].value,
        Value::Integer(0),
        "counter starts at zero"
    );
    assert_eq!(
        objects[1].value,
        Value::Integer(-3),
        "limit's constant initializer"
    );
    assert_eq!(
        objects[2].value,
        Value::Array(vec![
            Value::Integer(4),
            Value::Integer(0),
            Value::Integer(0)
        ])
    );
    assert!(matches!(
        &objects[3].value,
        Value::Struct { members, .. } if members == &[Value::Integer(0), Value::Integer(0)]
    ));
    assert!(matches!(
        objects[4].value,
        Value::Pointer { target: None, .. }
    ));
    // Zero initialization and constant initializers cite their paragraphs.
    assert!(cited(initial[0]).contains(&"6.7.8p10"));
    assert!(cited(initial[1]).contains(&"6.7.8p4"));
    assert!(
        initial[0].event.summary.contains("starts as zero"),
        "{}",
        initial[0].event.summary
    );
    assert!(
        initial[4].event.summary.contains("null pointer"),
        "{}",
        initial[4].event.summary
    );
    assert!(initial[0].event.note.contains("Static objects"));
    // Main begins after them, and the statics outlive its frame.
    assert_eq!(states[6].frames.len(), 1, "main's first step");
    let last = states.last().expect("a final state");
    assert_eq!(last.status, "finished", "{:?}", last.diagnostics);
    assert_eq!(last.return_value, Some(Value::Integer(1 - 3 + 4)));
    assert!(
        last.objects
            .iter()
            .filter(|object| object.storage_duration == "static")
            .all(|object| object.lifetime == "alive")
    );
    // A global is read and written like any object.
    assert!(
        states
            .iter()
            .any(|state| state.event.title == "Store 1 into `counter`")
    );
}

#[test]
fn static_locals_keep_their_value_between_calls() {
    let source = "int tick(void) {\n    static int calls;\n    calls = calls + 1;\n    return \
                  calls;\n}\nint main(void) {\n    int first = tick();\n    int second = \
                  tick();\n    return first * 10 + second;\n}\n";
    let states = states(source);
    assert_eq!(states[1].event.kind, EventKind::StaticStorage);
    assert_eq!(states[1].objects[0].function.as_deref(), Some("tick"));
    assert!(states[1].event.summary.contains("starts as zero"));
    let reached: Vec<&State> = states
        .iter()
        .filter(|state| state.event.kind == EventKind::StaticLocal)
        .collect();
    assert_eq!(reached.len(), 2, "each call reaches the declaration");
    assert!(
        reached[1].event.summary.contains("keeps its value, 1"),
        "{}",
        reached[1].event.summary
    );
    assert!(
        reached[0]
            .event
            .summary
            .contains("does nothing at run time")
    );
    assert_eq!(reached[0].event.objects, [1]);
    assert!(cited(reached[0]).contains(&"6.2.4p3"));
    assert_eq!(
        states.last().and_then(|state| state.return_value.clone()),
        Some(Value::Integer(12))
    );
}

#[test]
fn structures_are_laid_out_with_padding_on_x86_64() {
    let program = compile(
        "struct entry { char tag; int count; char flag; };\nstruct line { struct entry first; int \
         *p; int a[3]; };\nint main(void) { struct entry e; e.count = 1; return e.count; }\n",
    )
    .expect("compiles");
    let entry = &program.structures()[0];
    assert_eq!((entry.size, entry.align), (12, 4));
    let layout: Vec<(&str, usize, usize, usize)> = entry
        .members
        .iter()
        .map(|member| {
            (
                member.name.as_str(),
                member.offset,
                member.size,
                member.padding,
            )
        })
        .collect();
    assert_eq!(
        layout,
        [("tag", 0, 1, 3), ("count", 4, 4, 0), ("flag", 8, 1, 3)]
    );
    let line = &program.structures()[1];
    assert_eq!((line.size, line.align), (40, 8));
    assert_eq!(line.members[1].offset, 16, "the pointer is aligned to 8");
    assert_eq!(line.members[0].padding, 4);
    assert_eq!(line.members[2].r#type, "int[3]");
    assert_eq!(line.members[2].first, 4, "leaves: tag, count, flag, p");
    let mut session = program.session();
    let declared = loop {
        let state = session.step().clone();
        if state.event.kind == EventKind::Declare {
            break state;
        }
    };
    assert_eq!(declared.event.title, "Create `e` (struct entry)");
    assert!(
        declared
            .event
            .explanation
            .contains("`tag` (char) at byte 0 followed by 3 bytes of padding"),
        "{}",
        declared.event.explanation
    );
    assert!(cited(&declared).contains(&"6.7.2.1p13"));
    assert!(
        declared.event.note.contains("padding"),
        "{}",
        declared.event.note
    );
}

#[test]
fn members_are_designated_by_dot_and_arrow() {
    let source = "struct point { int x; int y; };\nint main(void) {\n    struct point p = {1, \
                  2};\n    struct point *at = &p;\n    at->y = p.x + 5;\n    return p.y;\n}\n";
    let states = states(source);
    let last = states.last().expect("a final state");
    assert_eq!(
        last.return_value,
        Some(Value::Integer(6)),
        "{:?}",
        last.diagnostics
    );
    let members: Vec<&State> = states
        .iter()
        .filter(|state| state.event.kind == EventKind::Member)
        .collect();
    let arrow = members
        .iter()
        .find(|state| state.event.title.starts_with("`at->y`"))
        .expect("at->y designates a member");
    assert_eq!(arrow.event.title, "`at->y` designates `p.y`");
    assert!(cited(arrow).contains(&"6.5.2.3p4"));
    assert!(
        arrow.event.summary.contains("4 bytes into it"),
        "{}",
        arrow.event.summary
    );
    assert!(matches!(
        arrow.event.result,
        Some(Operand::Address {
            target:    1,
            offset:    1,
            structure: None,
        })
    ));
    let dot = members
        .iter()
        .find(|state| state.event.title.starts_with("`p.x`"))
        .expect("p.x designates a member");
    assert!(cited(dot).contains(&"6.5.2.3p3"));
    assert_eq!(dot.event.kind.category(), "memory");
    // `p` before `.x` is located, not read.
    assert!(
        states
            .iter()
            .any(|state| state.event.title == "`p` holds the member to select")
    );
    // The read of the member names it.
    assert!(
        states
            .iter()
            .any(|state| state.event.title == "Read `p.x` → 1")
    );
    // `->` through a null pointer is undefined.
    let null = run(
        "struct point { int x; int y; };\nint main(void) { struct point *at = 0; return at->x; }\n",
    );
    assert_eq!(null.state().status, "undefined_behavior");
    assert_eq!(
        null.state().diagnostics[0].undefined,
        Some(super::Undefined::NullIndirection)
    );
}

#[test]
fn structures_are_copied_by_assignment_argument_and_return() {
    let source =
        "struct point { int x; int y; };\nstruct point moved(struct point p, int dx) {\n    p.x = \
         p.x + dx;\n    return p;\n}\nint main(void) {\n    struct point a = {1, 2};\n    struct \
         point b;\n    b = a;\n    b.x = 9;\n    struct point c = moved(a, 3);\n    return a.x * \
         100 + b.x * 10 + c.x;\n}\n";
    let states = states(source);
    let last = states.last().expect("a final state");
    assert_eq!(last.status, "finished", "{:?}", last.diagnostics);
    assert_eq!(last.return_value, Some(Value::Integer(100 + 90 + 4)));
    let copy = states
        .iter()
        .find(|state| {
            state.event.kind == EventKind::Assign && state.event.title.starts_with("Copy")
        })
        .expect("b = a copies");
    assert_eq!(copy.event.title, "Copy {.x = 1, .y = 2} into `b`");
    assert!(cited(copy).contains(&"6.5.16.1p1"));
    assert!(
        copy.event.summary.contains("all 2 members"),
        "{}",
        copy.event.summary
    );
    let read = states
        .iter()
        .find(|state| {
            state.event.kind == EventKind::LvalueConversion
                && state.event.title.starts_with("Read `a`")
        })
        .expect("a is read whole");
    assert!(
        read.event.summary.contains("every member"),
        "{}",
        read.event.summary
    );
    let call = states
        .iter()
        .find(|state| state.event.kind == EventKind::Call)
        .expect("moved is called");
    assert!(
        call.event.explanation.contains("copied whole"),
        "{}",
        call.event.explanation
    );
    let returned = states
        .iter()
        .find(|state| state.event.kind == EventKind::Return && state.frames.len() == 1)
        .expect("moved returns");
    assert!(returned.event.explanation.contains("returned by value"));
    let initialized = states
        .iter()
        .find(|state| {
            state.event.kind == EventKind::Initialize
                && state.event.title.starts_with("Initialize `c`")
        })
        .expect("c is initialized from the call");
    assert!(cited(initialized).contains(&"6.7.8p13"));
    assert!(initialized.event.sequence_point);
}

#[test]
fn copying_a_partly_initialized_structure_copies_its_indeterminate_members() {
    let states = states(
        "struct point { int x; int y; };\nint main(void) { struct point a; a.x = 1; struct point \
         b = a; return b.x; }\n",
    );
    let last = states.last().expect("a final state");
    assert_eq!(
        last.return_value,
        Some(Value::Integer(1)),
        "{:?}",
        last.diagnostics
    );
    let read = states
        .iter()
        .find(|state| state.event.title.starts_with("Read `a`"))
        .expect("a is read whole");
    assert!(cited(read).contains(&"6.2.6.1p6"), "{:?}", cited(read));
    assert!(read.event.explanation.contains("still indeterminate"));
    // Reading the indeterminate member itself still stops.
    let stopped = run(
        "struct point { int x; int y; };\nint main(void) { struct point a; a.x = 1; struct point \
         b = a; return b.y; }\n",
    );
    assert_eq!(stopped.state().status, "error");
    assert_eq!(stopped.state().diagnostics[0].kind, "indeterminate");
}

#[test]
fn brace_and_designated_initializers_fill_members_and_zero_the_rest() {
    let states = states(
        "struct inner { int a; int b; };\nstruct outer { int x; struct inner in; int *p; int \
         n[2]; };\nint main(void) {\n    struct outer o = {.in = {.b = 7}, .x = 2};\n    return \
         o.x + o.in.a + o.in.b + o.n[1] + (o.p == 0);\n}\n",
    );
    let last = states.last().expect("a final state");
    assert_eq!(
        last.return_value,
        Some(Value::Integer(2 + 7 + 1)),
        "{:?}",
        last.diagnostics
    );
    let selected: Vec<&str> = states
        .iter()
        .filter(|state| state.event.title.ends_with("to initialize"))
        .map(|state| state.event.title.as_str())
        .collect();
    assert_eq!(
        selected,
        [
            "Select `o.in.b` to initialize",
            "Select `o.x` to initialize",
            "Select `o.in.a` to initialize",
            "Select `o.p` to initialize",
            "Select o.n[0] to initialize",
            "Select o.n[1] to initialize",
        ]
    );
    let first = states
        .iter()
        .find(|state| state.event.title == "Select `o.in.b` to initialize")
        .expect("a designated member");
    assert!(cited(first).contains(&"6.7.8p7"));
    let zero = states
        .iter()
        .find(|state| state.event.title == "Supply an implicit 0")
        .expect("an implicit zero");
    assert!(cited(zero).contains(&"6.7.8p19"));
    // The zero stored into the pointer member makes it null.
    assert!(
        states
            .iter()
            .any(|state| state.event.title == "Initialize `o.p` = NULL")
    );
    // Only the list's last store ends the full expression.
    let stores: Vec<bool> = states
        .iter()
        .filter(|state| state.event.kind == EventKind::Initialize)
        .map(|state| state.event.sequence_point)
        .collect();
    assert_eq!(stores, [false, false, false, false, false, true]);
}

#[test]
fn a_repeated_initializer_replaces_its_whole_subobject() {
    for (declarations, object) in [
        ("", "struct O x = {.p = {1, 2}, .p = {3}};\n    "),
        ("struct O x = {.p = {1, 2}, .p = {3}};\n", ""),
        ("", "struct A x = {.a = {1, 2}, .a = {3}};\n    "),
        ("struct A x = {.a = {1, 2}, .a = {3}};\n", ""),
    ] {
        let source = format!(
            "struct P {{ int a; int b; }};\nstruct O {{ struct P p; }};\nstruct A {{ int a[2]; \
             }};\n{declarations}int main(void) {{\n    {object}return {};\n}}\n",
            if declarations.contains("struct O") || object.contains("struct O") {
                "x.p.a * 10 + x.p.b"
            } else {
                "x.a[0] * 10 + x.a[1]"
            },
        );
        let states = states(&source);
        let last = states.last().expect("a final state");
        assert_eq!(
            last.return_value,
            Some(Value::Integer(30)),
            "{source}: {:?}",
            last.diagnostics
        );
    }
}

#[test]
fn a_static_initializer_with_arithmetic_says_what_the_subset_takes() {
    let Err(diagnostic) = compile("int n = 1 + 2;\nint main(void) {\n    return n;\n}\n") else {
        panic!("arithmetic in a static initializer must be refused");
    };
    assert!(
        diagnostic.message.contains("no arithmetic"),
        "{diagnostic:?}"
    );
}

#[test]
fn pointers_to_members_stay_within_their_member() {
    let source = "struct pair { int a[2]; int b; };\nint main(void) {\n    struct pair s = {{1, \
                  2}, 3};\n    int *pb = &s.b;\n    *pb = 8;\n    int *q = s.a;\n    q = q + \
                  1;\n    return s.b + *q;\n}\n";
    let states = states(source);
    let last = states.last().expect("a final state");
    assert_eq!(
        last.return_value,
        Some(Value::Integer(10)),
        "{:?}",
        last.diagnostics
    );
    assert!(
        states
            .iter()
            .any(|state| state.event.title == "Take the address of `s.b`")
    );
    let decayed = states
        .iter()
        .find(|state| state.event.title == "`s.a` decays to a pointer")
        .expect("s.a decays");
    assert!(cited(decayed).contains(&"6.3.2.1p3"));
    assert!(decayed.event.explanation.contains("a pointer to s.a[0]"));
    // One past s.a is s.b's address, but dereferencing it is undefined.
    let past = run(
        "struct pair { int a[2]; int b; };\nint main(void) { struct pair s = {{1, 2}, 3}; int *q \
         = s.a + 2; return *q; }\n",
    );
    let diagnostic = &past.state().diagnostics[0];
    assert_eq!(diagnostic.undefined, Some(super::Undefined::OnePastEnd));
    assert!(
        diagnostic.message.contains("`s.a` has 2 elements"),
        "{}",
        diagnostic.message
    );
    let beyond = run(
        "struct pair { int a[2]; int b; };\nint main(void) { struct pair s = {{1, 2}, 3}; int *q \
         = &s.b + 1; return *q; }\n",
    );
    assert_eq!(
        beyond.state().diagnostics[0].undefined,
        Some(super::Undefined::OnePastEnd)
    );
}

#[test]
fn schema_three_encodes_structures_and_static_objects() {
    let program = compile(
        "struct point { char c; int y; };\nstruct point origin = {1, 2};\nint main(void) { return \
         origin.y; }\n",
    )
    .expect("compiles");
    let mut session = program.session();
    let _ = session.step();
    let json = session.state_json();
    assert!(
        json.contains(
            "\"kind\":\"struct\",\"structure\":0,\"members\":[{\"kind\":\"integer\",\"value\":1}"
        ),
        "{json}"
    );
    assert!(json.contains("\"storage_duration\":\"static\""), "{json}");
    assert!(json.contains("\"kind\":\"static_storage\",\"category\":\"lifetime\""));
    let trace = session.trace_json();
    assert!(
        trace.contains(
            "\"structures\":[{\"name\":\"point\",\"size\":8,\"align\":4,\"members\":[{\"name\":\"\
             c\",\"type\":\"char\",\"offset\":0,\"size\":1,\"padding\":3,\"first\":0}"
        ),
        "{trace}"
    );
}

#[test]
fn a_member_of_a_first_member_names_the_structure_it_is_in() {
    let states = states(
        "struct inner { int a; int b; };\nstruct outer { struct inner in; int z; };\nint \
         main(void) {\n    struct outer o = {{1, 2}, 3};\n    return o.in.b;\n}\n",
    );
    let last = states.last().expect("a final state");
    assert_eq!(
        last.return_value,
        Some(Value::Integer(2)),
        "{:?}",
        last.diagnostics
    );
    let selected = states
        .iter()
        .find(|state| state.event.title.starts_with("`o.in.b`"))
        .expect("o.in.b designates a member");
    assert!(
        selected
            .event
            .summary
            .contains("inside `o.in`, 4 bytes into it"),
        "{}",
        selected.event.summary
    );
}

/// The bytes a terminated run wrote to standard output.
fn output(source: &str) -> (State, String) {
    let session = run(source);
    let state = session.state().clone();
    let text = String::from_utf8_lossy(&state.output).into_owned();
    (state, text)
}

/// The undefined behavior a run stops with, and its title.
fn undefined(source: &str) -> (Option<super::Undefined>, String) {
    let session = run(source);
    let state = session.state();
    assert_eq!(
        state.status, "undefined_behavior",
        "{source}: {:?}",
        state.diagnostics
    );
    (
        state
            .diagnostics
            .first()
            .and_then(|diagnostic| diagnostic.undefined),
        state.event.title.clone(),
    )
}

fn rejected(source: &str) -> String {
    compile(source).map(|_| ()).expect_err(source).message
}

const HELLO: &str = include_str!("../../web/demos/hello.c");

#[test]
fn the_first_program_prints_through_printf_and_puts() {
    let states = states(HELLO);
    let last = states.last().expect("a final state");
    assert_eq!(last.status, "finished", "{:?}", last.diagnostics);
    assert_eq!(last.return_value, Some(Value::Integer(0)));
    assert_eq!(last.output, b"Hello, world!\nAda has 3 letters\n");
    // Translation phase 4 first, then the string literals' arrays, before
    // main; nothing has a frame yet.
    assert_eq!(states[1].event.kind, EventKind::Directive);
    assert_eq!(states[1].frames.len(), 0, "no call before main");
    assert!(states[1].event.summary.contains("printf, puts and putchar"));
    assert!(cited(&states[1]).contains(&"6.10.2p2"));
    let literals: Vec<&str> = states
        .iter()
        .take_while(|state| state.frames.is_empty() && state.step > 1)
        .chain(states.iter().skip(2).take(2))
        .filter(|state| state.event.kind == EventKind::StaticStorage)
        .map(|state| state.objects.last().expect("an object").name.as_str())
        .collect();
    assert_eq!(
        literals[..2],
        ["\"Hello, world!\"", "\"%s has %d letters\\n\""]
    );
    let format = &states[3].objects[1];
    assert_eq!(
        (
            format.r#type.as_str(),
            format.storage_duration,
            format.literal
        ),
        ("char[19]", "static", true)
    );
    // `char name[] = "Ada";` makes an array of its own, sized by the literal.
    let name = last
        .objects
        .iter()
        .find(|object| object.name == "name")
        .expect("name");
    assert_eq!(name.r#type, "char[4]");
    assert_eq!(
        name.value,
        Value::Array(vec![
            Value::Integer(65),
            Value::Integer(100),
            Value::Integer(97),
            Value::Integer(0)
        ])
    );
    // Each call writes in one step, and that step's bytes are the last
    // ones of the output so far.
    let writes: Vec<(&str, String)> = states
        .iter()
        .filter(|state| state.written > 0)
        .map(|state| {
            let tail = &state.output[state.output.len() - state.written..];
            (
                state.event.title.as_str(),
                String::from_utf8_lossy(tail).into_owned(),
            )
        })
        .collect();
    assert_eq!(
        writes,
        [
            (
                "puts writes \"Hello, world!\\n\"",
                String::from("Hello, world!\n")
            ),
            (
                "printf writes \"Ada has 3 letters\\n\"",
                String::from("Ada has 3 letters\n")
            ),
        ]
    );
    let printf = states
        .iter()
        .find(|state| state.event.title.starts_with("printf"))
        .expect("printf");
    assert_eq!(printf.event.kind, EventKind::LibraryCall);
    let text = &printf.event.explanation;
    assert!(
        text.contains("`%s` takes argument 1 after the format, `name`")
            && text.contains("writes \"Ada\"")
            && text.contains("`%d` takes argument 2 after the format, `len`, the int 3")
            && text.contains("default argument promotions"),
        "{text}"
    );
    assert!(cited(printf).contains(&"7.19.6.1p8") && cited(printf).contains(&"6.5.2.2p7"));
    assert_eq!(
        printf.event.result,
        Some(Operand::Value(Value::Integer(18)))
    );
    // The prototype lets main call length before its definition.
    assert!(states.iter().any(|state| state.frames.len() == 2));
    for state in &states {
        assert!(
            !state.event.title.is_empty() && state.event.summary.ends_with('.'),
            "step {}: {:?}",
            state.step,
            state.event
        );
    }
    // Back restores the output with the rest of the state.
    let program = compile(HELLO).expect("compiles");
    let mut session = program.session();
    while session.state().output.is_empty() {
        let _ = session.step();
    }
    let _ = session.back();
    assert_eq!(session.state().output, b"");
    assert!(session.state_json().contains("\"output\":\"\""));
    let _ = session.step();
    assert!(
        session
            .state_json()
            .contains("\"output\":\"Hello, world!\\n\",\"written\":14")
    );
}

#[test]
fn string_literals_are_static_char_arrays_with_a_null() {
    let states = states(
        "const char *greeting = \"hi\";\nint main(void) {\n    const char *p = \"abc\";\n    int \
         n = 0;\n    while (*p != '\\0') {\n        n++;\n        p++;\n    }\n    return n + \
         greeting[1];\n}\n",
    );
    let last = states.last().expect("final");
    assert_eq!(
        last.return_value,
        Some(Value::Integer(3 + 105)),
        "{:?}",
        last.diagnostics
    );
    // Both literals exist before main, in source order; the file-scope
    // pointer points at the first.
    assert_eq!(last.objects[0].name, "\"hi\"");
    assert_eq!(last.objects[1].name, "greeting");
    assert_eq!(last.objects[2].name, "\"abc\"");
    assert!(matches!(
        last.objects[1].value,
        Value::Pointer {
            target: Some(1),
            offset: 0,
            ..
        }
    ));
    let literal = states
        .iter()
        .find(|state| {
            state
                .event
                .title
                .starts_with("Before main: the string literal \"abc\"")
        })
        .expect("the literal's array is created");
    assert!(
        literal.event.summary.contains("{'a', 'b', 'c', '\\0'}"),
        "{}",
        literal.event.summary
    );
    assert!(cited(literal).contains(&"6.4.5p5"));
    let decay = states
        .iter()
        .find(|state| state.event.kind == EventKind::ArrayDecay)
        .expect("the literal decays");
    assert!(
        decay.event.title.contains("first char"),
        "{}",
        decay.event.title
    );
    // A char read says which character it is, and a character constant
    // its code.
    assert!(
        states
            .iter()
            .any(|state| state.event.title == "Read `p`'s target → 97 ('a')"
                || state.event.summary.contains("97 ('a')"))
    );
    let constant = states
        .iter()
        .find(|state| state.event.title == "Evaluate the character constant '\\0'")
        .expect("'\\0' evaluates");
    assert!(cited(constant).contains(&"6.4.4.4p10"));
    // Pointer arithmetic counts chars.
    let moved = self::states("int main(void) { char s[] = \"ab\"; char *p = s + 1; return *p; }");
    assert_eq!(
        moved.last().and_then(|state| state.return_value.clone()),
        Some(Value::Integer(98))
    );
    assert!(
        moved
            .iter()
            .any(|state| state.event.explanation.contains("moves 1 char forward"))
    );
}

#[test]
fn char_arrays_take_string_and_brace_initializers() {
    returned(
        "int main(void) { char s[8] = \"hi\"; return s[0] + s[1] + s[2] + s[7]; }",
        104 + 105,
    );
    returned("int main(void) { char s[2] = \"hi\"; return s[1]; }", 105);
    returned(
        "int main(void) { char s[3] = {'a', 98}; return s[1] + s[2]; }",
        98,
    );
    returned(
        "struct entry { char tag[4]; int count; };\nint main(void) {\n    struct entry e = \
         {\"ab\", 3};\n    return e.tag[1] + e.count + e.tag[3];\n}\n",
        98 + 3,
    );
    returned(
        "char name[] = \"Ada\";\nint main(void) { return name[3] + name[2]; }",
        97,
    );
    returned(
        "int main(void) { char e[] = \"\\x41\\101\\t\\\\\"; return e[0] + e[1] + e[2] + e[3] + \
         e[4]; }",
        65 + 65 + 9 + 92,
    );
    returned("int main(void) { return '\\xff' + 'A'; }", -1 + 65);
    let states = states("int main(void) { char s[8] = \"hi\"; return 0; }");
    let initialized = states
        .iter()
        .find(|state| state.event.title == "Initialize `s` from \"hi\"")
        .expect("one step initializes the array");
    assert!(initialized.event.sequence_point);
    assert!(
        initialized
            .event
            .summary
            .contains("the 5 elements after them are zero"),
        "{}",
        initialized.event.summary
    );
    assert!(cited(initialized).contains(&"6.7.8p14"));
    for (source, message) in [
        (
            "int main(void) { char s[1] = \"hi\"; return 0; }",
            "more than the 1 elements",
        ),
        ("int main(void) { char s[]; return 0; }", "unknown size"),
        (
            "int main(void) { char c = 'ab'; return 0; }",
            "implementation-defined",
        ),
        ("int main(void) { char c = L'a'; return 0; }", "wide"),
        (
            "int main(void) { char s[] = \"\\q\"; return 0; }",
            "unknown escape",
        ),
        (
            "int main(void) { char s[] = \"open; return 0; }",
            "unterminated string",
        ),
        (
            "struct s { const int x; }; int main(void) { return 0; }",
            "const members",
        ),
    ] {
        let found = rejected(source);
        assert!(found.contains(message), "{source}: {found}");
    }
}

#[test]
fn modifying_a_string_literal_is_undefined_and_const_is_enforced() {
    let (cause, title) = undefined("int main(void) { char *p = \"hi\"; p[0] = 'H'; return 0; }");
    assert_eq!(cause, Some(super::Undefined::StringLiteralWrite));
    assert_eq!(title, "Undefined behavior: modifying a string literal");
    let (cause, _) = undefined("int main(void) { char *p = \"hi\"; (*p)++; return 0; }");
    assert_eq!(cause, Some(super::Undefined::StringLiteralWrite));
    // A copy in an array of one's own may change.
    returned(
        "int main(void) { char s[] = \"hi\"; s[0] = 'H'; return s[0]; }",
        72,
    );
    for source in [
        "int main(void) { const char *p = \"hi\"; *p = 'H'; return 0; }",
        "int main(void) { const char *p = \"hi\"; p[1]++; return 0; }",
    ] {
        assert!(
            rejected(source).contains("not a modifiable lvalue"),
            "{source}"
        );
    }
    assert!(
        rejected("int main(void) { const char *p = \"hi\"; char *q = p; return 0; }")
            .contains("does not convert to a char *")
    );
}

#[test]
fn printf_converts_each_argument_with_flags_and_widths() {
    let (state, text) = output(
        "#include <stdio.h>\nint main(void) {\n    char c = 'A';\n    int n = \
         printf(\"%d|%i|%u|%x|%c|%c|%s|%%|%5d|%-5d|%05d|%3s|%-3c|\\n\", -42, 7, 42, 255, c, 66, \
         \"ab\", 42, 42, -42, \"x\", 'y');\n    return n;\n}\n",
    );
    let expected = "-42|7|42|ff|A|B|ab|%|   42|42   |-0042|  x|y  |\n";
    assert_eq!(text, expected);
    assert_eq!(
        state.return_value,
        Some(Value::Integer(
            i128::try_from(expected.len()).expect("short")
        ))
    );
    let trace = states(
        "#include <stdio.h>\nint main(void) {\n    char c = 'A';\n    printf(\"%c\", c);\n    \
         return 0;\n}\n",
    );
    let promoted = trace
        .iter()
        .find(|state| state.event.title == "Promote char 65 to int for `...`")
        .expect("a char argument is promoted");
    assert!(cited(promoted).contains(&"6.5.2.2p6"));
    assert!(promoted.event.summary.contains("('A')"));
    // putchar and puts return what C says.
    let (state, text) = output(
        "#include <stdio.h>\nint main(void) {\n    int a = putchar('o');\n    int b = \
         putchar('k');\n    int c = puts(\"\");\n    return a + b + c;\n}\n",
    );
    assert_eq!(text, "ok\n");
    assert_eq!(state.return_value, Some(Value::Integer(111 + 107 + 1)));
    let (state, _) = output("#include <stdio.h>\nint main(void) { return putchar(-56); }\n");
    assert_eq!(state.output, [200]);
    assert_eq!(state.return_value, Some(Value::Integer(200)));
    // A second #include of the header is a step that changes nothing.
    let twice = states("#include <stdio.h>\n#include <stdio.h>\nint main(void) { return 0; }\n");
    assert_eq!(kinds(&twice)[1..3], ["directive", "directive"]);
    assert!(cited(&twice[2]).contains(&"7.1.2p4"));
}

#[test]
fn printf_stops_on_undefined_conversions_and_arguments() {
    use super::Undefined::{
        InvalidConversion,
        MissingArgument,
        NullString,
        Unterminated,
        WrongArgument,
    };
    for (body, expected) in [
        ("int x = 1; printf(\"%d\", &x);", WrongArgument),
        ("printf(\"%s\", 5);", WrongArgument),
        ("printf(\"%u\", -1);", WrongArgument),
        ("printf(\"%d %d\", 1);", MissingArgument),
        ("printf(\"%y\", 1);", InvalidConversion),
        ("printf(\"%05s\", \"a\");", InvalidConversion),
        ("printf(\"100%\");", InvalidConversion),
        ("printf(\"%5%\");", InvalidConversion),
        ("char s[2] = \"hi\"; printf(\"%s\", s);", Unterminated),
        ("char *p = 0; puts(p);", NullString),
        ("char *p = 0; printf(p);", NullString),
    ] {
        let source = format!("#include <stdio.h>\nint main(void) {{ {body} return 0; }}\n");
        let (cause, _) = undefined(&source);
        assert_eq!(cause, Some(expected), "{body}");
    }
    let (_, title) =
        undefined("#include <stdio.h>\nint main(void) { printf(\"%d\\n\"); return 0; }\n");
    assert_eq!(title, "Undefined behavior: printf has too few arguments");
    // Extra arguments are evaluated and ignored.
    let (state, text) =
        output("#include <stdio.h>\nint main(void) { printf(\"x\", 1); return 0; }\n");
    assert_eq!((state.status, text.as_str()), ("finished", "x"));
    // Valid C the machine does not model stops without blaming the program.
    for body in [
        "printf(\"%o\", 1);",
        "printf(\"%*d\", 2, 1);",
        "printf(\"%jd\", 1);",
        "printf(\"%+d\", 1);",
    ] {
        let source = format!("#include <stdio.h>\nint main(void) {{ {body} return 0; }}\n");
        let session = run(&source);
        assert_eq!(session.state().status, "error", "{body}");
        assert_eq!(session.state().diagnostics[0].kind, "unsupported", "{body}");
    }
    // Output is bounded.
    let session = run(
        "#include <stdio.h>\nint main(void) {\n    for (int i = 0; i < 400; i++) {\n        \
         puts(\"0123456789abcdef\");\n    }\n    return 0;\n}\n",
    );
    assert_eq!(session.state().status, "limit");
    assert!(session.state().output.len() <= 4096);
}

#[test]
fn the_machine_runs_only_the_stdio_include() {
    for (source, message) in [
        (
            "int main(void) { printf(\"hi\"); return 0; }",
            "#include <stdio.h>",
        ),
        (
            "#undef X\nint main(void) { return 0; }",
            "only #include <stdio.h> and object-like #define",
        ),
        (
            "#include <stdlib.h>\nint main(void) { return 0; }",
            "only <stdio.h>",
        ),
        (
            "int main(void) {\n#include <stdio.h>\nreturn 0; }",
            "directive inside a function",
        ),
        (
            "#include <stdio.h>\nint puts(int x) { return x; }\nint main(void) { return 0; }",
            "already declared by <stdio.h>",
        ),
        (
            "int puts(int x) { return x; }\n#include <stdio.h>\nint main(void) { return 0; }",
            "already declares",
        ),
        (
            "#include <stdio.h>\nint main(void) { puts(1); return 0; }",
            "puts's argument is a string",
        ),
        (
            "#include <stdio.h>\nint main(void) { printf(); return 0; }",
            "at least its format",
        ),
        (
            "int x; #include <stdio.h>\nint main(void) { return 0; }",
            "unsupported character",
        ),
    ] {
        let found = rejected(source);
        assert!(found.contains(message), "{source}: {found}");
    }
    // A comment may follow the header name.
    returned(
        "#include <stdio.h> // output\nint main(void) { return 0; }\n",
        0,
    );
}

#[test]
fn a_prototype_lets_a_call_come_before_the_definition() {
    returned(
        "int f(int);\nint main(void) { return f(1); }\nint f(int x) { return x + 41; }\n",
        42,
    );
    returned(
        "int twice(int value);\nint twice(int);\nint main(void) { return twice(4); }\nint \
         twice(int n) { return n * 2; }\n",
        8,
    );
    returned(
        "int first(const char *s);\nint main(void) { return first(\"z\"); }\nint first(const char \
         *s) { return s[0]; }\n",
        122,
    );
    for (source, message) in [
        (
            "int main(void) { return f(1); }\nint f(int x) { return x; }\n",
            "declare it first with a prototype",
        ),
        (
            "int f(int);\nint main(void) { return f(1); }\n",
            "declared but never defined",
        ),
        (
            "int f(int);\nint main(void) { return 0; }\nint f(int *x) { return 0; }\n",
            "differs from the function's prototype",
        ),
        (
            "int f();\nint main(void) { return 0; }\n",
            "without a prototype",
        ),
        (
            "int f(int);\nint main(void) { return 0; }\nint f(int) { return 0; }\n",
            "names each of its parameters",
        ),
    ] {
        let found = rejected(source);
        assert!(found.contains(message), "{source}: {found}");
    }
    // Arguments are checked against the prototype when the call is compiled.
    assert!(
        rejected("int f(int *p);\nint main(void) { return f(3); }\nint f(int *p) { return 0; }\n")
            .contains("argument type")
    );
}

const CONSTANTS: &str = include_str!("../../web/demos/constants.c");

#[test]
fn the_constants_lesson_narrates_define_const_cast_and_compound_assignment() {
    let states = states(CONSTANTS);
    let last = states.last().expect("a final state");
    assert_eq!(last.status, "finished", "{:?}", last.diagnostics);
    assert_eq!(last.output, b"total is 056\n");
    // Phase 4 runs the include and both definitions, one step each.
    let directives: Vec<&str> = states
        .iter()
        .filter(|state| state.event.kind == EventKind::Directive)
        .map(|state| state.event.title.as_str())
        .collect();
    assert_eq!(
        directives,
        [
            "#include <stdio.h>: declare printf, puts and putchar",
            "#define LIMIT: LIMIT stands for 4",
            "#define LABEL: LABEL stands for \"total\"",
        ]
    );
    // Every use of LIMIT is narrated as its replacement.
    let limit = states
        .iter()
        .find(|state| state.event.title.starts_with("Macro LIMIT"))
        .expect("the loop condition evaluates LIMIT");
    assert_eq!(limit.event.title, "Macro LIMIT: evaluate the constant 4");
    assert!(
        limit.event.summary.starts_with(
            "`LIMIT` is a macro the preprocessor replaced with `4` before the program ran: the \
             constant 4"
        ),
        "{}",
        limit.event.summary
    );
    assert_eq!(cited(limit)[0], "6.10.3p9");
    assert!(limit.event.explanation.contains("the #define on line 5"));
    // LABEL " is %.3d\n" is one literal after phase 6.
    let joined = states
        .iter()
        .find(|state| {
            state
                .event
                .title
                .starts_with("Before main: 2 literals joined")
        })
        .expect("the joined literal is created before main");
    assert!(
        joined.event.title.ends_with("from the macro LABEL"),
        "{}",
        joined.event.title
    );
    assert!(cited(joined).contains(&"6.4.5p4"));
    assert!(
        last.objects
            .iter()
            .any(|object| object.literal && object.name == "\"total\" \" is %.3d\\n\"")
    );
    // const is shown in the type and explained at the declaration.
    let step = last
        .objects
        .iter()
        .find(|object| object.name == "step")
        .expect("step");
    assert_eq!(step.r#type, "const int");
    let declared = states
        .iter()
        .find(|state| state.event.title == "Create `step` (const int)")
        .expect("step is declared");
    assert!(declared.event.explanation.contains("defined const"));
    // The cast wraps, and says the result is implementation-defined.
    let cast = states
        .iter()
        .find(|state| state.event.kind == EventKind::Cast)
        .expect("the cast");
    assert_eq!(cast.event.title, "Cast 300 to char → 44");
    assert_eq!(
        cast.event.summary,
        "300 does not fit in a char, so the result is implementation-defined; bcc and GCC wrap it \
         to 44."
    );
    assert!(cited(cast).contains(&"6.3.1.3p3"));
    // += is E1 = E1 + (E2), with E1 evaluated once.
    let updates: Vec<&State> = states
        .iter()
        .filter(|state| state.event.kind == EventKind::CompoundAssign)
        .collect();
    assert_eq!(updates.len(), 5);
    assert_eq!(updates[0].event.title, "Update `total`: 0 → 3");
    assert!(
        updates[0]
            .event
            .summary
            .contains("`total += step` is `total = total + (step)` with total evaluated once"),
        "{}",
        updates[0].event.summary
    );
    assert!(updates[0].event.sequence_point);
    assert_eq!(cited(updates[0])[0], "6.5.16.2p3");
    // The precision pads 56 to three digits.
    let printed = states
        .iter()
        .find(|state| state.event.kind == EventKind::LibraryCall)
        .expect("printf");
    assert!(
        printed
            .event
            .explanation
            .contains("the precision .3 asks for at least 3 digits, so 1 leading zero is added"),
        "{}",
        printed.event.explanation
    );
    assert!(cited(printed).contains(&"7.19.6.1p4"));
}

#[test]
fn object_like_macros_replace_every_later_use() {
    let source = "#define N 3\n#define FIRST 'A'\n#define N 3\nint table[N];\nint main(void) { \
                  int a[N] = {N}; char c = FIRST; return a[0] + table[2] + c - N; }";
    let states = states(source);
    let last = states.last().expect("a final state");
    assert_eq!(last.return_value, Some(Value::Integer(65)));
    // An identical redefinition is allowed and changes nothing.
    assert!(
        states
            .iter()
            .any(|state| state.event.title == "#define N again: nothing changes")
    );
    // The array sizes say where their macro went.
    let table = states
        .iter()
        .find(|state| state.event.kind == EventKind::StaticStorage)
        .expect("table");
    assert!(
        table
            .event
            .explanation
            .contains("In the declaration, `N` is not a variable"),
        "{}",
        table.event.explanation
    );
    let character = states
        .iter()
        .find(|state| state.event.title.starts_with("Macro FIRST"))
        .expect("FIRST");
    assert_eq!(
        character.event.title,
        "Macro FIRST: evaluate the character constant 'A'"
    );
    // A use before the definition is not replaced: the name is undeclared.
    assert!(rejected("int main(void) { return N; }\n#define N 1\n").contains("undeclared"));
    // A different redefinition breaks a constraint.
    let diagnostic = compile("#define N 1\n#define N 2\nint main(void) { return N; }")
        .map(|_| ())
        .expect_err("redefinition");
    assert_eq!(diagnostic.kind, "constraint");
    assert_eq!(diagnostic.clause.map(|clause| clause.id), Some("6.10.3p2"));
    // Only constants and literals; function-like macros stay the
    // preprocessor's.
    for (source, message) in [
        (
            "#define SQ(x) ((x) * (x))\nint main(void) { return 0; }",
            "function-like",
        ),
        (
            "#define N (1 + 2)\nint main(void) { return 0; }",
            "one integer constant",
        ),
        (
            "#define N -1\nint main(void) { return 0; }",
            "one integer constant",
        ),
        (
            "#define N 010\nint main(void) { return 0; }",
            "one integer constant",
        ),
        (
            "#define int long\nint main(void) { return 0; }",
            "macro name",
        ),
        (
            "#define N\nint main(void) { return 0; }",
            "one integer constant",
        ),
        (
            "#define C 'ab'\nint main(void) { return 0; }",
            "implementation-defined",
        ),
    ] {
        let found = rejected(source);
        assert!(found.contains(message), "{source}: {found}");
    }
}

#[test]
fn adjacent_string_literals_are_one_array() {
    let source = "#include <stdio.h>\nint main(void) {\n    char s[] = \"ab\" \"cd\";\n    \
                  printf(\"%s\" \"!\" \"\\n\", s);\n    puts(\"\\x4\" \"1\");\n    return 0;\n}\n";
    let (state, text) = output(source);
    assert_eq!(state.status, "finished", "{:?}", state.diagnostics);
    // Phase 6 joins after escapes are replaced: "\x4" "1" is two characters.
    assert_eq!(text, "abcd!\n\u{4}1\n");
    let s = state
        .objects
        .iter()
        .find(|object| object.name == "s")
        .expect("s");
    assert_eq!(s.r#type, "char[5]");
    let format = state
        .objects
        .iter()
        .find(|object| object.literal && object.name.starts_with("\"%s\""))
        .expect("the joined format");
    assert_eq!(format.r#type, "char[5]");
    let states = states(source);
    let initialized = states
        .iter()
        .find(|state| state.event.title.starts_with("Initialize `s`"))
        .expect("s is initialized");
    assert!(
        initialized
            .event
            .explanation
            .contains("2 adjacent string literals")
    );
    assert!(cited(initialized).contains(&"6.4.5p4"));
}

#[test]
fn const_objects_reject_assignment_and_stop_a_store_through_a_cast() {
    for (source, message, clause) in [
        (
            "int main(void) { const int limit = 3; limit = 4; return 0; }",
            "`limit` is defined const",
            "6.5.16p2",
        ),
        (
            "int main(void) { const int limit = 3; limit += 1; return 0; }",
            "assignment's left operand",
            "6.5.16p2",
        ),
        (
            "int main(void) { const char c = 'a'; c++; return 0; }",
            "++ and -- need a modifiable lvalue",
            "6.5.2.4p1",
        ),
        (
            "const int table[2] = {1, 2};\nint main(void) { table[0] = 5; return 0; }",
            "pointer to const",
            "6.5.16p2",
        ),
        (
            "int main(void) { const int limit = 3; int *p = &limit; return 0; }",
            "a const int * does not convert to a int *",
            "6.5.16.1p1",
        ),
    ] {
        let diagnostic = compile(source).map(|_| ()).expect_err(source);
        assert_eq!(diagnostic.kind, "constraint", "{source}");
        assert!(
            diagnostic.message.contains(message),
            "{source}: {}",
            diagnostic.message
        );
        assert_eq!(
            diagnostic.clause.map(|clause| clause.id),
            Some(clause),
            "{source}"
        );
    }
    // Reading through a pointer that lost its const is fine; storing is not.
    returned(
        "int main(void) { const int limit = 3; const int *p = &limit; int *q = (int *)p; return \
         *q; }",
        3,
    );
    let (undefined_kind, title) = undefined(
        "const int limit = 3;\nint main(void) { int *p = (int *)&limit; *p = 4; return limit; }",
    );
    assert_eq!(undefined_kind, Some(super::Undefined::ConstWrite));
    assert_eq!(title, "Undefined behavior: modifying a const object");
    let session = run("int main(void) { const int n = 1; int *p = (int *)&n; (*p)++; return n; }");
    assert_eq!(
        session.state().diagnostics[0]
            .clause
            .map(|clause| clause.id),
        Some("6.7.3p5")
    );
    // The cast that removes const says what it allows and what it does not.
    let states = states(
        "int main(void) { int x = 1; const int *p = &x; int *q = (int *)p; *q = 2; return x; }",
    );
    let cast = states
        .iter()
        .find(|state| state.event.kind == EventKind::Cast)
        .expect("the cast");
    assert_eq!(cast.event.title, "Cast to int *: remove const");
    assert!(cited(cast).contains(&"6.7.3p5"));
    // x itself is not const, so the store is fine.
    assert_eq!(states.last().unwrap().return_value, Some(Value::Integer(2)));
}

#[test]
fn casts_between_int_and_char_narrate_the_conversion() {
    returned("int main(void) { char c = (char)300; return c; }", 44);
    returned(
        "int main(void) { return (char)-129 + (char)200; }",
        127 - 56,
    );
    returned(
        "int main(void) { char c = 'A'; return (int)c + (char)1; }",
        66,
    );
    let states = states(
        "int main(void) { char c = 'A'; int a = (int)c; int b = (int)a; char d = (char)65; return \
         a + b + d; }",
    );
    let casts: Vec<&State> = states
        .iter()
        .filter(|state| state.event.kind == EventKind::Cast)
        .collect();
    assert_eq!(casts.len(), 3);
    assert_eq!(casts[0].event.title, "Cast 65 to int: the value fits");
    assert!(cited(casts[0]).contains(&"6.3.1.3p1"));
    assert_eq!(casts[1].event.title, "Cast to int: no conversion, 65");
    assert_eq!(cited(casts[1]), ["6.5.4p4"]);
    assert_eq!(casts[2].event.title, "Cast 65 to char: the value fits");
    assert_eq!(casts[0].event.kind.category(), "convert");
    for (source, message) in [
        ("int main(void) { return (void)1; }", "void"),
        (
            "int main(void) { int x = 0; int *p = (int *)x; return 0; }",
            "integers and pointers",
        ),
        (
            "int main(void) { int x = 0; char *p = (char *)&x; return 0; }",
            "different pointer types",
        ),
        (
            "int main(void) { int x = 0; (int)x = 1; return 0; }",
            "modifiable scalar lvalue",
        ),
        (
            "int main(void) { int x = 0; return (void)x, 0; }",
            "cast to void",
        ),
    ] {
        let found = rejected(source);
        assert!(found.contains(message), "{source}: {found}");
    }
}

#[test]
fn compound_assignments_update_once_and_stop_where_the_operator_would() {
    returned(
        "int main(void) { int x = 10; x += 5; x -= 3; x *= 4; x /= 5; x %= 7; return x; }",
        2,
    );
    returned(
        "int main(void) { int a[3] = {1, 2, 3}; int *p = a; p += 2; p -= 1; return *p; }",
        2,
    );
    // A char is promoted for the arithmetic and converted back to char.
    let states = states("int main(void) { char c = 100; c += 100; return c; }");
    let update = states
        .iter()
        .find(|state| state.event.kind == EventKind::CompoundAssign)
        .expect("c += 100");
    assert_eq!(update.event.title, "Update `c`: 100 → -56");
    assert!(update.event.explanation.contains("implementation-defined"));
    // The left operand is designated once, as an update.
    let designated = states
        .iter()
        .find(|state| state.event.title == "`c` is the object to update")
        .expect("c is designated as the object to update");
    assert!(cited(designated).contains(&"6.5.16.2p3"));
    // The same undefined behavior as the plain operators.
    assert_eq!(
        undefined("int main(void) { int x = 2147483647; x += 1; return 0; }").0,
        Some(super::Undefined::SignedOverflow)
    );
    assert_eq!(
        undefined("int main(void) { int x = 1; int z = 0; x /= z; return 0; }").0,
        Some(super::Undefined::DivisionByZero)
    );
    assert_eq!(
        undefined("int main(void) { int x = 1; x %= 0; return 0; }").0,
        Some(super::Undefined::DivisionByZero)
    );
    assert_eq!(
        undefined("int main(void) { int a[2] = {1, 2}; int *p = a; p += 3; return 0; }").0,
        Some(super::Undefined::PointerOutside)
    );
    // A call may be the whole right operand.
    returned(
        "int two(void) { return 2; } int main(void) { int x = 1; x += two(); return x; }",
        3,
    );
    for source in [
        "int main(void) { int a[2] = {1, 2}; int *p = a; p *= 2; return 0; }",
        "int main(void) { int x = 1; int *p = &x; x += p; return 0; }",
    ] {
        let diagnostic = compile(source).map(|_| ()).expect_err(source);
        assert_eq!(diagnostic.kind, "constraint", "{source}");
        assert_eq!(
            diagnostic.clause.map(|clause| clause.id),
            Some("6.5.16.2p1")
        );
    }
    assert!(
        rejected("int main(void) { int x = 1; int y = x += 1; return y; }").contains("expected")
    );
}

#[test]
fn printf_precision_sets_minimum_digits_and_maximum_characters() {
    for (body, expected) in [
        (
            "printf(\"%.3d|%5.2d|%-6.3i|\", 7, -4, 12);",
            "007|  -04|012   |",
        ),
        ("printf(\"%.0d|%.d|%.1d|\", 0, 0, 0);", "||0|"),
        (
            "printf(\"%.4x|%.2u|%05.2d|\", 255, 3, 9);",
            "00ff|03|   09|",
        ),
        (
            "printf(\"%.2s|%5.1s|%-4.9s|\", \"Ada\", \"Bo\", \"Cy\");",
            "Ad|    B|Cy  |",
        ),
    ] {
        let source = format!("#include <stdio.h>\nint main(void) {{ {body} return 0; }}\n");
        let (state, text) = output(&source);
        assert_eq!(state.status, "finished", "{body}: {:?}", state.diagnostics);
        assert_eq!(text, expected, "{body}");
    }
    // A precision lets %s print part of an array that has no null.
    let (state, text) = output(
        "#include <stdio.h>\nint main(void) { char s[3] = {'a', 'b', 'c'}; printf(\"%.2s\", s); \
         return 0; }\n",
    );
    assert_eq!((state.status, text.as_str()), ("finished", "ab"));
    let states = states(
        "#include <stdio.h>\nint main(void) { printf(\"%.2s %05.3d\", \"Ada\", 7); return 0; }\n",
    );
    let printed = states
        .iter()
        .find(|state| state.event.kind == EventKind::LibraryCall)
        .expect("printf");
    for phrase in [
        "the precision .2 writes at most 2 characters of the string",
        "the precision .3 asks for at least 3 digits, so 2 leading zeros are added; with a \
         precision the 0 flag is ignored",
    ] {
        assert!(
            printed.event.explanation.contains(phrase),
            "{}",
            printed.event.explanation
        );
    }
    // A precision with %c is undefined; a precision beyond the bound is a
    // limit.
    let (kind, _) =
        undefined("#include <stdio.h>\nint main(void) { printf(\"%.2c\", 'a'); return 0; }\n");
    assert_eq!(kind, Some(super::Undefined::InvalidConversion));
    let session = run("#include <stdio.h>\nint main(void) { printf(\"%.300d\", 1); return 0; }\n");
    assert_eq!(session.state().status, "limit");
}

#[test]
fn integer_types_have_their_x86_64_sizes_and_say_what_c_promises() {
    let source = "struct mixed { char c; long l; unsigned short s; };\nint main(void) {\n    \
                  signed char a = -1;\n    unsigned char b = 255;\n    short c = -32768;\n    \
                  unsigned short d = 65535;\n    unsigned int e = 4294967295u;\n    long f = \
                  -1;\n    unsigned long g = 1;\n    long long h = -1;\n    unsigned long long i \
                  = 1;\n    long int j = 0;\n    int unsigned k = 0;\n    unsigned long l[2] = \
                  {1, 2};\n    struct mixed m = {1, 2, 3};\n    return 0;\n}\n";
    let states = states(source);
    let last = states.last().expect("a final state");
    assert_eq!(last.status, "finished", "{:?}", last.diagnostics);
    let objects: Vec<(&str, &str, usize)> = last
        .objects
        .iter()
        .map(|object| (object.name.as_str(), object.r#type.as_str(), object.size))
        .collect();
    assert_eq!(
        objects,
        [
            ("a", "signed char", 1),
            ("b", "unsigned char", 1),
            ("c", "short", 2),
            ("d", "unsigned short", 2),
            ("e", "unsigned int", 4),
            ("f", "long", 8),
            ("g", "unsigned long", 8),
            ("h", "long long", 8),
            ("i", "unsigned long long", 8),
            ("j", "long", 8),
            ("k", "unsigned int", 4),
            ("l", "unsigned long[2]", 16),
            ("m", "struct mixed", 24),
        ]
    );
    let declared = states
        .iter()
        .find(|state| state.event.title == "Create `e` (unsigned int)")
        .expect("e is declared");
    assert!(
        declared
            .event
            .explanation
            .contains("4 bytes on x86-64, 0 to 4294967295; C promises only 0 to 65535"),
        "{}",
        declared.event.explanation
    );
    assert!(cited(declared).contains(&"5.2.4.2.1p1"));
    assert!(declared.event.note.contains("minimum ranges"));
    // `int` and `char` keep their notes on indeterminate values.
    returned("int main(void) { int x = 3; char y = 4; return x + y; }", 7);
}

#[test]
fn unsigned_arithmetic_wraps_where_signed_arithmetic_stops() {
    let source = "int main(void) {\n    unsigned int u = 2;\n    u = u - 3;\n    return u == \
                  4294967295u;\n}\n";
    returned(source, 1);
    let states = states(source);
    let wrapped = states
        .iter()
        .find(|state| state.event.kind == EventKind::Arithmetic)
        .expect("the subtraction");
    assert_eq!(wrapped.event.title, "Compute 2 - 3 = 4294967295 (wrapped)");
    assert!(
        wrapped
            .event
            .summary
            .contains("is -1, outside unsigned int, so the result is reduced modulo 2^32"),
        "{}",
        wrapped.event.summary
    );
    assert!(
        wrapped.event.explanation.contains("-1 + 2^32 = 4294967295"),
        "{}",
        wrapped.event.explanation
    );
    assert!(cited(wrapped).contains(&"6.2.5p9"));
    assert!(wrapped.event.note.contains("never overflows"));
    assert_eq!(wrapped.event.result_type.as_deref(), Some("unsigned int"));
    // The widest unsigned type wraps too, products included.
    returned(
        "int main(void) { unsigned long long m = 18446744073709551615ull; m = m + 1; return m == \
         0; }",
        1,
    );
    returned(
        "int main(void) { unsigned long long m = 18446744073709551615ull; return m * m == 1; }",
        1,
    );
    returned("int main(void) { unsigned int u = 0; return -u == 0; }", 1);
    returned(
        "int main(void) { unsigned int u = 1; return -u == 4294967295u; }",
        1,
    );
    // Signed types of every width still stop.
    let (cause, title) =
        undefined("int main(void) { long big = 9223372036854775807; big = big + 1; return 0; }");
    assert_eq!(cause, Some(super::Undefined::SignedOverflow));
    assert_eq!(title, "Undefined behavior: signed overflow");
    let session =
        run("int main(void) { long big = 9223372036854775807; big = big + 1; return 0; }");
    let message = &session.state().diagnostics[0].message;
    assert!(
        message.contains("9223372036854775808, which does not fit in a long"),
        "{message}"
    );
    assert!(
        session
            .state()
            .event
            .summary
            .contains("an unsigned type would have wrapped"),
        "{}",
        session.state().event.summary
    );
    let (cause, _) =
        undefined("int main(void) { long long m = -9223372036854775807ll - 1; return -m == 0; }");
    assert_eq!(cause, Some(super::Undefined::SignedOverflow));
}

#[test]
fn the_usual_arithmetic_conversions_say_which_operand_converts_and_why() {
    let source = "int main(void) { return -1 < 1u; }";
    returned(source, 0);
    let steps = states(source);
    let conversions: Vec<&State> = steps
        .iter()
        .filter(|state| state.event.kind == EventKind::ArithmeticConversion)
        .collect();
    assert_eq!(conversions.len(), 1);
    let event = &conversions[0].event;
    assert_eq!(event.title, "Convert -1 to unsigned int → 4294967295");
    assert_eq!(
        event.summary,
        "`-1 < 1u` mixes an int and an unsigned int, so the left operand converts to unsigned \
         int: -1 + 2^32 = 4294967295."
    );
    assert!(
        event
            .explanation
            .contains("int and unsigned int have the same rank"),
        "{}",
        event.explanation
    );
    assert_eq!(
        cited(conversions[0]),
        ["6.3.1.8p1", "6.3.1.1p1", "6.3.1.3p2"]
    );
    assert_eq!(event.kind.category(), "convert");
    assert_eq!(event.from_type.as_deref(), Some("int"));
    assert_eq!(event.result_type.as_deref(), Some("unsigned int"));
    assert!(event.note.contains("-Wsign-compare"));
    let comparison = steps
        .iter()
        .find(|state| state.event.kind == EventKind::Comparison)
        .expect("the comparison");
    assert_eq!(comparison.event.title, "Compare 4294967295 < 1 → 0");
    assert!(
        comparison
            .event
            .explanation
            .contains("common type unsigned int"),
        "{}",
        comparison.event.explanation
    );
    // A long holds every unsigned int, so the unsigned operand converts.
    let source = "int main(void) { long l = -1; unsigned int u = 1; return l < u; }";
    returned(source, 1);
    let steps = states(source);
    let conversion = steps
        .iter()
        .find(|state| state.event.kind == EventKind::ArithmeticConversion)
        .expect("a conversion");
    assert_eq!(conversion.event.title, "Convert 1 to long");
    assert!(
        conversion
            .event
            .explanation
            .contains("is wider (64 bits against 32)"),
        "{}",
        conversion.event.explanation
    );
    // A long long is no wider than an unsigned long: both convert.
    let source = "int main(void) { long long a = -1; unsigned long b = 1; return a < b; }";
    returned(source, 0);
    let titles: Vec<String> = states(source)
        .iter()
        .filter(|state| state.event.kind == EventKind::ArithmeticConversion)
        .map(|state| state.event.title.clone())
        .collect();
    assert_eq!(
        titles,
        [
            "Convert -1 to unsigned long long → 18446744073709551615",
            "Convert 1 to unsigned long long"
        ]
    );
    // Two ints, or a char and an int, need no conversion step.
    for source in [
        "int main(void) { int a = 1; return a + 2; }",
        "int main(void) { char c = 1; return c + 2; }",
    ] {
        assert!(
            !kinds(&states(source)).contains(&"arithmetic_conversion"),
            "{source}"
        );
    }
}

#[test]
fn every_type_narrower_than_int_is_promoted() {
    let source = "int main(void) { unsigned char c = 255; short s = -2; unsigned short u = 65535; \
                  return c + s + u; }";
    returned(source, 65788);
    let titles: Vec<String> = states(source)
        .iter()
        .filter(|state| state.event.kind == EventKind::IntegerPromotion)
        .map(|state| state.event.title.clone())
        .collect();
    assert_eq!(
        titles,
        [
            "Promote unsigned char 255 to int",
            "Promote short -2 to int",
            "Promote unsigned short 65535 to int"
        ]
    );
    let steps = states(source);
    let promotion = steps
        .iter()
        .find(|state| state.event.kind == EventKind::IntegerPromotion)
        .expect("a promotion");
    assert!(
        promotion
            .event
            .explanation
            .contains("promotes to int, not to unsigned int"),
        "{}",
        promotion.event.explanation
    );
    assert_eq!(cited(promotion), ["6.3.1.1p2", "6.3.1.1p1"]);
}

#[test]
fn conversions_to_unsigned_wrap_and_to_signed_are_the_implementations() {
    let source = "int main(void) {\n    unsigned int u = -1;\n    unsigned char b = 300;\n    \
                  short s = 40000;\n    int back = (int)4294967295u;\n    return (unsigned)-1 == \
                  u;\n}\n";
    returned(source, 1);
    let states = states(source);
    let last = states.last().expect("a final state");
    let values: Vec<&Value> = last.objects.iter().map(|object| &object.value).collect();
    assert_eq!(
        values,
        [
            &Value::Integer(4_294_967_295),
            &Value::Integer(44),
            &Value::Integer(-25_536),
            &Value::Integer(-1)
        ]
    );
    let stored = |title: &str| {
        states
            .iter()
            .find(|state| state.event.title == title)
            .unwrap_or_else(|| panic!("{title}"))
            .event
            .clone()
    };
    let unsigned = stored("Initialize `u` = 4294967295");
    assert!(
        unsigned.summary.starts_with(
            "-1 does not fit in an unsigned int, so storing it reduces it modulo 2^32: -1 + 2^32 \
             = 4294967295."
        ),
        "{}",
        unsigned.summary
    );
    assert!(
        unsigned
            .clauses
            .iter()
            .any(|clause| clause.id == "6.3.1.3p2")
    );
    assert_eq!(unsigned.from_type.as_deref(), Some("int"));
    assert_eq!(unsigned.result_type.as_deref(), Some("unsigned int"));
    let narrow = stored("Initialize `b` = 44");
    assert!(
        narrow.explanation.contains("300 - 256 = 44"),
        "{}",
        narrow.explanation
    );
    let short = stored("Initialize `s` = -25536");
    assert!(
        short
            .summary
            .contains("C leaves that result to the implementation"),
        "{}",
        short.summary
    );
    assert!(short.clauses.iter().any(|clause| clause.id == "6.3.1.3p3"));
    let casts: Vec<&State> = states
        .iter()
        .filter(|state| state.event.kind == EventKind::Cast)
        .collect();
    assert_eq!(casts[0].event.title, "Cast 4294967295 to int → -1");
    assert!(
        casts[0].event.summary.contains("bcc and GCC wrap it to -1"),
        "{}",
        casts[0].event.summary
    );
    assert_eq!(casts[1].event.title, "Cast -1 to unsigned int → 4294967295");
    assert!(
        casts[1].event.summary.contains("-1 + 2^32 = 4294967295"),
        "{}",
        casts[1].event.summary
    );
    // Static objects convert their constant initializers the same way.
    returned(
        "unsigned int max = -1;\nunsigned char low = 257;\nint main(void) { return low + (max == \
         4294967295u); }",
        2,
    );
}

#[test]
fn shifts_promote_each_operand_and_stop_outside_their_width() {
    returned("int main(void) { return (1u << 31) == 2147483648u; }", 1);
    returned(
        "int main(void) { unsigned char c = 1; return c << 8; }",
        256,
    );
    returned("int main(void) { return -8 >> 1; }", -4);
    returned(
        "int main(void) { long one = 1; return (one << 40) >> 38; }",
        4,
    );
    let source = "int main(void) { unsigned int top = 1u << 31; return (top << 1) == 0; }";
    returned(source, 1);
    let steps = states(source);
    let shifts: Vec<&State> = steps
        .iter()
        .filter(|state| state.event.kind == EventKind::Arithmetic)
        .collect();
    assert_eq!(shifts[0].event.title, "Compute 1 << 31 = 2147483648");
    assert_eq!(shifts[1].event.title, "Compute 2147483648 << 1 = 0");
    assert!(
        shifts[1].event.summary.contains("reduced modulo 2^32: 0"),
        "{}",
        shifts[1].event.summary
    );
    assert!(cited(shifts[1]).contains(&"6.5.7p4"));
    assert!(shifts[1].event.note.contains("unsigned types"));
    // A shift promotes each operand alone: no common type, no conversion.
    assert!(
        !kinds(&states(
            "int main(void) { unsigned int u = 1; return 1 << u; }"
        ))
        .contains(&"arithmetic_conversion")
    );
    let negative = states("int main(void) { return -8 >> 1; }");
    let right = negative
        .iter()
        .find(|state| state.event.kind == EventKind::Arithmetic)
        .expect("the shift");
    assert!(
        right
            .event
            .summary
            .contains("implementation-defined; bcc and GCC"),
        "{}",
        right.event.summary
    );
    for (source, expected, message) in [
        (
            "int main(void) { int n = 32; return 1 << n; }",
            super::Undefined::ShiftCount,
            "a shift count must be 0 to 31",
        ),
        (
            "int main(void) { int n = -1; return 1u >> n; }",
            super::Undefined::ShiftCount,
            "by -1",
        ),
        (
            "int main(void) { long n = 64; long one = 1; return one << n; }",
            super::Undefined::ShiftCount,
            "0 to 63",
        ),
        (
            "int main(void) { int m = -1; return m << 1; }",
            super::Undefined::ShiftNegative,
            "shifts a negative int left",
        ),
        (
            "int main(void) { return 1 << 31; }",
            super::Undefined::ShiftOverflow,
            "1 × 2^31 = 2147483648",
        ),
    ] {
        let session = run(source);
        let diagnostic = &session.state().diagnostics[0];
        assert_eq!(diagnostic.undefined, Some(expected), "{source}");
        assert!(
            diagnostic.message.contains(message),
            "{source}: {}",
            diagnostic.message
        );
        assert!(
            diagnostic
                .citation
                .is_some_and(|citation| citation.contains("6.5.7")),
            "{source}"
        );
    }
    let (_, title) = undefined("int main(void) { int n = 40; return 1 << n; }");
    assert_eq!(title, "Undefined behavior: shift count out of range");
    let found = compile("int main(void) { int *p = 0; return p << 1; }")
        .map(|_| ())
        .expect_err("a pointer is no shift operand");
    assert_eq!(found.kind, "constraint");
}

#[test]
fn sizeof_gives_a_size_t_without_evaluating_its_operand() {
    let source = "struct pair { char c; int i; };\nint main(void) {\n    int values[3] = {1, 2, \
                  3};\n    struct pair p = {1, 2};\n    unsigned long size = sizeof values;\n    \
                  return size + sizeof(long) + sizeof p + sizeof(char) + sizeof values[0];\n}\n";
    returned(source, 12 + 8 + 8 + 1 + 4);
    let steps = states(source);
    let sizes: Vec<(&str, &Option<Operand>)> = steps
        .iter()
        .filter(|state| state.event.kind == EventKind::Sizeof)
        .map(|state| (state.event.title.as_str(), &state.event.result))
        .collect();
    assert_eq!(sizes.len(), 5);
    assert_eq!(sizes[0].0, "`sizeof values` is 12");
    assert_eq!(sizes[0].1, &Some(Operand::Value(Value::Integer(12))));
    let first = steps
        .iter()
        .find(|state| state.event.kind == EventKind::Sizeof)
        .expect("a sizeof");
    assert!(
        first
            .event
            .explanation
            .contains("does not decay to a pointer"),
        "{}",
        first.event.explanation
    );
    assert!(first.event.explanation.contains("unsigned long on x86-64"));
    assert_eq!(
        cited(first),
        ["6.5.3.4p2", "6.5.3.4p4", "7.17p2", "6.3.2.1p3"]
    );
    assert_eq!(first.event.kind.category(), "evaluate");
    assert_eq!(first.event.result_type.as_deref(), Some("unsigned long"));
    // The operand is never evaluated: nothing is read, nothing can stop.
    assert!(
        !steps
            .iter()
            .any(|state| state.event.title.starts_with("Read values")),
        "sizeof reads nothing"
    );
    returned("int main(void) { int x; return sizeof x; }", 4);
    // size_t comes from <stdio.h>.
    returned(
        "#include <stdio.h>\nint main(void) { size_t n = sizeof(size_t); return n; }",
        8,
    );
    assert!(compile("int main(void) { size_t n = 1; return 0; }").is_err());
}

#[test]
fn integer_constants_take_their_type_from_their_value_and_suffix() {
    let source = "int main(void) {\n    long big = 2147483648;\n    unsigned long a = 10ul;\n    \
                  long long b = 5LL;\n    unsigned int c = 7u;\n    return big > 0;\n}\n";
    returned(source, 1);
    let steps = states(source);
    let constants: Vec<(&str, Option<&str>)> = steps
        .iter()
        .filter(|state| state.event.kind == EventKind::Constant)
        .map(|state| {
            (
                state.event.summary.as_str(),
                state.event.result_type.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        constants,
        [
            (
                "The constant 2147483648 is the long value 2147483648.",
                Some("long")
            ),
            (
                "The constant 10ul is the unsigned long value 10.",
                Some("unsigned long")
            ),
            (
                "The constant 5LL is the long long value 5.",
                Some("long long")
            ),
            (
                "The constant 7u is the unsigned int value 7.",
                Some("unsigned int")
            ),
            ("The constant 0 is simply the int value 0.", Some("int")),
        ]
    );
    let big = steps
        .iter()
        .find(|state| state.event.kind == EventKind::Constant)
        .expect("a constant");
    assert!(
        big.event.explanation.contains("too large for an int"),
        "{}",
        big.event.explanation
    );
    assert!(rejected("int main(void) { return 0x10; }").contains("hexadecimal"));
    assert!(rejected("int main(void) { return 1lul; }").contains("suffix"));
    assert!(rejected("int main(void) { return 99999999999999999999; }").contains("no type"));
    returned(
        "int main(void) { return 18446744073709551615u == -1ull; }",
        1,
    );
}

#[test]
fn printf_takes_length_modifiers_and_checks_each_argument_s_type() {
    let source = "#include <stdio.h>\nint main(void) {\n    unsigned long long m = \
                  18446744073709551615ull;\n    printf(\"%u %lu %llu %ld %lld %x %hhu %hd \
                  %zu\\n\", 4294967295u, 18446744073709551615ul, m + 1, -1l, \
                  -9223372036854775807ll - 1, 255u, 300, 40000, sizeof(long));\n    unsigned char \
                  b = 300;\n    short s = 40000;\n    printf(\"%d %d %hhd %hu %llx\\n\", b, s, \
                  200, 70000, m);\n    return 0;\n}\n";
    // GCC's and bcc's output for the same program.
    let (state, text) = output(source);
    assert_eq!(state.status, "finished", "{:?}", state.diagnostics);
    assert_eq!(
        text,
        "4294967295 18446744073709551615 0 -1 -9223372036854775808 ff 44 -25536 8\n44 -25536 -56 \
         4464 ffffffffffffffff\n"
    );
    let steps = states(source);
    let promotions: Vec<&str> = steps
        .iter()
        .filter(|state| state.event.title.ends_with("for `...`"))
        .map(|state| state.event.title.as_str())
        .collect();
    assert_eq!(
        promotions,
        [
            "Promote unsigned char 44 to int for `...`",
            "Promote short -25536 to int for `...`"
        ]
    );
    for (body, message) in [
        (
            "printf(\"%lu\", 1);",
            "`%lu` needs an unsigned long, but argument 1",
        ),
        (
            "printf(\"%d\", 4294967295u);",
            "with the value 4294967295, which an int cannot hold",
        ),
        ("printf(\"%u\", 1l);", "is a long"),
        ("printf(\"%hhs\", \"x\");", "length modifier hh"),
    ] {
        let source = format!("#include <stdio.h>\nint main(void) {{ {body} return 0; }}\n");
        let session = run(&source);
        assert_eq!(session.state().status, "undefined_behavior", "{body}");
        let found = &session.state().diagnostics[0].message;
        assert!(found.contains(message), "{body}: {found}");
    }
    // A value both types hold may stand for either.
    let (_, text) = output(
        "#include <stdio.h>\nint main(void) { printf(\"%u %d %lu\", 5, 6u, sizeof(int)); return \
         0; }\n",
    );
    assert_eq!(text, "5 6 4");
}

#[test]
fn a_condition_may_count_down_with_a_postfix_decrement() {
    let source =
        "int main(void) {\n    int values[3] = {4, 5, 6};\n    unsigned int sum = 0;\n    for \
         (unsigned int k = 3; k-- > 0;) {\n        sum += values[k];\n    }\n    return sum;\n}\n";
    returned(source, 15);
    let steps = states(source);
    let updates: Vec<&State> = steps
        .iter()
        .filter(|state| state.event.kind == EventKind::Increment)
        .collect();
    assert_eq!(
        updates.len(),
        4,
        "3 passes, then the test that ends the loop"
    );
    let first = &updates[0].event;
    assert_eq!(first.title, "Update `k`: 3 → 2");
    assert_eq!(
        first.summary,
        "`k--` yields `k`'s old value, 3, to the condition, and changes `k` from 3 to 2."
    );
    assert!(
        !first.sequence_point,
        "the condition's end is the sequence point"
    );
    assert_eq!(first.result, Some(Operand::Value(Value::Integer(3))));
    assert!(cited(updates[0]).contains(&"6.5p2"));
    let last = &updates[3].event;
    assert_eq!(last.title, "Update `k`: 0 → 4294967295");
    assert!(
        last.explanation
            .contains("reduced modulo 2^32 (4294967296)"),
        "{}",
        last.explanation
    );
    // The constant 0 converts to unsigned int for the comparison.
    assert!(
        steps
            .iter()
            .any(|state| state.event.title == "Convert 0 to unsigned int"),
        "{:?}",
        kinds(&steps)
    );
    returned(
        "int main(void) { int n = 0; int i = 3; while (--i) { n = n + 1; } return n; }",
        2,
    );
    returned(
        "int main(void) { int a[3] = {1, 2, 3}; int *p = a; int *end = a + 3; int n = 0; while \
         (p++ != end - 1) { n = n + 1; } return n; }",
        2,
    );
    // The last p++ moves p past one-past-the-end, which is undefined.
    let (cause, _) = undefined(
        "int main(void) { int a[3] = {1, 2, 3}; int *p = a; int *end = a + 3; while (p++ != end) \
         {} return 0; }",
    );
    assert_eq!(cause, Some(super::Undefined::PointerOutside));
    for source in [
        "int main(void) { int i = 0; i++ > 0; return 0; }",
        "int main(void) { int i = 0; while (i++ < i) {} return 0; }",
        "int main(void) { int i = 0; int j = 0; while (i++ < j++) {} return 0; }",
        "int main(void) { int a[2] = {0, 0}; int i = 0; while (a[i]++ < 1) {} return 0; }",
        "int main(void) { const int i = 1; while (i--) {} return 0; }",
    ] {
        assert!(compile(source).is_err(), "{source}");
    }
}

#[test]
fn increments_and_compound_assignments_wrap_in_unsigned_types() {
    returned(
        "int main(void) { unsigned int u = 0; u--; return u == 4294967295u; }",
        1,
    );
    let steps = states("int main(void) { unsigned char c = 255; c++; return c; }");
    let increment = steps
        .iter()
        .find(|state| state.event.kind == EventKind::Increment)
        .expect("c++");
    assert_eq!(increment.event.title, "Update `c`: 255 → 0");
    assert!(
        increment
            .event
            .explanation
            .contains("converting that back to unsigned char reduces it modulo 256"),
        "{}",
        increment.event.explanation
    );
    let source = "int main(void) { unsigned int u = 1; u += -2; unsigned int w = 1; w <<= 31; \
                  return (u == 4294967295u) + (w == 2147483648u); }";
    returned(source, 2);
    let steps = states(source);
    let compound: Vec<&State> = steps
        .iter()
        .filter(|state| state.event.kind == EventKind::CompoundAssign)
        .collect();
    assert_eq!(compound[0].event.title, "Update `u`: 1 → 4294967295");
    assert!(
        compound[0].event.explanation.contains(
            "the right operand, -2, is converted to unsigned int: -2 + 2^32 = 4294967294"
        ),
        "{}",
        compound[0].event.explanation
    );
    assert_eq!(compound[1].event.title, "Update `w`: 1 → 2147483648");
    let (cause, _) = undefined("int main(void) { int i = 1; i <<= 31; return 0; }");
    assert_eq!(cause, Some(super::Undefined::ShiftOverflow));
}

#[test]
fn parameters_returns_members_and_pointers_take_every_integer_type() {
    let source = "struct wide { char tag; unsigned long count; };\nunsigned int down(unsigned int \
                  x) {\n    return x - 1;\n}\nlong widen(int x) {\n    return x;\n}\nint \
                  main(void) {\n    unsigned int a[2] = {7, 8};\n    unsigned int *p = a;\n    \
                  p++;\n    struct wide w = {1, 4294967296};\n    unsigned long *q = &w.count;\n    \
                  *q = *q + 1;\n    return (down(0) == 4294967295u) + (down(-1) == 4294967294u) + \
                  *p + (widen(-1) == -1) + (w.count == 4294967297);\n}\n";
    assert!(
        compile(source).is_err(),
        "nested calls are outside the subset"
    );
    let source = "struct wide { char tag; unsigned long count; };\nunsigned int down(unsigned int \
                  x) {\n    return x - 1;\n}\nlong widen(int x) {\n    return x;\n}\nint \
                  main(void) {\n    unsigned int a[2] = {7, 8};\n    unsigned int *p = a;\n    \
                  p++;\n    struct wide w = {1, 4294967296};\n    unsigned long *q = &w.count;\n    \
                  *q = *q + 1;\n    unsigned int d = down(-1);\n    long l = widen(-1);\n    \
                  return (d == 4294967294u) + *p + (l == -1) + (w.count == 4294967297);\n}\n";
    returned(source, 1 + 8 + 1 + 1);
    let steps = states(source);
    let call = steps
        .iter()
        .find(|state| state.event.kind == EventKind::Call)
        .expect("down(-1)");
    assert!(
        call.event
            .explanation
            .contains("The argument for `x` converts to the parameter's type, unsigned int"),
        "{}",
        call.event.explanation
    );
    let widened = steps
        .iter()
        .find(|state| state.event.title == "Return -1 from widen")
        .expect("widen returns");
    assert!(
        widened
            .event
            .explanation
            .contains("converts to widen's return type, long"),
        "{}",
        widened.event.explanation
    );
    let last = steps.last().expect("a final state");
    let wide = last
        .objects
        .iter()
        .find(|object| object.name == "w")
        .expect("w");
    assert_eq!(wide.size, 16);
    let program = compile(source).expect("compiles");
    let layout = &program.structures()[0];
    assert_eq!(
        layout
            .members
            .iter()
            .map(|member| (member.r#type.as_str(), member.offset, member.size))
            .collect::<Vec<_>>(),
        [("char", 0, 1), ("unsigned long", 8, 8)]
    );
    // Incompatible pointer types are still outside the subset.
    assert!(compile("int main(void) { int x = 0; unsigned int *p = &x; return 0; }").is_err());
}

const UNSIGNED: &str = include_str!("../../web/demos/unsigned.c");

#[test]
fn the_unsigned_lesson_wraps_converts_and_counts_down() {
    let (state, text) = output(UNSIGNED);
    assert_eq!(state.status, "finished", "{:?}", state.diagnostics);
    // GCC's and bcc's output for the same program.
    assert_eq!(
        text,
        "2u - 3 is 4294967295\nthe countdown ended at 4294967295\n12 bytes, sum 15, -1 < 1u is 0, \
         2147483648, 0\n"
    );
    assert_eq!(state.return_value, Some(Value::Integer(0)));
    let steps = states(UNSIGNED);
    for kind in [
        "arithmetic_conversion",
        "sizeof",
        "increment",
        "compound_assign",
        "library_call",
    ] {
        assert!(kinds(&steps).contains(&kind), "{kind}");
    }
    assert!(
        steps
            .iter()
            .any(|state| state.event.title == "Update `i`: 0 → 4294967295"),
        "the countdown wraps"
    );
    assert!(
        steps
            .iter()
            .any(|state| state.event.title == "Convert -1 to unsigned int → 4294967295")
    );
}

#[test]
fn objects_and_events_carry_sizes_and_types_in_json() {
    let session = run("int main(void) { unsigned char c = 1; long l = c; return -1 < 1u; }");
    let json = session.trace_json();
    assert!(
        json.contains("\"type\":\"unsigned char\",\"size\":1"),
        "{json}"
    );
    assert!(json.contains("\"type\":\"long\",\"size\":8"));
    assert!(json.contains("\"from_type\":\"int\",\"result_type\":\"unsigned int\""));
    assert!(json.contains("\"kind\":\"arithmetic_conversion\""));
}

/// Floating values of every type, printed with each floating conversion;
/// `FLOATING_PRINTED` is what the same program prints built with GCC 13.3
/// and with bcc's native back end (`--gcc-mode`), byte for byte.
const FLOATING_PRINTF: &str = "#include <stdio.h>

int main(void) {
    double tenth = 0.1;
    double big = 1e21;
    double tiny = 4.9e-324;
    double negative = -2.5;
    float single = 0.1f;
    long double third = 1.0L / 3.0L;
    double zero = 0.0;
    printf(\"[%f] [%.3f] [%10.2f] [%-10.1f] [%010.3f] [%.0f] [%.0f]\\n\", tenth, tenth, negative, \
                               negative, negative, 0.5, 1.5);
    printf(\"[%e] [%.2e] [%E] [%12.4e] [%.0e]\\n\", tenth, big, negative, tiny, 12345.0);
    printf(\"[%g] [%g] [%g] [%.3g] [%G] [%.17g] [%g]\\n\", tenth, big, tiny, 1234.5, 1e-10, tenth, \
                               100000.0);
    printf(\"[%a] [%A] [%.2a] [%a] [%15a]\\n\", tenth, negative, tenth, tiny, 1.0);
    printf(\"[%f] [%.17g] [%a]\\n\", single, single, single);
    printf(\"[%Lf] [%.25Lf] [%Le] [%Lg] [%La]\\n\", third, third, third, third, third);
    printf(\"[%f] [%f] [%5.1f] [%-6f] [%e] [%g]\\n\", 1.0 / zero, -1.0 / zero, zero / zero, -(zero \
                               / zero), 1.0 / zero, -0.0);
    printf(\"[%lf] [%.1f] [%.2f] [%.2f]\\n\", tenth, 0.25, 0.125, 0.375);
    int printed = printf(\"%.20f\\n\", tenth);
    printf(\"%d\\n\", printed);
    return 0;
}
";

const FLOATING_PRINTED: &str = "[0.100000] [0.100] [     -2.50] [-2.5      ] [-00002.500] [0] [2]
[1.000000e-01] [1.00e+21] [-2.500000E+00] [ 4.9407e-324] [1e+04]
[0.1] [1e+21] [4.94066e-324] [1.23e+03] [1E-10] [0.10000000000000001] [100000]
[0x1.999999999999ap-4] [-0X1.4P+1] [0x1.9ap-4] [0x0.0000000000001p-1022] [         0x1p+0]
[0.100000] [0.10000000149011612] [0x1.99999ap-4]
[0.333333] [0.3333333333333333333423684] [3.333333e-01] [0.333333] [0xa.aaaaaaaaaaaaaabp-5]
[inf] [-inf] [ -nan] [nan   ] [inf] [-0]
[0.100000] [0.2] [0.12] [0.38]
0.10000000000000000555
23
";

#[test]
fn printf_writes_floating_values_as_glibc_does() {
    let (state, text) = output(FLOATING_PRINTF);
    assert_eq!(state.status, "finished", "{:?}", state.diagnostics);
    assert_eq!(text, FLOATING_PRINTED);
    let steps = states(FLOATING_PRINTF);
    // A float passed through `...` arrives as a double.
    assert!(steps.iter().any(|state| {
        state.event.title == "Promote float 0.1 to double for `...`"
            && state.event.result_type.as_deref() == Some("double")
    }));
    let call = steps
        .iter()
        .find(|state| state.event.title.starts_with("printf writes \"[0.333333]"))
        .expect("the long double line");
    assert!(cited(call).contains(&"7.19.6.1p7"), "the L modifier");
    assert!(cited(call).contains(&"7.19.6.1p8"));
}

/// Conversions on initialization, assignment, argument passing and return,
/// in structures and arrays and through pointers; `FLOATING_CONVERTED` is
/// the output of GCC 13.3 and of bcc's native back end.
const FLOATING_CONVERSIONS: &str = "#include <stdio.h>

struct sample {
    float f;
    double d;
    long double l;
};

double half(double x) {
    return x / 2;
}

int truncated(double x) {
    return x;
}

int main(void) {
    long long big = 9007199254740993;
    double d = big;
    float f = 16777217;
    double back = f;
    int i = 2.99;
    int j = -2.99;
    unsigned int u = 3e9;
    char c = 65.7;
    double mixed = 'A' + 0.5f;
    double values[3] = {1.5, 2.5};
    double *p = &values[1];
    *p += 1;
    struct sample s = {0.1f, 0.1, 0.1L};
    double dd = 0.1;
    dd++;
    i += 2.5;
    printf(\"%.17g %.9g %.17g %d %d %u %d %.17g\\n\", d, f, back, i, j, u, c, mixed);
    printf(\"%g %g %g %g\\n\", values[0], values[1], values[2], *p);
    printf(\"%.20g %.20g %.25Lg\\n\", s.f, s.d, s.l);
    double halved = half(3);
    int cut = truncated(-7.9);
    printf(\"%.17g %d %d\\n\", halved, cut, (int)(0.1 * 3 * 10));
    printf(\"%.17g\\n\", dd);
    float narrowed = 1e40;
    printf(\"%f\\n\", narrowed);
    static double limit = -1.5e300;
    printf(\"%g %d\\n\", limit, 1.0 / 3 > 0.33);
    return 0;
}
";

const FLOATING_CONVERTED: &str = "9007199254740992 16777216 16777216 4 -2 3000000000 65 65.5
1.5 3.5 0 3.5
0.10000000149011611938 0.10000000000000000555 0.1000000000000000000013553
1.5 -7 3
1.1000000000000001
inf
-1.5e+300 1
";

#[test]
fn floating_conversions_round_truncate_and_widen() {
    let (state, text) = output(FLOATING_CONVERSIONS);
    assert_eq!(state.status, "finished", "{:?}", state.diagnostics);
    assert_eq!(text, FLOATING_CONVERTED);
    let steps = states(FLOATING_CONVERSIONS);
    let titled = |title: &str| {
        steps
            .iter()
            .find(|state| state.event.title == title)
            .unwrap_or_else(|| panic!("no step titled {title}"))
    };
    // 2^53 + 1 has no exact double (C99 §6.3.1.4p2).
    let rounded = titled("Initialize `d` = 9007199254740992.0");
    assert!(
        rounded.event.summary.contains("no exact double"),
        "{}",
        rounded.event.summary
    );
    assert!(cited(rounded).contains(&"6.3.1.4p2"));
    // A store into an int truncates toward zero (§6.3.1.4p1).
    let truncated = titled("Initialize `j` = -2");
    assert!(truncated.event.summary.contains("truncating toward zero"));
    assert!(cited(truncated).contains(&"6.3.1.4p1"));
    assert_eq!(truncated.event.from_type.as_deref(), Some("double"));
    assert_eq!(truncated.event.result_type.as_deref(), Some("int"));
    // A character constant is an int, which converts to the float.
    assert_eq!(
        titled("Convert 65 to float → 65.0")
            .event
            .from_type
            .as_deref(),
        Some("int")
    );
    // With a floating operand a char converts without being promoted.
    let steps = states("int main(void) { char c = 'A'; float f = c + 0.5f; return 0; }");
    let mixed = steps
        .iter()
        .find(|state| state.event.kind == EventKind::ArithmeticConversion)
        .expect("the char converts");
    assert_eq!(mixed.event.title, "Convert 65 to float → 65.0");
    assert_eq!(mixed.event.from_type.as_deref(), Some("char"));
    assert!(
        mixed
            .event
            .explanation
            .contains("integer promotions do not happen")
    );
    assert!(
        !steps
            .iter()
            .any(|state| state.event.kind == EventKind::IntegerPromotion)
    );
    // Widening keeps the value; narrowing beyond the range is an infinity
    // under Annex F.
    assert!(cited(titled("Initialize `back` = 16777216.0")).contains(&"6.3.1.5p1"));
    let narrowed = titled("Initialize `narrowed` = inf");
    assert!(narrowed.event.explanation.contains("Annex F"));
    // Sizes on x86-64.
    let sizes: Vec<(&str, usize)> = state
        .objects
        .iter()
        .filter(|object| ["d", "f", "s", "values"].contains(&object.name.as_str()))
        .map(|object| (object.r#type.as_str(), object.size))
        .collect();
    assert_eq!(
        sizes,
        [
            ("double", 8),
            ("float", 4),
            ("double[3]", 24),
            ("struct sample", 32)
        ]
    );
}

#[test]
fn floating_constants_show_their_exact_stored_value() {
    let steps = states(
        "int main(void) { double a = 0.1; double b = 0.5; float c = 0x1.8p1f; long double d = \
         1e-3L; return 0; }",
    );
    let constant = |spelling: &str| {
        steps
            .iter()
            .find(|state| {
                state.event.kind == EventKind::Constant
                    && state
                        .event
                        .title
                        .starts_with(&format!("Evaluate the constant {spelling} "))
            })
            .unwrap_or_else(|| panic!("no constant {spelling}"))
    };
    let tenth = constant("0.1");
    assert_eq!(tenth.event.title, "Evaluate the constant 0.1 → 0.1");
    assert!(
        tenth
            .event
            .summary
            .contains("0.1000000000000000055511151231257827"),
        "{}",
        tenth.event.summary
    );
    assert!(
        tenth
            .event
            .explanation
            .contains("0.1000000000000000055511151231257827021181583404541015625")
    );
    assert!(cited(tenth).contains(&"6.4.4.2p3"));
    assert_eq!(tenth.event.result_type.as_deref(), Some("double"));
    assert!(!tenth.event.note.is_empty(), "values are not exact");
    let half = constant("0.5");
    assert!(
        half.event.summary.contains("exactly"),
        "{}",
        half.event.summary
    );
    assert_eq!(
        constant("0x1.8p1f").event.result_type.as_deref(),
        Some("float")
    );
    assert_eq!(
        constant("1e-3L").event.result_type.as_deref(),
        Some("long double")
    );
    // A constant beyond its type's range breaks a constraint (C99 §6.4.4p2).
    let error = compile("int main(void) { double x = 1e309; return 0; }")
        .map(|_| ())
        .expect_err("1e309");
    assert_eq!(error.kind, "constraint");
    assert!(
        error.message.contains("largest double"),
        "{}",
        error.message
    );
    let _program = compile("int main(void) { long double x = 1e309L; return 0; }")
        .expect("a long double holds 1e309");
}

#[test]
fn floating_arithmetic_rounds_and_comparisons_see_exact_values() {
    let source = "int main(void) { double sum = 0.1 + 0.2; int equal = sum == 0.3; return equal; }";
    returned(source, 0);
    let steps = states(source);
    let addition = steps
        .iter()
        .find(|state| state.event.kind == EventKind::Arithmetic)
        .expect("the addition");
    assert_eq!(
        addition.event.title,
        "Compute 0.1 + 0.2 = 0.30000000000000004"
    );
    assert!(
        addition
            .event
            .explanation
            .contains("0.3000000000000000166533453693773481063544750213623046875"),
        "the exact sum: {}",
        addition.event.explanation
    );
    assert!(cited(addition).contains(&"F.3p1"));
    let comparison = steps
        .iter()
        .find(|state| state.event.kind == EventKind::Comparison)
        .expect("the comparison");
    assert_eq!(
        comparison.event.title,
        "Compare 0.30000000000000004 == 0.3 → 0"
    );
    for exact in [
        "0.3000000000000000444089209850062616169452667236328125",
        "0.299999999999999988897769753748434595763683319091796875",
    ] {
        assert!(
            comparison.event.summary.contains(exact),
            "{}",
            comparison.event.summary
        );
    }
    // Float arithmetic happens in float on x86-64.
    let steps = states("int main(void) { float a = 0.1f; float b = a * 3; return 0; }");
    let product = steps
        .iter()
        .find(|state| state.event.kind == EventKind::Arithmetic)
        .expect("the product");
    assert_eq!(product.event.result_type.as_deref(), Some("float"));
    assert!(cited(product).contains(&"5.2.4.2.2p8"));
}

#[test]
fn division_by_zero_and_nan_follow_annex_f() {
    let source = "#include <stdio.h>
int main(void) {
    double zero = 0.0;
    double infinity = 1.0 / zero;
    double nan = zero / zero;
    printf(\"%f %f %d %d %d\\n\", infinity, nan, nan == nan, nan != nan, nan < 1.0);
    if (nan) {
        return 1;
    }
    return 0;
}
";
    let (state, text) = output(source);
    assert_eq!(text, "inf -nan 0 1 0\n");
    assert_eq!(state.return_value, Some(Value::Integer(1)), "a NaN is true");
    let steps = states(source);
    let division = steps
        .iter()
        .find(|state| state.event.title == "Compute 1.0 / 0.0 = inf")
        .expect("1.0 / zero");
    assert!(division.event.summary.contains("Annex F"));
    assert!(division.event.explanation.contains("__STDC_IEC_559__"));
    assert!(cited(division).contains(&"6.5.5p5"));
    assert!(cited(division).contains(&"6.10.8p2"));
    let unordered = steps
        .iter()
        .find(|state| state.event.title == "Compare -nan == -nan → 0")
        .expect("nan == nan");
    assert!(unordered.event.summary.contains("unordered"));
    // Integer division by zero is still undefined.
    let (cause, _) = undefined("int main(void) { int zero = 0; return 1 / zero; }");
    assert_eq!(cause, Some(super::Undefined::DivisionByZero));
}

#[test]
fn a_floating_value_that_does_not_fit_an_integer_is_undefined() {
    for (source, message) in [
        (
            "int main(void) { double big = 3e9; int x = big; return x; }",
            "3000000000",
        ),
        (
            "int main(void) { double big = 3e9; return (int)big; }",
            "3000000000",
        ),
        (
            "int main(void) { double minus = -1.5; unsigned int u = minus; return 0; }",
            "-1",
        ),
        (
            "int main(void) { double zero = 0.0; int x = zero / zero; return x; }",
            "integral part",
        ),
    ] {
        let session = run(source);
        let state = session.state();
        assert_eq!(state.status, "undefined_behavior", "{source}");
        let diagnostic = &state.diagnostics[0];
        assert_eq!(
            diagnostic.undefined,
            Some(super::Undefined::FloatingConversion)
        );
        assert!(
            diagnostic.message.contains(message),
            "{}",
            diagnostic.message
        );
        assert_eq!(diagnostic.clause.map(|clause| clause.id), Some("6.3.1.4p1"));
        assert_eq!(
            state.event.title,
            "Undefined behavior: a floating value that does not fit"
        );
    }
    // -0.5 truncates to 0, which an unsigned int holds.
    returned(
        "int main(void) { double minus = -0.5; unsigned int u = minus; return u; }",
        0,
    );
}

#[test]
fn printf_checks_floating_arguments_and_modifiers() {
    for (source, cause) in [
        (
            "#include <stdio.h>\nint main(void) { printf(\"%d\", 1.5); return 0; }",
            super::Undefined::WrongArgument,
        ),
        (
            "#include <stdio.h>\nint main(void) { printf(\"%f\", 1); return 0; }",
            super::Undefined::WrongArgument,
        ),
        (
            "#include <stdio.h>\nint main(void) { printf(\"%f\", 1.0L); return 0; }",
            super::Undefined::WrongArgument,
        ),
        (
            "#include <stdio.h>\nint main(void) { printf(\"%Lf\", 1.0); return 0; }",
            super::Undefined::WrongArgument,
        ),
        (
            "#include <stdio.h>\nint main(void) { printf(\"%hf\", 1.0); return 0; }",
            super::Undefined::InvalidConversion,
        ),
        (
            "#include <stdio.h>\nint main(void) { printf(\"%Ld\", 1); return 0; }",
            super::Undefined::InvalidConversion,
        ),
    ] {
        assert_eq!(undefined(source).0, Some(cause), "{source}");
    }
    let session = run("#include <stdio.h>\nint main(void) { printf(\"%+f\", 1.0); return 0; }");
    assert_eq!(session.state().diagnostics[0].kind, "unsupported");
}

#[test]
fn floating_objects_take_part_in_every_construct() {
    let source = "struct point { double x; float y; };
double scale(double value, float factor) { return value * factor; }
int main(void) {
    struct point p = {1.5, 2.0f};
    double values[2] = {0.25, 0.5};
    double *q = &values[0];
    q[1] += p.x;
    p.y -= 1;
    values[0]++;
    double total = scale(values[1], p.y);
    total = total + values[0];
    long double wide = total;
    float narrow = -total;
    if (narrow < 0) {
        if (wide > 3.2L) {
            return (int)(total * 10);
        }
    }
    return 0;
}
";
    returned(source, 32);
    let (state, _) = output(source);
    let object = |name: &str| {
        state
            .objects
            .iter()
            .find(|object| object.name == name)
            .unwrap_or_else(|| panic!("no {name}"))
    };
    assert_eq!(object("p").r#type, "struct point");
    assert_eq!(object("p").size, 16);
    assert_eq!(object("wide").size, 16);
    assert_eq!(object("wide").r#type, "long double");
    let Value::Floating(total) = object("total").value else {
        panic!("total is a double");
    };
    assert_eq!(total.short(), "3.25");
    assert_eq!(total.bytes(), 3.25_f64.to_le_bytes());
    let fields = total.fields();
    assert_eq!(
        (fields.biased, fields.scale, fields.class),
        (1024, 1, "normal")
    );
    assert_eq!(fields.significand, "1.625");
    for kind in ["compound_assign", "increment", "call", "return", "cast"] {
        assert!(kinds(&states(source)).contains(&kind), "{kind}");
    }
}

#[test]
fn floating_values_and_types_reach_the_json() {
    let session = run("int main(void) { double d = 0.1; float f = d; return 0; }");
    let json = session.trace_json();
    assert!(json.starts_with("{\"schema_version\":7,"), "{json}");
    assert!(json.contains(
        "{\"kind\":\"floating\",\"type\":\"double\",\"value\":\"0.1\",\"exact\":\"0.\
         1000000000000000055511151231257827021181583404541015625\",\"bytes\":[154,153,153,153,153,\
         153,185,63]}"
    ));
    assert!(json.contains("\"type\":\"float\",\"size\":4"));
    assert!(json.contains("\"from_type\":\"double\",\"result_type\":\"float\""));
}

#[test]
fn static_floating_objects_start_with_their_constant_value() {
    let source = "#include <stdio.h>
#define RATE 0.25
double rate = RATE;
static float scaled = -2;
int main(void) {
    static long double zero;
    printf(\"%g %g %Lg\\n\", rate, scaled, zero);
    return 0;
}
";
    let (state, text) = output(source);
    assert_eq!(state.status, "finished", "{:?}", state.diagnostics);
    assert_eq!(text, "0.25 -2 0\n");
    assert!(rejected("int x = 1e10; int main(void) { return x; }").contains("does not fit"));
}

const FLOATING: &str = include_str!("../../web/demos/floating.c");

#[test]
fn the_floating_lesson_is_close_but_not_exact() {
    let (state, text) = output(FLOATING);
    assert_eq!(state.status, "finished", "{:?}", state.diagnostics);
    // GCC's and bcc's output for the same program.
    assert_eq!(
        text,
        "total / count is 3.000000\n(double)total / count is 3.500000\n0.1 + 0.2 == 0.3 is 0\n0.1 \
         + 0.2 is 0.30000000000000004\nten 0.1s make 0.99999999999999989\n(int)2.99 is 2\n"
    );
    let steps = states(FLOATING);
    for title in [
        "Initialize `average` = 3.0",
        "Cast 7 to double → 7.0",
        "Convert 2 to double → 2.0",
        "Compute 7.0 / 2.0 = 3.5",
        "Compute 0.1 + 0.2 = 0.30000000000000004",
        "Compare 0.30000000000000004 == 0.3 → 0",
        "Update `tenth`: 0.8999999999999999 → 0.9999999999999999",
        "Cast 2.99 to int → 2",
    ] {
        assert!(
            steps.iter().any(|state| state.event.title == title),
            "{title}: {:?}",
            steps
                .iter()
                .map(|state| &state.event.title)
                .collect::<Vec<_>>()
        );
    }
    for state in &steps {
        assert!(!state.event.title.is_empty() && !state.event.summary.is_empty());
    }
}
