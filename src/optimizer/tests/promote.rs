//! `promote`: slots that become SSA values, and slots that must stay.

use super::*;

/// Runs `promote` and checks the printed result.
fn assert_promote(input: &str, expected: &str) {
    assert_pass(Pass::Promote, input, expected);
}

#[test]
fn a_loop_counter_becomes_a_block_parameter() {
    let input = "\
function @f(i32) -> i32 external {
    slot0 = stack_slot 4, align 4
block0(v0: i32):
    v1 = stack_addr slot0
    v2 = iconst.i32 0
    store.i32 v2, v1, align 4
    jump block1
block1:
    v3 = load.i32 v1, align 4
    v4 = icmp.i32 slt v3, v0
    brif v4, block2, block3
block2:
    v5 = iconst.i32 1
    v6 = iadd.i32 v3, v5
    store.i32 v6, v1, align 4
    jump block1
block3:
    return v3
}
";
    let expected = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 0
    jump block1(v1)
block1(v2: i32):
    v3 = icmp.i32 slt v2, v0
    brif v3, block2, block3
block2:
    v4 = iconst.i32 1
    v5 = iadd.i32 v2, v4
    jump block1(v5)
block3:
    return v2
}
";
    assert_promote(input, expected);
}

#[test]
fn an_address_taken_again_in_a_loop_reads_the_loop_value() {
    // The loop's own `stack_addr` comes after the entry's store in the
    // accesses; the load must still read the parameter, not the entry's value.
    let input = "\
function @f(i32, i32) -> i32 external {
    slot0 = stack_slot 4, align 4
block0(v0: i32, v1: i32):
    v2 = stack_addr slot0
    store.i32 v1, v2, align 4
    v3 = iconst.i32 0
    jump block1(v3)
block1(v4: i32):
    v5 = stack_addr slot0
    v6 = load.i32 v5, align 4
    v7 = iadd.i32 v6, v6
    store.i32 v7, v5, align 4
    v8 = iconst.i32 1
    v9 = iadd.i32 v4, v8
    v10 = icmp.i32 slt v9, v0
    brif v10, block1(v9), block2
block2:
    v11 = load.i32 v2, align 4
    return v11
}
";
    let expected = "\
function @f(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = iconst.i32 0
    jump block1(v2, v1)
block1(v3: i32, v4: i32):
    v5 = iadd.i32 v4, v4
    v6 = iconst.i32 1
    v7 = iadd.i32 v3, v6
    v8 = icmp.i32 slt v7, v0
    brif v8, block1(v7, v5), block2
block2:
    return v5
}
";
    assert_promote(input, expected);
}

#[test]
fn a_diamond_merges_its_stores_in_a_parameter() {
    let input = "\
function @f(i1, i64, i64) -> i64 external {
    slot0 = stack_slot 8, align 8
block0(v0: i1, v1: i64, v2: i64):
    v3 = stack_addr slot0
    brif v0, block1, block2
block1:
    store.i64 v1, v3, align 8
    jump block3
block2:
    store.i64 v2, v3, align 8
    jump block3
block3:
    v4 = load.i64 v3, align 8
    return v4
}
";
    let expected = "\
function @f(i1, i64, i64) -> i64 external {
block0(v0: i1, v1: i64, v2: i64):
    brif v0, block1, block2
block1:
    jump block3(v1)
block2:
    jump block3(v2)
block3(v3: i64):
    return v3
}
";
    assert_promote(input, expected);
}

#[test]
fn several_slots_are_promoted_together() {
    // `slot0` is the sum and `slot1` the index of a loop; a third slot is a
    // pointer that only one arm stores before the join reads it.
    let input = "\
function @f(i32, ptr) -> i32 external {
    slot0 = stack_slot 4, align 4
    slot1 = stack_slot 4, align 4
    slot2 = stack_slot 8, align 8
block0(v0: i32, v1: ptr):
    v2 = stack_addr slot0
    v3 = stack_addr slot1
    v4 = stack_addr slot2
    v5 = iconst.i32 0
    store.i32 v5, v2, align 4
    store.i32 v5, v3, align 4
    store.ptr v1, v4, align 8
    jump block1
block1:
    v6 = load.i32 v3, align 4
    v7 = icmp.i32 slt v6, v0
    brif v7, block2, block3
block2:
    v8 = load.i32 v2, align 4
    v9 = iadd.i32 v8, v6
    store.i32 v9, v2, align 4
    v10 = iconst.i32 1
    v11 = iadd.i32 v6, v10
    store.i32 v11, v3, align 4
    jump block1
block3:
    v12 = load.ptr v4, align 8
    v13 = load.i32 v12, align 4
    v14 = load.i32 v2, align 4
    v15 = iadd.i32 v13, v14
    return v15
}
";
    let expected = "\
function @f(i32, ptr) -> i32 external {
block0(v0: i32, v1: ptr):
    v2 = iconst.i32 0
    jump block1(v2, v2)
block1(v3: i32, v4: i32):
    v5 = icmp.i32 slt v4, v0
    brif v5, block2, block3
block2:
    v6 = iadd.i32 v3, v4
    v7 = iconst.i32 1
    v8 = iadd.i32 v4, v7
    jump block1(v6, v8)
block3:
    v9 = load.i32 v1, align 4
    v10 = iadd.i32 v9, v3
    return v10
}
";
    assert_promote(input, expected);
}

#[test]
fn a_load_before_any_store_reads_poison() {
    let input = "\
function @f(i1) -> i32 external {
    slot0 = stack_slot 4, align 4
block0(v0: i1):
    v1 = stack_addr slot0
    brif v0, block1, block2
block1:
    v2 = iconst.i32 7
    store.i32 v2, v1, align 4
    jump block2
block2:
    v3 = load.i32 v1, align 4
    return v3
}
";
    let expected = "\
function @f(i1) -> i32 external {
block0(v0: i1):
    v1 = poison.i32
    brif v0, block1, block2(v1)
block1:
    v2 = iconst.i32 7
    jump block2(v2)
block2(v3: i32):
    return v3
}
";
    assert_promote(input, expected);
    let straight = "\
function @f() -> i8 external {
    slot0 = stack_slot 1, align 1
block0:
    v0 = stack_addr slot0
    v1 = load.i8 v0, align 1
    return v1
}
";
    let expected = "\
function @f() -> i8 external {
block0:
    v0 = poison.i8
    return v0
}
";
    assert_promote(straight, expected);
}

#[test]
fn a_store_that_nothing_reads_needs_no_parameter() {
    // The loop stores the slot, but nothing loads it after the loop, so the
    // pruned construction adds no parameter to the header.
    let input = "\
function @f(i32) external {
    slot0 = stack_slot 4, align 4
block0(v0: i32):
    v1 = stack_addr slot0
    jump block1(v0)
block1(v2: i32):
    store.i32 v2, v1, align 4
    v3 = iconst.i32 1
    v4 = isub.i32 v2, v3
    v5 = load.i32 v1, align 4
    v6 = icmp.i32 ne v5, v3
    brif v6, block1(v4), block2
block2:
    return
}
";
    let expected = "\
function @f(i32) external {
block0(v0: i32):
    jump block1(v0)
block1(v1: i32):
    v2 = iconst.i32 1
    v3 = isub.i32 v1, v2
    v4 = icmp.i32 ne v1, v2
    brif v4, block1(v3), block2
block2:
    return
}
";
    assert_promote(input, expected);
}

#[test]
fn unused_slots_and_bare_addresses_disappear() {
    let input = "\
function @f() -> i32 external {
    slot0 = stack_slot 4, align 4
    slot1 = stack_slot 16, align 8
block0:
    v0 = stack_addr slot1
    v1 = iconst.i32 3
    return v1
}
";
    let expected = "\
function @f() -> i32 external {
block0:
    v0 = iconst.i32 3
    return v0
}
";
    assert_promote(input, expected);
}

#[test]
fn unreachable_blocks_lose_their_accesses() {
    let input = "\
function @f() -> i32 external {
    slot0 = stack_slot 4, align 4
block0:
    v0 = stack_addr slot0
    v1 = iconst.i32 5
    store.i32 v1, v0, align 4
    jump block2
block1:
    v2 = load.i32 v0, align 4
    v3 = iconst.i32 1
    v4 = iadd.i32 v2, v3
    store.i32 v4, v0, align 4
    jump block2
block2:
    v5 = load.i32 v0, align 4
    return v5
}
";
    // `block2` has one reachable predecessor, so it needs no parameter, and
    // the store in `block1` is simply dropped.
    let expected = "\
function @f() -> i32 external {
block0:
    v0 = poison.i32
    v1 = iconst.i32 5
    jump block2
block1:
    v2 = iconst.i32 1
    v3 = iadd.i32 v0, v2
    jump block2
block2:
    return v1
}
";
    assert_promote(input, expected);
    // An unreachable predecessor of a join passes its own exit value.
    let join = "\
function @f(i1) -> i32 external {
    slot0 = stack_slot 4, align 4
block0(v0: i1):
    v1 = stack_addr slot0
    brif v0, block1, block2
block1:
    v2 = iconst.i32 1
    store.i32 v2, v1, align 4
    jump block4
block2:
    v3 = iconst.i32 2
    store.i32 v3, v1, align 4
    jump block4
block3:
    v4 = load.i32 v1, align 4
    v5 = iadd.i32 v4, v4
    store.i32 v5, v1, align 4
    jump block4
block4:
    v6 = load.i32 v1, align 4
    return v6
}
";
    let expected = "\
function @f(i1) -> i32 external {
block0(v0: i1):
    v1 = poison.i32
    brif v0, block1, block2
block1:
    v2 = iconst.i32 1
    jump block4(v2)
block2:
    v3 = iconst.i32 2
    jump block4(v3)
block3:
    v4 = iadd.i32 v1, v1
    jump block4(v4)
block4(v5: i32):
    return v5
}
";
    assert_promote(join, expected);
}

/// A function that stores `7` through `slot0`'s address, uses the address as
/// `use_line` says, and returns what it loads.
fn escaping(use_line: &str) -> String {
    format!(
        "function @g(ptr) external

function @f(i64) -> i32 external {{
    slot0 = stack_slot 4, align 4
    slot1 = stack_slot 8, align 8
block0(v0: i64):
    v1 = stack_addr slot0
    v2 = iconst.i32 7
    store.i32 v2, v1, align 4
    v3 = stack_addr slot1
{use_line}
    v4 = load.i32 v1, align 4
    return v4
}}
"
    )
}

#[test]
fn an_escaping_address_keeps_its_slot() {
    for use_line in [
        "    call @g(v1)",
        "    store.ptr v1, v3, align 8",
        "    v5 = ptr_add v1, v0",
        "    v5 = ptrtoint.i64 v1",
        "    copy v3, v1, v0, align 4",
        "    v5 = iconst.i8 0\n    fill v1, v5, v0, align 4",
    ] {
        let input = escaping(use_line);
        let (actual, report) = optimize_text(&input, &only(&[Pass::Promote]));
        assert!(
            actual.contains("slot0 = stack_slot 4"),
            "{use_line}\n{actual}"
        );
        assert!(actual.contains("load.i32"), "{use_line}\n{actual}");
        // `slot1` goes unless `copy` reads it: storing to it is promotable.
        assert_eq!(
            report.transformations(),
            u64::from(!use_line.contains("copy")),
            "{use_line}\n{actual}"
        );
    }
}

#[test]
fn an_address_passed_on_an_edge_or_returned_escapes() {
    let input = "\
function @f() -> ptr external {
    slot0 = stack_slot 8, align 8
block0:
    v0 = stack_addr slot0
    v1 = null
    store.ptr v1, v0, align 8
    jump block1(v0)
block1(v2: ptr):
    return v2
}
";
    assert_pass_leaves(Pass::Promote, input);
}

#[test]
fn volatile_mixed_and_partial_accesses_keep_their_slots() {
    let volatile = "\
function @f() -> i32 external {
    slot0 = stack_slot 4, align 4
block0:
    v0 = stack_addr slot0
    v1 = iconst.i32 1
    store.i32 volatile v1, v0, align 4
    v2 = load.i32 v0, align 4
    return v2
}
";
    assert_pass_leaves(Pass::Promote, volatile);
    let mixed = "\
function @f() -> f32 external {
    slot0 = stack_slot 4, align 4
block0:
    v0 = stack_addr slot0
    v1 = iconst.i32 1065353216
    store.i32 v1, v0, align 4
    v2 = load.f32 v0, align 4
    return v2
}
";
    assert_pass_leaves(Pass::Promote, mixed);
    let partial = "\
function @f() -> i32 external {
    slot0 = stack_slot 8, align 8
block0:
    v0 = stack_addr slot0
    v1 = iconst.i32 1
    store.i32 v1, v0, align 8
    v2 = load.i32 v0, align 8
    return v2
}
";
    assert_pass_leaves(Pass::Promote, partial);
}

#[test]
fn a_kept_slot_is_renumbered_after_a_promoted_one() {
    let input = "\
function @g(ptr) external

function @f() -> i32 external {
    slot0 = stack_slot 4, align 4
    slot1 = stack_slot 4, align 4
block0:
    v0 = stack_addr slot0
    v1 = stack_addr slot1
    v2 = iconst.i32 2
    store.i32 v2, v0, align 4
    call @g(v1)
    v3 = load.i32 v0, align 4
    v4 = load.i32 v1, align 4
    v5 = iadd.i32 v3, v4
    return v5
}
";
    let expected = "\
function @g(ptr) external

function @f() -> i32 external {
    slot0 = stack_slot 4, align 4
block0:
    v0 = stack_addr slot0
    v1 = iconst.i32 2
    call @g(v0)
    v2 = load.i32 v0, align 4
    v3 = iadd.i32 v1, v2
    return v3
}
";
    assert_promote(input, expected);
}

#[test]
fn each_promoted_slot_is_one_rewrite_for_bisecting() {
    let input = "\
function @f() -> i32 external {
    slot0 = stack_slot 4, align 4
    slot1 = stack_slot 4, align 4
block0:
    v0 = stack_addr slot0
    v1 = stack_addr slot1
    v2 = iconst.i32 2
    store.i32 v2, v0, align 4
    store.i32 v2, v1, align 4
    v3 = load.i32 v0, align 4
    v4 = load.i32 v1, align 4
    v5 = iadd.i32 v3, v4
    return v5
}
";
    let first_only = "\
function @f() -> i32 external {
    slot0 = stack_slot 4, align 4
block0:
    v0 = stack_addr slot0
    v1 = iconst.i32 2
    store.i32 v1, v0, align 4
    v2 = load.i32 v0, align 4
    v3 = iadd.i32 v1, v2
    return v3
}
";
    let limited = OptimizerOptions {
        bisect_limit: Some(1),
        ..only(&[Pass::Promote])
    };
    let (text, report) = optimize_text(input, &limited);
    pretty_assertions::assert_eq!(text, first_only);
    assert_eq!(report.transformations(), 1);
    assert!(report.limit_reached());
    let (_, report) = optimize_text(input, &only(&[Pass::Promote]));
    assert_eq!(report.transformations(), 2);
}
