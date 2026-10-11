//! `simplify-cfg`: unreachable blocks, merging, jump threading and identical
//! branches.

use super::*;

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
fn a_block_with_two_predecessors_is_not_merged() {
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
    assert_pass_leaves(Pass::SimplifyCfg, input);
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
fn threading_leaves_edges_that_would_conflict() {
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
    let expected = "\
function @f(i32, i1) -> i32 external {
block0(v0: i32, v1: i1):
    v2 = iconst.i32 9
    brif v1, block2(v0), block1
block1:
    jump block2(v2)
block2(v3: i32):
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
    brif v1, block1(v2), block2
block1(v3: i32):
    v4 = iadd.i32 v3, v3
    return v4
block2:
    jump block1(v0)
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
