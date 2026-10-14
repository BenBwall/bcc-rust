//! `sroa`: slots split into fields, and slots that must stay whole.

use super::*;

/// Runs `sroa` and checks the printed result.
fn assert_sroa(input: &str, expected: &str) {
    assert_pass(Pass::Sroa, input, expected);
}

#[test]
fn members_of_a_local_structure_become_their_own_slots() {
    let input = "\
function @f(i32, i64) -> i64 external {
    slot0 = stack_slot 16, align 8
block0(v0: i32, v1: i64):
    v2 = stack_addr slot0
    store.i32 v0, v2, align 8
    v3 = iconst.i64 8
    v4 = ptr_add inbounds v2, v3
    store.i64 v1, v4, align 8
    v5 = load.i32 v2, align 8
    v6 = sext.i64 v5
    v7 = load.i64 v4, align 8
    v8 = iadd.i64 v6, v7
    return v8
}
";
    let expected = "\
function @f(i32, i64) -> i64 external {
    slot0 = stack_slot 4, align 8
    slot1 = stack_slot 8, align 8
block0(v0: i32, v1: i64):
    v2 = stack_addr slot1
    v3 = stack_addr slot0
    store.i32 v0, v3, align 8
    v4 = iconst.i64 8
    store.i64 v1, v2, align 8
    v5 = load.i32 v3, align 8
    v6 = sext.i64 v5
    v7 = load.i64 v2, align 8
    v8 = iadd.i64 v6, v7
    return v8
}
";
    assert_sroa(input, expected);
}

#[test]
fn the_pipeline_turns_split_members_into_values() {
    let input = "\
function @f(i32, i64) -> i64 external {
    slot0 = stack_slot 16, align 8
block0(v0: i32, v1: i64):
    v2 = stack_addr slot0
    store.i32 v0, v2, align 8
    v3 = iconst.i64 8
    v4 = ptr_add inbounds v2, v3
    store.i64 v1, v4, align 8
    v5 = load.i32 v2, align 8
    v6 = sext.i64 v5
    v7 = load.i64 v4, align 8
    v8 = iadd.i64 v6, v7
    return v8
}
";
    let expected = "\
function @f(i32, i64) -> i64 external {
block0(v0: i32, v1: i64):
    v2 = sext.i64 v0
    v3 = iadd.i64 v2, v1
    return v3
}
";
    assert_pipeline(input, expected);
}

#[test]
fn a_copy_between_split_slots_moves_each_field() {
    let input = "\
function @f(i32, i32) -> i32 external {
    slot0 = stack_slot 8, align 4
    slot1 = stack_slot 8, align 4
block0(v0: i32, v1: i32):
    v2 = stack_addr slot0
    v3 = stack_addr slot1
    v4 = iconst.i64 4
    v5 = ptr_add inbounds v2, v4
    store.i32 v0, v2, align 4
    store.i32 v1, v5, align 4
    v6 = iconst.i64 8
    copy v3, v2, v6, align 4
    v7 = ptr_add inbounds v3, v4
    v8 = load.i32 v7, align 4
    return v8
}
";
    let (actual, _) = optimize_text(input, &only(&[Pass::Sroa, Pass::Promote, Pass::Dce]));
    pretty_assertions::assert_eq!(
        actual,
        "\
function @f(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    return v1
}
"
    );
    let (split, _) = optimize_text(input, &only(&[Pass::Sroa]));
    assert!(!split.contains("copy"), "{split}");
}

#[test]
fn a_copy_in_from_other_memory_loads_each_field_there() {
    let input = "\
function @f(ptr) -> i32 external {
    slot0 = stack_slot 8, align 4
block0(v0: ptr):
    v1 = stack_addr slot0
    v2 = iconst.i64 8
    copy v1, v0, v2, align 4
    v3 = iconst.i64 4
    v4 = ptr_add inbounds v1, v3
    v5 = load.i32 v4, align 4
    v6 = load.i32 v1, align 4
    v7 = iadd.i32 v5, v6
    return v7
}
";
    let expected = "\
function @f(ptr) -> i32 external {
block0(v0: ptr):
    v1 = iconst.i64 4
    v2 = load.i32 v0, align 4
    v3 = ptr_add inbounds v0, v1
    v4 = load.i32 v3, align 4
    v5 = iadd.i32 v4, v2
    return v5
}
";
    let (actual, _) = optimize_text(input, &only(&[Pass::Sroa, Pass::Promote, Pass::Dce]));
    pretty_assertions::assert_eq!(actual, expected);
}

#[test]
fn a_copy_out_carries_the_bytes_no_access_covers() {
    // Bytes 4..8 are padding: the fill defines them and the copy must still
    // write them out, as an `i32` field of their own.
    let input = "\
function @f(ptr, i32, i64) external {
    slot0 = stack_slot 16, align 8
block0(v0: ptr, v1: i32, v2: i64):
    v3 = stack_addr slot0
    v4 = iconst.i8 0
    v5 = iconst.i64 16
    fill v3, v4, v5, align 8
    store.i32 v1, v3, align 8
    v6 = iconst.i64 8
    v7 = ptr_add inbounds v3, v6
    store.i64 v2, v7, align 8
    copy v0, v3, v5, align 8
    return
}
";
    let expected = "\
function @f(ptr, i32, i64) external {
block0(v0: ptr, v1: i32, v2: i64):
    v3 = iconst.i32 0
    v4 = iconst.i64 4
    v5 = iconst.i64 8
    store.i32 v1, v0, align 8
    v6 = ptr_add inbounds v0, v4
    store.i32 v3, v6, align 4
    v7 = ptr_add inbounds v0, v5
    store.i64 v2, v7, align 8
    return
}
";
    let (actual, _) = optimize_text(input, &only(&[Pass::Sroa, Pass::Promote, Pass::Dce]));
    pretty_assertions::assert_eq!(actual, expected);
}

#[test]
fn a_fill_gives_each_field_its_bytes() {
    let input = "\
function @f() -> i64 external {
    slot0 = stack_slot 16, align 8
block0:
    v0 = stack_addr slot0
    v1 = iconst.i8 1
    v2 = iconst.i64 16
    fill v0, v1, v2, align 8
    v3 = iconst.i64 8
    v4 = ptr_add inbounds v0, v3
    v5 = load.i64 v4, align 8
    v6 = load.i16 v0, align 8
    v7 = zext.i64 v6
    v8 = iadd.i64 v5, v7
    return v8
}
";
    let expected = "\
function @f() -> i64 external {
block0:
    v0 = iconst.i16 257
    v1 = iconst.i64 72340172838076673
    v2 = zext.i64 v0
    v3 = iadd.i64 v1, v2
    return v3
}
";
    let (actual, _) = optimize_text(input, &only(&[Pass::Sroa, Pass::Promote, Pass::Dce]));
    pretty_assertions::assert_eq!(actual, expected);
}

#[test]
fn slots_whose_address_escapes_stay_whole() {
    let input = "\
function @g(ptr) external

function @f(i32) -> i32 external {
    slot0 = stack_slot 8, align 4
block0(v0: i32):
    v1 = stack_addr slot0
    store.i32 v0, v1, align 4
    v2 = iconst.i64 4
    v3 = ptr_add inbounds v1, v2
    store.i32 v0, v3, align 4
    call @g(v3)
    v4 = load.i32 v1, align 4
    return v4
}
";
    assert_pass_leaves(Pass::Sroa, input);
}

#[test]
fn a_variable_index_keeps_an_array_whole() {
    let input = "\
function @f(i64, i32) -> i32 external {
    slot0 = stack_slot 16, align 4
block0(v0: i64, v1: i32):
    v2 = stack_addr slot0
    store.i32 v1, v2, align 4
    v3 = ptr_add inbounds v2, v0
    v4 = load.i32 v3, align 4
    return v4
}
";
    assert_pass_leaves(Pass::Sroa, input);
}

#[test]
fn overlapping_members_keep_a_union_whole() {
    let input = "\
function @f(i64) -> i32 external {
    slot0 = stack_slot 8, align 8
block0(v0: i64):
    v1 = stack_addr slot0
    store.i64 v0, v1, align 8
    v2 = iconst.i64 4
    v3 = ptr_add inbounds v1, v2
    v4 = load.i32 v3, align 4
    return v4
}
";
    assert_pass_leaves(Pass::Sroa, input);
}

#[test]
fn a_component_copied_both_in_and_out_stays_whole() {
    // Field-wise moves would turn a partly poisoned field of the source into
    // a wholly poisoned one at the destination.
    let input = "\
function @f(ptr, ptr) -> i32 external {
    slot0 = stack_slot 8, align 4
block0(v0: ptr, v1: ptr):
    v2 = stack_addr slot0
    v3 = iconst.i64 8
    copy v2, v0, v3, align 4
    v4 = load.i32 v2, align 4
    copy v1, v2, v3, align 4
    return v4
}
";
    assert_pass_leaves(Pass::Sroa, input);
}

#[test]
fn volatile_and_partial_accesses_keep_a_slot_whole() {
    let volatile = "\
function @f(i32) -> i32 external {
    slot0 = stack_slot 8, align 4
block0(v0: i32):
    v1 = stack_addr slot0
    store.i32 v0, v1, align 4
    v2 = iconst.i64 4
    v3 = ptr_add inbounds v1, v2
    store.i32 volatile v0, v3, align 4
    v4 = load.i32 v1, align 4
    return v4
}
";
    assert_pass_leaves(Pass::Sroa, volatile);
    let partial = "\
function @f(ptr, i32) -> i32 external {
    slot0 = stack_slot 8, align 4
block0(v0: ptr, v1: i32):
    v2 = stack_addr slot0
    store.i32 v1, v2, align 4
    v3 = iconst.i64 4
    copy v0, v2, v3, align 4
    v4 = load.i32 v2, align 4
    return v4
}
";
    assert_pass_leaves(Pass::Sroa, partial);
}

#[test]
fn a_scalar_slot_is_left_to_promote() {
    let input = "\
function @f(i32) -> i32 external {
    slot0 = stack_slot 4, align 4
block0(v0: i32):
    v1 = stack_addr slot0
    store.i32 v0, v1, align 4
    v2 = load.i32 v1, align 4
    return v2
}
";
    let (_, report) = optimize_text(input, &only(&[Pass::Sroa]));
    assert_eq!(report.transformations(), 0);
}

#[test]
fn each_component_is_one_bisectable_rewrite() {
    let input = "\
function @f(i32) -> i32 external {
    slot0 = stack_slot 8, align 4
    slot1 = stack_slot 8, align 4
block0(v0: i32):
    v1 = stack_addr slot0
    v2 = stack_addr slot1
    v3 = iconst.i64 4
    v4 = ptr_add inbounds v1, v3
    v5 = ptr_add inbounds v2, v3
    store.i32 v0, v4, align 4
    store.i32 v0, v5, align 4
    v6 = load.i32 v4, align 4
    v7 = load.i32 v5, align 4
    v8 = iadd.i32 v6, v7
    return v8
}
";
    let (_, report) = optimize_text(input, &only(&[Pass::Sroa]));
    assert_eq!(report.transformations(), 2);
    let limited = OptimizerOptions {
        bisect_limit: Some(1),
        ..only(&[Pass::Sroa])
    };
    let (actual, _) = optimize_text(input, &limited);
    assert_eq!(actual.matches("stack_slot").count(), 2, "{actual}");
}
