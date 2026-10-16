//! Control flow: loops, block arguments, switches, recursion on the explicit
//! call stack, and the step and depth limits.

use super::*;

/// The second example of `middle-end.md`: the sum of `0..n`.
const COUNT: &str = "\
function @count(i32) -> i32 external {
    slot0 = stack_slot 4, align 4
block0(v0: i32):
    v1 = iconst.i32 0
    jump block1(v1, v1)
block1(v2: i32, v3: i32):
    v4 = icmp.i32 slt v2, v0
    brif v4, block2, block3
block2:
    v5 = iadd.i32 nsw v3, v2
    v6 = iconst.i32 1
    v7 = iadd.i32 nsw v2, v6
    jump block1(v7, v5)
block3:
    return v3
}
";

#[test]
fn loops_sum_a_range() {
    assert_eq!(returned_with(COUNT, "count", &[int(101)]), int(5050));
    assert_eq!(returned_with(COUNT, "count", &[int(0)]), int(0));
}

#[test]
fn block_arguments_are_assigned_at_once() {
    // Swapping through block parameters must read both arguments before
    // writing either parameter.
    let text = "\
function @f() -> i32 external {
block0:
    v0 = iconst.i32 1
    v1 = iconst.i32 2
    v2 = iconst.i32 3
    jump block1(v0, v1, v2)
block1(v3: i32, v4: i32, v5: i32):
    v6 = iconst.i32 1
    v7 = isub.i32 v5, v6
    v8 = icmp.i32 eq v5, v6
    brif v8, block2, block1(v4, v3, v7)
block2:
    v9 = iconst.i32 10
    v10 = imul.i32 v3, v9
    v11 = iadd.i32 v10, v4
    return v11
}
";
    // Three passes swap (1, 2) twice, leaving (1, 2): 12.
    assert_eq!(returned(text, "f"), int(12));
}

#[test]
fn switch_takes_the_matching_case_or_the_default() {
    let text = "\
function @f(i16) -> i32 external {
block0(v0: i16):
    switch v0, block1, [-1: block2, 7: block3]
block1:
    v1 = iconst.i32 100
    return v1
block2:
    v2 = iconst.i32 200
    return v2
block3:
    v3 = iconst.i32 300
    return v3
}
";
    assert_eq!(returned_with(text, "f", &[int(0xFFFF)]), int(200));
    assert_eq!(returned_with(text, "f", &[int(7)]), int(300));
    assert_eq!(returned_with(text, "f", &[int(8)]), int(100));
}

const FACTORIAL: &str = "\
function @factorial(i64) -> i64 external {
block0(v0: i64):
    v1 = iconst.i64 1
    v2 = icmp.i64 sle v0, v1
    brif v2, block1, block2
block1:
    return v1
block2:
    v3 = isub.i64 v0, v1
    v4 = call @factorial(v3)
    v5 = imul.i64 v0, v4
    return v5
}
";

#[test]
fn recursion_computes_factorial_and_fibonacci() {
    assert_eq!(
        returned_with(FACTORIAL, "factorial", &[int(20)]),
        int(2_432_902_008_176_640_000)
    );
    let fibonacci = "\
function @fib(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 2
    v2 = icmp.i32 slt v0, v1
    brif v2, block1, block2
block1:
    return v0
block2:
    v3 = iconst.i32 1
    v4 = isub.i32 nsw v0, v3
    v5 = call @fib(v4)
    v6 = isub.i32 nsw v0, v1
    v7 = call @fib(v6)
    v8 = iadd.i32 nsw v5, v7
    return v8
}
";
    assert_eq!(returned_with(fibonacci, "fib", &[int(20)]), int(6765));
}

/// The sum of `1..=n`, by recursion `n` calls deep.
const DEEP_SUM: &str = "\
function @sum(i64) -> i64 external {
    slot0 = stack_slot 8, align 8
block0(v0: i64):
    v1 = stack_addr slot0
    store.i64 v0, v1, align 8
    v2 = iconst.i64 0
    v3 = icmp.i64 eq v0, v2
    brif v3, block1, block2
block1:
    return v2
block2:
    v4 = iconst.i64 1
    v5 = isub.i64 v0, v4
    v6 = call @sum(v5)
    v7 = load.i64 v1, align 8
    v8 = iadd.i64 v6, v7
    return v8
}
";

#[test]
fn deep_recursion_runs_on_the_explicit_stack() {
    // A native recursion per call would overflow the test thread's stack
    // long before this depth.
    assert_eq!(
        returned_with(DEEP_SUM, "sum", &[int(100_000)]),
        int(5_000_050_000)
    );
}

#[test]
fn limits_stop_runaway_programs() {
    let limits = Limits {
        stack_depth: 1000,
        ..Limits::default()
    };
    let run = run_text_with(DEEP_SUM, "sum", &[int(5000)], limits);
    assert!(
        matches!(run.result, Err(Trap::StackDepthLimit(_))),
        "{:?}",
        run.result
    );
    let spin = "\
function @f() -> i32 external {
block0:
    jump block1
block1:
    jump block1
}
";
    let limits = Limits {
        steps: 10_000,
        ..Limits::default()
    };
    let run = run_text_with(spin, "f", &[], limits);
    assert_eq!(run.result, Err(Trap::StepLimit));
    let outcome = run_text_with(COUNT, "count", &[int(10)], Limits::default())
        .result
        .unwrap();
    assert_eq!(outcome.termination, Termination::Returned(Some(int(45))));
    assert_eq!(
        outcome.steps,
        2 + 10 * 6 + 2 + 1,
        "the instructions executed"
    );
}

#[test]
fn reaching_unreachable_is_undefined() {
    let text = "\
function @f() -> i32 external {
block0:
    unreachable
}
";
    assert_eq!(undefined(text, "f"), UbKind::Unreachable);
}

#[test]
fn entry_errors_are_reported() {
    let bad_entry = |entry: &str, args: &[RuntimeValue]| match run_text_with(
        COUNT,
        entry,
        args,
        Limits::default(),
    )
    .result
    {
        | Err(Trap::BadEntry(error)) => error,
        | other => panic!("{other:?}"),
    };
    assert_eq!(bad_entry("missing", &[]), EntryError::NotFound);
    assert_eq!(bad_entry("count", &[]), EntryError::Arguments);
    assert_eq!(
        bad_entry("count", &[RuntimeValue::F32(1.0)]),
        EntryError::Arguments
    );
    let declared = "function @f() -> i32 external\n";
    match run_text(declared, "f").result {
        | Err(Trap::BadEntry(EntryError::NotDefined)) => {},
        | other => panic!("{other:?}"),
    }
}
