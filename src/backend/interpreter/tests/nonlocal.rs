//! Non-local jumps: `setjmp` returning again through `longjmp` from deeper
//! frames, the value 0 delivered as 1, buffers whose frame has returned,
//! MinGW-w64's `mingw_getsp`, and returns-twice functions without a model.
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
