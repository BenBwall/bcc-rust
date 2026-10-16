//! `simplify-cfg`: unreachable blocks, merging, jump threading and identical
//! branches.

use super::*;

#[test]
fn switch_threading_preserves_duplicate_arguments_at_every_bisect_prefix() {
    use crate::backend::interpreter::RuntimeValue;
    let input = "function @f(i32, i32, i32) -> i32 external {
block0(v0: i32, v1: i32, v2: i32):
    switch v0, block1(v1), [1: block1(v1), 2: block2(v2), 3: block3(v1)]
block1(v3: i32):
    jump block4(v3)
block2(v4: i32):
    jump block4(v4)
block3(v5: i32):
    jump block2(v5)
block4(v6: i32):
    return v6
}
";
    let (_, report) = optimize_text(input, &only(&[Pass::SimplifyCfg]));
    for limit in 0..=report.transformations() {
        let options = OptimizerOptions {
            bisect_limit: Some(limit),
            ..only(&[Pass::SimplifyCfg])
        };
        let (optimized, _) = optimize_text(input, &options);
        for selector in 0..=4 {
            let actual = equivalence::returned(
                &optimized,
                "f",
                &[
                    RuntimeValue::Int(selector),
                    RuntimeValue::Int(11),
                    RuntimeValue::Int(29),
                ],
            );
            assert_eq!(
                actual,
                RuntimeValue::Int(if selector == 2 { 29 } else { 11 }),
                "limit {limit}, selector {selector}"
            );
        }
    }
}

#[test]
fn twenty_thousand_switch_edges_optimize_quickly() {
    use std::{
        fmt::Write,
        sync::mpsc,
        thread,
        time::Duration,
    };
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        let mut input = String::from(
            "function @f(i32) -> i32 external {\nblock0(v0: i32):\n    switch v0, block1(v0), [",
        );
        for case in 1..=20_000 {
            if case != 1 {
                input.push_str(", ");
            }
            write!(input, "{case}: block{}(v0)", case + 1).unwrap();
        }
        input.push_str("]\n");
        for block in 1..=20_001 {
            writeln!(
                input,
                "block{block}(v{block}: i32):\n    jump block20002(v{block})"
            )
            .unwrap();
        }
        input.push_str("block20002(v20002: i32):\n    return v20002\n}\n");
        let (optimized, report) = optimize_text(&input, &only(&[Pass::SimplifyCfg]));
        assert_eq!(
            optimized,
            "function @f(i32) -> i32 external {\nblock0(v0: i32):\n    return v0\n}\n"
        );
        for value in [0, 1, 19_999, 20_000, 20_001] {
            let argument = crate::backend::interpreter::RuntimeValue::Int(value);
            assert_eq!(
                equivalence::returned(&optimized, "f", &[argument]),
                argument
            );
        }
        eprintln!(
            "20,000 switch edges: {:?}",
            report.stats(Pass::SimplifyCfg).time
        );
        sender.send(()).unwrap();
    });
    receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("20,000 switch edges must optimize within ten seconds");
    worker.join().unwrap();
}

#[test]
fn twenty_thousand_forwarding_blocks_optimize_quickly() {
    use std::{
        fmt::Write,
        sync::mpsc,
        thread,
        time::Duration,
    };

    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        for backwards in [false, true] {
            let mut input = String::from(
                "function @f(i32) -> i32 external {\nblock0(v0: i32):\n    jump block1(v0)\n",
            );
            for position in 1..=20_000 {
                let block = if backwards {
                    20_001 - position
                } else {
                    position
                };
                writeln!(
                    input,
                    "block{block}(v{block}: i32):\n    jump block{}(v{block})",
                    block + 1
                )
                .unwrap();
            }
            input.push_str("block20001(v20001: i32):\n    return v20001\n}\n");
            let (optimized, report) = optimize_text(&input, &only(&[Pass::SimplifyCfg]));
            assert_eq!(
                optimized,
                "function @f(i32) -> i32 external {\nblock0(v0: i32):\n    return v0\n}\n"
            );
            assert_eq!(
                equivalence::returned(
                    &optimized,
                    "f",
                    &[crate::backend::interpreter::RuntimeValue::Int(9)]
                ),
                crate::backend::interpreter::RuntimeValue::Int(9),
            );
            eprintln!(
                "20,000 forwarding blocks, backwards={backwards}: {:?}",
                report.stats(Pass::SimplifyCfg).time
            );
        }
        sender.send(()).unwrap();
    });
    // A detached worker on failure lets the test process exit instead of
    // spending minutes finishing a regressed traversal. Include verification
    // in the generous deadline so this exercises the optimizer's public seam.
    receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("20,000-block chains must optimize within ten seconds");
    worker.join().unwrap();
}

#[test]
fn a_forwarding_cycle_with_permuted_arguments_is_left_alone() {
    let input = "function @f(i32, i32, i1) internal {
block0(v0: i32, v1: i32, v2: i1):
    brif v2, block1(v0, v1), block2(v1, v0)
block1(v3: i32, v4: i32):
    jump block2(v4, v3)
block2(v5: i32, v6: i32):
    jump block1(v6, v5)
}
";
    assert_pass_leaves(Pass::SimplifyCfg, input);
}

#[test]
fn threading_bisect_prefixes_keep_argument_mappings_valid() {
    use crate::backend::interpreter::RuntimeValue;

    let input = "function @f(i32, i32, i1) -> i32 external {
block0(v0: i32, v1: i32, v2: i1):
    brif v2, block1(v0, v1), block3(v0, v1)
block1(v3: i32, v4: i32):
    jump block2(v4, v3)
block2(v5: i32, v6: i32):
    jump block4(v6, v5)
block3(v7: i32, v8: i32):
    jump block4(v8, v7)
block4(v9: i32, v10: i32):
    v11 = isub.i32 v9, v10
    return v11
}
";
    let options = only(&[Pass::SimplifyCfg]);
    let (_, full) = optimize_text(input, &options);
    for limit in 0..=full.transformations() {
        let (optimized, report) = optimize_text(
            input,
            &OptimizerOptions {
                bisect_limit: Some(limit),
                ..options
            },
        );
        assert_eq!(report.transformations(), limit);
        for condition in [0, 1] {
            let args = [
                RuntimeValue::Int(10),
                RuntimeValue::Int(1),
                RuntimeValue::Int(condition),
            ];
            assert_eq!(
                equivalence::returned(&optimized, "f", &args),
                equivalence::returned(input, "f", &args),
                "bisect limit {limit}"
            );
        }
    }
}

#[test]
fn unreachable_blocks_are_removed() {
    let input = "\
function @f() -> i32 external {
block0:
    v0 = iconst.i32 1
    return v0
block1:
    v1 = iconst.i32 2
    jump block2(v1)
block2(v2: i32):
    return v2
}
";
    let expected = "\
function @f() -> i32 external {
block0:
    v0 = iconst.i32 1
    return v0
}
";
    assert_pass(Pass::SimplifyCfg, input, expected);
}

#[test]
fn unreachable_loops_and_chains_are_removed_together() {
    let input = "\
function @f() internal {
block0:
    return
block1:
    jump block2
block2:
    jump block1
block3:
    unreachable
}
";
    let expected = "\
function @f() internal {
block0:
    return
}
";
    assert_pass(Pass::SimplifyCfg, input, expected);
}

#[test]
fn a_chain_of_jumps_merges_into_one_block() {
    let input = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iadd.i32 v0, v0
    jump block1(v1)
block1(v2: i32):
    v3 = imul.i32 v2, v2
    jump block2(v3, v0)
block2(v4: i32, v5: i32):
    v6 = isub.i32 v4, v5
    return v6
}
";
    let expected = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iadd.i32 v0, v0
    v2 = imul.i32 v1, v1
    v3 = isub.i32 v2, v0
    return v3
}
";
    assert_pass(Pass::SimplifyCfg, input, expected);
}

#[test]
fn a_chain_laid_out_backwards_merges_too() {
    let input = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    jump block2(v0)
block1(v1: i32):
    v2 = iadd.i32 v1, v1
    return v2
block2(v3: i32):
    v4 = isub.i32 v3, v0
    jump block1(v4)
}
";
    let expected = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = isub.i32 v0, v0
    v2 = iadd.i32 v1, v1
    return v2
}
";
    assert_pass(Pass::SimplifyCfg, input, expected);
}

#[test]
fn a_two_predecessor_join_becomes_a_select() {
    let input = "\
function @f(i1) -> i32 external {
block0(v0: i1):
    v1 = iconst.i32 1
    v2 = iconst.i32 2
    brif v0, block1, block2
block1:
    v3 = iadd.i32 v1, v2
    jump block3(v3)
block2:
    v4 = isub.i32 v1, v2
    jump block3(v4)
block3(v5: i32):
    return v5
}
";
    let expected = "function @f(i1) -> i32 external {
block0(v0: i1):
    v1 = iconst.i32 1
    v2 = iconst.i32 2
    v3 = iadd.i32 v1, v2
    v4 = isub.i32 v1, v2
    v5 = select.i32 v0, v3, v4
    return v5
}
";
    assert_pass(Pass::SimplifyCfg, input, expected);
}

#[test]
fn a_loop_keeps_its_header_and_body() {
    assert_pass_leaves(Pass::SimplifyCfg, tests_text::COUNT_LOOP);
}

/// A loop from `middle-end.md`.
mod tests_text {
    pub(super) const COUNT_LOOP: &str = "\
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
}

#[test]
fn jumps_thread_through_forwarding_blocks() {
    let input = "\
function @f(i32, i1) -> i32 external {
block0(v0: i32, v1: i1):
    brif v1, block1(v0), block2
block1(v2: i32):
    jump block3(v2)
block2:
    jump block3(v0)
block3(v3: i32):
    return v3
}
";
    let expected = "\
function @f(i32, i1) -> i32 external {
block0(v0: i32, v1: i1):
    return v0
}
";
    assert_pass(Pass::SimplifyCfg, input, expected);
}

#[test]
fn selects_resolve_edges_that_threading_cannot_merge() {
    // Both forwarding blocks lead to block3, with different arguments, so at
    // most one edge of the `brif` may go there directly.
    let input = "\
function @f(i32, i1) -> i32 external {
block0(v0: i32, v1: i1):
    v2 = iconst.i32 9
    brif v1, block1(v0), block2
block1(v3: i32):
    jump block3(v3)
block2:
    jump block3(v2)
block3(v4: i32):
    return v4
}
";
    let expected = "function @f(i32, i1) -> i32 external {
block0(v0: i32, v1: i1):
    v2 = iconst.i32 9
    v3 = select.i32 v1, v0, v2
    return v3
}
";
    assert_pass(Pass::SimplifyCfg, input, expected);
}

#[test]
fn threading_maps_permuted_parameters() {
    let input = "\
function @f(i32, i32, i1) -> i32 external {
block0(v0: i32, v1: i32, v2: i1):
    brif v2, block1(v0, v1), block2(v1, v0)
block1(v3: i32, v4: i32):
    jump block2(v4, v3)
block2(v5: i32, v6: i32):
    v7 = isub.i32 v5, v6
    return v7
}
";
    let expected = "\
function @f(i32, i32, i1) -> i32 external {
block0(v0: i32, v1: i32, v2: i1):
    v3 = isub.i32 v1, v0
    return v3
}
";
    assert_pass(Pass::SimplifyCfg, input, expected);
}

#[test]
fn threading_passes_values_defined_above_the_forwarding_block() {
    // `block1` forwards `v2`, which is defined in the entry and so dominates
    // every predecessor of `block1`. `block3` forwards a different argument
    // to the same block, so only one of the two edges can be redirected.
    let input = "function @f(i32, i1) -> i32 external {
block0(v0: i32, v1: i1):
    v2 = iconst.i32 5
    brif v1, block1, block3
block1:
    jump block2(v2)
block2(v3: i32):
    v4 = iadd.i32 v3, v3
    return v4
block3:
    jump block2(v0)
}
";
    let expected = "function @f(i32, i1) -> i32 external {
block0(v0: i32, v1: i1):
    v2 = iconst.i32 5
    v3 = select.i32 v1, v2, v0
    v4 = iadd.i32 v3, v3
    return v4
}
";
    assert_pass(Pass::SimplifyCfg, input, expected);
}

#[test]
fn a_cycle_of_forwarding_blocks_is_left_alone() {
    let input = "\
function @f(i1) internal {
block0(v0: i1):
    brif v0, block1, block2
block1:
    jump block2
block2:
    jump block1
}
";
    assert_pass_leaves(Pass::SimplifyCfg, input);
}

#[test]
fn a_branch_to_one_block_becomes_a_jump() {
    let input = "\
function @f(i32, i1) -> i32 external {
block0(v0: i32, v1: i1):
    v2 = iadd.i32 v0, v0
    brif v1, block1(v2), block1(v2)
block1(v3: i32):
    v4 = imul.i32 v3, v3
    return v4
}
";
    let expected = "\
function @f(i32, i1) -> i32 external {
block0(v0: i32, v1: i1):
    v2 = iadd.i32 v0, v0
    v3 = imul.i32 v2, v2
    return v3
}
";
    assert_pass(Pass::SimplifyCfg, input, expected);
}

#[test]
fn a_switch_whose_edges_all_agree_becomes_a_jump() {
    let input = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    switch v0, block1(v0), [1: block1(v0), 2: block1(v0)]
block1(v1: i32):
    return v1
}
";
    let expected = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    return v0
}
";
    assert_pass(Pass::SimplifyCfg, input, expected);
    let empty = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    switch v0, block1(v0), []
block1(v1: i32):
    return v1
}
";
    assert_pass(Pass::SimplifyCfg, empty, expected);
}

#[test]
fn a_switch_with_different_edges_stays() {
    let input = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    switch v0, block1, [1: block2]
block1:
    return v0
block2:
    v1 = iadd.i32 v0, v0
    return v1
}
";
    assert_pass_leaves(Pass::SimplifyCfg, input);
}

#[test]
fn the_entry_block_absorbs_its_successor() {
    let input = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    jump block1
block1:
    return v0
}
";
    let expected = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    return v0
}
";
    assert_pass(Pass::SimplifyCfg, input, expected);
}

#[test]
fn removing_unreachable_blocks_counts_as_one_rewrite() {
    let input = "\
function @f() internal {
block0:
    return
block1:
    jump block2
block2:
    return
block3:
    return
}
";
    let (_, report) = optimize_text(input, &only(&[Pass::SimplifyCfg]));
    assert_eq!(report.transformations(), 1);
    let options = OptimizerOptions {
        bisect_limit: Some(0),
        ..only(&[Pass::SimplifyCfg])
    };
    let (text, report) = optimize_text(input, &options);
    assert_eq!(text, input);
    assert_eq!(report.transformations(), 0);
    assert!(report.limit_reached());
}

/// Found by the differential tests (seed 0xbcc0d217): `block3` forwards to
/// `block4`, which uses its parameter, so an edge may be threaded to
/// `block3` but not past it.
#[test]
fn a_forwarding_parameter_used_later_is_replaced_by_a_select() {
    let input = "\
function @f(i32, i1) -> i32 external {
block0(v0: i32, v1: i1):
    brif v1, block1, block2
block1:
    jump block3(v0)
block2:
    v2 = iconst.i32 7
    jump block3(v2)
block3(v3: i32):
    jump block4
block4:
    return v3
}
";
    let expected = "function @f(i32, i1) -> i32 external {
block0(v0: i32, v1: i1):
    v2 = iconst.i32 7
    v3 = select.i32 v1, v0, v2
    return v3
}
";
    assert_pass(Pass::SimplifyCfg, input, expected);
}
