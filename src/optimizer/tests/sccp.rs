//! `sccp`: constants found along executable edges, poison, and decided
//! branches.

use super::*;

/// Runs `sccp` and checks the printed result.
fn assert_sccp(input: &str, expected: &str) {
    assert_pass(Pass::Sccp, input, expected);
}

/// A loop that sets `x` to 2 only if `x != 1`, which never happens: `x` is 1
/// along every executable edge, though the dead arm would pass 2.
const LOOP_WITH_A_DEAD_ARM: &str = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 1
    v2 = iconst.i32 0
    jump block1(v1, v2)
block1(v3: i32, v4: i32):
    v5 = icmp.i32 slt v4, v0
    brif v5, block2, block5
block2:
    v6 = icmp.i32 ne v3, v1
    brif v6, block3, block4(v3)
block3:
    v7 = iconst.i32 2
    jump block4(v7)
block4(v8: i32):
    v9 = iadd.i32 v4, v1
    jump block1(v8, v9)
block5:
    return v3
}
";

#[test]
fn a_value_constant_along_executable_edges_is_found() {
    let expected = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 1
    v2 = iconst.i32 1
    v3 = iconst.i32 0
    jump block1(v3)
block1(v4: i32):
    v5 = icmp.i32 slt v4, v0
    brif v5, block2, block5
block2:
    v6 = iconst.i1 0
    jump block4
block3:
    v7 = iconst.i32 2
    jump block4
block4:
    v8 = iadd.i32 v4, v2
    jump block1(v8)
block5:
    return v1
}
";
    assert_sccp(LOOP_WITH_A_DEAD_ARM, expected);
}

#[test]
fn a_loop_that_never_changes_its_counter_is_decided() {
    let input = "\
function @f() -> i32 external {
block0:
    v0 = iconst.i32 0
    jump block1(v0)
block1(v1: i32):
    v2 = icmp.i32 eq v1, v0
    brif v2, block2, block3
block2:
    v3 = iadd.i32 v1, v0
    jump block1(v3)
block3:
    return v1
}
";
    let expected = "\
function @f() -> i32 external {
block0:
    v0 = iconst.i32 0
    v1 = iconst.i32 0
    jump block1
block1:
    v2 = iconst.i1 1
    jump block2
block2:
    v3 = iconst.i32 0
    jump block1
block3:
    return v0
}
";
    assert_sccp(input, expected);
}

#[test]
fn an_overdefined_parameter_stays() {
    // The count rises on every trip, so the header's parameter is not
    // constant, though both of its incoming values are.
    let input = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 0
    v2 = iconst.i32 1
    jump block1(v1)
block1(v3: i32):
    v4 = icmp.i32 slt v3, v0
    brif v4, block2, block3
block2:
    v5 = iadd.i32 v3, v2
    jump block1(v5)
block3:
    return v3
}
";
    assert_pass_leaves(Pass::Sccp, input);
}

#[test]
fn overflow_gives_poison_and_undefined_division_is_left_alone() {
    let input = "\
function @f() -> i32 external {
block0:
    v0 = iconst.i32 2147483647
    v1 = iconst.i32 1
    v2 = iadd.i32 nsw v0, v1
    v3 = iadd.i32 v0, v1
    v4 = iconst.i32 0
    v5 = sdiv.i32 v1, v4
    v6 = iconst.i32 -2147483648
    v7 = iconst.i32 -1
    v8 = sdiv.i32 v6, v7
    v9 = udiv.i32 v1, v2
    v10 = udiv.i32 v2, v1
    v11 = imul.i32 v2, v1
    v12 = shl.i32 v1, v0
    v13 = iadd.i32 v5, v8
    v14 = iadd.i32 v13, v9
    return v14
}
";
    let expected = "\
function @f() -> i32 external {
block0:
    v0 = iconst.i32 2147483647
    v1 = iconst.i32 1
    v2 = poison.i32
    v3 = iconst.i32 -2147483648
    v4 = iconst.i32 0
    v5 = sdiv.i32 v1, v4
    v6 = iconst.i32 -2147483648
    v7 = iconst.i32 -1
    v8 = sdiv.i32 v6, v7
    v9 = udiv.i32 v1, v2
    v10 = poison.i32
    v11 = poison.i32
    v12 = poison.i32
    v13 = iadd.i32 v5, v8
    v14 = iadd.i32 v13, v9
    return v14
}
";
    assert_sccp(input, expected);
}

#[test]
fn poison_on_one_edge_refines_to_the_constant_on_the_other() {
    let input = "\
function @f(i1) -> i32 external {
block0(v0: i1):
    v1 = poison.i32
    v2 = iconst.i32 7
    brif v0, block1, block2
block1:
    jump block3(v1)
block2:
    jump block3(v2)
block3(v3: i32):
    v4 = iadd.i32 v3, v2
    return v4
}
";
    let expected = "\
function @f(i1) -> i32 external {
block0(v0: i1):
    v1 = iconst.i32 7
    v2 = poison.i32
    v3 = iconst.i32 7
    brif v0, block1, block2
block1:
    jump block3
block2:
    jump block3
block3:
    v4 = iconst.i32 14
    return v4
}
";
    assert_sccp(input, expected);
}

#[test]
fn a_branch_on_poison_is_overdefined_and_stays() {
    let input = "\
function @f() -> i32 external {
block0:
    v0 = poison.i1
    brif v0, block1, block2
block1:
    v1 = iconst.i32 1
    jump block3(v1)
block2:
    v2 = iconst.i32 2
    jump block3(v2)
block3(v3: i32):
    return v3
}
";
    assert_pass_leaves(Pass::Sccp, input);
    // A condition that only becomes poison through a parameter is resolved
    // the same way, and both arms are still analysed.
    let through_a_param = "\
function @f() -> i32 external {
block0:
    v0 = poison.i32
    v1 = iconst.i32 0
    jump block1(v0)
block1(v2: i32):
    v3 = icmp.i32 eq v2, v1
    brif v3, block2, block3
block2:
    v4 = iconst.i32 1
    v5 = iadd.i32 v4, v4
    return v5
block3:
    v6 = iconst.i32 3
    v7 = iadd.i32 v6, v6
    return v7
}
";
    let expected = "\
function @f() -> i32 external {
block0:
    v0 = poison.i32
    v1 = poison.i32
    v2 = iconst.i32 0
    jump block1
block1:
    v3 = poison.i1
    brif v3, block2, block3
block2:
    v4 = iconst.i32 1
    v5 = iconst.i32 2
    return v5
block3:
    v6 = iconst.i32 3
    v7 = iconst.i32 6
    return v7
}
";
    assert_sccp(through_a_param, expected);
}

#[test]
fn a_switch_on_a_propagated_constant_becomes_a_jump() {
    let input = "\
function @f(i1) -> i32 external {
block0(v0: i1):
    v1 = iconst.i32 2
    brif v0, block1(v1), block2
block1(v2: i32):
    switch v2, block3, [1: block4, 2: block5]
block2:
    v3 = iconst.i32 2
    jump block1(v3)
block3:
    v4 = iconst.i32 30
    return v4
block4:
    v5 = iconst.i32 40
    return v5
block5:
    v6 = iconst.i32 50
    return v6
}
";
    let expected = "\
function @f(i1) -> i32 external {
block0(v0: i1):
    v1 = iconst.i32 2
    v2 = iconst.i32 2
    brif v0, block1, block2
block1:
    jump block5
block2:
    v3 = iconst.i32 2
    jump block1
block3:
    v4 = iconst.i32 30
    return v4
block4:
    v5 = iconst.i32 40
    return v5
block5:
    v6 = iconst.i32 50
    return v6
}
";
    assert_sccp(input, expected);
}

#[test]
fn absorbing_operands_and_freeze_decide_values() {
    let input = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 0
    v2 = imul.i32 v0, v1
    v3 = isub.i32 v0, v0
    v4 = iconst.i32 -1
    v5 = or.i32 v4, v0
    v6 = iconst.i32 1
    v7 = urem.i32 v0, v6
    v8 = icmp.i32 sle v0, v0
    v9 = freeze.i32 v6
    v10 = poison.i32
    v11 = freeze.i32 v10
    v12 = iadd.i32 v2, v11
    return v12
}
";
    let expected = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 0
    v2 = iconst.i32 0
    v3 = iconst.i32 0
    v4 = iconst.i32 -1
    v5 = iconst.i32 -1
    v6 = iconst.i32 1
    v7 = iconst.i32 0
    v8 = iconst.i1 1
    v9 = iconst.i32 1
    v10 = poison.i32
    v11 = freeze.i32 v10
    v12 = iadd.i32 v2, v11
    return v12
}
";
    assert_sccp(input, expected);
}

#[test]
fn blocks_the_solver_never_reaches_keep_their_code() {
    let input = "\
function @f() -> i32 external {
block0:
    v0 = iconst.i1 0
    v1 = iconst.i32 4
    brif v0, block1, block2
block1:
    v2 = iadd.i32 v1, v1
    return v2
block2:
    return v1
}
";
    let expected = "\
function @f() -> i32 external {
block0:
    v0 = iconst.i1 0
    v1 = iconst.i32 4
    jump block2
block1:
    v2 = iadd.i32 v1, v1
    return v2
block2:
    return v1
}
";
    assert_sccp(input, expected);
}

#[test]
fn each_sccp_rewrite_counts_for_bisecting() {
    let (full_text, full) = optimize_text(LOOP_WITH_A_DEAD_ARM, &only(&[Pass::Sccp]));
    let total = full.transformations();
    assert_eq!(total, 4, "two parameters, one comparison and one branch");
    let mut previous = LOOP_WITH_A_DEAD_ARM.to_owned();
    for limit in 1..=total {
        let limited = OptimizerOptions {
            bisect_limit: Some(limit),
            ..only(&[Pass::Sccp])
        };
        let (text, report) = optimize_text(LOOP_WITH_A_DEAD_ARM, &limited);
        assert_eq!(report.transformations(), limit);
        assert_ne!(text, previous, "rewrite {limit} changed nothing");
        previous = text;
    }
    assert_eq!(previous, full_text);
}
