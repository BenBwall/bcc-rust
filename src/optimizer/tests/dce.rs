//! `dce`: unused instructions and block parameters.

use super::*;

#[test]
fn unused_pure_chains_are_removed() {
    let input = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 1
    v2 = iadd.i32 v0, v1
    v3 = imul.i32 v2, v2
    v4 = iadd.i32 v0, v0
    return v4
}
";
    let expected = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iadd.i32 v0, v0
    return v1
}
";
    assert_pass(Pass::Dce, input, expected);
}

#[test]
fn instructions_with_side_effects_are_kept() {
    let input = "\
function @g(i32) -> i32 external

function @f(ptr, ptr) internal {
    slot0 = stack_slot 8, align 8
block0(v0: ptr, v1: ptr):
    v2 = iconst.i32 1
    store.i32 v2, v0, align 4
    v3 = load.i32 volatile v0, align 4
    v4 = call @g(v2)
    v5 = iconst.i64 8
    copy v0, v1, v5, align 1
    v6 = iconst.i8 0
    fill v0, v6, v5, align 1
    return
}
";
    assert_pass_leaves(Pass::Dce, input);
}

#[test]
fn an_unused_plain_load_is_removed_but_a_volatile_one_is_not() {
    let input = "\
function @f(ptr) internal {
block0(v0: ptr):
    v1 = load.i32 v0, align 4
    v2 = load.i32 volatile v0, align 4
    v3 = sdiv.i32 v1, v2
    return
}
";
    let expected = "\
function @f(ptr) internal {
block0(v0: ptr):
    v1 = load.i32 volatile v0, align 4
    return
}
";
    assert_pass(Pass::Dce, input, expected);
}

#[test]
fn unused_block_parameters_lose_their_arguments() {
    let input = "\
function @f(i32, i1) -> i32 external {
block0(v0: i32, v1: i1):
    v2 = iconst.i32 7
    brif v1, block1(v0, v2), block2(v0, v2)
block1(v3: i32, v4: i32):
    return v3
block2(v5: i32, v6: i32):
    return v5
}
";
    let expected = "\
function @f(i32, i1) -> i32 external {
block0(v0: i32, v1: i1):
    brif v1, block1(v0), block2(v0)
block1(v2: i32):
    return v2
block2(v3: i32):
    return v3
}
";
    assert_pass(Pass::Dce, input, expected);
}

#[test]
fn a_removed_parameter_releases_the_instruction_that_fed_it() {
    let input = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iadd.i32 v0, v0
    jump block1(v1)
block1(v2: i32):
    return v0
}
";
    let expected = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    jump block1
block1:
    return v0
}
";
    assert_pass(Pass::Dce, input, expected);
}

#[test]
fn entry_parameters_stay_even_when_unused() {
    let input = "\
function @f(i32, i64) internal {
block0(v0: i32, v1: i64):
    return
}
";
    assert_pass_leaves(Pass::Dce, input);
}

#[test]
fn a_parameter_in_one_of_two_switch_edges_is_removed_from_all_of_them() {
    let input = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    switch v0, block1(v0), [1: block2(v0), 2: block1(v0)]
block1(v1: i32):
    return v0
block2(v2: i32):
    return v2
}
";
    let expected = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    switch v0, block1, [1: block2(v0), 2: block1]
block1:
    return v0
block2(v1: i32):
    return v1
}
";
    assert_pass(Pass::Dce, input, expected);
}

#[test]
fn dead_cycles_through_block_parameters_remain() {
    // `v3` is only used to compute the value it is passed back as. Removing
    // such a cycle needs optimistic liveness, which `dce` does not do.
    let input = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 0
    jump block1(v1, v1)
block1(v2: i32, v3: i32):
    v4 = icmp.i32 slt v2, v0
    v5 = iconst.i32 1
    v6 = iadd.i32 nsw v2, v5
    v7 = iadd.i32 v3, v2
    brif v4, block1(v6, v7), block2
block2:
    return v0
}
";
    assert_pass_leaves(Pass::Dce, input);
}

#[test]
fn the_bisect_limit_stops_removals_without_breaking_the_function() {
    let input = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iadd.i32 v0, v0
    v2 = imul.i32 v1, v1
    v3 = isub.i32 v2, v1
    return v0
}
";
    for (limit, expected_insts) in [(0, 3), (1, 2), (2, 1), (3, 0), (4, 0)] {
        let options = OptimizerOptions {
            bisect_limit: Some(limit),
            ..only(&[Pass::Dce])
        };
        let (text, report) = optimize_text(input, &options);
        let insts = text.lines().filter(|line| line.contains(" = ")).count();
        assert_eq!(insts, expected_insts, "limit {limit}\n{text}");
        assert_eq!(report.transformations(), limit.min(3));
    }
}
