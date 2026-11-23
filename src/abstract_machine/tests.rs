//! Source-driven evaluator regressions; no fixed-source execution fixtures.

use super::{
    DEMO_SOURCE,
    EventKind,
    Operand,
    Session,
    State,
    Upcoming,
    Value,
    compile,
};

mod enumerations;
mod jumps;

fn run(source: &str) -> Session {
    let program = compile(source).expect("test source must compile");
    let mut session = program.session();
    for _ in 0..=super::model::MAX_STEPS + 1 {
        if session.state().terminal() {
            assert_eq!(
                session.state().upcoming,
                None,
                "an ended run has no next step"
            );
            return session;
        }
        let upcoming = session.state().upcoming.clone();
        let state = session.step();
        foretold(upcoming.as_ref(), state);
    }
    panic!("session did not terminate at its resource bound");
}

/// Checks that the description of the next operation, `upcoming`, named
/// the step that ran: the event it said, or a stop there.
fn foretold(upcoming: Option<&Upcoming>, state: &State) {
    let upcoming = upcoming.expect("a state that can step describes its next operation");
    assert!(
        state.event.kind == upcoming.kind || state.event.kind == EventKind::Fault,
        "step {} was described as {:?} ({}) but recorded {:?} ({})",
        state.step,
        upcoming.kind,
        upcoming.text,
        state.event.kind,
        state.event.title,
    );
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
fn dereference_increment_precedence_is_preserved() {
    // `*p++` is `*(p++)`: it reads the element p pointed to and moves p.
    for operation in ["*p++", "*(p)++"] {
        let mut source = String::from("int main(void) { int a[2] = {3, 7}; int *p = a; int x = ");
        source.push_str(operation);
        source.push_str("; return x * 10 + *p; }");
        returned(&source, 37);
    }
    for operation in ["*p--", "*(p)--"] {
        let mut source =
            String::from("int main(void) { int a[2] = {3, 7}; int *p = a + 1; int x = ");
        source.push_str(operation);
        source.push_str("; return x * 10 + *p; }");
        returned(&source, 73);
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

/// The state after the first step of `kind` in `source`'s run.
fn first(source: &str, kind: EventKind) -> State {
    states(source)
        .into_iter()
        .find(|state| state.event.kind == kind)
        .expect("the run has a step of this kind")
}

// C99 §6.2.4p5: an automatic object lives from entry into its block, before
// its declaration is reached; C99 §6.2.1p7: its name's scope begins only
// after its declarator.
#[test]
fn entering_a_block_begins_the_lifetimes_of_its_objects() {
    let entered = first(
        "int main(void) { int a = 1; { int b; b = 2; } int c = a; return c; }",
        EventKind::EnterBlock,
    );
    let names: Vec<&str> = entered
        .event
        .objects
        .iter()
        .map(|&id| entered.objects[id - 1].name.as_str())
        .collect();
    assert_eq!(names, ["a", "c"], "only the block's own objects, not b");
    assert_eq!(entered.event.title, "Begin the lifetime of `a` and `c`");
    assert!(
        entered
            .objects
            .iter()
            .all(|object| object.value == Value::Indeterminate
                && object.lifetime == "alive"
                && !object.declared)
    );
    assert!(
        entered
            .event
            .clauses
            .iter()
            .any(|clause| clause.id == "6.2.4p5")
    );
    let states = states("int main(void) { int a = 1; int c; return a; }");
    let declared: Vec<&State> = states
        .iter()
        .filter(|state| state.event.kind == EventKind::Declare)
        .collect();
    assert_eq!(declared.len(), 2);
    assert_eq!(declared[0].event.title, "Declare `a` (int)");
    assert!(declared[0].objects[0].declared && !declared[0].objects[1].declared);
    assert!(
        declared[0].event.summary.contains("initializer runs next"),
        "{}",
        declared[0].event.summary
    );
    assert!(
        declared[1].event.summary.contains("no initializer"),
        "{}",
        declared[1].event.summary
    );
    assert_eq!(
        declared[0].objects.len(),
        2,
        "reaching a declaration creates nothing"
    );
    assert_eq!(states.last().map(|state| state.status), Some("finished"));
    let many = first(
        "int main(void) { int a; int b; int c; int d; int e; int f; return 0; }",
        EventKind::EnterBlock,
    );
    assert!(
        many.event
            .summary
            .contains("its 6 objects, `a`, `b` and the rest")
            && many.event.title.contains("`e` and `f`"),
        "{} / {}",
        many.event.title,
        many.event.summary
    );
    let empty = first("int main(void) { return 0; }", EventKind::EnterBlock);
    assert_eq!(empty.event.title, "Enter a block");
    assert!(empty.event.objects.is_empty());
}

#[test]
fn a_name_designates_an_outer_object_until_the_inner_declaration() {
    // The inner `x` exists from the inner brace, but `y = x` still reads
    // the outer one.
    let session =
        run("int main(void) { int x = 1; { int y = x; int x = 2; y = y + x; return y; } }");
    assert_eq!(session.state().return_value, Some(Value::Integer(3)));
    // `int x = x;` reads the object being declared, still indeterminate.
    let session = run("int main(void) { int x = x; return x; }");
    assert_eq!(session.state().status, "error");
    assert_eq!(session.state().diagnostics[0].kind, "indeterminate");
}

// C99 §6.8.5p5: a loop body is a block, entered anew on each iteration.
#[test]
fn each_iteration_begins_a_new_lifetime() {
    let states = states(
        "int main(void) { int s = 0; for (int i = 0; i < 3; i++) { int t = i; s = s + t; } return \
         s; }",
    );
    let begun: Vec<usize> = states
        .iter()
        .filter(|state| state.event.kind == EventKind::EnterBlock)
        .flat_map(|state| state.event.objects.clone())
        .collect();
    let named = |name: &str| {
        let last = states.last().expect("a final state");
        begun
            .iter()
            .filter(|&&id| last.objects[id - 1].name == name)
            .count()
    };
    assert_eq!((named("s"), named("i"), named("t")), (1, 1, 3));
    let body = states
        .iter()
        .find(|state| state.event.title == "Begin the lifetime of `t`")
        .expect("the body's entry");
    assert!(
        body.event
            .summary
            .starts_with("Each pass through the loop body creates a new `t`")
            && body.event.clauses[1].id == "6.8.5p5",
        "{}",
        body.event.summary
    );
    let header = states
        .iter()
        .find(|state| state.event.title == "Begin the lifetime of `i`")
        .expect("the for statement's entry");
    assert!(header.event.summary.starts_with("Entering the block"));
    assert_eq!(
        states.last().map(|state| state.return_value.clone()),
        Some(Some(Value::Integer(3)))
    );
}

#[test]
fn a_return_ends_objects_whose_declarations_were_never_reached() {
    let states = states(
        "int f(int c) { int a = 1; if (c) return a; int b = 2; return a + b; } int main(void) { \
         return f(1); }",
    );
    let returned = states
        .iter()
        .find(|state| state.event.kind == EventKind::Return)
        .expect("f returns");
    let ended: Vec<(&str, bool)> = returned
        .event
        .objects
        .iter()
        .map(|&id| {
            let object = &returned.objects[id - 1];
            (object.name.as_str(), object.declared)
        })
        .collect();
    assert_eq!(ended, [("c", true), ("a", true), ("b", false)]);
    assert!(
        returned.event.explanation.contains(
            "`b` existed from its block's opening brace, though its declaration was never reached."
        ),
        "{}",
        returned.event.explanation
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
        "int main(void) { static int x = 1 + 1 / 0; return x; }",
        "int main(void) { return 1.5 % 2; }",
        "int main(void) { if (0) { _Bool x; } return 0; }",
        "int main(void) { int i = 0; return i++ + i; }",
        "int main(void) { return missing(); }",
        "int f(int x) { return x; } int main(void) { return f(); }",
        "int main(void) { int *p; int x = 0; p = x; return 0; }",
        "int main(void) { int a[2]; a = 0; return 0; }",
        "int main(void) { int x; int x; return 0; }",
        "int f(int x) { int x; return 0; } int main(void) { return f(1); }",
        "int main(void) { return 08; }",
        "int main(void) { return 0x; }",
        "struct s { int x; }; int main(void) { struct s a = {1}; return a && 1; }",
        "int main(void) { double d = 1.0; return d & 1; }",
        "int main(void) { double d = 1.0; return ~d; }",
        "int main(void) { int *p = 0; return 1 ? p : 2; }",
        "int main(void) { double _Complex x; return 0; }",
        "int main(void) { long float x; return 0; }",
        "int main(void) { unsigned double x; return 0; }",
        "int main(void) { double x = 1e999; return 0; }",
        "int main(void) { double x = 0x1.8; return 0; }",
        "int main(void) { double d = 1.0; d <<= 1; return 0; }",
        "int main(void) { double d = 1.0; return (int)d % 2.0; }",
        "int main(void) { return 99999999999999999999; }",
        "int main(void) { return 1lul; }",
        "int main(void) { int i = 0; while (i++ < i) {} return 0; }",
        "int main(void) { int i = 0; while (i++ < i++) {} return 0; }",
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
    assert!(json.contains("\"schema_version\":11"), "versioned schema");
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
fn a_return_ends_only_the_automatic_objects_of_its_frame() {
    // C99 §6.2.4p3: a static local lives for the whole run, so the return
    // names only the parameters and automatic locals that end (§6.2.4p5).
    let source = "int count(int step) { static int total; int next = total + step; total = next; \
                  return next; } int main(void) { count(1); return count(2); }";
    let returns: Vec<_> = states(source)
        .into_iter()
        .filter(|state| state.event.kind == EventKind::Return)
        .map(|state| state.event)
        .collect();
    let first = &returns[0];
    assert_eq!(
        first.summary,
        "The return hands 1 back as the value of `count(1)`, and the frame of count ends: `step` \
         and `next` cease to exist."
    );
    assert!(!first.summary.contains("total"), "{}", first.summary);
    assert!(
        !first.explanation.contains("`total`"),
        "{}",
        first.explanation
    );
    assert!(
        first
            .note
            .contains("automatic locals (those not declared static)")
            && first.note.contains("a static local lasts the whole run"),
        "{}",
        first.note
    );
    // One ended object takes a singular verb; more than three are summed up.
    let one = states("int id(int x) { return x; } int main(void) { return id(4); }")
        .into_iter()
        .find(|state| state.event.kind == EventKind::Return)
        .expect("id returns")
        .event;
    assert!(
        one.summary
            .ends_with("the frame of id ends: `x` ceases to exist."),
        "{}",
        one.summary
    );
    let many = states(
        "int sum(int a, int b, int c, int d) { return a + b + c + d; } int main(void) { return \
         sum(1, 2, 3, 4); }",
    )
    .into_iter()
    .find(|state| state.event.kind == EventKind::Return)
    .expect("sum returns")
    .event;
    assert!(
        many.summary
            .ends_with("the frame of sum ends: its parameters cease to exist."),
        "{}",
        many.summary
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
    assert_eq!(
        event.summary,
        "The return hands 6 back as the value of `twice(3)`, and the frame of twice ends: `n` and \
         `r` cease to exist."
    );
    // C99 §6.2.4p2: a pointer to an ended object becomes indeterminate.
    assert!(event.note.contains("dangling pointer"), "{}", event.note);
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
    // C99 §6.5.3.2p4: `*p` is an lvalue designating the object, not its
    // value; footnote 87 names a null or dangling pointer as invalid.
    assert_eq!(
        indirection[0].summary,
        "`*p` follows the pointer `p` to the object it points to: `*p` now stands for a[1] itself."
    );
    assert!(
        indirection[0].explanation.contains("not a copy of it"),
        "{}",
        indirection[0].explanation
    );
    assert!(
        indirection[0].note.contains("null or dangling pointer"),
        "{}",
        indirection[0].note
    );
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
    // The object began with its block (C99 §6.2.4p5); the declaration
    // creates nothing, it names the object its initializer fills.
    assert!(
        initializer
            .summary
            .ends_with("the object this declaration names.")
            && initializer
                .explanation
                .contains("its object already exists"),
        "{}",
        initializer.explanation
    );
    // C99 §6.7.8p14 and §6.7.8p13: a char array takes a string literal's
    // characters, a structure the value of one of its type.
    let cited = |source: &str| {
        self::states(source)
            .into_iter()
            .find(|state| state.event.title.ends_with("receives its initial value"))
            .map(|state| state.event.clauses[0].id)
            .expect("a declaration with an initializer")
    };
    assert_eq!(
        cited("int main(void) { char s[] = \"ab\"; return s[0]; }"),
        "6.7.8p14"
    );
    assert_eq!(
        cited(
            "struct p { int x; }; int main(void) { struct p a = {1}; struct p b = a; return b.x; }"
        ),
        "6.7.8p13"
    );
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
    // The lead quotes the program's own subscript, its index parenthesized
    // as C99 §6.5.2.1p2's (E2) is.
    assert!(
        event
            .summary
            .starts_with("`q[i + 1]` means `*(q + (i + 1))`"),
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
        self::states(source)
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
        declared.summary.contains("existed since its block began"),
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
fn a_subscript_is_quoted_with_the_parentheses_its_meaning_needs() {
    // C99 §6.5.2.1p2: E1[E2] is (*((E1)+(E2))). An index that is not a
    // primary expression keeps its parentheses, and the base, a postfix
    // expression, keeps the ones it was written with.
    let leads = |source: &str| {
        states(source)
            .into_iter()
            .filter(|state| state.event.kind == EventKind::Subscript)
            .map(|state| (state.event.summary, state.event.explanation))
            .collect::<Vec<_>>()
    };
    let program = "struct s { int a[3]; }; int main(void) { int v[4] = {1, 2, 3, 4}; int i = 1; \
                   int n = 2; int *p = v; struct s t = {{5, 6, 7}}; return v[i << 1] + v[n == 1] \
                   + v[n - 1] + (p + 1)[2] + t.a[i] + v[(i)] + v[3] + v['\\0']; }";
    let quoted: Vec<_> = leads(program)
        .into_iter()
        .map(|(summary, explanation)| {
            let meaning = summary
                .split('`')
                .nth(3)
                .map(String::from)
                .expect("the lead quotes the meaning");
            assert!(
                explanation.contains(&format!("as `{meaning}`")),
                "{explanation}"
            );
            meaning
        })
        .collect();
    assert_eq!(
        quoted,
        [
            "*(v + (i << 1))",
            "*(v + (n == 1))",
            "*(v + (n - 1))",
            "*((p + 1) + 2)",
            "*(t.a + i)",
            "*(v + (i))",
            "*(v + 3)",
            "*(v + '\\0')",
        ]
    );
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
    // C99 §6.5.2.1p2: E1[E2] is (*((E1)+(E2))), quoted from the program.
    assert_eq!(
        subscript.summary,
        "`a[1]` means `*(a + 1)`: it moves 1 element forward and designates a[1]."
    );
    assert!(
        subscript
            .explanation
            .contains("Here `a` gives a pointer to a[0] and the index is 1"),
        "{}",
        subscript.explanation
    );
    assert!(
        subscript.note.contains("Nothing checks the index"),
        "{}",
        subscript.note
    );
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
    let _diagnostic = compile("int main(void) { char c = 1; int *p = &c; return 0; }")
        .expect_err("a char's address is no int pointer");
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
    assert!(json.contains("\"schema_version\":11"));
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
    assert_eq!(declared.event.title, "Declare `e` (struct entry)");
    assert!(
        declared
            .event
            .explanation
            .contains("`tag` (char) at byte 0 followed by 3 bytes of padding"),
        "{}",
        declared.event.explanation
    );
    assert!(
        declared
            .event
            .explanation
            .contains("of padding at the end. Each member's value"),
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
fn a_static_initializer_that_reads_an_object_says_why_it_is_refused() {
    let Err(diagnostic) =
        compile("int m = 1;\nint n = m + 2;\nint main(void) {\n    return n;\n}\n")
    else {
        panic!("a read of an object in a static initializer must be refused");
    };
    assert!(
        diagnostic.message.contains("a read of the object `m`"),
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
             c\",\"type\":\"char\",\"offset\":0,\"size\":1,\"padding\":3,\"first\":0,\"\
             enumeration\":null}"
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
                .starts_with("Before main: the string literal `\"abc\"`")
        })
        .expect("the literal's array is created");
    assert!(
        literal
            .event
            .summary
            .contains("`\"abc\"` is an array of 4 chars"),
        "{}",
        literal.event.summary
    );
    assert!(
        literal.event.explanation.contains("{'a', 'b', 'c', '\\0'}"),
        "{}",
        literal.event.explanation
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
fn a_multibyte_character_after_a_backslash_is_rejected_whole() {
    for (source, quoted) in [
        (
            "#include <stdio.h>\nint main(void) { puts(\"\\é\"); return 0; }",
            "\"\\é\"",
        ),
        ("int main(void) { char c = '\\é'; return c; }", "'\\é'"),
        ("int main(void) { char c = '\\€'; return c; }", "'\\€'"),
    ] {
        let diagnostic = compile(source).map(|_| ()).expect_err(source);
        assert_eq!(diagnostic.kind, "unsupported", "{source}");
        assert!(
            diagnostic.message.contains("non-ASCII"),
            "{source}: {}",
            diagnostic.message
        );
        // The range covers the whole character after the backslash.
        let after = source.find(quoted).expect("the literal") + 2;
        let character = &source[after..diagnostic.source_range.end];
        assert_eq!(diagnostic.source_range.start, after, "{source}");
        assert_eq!(character.chars().count(), 1, "{source}: {character:?}");
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
            "Before the program ran, the preprocessor replaced the macro `LIMIT` with `4`: the \
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
    assert!(
        joined.event.summary.starts_with(
            "Before the program ran, the preprocessor replaced the macro `LABEL` with \
             `\"total\"`, and translation phase 6 joined"
        ),
        "{}",
        joined.event.summary
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
        .find(|state| state.event.title == "Declare `step` (const int)")
        .expect("step is declared");
    assert!(declared.event.explanation.contains("defined const"));
    assert!(
        declared.event.summary.contains("a const int object"),
        "{}",
        declared.event.summary
    );
    // printf's result is named as what printf returned.
    let discarded = states
        .iter()
        .find(|state| state.event.title == "Discard the result")
        .expect("printf's result is discarded");
    assert!(
        discarded
            .event
            .summary
            .starts_with("The value `printf` returned, 13,"),
        "{}",
        discarded.event.summary
    );
    // The cast wraps, and says the result is implementation-defined.
    let cast = states
        .iter()
        .find(|state| state.event.kind == EventKind::Cast)
        .expect("the cast");
    assert_eq!(cast.event.title, "Cast 300 to char → 44");
    assert_eq!(
        cast.event.summary,
        "300 does not fit in a char, so the result is implementation-defined; bcc, GCC and Clang \
         wrap it to 44."
    );
    assert!(cited(cast).contains(&"6.3.1.3p3"));
    // An unsigned plain char gives the same byte, 44, but defined.
    assert!(
        cast.event
            .explanation
            .contains("as on ARM Linux, 300 becomes 44 too, and there the result is defined"),
        "{}",
        cast.event.explanation
    );
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
            "#define N 08\nint main(void) { return 0; }",
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
            "other pointer types",
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
    // As an operand, a compound assignment yields the value it stored.
    returned(
        "int main(void) { int x = 1; int y = x += 1; return y * 10 + x; }",
        22,
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
        .find(|state| state.event.title == "Declare `e` (unsigned int)")
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
    assert_eq!(event.other_type.as_deref(), Some("unsigned int"));
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
    let steps = states(source);
    let both: Vec<&super::Event> = steps
        .iter()
        .filter(|state| state.event.kind == EventKind::ArithmeticConversion)
        .map(|state| &state.event)
        .collect();
    let titles: Vec<&str> = both.iter().map(|event| event.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Convert -1 to unsigned long long → 18446744073709551615",
            "Convert 1 to unsigned long long"
        ]
    );
    // Each step names the operand it converted and the one it met.
    let types: Vec<(Option<&str>, Option<&str>)> = both
        .iter()
        .map(|event| (event.from_type.as_deref(), event.other_type.as_deref()))
        .collect();
    assert_eq!(
        types,
        [
            (Some("long long"), Some("unsigned long")),
            (Some("unsigned long"), Some("long long"))
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
        casts[0]
            .event
            .summary
            .contains("bcc, GCC and Clang wrap it to -1"),
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
fn static_initializers_negate_an_unsigned_constant_in_its_own_type() {
    // -1u is 4294967295 before the initializer converts it to long, in a
    // file-scope object, a static local and aggregate leaves as at block
    // scope; a negated signed constant keeps its value.
    let source =
        "#include <stdio.h>\nlong x = -1u;\nstruct pair { long a; unsigned long b; };\nstruct \
         pair p = {-1u, -1ul};\nlong table[3] = {-1u, -1, -2147483648};\nlong get(void) {\n    \
         static long s = -1u;\n    return s;\n}\nint main(void) {\n    long y = -1u;\n    long z \
         = get();\n    printf(\"%ld %ld %ld\\n\", x, y, z);\n    printf(\"%ld %lu\\n\", p.a, \
         p.b);\n    printf(\"%ld %ld %ld\\n\", table[0], table[1], table[2]);\n    return 0;\n}\n";
    let (state, text) = output(source);
    assert_eq!(state.status, "finished", "{:?}", state.diagnostics);
    assert_eq!(
        text,
        "4294967295 4294967295 4294967295\n4294967295 18446744073709551615\n4294967295 -1 \
         -2147483648\n"
    );
}

#[test]
fn repeating_a_type_specifier_past_its_counter_still_names_no_type() {
    // A narrow counter would wrap back to a valid combination after 256
    // repetitions, or overflow summing `signed` and `unsigned`.
    let mut sources = vec![format!(
        "int main(void){{{}x=1;return x;}}\n",
        "int ".repeat(257)
    )];
    for words in ["int ", "long ", "char ", "unsigned ", "float ", "double "] {
        sources.push(format!(
            "int main(void) {{ {} x = 1; return x; }}\n",
            words.repeat(256)
        ));
    }
    sources.push(format!(
        "int main(void) {{ {}{}int x = 1; return x; }}\n",
        "signed ".repeat(128),
        "unsigned ".repeat(128)
    ));
    for source in &sources {
        let diagnostic = compile(source).map(|_| ()).expect_err(source);
        assert_eq!(diagnostic.kind, "constraint", "{}", diagnostic.message);
        assert_eq!(diagnostic.clause.map(|clause| clause.id), Some("6.7.2p2"));
    }
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
            .contains("implementation-defined; bcc, GCC and Clang"),
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
fn long_leads_leave_the_details_to_the_explanation() {
    let source = "#include <stdio.h>\nint main(void) {\n    printf(\"%d and %d, %d and %d \
                  together\\n\", 1, 2, 3, 4);\n    printf(\"%d\\n\", 5);\n    return 0;\n}\n";
    let steps = states(source);
    let printed: Vec<&State> = steps
        .iter()
        .filter(|state| state.event.title.starts_with("printf writes"))
        .collect();
    assert_eq!(printed.len(), 2);
    // Four conversions: the lead names each one's argument, and the
    // explanation says what each wrote.
    assert_eq!(
        printed[0].event.summary,
        "printf writes its format to standard output with its 4 conversions filled in: %d took \
         `1`, %d took `2`, %d took `3`, and %d took `4`."
    );
    assert!(
        printed[0]
            .event
            .explanation
            .contains("takes argument 4 after the format, `4`, the int 4, and writes \"4\""),
        "{}",
        printed[0].event.explanation
    );
    // One conversion: the lead says what it wrote.
    assert!(
        printed[1]
            .event
            .summary
            .contains("%d took `5` and wrote \"5\""),
        "{}",
        printed[1].event.summary
    );
    // A long literal's decay spells it once; a short one names `"…"[0]`.
    let decays: Vec<&str> = steps
        .iter()
        .filter(|state| state.event.kind == EventKind::ArrayDecay)
        .map(|state| state.event.summary.as_str())
        .collect();
    assert_eq!(decays.len(), 2);
    assert!(
        decays[0].ends_with("becomes a pointer to its first element."),
        "{}",
        decays[0]
    );
    assert!(decays[1].ends_with("`\"%d\\n\"[0]`."), "{}", decays[1]);
}

#[test]
fn an_object_declared_size_t_keeps_the_typedef_name() {
    let source = "#include <stdio.h>\nconst size_t limit = 4;\nint main(void) {\n    int \
                  values[3] = {1, 2, 3};\n    size_t size = sizeof values;\n    size_t *at = \
                  &size;\n    static size_t calls;\n    unsigned long plain = 2;\n    \
                  printf(\"%zu %zu\\n\", size, plain);\n    return *at + limit + calls;\n}\n";
    returned(source, 16);
    let steps = states(source);
    let last = steps.last().expect("a step");
    let ty = |name: &str| {
        last.objects
            .iter()
            .find(|object| object.name == name)
            .map(|object| object.r#type.as_str())
    };
    // C99 §6.7.7p3: the typedef name is a synonym for unsigned long, and
    // the machine spells each object's type as its declaration did.
    assert_eq!(ty("size"), Some("size_t"));
    assert_eq!(ty("at"), Some("size_t *"));
    assert_eq!(ty("limit"), Some("const size_t"));
    assert_eq!(ty("calls"), Some("size_t"));
    assert_eq!(ty("plain"), Some("unsigned long"));
    let declared = steps
        .iter()
        .find(|state| state.event.title == "Declare `size` (size_t)")
        .expect("size is declared");
    assert!(
        declared.event.summary.contains("a size_t object"),
        "{}",
        declared.event.summary
    );
    // The disclosure names the type it stands for.
    assert!(
        declared
            .event
            .explanation
            .contains("<stdio.h> defines here as unsigned long (8 bytes"),
        "{}",
        declared.event.explanation
    );
    assert!(cited(declared).contains(&"7.17p2"));
    // %zu names the size_t it takes; an unsigned long given to it is one.
    let printed = steps
        .iter()
        .find(|state| state.event.title.starts_with("printf writes"))
        .expect("printf runs");
    assert!(
        printed.event.explanation.contains("`size`, the size_t 12"),
        "{}",
        printed.event.explanation
    );
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
    assert!(rejected("int main(void) { return 0x; }").contains("hexadecimal digits"));
    assert!(rejected("int main(void) { return 09; }").contains("not an octal digit"));
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
    // Two variables may each be updated: they are different objects, so the
    // order of the two stores does not matter (C99 §6.5p2-3).
    returned(
        "int main(void) { int i = 0; int j = 2; int n = 0; while (i++ < j--) n = n + 1; return n \
         * 100 + i * 10 + j + 1; }",
        121,
    );
    // An update whose value a statement discards, and one through a
    // subscript, run.
    returned("int main(void) { int i = 0; i++ > 0; return i; }", 1);
    returned(
        "int main(void) { int a[2] = {0, 0}; int i = 0; while (a[i]++ < 1) {} return a[0]; }",
        2,
    );
    for source in [
        "int main(void) { int i = 0; while (i++ < i) {} return 0; }",
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
    // Calls are operands like any other.
    returned(source, 1 + 1 + 8 + 1 + 1);
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
    assert!(json.contains(
        "\"from_type\":\"int\",\"result_type\":\"unsigned int\",\"other_type\":\"unsigned int\""
    ));
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
    assert!(json.starts_with("{\"schema_version\":11,"), "{json}");
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
    // -1u is 4294967295 before the initializer converts it to double, in a
    // static object as in an automatic one.
    let (state, text) = output(
        "#include <stdio.h>\ndouble x = -1u;\nint main(void) {\n    static double s = -1u;\n    \
         double y = -1u;\n    printf(\"%.0f %.0f %.0f\\n\", x, s, y);\n    return 0;\n}\n",
    );
    assert_eq!(state.status, "finished", "{:?}", state.diagnostics);
    assert_eq!(text, "4294967295 4294967295 4294967295\n");
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
    // int division says what it drops, and exact values are set as code.
    let lead = |title: &str| {
        steps
            .iter()
            .find(|state| state.event.title == title)
            .map(|state| state.event.summary.clone())
            .expect(title)
    };
    assert_eq!(
        lead("Compute 7 / 2 = 3"),
        "7 / 2 = 3: dividing one int by another gives an int, so the remainder, 1, is dropped and \
         the quotient truncates toward zero."
    );
    assert!(
        lead("Compute 0.1 + 0.2 = 0.30000000000000004")
            .starts_with("No double holds the exact sum `0.3000000000000000166533453693773481"),
        "{}",
        lead("Compute 0.1 + 0.2 = 0.30000000000000004")
    );
}

/// The site's first lesson, "Objects, loops and calls".
const OBJECTS: &str = include_str!("../../web/demos/objects.c");

/// The site's lessons, which the page steps through note by note.
const LESSONS: [&str; 12] = [
    OBJECTS,
    HELLO,
    CONSTANTS,
    include_str!("../../web/demos/recursion.c"),
    include_str!("../../web/demos/promotion.c"),
    UNSIGNED,
    include_str!("../../web/demos/overflow.c"),
    FLOATING,
    include_str!("../../web/demos/pointers.c"),
    include_str!("../../web/demos/structs.c"),
    include_str!("../../web/demos/bounds.c"),
    include_str!("../../web/demos/indeterminate.c"),
];

#[test]
fn every_lesson_step_is_described_before_it_runs() {
    let mut steps = 0;
    for lesson in LESSONS {
        let session = run(lesson);
        steps += session.state().step;
    }
    assert_eq!(
        steps, 1242,
        "the lessons' steps, which the site's notes follow"
    );
}

#[test]
fn the_next_operation_is_described_in_plain_words() {
    let program = compile(OBJECTS).expect("demo compiles");
    let mut session = program.session();
    let mut said = Vec::new();
    while !session.state().terminal() {
        if let Some(upcoming) = &session.state().upcoming {
            said.push((upcoming.text.clone(), upcoming.clause.id));
        }
        let _state = session.step();
    }
    for (text, clause) in [
        (
            "begin the lifetime of `total`, `values` and `cursor`",
            "6.2.4p5",
        ),
        ("reach the declaration of `total`", "6.2.1p7"),
        ("find the object `total`", "6.5.1p2"),
        ("evaluate the constant `0`", "6.4.4.1p5"),
        ("initialize `total`", "6.7.8p11"),
        ("select `values[1]` to initialize", "6.7.8p17"),
        (
            "turn the array `values` into a pointer to its first element",
            "6.3.2.1p3",
        ),
        ("read `total`", "6.3.2.1p2"),
        ("compare `i < 3`", "6.5.8p6"),
        ("test the `for` condition `i < 3`", "6.8.5.3p1"),
        ("call `bump`", "6.5.2.2p4"),
        ("find the object `value` points to", "6.5.3.2p4"),
        ("add `before` and `1`", "6.5.6p5"),
        ("store into `values[0]` (in main)", "6.5.16.1p2"),
        ("return from `bump`", "6.8.6.4p3"),
        ("move a pointer: `cursor + 1`", "6.5.6p8"),
        ("end the lifetime of `next`", "6.2.4p5"),
        ("jump back to the loop condition", "6.8.5p4"),
        ("return from `main`, which ends the program", "5.1.2.2.3p1"),
    ] {
        assert!(
            said.iter().any(|(said, id)| said == text && *id == clause),
            "missing {text:?} ({clause}) in {said:#?}"
        );
    }
}

/// Only the initial call of `main` ends the program, and only `main`
/// returns 0 from its closing brace (C99 §5.1.2.2.3p1); another
/// function's closing brace returns no value its caller may use
/// (§6.9.1p12), where the machine stops.
#[test]
fn returns_and_closing_braces_say_what_they_do() {
    let said = |source: &str| {
        let program = compile(source).expect("test source compiles");
        let mut session = program.session();
        let mut said = Vec::new();
        while !session.state().terminal() {
            if let Some(upcoming) = &session.state().upcoming {
                said.push((upcoming.text.clone(), upcoming.clause.id));
            }
            let _state = session.step();
        }
        said
    };
    let recursive =
        said("int main(void) { static int n = 0; n++; if (n < 2) { int x = main(); } return 0; }");
    let returns: Vec<_> = recursive
        .iter()
        .filter(|(text, _)| text.starts_with("return from `main`"))
        .collect();
    assert_eq!(
        returns,
        [
            &(String::from("return from `main`"), "6.8.6.4p3"),
            &(
                String::from("return from `main`, which ends the program"),
                "5.1.2.2.3p1"
            ),
        ],
        "{recursive:#?}"
    );
    let falling = said("int main(void) { static int n = 0; n++; if (n < 2) { int x = main(); } }");
    let ends: Vec<_> = falling
        .iter()
        .filter(|(text, _)| text.starts_with("reach the end of"))
        .collect();
    assert_eq!(
        ends,
        [
            &(
                String::from("reach the end of `main`, which returns 0 to its caller"),
                "5.1.2.2.3p1"
            ),
            &(
                String::from("reach the end of `main`, which returns 0 and ends the program"),
                "5.1.2.2.3p1"
            ),
        ],
        "{falling:#?}"
    );
    let other = said("int f(void) {} int main(void) { return f(); }");
    assert!(
        other.contains(&(
            String::from("reach the end of `f` without a return statement"),
            "6.9.1p12"
        )),
        "{other:#?}"
    );
    assert!(
        !other.iter().any(|(text, _)| text.contains("returns 0")),
        "{other:#?}"
    );
}

#[test]
fn conversions_subscripts_and_library_calls_are_described() {
    let said = |source: &str| {
        let program = compile(source).expect("lesson compiles");
        let mut session = program.session();
        let mut said = Vec::new();
        while !session.state().terminal() {
            said.extend(
                session
                    .state()
                    .upcoming
                    .as_ref()
                    .map(|upcoming| upcoming.text.clone()),
            );
            let _state = session.step();
        }
        said
    };
    let hello = said(HELLO);
    for text in [
        "include the header `<stdio.h>`",
        "create the string literal `\"Hello, world!\"`, before `main` runs",
        "call `puts`, which writes to standard output",
        "store the string's characters into `name`",
        "select `text[count]`, checking it is inside its array",
        "promote `text[count]` from `char` to `int`",
        "add 1 to `count`",
    ] {
        assert!(
            hello.iter().any(|said| said == text),
            "missing {text:?} in {hello:#?}"
        );
    }
    let unsigned = said(UNSIGNED);
    for text in [
        "convert `3` from `int` to `unsigned int`",
        "subtract 1 from `i`",
        "add `values[k]` to `sum`",
        "shift the bits of `1u` left by `31`",
    ] {
        assert!(
            unsigned.iter().any(|said| said == text),
            "missing {text:?} in {unsigned:#?}"
        );
    }
    let floating = said(FLOATING);
    assert!(
        floating
            .iter()
            .any(|said| said == "cast `price` from `double` to `int`")
    );
}

#[test]
fn a_stop_ends_the_description_and_the_trace_carries_it() {
    let session = run(include_str!("../../web/demos/bounds.c"));
    assert_eq!(session.state().status, "undefined_behavior");
    assert_eq!(session.state().upcoming, None);
    let program = compile(OBJECTS).expect("lesson compiles");
    let json = program.session().state_json();
    assert!(
        json.contains(
            "\"upcoming\":{\"kind\":\"enter_block\",\"text\":\"begin the lifetime of `total`, \
             `values` and `cursor`\""
        ),
        "{json}"
    );
}

/// Every state of a run, each step checked against the description of it
/// the state before gave.
fn stepped(source: &str) -> Vec<State> {
    let program = compile(source).expect("test source must compile");
    let mut session = program.session();
    let mut states = vec![session.state().clone()];
    while !session.state().terminal() {
        let upcoming = session.state().upcoming.clone();
        let state = session.step().clone();
        foretold(upcoming.as_ref(), &state);
        states.push(state);
    }
    states
}

/// The first step of `kind` whose title starts with `title`.
fn step<'a>(states: &'a [State], kind: EventKind, title: &str) -> &'a State {
    states
        .iter()
        .find(|state| state.event.kind == kind && state.event.title.starts_with(title))
        .unwrap_or_else(|| {
            panic!(
                "no {kind:?} step titled {title:?} in {:?}",
                states
                    .iter()
                    .map(|state| &state.event.title)
                    .collect::<Vec<_>>()
            )
        })
}

// C99 §6.5.10-§6.5.12: `&`, `^` and `|` combine the bits of their operands
// after the usual arithmetic conversions; §6.5.3.3p4: `~` flips the bits of
// its promoted operand.
#[test]
fn bitwise_operators_show_the_bits_they_combine() {
    let states =
        stepped(
            "int main(void) {\n    int flags = 13;\n    int low = flags & 3;\n    int set = 8 | \
             1;\n    int toggled = 12 ^ 10;\n    unsigned char byte = 15;\n    int inverted = \
             ~byte;\n    unsigned int all = ~0u;\n    return low + set + toggled + (inverted & \
             255) + (all == 4294967295u);\n}\n",
        );
    let and = step(&states, EventKind::Arithmetic, "Compute 13 & 3 = 1");
    assert_eq!(
        and.event.summary,
        "`&` keeps only the bits that are 1 in both operands: 0000 1101 & 0000 0011 = 0000 0001, \
         which is 1."
    );
    assert!(
        and.event
            .explanation
            .contains("the 24 bits above those shown are 0 in all three"),
        "{}",
        and.event.explanation
    );
    assert_eq!(and.event.clauses[0].id, "6.5.10p4");
    assert!(
        and.event
            .note
            .starts_with("The bitwise operators treat an integer")
    );
    let or = step(&states, EventKind::Arithmetic, "Compute 8 | 1 = 9");
    assert_eq!(or.event.clauses[0].id, "6.5.12p4");
    let xor = step(&states, EventKind::Arithmetic, "Compute 12 ^ 10 = 6");
    assert!(
        xor.event
            .summary
            .contains("0000 1100 ^ 0000 1010 = 0000 0110")
    );
    assert_eq!(xor.event.clauses[0].id, "6.5.11p4");
    // The unsigned char is promoted to int before `~` flips 32 bits.
    let promoted = states
        .iter()
        .position(|state| state.event.kind == EventKind::IntegerPromotion)
        .expect("the char is promoted");
    let complement = step(&states, EventKind::Unary, "Complement 15 → -16");
    assert!(states[promoted].step < complement.step);
    assert_eq!(
        complement.event.summary,
        "`~` flips every bit of 15: 0000 … 0000 1111 becomes 1111 … 1111 0000, which is -16."
    );
    assert!(complement.event.explanation.contains("~x is -x - 1 there"));
    assert_eq!(complement.event.clauses[0].id, "6.5.3.3p4");
    let unsigned = step(&states, EventKind::Unary, "Complement 0 → 4294967295");
    assert!(
        unsigned
            .event
            .explanation
            .contains("the largest value minus x: 4294967295 - 0 = 4294967295"),
        "{}",
        unsigned.event.explanation
    );
    returned(
        "int main(void) { int a = -16; return (a & 255) + (-1 ^ 1) + (5 | -8); }",
        240 - 2 - 3,
    );
    for source in [
        "int main(void) { double d = 1.0; return d & 1; }",
        "int main(void) { int *p = 0; return 1 | p; }",
        "int main(void) { double d = 1.0; return ~d; }",
        "int main(void) { double d = 1.0; d |= 1; return 0; }",
    ] {
        let diagnostic = compile(source).expect_err(source);
        assert_eq!(diagnostic.kind, "constraint", "{source}");
    }
}

// C99 §6.5.16.2p3: `E1 op= E2` is `E1 = E1 op (E2)` with E1 evaluated once.
#[test]
fn bitwise_compound_assignments_set_clear_and_flip_bits() {
    let source = "int main(void) {\n    unsigned int flags = 0;\n    flags |= 4;\n    flags |= \
                  1;\n    flags &= 6;\n    flags ^= 3;\n    return flags;\n}\n";
    returned(source, 7);
    let states = stepped(source);
    let set = step(&states, EventKind::CompoundAssign, "Update `flags`: 4 → 5");
    assert!(
        set.event
            .explanation
            .contains("In binary, 4 is 0000 0100 and 1 is 0000 0001, so the result is 0000 0101"),
        "{}",
        set.event.explanation
    );
    assert!(
        set.event
            .clauses
            .iter()
            .any(|clause| clause.id == "6.5.12p4")
    );
    let cleared = step(&states, EventKind::CompoundAssign, "Update `flags`: 5 → 4");
    assert!(
        cleared
            .event
            .explanation
            .contains("keeps only the bits it shares with 6")
    );
    assert!(
        compile("int main(void) { double d = 1.0; d ^= 2; return 0; }").is_err(),
        "bitwise operands are integers"
    );
}

// C99 §6.5.13p3-4 and §6.5.14p3-4: `&&` and `||` evaluate their left operand
// first, with a sequence point after it, and the right one only when the
// left leaves the result open.
#[test]
fn logical_operators_skip_their_right_operand_when_the_left_decides() {
    let source = "int main(void) {\n    int *p = 0;\n    int positive = 0;\n    if (p != 0 && *p \
                  > 0) {\n        positive = 1;\n    }\n    return positive;\n}\n";
    returned(source, 0);
    let states = stepped(source);
    assert!(
        states
            .iter()
            .all(|state| state.event.kind != EventKind::Dereference),
        "`*p` is never evaluated"
    );
    let test = step(&states, EventKind::Logical, "Left of && is false");
    assert_eq!(
        test.event.title,
        "Left of && is false: skip the right, result 0"
    );
    assert_eq!(
        test.event.summary,
        "`p != 0` is 0, so `p != 0 && *p > 0` is 0 whatever its right operand would give, and `*p \
         > 0` is never evaluated."
    );
    assert!(test.event.sequence_point, "a sequence point follows");
    assert_eq!(test.event.result, Some(Operand::Value(Value::Integer(0))));
    let cited: Vec<&str> = test.event.clauses.iter().map(|clause| clause.id).collect();
    assert_eq!(cited, ["6.5.13p4", "6.5.13p3", "Cp1"]);
    // The skipped operand's nodes are recorded as never evaluated, and
    // none of them has a value.
    let evaluation = test
        .evaluation
        .as_ref()
        .expect("the condition's evaluation");
    let program = compile(source).expect("compiles");
    let nodes = program.expression(evaluation.function, evaluation.root);
    let skipped: Vec<&str> = nodes
        .iter()
        .enumerate()
        .filter(|(index, _)| evaluation.skipped.contains(&(nodes[0].first + index)))
        .map(|(_, node)| node.label.as_str())
        .collect();
    assert_eq!(skipped, ["p", "*", "0", ">"]);
    assert!(
        evaluation
            .values
            .iter()
            .all(|(node, _)| !evaluation.skipped.contains(node))
    );

    let source = "int main(void) {\n    int x = 5;\n    int y = 0;\n    int inside = x > 0 && x < \
                  10;\n    int either = y == 1 || x == 5;\n    int none = y || 0;\n    return \
                  inside + either + none;\n}\n";
    returned(source, 2);
    let states = stepped(source);
    let open = step(&states, EventKind::Logical, "Left of && is true");
    assert_eq!(open.event.title, "Left of && is true: evaluate the right");
    assert_eq!(open.event.result, None);
    assert!(open.event.sequence_point);
    let decided = step(&states, EventKind::Logical, "`x < 10` is 1");
    assert_eq!(decided.event.title, "`x < 10` is 1: && yields 1");
    assert!(!decided.event.sequence_point);
    assert_eq!(decided.event.result_type.as_deref(), Some("int"));
    let either = step(&states, EventKind::Logical, "`x == 5` is 1");
    assert!(
        either
            .event
            .summary
            .contains("its left operand was 0, but its right one is nonzero"),
        "{}",
        either.event.summary
    );
    let none = step(&states, EventKind::Logical, "`0` is 0");
    assert!(none.event.summary.contains("neither operand is nonzero"));
    // `||` with a nonzero left operand yields 1 at once; a guard keeps a
    // division by zero from running.
    returned(
        "int main(void) { int d = 0; int safe = d == 0 || 10 / d > 1; return safe; }",
        1,
    );
    returned(
        "int main(void) { int v[5] = {4, 8, 15, 16, 23}; int i = 0; while (i < 5 && v[i] != 15) { \
         i++; } return i; }",
        2,
    );
    returned(
        "int main(void) { double half = 0.5; int *p = 0; return (half && 1) + !(p || 0); }",
        2,
    );
}

// C99 §6.5.15p4-5: `?:` evaluates its first operand, then only the operand
// it chooses, converted to the type of the result.
#[test]
fn the_conditional_operator_evaluates_only_the_chosen_operand() {
    let source = "int main(void) {\n    int a = 4;\n    int b = 9;\n    int max = a > b ? a : \
                  b;\n    return max;\n}\n";
    returned(source, 9);
    let states = stepped(source);
    let test = step(&states, EventKind::Conditional, "Condition of ?: is false");
    assert_eq!(test.event.title, "Condition of ?: is false: choose `b`");
    assert!(test.event.sequence_point);
    let cited: Vec<&str> = test.event.clauses.iter().map(|clause| clause.id).collect();
    assert_eq!(cited, ["6.5.15p4", "Cp1"]);
    let yielded = step(&states, EventKind::Conditional, "`?:` yields 9");
    assert_eq!(
        yielded.event.summary,
        "The chosen operand `b` is 9, so `a > b ? a : b` is 9."
    );
    assert_eq!(
        states
            .iter()
            .filter(|state| state.event.kind == EventKind::LvalueConversion)
            .filter(|state| state.event.title.starts_with("Read `a`"))
            .count(),
        1,
        "`a` is read once, for the condition only"
    );
    // A division by zero in the operand not chosen never happens.
    returned(
        "int main(void) { int n = 0; int total = 10; return n != 0 ? total / n : -1; }",
        -1,
    );
    // The result has the operands' common type: an int converts to double.
    let states = stepped("int main(void) { int c = 1; double d = c ? 1 : 2.5; return d == 1.0; }");
    let converted = step(&states, EventKind::Conditional, "`?:` yields 1.0");
    assert!(
        converted
            .event
            .explanation
            .contains("the second and third operands together, double, so 1 is converted to 1.0"),
        "{}",
        converted.event.explanation
    );
    assert!(
        converted
            .event
            .clauses
            .iter()
            .any(|clause| clause.id == "6.5.15p5")
    );
    returned(
        "#include <stdio.h>\nint main(void) { int count = 2; printf(\"%d item%s\\n\", count, \
         count == 1 ? \"\" : \"s\"); return count > 1 ? count < 3 ? 7 : 8 : 9; }",
        7,
    );
    returned(
        "int main(void) { int x = 3; int *p = x > 2 ? &x : 0; return p != 0; }",
        1,
    );
    for source in [
        "int main(void) { int *p = 0; return 1 ? p : 2; }",
        "struct s { int x; }; int main(void) { struct s a = {1}; return a ? 1 : 2; }",
    ] {
        assert_eq!(
            compile(source).expect_err(source).kind,
            "constraint",
            "{source}"
        );
    }
}

// C99 §6.5.17p2: the comma operator's left operand is evaluated as a void
// expression, a sequence point follows, and the right gives the result.
#[test]
fn the_comma_operator_sequences_two_expressions() {
    let source =
        "int main(void) {\n    int values[4] = {1, 2, 3, 4};\n    int i;\n    int j;\n    for (i \
         = 0, j = 3; i < j; i++, j--) {\n        int saved = values[i];\n        values[i] = \
         values[j];\n        values[j] = saved;\n    }\n    return values[0] * 1000 + values[1] * \
         100 + values[2] * 10 + values[3];\n}\n";
    returned(source, 4321);
    let states = stepped(source);
    let comma = step(&states, EventKind::Comma, "Comma: `i = 0` is done");
    assert_eq!(
        comma.event.title,
        "Comma: `i = 0` is done, go on to `j = 3`"
    );
    assert!(comma.event.sequence_point);
    let cited: Vec<&str> = comma.event.clauses.iter().map(|clause| clause.id).collect();
    assert_eq!(cited, ["6.5.17p2", "Cp1"]);
    // Only the last operand ends the full expression.
    let first = step(&states, EventKind::Assign, "Store 0 into `i`");
    assert!(!first.event.sequence_point);
    assert!(!first.event.explanation.contains("full expression ends"));
    let second = step(&states, EventKind::Assign, "Store 3 into `j`");
    assert!(second.event.sequence_point);
    let increment = step(&states, EventKind::Increment, "Update `i`: 0 → 1");
    assert!(!increment.event.sequence_point);
    assert!(
        increment
            .event
            .explanation
            .contains("the left operand of a comma operator"),
        "{}",
        increment.event.explanation
    );
    let decrement = step(&states, EventKind::Increment, "Update `j`: 3 → 2");
    assert!(decrement.event.sequence_point);
    // In parentheses the left operand's value is discarded, and the right
    // operand's is the result.
    let states = stepped("int main(void) { int x = 2; int y = (x + 1, x * 10); return y; }");
    let discarded = step(&states, EventKind::Comma, "Comma: `x + 1` is done");
    assert!(
        discarded
            .event
            .summary
            .contains("throws away `x + 1`'s value, 3")
    );
    let yielded = step(&states, EventKind::Comma, "The comma operator yields 20");
    assert_eq!(
        yielded.event.result,
        Some(Operand::Value(Value::Integer(20)))
    );
    returned(
        "int main(void) { int a = 1; int b = 2; a += 2, b *= 3; return a * 10 + b; }",
        36,
    );
}

// C99 Annex C, §6.5.2.2p10, §6.5.13p4, §6.5.14p4, §6.5.15p4, §6.5.17p2,
// §6.8p4: an update inside a condition completes its store by the sequence
// point that next follows it, and the next step that is a sequence point is
// that one.
#[test]
fn an_update_in_a_condition_completes_by_the_next_sequence_point() {
    let cases: [(&str, &str, EventKind, Option<&str>); 8] = [
        (
            "int main(void) { int i = 2; int n = 0; while (i-- > 0 && n < 5) n = n + 1; return n; \
             }",
            "the end of `i-- > 0`, the first operand of `&&`",
            EventKind::Logical,
            Some("6.5.13p4"),
        ),
        (
            "int main(void) { int i = 2; int s = 1; while (i-- > 0 || s < 0) {} return i; }",
            "the end of `i-- > 0`, the first operand of `||`",
            EventKind::Logical,
            Some("6.5.14p4"),
        ),
        (
            "int main(void) { int i = 2; while (i-- ? 1 : 0) {} return i; }",
            "the end of `i--`, the first operand of `?:`",
            EventKind::Conditional,
            Some("6.5.15p4"),
        ),
        (
            "int main(void) { int i = 2; while ((i--, i >= 0)) {} return i; }",
            "the end of `i--`, the left operand of a comma operator",
            EventKind::Comma,
            Some("6.5.17p2"),
        ),
        (
            "int f(int x) { return x; } int main(void) { int i = 2; while (f(i--)) {} return i; }",
            "the one just before the call `f(i--)`, once its arguments are evaluated",
            EventKind::Call,
            Some("6.5.2.2p10"),
        ),
        (
            "int main(void) { int i = 2; int x = 1; if ((x && i++ == 2) && x) {} return i; }",
            "the end of `(x && i++ == 2)`, the first operand of `&&`",
            EventKind::Logical,
            Some("6.5.13p4"),
        ),
        // In the second operand of `&&`, or alone, the next sequence point
        // is the end of the condition.
        (
            "int main(void) { int i = 2; int x = 1; if (x && i++ == 2) {} return i; }",
            "the end of the condition",
            EventKind::Branch,
            None,
        ),
        (
            "int main(void) { int i = 2; while (i-- > 0) {} return i; }",
            "the end of the condition",
            EventKind::Branch,
            None,
        ),
    ];
    for (source, point, kind, clause) in cases {
        let states = stepped(source);
        let at = states
            .iter()
            .position(|state| state.event.kind == EventKind::Increment)
            .expect(source);
        let update = &states[at].event;
        assert!(
            update
                .explanation
                .contains(&format!("by the next sequence point, {point};")),
            "{source}: {}",
            update.explanation
        );
        assert!(!update.sequence_point, "{source}");
        let next = states[at + 1..]
            .iter()
            .find(|state| state.event.sequence_point)
            .expect(source);
        assert_eq!(next.event.kind, kind, "{source}: {}", next.event.title);
        let cited: Vec<&str> = update.clauses.iter().map(|clause| clause.id).collect();
        if let Some(clause) = clause {
            assert!(cited.contains(&clause), "{source}: {cited:?}");
        }
        assert!(cited.contains(&"6.5p2"), "{source}: {cited:?}");
    }
}

// C99 §6.5.2.2p10, §6.8p3-4: the comma operator is a sequence point, the
// commas between arguments give no order and no sequence point, and the
// commas between declarators leave each initializer a full expression,
// evaluated in order.
#[test]
fn the_comma_operator_s_note_tells_the_three_commas_apart() {
    let states = stepped("int main(void) { int x = 2; int y = (x + 1, x * 10); return y; }");
    let comma = step(&states, EventKind::Comma, "Comma: `x + 1` is done");
    let note = &comma.event.note;
    assert!(
        note.contains(
            "The commas between a call's arguments only separate them: C fixes no order among the \
             arguments and puts no sequence point between them."
        ),
        "{note}"
    );
    assert!(
        note.contains("each initializer is a full expression, evaluated in declarator order"),
        "{note}"
    );
    assert!(!note.contains("in a declaration only separate"), "{note}");
    // The left operand is a plain read: it has no side effects to keep.
    assert!(
        comma
            .event
            .explanation
            .contains("only its side effects, if it has any, matter"),
        "{}",
        comma.event.explanation
    );
}

// C99 §6.5p2, Annex C: a condition may update a variable and use it again,
// or call a function, where a sequence point separates the two; without
// one, the machine refuses, and says why without misstating C.
#[test]
fn a_condition_may_reuse_an_updated_variable_across_a_sequence_point() {
    // GCC 14 gives each of these the same result.
    returned(
        "int main(void) { int i = 0; int r = 0; if (i++ && i++) r = 1; return i * 10 + r; }",
        10,
    );
    returned(
        "int main(void) { int i = 0; int n = 0; if (i++ == 0 && i == 1) n = 1; return n; }",
        1,
    );
    returned(
        "int main(void) { int i = 0; int n = 0; while (i++ < 3 && i != 2) n = n + 1; return i * \
         10 + n; }",
        21,
    );
    returned(
        "int f(int x) { return x; } int main(void) { int i = 3; int n = 0; while (i-- > 0 && \
         f(1)) n = n + 1; return n; }",
        3,
    );
    returned(
        "int main(void) { int i = 1; int j = 5; int n = 0; while (i < 3 ? i++ : j--) n = n + 1; \
         return n; }",
        7,
    );
    // The read after `&&` sees the store, which the update says is
    // separated from it.
    let states = stepped(
        "int main(void) { int i = 0; int n = 0; if (i++ == 0 && i == 1) n = 1; return n; }",
    );
    let update = step(&states, EventKind::Increment, "Update `i`: 0 → 1");
    assert!(
        update
            .event
            .explanation
            .contains("separated from this update by a sequence point"),
        "{}",
        update.event.explanation
    );
    let read = step(&states, EventKind::LvalueConversion, "Read `i`");
    assert_eq!(read.event.title, "Read `i` → 1");
    // Unsequenced with the update: undefined in C, refused.
    for source in [
        "int main(void) { int i = 0; if (i++ + i) {} return 0; }",
        "int main(void) { int i = 0; if (i++ == i++) {} return 0; }",
        "int f(int x) { return x; } int main(void) { int i = 3; while (f(i--) > i) {} return 0; }",
    ] {
        let diagnostic = compile(source).expect_err(source);
        assert!(
            diagnostic
                .message
                .contains("with no sequence point between the two"),
            "{source}: {}",
            diagnostic.message
        );
    }
    // What `*`, `[]` and calls reach is decided by the objects the run
    // reaches: a call that never touches `i`, and a subscript of another
    // array, run; `*p` reaching `i` itself stops as undefined.
    returned(
        "int f(int x) { return x; } int main(void) { int i = 3; int n = 0; while (i-- > f(1)) \
         n++; return n * 10 + i; }",
        20,
    );
    returned(
        "int main(void) { int i = 0; int v[3] = {0, 0, 1}; while (v[i++] < 1) {} return i; }",
        3,
    );
    let (cause, _) =
        undefined("int main(void) { int i = 0; int *p = &i; if (i++ == *p) {} return 0; }");
    assert_eq!(cause, Some(super::Undefined::Unsequenced));
}

// C99 §6.4.4.1p3-5: an octal or hexadecimal constant is computed in its
// base, and its type list includes the unsigned types.
#[test]
fn octal_and_hexadecimal_constants_take_their_own_type_list() {
    let states = stepped(
        "int main(void) {\n    int mask = 0xFF;\n    int mode = 0755;\n    unsigned int all = \
         0xFFFFFFFF;\n    long big = 0x7FFFFFFFFFFFFFFF;\n    return mask + mode + (all == \
         4294967295u) + (big > 0) + 0x10u + 0;\n}\n",
    );
    let hexadecimal = step(
        &states,
        EventKind::Constant,
        "Evaluate the constant 0xFF = 255",
    );
    assert_eq!(
        hexadecimal.event.summary,
        "The hexadecimal constant 0xFF is the int value 255."
    );
    assert!(
        hexadecimal
            .event
            .explanation
            .contains("so 15 × 16 + 15 = 255. It has type int, the first of int, unsigned int"),
        "{}",
        hexadecimal.event.explanation
    );
    let cited: Vec<&str> = hexadecimal
        .event
        .clauses
        .iter()
        .map(|clause| clause.id)
        .collect();
    assert_eq!(cited, ["6.4.4.1p4", "6.4.4.1p5"]);
    let octal = step(
        &states,
        EventKind::Constant,
        "Evaluate the constant 0755 = 493",
    );
    assert!(octal.event.explanation.contains("7 × 64 + 5 × 8 + 5 = 493"));
    let all = step(
        &states,
        EventKind::Constant,
        "Evaluate the constant 0xFFFFFFFF",
    );
    assert_eq!(all.event.result_type.as_deref(), Some("unsigned int"));
    assert!(
        all.event
            .explanation
            .contains("A decimal constant without a u would never be unsigned")
    );
    let big = step(
        &states,
        EventKind::Constant,
        "Evaluate the constant 0x7FFFFFFFFFFFFFFF",
    );
    assert_eq!(big.event.result_type.as_deref(), Some("long"));
    let suffixed = step(
        &states,
        EventKind::Constant,
        "Evaluate the constant 0x10u = 16",
    );
    assert!(
        suffixed
            .event
            .explanation
            .contains("the first of unsigned int, unsigned long and unsigned long long"),
        "{}",
        suffixed.event.explanation
    );
    let zero = states
        .iter()
        .find(|state| state.event.title == "Evaluate the constant 0")
        .expect("a plain 0");
    assert_eq!(
        zero.event.summary,
        "The constant 0 is simply the int value 0."
    );
    returned(
        "int main(void) { return 0x7fffffff == 2147483647 && 017 == 15; }",
        1,
    );
    returned(
        "int main(void) { return 0xFFFFFFFF > 0 && -1 < 0xFFFFFFFFFFFFFFFF; }",
        0,
    );
}

// C99 §6.5.2.1p1-2: `E1[E2]` is `*((E1) + (E2))`, so the pointer may come
// second.
#[test]
fn a_subscript_takes_its_pointer_in_either_order() {
    returned(
        "int main(void) { int v[3] = {10, 20, 30}; int i = 2; return i[v] + 1[v]; }",
        50,
    );
    let states = stepped("int main(void) { int v[3] = {10, 20, 30}; int i = 2; return i[v]; }");
    let subscript = step(&states, EventKind::Subscript, "Select");
    assert!(
        subscript.event.summary.contains("*(i + v)"),
        "{}",
        subscript.event.summary
    );
}

// C99 §6.5.2.2p10: a call is an operand like any other, with a sequence
// point before it; the machine evaluates operands left to right.
#[test]
fn calls_are_operands_of_larger_expressions() {
    returned(
        "int square(int n) { return n * n; } int main(void) { return square(2) + square(3); }",
        13,
    );
    returned(
        "int twice(int n) { return 2 * n; } int main(void) { return twice(twice(3)) - \
         (int)(double)twice(1); }",
        10,
    );
    returned(
        "int is_even(int n) { return n % 2 == 0; } int main(void) { int n = 4; int v[2] = \
         {is_even(n), -is_even(3)}; return is_even(n) && n > 2 ? v[0] + v[1] : 5; }",
        1,
    );
    let states = stepped(
        "#include <stdio.h>\nint square(int n) { return n * n; }\nint main(void) { \
         printf(\"%d\\n\", square(3)); return 0; }",
    );
    assert_eq!(
        states.last().map(|state| state.output.clone()),
        Some(b"9\n".to_vec())
    );
}

// C99 §6.5.3.2p3-4: `&` of a pointer object makes a pointer to a pointer,
// and each `*` follows one pointer.
#[test]
fn pointers_to_pointers_follow_one_pointer_per_star() {
    let source = "int main(void) {\n    int x = 1;\n    int *p = &x;\n    int **pp = &p;\n    \
                  **pp = 5;\n    return x;\n}\n";
    returned(source, 5);
    let states = stepped(source);
    let declared = step(&states, EventKind::Declare, "Declare `pp`");
    assert_eq!(declared.event.title, "Declare `pp` (int **)");
    assert!(
        declared.event.summary.contains("a pointer to an int *"),
        "{}",
        declared.event.summary
    );
    let pp = states
        .last()
        .and_then(|state| state.objects.iter().find(|object| object.name == "pp"))
        .expect("pp");
    assert_eq!((pp.r#type.as_str(), pp.size), ("int **", 8));
    let outer = step(&states, EventKind::Dereference, "`*pp` designates `p`");
    assert!(outer.event.summary.contains("follows the pointer `pp`"));
    let inner = step(&states, EventKind::Dereference, "`**pp` designates `x`");
    assert_eq!(
        inner.event.summary,
        "`**pp` follows the pointer `*pp` to the object it points to: `**pp` now stands for `x` \
         itself."
    );
    // A function given a pointer to a caller's pointer can change it.
    returned(
        "int pick(int **out, int *choice) { *out = choice; return 0; }\nint main(void) { int a = \
         3; int *chosen = 0; pick(&chosen, &a); return *chosen; }",
        3,
    );
    returned(
        "int main(void) { int x = 7; int *p = &x; int **pp = &p; int ***ppp = &pp; return ***ppp \
         + (*pp == p) + (pp[0][0] == 7); }",
        9,
    );
    for source in [
        "int main(void) { int x = 1; int **pp = &x; return 0; }",
        "int main(void) { int *p = 0; char **pp = &p; return 0; }",
        "int main(void) { int *****p; return 0; }",
    ] {
        let _diagnostic = compile(source).expect_err(source);
    }
}

// C99 §6.7.5.2p3: an array may hold pointers; its name decays to a pointer
// to its first pointer, and `[]` takes its length from a brace-enclosed
// list (§6.7.8p22).
#[test]
fn arrays_hold_pointers_and_take_a_list_s_length() {
    let source = "int main(void) {\n    int a = 1;\n    int b = 2;\n    int c = 3;\n    int \
                  *slots[3] = {&a, &b, &c};\n    *slots[1] = 20;\n    return b;\n}\n";
    returned(source, 20);
    let states = stepped(source);
    let declared = step(&states, EventKind::Declare, "Declare `slots`");
    assert_eq!(declared.event.title, "Declare `slots` (int *[3])");
    let slots = states
        .last()
        .and_then(|state| state.objects.iter().find(|object| object.name == "slots"))
        .expect("slots");
    assert_eq!(slots.size, 24);
    assert!(matches!(&slots.value, Value::Array(elements) if elements.len() == 3));
    let element = step(&states, EventKind::LvalueConversion, "Read slots[1]");
    assert_eq!(element.event.title, "Read slots[1] → &b");
    returned(
        "#include <stdio.h>\nconst char *days[] = {\"Mon\", \"Tue\", \"Wed\"};\nint main(void) { \
         char *names[] = {\"Ada\", \"Grace\"}; int v[] = {4, 5, 6,}; printf(\"%s %s\\n\", \
         days[2], names[1]); char **next = names + 1; return (int)(sizeof v / sizeof v[0]) + \
         (*next)[0] + (int)sizeof days; }",
        3 + 71 + 24,
    );
    let output = stepped(
        "#include <stdio.h>\nint main(void) { char *names[3] = {\"Ada\", \"Grace\", \"Linus\"}; \
         for (int i = 0; i < 3; i++) { printf(\"%s\\n\", names[i]); } return 0; }",
    );
    assert_eq!(
        output.last().map(|state| state.output.clone()),
        Some(b"Ada\nGrace\nLinus\n".to_vec())
    );
    // An element left out of the list is a null pointer (C99 §6.7.8p21).
    returned(
        "int main(void) { int x = 1; int *p[2] = {&x}; return p[1] == 0; }",
        1,
    );
    let _diagnostic = compile("int main(void) { int v[]; return 0; }")
        .expect_err("an array of unknown size needs an initializer");
}

/// What the site's "Limits of this model" says the machine runs and
/// refuses: each claim it makes, checked.
#[test]
fn the_limits_the_site_states_hold() {
    for source in
        [
            // Decimal, octal and hexadecimal constants; every operator family.
            "int main(void) { int a = 10 + 010 + 0x10; a = a * 2 / 3 % 7 << 1 >> 1; a = (a & 3) | \
             (a ^ 5) | ~a; return a < 3 && a != 2 || !a ? -a : +a; }",
            // `?:` with structures, and the comma operator.
            "struct s { int x; }; int main(void) { struct s a = {1}; struct s b = {2}; struct s c \
             = a.x > 1 ? a : b; int n = (c.x, 3); return c.x + n; }",
            // Pointers to pointers and arrays of pointers.
            "int main(void) { int x = 1; int *p = &x; int **pp = &p; int *v[2] = {p, *pp}; return \
             *v[1]; }",
            // Calls anywhere in an expression.
            "int f(int n) { return n + 1; } int main(void) { return f(f(1)) * f(0) + (f(1) ? 2 : \
             3); }",
            // A statement's commas, and updates in a condition that sequence
            // points separate from other uses of the variable and from calls.
            "int main(void) { int i = 0; int j = 3; i++, j--; while (j-- > 0) {} return i; }",
            "int f(int n) { return n; } int main(void) { int i = 3; int n = 0; while (i-- > 0 && \
             f(i) && n < 5) n = n + 1; while (f(i++) < 2) {} return i == 3 && n == 2; }",
            // Subtracting and ordering pointers into one array or object.
            "struct s { int x; int y; }; int main(void) { int v[3] = {1, 2, 3}; int *p = v; int \
             *q = v + 3; struct s a = {1, 2}; return (q - p) + (p < q) + (&a.x < &a.y); }",
            // Assignments, compound assignments, ++ and -- anywhere in an
            // expression.
            "int main(void) { int x; int y; char s[3] = \"ab\"; char t[3]; char *d = t; const \
             char *c = s; x = y = 0; while ((*d++ = *c++) != 0) {} return x + (y += 2) + t[1]; }",
            "int k = 0; int next(void) { return k < 3 ? ++k : 0; } int main(void) { int c; int n \
             = 0; while ((c = next()) != 0) n += c; return n; }",
            // Pointers to functions, declared directly or with a typedef, in
            // variables, tables and parameters, called both ways, compared and
            // null; static tables of them; and constant expressions in static
            // initializers.
            "typedef int (*binop)(int, int); int add(int a, int b) { return a + b; } int \
             apply(binop f, int x) { return f(x, x); } int (*table[])(int, int) = {add, &add}; \
             static int mask = 1 << 3; int main(void) { int (*f)(int, int) = add; binop g = 0; \
             return apply(add, 1) + (*f)(1, 2) + table[1](1, 1) + (f == add) + (g == 0) + mask; }",
            // Enumerations and their constants in expressions, case labels and
            // array sizes; switch and break; typedefs of every type at file and
            // block scope; pointers to functions as members and return values.
            "enum color { RED, GREEN = 5, BLUE }; typedef struct { int x; } point; typedef enum \
             color          shade; int main(void) { int v[BLUE]; typedef double celsius; celsius \
             t = 1.5; point p =          {GREEN}; shade s = BLUE; int n = 0; switch (s) { case \
             RED: n = 1; break; case BLUE: n =          2; break; default: n = 3; } for (;;) { \
             break; } v[0] = n; return v[0] + p.x + (int)t; }",
            "int f(int x) { return x + 1; } struct ops { int (*apply)(int); }; int \
             (*pick(void))(int)          { return f; } typedef int (*unary)(int); unary \
             again(void) { return f; } int main(void) {          struct ops s = {f}; struct ops \
             *p = &s; return s.apply(1) + p->apply(2) + pick()(3) +          again()(4); }",
            // do, continue and goto; case labels anywhere in a switch's
            // body; functions that return pointers to objects.
            "int v[3] = {1, 2, 3}; int *at(int i) { return &v[i]; } int main(void) { int n = 0; \
             int i = 0; do { i++; if (i == 2) continue; n += *at(i - 1); } while (i < 3); for \
             (;;) { if (n > 0) goto out; } out: switch (n) { case 0: { case 4: n += 10; } } \
             return n; }",
            // A pointer to a pointer returned.
            "int x = 4; int *p; int **at(void) { return &p; } int main(void) { p = &x; return \
             **at(); }",
        ]
    {
        let session = run(source);
        assert_eq!(session.state().status, "finished", "{source}");
    }
    // The run stops where C's sequencing rules break, where a store reads
    // where it stores, and where an open order would decide the result.
    for (source, status) in [
        (
            "int main(void) { int i = 0; int *p = &i; *p = i++; return i; }",
            "undefined_behavior",
        ),
        (
            "int main(void) { int a[2] = {0, 5}; a[a[0]] = 1; return a[0]; }",
            "error",
        ),
        (
            "int g = 0; int f(void) { g = 1; return 1; } int main(void) { return g + f(); }",
            "error",
        ),
        (
            "#include <stdio.h>\nint main(void) { return printf(\"a\") + printf(\"b\"); }",
            "error",
        ),
        (
            "int main(void) { int (*f)(int) = 0; return f(1); }",
            "undefined_behavior",
        ),
        (
            "typedef int (*one)(int); int add(int a, int b) { return a + b; } int main(void) { \
             one f = (one)add; return f(1); }",
            "undefined_behavior",
        ),
    ] {
        let session = run(source);
        assert_eq!(session.state().status, status, "{source}");
    }
    for source in [
        "union u { int x; }; int main(void) { return 0; }",
        "void f(void) {} int main(void) { return 0; }",
        "static int f(void) { return 0; } int main(void) { return f(); }",
        "#include <stdio.h>\nint main(void) { int (*p)(const char *) = puts; return 0; }",
        "int x; int *p = &x; int main(void) { return 0; }",
        "int f(void) { return 1; } int x = f(); int main(void) { return x; }",
        "struct s { int x; }; int main(void) { struct s v[2]; return 0; }",
        "int main(void) { int v[2][2]; return 0; }",
        "int main(void) { int i = 0; int a[2] = {i++, 0}; return a[0]; }",
        "int main(void) { int i = 0; if (i++ + i) {} return 0; }",
        "int main(void) { int i = 0; i = i++; return i; }",
        "#include <string.h>\nint main(void) { return 0; }",
        "int main(void) { switch (1) { int x; case 1: return 0; } return 0; }",
        "void *f(void) { return 0; } int main(void) { return 0; }",
    ] {
        let _diagnostic = compile(source).expect_err(source);
    }
}

// C99 §6.2.4p5: a return ends the frame's objects; its lead names them
// by kind, and a shadowed name once, with how many objects it names.
#[test]
fn a_return_names_the_objects_it_ends_once_each() {
    let summary = |source: &str| {
        let states = stepped(source);
        step(&states, EventKind::Return, "Return 1 from f")
            .event
            .summary
            .clone()
    };
    assert!(
        summary("int f(int x) { { int x = 1; return x; } } int main(void) { return f(1); }")
            .ends_with("the frame of f ends: the two objects named `x` cease to exist."),
        "shadowed names are not repeated"
    );
    assert!(
        summary(
            "int f(int a, int b, int c, int d) { return a; } int main(void) { return f(1, 2, 3, \
             4); }"
        )
        .ends_with("its parameters cease to exist.")
    );
    assert!(
        summary(
            "int f(void) { int a = 1; int b = 2; int c = 3; int d = 4; return a; } int main(void) \
             { return f(); }"
        )
        .ends_with("its automatic locals cease to exist.")
    );
    assert!(
        summary(
            "int f(int a, int b) { int c = 1; int d = 2; return c; } int main(void) { return f(1, \
             2); }"
        )
        .ends_with("its parameters and automatic locals cease to exist.")
    );
}

// C99 §6.5.2.4p2, §6.5p2: `*s++` reads the element `s` points to and
// moves `s`; the machine decides by the object each access reaches whether
// two of them clash, so pointer idioms run and true clashes stop.
#[test]
fn pointer_idioms_follow_the_objects_they_reach() {
    let states =
        stepped(
            "int length(const char *s) {\n    const char *start = s;\n    while (*s++)\n        \
             ;\n    return s - start - 1;\n}\nint main(void) { return length(\"hello\"); }\n",
        );
    let last = states.last().expect("a final state");
    assert_eq!(last.status, "finished", "{:?}", last.diagnostics);
    assert_eq!(last.return_value, Some(Value::Integer(5)));
    let update = step(&states, EventKind::Increment, "Update `s`");
    assert!(
        update.event.summary.contains("yields `s`'s old value"),
        "{}",
        update.event.summary
    );
    assert!(
        update
            .event
            .explanation
            .contains("compares every access in the condition"),
        "{}",
        update.event.explanation
    );
    assert!(!update.event.sequence_point);
    assert!(cited(update).contains(&"6.5p2"));
    returned(
        "int main(void) { char s[] = \"copy\"; char t[8]; char *d = t; const char *c = s; while \
         ((*d++ = *c++) != 0) {} return t[3] * 0 + (int)(d - t); }",
        5,
    );
    returned(
        "int main(void) { int v[3]; int *p = v; *p++ = 1; *p++ = 2; *p = 3; return v[0] * 100 + \
         v[1] * 10 + v[2]; }",
        123,
    );
    returned(
        "int main(void) { int a[4] = {1, 2, 3, 4}; int *lo = a; int *hi = a + 3; while (lo < hi) \
         { int t = *lo; *lo++ = *hi; *hi-- = t; } return a[0] * 1000 + a[1] * 100 + a[2] * 10 + \
         a[3]; }",
        4321,
    );
    // Two pointers to one object, or a pointer to a variable the
    // expression also updates by name: the clash is decided on the object.
    returned(
        "int main(void) { int a[2] = {1, 2}; int i = 0; int j = 1; a[i] = a[j]++; return a[0] * \
         10 + a[1]; }",
        23,
    );
    for source in [
        "int main(void) { int i = 0; int *p = &i; *p = i++; return i; }",
        "int main(void) { int i = 0; int *a = &i; return a[i++]; }",
        "int main(void) { int x = 1; int *p = &x; x = (*p)++; return x; }",
        "int main(void) { int x = 0; int *p = &x; int *q = &x; return (*p)++ + (*q)++; }",
        "int main(void) { int a[2] = {1, 2}; int *p = a; int **pp = &p; *pp = p++; return *p; }",
        "int main(void) { int a[2] = {1, 2}; int i = 0; int j = 0; a[i] = a[j]++; return a[0]; }",
        "int main(void) { int x = 3; int y = 5; int *a = &x; int *b = &y; *a ^= *b ^= *a ^= *b; \
         return x; }",
        "int main(void) { int x = 0; int *p = &x; return (*p = 1) + x; }",
    ] {
        let session = run(source);
        let state = session.state();
        assert_eq!(state.status, "undefined_behavior", "{source}");
        let diagnostic = &state.diagnostics[0];
        assert_eq!(
            diagnostic.undefined,
            Some(super::Undefined::Unsequenced),
            "{source}"
        );
        assert!(
            diagnostic
                .message
                .contains("with no sequence point between the two"),
            "{source}: {}",
            diagnostic.message
        );
        assert_eq!(
            state.event.title,
            "Undefined behavior: unsequenced accesses"
        );
    }
    // A store that reads what it stores into to find where to store.
    let session = run("int main(void) { int a[2] = {0, 5}; a[a[0]] = 1; return a[0]; }");
    assert_eq!(session.state().status, "error");
    assert_eq!(
        session.state().event.title,
        "Stopped: a store that reads where it stores"
    );
}

// C99 §6.5.16p3: an assignment's value is the value stored in its left
// operand, and its store completes by the next sequence point.
#[test]
fn assignments_yield_the_value_they_store() {
    returned(
        "int main(void) { int x; int y; x = y = 7; return x + y; }",
        14,
    );
    returned(
        "int main(void) { int a = 0; int b = 5; a = b++; return a * 10 + b; }",
        56,
    );
    returned(
        "int k = 0; int next(void) { return k < 3 ? ++k : 0; } int main(void) { int c; int n = 0; \
         while ((c = next()) != 0) n += c; return n; }",
        6,
    );
    returned(
        "int main(void) { int total = 1; int r = (total += 4) * 2; return r + total; }",
        15,
    );
    returned(
        "int main(void) { int i = 1; i = (i++, i + 10); return i; }",
        12,
    );
    returned(
        "int id(int v) { return v + 1; } int main(void) { int x = 1; x = id(x++); return x; }",
        2,
    );
    let states = stepped("int main(void) { char c; int v = (c = 300); return v + 1; }");
    let store = step(&states, EventKind::Assign, "Store 44 into `c`");
    assert!(
        store
            .event
            .summary
            .contains("As an operand, `(c = 300)` yields that value, 44, to the expression"),
        "{}",
        store.event.summary
    );
    assert!(
        store
            .event
            .explanation
            .contains("by the next sequence point, the end of the full expression"),
        "{}",
        store.event.explanation
    );
    assert!(!store.event.sequence_point);
    assert!(cited(store).contains(&"6.5.16p3"));
    assert_eq!(
        states.last().map(|state| state.return_value.clone()),
        Some(Some(Value::Integer(45)))
    );
    let states = stepped(
        "int half(int x) { return x / 2; } int main(void) { int n; if ((n = half(9)) > 0) return \
         n; return 0; }",
    );
    let store = step(&states, EventKind::Assign, "Store 4 into `n`");
    assert!(
        store
            .event
            .summary
            .contains("yields that value, 4, to the condition"),
        "{}",
        store.event.summary
    );
    assert!(
        store.event.explanation.contains("the end of the condition"),
        "{}",
        store.event.explanation
    );
    // The same variable stored twice with nothing between: refused.
    for source in [
        "int main(void) { int i = 1; i = i++; return i; }",
        "int main(void) { int x = 0; x = (x = 1) + 1; return x; }",
        "int main(void) { int x = 1; x += x++; return x; }",
        "int main(void) { int i = 0; int a[2] = {i++, 0}; return a[0]; }",
    ] {
        let _diagnostic = compile(source).expect_err(source);
    }
}

// C99 §6.5.6p9, §6.5.8p5: pointers into one array subtract to the count
// of elements between them and order by subscript; others are undefined.
#[test]
fn pointer_differences_count_elements_and_order_by_place() {
    let states = stepped(
        "int main(void) { int v[5] = {1, 2, 3, 4, 5}; int *p = &v[1]; int *q = &v[4]; long n = q \
         - p; return (int)n; }",
    );
    let difference = step(&states, EventKind::PointerArithmetic, "Pointer difference");
    assert_eq!(difference.event.title, "Pointer difference: q - p = 3");
    assert_eq!(difference.event.result_type.as_deref(), Some("long"));
    assert!(
        difference.event.summary.contains("4 - 1 = 3, a ptrdiff_t"),
        "{}",
        difference.event.summary
    );
    assert_eq!(cited(difference)[0], "6.5.6p9");
    returned(
        "int main(void) { int v[3] = {1, 2, 3}; int *p; int n = 0; for (p = v; p < v + 3; p++) n \
         += *p; return n * 10 + (p == v + 3) + (v + 1 <= p); }",
        62,
    );
    returned(
        "struct pt { int x; int y; }; int main(void) { struct pt a = {1, 2}; int *p = &a.x; int \
         *q = &a.y; return (p < q) * 10 + (q >= p); }",
        11,
    );
    returned(
        "int main(void) { char s[10]; return sizeof(&s[9] - &s[0]); }",
        8,
    );
    for (source, cause) in [
        (
            "int main(void) { int x = 1; int y = 2; int *p = &x; int *q = &y; return q - p; }",
            super::Undefined::PointerDifference,
        ),
        (
            "int main(void) { int x[2] = {1, 2}; int y[2] = {3, 4}; return &x[0] < &y[0]; }",
            super::Undefined::PointerOrder,
        ),
        (
            "int main(void) { int v[2] = {1, 2}; int *p = 0; int *q = v; return q - p; }",
            super::Undefined::PointerDifference,
        ),
        (
            "struct two { int a[2]; int b[2]; }; int main(void) { struct two t = {{1, 2}, {3, \
             4}}; int *p = t.a; int *q = t.b; return q - p; }",
            super::Undefined::PointerDifference,
        ),
    ] {
        assert_eq!(undefined(source).0, Some(cause), "{source}");
    }
    let diagnostic =
        compile("int main(void) { int v[2] = {1, 2}; char c[2] = \"a\"; return &v[1] - &c[0]; }")
            .map(|_| ())
            .expect_err("pointers to different types");
    assert_eq!(diagnostic.kind, "constraint");
    assert_eq!(diagnostic.clause.map(|clause| clause.id), Some("6.5.6p3"));
}

// C99 §6.5p3, §6.5.2.2p10: a call's body runs before or after the other
// operands as a whole; when the order would decide the result, the machine
// stops instead of choosing.
#[test]
fn an_order_c_leaves_open_stops_the_run() {
    for source in [
        "int g = 0; int bump(void) { g = g + 1; return g; } int main(void) { return g + bump(); }",
        "#include <stdio.h>\nint counter = 0; int next(void) { return ++counter; } int main(void) \
         { printf(\"%d %d\\n\", next(), next()); return 0; }",
        "#include <stdio.h>\nint main(void) { return printf(\"a\") + printf(\"b\"); }",
        "int set(int *p) { *p = 5; return 1; } int main(void) { int x = 0; return set(&x) + x; }",
        "int counter = 0; int next(void) { return ++counter; } int main(void) { int a[2] = \
         {next(), next()}; return a[0]; }",
    ] {
        let session = run(source);
        let state = session.state();
        assert_eq!(state.status, "error", "{source}");
        let listed = source.contains("int a[2] =");
        assert_eq!(
            state.diagnostics[0].clause.map(|clause| clause.id),
            Some(if listed { "6.7.8p23" } else { "6.5p3" }),
            "{source}"
        );
        assert_eq!(
            state.event.title,
            "Stopped: the result can depend on an open order"
        );
        assert!(
            state.diagnostics[0]
                .message
                .contains("C99 leaves unspecified which of the two is evaluated first"),
            "{source}: {}",
            state.diagnostics[0].message
        );
    }
    // The order can decide the result, not must: `inc() + inc()` is 3
    // either way, and the stop says only that it can depend on the order.
    // Of an initializer list's elements, C99 §6.7.8p23 leaves the order
    // open.
    let session =
        run("int g = 0; int inc(void) { return ++g; } int main(void) { return inc() + inc(); }");
    let state = session.state();
    assert_eq!(state.status, "error");
    assert!(
        state.diagnostics[0]
            .message
            .contains("the result can depend on it"),
        "{}",
        state.diagnostics[0].message
    );
    assert!(
        state
            .event
            .summary
            .contains("the result can depend on which runs first"),
        "{}",
        state.event.summary
    );
    for text in [&state.event.summary, &state.event.explanation] {
        assert!(!text.contains("different result"), "{text}");
    }
    let session = run(
        "int g = 0; int inc(void) { return ++g; } int main(void) { int a[2] = {inc(), g}; return \
         a[0]; }",
    );
    let state = session.state();
    assert_eq!(state.status, "error");
    assert_eq!(
        state.diagnostics[0].clause.map(|clause| clause.id),
        Some("6.7.8p23")
    );
    assert!(
        state.diagnostics[0]
            .message
            .contains("another initializer, `g`, reads it"),
        "{}",
        state.diagnostics[0].message
    );
    assert!(
        state
            .event
            .summary
            .contains("another initializer in the list"),
        "{}",
        state.event.summary
    );
    // Calls that only read, a call's store before the assignment that uses
    // its value, and calls on different objects run.
    returned(
        "int g = 3; int get(void) { return g; } int main(void) { return get() + get() + g; }",
        9,
    );
    returned(
        "int g = 2; int f(void) { g = 9; return 4; } int main(void) { g = f(); return g; }",
        4,
    );
    returned(
        "int inc(int *p) { *p = *p + 1; return *p; } int main(void) { int x = 0; int y = 5; \
         return inc(&x) + inc(&y); }",
        7,
    );
}

// C99 §6.5.16.2p3: `E1 op= E2` is `E1 = E1 op (E2)` with `E1` evaluated
// once, so its read of `E1` is an operand of `op`, unsequenced with all of
// `E2` (§6.5p3): an update of `E1`'s object inside `E2` is undefined even
// behind a sequence point there (§6.5p2), as the spelled-out form is,
// and a call there that stores into it leaves the order open.
#[test]
fn a_compound_assignment_reads_its_left_operand_unsequenced_with_its_right() {
    // By name: refused before the first step.
    for source in [
        "int main(void) { int x = 1; x += (x++, 5); return x; }",
        "int main(void) { int x = 1; x += (x++ && 1); return x; }",
        "int main(void) { int x = 1; x |= (x++ || 1); return x; }",
        "int main(void) { int x = 1; x -= x++ ? 1 : 2; return x; }",
        "int main(void) { int x = 1; x *= (x = 3, 2); return x; }",
        "int id(int v) { return v; } int main(void) { int x = 1; x += id(x++); return x; }",
        "int main(void) { int x = 1; if ((x <<= (x++, 1)) > 0) {} return x; }",
        "struct s { int a; }; int main(void) { struct s v = {1}; struct s w = {2}; v.a %= (v = w, \
         4); return v.a; }",
    ] {
        let diagnostic = compile(source).expect_err(source);
        assert!(
            diagnostic.message.contains("and its right operand updates"),
            "{source}: {}",
            diagnostic.message
        );
    }
    // The refusal is a limit of the model, not a claim that C makes these
    // undefined: an update that is never evaluated breaks no rule, so the
    // message says C99 makes the clash undefined only when the update is
    // evaluated. GCC gives 1 and 2.
    for source in [
        "int main(void) { int x = 1; x += (0 && x++); return x; }",
        "int main(void) { int c = 1; int x = 1; x += (c ? 1 : x++); return x; }",
    ] {
        let message = compile(source).expect_err(source).message;
        assert!(
            message.contains("C99 makes that undefined whenever the update is evaluated (§6.5p2)"),
            "{source}: {message}"
        );
        assert!(
            !message.contains("which C99 makes undefined"),
            "{source}: {message}"
        );
    }
    // The spelled-out form is refused too.
    let _diagnostic = compile("int main(void) { int x = 1; x = x + (x++, 5); return x; }")
        .expect_err("x = x + (x++, 5)");
    // Through a pointer, a subscript or a member: the run stops there.
    for source in [
        "int main(void) { int x = 1; int *p = &x; *p += (x++, 5); return x; }",
        "int id(int v) { return v; } int main(void) { int x = 1; int *p = &x; *p += id((*p)++); \
         return x; }",
        "int main(void) { int x = 1; int *p = &x; x /= (*p = 3, 2); return x; }",
        "int main(void) { int a[2] = {1, 2}; int i = 0; int j = 0; a[i] ^= (a[j]++ || 1); return \
         a[0]; }",
        "struct s { int a; }; int main(void) { struct s v = {8}; v.a >>= (v.a++, 1); return v.a; }",
    ] {
        let session = run(source);
        let state = session.state();
        assert_eq!(state.status, "undefined_behavior", "{source}");
        assert_eq!(
            state.diagnostics[0].undefined,
            Some(super::Undefined::Unsequenced),
            "{source}"
        );
    }
    // Defined: another object updated, the left operand's object only
    // read, a sequence point inside the left operand, and a plain update.
    returned(
        "int main(void) { int x = 1; int y = 0; x += (y++, 5); return x * 10 + y; }",
        61,
    );
    returned(
        "int main(void) { int x = 1; int y = 0; int *p = &x; *p += (y++, 5); return x * 10 + y; }",
        61,
    );
    returned("int main(void) { int x = 2; x *= (x, 3); return x; }", 6);
    returned(
        "int main(void) { int x = 1; *(x++, &x) += 1; return x; }",
        3,
    );
    returned("int main(void) { int x = 1; x += 1; return x; }", 2);
    // A call in the right operand that stores into the left operand's
    // object: C99 lets the read come before or after the call, 6 or 15
    // here, so the run stops as an open order rather than show GCC's 15.
    for source in [
        "int g = 1; int bump(void) { g = g * 10; return 5; } int main(void) { g += bump(); return \
         g; }",
        "int g = 1; int bump(void) { g = g * 10; return 5; } int main(void) { int *p = &g; *p += \
         (bump(), 1); return g; }",
    ] {
        let session = run(source);
        let state = session.state();
        assert_eq!(state.status, "error", "{source}");
        assert_eq!(
            state.diagnostics[0].clause.map(|clause| clause.id),
            Some("6.5p3"),
            "{source}"
        );
        assert_eq!(
            state.event.title,
            "Stopped: the result can depend on an open order"
        );
        assert!(
            state.diagnostics[0]
                .message
                .contains("` reads it as the left operand of its `+` (§6.5.16.2p3)"),
            "{source}: {}",
            state.diagnostics[0].message
        );
    }
    // A call that only reads the object, or stores into another, runs.
    returned(
        "int g = 1; int get(void) { return g; } int main(void) { g += get(); return g; }",
        2,
    );
    returned(
        "int g = 1; int h = 0; int bump(void) { h = 7; return 5; } int main(void) { g += bump(); \
         return g + h; }",
        13,
    );
}

// C99 §6.5p2 with §6.5.16p3: an update the right operand completes at a
// sequence point of its own comes before the assignment's store, whether
// the store is to a variable or to a member of one; `s.a` designates part
// of `s` without reading it (§6.5.2.3p3).
#[test]
fn a_member_store_after_a_sequenced_update_of_its_structure_runs() {
    returned(
        "int main(void) { int i = 0; i = (i = 3, i + 10); return i; }",
        13,
    );
    returned(
        "struct s { int a; int b; }; int main(void) { struct s v = {1, 2}; struct s w = {3, 4}; \
         v.a = (v = w, 5); return v.a * 10 + v.b; }",
        54,
    );
    returned(
        "struct i { int a; int b; }; struct s { int a; struct i in; }; int main(void) { struct s \
         v = {1, {2, 3}}; struct s w = {7, {8, 9}}; v.in.a = (v = w, 5); return v.in.a * 10 + \
         v.a; }",
        57,
    );
    // The same update unsequenced with a read, or before a compound
    // assignment's read, is still refused.
    for source in [
        "struct s { int a; }; int main(void) { struct s v = {1}; struct s w = {2}; v.a = (v = w, \
         5) + v.a; return v.a; }",
        "struct s { int a; }; int main(void) { struct s v = {1}; struct s w = {2}; v.a += (v = w, \
         5); return v.a; }",
    ] {
        let _diagnostic = compile(source).expect_err(source);
    }
}

// Review fixes: a member of an assignment's value is refused for what it
// is; a substatement without braces is a block all the same (C99
// §6.8.4p3, §6.8.5p5); a quote of code spread over lines reads as one
// line.
#[test]
fn small_review_fixes_say_what_happens() {
    let diagnostic = compile(
        "struct s { int a; }; int main(void) { struct s v = {1}; struct s w = {2}; int r = (v = \
         w).a; return r; }",
    )
    .expect_err("(v = w).a");
    assert!(
        diagnostic
            .message
            .contains("a member of an assignment's value"),
        "{}",
        diagnostic.message
    );
    let states =
        stepped(
            "#include <stdio.h>\nint main(void) {\n    int i = 0;\n    while (i < 1)\n        \
             i++;\n    if (i)\n        i = 2;\n    printf(\"%d\\n\",\n           i);\n    return \
             0;\n}\n",
        );
    let entries: Vec<&State> = states
        .iter()
        .filter(|state| state.event.kind == EventKind::EnterBlock)
        .collect();
    // `main`'s braces begin `i`'s lifetime.
    assert!(!entries[0].event.summary.contains("no braces"));
    assert!(cited(entries[0]).contains(&"6.8.2p2"));
    let body = entries
        .iter()
        .find(|state| state.event.summary.contains("a loop body"))
        .expect("the loop body's block");
    assert!(body.event.summary.contains("no braces"));
    assert!(cited(body).contains(&"6.8.5p5"));
    let branch = entries
        .iter()
        .find(|state| state.event.summary.contains("if statement"))
        .expect("the branch's block");
    assert!(cited(branch).contains(&"6.8.4p3"));
    // "What comes next" before leaving each of them cites the same
    // paragraph as the step that entered it, not the compound statement's.
    let leaving: Vec<&str> = states
        .iter()
        .filter_map(|state| state.upcoming.as_ref())
        .filter(|upcoming| upcoming.text == "leave the block")
        .map(|upcoming| upcoming.clause.id)
        .collect();
    assert_eq!(leaving, ["6.8.5p5", "6.8.4p3"]);
    let discard = step(&states, EventKind::Discard, "Discard the result");
    assert!(
        discard
            .event
            .explanation
            .starts_with("`printf(\"%d\\n\", i)` is an expression statement"),
        "{}",
        discard.event.explanation
    );
}

/// The corpus of small beginner programs under `web/docs/corpus/`, one per
/// construct, and what the machine makes of each: every program runs to its
/// end
/// those whose behavior C leaves undefined (`u…`), which stop there,
/// those whose result depends on an order C leaves open (`o…`), which stop
/// rather than choose one, and those that read an indeterminate value
/// (`x…`), which stop there without calling it more than C does.
#[test]
fn the_corpus_of_beginner_programs_runs() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web/docs/corpus");
    let mut paths: Vec<_> = std::fs::read_dir(&directory)
        .expect("the corpus directory")
        .map(|entry| entry.expect("a corpus entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "c"))
        .collect();
    paths.sort();
    assert!(paths.len() >= 30, "the corpus has its programs");
    for path in paths {
        let source = std::fs::read_to_string(&path).expect("a corpus program");
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let program =
            compile(&source).unwrap_or_else(|diagnostic| panic!("{name}: {diagnostic:?}"));
        let mut session = program.session();
        while !session.state().terminal() {
            let upcoming = session.state().upcoming.clone();
            let state = session.step();
            foretold(upcoming.as_ref(), state);
        }
        let expected = match name.as_bytes().first() {
            | Some(b'u') => "undefined_behavior",
            | Some(b'o' | b'x') => "error",
            | _ => "finished",
        };
        assert_eq!(
            session.state().status,
            expected,
            "{name}: {:?}",
            session.state().diagnostics
        );
    }
}

/// The program `source` stepped to its end, as `stepped` does, with the
/// last state.
fn finished(source: &str, expected: i128) -> Vec<State> {
    let states = stepped(source);
    let last = states.last().expect("a final state");
    assert_eq!(last.status, "finished", "{source}: {:?}", last.diagnostics);
    assert_eq!(
        last.return_value,
        Some(Value::Integer(expected)),
        "{source}"
    );
    states
}

// C99 §6.3.2.1p4, §6.5.2.2p1, p3, §6.5.3.2p3-4, §6.5.9p6: a function's
// name becomes a pointer to it, `&f` and `*fp` give the same pointer, and a
// call through a pointer runs the function it points to.
#[test]
fn pointers_to_functions_are_values_a_call_follows() {
    let states = finished(
        "int square(int x) { return x * x; }\nint twice(int x) { return 2 * x; }\nint main(void) \
         {\n    int (*f)(int) = square;\n    int (*g)(int) = &twice;\n    return f(3) + (*g)(4) + \
         (f == square) + (f != g);\n}\n",
        19,
    );
    let designator = step(&states, EventKind::FunctionDesignator, "`square` becomes");
    assert_eq!(
        designator.event.result,
        Some(Operand::Value(Value::Function(0)))
    );
    assert!(
        designator.event.explanation.contains("defined on line 1")
            && designator.event.explanation.contains("of type int (int)")
            && designator
                .event
                .explanation
                .contains("A function is code, not an object"),
        "{}",
        designator.event.explanation
    );
    assert_eq!(designator.event.clauses[0].id, "6.3.2.1p4");
    let address = step(
        &states,
        EventKind::AddressOf,
        "Take the address of the function twice",
    );
    assert!(
        address
            .event
            .summary
            .contains("the same pointer `twice` alone would become")
    );
    let indirection = step(
        &states,
        EventKind::Dereference,
        "`*g` designates the function twice",
    );
    assert!(
        indirection
            .event
            .explanation
            .contains("`(*g)(…)` and `g(…)` call the same function")
    );
    let call = step(&states, EventKind::Call, "Call square through `f`");
    assert!(
        call.event
            .summary
            .starts_with("`f` holds a pointer to square, so the call runs square"),
        "{}",
        call.event.summary
    );
    assert!(
        call.event
            .explanation
            .contains("promised a function of type int (int)")
    );
    assert_eq!(call.event.clauses[0].id, "6.5.2.2p1");
    let _call = step(&states, EventKind::Call, "Call twice through `(*g)`");
    let equal = step(
        &states,
        EventKind::Comparison,
        "Compare pointers to functions: f == square",
    );
    assert_eq!(equal.event.result, Some(Operand::Value(Value::Integer(1))));
    assert_eq!(equal.event.clauses[0].id, "6.5.9p6");
    // Table 1 and the cells spell the type as C does.
    let last = states.last().expect("a final state");
    let f = last
        .objects
        .iter()
        .find(|object| object.name == "f")
        .expect("f");
    assert_eq!(f.r#type, "int (*)(int)");
    assert_eq!(f.size, 8);
}

// C99 §6.7.7p3, §6.7.5.3p8, §6.6p9: a typedef names a pointer type, a
// table holds pointers, a parameter takes one, and a static table points to
// functions before main runs.
#[test]
fn tables_typedefs_and_callbacks_of_pointers_to_functions_run() {
    let _states =
        finished(
            "typedef int (*binop)(int, int);\nint add(int a, int b) { return a + b; }\nint \
             mul(int a, int b) { return a * b; }\nint main(void) {\n    binop op = add;\n    int \
             s = op(2, 3);\n    op = mul;\n    return s + op(4, 5);\n}\n",
            25,
        );
    let states = finished(
        "int add(int a, int b) { return a + b; }\nint sub(int a, int b) { return a - b; }\nint \
         main(void) {\n    int (*ops[2])(int, int) = {add, sub};\n    int total = 0;\n    int \
         i;\n    for (i = 0; i < 2; i++)\n        total = total + ops[i](6, 2);\n    return \
         total;\n}\n",
        12,
    );
    let table = states
        .last()
        .and_then(|state| state.objects.iter().find(|object| object.name == "ops"))
        .expect("ops");
    assert_eq!(table.r#type, "int (*[2])(int, int)");
    assert_eq!(
        table.value,
        Value::Array(vec![Value::Function(0), Value::Function(1)])
    );
    let _call = step(&states, EventKind::Call, "Call sub through `ops[i]`");
    let states = finished(
        "int apply(int (*f)(int), int x) { return f(x); }\nint square(int x) { return x * x; \
         }\nint main(void) { return apply(square, 7); }\n",
        49,
    );
    let call = step(&states, EventKind::Call, "Call apply");
    assert!(
        call.event
            .explanation
            .contains("`f` = a pointer to the function square")
    );
    let states = finished(
        "int add(int a, int b);\nint (*ops[])(int, int) = {add, &add};\nint add(int a, int b) { \
         return a + b; }\nint main(void) { return ops[0](1, 2) + ops[1](3, 4); }\n",
        10,
    );
    let table = step(&states, EventKind::StaticStorage, "Before main: `ops`");
    assert!(
        table
            .event
            .explanation
            .contains("`add` and `&add` are address constants"),
        "{}",
        table.event.explanation
    );
}

// C99 §6.5.2.2p9, §6.3.2.3p3, p8, §6.5.3.2p4: a call through a null pointer,
// `*` of one, and a call through a pointer cast to another function type
// stop as undefined, each with its cause.
#[test]
fn undefined_calls_through_pointers_to_functions_stop() {
    let (cause, title) = undefined("int main(void) { int (*f)(int) = 0; return f(1); }");
    assert_eq!(cause, Some(super::Undefined::NullCall));
    assert_eq!(title, "Undefined behavior: call through a null pointer");
    let (cause, _) = undefined(
        "int add(int a, int b) { return a + b; } int main(void) { int (*ops[2])(int, int) = \
         {add}; return ops[1](1, 2); }",
    );
    assert_eq!(cause, Some(super::Undefined::NullCall));
    let (cause, _) = undefined("int main(void) { int (*f)(int) = 0; return (*f)(1); }");
    assert_eq!(cause, Some(super::Undefined::NullFunctionIndirection));
    let source = "typedef int (*one)(int);\nint add(int a, int b) { return a + b; }\nint \
                  main(void) {\n    one f = (one)add;\n    return f(3);\n}\n";
    let (cause, title) = undefined(source);
    assert_eq!(cause, Some(super::Undefined::CallType));
    assert_eq!(
        title,
        "Undefined behavior: call through the wrong function type"
    );
    let session = run(source);
    let diagnostic = &session.state().diagnostics[0];
    assert_eq!(diagnostic.clause.map(|clause| clause.id), Some("6.5.2.2p9"));
    // Undefined whether or not the two types pass their values in the same
    // places; a disagreement about where they are is only one consequence.
    let summary = &session.state().event.summary;
    assert!(
        summary.contains("has no meaning, even when it seems"),
        "{summary}"
    );
    assert!(summary.contains("may disagree about where"), "{summary}");
    assert!(
        diagnostic
            .message
            .contains("`f` points to add, a function of type int (int, int)"),
        "{}",
        diagnostic.message
    );
    // The same cast, undone before the call, is defined.
    returned(
        "typedef int (*one)(int); typedef int (*two)(int, int); int add(int a, int b) { return a \
         + b; } int main(void) { one f = (one)add; two g = (two)f; return g(3, 4); }",
        7,
    );
    let cast = stepped(
        "typedef int (*one)(int); int add(int a, int b) { return a + b; } int main(void) { one f \
         = (one)add; return 0; }",
    );
    let cast = step(
        &cast,
        EventKind::Cast,
        "Cast to int (*)(int): another function type",
    );
    assert!(cast.event.explanation.contains("and back again, unchanged"));
}

// C99 §6.5.2.2p1-2, §6.5.6p2, §6.5.8p2, §6.5.9p2, §6.5.16.1p1, §6.5.3.4p1:
// what C does not allow with pointers to functions is refused, as a
// constraint where C says so.
#[test]
fn constraints_on_pointers_to_functions_are_refused() {
    for (source, needle) in [
        (
            "int add(int a, int b) { return a + b; } int main(void) { int (*f)(int) = add; return \
             0; }",
            "a pointer to a function converts only to a pointer to a function of the same type",
        ),
        (
            "int f(int x) { return x; } int main(void) { int (*p)(int) = f; return p(1, 2); }",
            "this call passes 2 arguments, but the pointer points to a function of type int \
             (int), which takes 1",
        ),
        (
            "int main(void) { int x = 3; return x(2); }",
            "only a function or a pointer to a function can be called",
        ),
        (
            "int f(int x) { return x; } int main(void) { int (*p)(int) = f; p = p + 1; return 0; }",
            "`+` does not take a pointer to a function",
        ),
        (
            "int f(int x) { return x; } int main(void) { int (*p)(int) = f; return p < p; }",
            "`<` does not take a pointer to a function",
        ),
        (
            "int f(int x) { return x; } int g(int a, int b) { return a; } int main(void) { int \
             (*p)(int) = f; return p == g; }",
            "== and != compare a pointer to a function with a pointer to a function of the same \
             type",
        ),
        (
            "int f(int x) { return x; } int main(void) { return sizeof f; }",
            "sizeof does not apply to a function",
        ),
        (
            "int f(int x) { return x; } int main(void) { int (*p)(int) = f; p++; return 0; }",
            "a pointer to a function points to no object",
        ),
        (
            "int f(int x) { return x; } int g(int x) { return x; } int main(void) { f = g; return \
             0; }",
            "a function designator names a function, not an object",
        ),
        (
            "int apply(int f(int), int x) { return f(x); } int main(void) { return 0; }",
            "write that pointer, as in `int (*f)(int)`",
        ),
        (
            "typedef int (*f)(int); int main(void) { const f p = 0; return 0; }",
            "makes the pointer itself const",
        ),
        (
            "int f(int a, int b) { return a; } int (*p)(int) = f; int main(void) { return 0; }",
            "is initialized only from a function of its own type",
        ),
        (
            "int g(int x); int (*p)(int) = g; int main(void) { return 0; }",
            "`g` is declared but never defined",
        ),
    ] {
        let message = rejected(source);
        assert!(message.contains(needle), "{source}: {message}");
    }
}

// C99 §6.5.3.2p4, §6.3.2.1p4, §6.5.2p1: each `*` before a pointer to a
// function yields the function designator, which converts straight back to
// the pointer. The explanation says how its operand gives the pointer, and
// every spelling it quotes is C, parenthesized where a call needs it.
#[test]
fn each_star_before_a_pointer_to_a_function_is_explained_in_c() {
    let states = finished(
        "int sq(int x) { return x * x; }\nint main(void) {\n    int (*fp)(int) = sq;\n    int a = \
         (*sq)(1);\n    int b = (**sq)(2);\n    int c = (***sq)(3);\n    int d = (*&sq)(4);\n    \
         int (*e)(int) = &*sq;\n    int g = (*fp)(5);\n    int h = (**fp)(6);\n    int i = \
         (*&fp)(7);\n    int j = e(8);\n    return a + b + c + d + g + h + i + j;\n}\n",
        204,
    );
    for (title, gives, same) in [
        (
            "`*sq` designates",
            "`sq` designates the function sq, and like any function designator it converts to a \
             pointer to that function.",
            "`(*sq)(…)` and `sq(…)` call the same function",
        ),
        (
            "`**sq` designates",
            "`*sq` designates the function sq too, and converts to a pointer to it.",
            "`(**sq)(…)` and `(*sq)(…)` call the same function",
        ),
        (
            "`***sq` designates",
            "`**sq` designates the function sq too",
            "`(***sq)(…)` and `(**sq)(…)` call the same function",
        ),
        (
            "`*&sq` designates",
            "`&sq` yields a pointer to the function sq.",
            "`(*&sq)(…)` and `(&sq)(…)` call the same function",
        ),
        (
            "`*fp` designates",
            "`fp` holds a pointer to the function sq.",
            "`(*fp)(…)` and `fp(…)` call the same function",
        ),
        (
            "`**fp` designates",
            "`*fp` designates the function sq too",
            "`(**fp)(…)` and `(*fp)(…)` call the same function",
        ),
    ] {
        let explanation = &step(&states, EventKind::Dereference, title)
            .event
            .explanation;
        assert!(
            explanation.starts_with(gives) && explanation.contains(same),
            "{title}: {explanation}"
        );
    }
    // No step claims a call spelling C reads otherwise: `*sq(…)` applies *
    // to what sq returns, and `&sq(…)` takes the address of a value.
    for state in &states {
        let event = &state.event;
        for wrong in [
            "`*sq(…)`",
            "`&sq(…)`",
            "`*fp(…)`",
            "`sq` holds",
            "`*sq` holds",
        ] {
            assert!(
                !event.explanation.contains(wrong) && !event.summary.contains(wrong),
                "{}: {}",
                event.title,
                event.explanation
            );
        }
    }
    // `*&fp` is the object `fp`, which yields its pointer for the call.
    let call = step(&states, EventKind::Call, "Call sq through `(*&fp)`");
    assert!(
        call.event
            .summary
            .starts_with("`(*&fp)` yields a pointer to sq"),
        "{}",
        call.event.summary
    );
    // An operand that is not a postfix expression keeps its parentheses.
    let states = finished(
        "int sq(int x) { return x * x; }\nint main(void) {\n    int c = 1;\n    int a = (*(int \
         (*)(int))sq)(2);\n    int b = (*(c ? sq : sq))(3);\n    return a + b + (sq)(4);\n}\n",
        29,
    );
    let cast = step(
        &states,
        EventKind::Dereference,
        "`*(int (*)(int))sq` designates",
    );
    assert!(
        cast.event.explanation.contains(
            "`(*(int (*)(int))sq)(…)` and `((int (*)(int))sq)(…)` call the same function"
        ),
        "{}",
        cast.event.explanation
    );
    let conditional = step(
        &states,
        EventKind::Dereference,
        "`*(c ? sq : sq)` designates",
    );
    assert!(
        conditional
            .event
            .explanation
            .contains("`(*(c ? sq : sq))(…)` and `(c ? sq : sq)(…)` call the same function"),
        "{}",
        conditional.event.explanation
    );
    let named = step(&states, EventKind::Call, "Call sq through `(sq)`");
    assert!(
        named
            .event
            .summary
            .starts_with("`(sq)` designates sq and converts to a pointer to it"),
        "{}",
        named.event.summary
    );
}

// C99 §6.3.2.3p3, §6.6p6: a null pointer constant is any integer constant
// expression with the value 0, or one cast to `void *`, wherever a pointer
// takes one: an initializer, an assignment, a comparison, `?:`, an argument
// and a static initializer.
#[test]
fn any_null_pointer_constant_makes_a_null_pointer() {
    returned(
        "int f(int x) { return x + 1; }\nint main(void) {\n    int (*p)(int) = (1 - 1);\n    int \
         (*q)(int) = (void *)0;\n    int *r = (void *)0;\n    int *s = 0L;\n    int n = 0;\n    if \
         (p == (void *)0) n += 1;\n    if (q != (1 - 1)) n += 10;\n    p = (1 - 1);\n    p = \
         '\\0';\n    q = (void *)(2 - 2);\n    if (r == (void *)0) n += 100;\n    if (s != (1 - \
         1)) n += 1000;\n    p = f;\n    if (p != (void *)0) n += 2;\n    if (q == 0) n += 20;\n    \
         return n;\n}\n",
        123,
    );
    returned(
        "int f(int x) { return x + 1; }\nstatic int (*sp)(int) = (1 - 1);\nstatic int (*sv)(int) \
         = (void *)0;\nstatic int *so = (void *)0;\nstatic int (*tab[3])(int) = { f, (void *)0, \
         (2 * 0) };\nint main(void) {\n    int n = 0;\n    if (sp == 0) n += 1;\n    if (sv == 0) \
         n += 2;\n    if (so == 0) n += 4;\n    if (tab[1] == 0 && tab[2] == 0) n += 8;\n    \
         return n + tab[0](1);\n}\n",
        17,
    );
    returned(
        "int twice(int x) { return 2 * x; }\nint apply(int (*g)(int), int v) {\n    if (g == (void \
         *)0)\n        return -1;\n    return g(v);\n}\nint main(void) {\n    int c = 0;\n    int \
         (*p)(int) = c ? twice : (1 - 1);\n    int (*q)(int) = c ? (void *)0 : twice;\n    return \
         apply(p, 3) + apply(q, 4) + apply((void *)0, 5) + apply((0), 6);\n}\n",
        5,
    );
    returned(
        "int main(void) { int x = 3; int *q = &x; int n = 0; if (q != (1 - 1)) n += 1; q = (1 - \
         1); if (q == (void *)0) n += 2; if (!q) n += 4; return n + (q ? 8 : 0); }",
        7,
    );
    // A floating constant may be a cast's operand in an integer constant
    // expression.
    returned(
        "int f(int x) { return x; } int main(void) { int (*p)(int) = (int)0.5; return p == 0; }",
        1,
    );
    let states = stepped("int main(void) { int (*p)(int) = (void *)0; return p == 0; }");
    let cast = step(&states, EventKind::Cast, "Cast to void *: a null pointer");
    assert!(
        cast.event
            .summary
            .contains("converts the null pointer constant 0 to void *"),
        "{}",
        cast.event.summary
    );
    assert_eq!(cast.event.clauses[1].id, "6.3.2.3p3");
    for (source, needle) in [
        (
            "int main(void) { int (*p)(int) = (void *)1; return p == 0; }",
            "only as the null pointer constant `(void *)0`",
        ),
        (
            "int main(void) { int *p = 0; return p == (void *)0 + 1; }",
            "`(void *)0` is in this subset only as a null pointer constant",
        ),
        // Floating arithmetic is no integer constant expression.
        (
            "int main(void) { int (*p)(int) = (int)(0.5 + 0.1); return p == 0; }",
            "this is an int, where an int (*)(int) is needed",
        ),
        (
            "int f(int x) { return x; } int main(void) { int x = 1; int (*p)(int) = f; return p \
             == x - 1; }",
            "or with a null pointer constant such as 0",
        ),
    ] {
        let message = rejected(source);
        assert!(message.contains(needle), "{source}: {message}");
    }
}

// C99 §6.3.2.3p3, §6.5.3.2p4, §6.5.2.2p3, §4p2: a pointer that points to
// a function never equals a null pointer, though a pointer to a function
// may be one; `*` of a null pointer to a function, and a call through one,
// stop for what C leaves undefined.
#[test]
fn a_null_pointer_to_a_function_is_told_apart_from_one_to_a_function() {
    let states = finished(
        "int f(int x) { return x; }\nint main(void) {\n    int (*a)(int) = f;\n    int (*b)(int) \
         = 0;\n    int n = (a == b) + 2 * (a != b);\n    if (a) n += 4;\n    return n;\n}\n",
        6,
    );
    let compare = step(
        &states,
        EventKind::Comparison,
        "Compare pointers to functions: a == b",
    );
    assert!(
        compare
            .event
            .explanation
            .contains("A pointer that points to a function never compares equal to a null pointer")
            && !compare.event.explanation.contains("never null"),
        "{}",
        compare.event.explanation
    );
    for state in &states {
        assert!(
            !state.event.explanation.contains("is never null"),
            "{}: {}",
            state.event.title,
            state.event.explanation
        );
    }
    let (cause, title) = undefined("int main(void) { int (*f)(int) = 0; return (*f)(1); }");
    assert_eq!(cause, Some(super::Undefined::NullFunctionIndirection));
    assert_eq!(
        title,
        "Undefined behavior: * of a null pointer to a function"
    );
    let session = run("int main(void) { int (*f)(int) = 0; return f(1); }");
    let diagnostic = &session.state().diagnostics[0];
    assert_eq!(diagnostic.clause.map(|clause| clause.id), Some("4p2"));
    assert!(
        diagnostic.message.contains("(§6.5.2.2p3)") && diagnostic.message.contains("(§4p2)"),
        "{}",
        diagnostic.message
    );
}

// C99 §6.6p2-4, p8, §6.7.8p4: a static initializer may be a constant
// expression, worked out before the program starts; what needs the program
// to run, or has no value, is refused.
#[test]
fn static_initializers_take_constant_expressions() {
    let states = finished(
        "static int mask = 1 << 3;\nint table[] = { 1+1, 2*2 };\nint main(void) { return mask + \
         table[0] + table[1]; }\n",
        14,
    );
    let mask = step(&states, EventKind::StaticStorage, "Before main: `mask` = 8");
    assert!(
        mask.event.explanation.contains(
            "Its initializer `1 << 3` is a constant expression: the compiler works it out while \
             it translates the program"
        ) && mask
            .event
            .explanation
            .contains("The value is fixed before the program starts"),
        "{}",
        mask.event.explanation
    );
    assert!(mask.event.clauses.iter().any(|clause| clause.id == "6.6p2"));
    let table = step(
        &states,
        EventKind::StaticStorage,
        "Before main: `table` = {2, 4}",
    );
    assert!(
        table
            .event
            .explanation
            .contains("Its initializers `1+1` and `2*2` are constant expressions")
    );
    // A constant as written is narrated as before.
    let plain = stepped("int n = -5; int main(void) { return n + 5; }");
    let plain = step(&plain, EventKind::StaticStorage, "Before main: `n` = -5");
    assert!(!plain.event.explanation.contains("constant expression:"));
    returned(
        "double third = 1.0 / 3; unsigned u = 0u - 1; char c = 'a' + 2; int t = (int)3.99 + \
         (sizeof(int) > 2 ? 10 : 20); int main(void) { return (third > 0.33) + (u == 4294967295u) \
         + c + t; }",
        114,
    );
    returned(
        "int next(void) { static int count = 10 * 10; return ++count; } int main(void) { next(); \
         return next(); }",
        102,
    );
    for (source, needle) in [
        (
            "int f(void) { return 1; } int x = f(); int main(void) { return x; }",
            "a function call, which only the running program makes",
        ),
        (
            "const int a = 3; int b = a; int main(void) { return b; }",
            "C99's constant expressions read no object, not even a const one",
        ),
        (
            "int big = 2147483647 + 1; int main(void) { return 0; }",
            "a signed overflow",
        ),
        (
            "int z = 1 / 0; int main(void) { return 0; }",
            "it divides by zero",
        ),
        (
            "int s = 1 << 40; int main(void) { return 0; }",
            "it shifts an int by 40",
        ),
        (
            "int c = (1, 2); int main(void) { return c; }",
            "the comma operator",
        ),
    ] {
        let message = rejected(source);
        assert!(message.contains(needle), "{source}: {message}");
    }
}

// C99 §6.5p3: the open-order stop names the object where the call's access
// and the other operand's meet, a member when the call stores a whole
// structure.
#[test]
fn an_open_order_names_where_the_accesses_meet() {
    let message = |source: &str| {
        let session = run(source);
        assert_eq!(session.state().status, "error", "{source}");
        session.state().diagnostics[0].message.clone()
    };
    let prelude = "struct S { int a; int b; }; struct S s; struct S t; struct S u; int f(void) { \
                   s = t; return 1; } int g(void) { s = u; return 2; } ";
    assert!(
        message(&format!(
            "{prelude}int main(void) {{ int r = s.b + f(); return r; }}"
        ))
        .starts_with("the call `f()` stores into `s.b`, and another operand, `s.b`, reads it")
    );
    assert!(
        message(&format!(
            "{prelude}int main(void) {{ struct S *ps = &s; ps->b *= f(); return s.b; }}"
        ))
        .starts_with("the call `f()` stores into `s.b`, and `ps->b *= f()` reads it")
    );
    assert!(
        message(&format!("{prelude}int main(void) {{ return f() + g(); }}"))
            .starts_with("the call `g()` stores into `s`, and the call `f()` stores into it too")
    );
}
