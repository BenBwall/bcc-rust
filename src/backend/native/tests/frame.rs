//! Frame layout: IR stack slots that are never in use at once share an
//! offset; one whose address escapes, or any in a function that calls
//! `setjmp`, keeps its own.

use super::{
    with_allocation,
    with_selection,
};
use crate::{
    backend::native::{
        conv::CallConv,
        frame::Frame,
        mir::FrameObject,
    },
    target::Target,
    util::bump::Bump,
};

/// A function whose two 32-byte objects are used on the two sides of a
/// branch, as two disjoint block scopes are: `first` and `second` are
/// the statements of each side between the store and the load of its
/// object (`v2` and `v8` address the objects, `v1` is a pointer
/// parameter), and `before`, which defines `v6`, comes before the branch.
fn two_scopes(before: &str, first: &str, second: &str) -> String {
    format!(
        "abi lowered
function @use(ptr) external
function @_setjmp(ptr) -> i32 external returns_twice
function @f(i1, ptr) -> i64 external {{
    slot0 = stack_slot 32, align 8
    slot1 = stack_slot 32, align 8
block0(v0: i1, v1: ptr):
    v2 = stack_addr slot0
    v3 = stack_addr slot1
    v4 = iconst.i64 7
    v5 = iconst.i64 8
{before}
    brif v0, block1, block2
block1:
    store.i64 v4, v2, align 8
{first}
    v7 = load.i64 v2, align 8
    return v7
block2:
    v8 = ptr_add v3, v5
    store.i64 v4, v8, align 8
{second}
    v9 = load.i64 v8, align 8
    return v9
}}
"
    )
}

/// The `before` of [`two_scopes`] that does nothing else.
const NOTHING: &str = "    v6 = iconst.i64 0";

/// The slot whose offset each IR stack slot of `@f` takes.
fn homes(ir: &str) -> Vec<u32> {
    with_selection(ir, "f", Target::LinuxGnu, |_, func, _| {
        if func.slot_homes.is_empty() {
            (0..2).collect()
        } else {
            func.slot_homes.to_vec()
        }
    })
}

#[test]
fn objects_of_disjoint_scopes_share_an_offset() {
    let ir = two_scopes(NOTHING, "", "");
    assert_eq!(homes(&ir), [0, 0], "{ir}");
    // The frame holds one object's 32 bytes, and both have its offset.
    with_allocation(&ir, "f", Target::LinuxGnu, |_, func, allocation| {
        let arena = Bump::new();
        let frame = Frame::layout(func, allocation, CallConv::of(Target::LinuxGnu), &arena)
            .unwrap_or_else(|_| panic!("the frame is laid out"));
        assert_eq!(
            frame.address(FrameObject::Slot(0)),
            frame.address(FrameObject::Slot(1))
        );
        assert!(
            frame.size < 64,
            "{} bytes for two shared objects",
            frame.size
        );
    });
}

#[test]
fn an_object_whose_address_escapes_keeps_its_offset() {
    // `use` may keep the address and reach the object through it until its
    // scope ends, which the IR does not mark.
    for (first, second) in [("    call @use(v2)", ""), ("", "    call @use(v8)")] {
        let ir = two_scopes(NOTHING, first, second);
        assert_eq!(homes(&ir), [0, 1], "{ir}");
    }
    // Stored into memory, or compared, the address escapes as well.
    let stored = two_scopes(NOTHING, "    store.ptr v2, v1, align 8", "");
    assert_eq!(homes(&stored), [0, 1], "{stored}");
    let compared = two_scopes("    v6 = icmp.ptr eq v2, v3", "", "");
    assert_eq!(homes(&compared), [0, 1], "{compared}");
}

#[test]
fn objects_in_use_at_once_keep_their_offsets() {
    // The second object is written while the first holds its value.
    let ir = two_scopes(NOTHING, "    store.i64 v4, v3, align 8", "");
    assert_eq!(homes(&ir), [0, 1], "{ir}");
    // In a loop, the first object keeps the value stored before it from
    // one iteration to the next, across the second's use.
    let looped = "abi lowered
function @f(i1) -> i64 external {
    slot0 = stack_slot 8, align 8
    slot1 = stack_slot 8, align 8
block0(v0: i1):
    v1 = stack_addr slot0
    v2 = stack_addr slot1
    v3 = iconst.i64 7
    store.i64 v3, v1, align 8
    jump block1
block1:
    store.i64 v3, v2, align 8
    v4 = load.i64 v2, align 8
    v5 = load.i64 v1, align 8
    brif v0, block1, block2
block2:
    v6 = iadd.i64 v4, v5
    return v6
}
";
    assert_eq!(homes(looped), [0, 1], "{looped}");
}

#[test]
fn objects_written_whole_in_each_iteration_share_an_offset() {
    // Each iteration stores each object whole before it reads it, so
    // neither keeps a value across the other's use.
    let ir = "abi lowered
function @f(i1) -> i64 external {
    slot0 = stack_slot 8, align 8
    slot1 = stack_slot 8, align 8
block0(v0: i1):
    v1 = stack_addr slot0
    v2 = stack_addr slot1
    v3 = iconst.i64 7
    jump block1
block1:
    store.i64 v3, v1, align 8
    v4 = load.i64 v1, align 8
    store.i64 v4, v2, align 8
    v5 = load.i64 v2, align 8
    brif v0, block1, block2
block2:
    return v5
}
";
    assert_eq!(homes(ir), [0, 0], "{ir}");
}

#[test]
fn functions_that_call_setjmp_share_no_offset() {
    let ir = two_scopes("    v6 = call @_setjmp(v1)", "", "");
    assert_eq!(homes(&ir), [0, 1], "{ir}");
}
