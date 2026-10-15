//! Load CSE, forwarding, alias proofs and memory-scope boundaries on textual
//! IR.

use super::*;

#[test]
fn redundant_loads_in_a_block_and_a_single_predecessor_child() {
    let input = "\
function @f(ptr) -> i32 external {
block0(v0: ptr):
    v1 = load.i32 v0, align 4
    v2 = load.i32 v0, align 4
    jump block1
block1:
    v3 = load.i32 v0, align 4
    v4 = iadd.i32 v2, v3
    return v4
}
";
    let expected = "\
function @f(ptr) -> i32 external {
block0(v0: ptr):
    v1 = load.i32 v0, align 4
    jump block1
block1:
    v2 = iadd.i32 v1, v1
    return v2
}
";
    let (actual, report) = optimize_text(input, &only(&[Pass::Gvn]));
    assert_eq!(actual, expected);
    assert_eq!(report.transformations(), 2);
    for limit in 0..=2 {
        let options = OptimizerOptions {
            bisect_limit: Some(limit),
            ..only(&[Pass::Gvn])
        };
        let (actual, report) = optimize_text(input, &options);
        assert_eq!(report.transformations(), limit);
        assert_eq!(
            actual.matches("load.i32").count(),
            3 - usize::try_from(limit).unwrap()
        );
    }
}

#[test]
fn store_forwarding_crosses_a_single_predecessor_edge() {
    assert_pass(
        Pass::Gvn,
        "\
function @f(ptr, i32) -> i32 external {
block0(v0: ptr, v1: i32):
    store.i32 v1, v0, align 4
    jump block1
block1:
    v2 = load.i32 v0, align 4
    return v2
}
",
        "\
function @f(ptr, i32) -> i32 external {
block0(v0: ptr, v1: i32):
    store.i32 v1, v0, align 4
    jump block1
block1:
    return v1
}
",
    );
}

#[test]
fn barriers_keep_both_loads() {
    for barrier in [
        "store.i32 v2, v1, align 4",
        "call @g(v0)",
        "call_indirect v4(v0) : (ptr)",
        "v6 = load.i32 volatile v1, align 4",
        "store.i32 volatile v2, v1, align 4",
        "copy v1, v0, v3, align 1",
        "fill v1, v5, v3, align 1",
        "va_start v1",
        "v6 = va_arg.i32 v1",
        "va_copy v1, v0",
        "va_end v1",
    ] {
        let last = if barrier.starts_with("v6 =") { 8 } else { 7 };
        let barrier = barrier.replace("v6 =", "v7 =");
        let text = format!(
            "\
function @g(ptr) external

function @f(ptr, ptr, i32, i64, ptr, i8, ...) -> i32 external {{
block0(v0: ptr, v1: ptr, v2: i32, v3: i64, v4: ptr, v5: i8):
    v6 = load.i32 v0, align 4
    {barrier}
    v{last} = load.i32 v0, align 4
    return v{last}
}}
"
        );
        let (actual, report) = optimize_text(&text, &only(&[Pass::Gvn]));
        assert_eq!(report.transformations(), 0, "{barrier}:\n{actual}");
        assert_eq!(
            actual.matches(" = load.i32 v0,").count(),
            2,
            "{barrier}:\n{actual}"
        );
    }
}

#[test]
fn distinct_slots_and_globals_do_not_alias() {
    for (prefix, addresses) in [
        (
            "",
            "    slot0 = stack_slot 4, align 4\n    slot1 = stack_slot 4, align 4\n",
        ),
        (
            "global @a internal size 4, align 4 = zero\nglobal @b internal size 4, align 4 = \
             zero\n\n",
            "",
        ),
    ] {
        let (a, b) = if prefix.is_empty() {
            ("stack_addr slot0", "stack_addr slot1")
        } else {
            ("global_addr @a", "global_addr @b")
        };
        let text = format!(
            "{prefix}function @f(i32) -> i32 external {{
{addresses}block0(v0: i32):
    v1 = {a}
    v2 = {b}
    v3 = load.i32 v1, align 4
    store.i32 v0, v2, align 4
    v4 = load.i32 v1, align 4
    return v4
}}
"
        );
        let (actual, report) = optimize_text(&text, &only(&[Pass::Gvn]));
        assert_eq!(report.transformations(), 1, "{actual}");
        assert_eq!(actual.matches("load.i32").count(), 1, "{actual}");
    }
}

#[test]
fn constant_offsets_use_the_entire_access_range() {
    for (offset, load_ty, store_ty, rewrites) in [
        (4, "i32", "i32", 1),
        (-4, "i32", "i32", 1),
        (3, "i32", "i32", 0),
        (4, "i64", "i32", 0),
        (8, "i64", "i32", 1),
        (i64::MAX, "i32", "i32", 0),
    ] {
        let text = format!(
            "\
function @f(ptr, {store_ty}) -> {load_ty} external {{
block0(v0: ptr, v1: {store_ty}):
    v2 = iconst.i64 {offset}
    v3 = ptr_add v0, v2
    v4 = load.{load_ty} v0, align 1
    store.{store_ty} v1, v3, align 1
    v5 = load.{load_ty} v0, align 1
    return v5
}}
"
        );
        let (actual, report) = optimize_text(&text, &only(&[Pass::Gvn]));
        assert_eq!(report.transformations(), rewrites, "{text}\n{actual}");
    }
}

#[test]
fn private_slot_survives_an_unknown_store_but_escaped_slots_do_not() {
    for escape in [
        "",
        "    store.ptr v5, v1, align 8\n",
        "    call @g(v5)\n",
        "    v9 = ptrtoint.i64 v5\n",
        "    v9 = select.ptr v2, v5, v0\n",
    ] {
        // Escape is after the two loads: the analysis must inspect the whole
        // function, and must follow the derived address back to slot0.
        let text = format!(
            "\
function @g(ptr) external

function @f(ptr, ptr, i1, i64) -> i32 external {{
    slot0 = stack_slot 32, align 4
block0(v0: ptr, v1: ptr, v2: i1, v3: i64):
    v4 = stack_addr slot0
    v5 = ptr_add v4, v3
    v6 = load.i32 v5, align 4
    v7 = iconst.i32 7
    store.i32 v7, v0, align 4
    v8 = load.i32 v5, align 4
{escape}    return v8
}}
"
        );
        let (actual, report) = optimize_text(&text, &only(&[Pass::Gvn]));
        assert_eq!(
            report.transformations(),
            u64::from(escape.is_empty()),
            "{actual}"
        );
    }
}

#[test]
fn merge_points_and_loop_headers_keep_facts_no_incoming_path_clobbers() {
    // A store through the same pointer in either arm, a call, or a store
    // through a pointer that may alias reaches the join; an empty arm or a
    // store to a slot does not.
    for (arm, reused) in [
        ("", true),
        ("    store.i32 v2, v0, align 4\n", false),
        ("    call @g()\n", false),
        ("    store.i32 v2, v4, align 4\n", false),
        ("    store.i32 v2, v5, align 4\n", true),
    ] {
        let text = format!(
            "\
global @other internal size 4, align 4 = zero

function @g() external

function @f(ptr, i1, i32) -> i32 external {{
    slot0 = stack_slot 4, align 4
block0(v0: ptr, v1: i1, v2: i32):
    v3 = load.i32 v0, align 4
    v4 = global_addr @other
    v5 = stack_addr slot0
    brif v1, block1, block2
block1:
{arm}    jump block3
block2:
    jump block3
block3:
    v6 = load.i32 v0, align 4
    return v6
}}
"
        );
        let (_, report) = optimize_text(&text, &only(&[Pass::Gvn]));
        assert_eq!(report.transformations(), u64::from(reused), "{arm}");
    }
    // A loop whose body writes nothing keeps the preheader's load.
    let unclobbered = "\
function @f(ptr, i32) -> i32 external {
block0(v0: ptr, v1: i32):
    v2 = load.i32 v0, align 4
    v3 = iconst.i32 0
    jump block1(v3)
block1(v4: i32):
    v5 = load.i32 v0, align 4
    v6 = iadd.i32 v4, v5
    v7 = icmp.i32 slt v6, v1
    brif v7, block1(v6), block2
block2:
    return v2
}
";
    let (_, report) = optimize_text(unclobbered, &only(&[Pass::Gvn]));
    assert_eq!(report.transformations(), 1);
    let text = "\
function @f(ptr, i1, i32) -> i32 external {
block0(v0: ptr, v1: i1, v2: i32):
    store.i32 v2, v0, align 4
    jump block1
block1:
    v3 = load.i32 v0, align 4
    brif v1, block2, block3
block2:
    store.i32 v2, v0, align 4
    jump block1
block3:
    return v3
}
";
    assert_pass_leaves(Pass::Gvn, text);
}

#[test]
fn sibling_scopes_restore_overwritten_facts_and_generations() {
    let text = "\
function @g() external

function @f(ptr, i1, i32) -> i32 external {
block0(v0: ptr, v1: i1, v2: i32):
    v3 = load.i32 v0, align 4
    brif v1, block1, block2
block1:
    store.i32 v2, v0, align 4
    v4 = load.i32 v0, align 4
    call @g()
    v5 = load.i32 v0, align 4
    v6 = iadd.i32 v4, v5
    return v6
block2:
    v7 = load.i32 v0, align 4
    return v7
}
";
    let expected = "\
function @g() external

function @f(ptr, i1, i32) -> i32 external {
block0(v0: ptr, v1: i1, v2: i32):
    v3 = load.i32 v0, align 4
    brif v1, block1, block2
block1:
    store.i32 v2, v0, align 4
    call @g()
    v4 = load.i32 v0, align 4
    v5 = iadd.i32 v2, v4
    return v5
block2:
    return v3
}
";
    assert_pass(Pass::Gvn, text, expected);
}

#[test]
fn equal_width_type_mismatches_are_not_forwarded_or_reused() {
    let text = "\
function @f(ptr, i32) -> f32 external {
block0(v0: ptr, v1: i32):
    v2 = load.i32 v0, align 4
    v3 = load.f32 v0, align 4
    store.i32 v1, v0, align 4
    v4 = load.f32 v0, align 4
    return v4
}
";
    assert_pass_leaves(Pass::Gvn, text);
}

#[test]
fn returned_slot_addresses_escape() {
    let text = "\
function @f(ptr, i32) -> ptr external {
    slot0 = stack_slot 4, align 4
block0(v0: ptr, v1: i32):
    v2 = stack_addr slot0
    v3 = load.i32 v2, align 4
    store.i32 v1, v0, align 4
    v4 = load.i32 v2, align 4
    return v2
}
";
    assert_pass_leaves(Pass::Gvn, text);
}
