//! `gvn`: redundant pure instructions across the dominator tree.

use super::*;

#[test]
fn only_dominating_twins_are_reused() {
    // block0's sum reaches every block; block1's product reaches nothing
    // outside block1, so block2 and the join keep their own.
    let input = "\
function @f(i32, i32, i1) -> i32 external {
block0(v0: i32, v1: i32, v2: i1):
    v3 = iadd.i32 v0, v1
    brif v2, block1, block2
block1:
    v4 = iadd.i32 v0, v1
    v5 = imul.i32 v4, v1
    v6 = imul.i32 v4, v1
    jump block3(v6)
block2:
    v7 = imul.i32 v3, v1
    jump block3(v7)
block3(v8: i32):
    v9 = imul.i32 v3, v1
    v10 = iadd.i32 v8, v9
    v11 = iadd.i32 v10, v3
    return v11
}
";
    let expected = "\
function @f(i32, i32, i1) -> i32 external {
block0(v0: i32, v1: i32, v2: i1):
    v3 = iadd.i32 v0, v1
    brif v2, block1, block2
block1:
    v4 = imul.i32 v3, v1
    jump block3(v4)
block2:
    v5 = imul.i32 v3, v1
    jump block3(v5)
block3(v6: i32):
    v7 = imul.i32 v3, v1
    v8 = iadd.i32 v6, v7
    v9 = iadd.i32 v8, v3
    return v9
}
";
    assert_pass(Pass::Gvn, input, expected);
}

#[test]
fn commutative_operands_and_swapped_comparisons_match() {
    let input = "\
function @f(i32, i32) -> i1 external {
block0(v0: i32, v1: i32):
    v2 = imul.i32 v0, v1
    v3 = imul.i32 v1, v0
    v4 = isub.i32 v0, v1
    v5 = isub.i32 v1, v0
    v6 = icmp.i32 slt v2, v4
    v7 = icmp.i32 sgt v4, v3
    v8 = icmp.i32 slt v4, v3
    v9 = icmp.i32 eq v5, v4
    v10 = icmp.i32 eq v4, v5
    v11 = and.i1 v6, v7
    v12 = and.i1 v8, v9
    v13 = xor.i1 v11, v12
    v14 = xor.i1 v10, v13
    return v14
}
";
    // `isub` is not commutative and `slt v4, v3` is not `slt v2, v4`.
    let expected = "\
function @f(i32, i32) -> i1 external {
block0(v0: i32, v1: i32):
    v2 = imul.i32 v0, v1
    v3 = isub.i32 v0, v1
    v4 = isub.i32 v1, v0
    v5 = icmp.i32 slt v2, v3
    v6 = icmp.i32 slt v3, v2
    v7 = icmp.i32 eq v4, v3
    v8 = and.i1 v5, v5
    v9 = and.i1 v6, v7
    v10 = xor.i1 v8, v9
    v11 = xor.i1 v7, v10
    return v11
}
";
    assert_pass(Pass::Gvn, input, expected);
}

#[test]
fn a_merge_keeps_only_the_common_flags() {
    let input = "\
function @f(i32, i32, ptr, i64) -> i32 external {
block0(v0: i32, v1: i32, v2: ptr, v3: i64):
    v4 = iadd.i32 nsw nuw v0, v1
    v5 = iadd.i32 nsw v1, v0
    v6 = shl.i32 v0, v1
    v7 = shl.i32 nuw v0, v1
    v8 = sdiv.i32 exact v0, v1
    v9 = sdiv.i32 v0, v1
    v10 = ptr_add inbounds v2, v3
    v11 = ptr_add v2, v3
    v12 = icmp.ptr eq v10, v11
    v13 = zext.i32 v12
    v14 = iadd.i32 v4, v5
    v15 = iadd.i32 v6, v7
    v16 = iadd.i32 v8, v9
    v17 = iadd.i32 v14, v15
    v18 = iadd.i32 v16, v17
    v19 = iadd.i32 v18, v13
    return v19
}
";
    // The first of each pair survives with the flags both had: `nsw` for the
    // sums, none for the shifts (the first had none), the divisions and the
    // pointers.
    let expected = "\
function @f(i32, i32, ptr, i64) -> i32 external {
block0(v0: i32, v1: i32, v2: ptr, v3: i64):
    v4 = iadd.i32 nsw v0, v1
    v5 = shl.i32 v0, v1
    v6 = sdiv.i32 v0, v1
    v7 = ptr_add v2, v3
    v8 = icmp.ptr eq v7, v7
    v9 = zext.i32 v8
    v10 = iadd.i32 v4, v4
    v11 = iadd.i32 v5, v5
    v12 = iadd.i32 v6, v6
    v13 = iadd.i32 v10, v11
    v14 = iadd.i32 v12, v13
    v15 = iadd.i32 v14, v9
    return v15
}
";
    assert_pass(Pass::Gvn, input, expected);
}

#[test]
fn constants_and_addresses_merge_by_value() {
    let input = "\
function @f(i1) -> i64 external {
    slot0 = stack_slot 8, align 8
    slot1 = stack_slot 8, align 8
block0(v0: i1):
    v1 = iconst.i64 7
    v2 = stack_addr slot0
    brif v0, block1, block2
block1:
    v3 = iconst.i64 7
    v4 = iconst.i32 7
    v5 = stack_addr slot0
    v6 = stack_addr slot1
    v7 = ptr_add v5, v3
    v8 = ptr_add v6, v1
    v9 = icmp.ptr eq v7, v8
    v10 = zext.i64 v9
    v11 = zext.i64 v4
    v12 = iadd.i64 v10, v11
    return v12
block2:
    v13 = ptr_add v2, v1
    v14 = ptrtoint.i64 v13
    return v14
}
";
    // The `i32` 7 and the other slot stay.
    let expected = "\
function @f(i1) -> i64 external {
    slot0 = stack_slot 8, align 8
    slot1 = stack_slot 8, align 8
block0(v0: i1):
    v1 = iconst.i64 7
    v2 = stack_addr slot0
    brif v0, block1, block2
block1:
    v3 = iconst.i32 7
    v4 = stack_addr slot1
    v5 = ptr_add v2, v1
    v6 = ptr_add v4, v1
    v7 = icmp.ptr eq v5, v6
    v8 = zext.i64 v7
    v9 = zext.i64 v3
    v10 = iadd.i64 v8, v9
    return v10
block2:
    v11 = ptr_add v2, v1
    v12 = ptrtoint.i64 v11
    return v12
}
";
    assert_pass(Pass::Gvn, input, expected);
}

#[test]
fn loads_calls_and_freezes_are_never_merged() {
    let input = "\
function @g(i32) -> i32 external

function @f(ptr, i32) -> i32 external {
block0(v0: ptr, v1: i32):
    v2 = load.i32 v0, align 4
    v3 = load.i32 v0, align 4
    v4 = call @g(v1)
    v5 = call @g(v1)
    v6 = freeze.i32 v1
    v7 = freeze.i32 v1
    v8 = iadd.i32 v2, v3
    v9 = iadd.i32 v4, v5
    v10 = iadd.i32 v6, v7
    v11 = iadd.i32 v8, v9
    v12 = iadd.i32 v10, v11
    return v12
}
";
    assert_pass_leaves(Pass::Gvn, input);
}

#[test]
fn a_loop_body_reuses_the_header_but_not_the_reverse() {
    let input = "\
function @f(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = iconst.i32 0
    jump block1(v2)
block1(v3: i32):
    v4 = iadd.i32 v3, v0
    v5 = icmp.i32 slt v4, v1
    brif v5, block2, block3
block2:
    v6 = iadd.i32 v0, v3
    v7 = icmp.i32 sgt v1, v6
    v8 = zext.i32 v7
    v9 = iadd.i32 v6, v8
    jump block1(v9)
block3:
    v10 = iadd.i32 v3, v0
    return v10
}
";
    let expected = "\
function @f(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = iconst.i32 0
    jump block1(v2)
block1(v3: i32):
    v4 = iadd.i32 v3, v0
    v5 = icmp.i32 slt v4, v1
    brif v5, block2, block3
block2:
    v6 = zext.i32 v5
    v7 = iadd.i32 v4, v6
    jump block1(v7)
block3:
    return v4
}
";
    assert_pass(Pass::Gvn, input, expected);
}

#[test]
fn bisecting_stops_after_the_limit() {
    let input = "\
function @f(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = iadd.i32 v0, v1
    v3 = iadd.i32 v0, v1
    v4 = imul.i32 v0, v1
    v5 = imul.i32 v0, v1
    v6 = iadd.i32 v2, v3
    v7 = iadd.i32 v4, v5
    v8 = iadd.i32 v6, v7
    return v8
}
";
    let options = OptimizerOptions {
        bisect_limit: Some(1),
        ..only(&[Pass::Gvn])
    };
    let (actual, report) = optimize_text(input, &options);
    let expected = "\
function @f(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = iadd.i32 v0, v1
    v3 = imul.i32 v0, v1
    v4 = imul.i32 v0, v1
    v5 = iadd.i32 v2, v2
    v6 = iadd.i32 v3, v4
    v7 = iadd.i32 v5, v6
    return v7
}
";
    pretty_assertions::assert_eq!(actual, expected);
    assert_eq!(report.transformations(), 1);
    assert!(report.limit_reached());
}
