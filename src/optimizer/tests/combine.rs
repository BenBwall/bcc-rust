//! `combine`: peephole rewrites of boolean, range-check and constant chains.

use super::*;

/// Runs `combine` then `dce` and checks the printed result.
fn assert_combine(input: &str, expected: &str) {
    let (actual, _) = optimize_text(input, &only(&[Pass::Combine, Pass::Dce]));
    pretty_assertions::assert_eq!(actual, expected);
}

#[test]
fn an_extended_boolean_compared_with_zero_is_the_boolean() {
    let input = "\
function @f(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = icmp.i32 slt v0, v1
    v3 = zext.i32 v2
    v4 = iconst.i32 0
    v5 = icmp.i32 ne v3, v4
    brif v5, block1, block2
block1:
    return v0
block2:
    return v1
}
";
    let expected = "\
function @f(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = icmp.i32 slt v0, v1
    brif v2, block1, block2
block1:
    return v0
block2:
    return v1
}
";
    assert_combine(input, expected);
}

#[test]
fn a_boolean_equal_to_zero_is_the_inverse_comparison() {
    let input = "\
function @f(i32, i32) -> i1 external {
block0(v0: i32, v1: i32):
    v2 = icmp.i32 ult v0, v1
    v3 = zext.i8 v2
    v4 = iconst.i8 0
    v5 = icmp.i8 eq v3, v4
    return v5
}
";
    let expected = "\
function @f(i32, i32) -> i1 external {
block0(v0: i32, v1: i32):
    v2 = icmp.i32 uge v0, v1
    return v2
}
";
    assert_combine(input, expected);
}

#[test]
fn a_lowered_and_of_two_bounds_is_one_unsigned_comparison() {
    // `c >= 'a' && c <= 'z'` after select formation.
    let input = "\
function @f(i32) -> i1 external {
block0(v0: i32):
    v1 = iconst.i32 97
    v2 = iconst.i32 122
    v3 = iconst.i1 0
    v4 = icmp.i32 sge v0, v1
    v5 = icmp.i32 sle v0, v2
    v6 = select.i1 v4, v5, v3
    return v6
}
";
    let expected = "\
function @f(i32) -> i1 external {
block0(v0: i32):
    v1 = iconst.i32 97
    v2 = iconst.i32 25
    v3 = isub.i32 v0, v1
    v4 = icmp.i32 ule v3, v2
    return v4
}
";
    assert_combine(input, expected);
}

#[test]
fn a_lowered_or_outside_two_bounds_is_one_unsigned_comparison() {
    // `c < '0' || c > '9'`, with the bounds written as strict comparisons
    // and the constant first in one of them.
    let input = "\
function @f(i32) -> i1 external {
block0(v0: i32):
    v1 = iconst.i32 48
    v2 = iconst.i32 57
    v3 = iconst.i1 1
    v4 = icmp.i32 sgt v1, v0
    v5 = icmp.i32 sgt v0, v2
    v6 = select.i1 v4, v3, v5
    return v6
}
";
    let expected = "\
function @f(i32) -> i1 external {
block0(v0: i32):
    v1 = iconst.i32 48
    v2 = iconst.i32 9
    v3 = isub.i32 v0, v1
    v4 = icmp.i32 ugt v3, v2
    return v4
}
";
    assert_combine(input, expected);
}

#[test]
fn bounds_with_an_empty_interval_decide_the_check() {
    let input = "\
function @f(i8) -> i1 external {
block0(v0: i8):
    v1 = iconst.i8 10
    v2 = iconst.i8 5
    v3 = icmp.i8 uge v0, v1
    v4 = icmp.i8 ule v0, v2
    v5 = and.i1 v3, v4
    return v5
}
";
    let expected = "\
function @f(i8) -> i1 external {
block0(v0: i8):
    v1 = iconst.i1 0
    return v1
}
";
    assert_combine(input, expected);
}

#[test]
fn bounds_of_different_signedness_or_values_stay() {
    let mixed = "\
function @f(i32) -> i1 external {
block0(v0: i32):
    v1 = iconst.i32 10
    v2 = iconst.i32 20
    v3 = icmp.i32 sge v0, v1
    v4 = icmp.i32 ule v0, v2
    v5 = and.i1 v3, v4
    return v5
}
";
    assert_pass_leaves(Pass::Combine, mixed);
    let two_values = "\
function @f(i32, i32) -> i1 external {
block0(v0: i32, v1: i32):
    v2 = iconst.i32 10
    v3 = iconst.i32 20
    v4 = icmp.i32 sge v0, v2
    v5 = icmp.i32 sle v1, v3
    v6 = and.i1 v4, v5
    return v6
}
";
    assert_pass_leaves(Pass::Combine, two_values);
}

#[test]
fn constant_additions_combine_into_one() {
    let input = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 3
    v2 = iadd.i32 nsw v0, v1
    v3 = iconst.i32 1
    v4 = isub.i32 nsw v2, v3
    return v4
}
";
    let expected = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 2
    v2 = iadd.i32 v0, v1
    return v2
}
";
    assert_combine(input, expected);
}

#[test]
fn a_negated_condition_swaps_branch_targets_and_select_arms() {
    let input = "\
function @f(i1, i32, i32) -> i32 external {
block0(v0: i1, v1: i32, v2: i32):
    v3 = iconst.i1 1
    v4 = xor.i1 v0, v3
    v5 = select.i32 v4, v1, v2
    brif v4, block1, block2
block1:
    return v5
block2:
    return v1
}
";
    let expected = "\
function @f(i1, i32, i32) -> i32 external {
block0(v0: i1, v1: i32, v2: i32):
    v3 = select.i32 v0, v2, v1
    brif v0, block2, block1
block1:
    return v3
block2:
    return v1
}
";
    assert_combine(input, expected);
}

#[test]
fn each_rewrite_is_one_bisectable_step() {
    let input = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 3
    v2 = iadd.i32 v0, v1
    v3 = iadd.i32 v2, v1
    v4 = iadd.i32 v3, v1
    return v4
}
";
    let (_, report) = optimize_text(input, &only(&[Pass::Combine]));
    assert_eq!(report.transformations(), 2);
    let limited = OptimizerOptions {
        bisect_limit: Some(1),
        ..only(&[Pass::Combine])
    };
    let (limited, _) = optimize_text(input, &limited);
    assert!(
        limited.contains("iconst.i32 6") && !limited.contains("iconst.i32 9"),
        "{limited}"
    );
    let (full, _) = optimize_text(input, &only(&[Pass::Combine]));
    assert!(full.contains("iconst.i32 9"), "{full}");
}
