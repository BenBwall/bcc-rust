//! Memory: stack slots, globals with relocations, pointer arithmetic,
//! copies and fills, provenance, and every undefined access.

use super::*;

#[test]
fn stack_slots_store_little_endian_bytes() {
    let text = "\
function @f() -> i64 external {
    slot0 = stack_slot 8, align 8
block0:
    v0 = stack_addr slot0
    v1 = iconst.i32 287454020
    store.i32 v1, v0, align 8
    v2 = iconst.i64 4
    v3 = ptr_add inbounds v0, v2
    v4 = iconst.i32 1432778632
    store.i32 v4, v3, align 4
    v5 = load.i64 v0, align 8
    return v5
}
";
    assert_eq!(returned(text, "f"), int(0x5566_7788_1122_3344));
}

#[test]
fn unwritten_and_poisoned_bytes_load_as_poison() {
    let text = "\
function @f() -> i32 external {
    slot0 = stack_slot 4, align 4
block0:
    v0 = stack_addr slot0
    v1 = iconst.i16 7
    store.i16 v1, v0, align 2
    v2 = load.i32 v0, align 4
    return v2
}
";
    assert_eq!(returned(text, "f"), RuntimeValue::Poison);
    let text = "\
function @f() -> i8 external {
    slot0 = stack_slot 1, align 1
block0:
    v0 = stack_addr slot0
    v1 = poison.i8
    store.i8 v1, v0, align 1
    v2 = load.i8 v0, align 1
    return v2
}
";
    assert_eq!(returned(text, "f"), RuntimeValue::Poison);
    let text = "\
function @f() -> i1 external {
    slot0 = stack_slot 1, align 1
block0:
    v0 = stack_addr slot0
    v1 = iconst.i8 2
    store.i8 v1, v0, align 1
    v2 = load.i1 v0, align 1
    return v2
}
";
    assert_eq!(
        returned(text, "f"),
        RuntimeValue::Poison,
        "an i1 load of a byte other than 0 or 1"
    );
}

#[test]
fn globals_hold_initializers_and_relocations() {
    let text = format!(
        "{}global @table internal constant size 16, align 8 = bytes \
         \"00000000000000000000000000000000\" relocs [0: @message + 1, 8: @counter]
global @counter internal size 4, align 4 = zero

function @f() -> i32 external {{
block0:
    v0 = global_addr @table
    v1 = load.ptr v0, align 8
    v2 = load.i8 v1, align 1
    v3 = zext.i32 v2
    v4 = iconst.i64 8
    v5 = ptr_add inbounds v0, v4
    v6 = load.ptr v5, align 8
    v7 = load.i32 v6, align 4
    v8 = iadd.i32 v7, v3
    store.i32 v8, v6, align 4
    v9 = global_addr @counter
    v10 = load.i32 v9, align 4
    return v10
}}
",
        c_string("message", "hi")
    );
    assert_eq!(returned(&text, "f"), int(u128::from(b'i')));
}

#[test]
fn pointer_arithmetic_walks_an_array() {
    let text = "\
function @f() -> i32 external {
    slot0 = stack_slot 16, align 4
block0:
    v0 = stack_addr slot0
    v1 = iconst.i64 0
    jump block1(v1)
block1(v2: i64):
    v3 = iconst.i64 4
    v4 = imul.i64 v2, v3
    v5 = ptr_add inbounds v0, v4
    v6 = trunc.i32 v2
    v7 = imul.i32 v6, v6
    store.i32 v7, v5, align 4
    v8 = iconst.i64 1
    v9 = iadd.i64 v2, v8
    v10 = icmp.i64 ult v9, v3
    brif v10, block1(v9), block2
block2:
    v11 = iconst.i64 16
    v12 = ptr_add inbounds v0, v11
    v13 = iconst.i32 0
    jump block3(v0, v13)
block3(v14: ptr, v15: i32):
    v16 = icmp.ptr eq v14, v12
    brif v16, block5, block4
block4:
    v17 = load.i32 v14, align 4
    v18 = iadd.i32 v15, v17
    v19 = iconst.i64 4
    v20 = ptr_add inbounds v14, v19
    jump block3(v20, v18)
block5:
    return v15
}
";
    // 0 + 1 + 4 + 9.
    assert_eq!(returned(text, "f"), int(14));
}

#[test]
fn copies_and_fills_move_bytes_and_provenance() {
    let text = "\
function @f() -> i32 external {
    slot0 = stack_slot 16, align 8
    slot1 = stack_slot 16, align 8
    slot2 = stack_slot 4, align 4
block0:
    v0 = stack_addr slot0
    v1 = stack_addr slot1
    v2 = stack_addr slot2
    v3 = iconst.i32 40
    store.i32 v3, v2, align 4
    store.ptr v2, v0, align 8
    v4 = iconst.i64 8
    v5 = ptr_add inbounds v0, v4
    v6 = iconst.i8 2
    fill v5, v6, v4, align 8
    v7 = iconst.i64 16
    copy v1, v0, v7, align 8
    v8 = load.ptr v1, align 8
    v9 = load.i32 v8, align 4
    v10 = ptr_add inbounds v1, v4
    v11 = load.i8 v10, align 1
    v12 = zext.i32 v11
    v13 = iadd.i32 v9, v12
    return v13
}
";
    assert_eq!(returned(text, "f"), int(42));
}

#[test]
fn overlapping_copies_need_may_overlap() {
    let program = |flags: &str| {
        format!(
            "function @f() -> i32 external {{
    slot0 = stack_slot 8, align 4
block0:
    v0 = stack_addr slot0
    v1 = iconst.i32 1
    store.i32 v1, v0, align 4
    v2 = iconst.i64 4
    v3 = ptr_add inbounds v0, v2
    v4 = iconst.i32 2
    store.i32 v4, v3, align 4
    v5 = iconst.i64 2
    v6 = ptr_add inbounds v0, v5
    v7 = iconst.i64 6
    copy {flags}v6, v0, v7, align 1
    v8 = load.i32 v3, align 4
    return v8
}}
"
        )
    };
    assert_eq!(undefined(&program(""), "f"), UbKind::OverlappingCopy);
    // memmove of bytes 01 00 00 00 02 00 two bytes up gives
    // xx xx 01 00 00 00 02 00, so the second word is 0x0002_0000.
    assert_eq!(returned(&program("may_overlap "), "f"), int(0x0002_0000));
}

#[test]
fn integers_and_pointers_convert_through_addresses() {
    let text = "\
function @f() -> i32 external {
    slot0 = stack_slot 8, align 4
block0:
    v0 = stack_addr slot0
    v1 = iconst.i64 4
    v2 = ptr_add inbounds v0, v1
    v3 = iconst.i32 9
    store.i32 v3, v2, align 4
    v4 = ptrtoint.i64 v0
    v5 = iadd.i64 v4, v1
    v6 = inttoptr.ptr v5
    v7 = load.i32 v6, align 4
    v8 = icmp.ptr eq v6, v2
    v9 = zext.i32 v8
    v10 = iadd.i32 v7, v9
    return v10
}
";
    assert_eq!(returned(text, "f"), int(10));
    let text = "\
function @f() -> i32 external {
block0:
    v0 = iconst.i64 12345
    v1 = inttoptr.ptr v0
    v2 = load.i32 v1, align 4
    return v2
}
";
    assert_eq!(undefined(text, "f"), UbKind::NoProvenance);
}

#[test]
fn inbounds_offsets_leaving_their_object_are_poison() {
    let offset = |inbounds: &str, bytes: i64| {
        let text = format!(
            "function @f() -> ptr external {{
    slot0 = stack_slot 8, align 8
block0:
    v0 = stack_addr slot0
    v1 = iconst.i64 {bytes}
    v2 = ptr_add {inbounds}v0, v1
    v3 = ptr_add {inbounds}v2, v1
    return v3
}}
"
        );
        returned(&text, "f")
    };
    assert!(matches!(offset("inbounds ", 4), RuntimeValue::Ptr(_)));
    assert_eq!(offset("inbounds ", 5), RuntimeValue::Poison);
    assert_eq!(offset("inbounds ", -1), RuntimeValue::Poison);
    assert!(matches!(offset("", 5), RuntimeValue::Ptr(_)));
    let text = "\
function @f() -> i32 external {
    slot0 = stack_slot 8, align 8
block0:
    v0 = stack_addr slot0
    v1 = iconst.i64 8
    v2 = ptr_add v0, v1
    v3 = load.i32 v2, align 4
    return v3
}
";
    assert_eq!(undefined(text, "f"), UbKind::OutOfBounds);
}

#[test]
fn bad_accesses_are_undefined() {
    let access = |setup: &str, access: &str| {
        let text = format!(
            "{}function @f() -> i32 external {{
    slot0 = stack_slot 8, align 4
block0:
    v0 = stack_addr slot0
    v1 = iconst.i32 0
    store.i32 v1, v0, align 4
    {setup}
    {access}
    return v1
}}
",
            c_string("text", "abc")
        );
        undefined(&text, "f")
    };
    let cases = [
        (
            "v2 = null",
            "v3 = load.i32 v2, align 4",
            UbKind::NullDereference,
        ),
        (
            "v2 = poison.ptr",
            "v3 = load.i32 v2, align 4",
            UbKind::PoisonAddress,
        ),
        (
            "v2 = iconst.i64 6",
            "v3 = ptr_add v0, v2\n    v4 = load.i32 v3, align 4",
            UbKind::OutOfBounds,
        ),
        (
            "v2 = iconst.i64 1",
            "v3 = ptr_add v0, v2\n    v4 = load.i32 v3, align 4",
            UbKind::MisalignedAccess,
        ),
        (
            "v2 = global_addr @text",
            "store.i32 v1, v2, align 1",
            UbKind::WriteToConstant,
        ),
        (
            "v2 = func_addr @f",
            "v3 = load.i32 v2, align 1",
            UbKind::NotData,
        ),
        (
            "v2 = poison.i64",
            "copy v0, v0, v2, align 1",
            UbKind::PoisonAddress,
        ),
        (
            "v2 = iconst.i64 9",
            "v3 = iconst.i8 0\n    fill v0, v3, v2, align 1",
            UbKind::OutOfBounds,
        ),
    ];
    for (setup, operation, expected) in cases {
        assert_eq!(access(setup, operation), expected, "{setup}; {operation}");
    }
}

#[test]
fn slots_die_when_their_function_returns() {
    let text = "\
function @leak() -> ptr internal {
    slot0 = stack_slot 4, align 4
block0:
    v0 = stack_addr slot0
    v1 = iconst.i32 1
    store.i32 v1, v0, align 4
    return v0
}

function @f() -> i32 external {
block0:
    v0 = call @leak()
    v1 = call @leak()
    v2 = load.i32 v0, align 4
    return v2
}
";
    assert_eq!(undefined(text, "f"), UbKind::DanglingPointer);
}
