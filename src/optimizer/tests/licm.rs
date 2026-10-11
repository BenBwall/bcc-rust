//! `licm`: preheaders and the instructions that may leave a loop.

use super::*;

#[test]
fn invariants_move_to_a_jumping_predecessor() {
    // The header has block parameters; block0 is its only entry and ends in
    // a jump, so it is the preheader. The chain `v4`, `v5` moves together.
    let input = "\
function @f(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = iconst.i32 0
    jump block1(v2, v2)
block1(v3: i32, v4: i32):
    v5 = imul.i32 nsw v0, v1
    v6 = iadd.i32 nsw v5, v0
    v7 = iadd.i32 v4, v6
    v8 = iadd.i32 v3, v1
    v9 = icmp.i32 slt v8, v0
    brif v9, block1(v8, v7), block2
block2:
    return v4
}
";
    let expected = "\
function @f(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = iconst.i32 0
    v3 = imul.i32 nsw v0, v1
    v4 = iadd.i32 nsw v3, v0
    jump block1(v2, v2)
block1(v5: i32, v6: i32):
    v7 = iadd.i32 v6, v4
    v8 = iadd.i32 v5, v1
    v9 = icmp.i32 slt v8, v0
    brif v9, block1(v8, v7), block2
block2:
    return v6
}
";
    assert_pass(Pass::Licm, input, expected);
}

#[test]
fn a_branching_entry_gets_a_preheader_that_forwards_the_parameters() {
    let input = "\
function @f(i32, i32, i1) -> i32 external {
block0(v0: i32, v1: i32, v2: i1):
    v3 = iconst.i32 0
    brif v2, block1(v3), block2(v3)
block1(v4: i32):
    v5 = imul.i32 v0, v1
    v6 = iadd.i32 v4, v5
    v7 = icmp.i32 slt v6, v1
    brif v7, block1(v6), block2(v6)
block2(v8: i32):
    return v8
}
";
    let expected = "\
function @f(i32, i32, i1) -> i32 external {
block0(v0: i32, v1: i32, v2: i1):
    v3 = iconst.i32 0
    brif v2, block3(v3), block2(v3)
block1(v4: i32):
    v5 = iadd.i32 v4, v9
    v6 = icmp.i32 slt v5, v1
    brif v6, block1(v5), block2(v5)
block2(v7: i32):
    return v7
block3(v8: i32):
    v9 = imul.i32 v0, v1
    jump block1(v8)
}
";
    assert_pass(Pass::Licm, input, expected);
}

#[test]
fn several_entries_share_one_preheader() {
    let input = "\
function @f(i32, i32, i1) -> i32 external {
block0(v0: i32, v1: i32, v2: i1):
    brif v2, block1, block2
block1:
    v3 = iconst.i32 1
    jump block3(v3)
block2:
    v4 = iconst.i32 2
    jump block3(v4)
block3(v5: i32):
    v6 = isub.i32 v0, v1
    v7 = iadd.i32 v5, v6
    v8 = icmp.i32 slt v7, v1
    brif v8, block3(v7), block4
block4:
    return v7
}
";
    let expected = "\
function @f(i32, i32, i1) -> i32 external {
block0(v0: i32, v1: i32, v2: i1):
    brif v2, block1, block2
block1:
    v3 = iconst.i32 1
    jump block5(v3)
block2:
    v4 = iconst.i32 2
    jump block5(v4)
block3(v5: i32):
    v6 = iadd.i32 v5, v9
    v7 = icmp.i32 slt v6, v1
    brif v7, block3(v6), block4
block4:
    return v6
block5(v8: i32):
    v9 = isub.i32 v0, v1
    jump block3(v8)
}
";
    assert_pass(Pass::Licm, input, expected);
}

#[test]
fn a_loop_with_two_back_edges_hoists_from_both_paths() {
    let input = "\
function @f(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = iconst.i32 0
    jump block1(v2)
block1(v3: i32):
    v4 = iadd.i32 v0, v1
    v5 = iadd.i32 v3, v4
    v6 = icmp.i32 slt v5, v1
    brif v6, block2, block4
block2:
    v7 = and.i32 v5, v0
    v8 = icmp.i32 eq v7, v2
    brif v8, block1(v5), block3
block3:
    v9 = isub.i32 v0, v1
    v10 = iadd.i32 v5, v9
    jump block1(v10)
block4:
    return v3
}
";
    let expected = "\
function @f(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = iconst.i32 0
    v3 = iadd.i32 v0, v1
    v4 = isub.i32 v0, v1
    jump block1(v2)
block1(v5: i32):
    v6 = iadd.i32 v5, v3
    v7 = icmp.i32 slt v6, v1
    brif v7, block2, block4
block2:
    v8 = and.i32 v6, v0
    v9 = icmp.i32 eq v8, v2
    brif v9, block1(v6), block3
block3:
    v10 = iadd.i32 v6, v4
    jump block1(v10)
block4:
    return v5
}
";
    assert_pass(Pass::Licm, input, expected);
}

/// An outer loop over `i` around an inner loop over `j`.
const NESTED: &str = "\
function @f(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = iconst.i32 0
    jump block1(v2, v2)
block1(v3: i32, v4: i32):
    v5 = icmp.i32 slt v3, v0
    brif v5, block2(v2, v4), block4
block2(v6: i32, v7: i32):
    v8 = imul.i32 v0, v1
    v9 = iadd.i32 v3, v8
    v10 = iadd.i32 v7, v9
    v11 = iconst.i32 1
    v12 = iadd.i32 v6, v11
    v13 = icmp.i32 slt v12, v1
    brif v13, block2(v12, v10), block3
block3:
    v14 = iconst.i32 1
    v15 = iadd.i32 v3, v14
    jump block1(v15, v10)
block4:
    return v4
}
";

#[test]
fn nested_loops_hoist_each_invariant_as_far_as_it_goes() {
    // `v0 * v1` leaves both loops; `i + v0 * v1` only the inner one, so it
    // stays in the inner loop's new preheader, block5.
    let expected = "\
function @f(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = iconst.i32 0
    v3 = imul.i32 v0, v1
    v4 = iconst.i32 1
    v5 = iconst.i32 1
    jump block1(v2, v2)
block1(v6: i32, v7: i32):
    v8 = icmp.i32 slt v6, v0
    brif v8, block5(v2, v7), block4
block2(v9: i32, v10: i32):
    v11 = iadd.i32 v10, v17
    v12 = iadd.i32 v9, v4
    v13 = icmp.i32 slt v12, v1
    brif v13, block2(v12, v11), block3
block3:
    v14 = iadd.i32 v6, v5
    jump block1(v14, v11)
block4:
    return v7
block5(v15: i32, v16: i32):
    v17 = iadd.i32 v6, v3
    jump block2(v15, v16)
}
";
    assert_pass(Pass::Licm, NESTED, expected);
}

#[test]
fn trapping_and_effectful_instructions_stay() {
    // Division by a variable, by 0 or by -1 (signed) may trap, so it stays;
    // `udiv` by 7, `srem` by 3 and `urem` by -1 cannot, and move. Loads,
    // calls and `freeze` never move.
    let input = "\
function @g(i32) -> i32 external

function @f(i32, i32, ptr) -> i32 external {
block0(v0: i32, v1: i32, v2: ptr):
    v3 = iconst.i32 0
    v4 = iconst.i32 -1
    v5 = iconst.i32 7
    v6 = iconst.i32 3
    jump block1(v3)
block1(v7: i32):
    v8 = sdiv.i32 v0, v1
    v9 = udiv.i32 v0, v3
    v10 = sdiv.i32 v0, v4
    v11 = srem.i32 v0, v4
    v12 = udiv.i32 v0, v5
    v13 = srem.i32 v0, v6
    v14 = urem.i32 v0, v4
    v15 = load.i32 v2, align 4
    v16 = call @g(v0)
    v17 = freeze.i32 v0
    v18 = iadd.i32 v8, v9
    v19 = iadd.i32 v10, v11
    v20 = iadd.i32 v12, v13
    v21 = iadd.i32 v14, v15
    v22 = iadd.i32 v16, v17
    v23 = iadd.i32 v18, v19
    v24 = iadd.i32 v20, v21
    v25 = iadd.i32 v22, v23
    v26 = iadd.i32 v24, v25
    v27 = iadd.i32 v7, v26
    v28 = icmp.i32 slt v27, v1
    brif v28, block1(v27), block2
block2:
    return v27
}
";
    let expected = "\
function @g(i32) -> i32 external

function @f(i32, i32, ptr) -> i32 external {
block0(v0: i32, v1: i32, v2: ptr):
    v3 = iconst.i32 0
    v4 = iconst.i32 -1
    v5 = iconst.i32 7
    v6 = iconst.i32 3
    v7 = udiv.i32 v0, v5
    v8 = srem.i32 v0, v6
    v9 = urem.i32 v0, v4
    v10 = iadd.i32 v7, v8
    jump block1(v3)
block1(v11: i32):
    v12 = sdiv.i32 v0, v1
    v13 = udiv.i32 v0, v3
    v14 = sdiv.i32 v0, v4
    v15 = srem.i32 v0, v4
    v16 = load.i32 v2, align 4
    v17 = call @g(v0)
    v18 = freeze.i32 v0
    v19 = iadd.i32 v12, v13
    v20 = iadd.i32 v14, v15
    v21 = iadd.i32 v9, v16
    v22 = iadd.i32 v17, v18
    v23 = iadd.i32 v19, v20
    v24 = iadd.i32 v10, v21
    v25 = iadd.i32 v22, v23
    v26 = iadd.i32 v24, v25
    v27 = iadd.i32 v11, v26
    v28 = icmp.i32 slt v27, v1
    brif v28, block1(v27), block2
block2:
    return v27
}
";
    assert_pass(Pass::Licm, input, expected);
}

#[test]
fn code_without_invariants_is_left_alone() {
    let input = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 0
    v2 = iconst.i32 1
    jump block1(v1, v1)
block1(v3: i32, v4: i32):
    v5 = icmp.i32 slt v3, v0
    brif v5, block2, block3
block2:
    v6 = iadd.i32 nsw v4, v3
    v7 = iadd.i32 nsw v3, v2
    jump block1(v7, v6)
block3:
    return v4
}
";
    assert_pass_leaves(Pass::Licm, input);
}

#[test]
fn bisecting_moves_a_prefix_of_the_invariants() {
    let options = OptimizerOptions {
        bisect_limit: Some(1),
        ..only(&[Pass::Licm])
    };
    let (actual, report) = optimize_text(NESTED, &options);
    // The first move is `v0 * v1` into the inner loop's new preheader.
    let expected = "\
function @f(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = iconst.i32 0
    jump block1(v2, v2)
block1(v3: i32, v4: i32):
    v5 = icmp.i32 slt v3, v0
    brif v5, block5(v2, v4), block4
block2(v6: i32, v7: i32):
    v8 = iadd.i32 v3, v17
    v9 = iadd.i32 v7, v8
    v10 = iconst.i32 1
    v11 = iadd.i32 v6, v10
    v12 = icmp.i32 slt v11, v1
    brif v12, block2(v11, v9), block3
block3:
    v13 = iconst.i32 1
    v14 = iadd.i32 v3, v13
    jump block1(v14, v9)
block4:
    return v4
block5(v15: i32, v16: i32):
    v17 = imul.i32 v0, v1
    jump block2(v15, v16)
}
";
    pretty_assertions::assert_eq!(actual, expected);
    assert_eq!(report.transformations(), 1);
}
