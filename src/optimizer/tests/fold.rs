//! `fold`: constants, poison, identities and constant branches.

use super::*;

/// A function `@f` with `body` and the signature `() -> i32`.
fn nullary(body: &str) -> String {
    format!("function @f() -> i32 external {{\nblock0:\n{body}}}\n")
}

#[test]
fn integer_arithmetic_on_constants_is_computed() {
    let input = nullary(
        "    v0 = iconst.i32 6
    v1 = iconst.i32 7
    v2 = imul.i32 nsw v0, v1
    v3 = iadd.i32 v2, v0
    v4 = isub.i32 v0, v1
    v5 = sdiv.i32 v2, v0
    v6 = srem.i32 v4, v1
    v7 = icmp.i32 slt v4, v0
    v8 = zext.i32 v7
    v9 = iadd.i32 v3, v8
    return v9
",
    );
    let expected = nullary(
        "    v0 = iconst.i32 6
    v1 = iconst.i32 7
    v2 = iconst.i32 42
    v3 = iconst.i32 48
    v4 = iconst.i32 -1
    v5 = iconst.i32 7
    v6 = iconst.i32 -1
    v7 = iconst.i1 1
    v8 = iconst.i32 1
    v9 = iconst.i32 49
    return v9
",
    );
    assert_pass(Pass::Fold, &input, &expected);
}

#[test]
fn wrapping_arithmetic_wraps_and_flags_poison() {
    let input = nullary(
        "    v0 = iconst.i32 2147483647
    v1 = iconst.i32 1
    v2 = iadd.i32 v0, v1
    v3 = iadd.i32 nsw v0, v1
    v4 = iconst.i32 -1
    v5 = iadd.i32 nuw v4, v1
    v6 = isub.i32 nuw v1, v4
    v7 = imul.i32 nsw v0, v0
    v8 = iadd.i32 v3, v2
    return v8
",
    );
    let expected = nullary(
        "    v0 = iconst.i32 2147483647
    v1 = iconst.i32 1
    v2 = iconst.i32 -2147483648
    v3 = poison.i32
    v4 = iconst.i32 -1
    v5 = poison.i32
    v6 = poison.i32
    v7 = poison.i32
    v8 = poison.i32
    return v8
",
    );
    assert_pass(Pass::Fold, &input, &expected);
}

#[test]
fn division_flags_and_shifts_follow_the_poison_rules() {
    let input = nullary(
        "    v0 = iconst.i32 7
    v1 = iconst.i32 2
    v2 = udiv.i32 exact v0, v1
    v3 = udiv.i32 v0, v1
    v4 = iconst.i32 8
    v5 = lshr.i32 exact v4, v1
    v6 = lshr.i32 exact v0, v1
    v7 = iconst.i32 32
    v8 = shl.i32 v0, v7
    v9 = shl.i32 nuw v4, v7
    v10 = iconst.i32 31
    v11 = shl.i32 nsw v0, v10
    v12 = shl.i32 nuw v0, v10
    v13 = iconst.i32 -8
    v14 = ashr.i32 v13, v1
    return v3
",
    );
    let expected = nullary(
        "    v0 = iconst.i32 7
    v1 = iconst.i32 2
    v2 = poison.i32
    v3 = iconst.i32 3
    v4 = iconst.i32 8
    v5 = iconst.i32 2
    v6 = poison.i32
    v7 = iconst.i32 32
    v8 = poison.i32
    v9 = poison.i32
    v10 = iconst.i32 31
    v11 = poison.i32
    v12 = poison.i32
    v13 = iconst.i32 -8
    v14 = iconst.i32 -2
    return v3
",
    );
    assert_pass(Pass::Fold, &input, &expected);
}

#[test]
fn undefined_division_is_left_alone() {
    let input = nullary(
        "    v0 = iconst.i32 7
    v1 = iconst.i32 0
    v2 = sdiv.i32 v0, v1
    v3 = urem.i32 v0, v1
    v4 = iconst.i32 -2147483648
    v5 = iconst.i32 -1
    v6 = sdiv.i32 v4, v5
    v7 = srem.i32 v4, v5
    v8 = poison.i32
    v9 = udiv.i32 v0, v8
    v10 = iadd.i32 v2, v3
    return v10
",
    );
    assert_pass_leaves(Pass::Fold, &input);
}

#[test]
fn poison_operands_poison_the_result() {
    let input = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = poison.i32
    v2 = iadd.i32 v0, v1
    v3 = icmp.i32 eq v1, v0
    v4 = zext.i64 v3
    v5 = trunc.i32 v4
    v6 = iconst.i32 3
    v7 = udiv.i32 v1, v6
    v8 = iadd.i32 v5, v7
    return v8
}
";
    let expected = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = poison.i32
    v2 = poison.i32
    v3 = poison.i1
    v4 = poison.i64
    v5 = poison.i32
    v6 = iconst.i32 3
    v7 = poison.i32
    v8 = poison.i32
    return v8
}
";
    assert_pass(Pass::Fold, input, expected);
}

#[test]
fn integer_conversions_of_constants_are_computed() {
    let input = "\
function @f() -> i64 external {
block0:
    v0 = iconst.i8 -3
    v1 = sext.i64 v0
    v2 = zext.i64 v0
    v3 = trunc.i8 v2
    v4 = sext.i64 v3
    v5 = iadd.i64 v1, v4
    v6 = iadd.i64 v5, v2
    return v6
}
";
    let expected = "\
function @f() -> i64 external {
block0:
    v0 = iconst.i8 -3
    v1 = iconst.i64 -3
    v2 = iconst.i64 253
    v3 = iconst.i8 -3
    v4 = iconst.i64 -3
    v5 = iconst.i64 -6
    v6 = iconst.i64 247
    return v6
}
";
    assert_pass(Pass::Fold, input, expected);
}

#[test]
fn wide_constants_fold() {
    let input = "\
function @f() -> i128 external {
block0:
    v0 = iconst.i128 170141183460469231731687303715884105727
    v1 = iconst.i128 1
    v2 = iadd.i128 v0, v1
    v3 = iadd.i128 nsw v0, v1
    v4 = iadd.i128 nuw v0, v0
    v5 = isub.i128 v2, v1
    return v5
}
";
    let expected = "\
function @f() -> i128 external {
block0:
    v0 = iconst.i128 170141183460469231731687303715884105727
    v1 = iconst.i128 1
    v2 = iconst.i128 -170141183460469231731687303715884105728
    v3 = poison.i128
    v4 = iconst.i128 -2
    v5 = iconst.i128 170141183460469231731687303715884105727
    return v5
}
";
    assert_pass(Pass::Fold, input, expected);
}

#[test]
fn identities_replace_an_instruction_by_an_operand() {
    let input = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 0
    v2 = iadd.i32 v0, v1
    v3 = iconst.i32 1
    v4 = imul.i32 v2, v3
    v5 = iconst.i32 -1
    v6 = and.i32 v4, v5
    v7 = or.i32 v6, v1
    v8 = xor.i32 v7, v1
    v9 = isub.i32 v8, v8
    v10 = iadd.i32 v8, v9
    return v10
}
";
    let expected = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 0
    v2 = iconst.i32 1
    v3 = iconst.i32 -1
    v4 = iconst.i32 0
    return v0
}
";
    assert_pass(Pass::Fold, input, expected);
}

#[test]
fn identities_apply_with_the_constant_on_either_side() {
    let input = "\
function @f(i32, i64) -> i32 external {
block0(v0: i32, v1: i64):
    v2 = iconst.i32 0
    v3 = iadd.i32 nsw v2, v0
    v4 = iconst.i32 1
    v5 = imul.i32 nuw v4, v3
    v6 = shl.i32 v5, v2
    v7 = ashr.i32 exact v6, v2
    v8 = udiv.i32 exact v7, v4
    v9 = sdiv.i32 v8, v4
    v10 = null
    v11 = iconst.i64 0
    v12 = ptr_add inbounds v10, v11
    return v9
}
";
    let expected = "\
function @f(i32, i64) -> i32 external {
block0(v0: i32, v1: i64):
    v2 = iconst.i32 0
    v3 = iconst.i32 1
    v4 = null
    v5 = iconst.i64 0
    return v0
}
";
    assert_pass(Pass::Fold, input, expected);
}

#[test]
fn absorbing_constants_and_equal_operands_fold_to_constants() {
    let input = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 0
    v2 = imul.i32 v0, v1
    v3 = and.i32 v1, v0
    v4 = iconst.i32 -1
    v5 = or.i32 v0, v4
    v6 = xor.i32 v0, v0
    v7 = iconst.i32 1
    v8 = srem.i32 v0, v7
    v9 = and.i32 v0, v0
    v10 = or.i32 v9, v9
    v11 = iadd.i32 v2, v3
    v12 = iadd.i32 v5, v6
    v13 = iadd.i32 v8, v10
    v14 = iadd.i32 v11, v12
    v15 = iadd.i32 v14, v13
    return v15
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
    v6 = iconst.i32 0
    v7 = iconst.i32 1
    v8 = iconst.i32 0
    v9 = iconst.i32 0
    v10 = iconst.i32 -1
    v11 = iconst.i32 -1
    v12 = iadd.i32 v11, v0
    return v12
}
";
    assert_pass(Pass::Fold, input, expected);
}

#[test]
fn x_minus_x_is_zero_even_with_wrap_flags() {
    let input = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = isub.i32 nsw nuw v0, v0
    return v1
}
";
    let expected = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 0
    return v1
}
";
    assert_pass(Pass::Fold, input, expected);
}

#[test]
fn comparisons_fold_on_constants_and_equal_operands() {
    let input = "\
function @f(i32) -> i1 external {
block0(v0: i32):
    v1 = iconst.i32 -1
    v2 = iconst.i32 1
    v3 = icmp.i32 slt v1, v2
    v4 = icmp.i32 ult v1, v2
    v5 = icmp.i32 eq v0, v0
    v6 = icmp.i32 ult v0, v0
    v7 = icmp.i32 sge v0, v0
    v8 = icmp.i32 slt v0, v2
    v9 = and.i1 v3, v5
    v10 = or.i1 v4, v6
    v11 = xor.i1 v9, v10
    v12 = xor.i1 v11, v7
    return v12
}
";
    let expected = "\
function @f(i32) -> i1 external {
block0(v0: i32):
    v1 = iconst.i32 -1
    v2 = iconst.i32 1
    v3 = iconst.i1 1
    v4 = iconst.i1 0
    v5 = iconst.i1 1
    v6 = iconst.i1 0
    v7 = iconst.i1 1
    v8 = icmp.i32 slt v0, v2
    v9 = iconst.i1 1
    v10 = iconst.i1 0
    v11 = iconst.i1 1
    v12 = iconst.i1 0
    return v12
}
";
    assert_pass(Pass::Fold, input, expected);
}

#[test]
fn select_folds_on_a_constant_condition_or_equal_arms() {
    let input = "\
function @f(i32, i32, i1) -> i32 external {
block0(v0: i32, v1: i32, v2: i1):
    v3 = iconst.i1 1
    v4 = select.i32 v3, v0, v1
    v5 = iconst.i1 0
    v6 = select.i32 v5, v0, v1
    v7 = select.i32 v2, v1, v1
    v8 = select.i32 v2, v0, v1
    v9 = poison.i1
    v10 = select.i32 v9, v0, v1
    v11 = iadd.i32 v4, v6
    v12 = iadd.i32 v7, v8
    v13 = iadd.i32 v11, v12
    v14 = iadd.i32 v13, v10
    return v14
}
";
    let expected = "\
function @f(i32, i32, i1) -> i32 external {
block0(v0: i32, v1: i32, v2: i1):
    v3 = iconst.i1 1
    v4 = iconst.i1 0
    v5 = select.i32 v2, v0, v1
    v6 = poison.i1
    v7 = poison.i32
    v8 = iadd.i32 v0, v1
    v9 = iadd.i32 v1, v5
    v10 = iadd.i32 v8, v9
    v11 = poison.i32
    return v11
}
";
    assert_pass(Pass::Fold, input, expected);
}

#[test]
fn freeze_of_a_constant_is_the_constant_and_of_poison_stays() {
    let input = "\
function @f() -> i32 external {
block0:
    v0 = iconst.i32 5
    v1 = freeze.i32 v0
    v2 = poison.i32
    v3 = freeze.i32 v2
    v4 = iadd.i32 v1, v3
    return v4
}
";
    let expected = "\
function @f() -> i32 external {
block0:
    v0 = iconst.i32 5
    v1 = poison.i32
    v2 = freeze.i32 v1
    v3 = iadd.i32 v0, v2
    return v3
}
";
    assert_pass(Pass::Fold, input, expected);
}

#[test]
fn constants_flow_across_blocks_in_dominance_order() {
    // The user's block is laid out before the definition's block, so layout
    // order would miss the constant; reverse post-order does not.
    let input = "function @f() -> i32 external {
block0:
    jump block2
block1:
    v0 = iadd.i32 v1, v1
    return v0
block2:
    v1 = iconst.i32 21
    jump block1
}
";
    let expected = "function @f() -> i32 external {
block0:
    jump block2
block1:
    v0 = iconst.i32 42
    return v0
block2:
    v1 = iconst.i32 21
    jump block1
}
";
    assert_pass(Pass::Fold, input, expected);
}

#[test]
fn brif_on_a_constant_becomes_a_jump() {
    let input = "function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i1 0
    brif v1, block1(v0), block2(v0)
block1(v2: i32):
    return v2
block2(v3: i32):
    return v3
}
";
    let expected = "function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i1 0
    jump block2(v0)
block1(v2: i32):
    return v2
block2(v3: i32):
    return v3
}
";
    assert_pass(Pass::Fold, input, expected);
    let taken = input.replace("iconst.i1 0", "iconst.i1 1");
    assert_pass(
        Pass::Fold,
        &taken,
        &expected
            .replace("iconst.i1 0", "iconst.i1 1")
            .replace("jump block2(v0)", "jump block1(v0)"),
    );
}

#[test]
fn a_comparison_feeds_a_branch_in_one_run() {
    let input = "function @f() -> i32 external {
block0:
    v0 = iconst.i32 3
    v1 = iconst.i32 4
    v2 = icmp.i32 slt v0, v1
    brif v2, block1, block2
block1:
    return v0
block2:
    return v1
}
";
    let expected = "function @f() -> i32 external {
block0:
    v0 = iconst.i32 3
    v1 = iconst.i32 4
    v2 = iconst.i1 1
    jump block1
block1:
    return v0
block2:
    return v1
}
";
    assert_pass(Pass::Fold, input, expected);
}

#[test]
fn branching_on_poison_is_left_alone() {
    let input = "function @f() -> i32 external {
block0:
    v0 = poison.i1
    v1 = iconst.i32 1
    brif v0, block1, block2
block1:
    return v1
block2:
    switch v0, block1, [0: block2]
}
";
    // The switch value has the wrong type for a verified module; use an i32.
    let _ = input;
    let input = "function @f() -> i32 external {
block0:
    v0 = poison.i1
    v1 = poison.i32
    brif v0, block1, block2
block1:
    switch v1, block2, [0: block1]
block2:
    return v1
}
";
    assert_pass_leaves(Pass::Fold, input);
}

#[test]
fn switch_on_a_constant_jumps_to_the_matching_case_or_the_default() {
    let template = "function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 CASE
    switch v1, block1(v0), [1: block2(v0), -5: block3(v0)]
block1(v2: i32):
    return v2
block2(v3: i32):
    return v3
block3(v4: i32):
    return v4
}
";
    for (case, target) in [("1", "block2"), ("-5", "block3"), ("2", "block1")] {
        let input = template.replace("CASE", case);
        let expected = input.replace(
            "switch v1, block1(v0), [1: block2(v0), -5: block3(v0)]",
            &format!("jump {target}(v0)"),
        );
        assert_pass(Pass::Fold, &input, &expected);
    }
}

#[test]
fn floats_are_not_folded() {
    let input = "function @f() -> f64 external {
block0:
    v0 = fconst.f64 0x3FF0000000000000
    v1 = fadd.f64 v0, v0
    v2 = fneg.f64 v1
    v3 = fcmp.f64 olt v1, v2
    v4 = select.f64 v3, v1, v2
    return v4
}
";
    assert_pass_leaves(Pass::Fold, input);
}

#[test]
fn loads_calls_and_stores_are_never_folded_away() {
    let input = "function @g(i32) -> i32 external

function @f(ptr) -> i32 external {
block0(v0: ptr):
    v1 = iconst.i32 0
    v2 = load.i32 v0, align 4
    v3 = iadd.i32 v2, v1
    store.i32 v3, v0, align 4
    v4 = call @g(v3)
    return v4
}
";
    let expected = "function @g(i32) -> i32 external

function @f(ptr) -> i32 external {
block0(v0: ptr):
    v1 = iconst.i32 0
    v2 = load.i32 v0, align 4
    store.i32 v2, v0, align 4
    v3 = call @g(v2)
    return v3
}
";
    assert_pass(Pass::Fold, input, expected);
}
