//! Programs run in the interpreter before and after `gvn` and `licm`: the
//! optimized program must return what the original returned, or, where the
//! original returned poison, anything at all.

use super::*;
use crate::{
    backend::interpreter::{
        Limits,
        Outcome,
        RuntimeValue,
        Termination,
        run_text_with,
    },
    ir::Type,
};

/// The pass lists each program is checked under; `None` is the default
/// pipeline.
const PIPELINES: [Option<&[Pass]>; 6] = [
    Some(&[Pass::Inline]),
    Some(&[Pass::Gvn]),
    Some(&[Pass::Licm]),
    Some(&[Pass::Gvn, Pass::Licm, Pass::Dce]),
    Some(&[Pass::Licm, Pass::Gvn, Pass::Licm]),
    None,
];

/// Optimizes `text` under every pipeline and checks that `entry` returns
/// the same value for each argument list.
pub(super) fn assert_equivalent(text: &str, entry: &str, inputs: &[&[i64]]) {
    for passes in PIPELINES {
        let options = match passes {
            | Some(passes) => only(passes),
            | None => OptimizerOptions {
                verify_each: true,
                ..OptimizerOptions::default()
            },
        };
        let (optimized, _) = optimize_text(text, &options);
        for &input in inputs {
            let args: Vec<RuntimeValue> = input
                .iter()
                .map(|&arg| RuntimeValue::int(Type::I32, arg as u128))
                .collect();
            let before = returned(text, entry, &args);
            let after = returned(&optimized, entry, &args);
            if before != RuntimeValue::Poison {
                assert_eq!(
                    before, after,
                    "{entry}{input:?} after {passes:?}:\n{optimized}"
                );
            }
        }
    }
}

/// What `entry` returned, panicking if it trapped or returned nothing.
pub(super) fn returned(text: &str, entry: &str, args: &[RuntimeValue]) -> RuntimeValue {
    let run = run_text_with(text, entry, args, Limits::default());
    match run.result {
        | Ok(Outcome {
            termination: Termination::Returned(Some(value)),
            ..
        }) => value,
        | other => panic!("{other:?} {:?}\n{text}", run.message),
    }
}

#[test]
fn nested_loops_with_invariants_and_redundancy() {
    // sum over i < n, j < m of (i + a * b) + (b * a), with `a * b` computed
    // twice per iteration.
    let text = "\
function @f(i32, i32, i32, i32) -> i32 external {
block0(v0: i32, v1: i32, v2: i32, v3: i32):
    v4 = iconst.i32 0
    jump block1(v4, v4)
block1(v5: i32, v6: i32):
    v7 = icmp.i32 slt v5, v0
    brif v7, block2(v4, v6), block4
block2(v8: i32, v9: i32):
    v10 = imul.i32 nsw v2, v3
    v11 = iadd.i32 nsw v5, v10
    v12 = imul.i32 v3, v2
    v13 = iadd.i32 v11, v12
    v14 = iadd.i32 v9, v13
    v15 = iconst.i32 1
    v16 = iadd.i32 nsw v8, v15
    v17 = icmp.i32 slt v16, v1
    brif v17, block2(v16, v14), block3
block3:
    v18 = iconst.i32 1
    v19 = iadd.i32 nsw v5, v18
    jump block1(v19, v14)
block4:
    return v6
}
";
    assert_equivalent(
        text,
        "f",
        &[&[0, 0, 1, 2], &[3, 4, 5, 6], &[7, 1, -3, 9], &[2, 5, 0, 0]],
    );
}

#[test]
fn diamonds_with_flags_and_swapped_operands() {
    let text = "\
function @f(i32, i32, i32, i32) -> i32 external {
block0(v0: i32, v1: i32, v2: i32, v3: i32):
    v4 = iadd.i32 nsw v0, v1
    v5 = icmp.i32 slt v2, v3
    brif v5, block1, block2
block1:
    v6 = iadd.i32 v1, v0
    v7 = icmp.i32 sgt v3, v2
    v8 = zext.i32 v7
    v9 = iadd.i32 v6, v8
    jump block3(v9)
block2:
    v10 = shl.i32 nuw v0, v2
    v11 = shl.i32 v0, v2
    v12 = isub.i32 v10, v11
    jump block3(v12)
block3(v13: i32):
    v14 = iadd.i32 v0, v1
    v15 = imul.i32 v14, v13
    v16 = iadd.i32 v15, v4
    return v16
}
";
    assert_equivalent(
        text,
        "f",
        &[
            &[1, 2, 3, 4],
            &[2_147_483_647, 1, 0, 1],
            &[5, -7, 9, 2],
            &[-1, 3, 31, 0],
        ],
    );
}

#[test]
fn a_loop_that_never_runs_keeps_its_traps_and_poison() {
    // The header tests first, so with `n = 0` the body never runs: the
    // division by `d` (0 in the first two inputs) must not move out, and the
    // overflowing `nsw` sum that does move is poison nobody uses.
    let text = "\
function @f(i32, i32, i32) -> i32 external {
block0(v0: i32, v1: i32, v2: i32):
    v3 = iconst.i32 0
    jump block1(v3, v3)
block1(v4: i32, v5: i32):
    v6 = icmp.i32 slt v4, v0
    brif v6, block2, block3(v5)
block2:
    v7 = iconst.i32 2147483647
    v8 = iadd.i32 nsw v7, v2
    v9 = sdiv.i32 v8, v1
    v10 = iadd.i32 v5, v9
    v11 = iconst.i32 1
    v12 = iadd.i32 v4, v11
    jump block1(v12, v10)
block3(v13: i32):
    return v13
}
";
    assert_equivalent(
        text,
        "f",
        &[&[0, 0, 5], &[-3, 0, 1], &[4, 3, -10], &[1, -1, -1]],
    );
}

#[test]
fn loads_after_stores_in_a_loop_are_not_moved() {
    // Each iteration doubles the slot through memory; the load must stay in
    // the loop even though its address is invariant.
    let text = "\
function @f(i32, i32) -> i32 external {
    slot0 = stack_slot 4, align 4
block0(v0: i32, v1: i32):
    v2 = stack_addr slot0
    store.i32 v1, v2, align 4
    v3 = iconst.i32 0
    jump block1(v3)
block1(v4: i32):
    v5 = stack_addr slot0
    v6 = load.i32 v5, align 4
    v7 = iadd.i32 v6, v6
    store.i32 v7, v5, align 4
    v8 = iconst.i32 1
    v9 = iadd.i32 v4, v8
    v10 = icmp.i32 slt v9, v0
    brif v10, block1(v9), block2
block2:
    v11 = load.i32 v2, align 4
    return v11
}
";
    assert_equivalent(text, "f", &[&[1, 3], &[5, 1], &[10, -2]]);
}

#[test]
fn a_loop_entered_twice_with_two_back_edges() {
    let text = "\
function @f(i32, i32, i32) -> i32 external {
block0(v0: i32, v1: i32, v2: i32):
    v3 = iconst.i32 0
    v4 = icmp.i32 slt v2, v3
    brif v4, block1, block2
block1:
    v5 = iconst.i32 100
    jump block3(v5, v3)
block2:
    jump block3(v3, v3)
block3(v6: i32, v7: i32):
    v8 = imul.i32 v1, v1
    v9 = iadd.i32 v6, v8
    v10 = iconst.i32 1
    v11 = iadd.i32 v7, v10
    v12 = and.i32 v11, v10
    v13 = icmp.i32 eq v12, v3
    brif v13, block4, block5
block4:
    v14 = imul.i32 v1, v1
    v15 = isub.i32 v9, v14
    jump block3(v15, v11)
block5:
    v16 = icmp.i32 slt v11, v0
    brif v16, block3(v9, v11), block6
block6:
    return v9
}
";
    assert_equivalent(
        text,
        "f",
        &[&[0, 2, 1], &[5, 3, -1], &[8, -4, 0], &[3, 0, 7]],
    );
}

#[test]
fn a_wrapping_sum_survives_its_flagged_twin() {
    // `v2` overflows to poison but is never used; `v4` must still wrap after
    // it is merged into `v2`.
    let text = "\
function @f(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = iadd.i32 nsw v0, v1
    v3 = shl.i32 nuw v0, v1
    v4 = iadd.i32 v1, v0
    v5 = shl.i32 v0, v1
    v6 = xor.i32 v4, v5
    return v6
}
";
    assert_equivalent(text, "f", &[&[2_147_483_647, 1], &[-1, 4], &[3, 4]]);
}

#[test]
fn bytecode_style_dispatch_reuses_loads_without_crossing_the_back_edge() {
    // Dispatch repeatedly reads the opcode, then writes the next opcode.
    // The header must reload on each iteration, while the if-chain can reuse
    // that iteration's first load.
    let text = "\
function @f(i32, i32) -> i32 external {
    slot0 = stack_slot 4, align 4
block0(v0: i32, v1: i32):
    v2 = stack_addr slot0
    v3 = iconst.i32 0
    v4 = iconst.i32 1
    v5 = iconst.i32 2
    v6 = iconst.i32 3
    v7 = and.i32 v1, v6
    store.i32 v7, v2, align 4
    jump block1(v3, v3)
block1(v8: i32, v9: i32):
    v10 = icmp.i32 slt v8, v0
    brif v10, block2, block7
block2:
    v11 = load.i32 v2, align 4
    v12 = icmp.i32 eq v11, v3
    brif v12, block6(v4), block3
block3:
    v13 = load.i32 v2, align 4
    v14 = icmp.i32 eq v13, v4
    brif v14, block6(v5), block4
block4:
    v15 = load.i32 v2, align 4
    v16 = icmp.i32 eq v15, v5
    brif v16, block6(v6), block5
block5:
    v17 = load.i32 v2, align 4
    v18 = iadd.i32 v17, v4
    jump block6(v18)
block6(v19: i32):
    v20 = iadd.i32 v9, v19
    v21 = iadd.i32 v11, v4
    v22 = and.i32 v21, v6
    store.i32 v22, v2, align 4
    v23 = iadd.i32 v8, v4
    jump block1(v23, v20)
block7:
    return v9
}
";
    let (optimized, _) = optimize_text(text, &only(&[Pass::Gvn]));
    assert_eq!(optimized.matches("load.i32").count(), 1);
    assert_equivalent(
        text,
        "f",
        &[&[0, 0], &[1, 3], &[20, 0], &[31, 2], &[100, -1]],
    );
}

#[test]
fn sorts_style_merge_diamond_reuses_comparison_loads() {
    // Each iteration selects the smaller of two runs' heads into a temporary
    // slot and increments that head. Arm loads are redundant with the
    // comparison; stores in the join must prevent reuse on the next trip.
    let text = "\
function @f(i32, i32, i32) -> i32 external {
    slot0 = stack_slot 4, align 4
    slot1 = stack_slot 4, align 4
    slot2 = stack_slot 4, align 4
block0(v0: i32, v1: i32, v2: i32):
    v3 = stack_addr slot0
    v4 = stack_addr slot1
    v5 = stack_addr slot2
    store.i32 v1, v3, align 4
    store.i32 v2, v4, align 4
    v6 = iconst.i32 0
    v7 = iconst.i32 1
    jump block1(v6, v6)
block1(v8: i32, v9: i32):
    v10 = icmp.i32 slt v8, v0
    brif v10, block2, block6
block2:
    v11 = load.i32 v3, align 4
    v12 = load.i32 v4, align 4
    v13 = icmp.i32 slt v12, v11
    brif v13, block3, block4
block3:
    v14 = load.i32 v4, align 4
    v15 = iadd.i32 v14, v7
    jump block5(v14, v11, v15)
block4:
    v16 = load.i32 v3, align 4
    v17 = iadd.i32 v16, v7
    jump block5(v16, v17, v12)
block5(v18: i32, v19: i32, v20: i32):
    store.i32 v18, v5, align 4
    v21 = load.i32 v5, align 4
    store.i32 v19, v3, align 4
    store.i32 v20, v4, align 4
    v22 = iadd.i32 v9, v21
    v23 = iadd.i32 v8, v7
    jump block1(v23, v22)
block6:
    return v9
}
";
    let (optimized, _) = optimize_text(text, &only(&[Pass::Gvn]));
    assert_eq!(optimized.matches("load.i32").count(), 2);
    assert_equivalent(
        text,
        "f",
        &[&[0, 1, 2], &[1, 5, -2], &[20, 1, 7], &[31, 2, 2]],
    );
}

#[test]
fn a_store_through_an_escaped_slot_alias_changes_the_following_load() {
    let text = "\
function @f(i32, i32) -> i32 external {
    slot0 = stack_slot 4, align 4
    slot1 = stack_slot 4, align 4
block0(v0: i32, v1: i32):
    v2 = stack_addr slot0
    v3 = stack_addr slot1
    store.i32 v1, v2, align 4
    store.i32 v0, v3, align 4
    v4 = iconst.i32 0
    v5 = icmp.i32 eq v0, v4
    v6 = select.ptr v5, v2, v3
    v7 = load.i32 v2, align 4
    v8 = iadd.i32 v7, v1
    store.i32 v8, v6, align 4
    v9 = load.i32 v2, align 4
    return v9
}
";
    let (optimized, _) = optimize_text(text, &only(&[Pass::Gvn]));
    assert_eq!(optimized.matches("load.i32").count(), 1);
    assert_equivalent(text, "f", &[&[0, 5], &[1, 5], &[0, -7], &[-1, 99]]);
}
