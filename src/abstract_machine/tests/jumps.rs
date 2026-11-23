//! `do`, `continue`, `goto` and labels, case labels nested in a switch's
//! body, names that hide others, and functions that return pointers to
//! objects. Each run checks every step's "what comes next" against the step
//! that follows it.

use super::{
    EventKind,
    State,
    Value,
    compile,
    finished,
    run,
    step,
    stepped,
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

/// The index of the first step of `kind` whose title starts with `title`.
fn at(states: &[State], kind: EventKind, title: &str) -> usize {
    states
        .iter()
        .position(|state| state.event.kind == kind && state.event.title.starts_with(title))
        .unwrap_or_else(|| panic!("no {kind:?} step titled {title:?}"))
}

// C99 §6.8.5.2p1: a do statement tests its condition after each run of its
// body, so the body runs before any test.
#[test]
fn a_do_statement_runs_its_body_before_its_first_test() {
    let states = finished(
        "int main(void) {\n    int runs = 0;\n    do runs++; while (0);\n    return runs;\n}\n",
        1,
    );
    let start = at(&states, EventKind::Jump, "do: run the body before any test");
    assert!(
        states[start]
            .event
            .summary
            .contains("tests `0` only after it, so the body runs at least once"),
        "{}",
        states[start].event.summary
    );
    assert_eq!(states[start].event.clauses[0].id, "6.8.5.2p1");
    let test = at(
        &states,
        EventKind::Branch,
        "Condition is false: leave the loop",
    );
    assert!(start < test, "the body runs before the test");
    assert!(
        states[start..test]
            .iter()
            .any(|state| state.event.kind == EventKind::Increment),
        "the body ran"
    );
    assert_eq!(states[test].event.clauses[0].id, "6.8.5.2p1");
    let states =
        finished(
            "int main(void) {\n    int n = 0;\n    do {\n        n = n + 2;\n    } while (n < \
             7);\n    return n;\n}\n",
            8,
        );
    let again = step(
        &states,
        EventKind::Branch,
        "Condition is true: run the body again",
    );
    assert!(again.event.summary.ends_with("so the body runs again."));
    let _locals =
        finished(
            "int main(void) {\n    int i = 0;\n    int s = 0;\n    do {\n        int sq = i * \
             i;\n        s += sq;\n        i++;\n    } while (i < 4);\n    return s;\n}\n",
            14,
        );
    let _left =
        finished(
            "int main(void) {\n    int i = 10;\n    do {\n        if (i == 7) break;\n        \
             i--;\n    } while (i > 0);\n    return i;\n}\n",
            7,
        );
}

// C99 §6.8.6.2p2: a continue jumps to the end of the loop's body: a while's
// and a do's test come next, a for's third clause.
#[test]
fn continue_goes_on_to_each_loops_next_test() {
    for (source, expected, then, next) in [
        (
            "int main(void) {\n    int i = 0;\n    int s = 0;\n    while (i < 6) {\n        \
             i++;\n        if (i % 2) continue;\n        s += i;\n    }\n    return s;\n}\n",
            12,
            "goes straight to the condition `i < 6`",
            "`i` names an object to read",
        ),
        (
            "int main(void) {\n    int s = 0;\n    for (int i = 0; i < 6; i++) {\n        int \
             twice = 2 * i;\n        if (i == 3) continue;\n        s += twice;\n    }\n    \
             return s;\n}\n",
            24,
            "goes to the for statement's third clause, `i++`, and then to the condition",
            "`i` is the object to update",
        ),
        (
            "int main(void) {\n    int i = 0;\n    int s = 0;\n    do {\n        i++;\n        if \
             (i == 2) continue;\n        s += i;\n    } while (i < 4);\n    return s;\n}\n",
            8,
            "goes to the test `i < 4` after the body",
            "`i` names an object to read",
        ),
    ] {
        let states = finished(source, expected);
        let jump = at(&states, EventKind::Jump, "continue: end this pass");
        assert!(
            states[jump].event.summary.contains(then),
            "{}",
            states[jump].event.summary
        );
        assert_eq!(states[jump].event.clauses[0].id, "6.8.6.2p2");
        assert_eq!(states[jump + 1].event.title, next, "{source}");
        // The body's block ends first, said of the continue.
        let leave = &states[jump - 1];
        assert_eq!(leave.event.kind, EventKind::LeaveBlock);
        assert!(
            leave
                .event
                .summary
                .starts_with("`continue` leaves the block"),
            "{}",
            leave.event.summary
        );
    }
    let states = finished(
        "int main(void) {\n    int s = 0;\n    for (int i = 0; i < 6; i++) {\n        int twice = \
         2 * i;\n        if (i == 3) continue;\n        s += twice;\n    }\n    return s;\n}\n",
        24,
    );
    assert!(
        states
            .iter()
            .any(|state| state.event.kind == EventKind::LeaveBlock
                && state.event.title == "End the lifetime of `twice`"
                && state
                    .event
                    .summary
                    .starts_with("`continue` leaves the block, so `twice`")),
        "the continue ends the body's object"
    );
    let _nested = finished(
        "int main(void) {\n    int s = 0;\n    for (int i = 0; i < 3; i++) {\n        { int k = \
         i; if (k == 1) continue; }\n        for (int j = 0; j < 3; j++) {\n            if (j == \
         i) continue;\n            s += 10 * i + j;\n        }\n    }\n    return s;\n}\n",
        44,
    );
    let _omitted =
        finished(
            "int main(void) {\n    int i = 0;\n    int s = 0;\n    for (; i < 5;) {\n        \
             i++;\n        if (i == 3) continue;\n        s += i;\n    }\n    for (;;) { s++; if \
             (s < 20) continue; break; }\n    return s;\n}\n",
            20,
        );
    let _switch = finished(
        "int main(void) {\n    int s = 0;\n    for (int i = 0; i < 4; i++) {\n        switch (i) \
         {\n        case 1: continue;\n        case 2: s += 100; break;\n        default: s += \
         i;\n        }\n        s += 1;\n    }\n    return s;\n}\n",
        106,
    );
    let diagnostic = compile("int main(void) { continue; }").expect_err("outside a loop");
    assert_eq!(diagnostic.kind, "constraint");
    assert!(diagnostic.message.contains("C99 §6.8.6.2p1"));
    let diagnostic = compile("int main(void) { switch (1) { case 1: continue; } return 0; }")
        .expect_err("a switch is no loop");
    assert_eq!(diagnostic.kind, "constraint");
}

// C99 §6.8.6.1p2, §6.2.4p5: a goto leaves the blocks its label is not in,
// one step each, ending their objects' lifetimes, and jumps back or ahead.
#[test]
fn a_goto_leaves_the_blocks_its_label_is_not_in() {
    let states = finished(
        "int main(void) {\n    int n = 0;\nagain:\n    n++;\n    if (n < 3) goto again;\n    \
         return n;\n}\n",
        3,
    );
    let back = step(
        &states,
        EventKind::Jump,
        "goto `again`: jump back to line 3",
    );
    assert!(back.event.summary.contains("earlier in `main`"));
    assert_eq!(back.event.clauses[0].id, "6.8.6.1p2");
    let _ahead = finished(
        "int main(void) {\n    int r = 1;\n    goto done;\n    r = 99;\ndone:\n    return r;\n}\n",
        1,
    );
    let states = finished(
        "int main(void) {\n    int found = -1;\n    for (int i = 0; i < 4; i++) {\n        for \
         (int j = 0; j < 4; j++) {\n            int p = i * j;\n            if (p == 6) { found = \
         10 * i + j; goto out; }\n        }\n    }\nout:\n    return found;\n}\n",
        23,
    );
    let jump = at(&states, EventKind::Jump, "goto `out`: jump ahead to line 9");
    let left: Vec<&State> = states[..jump]
        .iter()
        .filter(|state| state.event.summary.starts_with("`goto out;` leaves"))
        .collect();
    assert_eq!(
        left.len(),
        5,
        "the if's, the inner body's, the inner for's, ..."
    );
    assert_eq!(left[1].event.objects.len(), 1, "p ends");
    assert!(
        left[2]
            .event
            .summary
            .contains("the for statement on line 4, so `j`"),
        "{}",
        left[2].event.summary
    );
    for state in &states[jump - 5..jump] {
        assert_eq!(state.event.kind, EventKind::LeaveBlock);
    }
    assert!(
        states[jump]
            .event
            .explanation
            .contains("The 5 steps before this one")
    );
    let state = &states[jump];
    for name in ["i", "j", "p"] {
        assert!(
            state
                .objects
                .iter()
                .filter(|object| object.name == name)
                .all(|object| object.lifetime == "ended"),
            "{name}"
        );
    }
    let _cleanup = finished(
        "#include <stdio.h>\nint check(int v) {\n    if (v < 0) goto bad;\n    printf(\"ok \
         %d\\n\", v);\n    return 0;\nbad:\n    printf(\"bad %d\\n\", v);\n    return 1;\n}\nint \
         main(void) {\n    int a = check(3);\n    int b = check(-2);\n    return a + b * 2;\n}\n",
        2,
    );
    let _count = finished(
        "int main(void) {\n    int n = 0;\nloop: n++;\n    if (n < 4) goto loop;\n    return \
         n;\n}\n",
        4,
    );
}

// C99 §6.2.4p5, §6.8p3: a jump into a block begins its objects' lifetimes,
// but passes their initializers, so they are indeterminate.
#[test]
fn a_jump_into_a_block_begins_lifetimes_without_initializers() {
    let states =
        finished(
            "int main(void) {\n    int r = 0;\n    goto inside;\n    {\n        int y = \
             5;\ninside:\n        y = 7;\n        r = y;\n    }\n    return r;\n}\n",
            7,
        );
    let jump = step(
        &states,
        EventKind::Jump,
        "goto `inside`: jump ahead to line 6",
    );
    let y = jump
        .objects
        .iter()
        .find(|object| object.name == "y")
        .expect("y");
    assert_eq!(y.lifetime, "alive");
    assert_eq!(y.value, Value::Indeterminate);
    assert!(jump.event.objects.contains(&y.id));
    for words in [
        "The label is inside the block that begins on line 4, which the jump enters",
        "The jump passed its declaration, so no initializer ran",
        "variably modified type, such as a variable-length array",
        "its name is in scope at the label",
    ] {
        assert!(
            jump.event.explanation.contains(words),
            "{words}: {}",
            jump.event.explanation
        );
    }
    let last = stopped(
        "int main(void) {\n    goto inside;\n    {\n        int y = 5;\ninside:\n        return \
         y;\n    }\n}\n",
        "error",
    );
    assert_eq!(last.diagnostics[0].kind, "indeterminate");
    let states = finished(
        "int main(void) {\n    int r = 0;\n    goto skip;\n    int x = 4;\nskip:\n    x = 9;\n    \
         r = x;\n    return r;\n}\n",
        9,
    );
    let jump = step(&states, EventKind::Jump, "goto `skip`");
    assert!(
        jump.event
            .explanation
            .contains("The jump passes the declaration of `x` on line 4"),
        "{}",
        jump.event.explanation
    );
    let _loop = finished(
        "int main(void) {\n    int s = 0;\n    int i = 0;\n    goto body;\n    while (i < 3) \
         {\nbody:\n        s += 5;\n        i++;\n    }\n    return s;\n}\n",
        15,
    );
    // A jump past an inner declaration reaches the inner object.
    let _hidden = finished("int main(void) {\n    int x = 1;\n    {\n        goto in;\n        int x = 2;\nin:\n        x = 3;\n    }\n    return x;\n}\n", 1);
}

// C99 §6.2.4p5, §6.8p3: a jump into a block begins the lifetimes of all
// its objects, but passes only the declarations before its label; those
// after it, and their initializers, run when execution reaches them.
#[test]
fn a_jump_into_a_block_passes_only_the_declarations_before_its_label() {
    let goto = "int main(void) {\n    goto in;\n    {\n        int a = 1;\n    in: ;\n        int \
                b = 2;\n        return b;\n    }\n}\n";
    let switch = "int main(void) {\n    switch (2) {\n    case 1: {\n        int a = 1;\n    case \
                  2: ;\n        int b = 2;\n        return b;\n    }\n    }\n    return 0;\n}\n";
    for (source, title) in [
        (goto, "goto `in`: jump ahead to line 5"),
        (switch, "switch on 2: jump to `case 2`"),
    ] {
        let states = finished(source, 2);
        let at = states
            .iter()
            .position(|state| state.event.title.starts_with(title))
            .unwrap_or_else(|| panic!("{title}"));
        let jump = &states[at];
        let text = &jump.event.explanation;
        for words in [
            "`a` and `b` begin to exist here, indeterminate.",
            "`a` is declared before the label: the jump passed its declaration, so no initializer \
             ran",
            "`b` is declared after the label, so the jump did not pass it: execution reaches it \
             as usual, and only then does its name come into scope and any initializer run.",
        ] {
            assert!(text.contains(words), "{words}: {text}");
        }
        assert!(!text.contains("passed their declarations"), "{text}");
        let b = jump
            .objects
            .iter()
            .find(|object| object.name == "b")
            .expect("b");
        assert!(!b.declared, "`b`'s name is not in scope at the label");
        let declared = at
            + states[at..]
                .iter()
                .position(|state| state.event.title.starts_with("Declare `b`"))
                .expect("execution reaches `b`'s declaration");
        assert!(
            states[declared..]
                .iter()
                .any(|state| state.event.kind == EventKind::Initialize),
            "`b`'s initializer runs"
        );
    }
    let states = finished(
        "int main(void) {\n    goto in;\n    {\n    in: ;\n        int b = 2;\n        return \
         b;\n    }\n}\n",
        2,
    );
    let jump = step(&states, EventKind::Jump, "goto `in`");
    assert!(
        jump.event.explanation.contains(
            "Its declaration comes after the label, so the jump did not pass it: execution \
             reaches it as usual"
        ),
        "{}",
        jump.event.explanation
    );
    assert!(!jump.event.explanation.contains("The jump passed"));
}

// C99 §6.2.4p5: reaching a declaration again stores its initializer's
// value, or an indeterminate value, again.
#[test]
fn a_goto_back_over_a_declaration_reaches_it_again() {
    let states = finished(
        "int main(void) {\n    int pass = 0;\n    int total = 0;\ntop:\n    pass++;\n    int x = \
         pass * 10;\n    total += x;\n    if (pass < 3) goto top;\n    return total;\n}\n",
        60,
    );
    let jump = step(&states, EventKind::Jump, "goto `top`: jump back to line 4");
    assert!(
        jump.event
            .explanation
            .contains("The jump goes back before the declaration of `x` on line 6"),
        "{}",
        jump.event.explanation
    );
    let again = states
        .iter()
        .find(|state| {
            state.event.kind == EventKind::Declare
                && state
                    .event
                    .summary
                    .starts_with("A goto went back over this declaration")
        })
        .expect("the declaration is reached again");
    assert!(again.event.explanation.contains("it held 10"));
    let last =
        stopped(
            "int main(void) {\n    int pass = 0;\ntop:;\n    int x;\n    if (pass == 1) return \
             x;\n    x = 5;\n    pass++;\n    goto top;\n}\n",
            "error",
        );
    assert_eq!(last.diagnostics[0].kind, "indeterminate");
    let message = &last.diagnostics[0].message;
    assert!(
        message.starts_with(
            "a value was stored in `x` earlier, but execution then reached the declaration of `x` \
             on line 4 again"
        ),
        "{message}"
    );
    assert_eq!(last.event.kind, EventKind::Fault);
    assert!(
        last.event
            .summary
            .starts_with("The program stored a value in this object earlier, but reaching its"),
        "{}",
        last.event.summary
    );
    assert!(!last.event.summary.contains("never set"));
    assert_eq!(last.event.clauses[0].id, "6.8p3");
    // Without an earlier store, the read is of a value never set.
    let last =
        stopped(
            "int main(void) {\n    int pass = 0;\ntop:;\n    int x;\n    if (pass == 1) return \
             x;\n    pass++;\n    goto top;\n}\n",
            "error",
        );
    assert!(
        last.diagnostics[0]
            .message
            .starts_with("nothing has been stored in `x` yet"),
        "{}",
        last.diagnostics[0].message
    );
    for (source, words) in [
        (
            "int main(void) {\n    goto nowhere;\n    return 0;\n}\n",
            "C99 §6.8.6.1p1",
        ),
        (
            "int main(void) {\na:  ;\na:  return 0;\n}\n",
            "C99 §6.8.1p3",
        ),
    ] {
        let diagnostic = compile(source).expect_err(source);
        assert_eq!(diagnostic.kind, "constraint", "{source}");
        assert!(diagnostic.message.contains(words), "{}", diagnostic.message);
    }
}

// C99 §6.8p3: each pass over a declaration without an initializer makes
// its object's value indeterminate again, so a value stored on any earlier
// pass stays lost, over further passes, until something is stored there.
// A copy's indeterminate member is the copy's (C99 §6.2.6.1p6).
#[test]
fn a_value_a_declaration_made_indeterminate_stays_lost_until_a_store() {
    // A store, then two jumps back.
    let states = stepped(
        "int main(void) {\n    int n = 0;\nagain:;\n    int k;\n    if (n == 2) return k;\n    if \
         (n == 0) k = 4;\n    n++;\n    goto again;\n}\n",
    );
    let last = states.last().expect("a final state");
    assert_eq!(last.status, "error");
    assert!(
        last.diagnostics[0].message.starts_with(
            "a value was stored in `k` earlier, but execution then reached the declaration of `k` \
             on line 4 again"
        ),
        "{}",
        last.diagnostics[0].message
    );
    assert!(
        !last.event.summary.contains("never set"),
        "{}",
        last.event.summary
    );
    assert_eq!(last.event.clauses[0].id, "6.8p3");
    let passes: Vec<&State> = states
        .iter()
        .filter(|state| {
            state.event.kind == EventKind::Declare
                && state
                    .event
                    .summary
                    .starts_with("A goto went back over this declaration")
        })
        .collect();
    assert_eq!(passes.len(), 2);
    assert!(passes[0].event.explanation.contains("it held 4"));
    let third = &passes[1].event.explanation;
    assert!(
        third.contains(
            "it held no value: the value stored in it earlier became indeterminate when execution \
             reached this declaration on an earlier pass"
        ),
        "{third}"
    );
    assert!(third.contains("so it still holds none"), "{third}");
    assert!(!third.contains("no value yet"), "{third}");
    // A store, a jump back, a new store, a jump back: the second store is
    // lost too, and a store after the last pass is read back.
    let last = stopped(
        "int main(void) {\n    int n = 0;\nagain:;\n    int k;\n    if (n == 2) return k;\n    k \
         = n + 4;\n    n++;\n    goto again;\n}\n",
        "error",
    );
    assert!(
        last.diagnostics[0]
            .message
            .starts_with("a value was stored in `k` earlier"),
        "{}",
        last.diagnostics[0].message
    );
    assert_eq!(last.event.clauses[0].id, "6.8p3");
    finished(
        "int main(void) {\n    int n = 0;\nagain:;\n    int k;\n    if (n == 0) k = 4;\n    if (n \
         == 2) {\n        k = 9;\n        return k;\n    }\n    n++;\n    goto again;\n}\n",
        9,
    );
    // A structure reached again, then assigned a copy whose member was
    // never set: the read of that member is of a value never set, since the
    // copy stored it after the declaration.
    let last = stopped(
        "struct S { int a; int b; };\nint main(void) {\n    int n = 0;\nagain:;\n    struct S \
         s;\n    struct S t;\n    if (n == 1) {\n        t.b = 1;\n        s = t;\n        return \
         s.a;\n    }\n    s.a = 3;\n    n++;\n    goto again;\n}\n",
        "error",
    );
    assert!(
        last.diagnostics[0]
            .message
            .starts_with("nothing has been stored in `s.a` yet"),
        "{}",
        last.diagnostics[0].message
    );
    assert!(
        last.event.summary.contains("never set"),
        "{}",
        last.event.summary
    );
    assert_ne!(last.event.clauses[0].id, "6.8p3");
}

// C99 §6.8.1p1, §6.8.6.3p1: a label must label a statement. `break;` is
// suggested only after a case label, inside a switch; elsewhere it could be
// a constraint violation itself.
#[test]
fn a_label_without_a_statement_is_told_what_to_write() {
    let diagnostic = compile("int main(void) {\n    { goto L; L: }\n    return 0;\n}\n")
        .expect_err("a label ends the block");
    assert_eq!(
        diagnostic.message,
        "a label is followed by a statement; write an empty statement, `;`, after the last label"
    );
    let diagnostic = compile("int main(void) {\n    switch (1) { case 1: }\n    return 0;\n}\n")
        .expect_err("a case label ends the switch");
    assert!(
        diagnostic.message.contains("write `break;`"),
        "{}",
        diagnostic.message
    );
}

// C99 §6.8.4.2p2-5: a case label may be anywhere in a switch's body; the
// jump to one inside a block enters the block.
#[test]
fn a_switch_jumps_to_labels_nested_in_its_body() {
    let states = finished(
        "int main(void) {\n    int from[7] = {1, 2, 3, 4, 5, 6, 7};\n    int to[7];\n    int \
         count = 7;\n    int *f = from;\n    int *t = to;\n    int n = (count + 3) / 4;\n    \
         switch (count % 4) {\n    case 0: do { *t++ = *f++;\n    case 3:      *t++ = *f++;\n    \
         case 2:      *t++ = *f++;\n    case 1:      *t++ = *f++;\n            } while (--n > \
         0);\n    }\n    return to[0] + to[6] * 10;\n}\n",
        71,
    );
    let fallen: Vec<&State> = states
        .iter()
        .filter(|state| state.event.title.starts_with("Run on past `case"))
        .collect();
    assert_eq!(
        fallen.len(),
        5,
        "case 2 and case 1, then case 3, 2 and 1 again but the last"
    );
    let states = finished("int main(void) {\n    int r = 0;\n    switch (2) {\n    case 1: {\n        int y = 10;\n        r += y;\n    case 2:\n        y = 20;\n        r += y;\n    }\n    }\n    return r;\n}\n", 20);
    let switch = step(&states, EventKind::Branch, "switch on 2: jump to `case 2`");
    let y = switch
        .objects
        .iter()
        .find(|object| object.name == "y")
        .expect("y");
    assert_eq!(y.value, Value::Indeterminate);
    assert!(switch.event.objects.contains(&y.id));
    assert!(
        switch
            .event
            .explanation
            .contains("The label is inside the block that begins on line 4")
    );
    let _if = finished("int main(void) {\n    int r = 0;\n    int c = 0;\n    switch (3) {\n    case 1:\n        if (c) {\n    case 3:\n            r = 30;\n        }\n        break;\n    default:\n        r = 1;\n    }\n    return r;\n}\n", 30);
    let last =
        stopped(
            "int main(void) {\n    switch (2) {\n    case 1: {\n        int y = 10;\n    case \
             2:\n        return y;\n    }\n    }\n    return 0;\n}\n",
            "error",
        );
    assert_eq!(last.diagnostics[0].kind, "indeterminate");
    let states =
        finished(
            "int main(void) {\n    int r = 0;\n    for (int k = 0; k < 3; k++) {\n        switch \
             (k) {\n        case 0: r += 1;\n            {\n        default: r += 10;\n            \
             }\n        }\n    }\n    return r;\n}\n",
            31,
        );
    let first = step(&states, EventKind::Jump, "Run on past `default:`");
    assert!(
        first
            .event
            .summary
            .starts_with("Execution has entered the block that `default:` begins"),
        "{}",
        first.event.summary
    );
    // A run of labels is said in the plural before it is reached.
    let states = stepped(
        "int main(void) { int r = 0; switch (1) { case 1: r = 1; case 2: case 3: r++; } return r; \
         }",
    );
    assert!(
        states
            .iter()
            .filter_map(|state| state.upcoming.as_ref())
            .any(|upcoming| upcoming.text
                == "run on past `case 2: case 3:` into the statements after them")
    );
}

// C99 §6.2.1p4, p7: an inner declaration hides an outer one from its
// declarator to the end of its block; before it, the outer one is meant.
#[test]
fn a_use_of_a_hidden_name_says_which_object_it_reaches() {
    let states = finished(
        "int main(void) {\n    int x = 1;\n    int r = 0;\n    {\n        r = x;\n        int x = \
         2;\n        r = r + x;\n    }\n    return r + x;\n}\n",
        4,
    );
    let reads: Vec<&State> = states
        .iter()
        .filter(|state| state.event.title == "`x` names an object to read")
        .collect();
    assert!(
        reads[0]
            .event
            .explanation
            .contains("until then `x` still means the one declared on line 2"),
        "{}",
        reads[0].event.explanation
    );
    assert!(
        reads[1]
            .event
            .explanation
            .contains("This `x` is the one declared on line 6"),
        "{}",
        reads[1].event.explanation
    );
    assert!(reads[1].event.explanation.contains("which still holds 1"));
    assert!(
        reads[1]
            .event
            .clauses
            .iter()
            .any(|clause| clause.id == "6.2.1p4")
    );
    assert!(
        !reads[2].event.explanation.contains("Another object"),
        "the block has ended"
    );
    let states = finished(
        "int x = 5;\nint main(void) {\n    int r = x;\n    int x = 7;\n    return r * 10 + x;\n}\n",
        57,
    );
    assert!(states.iter().any(|state| {
        state
            .event
            .explanation
            .contains("the x declared at file scope, on line 1")
    }));
    let states = finished(
        "int f(int x) {\n    {\n        int x = 3;\n        return x;\n    }\n}\nint main(void) \
         {\n    return f(9);\n}\n",
        3,
    );
    assert!(states.iter().any(|state| {
        state
            .event
            .explanation
            .contains("the x declared as a parameter of `f`")
    }));
}

// C99 §6.2.4p2: a function may return a pointer to an object; one to its
// own automatic object dangles once it returns, and using it is undefined.
#[test]
fn a_function_returns_pointers_to_objects_and_its_own_dangle() {
    let _global = finished(
        "int table[3] = {4, 5, 6};\nint *at(int i) { return &table[i]; }\nint main(void) {\n    \
         int *p = at(1);\n    *p = 50;\n    return table[1] + *at(2);\n}\n",
        56,
    );
    let _parameter = finished(
        "int *larger(int *a, int *b) { return *a > *b ? a : b; }\nint main(void) {\n    int x = \
         3;\n    int y = 8;\n    *larger(&x, &y) = 1;\n    return x * 10 + y;\n}\n",
        31,
    );
    let _null = finished(
        "int *find(int *v, int n, int key) {\n    for (int i = 0; i < n; i++)\n        if (v[i] \
         == key) return &v[i];\n    return 0;\n}\nint main(void) {\n    int v[4] = {3, 1, 4, \
         1};\n    int *p = find(v, 4, 4);\n    int *q = find(v, 4, 9);\n    return (p - v) * 10 + \
         (q == 0);\n}\n",
        21,
    );
    let _static = finished(
        "int *counter(void) {\n    static int n = 0;\n    n++;\n    return &n;\n}\nint main(void) \
         {\n    counter();\n    counter();\n    return *counter();\n}\n",
        3,
    );
    // `f()->m` selects a member of the structure the result points to.
    let _arrow = finished(
        "struct point { int x; int y; };\nstruct point origin = {3, 4};\nstruct point *get(void) \
         { return &origin; }\nint main(void) {\n    get()->x = 30;\n    return get()->x + \
         get()->y;\n}\n",
        34,
    );
    let _compound = finished(
        "int v[2] = {1, 2};\nint *second(void) { return &v[1]; }\nint main(void) {\n    *second() \
         += 40;\n    second()[0] = second()[0] * 2;\n    return v[1];\n}\n",
        84,
    );
    let _chars = finished(
        "char *skip(char *s) { while (*s == ' ') s++; return s; }\nint main(void) {\n    char \
         text[] = \"  hi\";\n    return *skip(text);\n}\n",
        104,
    );
    let states = stepped(
        "int *bad(void) {\n    int local = 3;\n    return &local;\n}\nint main(void) {\n    int \
         *p = bad();\n    return *p;\n}\n",
    );
    let returned = step(&states, EventKind::Return, "Return &local");
    assert!(
        returned
            .event
            .summary
            .contains("so the caller receives a dangling pointer"),
        "{}",
        returned.event.summary
    );
    let last = states.last().expect("a last state");
    assert_eq!(last.status, "undefined_behavior");
    assert_eq!(
        last.diagnostics[0].undefined,
        Some(super::super::Undefined::DanglingPointer)
    );
    assert_eq!(
        last.event.title,
        "Undefined behavior: use of a dangling pointer"
    );
    let last = stopped(
        "int *bad(void) {\n    int local = 3;\n    return &local;\n}\nint main(void) {\n    \
         return bad() != 0;\n}\n",
        "undefined_behavior",
    );
    assert_eq!(
        last.diagnostics[0].citation,
        Some("C99 §6.2.4p2, p. 32; PDF p. 44")
    );
    // Discarding the pointer does not use it.
    let _discarded = finished(
        "int *bad(void) {\n    int local = 3;\n    return &local;\n}\nint main(void) {\n    \
         bad();\n    return 0;\n}\n",
        0,
    );
    let diagnostic = compile("typedef int row[3]; row f(void);").expect_err("an array return");
    assert_eq!(diagnostic.kind, "constraint");
}

// Part 16's nits: an unnamed array parameter's refusal points at its type,
// and an enumeration with one constant is said in the singular.
#[test]
fn an_unnamed_parameter_and_a_single_constant_are_said_exactly() {
    let diagnostic =
        compile("typedef int row[3];\nint first(row);\nint main(void) { return 0; }\n")
            .expect_err("an array parameter");
    assert_eq!(
        (diagnostic.source_range.line, diagnostic.source_range.column),
        (2, 11),
        "{}",
        diagnostic.message
    );
    let states = finished(
        "enum one { ONLY };\nint main(void) {\n    enum one v = ONLY;\n    return v;\n}\n",
        0,
    );
    let declared = step(&states, EventKind::Declare, "Declare `v`");
    assert!(
        declared
            .event
            .explanation
            .contains("C does not check that it holds its only constant, ONLY."),
        "{}",
        declared.event.explanation
    );
}

#[test]
fn every_later_pass_says_an_earlier_pass_lost_the_value() {
    // A store on the first pass, then three jumps back: the fourth pass
    // must not say the previous pass lost the value, which it already
    // held none of (C99 §6.8p3).
    let states = stepped(
        "int main(void) {\n    int n = 0;\nagain:;\n    int k;\n    if (n == 3) return k;\n    if \
         (n == 0) k = 4;\n    n++;\n    goto again;\n}\n",
    );
    let passes: Vec<_> = states
        .iter()
        .filter(|state| {
            state.event.kind == EventKind::Declare
                && state
                    .event
                    .summary
                    .starts_with("A goto went back over this declaration")
        })
        .collect();
    assert_eq!(passes.len(), 3);
    for pass in &passes[1..] {
        let text = &pass.event.explanation;
        assert!(
            text.contains(
                "became indeterminate when execution reached this declaration on an earlier pass"
            ),
            "{text}"
        );
        assert!(!text.contains("last reached"), "{text}");
    }
}
