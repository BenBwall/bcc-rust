//! Enumerations, `switch` and `break`, typedefs of every type, pointers to
//! functions in structures and as return values, and the verb of `*`.

use super::{
    EventKind,
    Operand,
    State,
    Value,
    compile,
    finished,
    run,
    step,
};

/// The last state of `source`'s run, which must stop with `status`.
fn stopped(source: &str, status: &str) -> State {
    let session = run(source);
    assert_eq!(
        session.state().status,
        status,
        "{source}: {:?}",
        session.state().diagnostics
    );
    session.state().clone()
}

/// The diagnostic `source` is refused with, which must be of `kind`.
fn refused(source: &str, kind: &str) -> String {
    let diagnostic = compile(source).expect_err(source);
    assert_eq!(diagnostic.kind, kind, "{source}: {}", diagnostic.message);
    diagnostic.message
}

// C99 §6.4.4.3p2, §6.7.2.2p3: an enumeration declares int constants, which
// are no objects; its declaration is a step before main that creates
// nothing.
#[test]
fn an_enumeration_declares_constants_that_take_no_storage() {
    let states = finished(
        "enum color { RED, GREEN = 5, BLUE };\nint main(void) {\n    enum color c = BLUE;\n    \
         return c - GREEN;\n}\n",
        1,
    );
    let declared = step(
        &states,
        EventKind::Enumeration,
        "Declare the constants of `enum color`",
    );
    assert_eq!(declared.step, 1, "the declaration comes before main");
    assert_eq!(declared.objects.len(), 0, "no object is created");
    assert_eq!(declared.event.objects, Vec::<usize>::new());
    assert!(
        declared
            .event
            .summary
            .contains("three constants, RED = 0, GREEN = 5 and BLUE = 6"),
        "{}",
        declared.event.summary
    );
    for words in [
        "RED is the first constant and has no =, so it is 0",
        "GREEN = 5 gives it the value 5",
        "BLUE has no =, so it is the previous constant, GREEN, plus 1: 6",
        "The constants are not objects",
        "compatible with unsigned int",
    ] {
        assert!(
            declared.event.explanation.contains(words),
            "{words}: {}",
            declared.event.explanation
        );
    }
    assert_eq!(declared.event.clauses[0].id, "6.7.2.2p3");
    let constant = step(
        &states,
        EventKind::Constant,
        "Evaluate the enumeration constant BLUE",
    );
    assert_eq!(
        constant.event.result,
        Some(Operand::Value(Value::Integer(6)))
    );
    assert!(
        constant
            .event
            .summary
            .contains("stands for the int value 6, so nothing is read from memory"),
        "{}",
        constant.event.summary
    );
    assert!(
        constant
            .event
            .explanation
            .contains("even though enum color itself is compatible with unsigned int")
    );
    let last = states.last().expect("a final state");
    let c = last
        .objects
        .iter()
        .find(|object| object.name == "c")
        .expect("c");
    assert_eq!(c.r#type, "enum color");
    assert_eq!(c.enumeration, Some(0));
    let declare = step(&states, EventKind::Declare, "Declare `c` (enum color)");
    assert!(
        declare
            .event
            .explanation
            .contains("Its type, enum color, is an enumeration"),
        "{}",
        declare.event.explanation
    );
}

// C99 §6.7.2.2p4: GCC makes an enumeration without negative constants
// compatible with unsigned int, and one with a negative constant with int.
#[test]
fn an_enumerations_objects_compute_in_its_compatible_type() {
    let _compared = finished(
        "enum e { A, B }; enum n { M = -1, N }; int main(void) { enum e x = A; enum n y = N; \
         return (x - 1 > 0) * 10 + (y - 1 > 0); }",
        10,
    );
    // Constants in constant expressions: array sizes, static initializers,
    // other constants, and a block-scope enumeration.
    let states = finished(
        "enum { SIZE = 2 * 2, LAST = SIZE - 1 };\nstatic int last = LAST;\nint main(void) {\n    \
         int v[SIZE];\n    enum { ONE = 1 };\n    v[LAST] = ONE;\n    return v[last] + (int) \
         sizeof v;\n}\n",
        17,
    );
    let block = step(&states, EventKind::Enumeration, "Declare the constants ONE");
    assert!(
        block
            .event
            .explanation
            .contains("Declared in a block, the constants can be used from here")
    );
}

// C99 §6.8.4.2p5, §6.8.6.3p2: a switch jumps to the matching case label,
// to default, or past its body; execution falls through later labels until
// a break.
#[test]
fn a_switch_jumps_to_its_label_and_break_leaves_it() {
    let source = |value: &str| {
        format!(
            "enum shape {{ CIRCLE, SQUARE, TRIANGLE }};\nint sides(enum shape s) {{\n    int n = \
             0;\n    switch (s) {{\n    case CIRCLE:\n        n = 0;\n        break;\n    case \
             SQUARE:\n        n = 4;\n        break;\n    default:\n        n = 3;\n    }}\n    \
             return n;\n}}\nint main(void) {{ return sides({value}); }}\n"
        )
    };
    let states = finished(&source("SQUARE"), 4);
    let switched = step(
        &states,
        EventKind::Branch,
        "switch on 1: jump to `case SQUARE`",
    );
    assert!(switched.event.sequence_point);
    assert!(
        switched
            .event
            .explanation
            .contains("runs on through the statements that follow, past any later labels"),
        "{}",
        switched.event.explanation
    );
    let left = step(&states, EventKind::Jump, "break: leave the switch");
    assert_eq!(left.event.clauses[0].id, "6.8.6.3p2");
    let states = finished(&source("TRIANGLE"), 3);
    let _default = step(
        &states,
        EventKind::Branch,
        "switch on 2: no case matches, jump to `default`",
    );
    // Without a default nothing runs; without a break execution falls
    // through; a char is promoted first.
    let states = finished(
        "int main(void) { int n = 0; switch (7) { case 1: n = 1; } char c = 2; switch (c) { case \
         2: n += 1; case 3: n += 10; } return n; }",
        11,
    );
    let _skip = step(
        &states,
        EventKind::Branch,
        "switch on 7: no case matches, skip the body",
    );
}

// C99 §6.8.6.3p2, §6.2.4p5: a break leaves a loop, ending the lifetimes of
// the blocks it leaves on the way.
#[test]
fn break_leaves_a_loop_and_the_blocks_inside_it() {
    let states = finished(
        "int main(void) {\n    int found = -1;\n    for (int i = 0; i < 5; i++) {\n        int \
         twice = 2 * i;\n        if (twice > 4) {\n            found = i;\n            break;\n   \
         }\n    }\n    while (1) {\n        break;\n    }\n    return found;\n}\n",
        3,
    );
    let left = step(&states, EventKind::Jump, "break: leave the loop");
    assert!(left.event.summary.contains("the for loop"));
    let at = states
        .iter()
        .position(|state| std::ptr::eq(state, left))
        .expect("the break's step");
    let ended: Vec<&str> = states[..at]
        .iter()
        .rev()
        .take_while(|state| state.event.kind == EventKind::LeaveBlock)
        .chain(states[..at].iter().rev().skip(2).take(1))
        .flat_map(|state| state.event.title.split('`').nth(1))
        .collect();
    assert!(
        states[at - 1].event.kind == EventKind::LeaveBlock
            && states[at - 2].event.kind == EventKind::LeaveBlock,
        "the if's block and the loop body end before the jump: {ended:?}"
    );
    assert!(
        states[at]
            .objects
            .iter()
            .any(|object| object.name == "twice" && object.lifetime == "ended")
    );
}

// C99 §6.8.4.2p1, p3, §6.8.1p2, §6.8.6.3p1, §6.7.2.2p2, §6.7.2.3p1-3,
// §6.7p3: what C refuses, with the rule each breaks.
#[test]
fn enumerations_and_switches_refuse_what_c_forbids() {
    for (source, words) in [
        (
            "int main(void) { switch (1) { case 1: case 1: break; } return 0; }",
            "no two case labels of one switch may have the same value",
        ),
        (
            "int main(void) { switch (1) { default: break; default: break; } return 0; }",
            "at most one default label",
        ),
        (
            "int main(void) { int x = 1; switch (1) { case x: break; } return 0; }",
            "a case label must be an integer constant expression",
        ),
        (
            "int main(void) { double d = 1; switch (d) { case 1: break; } return 0; }",
            "must have an integer type",
        ),
        (
            "int main(void) { case 1: return 0; }",
            "belongs inside a switch statement",
        ),
        (
            "int main(void) { break; return 0; }",
            "belongs inside a switch or a loop",
        ),
        (
            "enum e { A = 2147483647, B }; int main(void) { return 0; }",
            "which an int cannot hold",
        ),
        (
            "enum e { A = 3000000000 }; int main(void) { return 0; }",
            "which an int cannot hold",
        ),
        (
            "enum e { A }; enum e { B }; int main(void) { return 0; }",
            "already defined in this scope",
        ),
        (
            "enum e { A, A }; int main(void) { return 0; }",
            "already declared in this scope",
        ),
        (
            "int A; enum e { A }; int main(void) { return 0; }",
            "already declared in this scope",
        ),
        (
            "enum e { A }; int A; int main(void) { return 0; }",
            "already an enumeration constant in this scope",
        ),
        (
            "int main(void) { enum f x; return 0; }",
            "no enumeration tagged f is defined",
        ),
        (
            "struct s { int x; }; enum s { A }; int main(void) { return 0; }",
            "already the tag of a structure",
        ),
        (
            "enum s { A }; int main(void) { struct s *p; return 0; }",
            "the tag of an enumeration",
        ),
        (
            "int main(void) { int n = 3; int v[n]; return 0; }",
            "variable length array",
        ),
        (
            "int main(void) { int v[2 - 2]; return 0; }",
            "must be greater than zero",
        ),
        (
            "enum e { A }; int main(void) { return A(); }",
            "only a function or a pointer to a function can be called",
        ),
    ] {
        let diagnostic = compile(source).expect_err(source);
        assert!(
            diagnostic.message.contains(words),
            "{source}: {}",
            diagnostic.message
        );
        if !source.contains("v[n]") {
            assert_eq!(diagnostic.kind, "constraint", "{source}");
        }
    }
    for source in [
        "int main(void) { switch (1) { int x; case 1: break; } return 0; }",
        "int main(void) { switch (1) case 1: return 0; }",
        "enum e { A }; int main(void) { int A = 1; return A; }",
    ] {
        let diagnostic = compile(source).expect_err(source);
        assert_ne!(
            diagnostic.kind, "constraint",
            "{source}: {}",
            diagnostic.message
        );
    }
}

// C99 §6.7.7p3: a typedef names a type and creates no object, at file and
// block scope, for every kind of type.
#[test]
fn a_typedef_names_any_type_and_creates_no_object() {
    let states = finished(
        "typedef double celsius;\ntypedef char *string;\ntypedef int row[3];\ntypedef struct { \
         int x; int y; } point;\ntypedef struct node node;\nstruct node { int value; node *next; \
         };\ntypedef enum { OFF, ON } state;\nint main(void) {\n    celsius t = 21.5;\n    string \
         s = \"Ada\";\n    row r = {1, 2, 3};\n    point p = {3, 4};\n    node n = {7, 0};\n    \
         state lamp = ON;\n    typedef int count;\n    count k = r[2];\n    return (int)t + s[1] \
         + p.y + n.value + lamp + k;\n}\n",
        21 + 100 + 4 + 7 + 1 + 3,
    );
    let declared = step(&states, EventKind::Declare, "Declare `t` (double)");
    assert!(
        declared.event.explanation.contains(
            "spells the type with the typedef name `celsius`, which line 1 declared as a synonym \
             for double: a typedef names a type and creates no object"
        ),
        "{}",
        declared.event.explanation
    );
    let last = states.last().expect("a final state");
    let ty = |name: &str| {
        last.objects
            .iter()
            .find(|object| object.name == name)
            .map(|object| object.r#type.clone())
            .unwrap_or_default()
    };
    assert_eq!(ty("s"), "char *");
    assert_eq!(ty("r"), "int[3]");
    assert_eq!(ty("p"), "point");
    assert_eq!(ty("n"), "struct node");
    assert_eq!(ty("lamp"), "state");
    let enumeration = step(
        &states,
        EventKind::Enumeration,
        "Declare the constants of `state`",
    );
    assert!(
        enumeration
            .event
            .explanation
            .contains("makes `state` a typedef name for the enumeration")
    );
    // A block's typedef ends with the block; a typedef cannot be const or
    // shadowed.
    let _scoped = refused(
        "int main(void) { { typedef int count; } count k = 0; return k; }",
        "unsupported",
    );
    let _twice = refused(
        "typedef int a; typedef int a; int main(void) { return 0; }",
        "constraint",
    );
    let _pointer_to_array = refused(
        "typedef int row[3]; int main(void) { row *p; return 0; }",
        "unsupported",
    );
}

// C99 §6.5.2.3p3-4, §6.5.2.2p3, §6.7.5.3p1: a structure member that points
// to a function is called through `.` and `->`, and a function may return
// a pointer to a function, which a call then follows.
#[test]
fn pointers_to_functions_live_in_structures_and_return_from_calls() {
    let states = finished(
        "int twice(int x) { return 2 * x; }\nint square(int x) { return x * x; }\nstruct ops { int \
         (*apply)(int); int count; };\nint (*pick(int up))(int) {\n    if (up) {\n        return \
         square;\n    }\n    return twice;\n}\nint main(void) {\n    struct ops s = {twice, \
         1};\n    struct ops *p = &s;\n    int a = s.apply(3);\n    p->apply = pick(1);\n    \
         return a + p->apply(3) + pick(0)(5);\n}\n",
        6 + 9 + 10,
    );
    let call = step(&states, EventKind::Call, "Call twice through `s.apply`");
    assert!(
        call.event
            .summary
            .starts_with("`s.apply` holds a pointer to twice"),
        "{}",
        call.event.summary
    );
    let _arrow = step(&states, EventKind::Call, "Call square through `p->apply`");
    let returned = step(&states, EventKind::Call, "Call twice through `pick(0)`");
    assert!(
        returned
            .event
            .summary
            .starts_with("`pick(0)` yields a pointer to twice"),
        "{}",
        returned.event.summary
    );
    let last = states.last().expect("a final state");
    let s = last
        .objects
        .iter()
        .find(|object| object.name == "s")
        .expect("s");
    assert!(matches!(
        &s.value,
        Value::Struct { members, .. } if members[0] == Value::Function(1)
    ));
    // A call through a null member stops.
    let stop = stopped(
        "struct handler { int (*on)(int); };\nint main(void) { struct handler h = {0}; return \
         h.on(1); }\n",
        "undefined_behavior",
    );
    assert!(
        stop.diagnostics[0].message.contains("null pointer"),
        "{}",
        stop.diagnostics[0].message
    );
}

// C99 §6.5.3.2p4: `*` follows a pointer an object holds, or one any other
// expression yields.
#[test]
fn indirection_says_whether_its_operand_holds_or_yields_the_pointer() {
    let states = finished(
        "int main(void) { int t[2] = {4, 5}; int *p = t; int **pp = &p; return *(t + 1) + *p + \
         **pp; }",
        13,
    );
    let yielded = step(&states, EventKind::Dereference, "`*(t + 1)` designates");
    assert!(
        yielded
            .event
            .explanation
            .starts_with("`(t + 1)` yields a pointer"),
        "{}",
        yielded.event.explanation
    );
    let held = step(&states, EventKind::Dereference, "`*p` designates");
    assert!(held.event.explanation.starts_with("`p` holds a pointer"));
    let inner = step(&states, EventKind::Dereference, "`**pp` designates");
    assert!(
        inner.event.explanation.starts_with("`*pp` holds a pointer"),
        "{}",
        inner.event.explanation
    );
}

// C99 §6.7.5.3p7-8: a parameter of array type is adjusted to a pointer,
// whether `[N]` follows its name or a typedef name gives it the array type.
// The subset asks for the pointer as written, and a typedef cannot name a
// function type, so neither reaches a call unadjusted.
#[test]
fn a_typedef_of_an_array_is_no_parameter_type() {
    for source in [
        "typedef int row[3];\nint first(row r) { return r[0]; }\nint main(void) { int (*fp)(int \
         *) = first; int t[3] = { 7, 8, 9 }; return fp(t); }\n",
        "typedef int row[3];\nint first(row r) { return r[0]; }\nint main(void) { int t[3] = { 7, \
         8, 9 }; return first(t); }\n",
        "typedef int row[3];\nint size(row r) { return sizeof r; }\nint main(void) { return 0; }\n",
        "typedef int row[3];\nint first(row);\nint main(void) { return 0; }\n",
        "typedef char *names[2];\nint count(names n) { return 0; }\nint main(void) { return 0; }\n",
        "typedef int row[3];\nint main(void) { int (*fp)(row); return 0; }\n",
        "int first(int r[3]) { return r[0]; }\nint main(void) { return 0; }\n",
    ] {
        let message = refused(source, "unsupported");
        assert!(
            message.starts_with("array parameters are outside this subset"),
            "{source}: {message}"
        );
    }
    let message = refused(
        "typedef int fn(int);\nint apply(fn f, int x) { return f(x); }\nint main(void) { return \
         0; }\n",
        "unsupported",
    );
    assert!(message.contains("typedef of a function type"), "{message}");
    // The pointer the parameter adjusts to runs.
    let _adjusted = finished(
        "typedef int row[3];\nint first(int *r) { return r[0]; }\nint main(void) { row t = { 7, \
         8, 9 }; return first(t); }\n",
        7,
    );
}

// C99 §6.8.6.3p2, §6.8.1p4: a break says what it skips only when something
// follows it in the switch's body; falling through to a label is a step of
// its own, and the switch's jump to a label is not.
#[test]
fn break_and_fall_through_say_what_the_body_holds() {
    let explanation = |source: &str, expected: i128| {
        let states = finished(source, expected);
        step(&states, EventKind::Jump, "break: leave the switch")
            .event
            .explanation
            .clone()
    };
    let last = explanation(
        "int main(void) { int x = 3; int r = 0; switch (x) { case 1: r = 1; break; default: r = \
         2; break; } return r; }",
        2,
    );
    assert!(
        last.contains("Nothing in the body follows the break, so it skips nothing"),
        "{last}"
    );
    assert!(
        !last.contains("later") && !last.contains("next label"),
        "{last}"
    );
    let nested = explanation(
        "int main(void) { int r = 0; switch (2) { case 2: { int y = 5; r = y; break; } } return \
         r; }",
        5,
    );
    assert!(nested.contains("skips nothing"), "{nested}");
    let branches = explanation(
        "int main(void) { int r = 0; switch (1) { case 1: if (r == 0) { r = 1; break; } else { r \
         = 2; break; } } return r; }",
        1,
    );
    assert!(branches.contains("skips nothing"), "{branches}");
    let one = explanation(
        "int main(void) { int r = 0; switch (1) { case 1: r = 1; break; case 2: r = 2; } return \
         r; }",
        1,
    );
    assert!(
        one.contains("including those under the later label, do not run")
            && one.contains("run on past that label"),
        "{one}"
    );
    let two = explanation(
        "int main(void) { int r = 0; switch (1) { case 1: r = 1; break; case 2: case 3: r = 2; } \
         return r; }",
        1,
    );
    assert!(two.contains("under the later labels"), "{two}");
    let statements = explanation(
        "int main(void) { int r = 0; switch (4) { default: if (r == 0) break; r = 9; } return r; }",
        0,
    );
    assert!(
        statements.contains("the statements after the break, to the end of the body, do not run"),
        "{statements}"
    );
    // A loop's break inside a switch leaves the loop alone.
    let states = finished(
        "int main(void) { int r = 0; switch (1) { case 1: while (1) { r = 1; break; } r += 1; } \
         return r; }",
        2,
    );
    let _loop = step(&states, EventKind::Jump, "break: leave the loop");
    // Falling through: one step at the run of labels it passes, none where
    // the switch jumps to a label.
    let states = finished(
        "int main(void) {\n    int r = 0;\n    switch (1) {\n    case 1:\n        r = 1;\n    \
         case 2:\n    case ':':\n        r += 10;\n    default:\n        r += 100;\n    }\n    \
         return r;\n}\n",
        111,
    );
    let past: Vec<&str> = states
        .iter()
        .filter(|state| state.event.title.starts_with("Run on past"))
        .map(|state| state.event.title.as_str())
        .collect();
    assert_eq!(
        past,
        ["Run on past `case 2: case ':':`", "Run on past `default:`"]
    );
    let first = step(&states, EventKind::Jump, "Run on past `case 2: case ':':`");
    assert!(
        first
            .event
            .summary
            .contains("runs on past the labels into the statements after them"),
        "{}",
        first.event.summary
    );
    assert_eq!(first.event.clauses[0].id, "6.8.1p4");
    let second = step(&states, EventKind::Jump, "Run on past `default:`");
    assert!(
        second
            .event
            .summary
            .contains("past the label into the statements after it"),
        "{}",
        second.event.summary
    );
    let states = finished(
        "int main(void) { int r = 0; switch (2) { case 1: r = 1; case 2: r += 2; } return r; }",
        2,
    );
    assert!(
        states
            .iter()
            .all(|state| !state.event.title.starts_with("Run on past")),
        "the switch's jump to `case 2:` passes no label"
    );
}

// C99 §6.7.2.2p4: an enumerated type promises only the range of its
// constants, whatever its compatible type holds.
#[test]
fn an_enumerations_object_promises_only_its_constants_range() {
    let declared = |source: &str, title: &str| {
        let states = finished(source, 0);
        step(&states, EventKind::Declare, title)
            .event
            .explanation
            .clone()
    };
    let color = declared(
        "enum color { RED, GREEN = 5, BLUE };\nint main(void) {\n    enum color c = BLUE;\n    \
         return c - BLUE;\n}\n",
        "Declare `c` (enum color)",
    );
    assert!(
        color.contains(
            "an enum color object (4 bytes on x86-64, 0 to 4294967295; C promises only that its \
             type can represent its constants' values, so at least 0 to 6)"
        ),
        "{color}"
    );
    assert!(!color.contains("65535"), "{color}");
    assert!(
        color.contains(
            "Its type, enum color, is an enumeration, compatible with unsigned int: C lets each \
             implementation choose"
        ),
        "{color}"
    );
    let signed = declared(
        "enum level { LOW = -3, HIGH = 11 };\nint main(void) {\n    enum level l = LOW;\n    \
         return l + 3;\n}\n",
        "Declare `l` (enum level)",
    );
    assert!(
        signed.contains("-2147483648 to 2147483647; C promises only"),
        "{signed}"
    );
    assert!(signed.contains("values, so at least -3 to 11"), "{signed}");
    assert!(
        signed.contains("GCC chooses int when a constant is negative, as one is here"),
        "{signed}"
    );
    let elements = declared(
        "enum color { RED, GREEN };\nint main(void) {\n    enum color a[2] = { GREEN, RED };\n    \
         return a[1];\n}\n",
        "Declare `a`",
    );
    assert!(
        elements.contains(
            "Its element type, enum color, is an enumeration, compatible with unsigned int"
        ) && elements.contains("So each element may hold any value of that type"),
        "{elements}"
    );
    let single = declared(
        "enum one { ONLY = 4 };\nint main(void) {\n    enum one o = ONLY;\n    return o - 4;\n}\n",
        "Declare `o` (enum one)",
    );
    assert!(single.contains("its constant's value, 4)"), "{single}");
    // A parameter of an enumerated type is named as declared.
    let states = finished(
        "enum color { RED, GREEN };\nint is_green(enum color c) { return c == GREEN; }\nint \
         main(void) { return is_green(-1); }\n",
        0,
    );
    let call = states
        .iter()
        .find(|state| state.event.kind == EventKind::Call)
        .expect("the call");
    assert!(
        call.event.explanation.contains(
            "The argument for `c` converts to the parameter's type, enum color (compatible with \
             unsigned int)"
        ),
        "{}",
        call.event.explanation
    );
}
