//! Non-local jumps: `setjmp` returning again through `longjmp` from deeper
//! frames, the value 0 delivered as 1, buffers whose frame has returned,
//! MinGW-w64's `mingw_getsp`, corrupted buffers, dynamic objects across a
//! jump, the accepted `setjmp` shapes, and returns-twice functions without
//! a model.
//! C99: §7.13.2.1 paragraphs 2-4, p. 244; PDF p. 256.

use super::*;

/// `@main` saves into a stack buffer, calls `@thrower` (two frames deep)
/// with `VALUE`, and returns what the second `setjmp` return gave plus the
/// number of times the first branch ran, read from a slot through
/// volatile accesses, as C requires of a local changed between the two
/// (C99 §7.13.2.1p3).
const THROW: &str = "\
function @setjmp(ptr) -> i32 external
function @longjmp(ptr, i32) external

function @thrower(ptr, i32) internal {
block0(v0: ptr, v1: i32):
    call @longjmp(v0, v1)
    unreachable
}

function @middle(ptr, i32) -> i32 internal {
    slot0 = stack_slot 16, align 8
block0(v0: ptr, v1: i32):
    call @thrower(v0, v1)
    v2 = iconst.i32 100
    return v2
}

function @main() -> i32 external {
    slot0 = stack_slot 200, align 8
    slot1 = stack_slot 4, align 4
block0:
    v0 = stack_addr slot0
    v1 = stack_addr slot1
    v2 = iconst.i32 0
    store.i32 v2, v1, align 4
    v3 = call @setjmp(v0)
    v4 = icmp.i32 eq v3, v2
    brif v4, block1, block2
block1:
    v5 = load.i32 volatile v1, align 4
    v6 = iconst.i32 1
    v7 = iadd.i32 v5, v6
    store.i32 volatile v7, v1, align 4
    v8 = iconst.i32 VALUE
    v9 = call @middle(v0, v8)
    return v9
block2:
    v10 = load.i32 volatile v1, align 4
    v11 = iadd.i32 v3, v10
    return v11
}
";

#[test]
fn longjmp_returns_from_setjmp_again_with_its_value() {
    assert_eq!(returned(&THROW.replace("VALUE", "9"), "main"), int(10));
    // C99 §7.13.2.1p4: setjmp cannot return 0 through longjmp.
    assert_eq!(returned(&THROW.replace("VALUE", "0"), "main"), int(2));
}

#[test]
fn longjmp_to_a_returned_frame_is_undefined() {
    let text = "\
function @setjmp(ptr) -> i32 external
function @longjmp(ptr, i32) external

function @save(ptr) -> i32 internal {
block0(v0: ptr):
    v1 = call @setjmp(v0)
    return v1
}

function @main() -> i32 external {
    slot0 = stack_slot 200, align 8
block0:
    v0 = stack_addr slot0
    v1 = call @save(v0)
    v2 = iconst.i32 1
    call @longjmp(v0, v2)
    return v1
}
";
    assert_eq!(undefined(text, "main"), UbKind::InvalidJmpBuf);
    // A buffer no setjmp filled holds poison.
    let unfilled = text.replace("v1 = call @save(v0)", "v1 = iconst.i32 0");
    assert_eq!(undefined(&unfilled, "main"), UbKind::InvalidJmpBuf);
}

#[test]
fn mingw_setjmp_takes_the_stack_pointer() {
    // MinGW-w64's non-Clang `setjmp(b)` is `_setjmp(b, mingw_getsp())`.
    let text = "\
function @mingw_getsp() -> ptr external
function @_setjmp(ptr, ptr) -> i32 external returns_twice
function @longjmp(ptr, i32) external

function @main() -> i32 external {
    slot0 = stack_slot 256, align 16
block0:
    v0 = stack_addr slot0
    v1 = call @mingw_getsp()
    v2 = call @_setjmp(v0, v1)
    v3 = iconst.i32 0
    v4 = icmp.i32 eq v2, v3
    brif v4, block1, block2
block1:
    v5 = iconst.i32 6
    call @longjmp(v0, v5)
    unreachable
block2:
    return v2
}
";
    assert_eq!(returned(text, "main"), int(6));
}

#[test]
fn other_returns_twice_functions_are_unsupported() {
    for declaration in [
        "function @vfork() -> i32 external",
        "function @vfork() -> i32 external returns_twice",
        "function @save_context() -> i32 external returns_twice",
    ] {
        let name = if declaration.contains("vfork") {
            "vfork"
        } else {
            "save_context"
        };
        let text = format!(
            "{declaration}\nfunction @main() -> i32 external {{\nblock0:\n    v0 = call \
             @{name}()\n    return v0\n}}\n"
        );
        match run_text(&text, "main").result {
            | Err(Trap::Unsupported(trap::Unsupported::ReturnsTwice, _)) => {},
            | other => panic!("{declaration}: {other:?}"),
        }
    }
}

#[test]
fn environment_functions_check_their_signatures() {
    for (declaration, call) in [
        (
            "function @setjmp(ptr) -> i64 external",
            "v1 = call @setjmp(v0)",
        ),
        (
            "function @setjmp(i64) -> i32 external",
            "v1 = call @setjmp(v2)",
        ),
        (
            "function @longjmp(ptr) external",
            "call @longjmp(v0)\n    v1 = iconst.i32 0",
        ),
    ] {
        let text = format!(
            "{declaration}\nfunction @main() -> i32 external {{\n    slot0 = stack_slot 200, \
             align 8\nblock0:\n    v0 = stack_addr slot0\n    v2 = iconst.i64 0\n    {call}\n    \
             v3 = iconst.i32 0\n    return v3\n}}\n"
        );
        assert_eq!(
            undefined(&text, "main"),
            UbKind::SignatureMismatch,
            "{declaration}"
        );
    }
}

#[test]
fn a_corrupted_resume_block_traps_instead_of_panicking() {
    // Word 2 of the record is the resume block; u64::MAX names no block.
    let text = "\
function @setjmp(ptr) -> i32 external
function @longjmp(ptr, i32) external

function @main() -> i32 external {
    slot0 = stack_slot 200, align 8
block0:
    v0 = stack_addr slot0
    v1 = call @setjmp(v0)
    v2 = iconst.i32 0
    v3 = icmp.i32 eq v1, v2
    brif v3, block1, block2
block1:
    v4 = iconst.i64 16
    v5 = ptr_add v0, v4
    v6 = iconst.i64 -1
    store.i64 v6, v5, align 8
    v7 = iconst.i32 1
    call @longjmp(v0, v7)
    unreachable
block2:
    return v1
}
";
    assert_eq!(undefined(text, "main"), UbKind::InvalidJmpBuf);
}

/// `@main(mode)` allocates one dynamic object before its `setjmp` and one
/// after it, and `@deep` allocates its own before jumping back. Mode 0
/// reads the first, which the jump keeps; modes 1 and 2 read the other two,
/// which the jump releases with the stack they lived on.
/// C99: §7.13.2.1 paragraph 2, p. 244; PDF p. 256.
const DYNAMIC: &str = "\
function @setjmp(ptr) -> i32 external
function @longjmp(ptr, i32) external

function @deep(ptr, ptr) internal {
block0(v0: ptr, v1: ptr):
    v2 = iconst.i64 4
    v3 = dyn_alloca.i64 v2, align 4
    v4 = iconst.i32 9
    store.i32 v4, v3, align 4
    store.ptr v3, v1, align 8
    v5 = iconst.i32 1
    call @longjmp(v0, v5)
    unreachable
}

function @main(i32) -> i32 external {
    slot0 = stack_slot 200, align 8
    slot1 = stack_slot 8, align 8
    slot2 = stack_slot 8, align 8
block0(v0: i32):
    v1 = stack_addr slot0
    v2 = stack_addr slot1
    v3 = stack_addr slot2
    v4 = iconst.i64 4
    v5 = dyn_alloca.i64 v4, align 4
    v6 = iconst.i32 7
    store.i32 v6, v5, align 4
    v7 = call @setjmp(v1)
    v8 = iconst.i32 0
    v9 = icmp.i32 eq v7, v8
    brif v9, block1, block2
block1:
    v10 = dyn_alloca.i64 v4, align 4
    store.i32 v6, v10, align 4
    store.ptr volatile v10, v2, align 8
    call @deep(v1, v3)
    unreachable
block2:
    v11 = iconst.i32 1
    v12 = icmp.i32 eq v0, v11
    brif v12, block3, block4
block3:
    v13 = load.ptr volatile v2, align 8
    v14 = load.i32 v13, align 4
    return v14
block4:
    v15 = iconst.i32 2
    v16 = icmp.i32 eq v0, v15
    brif v16, block5, block6
block5:
    v17 = load.ptr volatile v3, align 8
    v18 = load.i32 v17, align 4
    return v18
block6:
    v19 = load.i32 v5, align 4
    return v19
}
";

#[test]
fn longjmp_keeps_older_dynamic_objects_and_releases_newer_ones() {
    assert_eq!(returned_with(DYNAMIC, "main", &[int(0)]), int(7));
    for mode in [1, 2] {
        match run_text_with(DYNAMIC, "main", &[int(mode)], Limits::default()).result {
            | Err(Trap::UndefinedBehavior(UbKind::DanglingPointer, _)) => {},
            | other => panic!("mode {mode}: {other:?}"),
        }
    }
}

#[test]
fn setjmp_accepts_the_three_x86_64_library_shapes_only() {
    let program = |params: &str, args: &str| {
        format!(
            "function @sigsetjmp({params}) -> i32 external\nfunction @main() -> i32 external {{\n    \
             slot0 = stack_slot 200, align 8\nblock0:\n    v0 = stack_addr slot0\n    v1 = \
             iconst.i32 1\n    v2 = iconst.i64 1\n    v3 = call @sigsetjmp({args})\n    return \
             v3\n}}\n"
        )
    };
    // glibc's buffer alone, `sigsetjmp`'s mask flag, MinGW-w64's frame.
    for (params, args) in [
        ("ptr", "v0"),
        ("ptr, i32", "v0, v1"),
        ("ptr, ptr", "v0, v0"),
    ] {
        assert_eq!(returned(&program(params, args), "main"), int(0), "{params}");
    }
    for (params, args) in [
        ("ptr, i64", "v0, v2"),
        ("ptr, ptr, ptr", "v0, v0, v0"),
        ("ptr, i32, ...", "v0, v1, v1"),
    ] {
        assert_eq!(
            undefined(&program(params, args), "main"),
            UbKind::SignatureMismatch,
            "{params}"
        );
    }
}
