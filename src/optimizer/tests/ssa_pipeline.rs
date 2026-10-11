//! `promote` and `sccp` with the other passes, and interpreter checks that
//! optimized programs compute what the originals do.

use super::*;
use crate::backend::interpreter::{
    Limits,
    RuntimeValue,
    run_text_with,
};

/// `int sum(int n) { int s = 0; for (int i = 0; i < n; i++) s += i; return s;
/// }` as unoptimized lowering writes it: every local, the parameter included,
/// lives in a stack slot.
const SUM: &str = "\
function @sum(i32) -> i32 external {
    slot0 = stack_slot 4, align 4
    slot1 = stack_slot 4, align 4
    slot2 = stack_slot 4, align 4
block0(v0: i32):
    v1 = stack_addr slot0
    v2 = stack_addr slot1
    v3 = stack_addr slot2
    store.i32 v0, v1, align 4
    v4 = iconst.i32 0
    store.i32 v4, v2, align 4
    store.i32 v4, v3, align 4
    jump block1
block1:
    v5 = load.i32 v3, align 4
    v6 = load.i32 v1, align 4
    v7 = icmp.i32 slt v5, v6
    brif v7, block2, block3
block2:
    v8 = load.i32 v2, align 4
    v9 = load.i32 v3, align 4
    v10 = iadd.i32 nsw v8, v9
    store.i32 v10, v2, align 4
    v11 = load.i32 v3, align 4
    v12 = iconst.i32 1
    v13 = iadd.i32 nsw v11, v12
    store.i32 v13, v3, align 4
    jump block1
block3:
    v14 = load.i32 v2, align 4
    return v14
}
";

/// A loop guarded by a flag kept in a slot that nothing ever sets, so the
/// loop body never runs.
const NEVER_SET_FLAG: &str = "\
function @f(i32) -> i32 external {
    slot0 = stack_slot 1, align 1
    slot1 = stack_slot 4, align 4
block0(v0: i32):
    v1 = stack_addr slot0
    v2 = stack_addr slot1
    v3 = iconst.i8 0
    store.i8 v3, v1, align 1
    store.i32 v0, v2, align 4
    jump block1
block1:
    v4 = load.i8 v1, align 1
    v5 = icmp.i8 ne v4, v3
    brif v5, block2, block3
block2:
    v6 = load.i32 v2, align 4
    v7 = iconst.i32 1
    v8 = iadd.i32 v6, v7
    store.i32 v8, v2, align 4
    jump block1
block3:
    v9 = load.i32 v2, align 4
    return v9
}
";

/// Prints a letter per trip through a loop and sums the even indices; the
/// output and the result both have to survive.
const LETTERS: &str = "\
function @putchar(i32) -> i32 external

function @letters(i32) -> i32 external {
    slot0 = stack_slot 4, align 4
    slot1 = stack_slot 4, align 4
block0(v0: i32):
    v1 = stack_addr slot0
    v2 = stack_addr slot1
    v3 = iconst.i32 0
    store.i32 v3, v1, align 4
    store.i32 v3, v2, align 4
    jump block1
block1:
    v4 = load.i32 v1, align 4
    v5 = icmp.i32 slt v4, v0
    brif v5, block2, block5
block2:
    v6 = iconst.i32 97
    v7 = iadd.i32 nsw v6, v4
    v8 = call @putchar(v7)
    v9 = iconst.i32 2
    v10 = srem.i32 v4, v9
    v11 = icmp.i32 eq v10, v3
    brif v11, block3, block4
block3:
    v12 = load.i32 v2, align 4
    v13 = iadd.i32 nsw v12, v4
    store.i32 v13, v2, align 4
    jump block4
block4:
    v14 = load.i32 v1, align 4
    v15 = iconst.i32 1
    v16 = iadd.i32 nsw v14, v15
    store.i32 v16, v1, align 4
    jump block1
block5:
    v17 = load.i32 v2, align 4
    return v17
}
";

/// Recursive factorial with its parameter and result spilled to slots.
const FACTORIAL: &str = "\
function @fact(i32) -> i32 external {
    slot0 = stack_slot 4, align 4
    slot1 = stack_slot 4, align 4
block0(v0: i32):
    v1 = stack_addr slot0
    store.i32 v0, v1, align 4
    v2 = stack_addr slot1
    v3 = load.i32 v1, align 4
    v4 = iconst.i32 1
    v5 = icmp.i32 sle v3, v4
    brif v5, block1, block2
block1:
    store.i32 v4, v2, align 4
    jump block3
block2:
    v6 = load.i32 v1, align 4
    v7 = isub.i32 nsw v6, v4
    v8 = call @fact(v7)
    v9 = load.i32 v1, align 4
    v10 = imul.i32 nsw v9, v8
    store.i32 v10, v2, align 4
    jump block3
block3:
    v11 = load.i32 v2, align 4
    return v11
}
";

/// A mode kept in a slot picks a `switch` arm; another slot escapes to a
/// callee that writes it, so it stays in memory.
const SWITCH_AND_ESCAPE: &str = "\
function @set(ptr, i32) internal {
block0(v0: ptr, v1: i32):
    v2 = iconst.i32 2
    v3 = imul.i32 v1, v2
    store.i32 v3, v0, align 4
    return
}

function @main(i32) -> i32 external {
    slot0 = stack_slot 4, align 4
    slot1 = stack_slot 4, align 4
    slot2 = stack_slot 4, align 4
block0(v0: i32):
    v1 = stack_addr slot0
    v2 = stack_addr slot1
    v3 = stack_addr slot2
    v4 = iconst.i32 2
    store.i32 v4, v1, align 4
    v5 = iconst.i32 0
    store.i32 v5, v3, align 4
    call @set(v2, v0)
    v6 = load.i32 v1, align 4
    switch v6, block3, [1: block1, 2: block2]
block1:
    v7 = iconst.i32 100
    store.i32 v7, v3, align 4
    jump block4
block2:
    v8 = load.i32 v2, align 4
    v9 = iconst.i32 1
    v10 = iadd.i32 v8, v9
    store.i32 v10, v3, align 4
    jump block4
block3:
    v11 = iconst.i32 -1
    store.i32 v11, v3, align 4
    jump block4
block4:
    v12 = load.i32 v3, align 4
    return v12
}
";

/// Walks a constant array through a pointer kept in a slot, with an `i64`
/// index in another.
const ARRAY_WALK: &str = "\
global @data internal constant size 16, align 4 = bytes \"01000000020000000300000004000000\"

function @total() -> i32 external {
    slot0 = stack_slot 8, align 8
    slot1 = stack_slot 4, align 4
    slot2 = stack_slot 8, align 8
block0:
    v0 = stack_addr slot0
    v1 = stack_addr slot1
    v2 = stack_addr slot2
    v3 = global_addr @data
    store.ptr v3, v0, align 8
    v4 = iconst.i32 0
    store.i32 v4, v1, align 4
    v5 = iconst.i64 0
    store.i64 v5, v2, align 8
    jump block1
block1:
    v6 = load.i64 v2, align 8
    v7 = iconst.i64 4
    v8 = icmp.i64 slt v6, v7
    brif v8, block2, block3
block2:
    v9 = load.ptr v0, align 8
    v10 = load.i32 v9, align 4
    v11 = load.i32 v1, align 4
    v12 = iadd.i32 v11, v10
    store.i32 v12, v1, align 4
    v13 = ptr_add inbounds v9, v7
    store.ptr v13, v0, align 8
    v14 = iconst.i64 1
    v15 = iadd.i64 v6, v14
    store.i64 v15, v2, align 8
    jump block1
block3:
    v16 = load.i32 v1, align 4
    return v16
}
";

/// `x = 1; for (i = 0; i < n; i++) if (x != 1) x = 2; return x * 10 + i;`:
/// `x` is 1 along every edge that can run, which only `sccp` sees, because
/// the dead arm's store reaches the loop header through a parameter.
const STICKY_FLAG: &str = "\
function @sticky(i32) -> i32 external {
    slot0 = stack_slot 4, align 4
    slot1 = stack_slot 4, align 4
block0(v0: i32):
    v1 = stack_addr slot0
    v2 = stack_addr slot1
    v3 = iconst.i32 1
    store.i32 v3, v1, align 4
    v4 = iconst.i32 0
    store.i32 v4, v2, align 4
    jump block1
block1:
    v5 = load.i32 v2, align 4
    v6 = icmp.i32 slt v5, v0
    brif v6, block2, block5
block2:
    v7 = load.i32 v1, align 4
    v8 = icmp.i32 ne v7, v3
    brif v8, block3, block4
block3:
    v9 = iconst.i32 2
    store.i32 v9, v1, align 4
    jump block4
block4:
    v10 = load.i32 v2, align 4
    v11 = iadd.i32 nsw v10, v3
    store.i32 v11, v2, align 4
    jump block1
block5:
    v12 = load.i32 v1, align 4
    v13 = iconst.i32 10
    v14 = imul.i32 nsw v12, v13
    v15 = load.i32 v2, align 4
    v16 = iadd.i32 nsw v14, v15
    return v16
}
";

/// An irreducible loop of two blocks, entered at either one depending on the
/// parity of `n`, with the count and an accumulator in slots.
const IRREDUCIBLE: &str = "\
function @irreducible(i32) -> i32 external {
    slot0 = stack_slot 4, align 4
    slot1 = stack_slot 4, align 4
block0(v0: i32):
    v1 = stack_addr slot0
    v2 = stack_addr slot1
    store.i32 v0, v1, align 4
    v3 = iconst.i32 0
    store.i32 v3, v2, align 4
    v4 = iconst.i32 1
    v5 = and.i32 v0, v4
    v6 = icmp.i32 ne v5, v3
    brif v6, block1, block2
block1:
    v7 = load.i32 v2, align 4
    v8 = iconst.i32 3
    v9 = iadd.i32 v7, v8
    store.i32 v9, v2, align 4
    v10 = load.i32 v1, align 4
    v11 = isub.i32 v10, v4
    store.i32 v11, v1, align 4
    v12 = icmp.i32 sgt v11, v3
    brif v12, block2, block3
block2:
    v13 = load.i32 v2, align 4
    v14 = iconst.i32 5
    v15 = iadd.i32 v13, v14
    store.i32 v15, v2, align 4
    v16 = load.i32 v1, align 4
    v17 = isub.i32 v16, v4
    store.i32 v17, v1, align 4
    v18 = icmp.i32 sgt v17, v3
    brif v18, block1, block3
block3:
    v19 = load.i32 v2, align 4
    return v19
}
";

/// The default pipeline with verification after every pass.
fn verified() -> OptimizerOptions<'static> {
    OptimizerOptions {
        verify_each: true,
        ..OptimizerOptions::default()
    }
}

#[test]
fn promotion_then_propagation_gives_a_clean_loop() {
    let expected = "\
function @sum(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 0
    jump block1(v1, v1)
block1(v2: i32, v3: i32):
    v4 = icmp.i32 slt v3, v0
    brif v4, block2, block3
block2:
    v5 = iadd.i32 nsw v2, v3
    v6 = iconst.i32 1
    v7 = iadd.i32 nsw v3, v6
    jump block1(v5, v7)
block3:
    return v2
}
";
    let passes = [Pass::Promote, Pass::Sccp, Pass::SimplifyCfg, Pass::Dce];
    let (text, _) = optimize_text(SUM, &only(&passes));
    pretty_assertions::assert_eq!(text, expected);
    // The default pipeline also hoists the constant out of the loop.
    let hoisted = "\
function @sum(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 0
    v2 = iconst.i32 1
    jump block1(v1, v1)
block1(v3: i32, v4: i32):
    v5 = icmp.i32 slt v4, v0
    brif v5, block2, block3
block2:
    v6 = iadd.i32 nsw v3, v4
    v7 = iadd.i32 nsw v4, v2
    jump block1(v6, v7)
block3:
    return v3
}
";
    let (text, _) = optimize_text(SUM, &verified());
    pretty_assertions::assert_eq!(text, hoisted);
}

#[test]
fn a_flag_that_is_never_set_removes_its_loop() {
    let expected = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    return v0
}
";
    let passes = [Pass::Promote, Pass::Sccp, Pass::SimplifyCfg, Pass::Dce];
    let (text, _) = optimize_text(NEVER_SET_FLAG, &only(&passes));
    pretty_assertions::assert_eq!(text, expected);
    // Without promotion, `sccp` cannot see through memory.
    let (text, report) = optimize_text(NEVER_SET_FLAG, &only(&[Pass::Sccp]));
    assert_eq!(text, NEVER_SET_FLAG);
    assert_eq!(report.transformations(), 0);
}

#[test]
fn a_switch_on_a_promoted_mode_keeps_the_escaping_slot() {
    let (text, _) = optimize_text(SWITCH_AND_ESCAPE, &verified());
    let main = text
        .split("function @main")
        .nth(1)
        .expect("the module keeps @main");
    let expected = "(i32) -> i32 external {
    slot0 = stack_slot 4, align 4
block0(v0: i32):
    v1 = stack_addr slot0
    call @set(v1, v0)
    v2 = load.i32 v1, align 4
    v3 = iconst.i32 1
    v4 = iadd.i32 v2, v3
    return v4
}
";
    pretty_assertions::assert_eq!(main, expected);
}

#[test]
fn optimized_programs_compute_what_the_originals_do() {
    let int = |value: i32| RuntimeValue::Int(u128::from(value.cast_unsigned()));
    let programs: [(&str, &str, Vec<Vec<RuntimeValue>>); 8] = [
        (
            SUM,
            "sum",
            [0, 1, 5, 100, -3].map(|n| vec![int(n)]).to_vec(),
        ),
        (
            NEVER_SET_FLAG,
            "f",
            [0, 41, -7].map(|n| vec![int(n)]).to_vec(),
        ),
        (
            LETTERS,
            "letters",
            [0, 1, 5, 26].map(|n| vec![int(n)]).to_vec(),
        ),
        (
            FACTORIAL,
            "fact",
            [0, 1, 5, 10, 12].map(|n| vec![int(n)]).to_vec(),
        ),
        (
            SWITCH_AND_ESCAPE,
            "main",
            [0, 20, -4].map(|n| vec![int(n)]).to_vec(),
        ),
        (ARRAY_WALK, "total", vec![vec![]]),
        (
            STICKY_FLAG,
            "sticky",
            [0, 3, 7, -2].map(|n| vec![int(n)]).to_vec(),
        ),
        (
            IRREDUCIBLE,
            "irreducible",
            [0, 1, 2, 7, 10, -3].map(|n| vec![int(n)]).to_vec(),
        ),
    ];
    for (text, entry, inputs) in programs {
        let (optimized, report) = optimize_text(text, &verified());
        assert!(report.transformations() > 0, "{entry} was optimized");
        for args in inputs {
            let before = run_text_with(text, entry, &args, Limits::default());
            let after = run_text_with(&optimized, entry, &args, Limits::default());
            let before_result = before
                .result
                .unwrap_or_else(|trap| panic!("{entry}{args:?} trapped: {trap:?}"));
            let after_result = after.result.unwrap_or_else(|trap| {
                panic!("optimized {entry}{args:?} trapped: {trap:?}\n{optimized}")
            });
            assert_eq!(
                after_result.termination, before_result.termination,
                "{entry}{args:?}\n{optimized}"
            );
            assert_eq!(after.output, before.output, "{entry}{args:?}");
            assert!(
                after_result.steps <= before_result.steps,
                "{entry}{args:?} got slower"
            );
        }
    }
}

#[test]
fn promotable_slots_leave_no_memory_traffic() {
    for text in [
        SUM,
        LETTERS,
        FACTORIAL,
        ARRAY_WALK,
        STICKY_FLAG,
        IRREDUCIBLE,
    ] {
        let (optimized, _) = optimize_text(text, &verified());
        assert!(!optimized.contains("stack_slot"), "{optimized}");
        assert!(!optimized.contains("store"), "{optimized}");
    }
}

#[test]
fn bisecting_through_promote_and_sccp_keeps_every_prefix_valid() {
    // Every prefix of the rewrites must verify and still compute the same
    // result; the last must match the full run.
    let (full_text, full) = optimize_text(SUM, &verified());
    let total = full.transformations();
    assert!(total >= 3);
    let args = [RuntimeValue::Int(10)];
    let expected = run_text_with(SUM, "sum", &args, Limits::default())
        .result
        .expect("the original runs")
        .termination;
    for limit in 0..=total {
        let limited = OptimizerOptions {
            bisect_limit: Some(limit),
            ..verified()
        };
        let (text, report) = optimize_text(SUM, &limited);
        assert_eq!(report.transformations(), limit);
        let run = run_text_with(&text, "sum", &args, Limits::default());
        assert_eq!(
            run.result.expect("each prefix runs").termination,
            expected,
            "limit {limit}\n{text}"
        );
        if limit == total {
            assert_eq!(text, full_text);
        }
    }
}

#[test]
fn a_flag_constant_only_along_executable_edges_folds_away() {
    let (text, report) = optimize_text(STICKY_FLAG, &verified());
    assert!(report.stats(Pass::Sccp).changes > 0, "{report}");
    let expected = "\
function @sticky(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 1
    v2 = iconst.i32 0
    jump block1(v2)
block1(v3: i32):
    v4 = icmp.i32 slt v3, v0
    brif v4, block2, block3
block2:
    v5 = iadd.i32 nsw v3, v1
    jump block1(v5)
block3:
    v6 = iconst.i32 10
    v7 = iadd.i32 nsw v6, v3
    return v7
}
";
    pretty_assertions::assert_eq!(text, expected);
}
