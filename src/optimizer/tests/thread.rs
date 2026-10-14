//! `simplify-cfg`'s branch threading: constant conditions skip the block
//! that tests them, and a second test of a shared destination merges with
//! the first.

use super::*;

/// Runs `simplify-cfg` then `dce` and checks the printed result.
fn assert_threaded(input: &str, expected: &str) {
    let (actual, _) = optimize_text(input, &only(&[Pass::SimplifyCfg, Pass::Dce]));
    pretty_assertions::assert_eq!(actual, expected);
}

#[test]
fn a_decided_or_join_branches_straight_to_its_successor() {
    // `if (a < 0 || a > b)`: the first test's true edge passes `true` to the
    // join, which then needs no test on that path; the merged conditions
    // leave one branch on one `select`.
    let input = "\
function @f(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = iconst.i32 0
    v3 = icmp.i32 slt v0, v2
    v4 = iconst.i1 1
    brif v3, block2(v4), block1
block1:
    v5 = icmp.i32 sgt v0, v1
    jump block2(v5)
block2(v6: i1):
    brif v6, block3, block4
block3:
    return v2
block4:
    return v0
}
";
    let expected = "\
function @f(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = iconst.i1 1
    v3 = iconst.i32 0
    v4 = icmp.i32 slt v0, v3
    v5 = icmp.i32 sgt v0, v1
    v6 = select.i1 v4, v2, v5
    brif v6, block1, block2
block1:
    return v3
block2:
    return v0
}
";
    assert_threaded(input, expected);
}

#[test]
fn an_and_of_two_tests_for_one_failure_edge_merges() {
    let input = "\
function @f(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = iconst.i32 0
    v3 = icmp.i32 sgt v0, v2
    brif v3, block1, block3(v2)
block1:
    v4 = icmp.i32 slt v0, v1
    brif v4, block2, block3(v2)
block2:
    return v1
block3(v5: i32):
    return v5
}
";
    let expected = "\
function @f(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = iconst.i1 0
    v3 = iconst.i32 0
    v4 = icmp.i32 sgt v0, v3
    v5 = icmp.i32 slt v0, v1
    v6 = select.i1 v4, v5, v2
    brif v6, block1, block2(v3)
block1:
    return v1
block2(v7: i32):
    return v7
}
";
    assert_threaded(input, expected);
}

#[test]
fn an_arm_that_loads_does_not_merge() {
    let input = "\
function @f(i32, ptr) -> i32 external {
block0(v0: i32, v1: ptr):
    v2 = iconst.i32 0
    v3 = icmp.i32 sgt v0, v2
    brif v3, block1, block3
block1:
    v4 = load.i32 v1, align 4
    v5 = icmp.i32 slt v0, v4
    brif v5, block2, block3
block2:
    return v0
block3:
    return v2
}
";
    assert_pass_leaves(Pass::SimplifyCfg, input);
}

#[test]
fn a_loop_header_is_not_threaded() {
    // block1 branches on its parameter, but it heads the loop through
    // block2: threading block2's constant edge would enter the loop past its
    // header.
    let input = "\
function @f(i1, i32) -> i32 external {
block0(v0: i1, v1: i32):
    jump block1(v0)
block1(v2: i1):
    brif v2, block2, block3
block2:
    v3 = iconst.i1 0
    call @g()
    jump block1(v3)
block3:
    return v1
}

function @g() external
";
    assert_pass_leaves(Pass::SimplifyCfg, input);
}

#[test]
fn a_join_whose_parameter_is_used_below_is_not_threaded() {
    // block3 uses block2's parameter; a path from block0 straight to block3
    // would not define it. The load keeps block1 from becoming a select.
    // Found by the differential sweep.
    let input = "function @f(i1, ptr) -> i32 external {
block0(v0: i1, v1: ptr):
    v2 = iconst.i1 1
    brif v0, block2(v2), block1
block1:
    v3 = load.i1 v1, align 1
    jump block2(v3)
block2(v4: i1):
    brif v4, block3, block4
block3:
    v5 = zext.i32 v4
    return v5
block4:
    v6 = iconst.i32 7
    return v6
}
";
    assert_pass_leaves(Pass::SimplifyCfg, input);
}
