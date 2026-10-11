//! Callers and callees rewritten from textual IR: each module is lowered
//! for System V and Win64 and compared with a snapshot under
//! `tests/fixtures/abi/` (`BLESS=1` rewrites them), and run in the
//! interpreter before and after lowering, which must agree.

use std::fmt::Write as _;

use super::*;
use crate::{
    backend::interpreter::{
        DefaultHost,
        Outcome,
        RuntimeValue,
        Termination,
        run,
    },
    test_support::assert_snapshot,
};

/// Lowers `text` for System V and Win64, compares each with the snapshot
/// `name`, and checks that `@main` returns `expected` before and after.
#[track_caller]
fn check(name: &str, text: &str, expected: u128) {
    let mut snapshot = String::new();
    for (target, suffix) in [(Target::LinuxGnu, "sysv"), (Target::WindowsGnu, "win64")] {
        let lowered = lower_text(text, target);
        assert_eq!(main_result(text), expected, "{name} before ABI lowering");
        assert_eq!(
            main_result(&lowered),
            expected,
            "{name} after ABI lowering for {suffix}:\n{lowered}"
        );
        writeln!(snapshot, "; {suffix}\n{lowered}").unwrap();
    }
    assert_snapshot(&format!("tests/fixtures/abi/{name}.ir"), &snapshot);
}

/// What `@main` of `text` returns in the interpreter, as integer bits.
#[track_caller]
pub(super) fn main_result(text: &str) -> u128 {
    let arena = Bump::new();
    let module = parse_module(&arena, text).unwrap_or_else(|error| panic!("{error}\n{text}"));
    let mut host = DefaultHost::new_in(&arena);
    match run(&module, "main", &[], &mut host) {
        | Ok(Outcome {
            termination: Termination::Returned(Some(RuntimeValue::Int(bits))),
            ..
        }) => bits,
        | other => panic!(
            "{:?}\n{text}",
            other.map_err(|trap| trap.display_in(&module).to_string())
        ),
    }
}

/// A 12-byte structure of an `int` and two `float`s in, a 16-byte pair of
/// `long`s out: System V passes both in registers, Win64 neither.
#[test]
fn registers_or_references() {
    check(
        "registers",
        "\
function @pair(ptr result(size 16, align 8, [0: i64, 8: i64]), ptr aggregate(size 12, align 4, [0: \
         i32, 4: f32, 8: f32])) external {
block0(v0: ptr, v1: ptr):
    v2 = load.i32 v1, align 4
    v3 = sext.i64 v2
    store.i64 v3, v0, align 8
    v4 = iconst.i64 8
    v5 = ptr_add inbounds v1, v4
    v6 = load.i32 v5, align 4
    v7 = zext.i64 v6
    v8 = ptr_add inbounds v0, v4
    store.i64 v7, v8, align 8
    return
}

function @main() -> i64 external {
    slot0 = stack_slot 16, align 8
    slot1 = stack_slot 12, align 4
block0:
    v0 = stack_addr slot0
    v1 = stack_addr slot1
    v2 = iconst.i32 -5
    store.i32 v2, v1, align 4
    v3 = iconst.i64 4
    v4 = ptr_add inbounds v1, v3
    v5 = iconst.i32 1065353216
    store.i32 v5, v4, align 4
    v6 = iconst.i64 8
    v7 = ptr_add inbounds v1, v6
    v8 = iconst.i32 1073741824
    store.i32 v8, v7, align 4
    call @pair(v0, v1)
    v9 = load.i64 v0, align 8
    v10 = ptr_add inbounds v0, v6
    v11 = load.i64 v10, align 8
    v12 = iadd.i64 v9, v11
    return v12
}
",
        1_073_741_819,
    );
}

/// A 24-byte structure in and out: `byval` and `sret` on System V, a
/// reference and `sret` on Win64. The argument copy is not a slot of the
/// caller, so System V copies it to an aligned temporary; the result goes
/// straight to the caller's slot.
#[test]
fn memory_arguments_and_results() {
    check(
        "memory",
        "\
function @twice(ptr result(size 24, align 8), ptr aggregate(size 24, align 4)) internal {
block0(v0: ptr, v1: ptr):
    v2 = iconst.i64 24
    copy v0, v1, v2, align 4
    v3 = load.i32 v1, align 4
    v4 = iadd.i32 v3, v3
    store.i32 v4, v0, align 8
    return
}

function @main() -> i32 external {
    slot0 = stack_slot 24, align 8
    slot1 = stack_slot 48, align 4
block0:
    v0 = stack_addr slot1
    v1 = iconst.i64 24
    v2 = ptr_add inbounds v0, v1
    v3 = iconst.i32 21
    store.i32 v3, v2, align 4
    v4 = stack_addr slot0
    call @twice(v4, v2)
    v5 = load.i32 v4, align 8
    return v5
}
",
        42,
    );
}

/// A union of a `float`, an `int` and three `char`s whose last member
/// written is the `char`s: System V and Win64 both pass it as an `i32`
/// built a byte at a time, and the result comes back through a return area
/// that is not a slot, so the caller copies it out of a temporary.
#[test]
fn unions_pack_bytes_and_results_copy_back() {
    check(
        "union",
        "\
function @echo(ptr result(size 4, align 4, [0: f32, 0: i32, 0: i8, 1: i8, 2: i8]), ptr \
         aggregate(size 4, align 4, [0: f32, 0: i32, 0: i8, 1: i8, 2: i8])) internal {
block0(v0: ptr, v1: ptr):
    v2 = iconst.i64 4
    copy v0, v1, v2, align 4
    return
}

function @main() -> i32 external {
    slot0 = stack_slot 8, align 4
    slot1 = stack_slot 4, align 4
block0:
    v0 = stack_addr slot1
    v1 = iconst.i8 7
    store.i8 v1, v0, align 1
    v2 = iconst.i64 1
    v3 = ptr_add inbounds v0, v2
    v4 = iconst.i8 3
    store.i8 v4, v3, align 1
    v5 = iconst.i64 2
    v6 = ptr_add inbounds v0, v5
    store.i8 v4, v6, align 1
    v7 = stack_addr slot0
    v8 = iconst.i64 4
    v9 = ptr_add inbounds v7, v8
    call @echo(v9, v0)
    v10 = load.i8 v9, align 4
    v11 = ptr_add inbounds v9, v2
    v12 = load.i8 v11, align 1
    v13 = iadd.i8 v10, v12
    v14 = zext.i32 v13
    return v14
}
",
        10,
    );
}

/// An indirect call and a call with an empty structure: System V passes
/// nothing for it and the callee gets an empty slot; Win64 passes a
/// reference.
#[test]
fn indirect_calls_and_empty_structures() {
    check(
        "indirect",
        "\
function @add(ptr aggregate(size 8, align 4, [0: i32, 4: i32]), ptr aggregate(size 0, align 1), \
         i32) -> i32 internal {
block0(v0: ptr, v1: ptr, v2: i32):
    v3 = load.i32 v0, align 4
    v4 = iconst.i64 4
    v5 = ptr_add inbounds v0, v4
    v6 = load.i32 v5, align 4
    v7 = iadd.i32 v3, v6
    v8 = iadd.i32 v7, v2
    return v8
}

function @main() -> i32 external {
    slot0 = stack_slot 8, align 4
    slot1 = stack_slot 0, align 1
block0:
    v0 = stack_addr slot0
    v1 = iconst.i32 30
    store.i32 v1, v0, align 4
    v2 = iconst.i64 4
    v3 = ptr_add inbounds v0, v2
    v4 = iconst.i32 10
    store.i32 v4, v3, align 4
    v5 = stack_addr slot1
    v6 = func_addr @add
    v7 = iconst.i32 2
    v8 = call_indirect v6(v0, v5, v7) : (ptr aggregate(size 8, align 4, [0: i32, 4: i32]), ptr \
         aggregate(size 0, align 1), i32) -> i32
    return v8
}
",
        42,
    );
}
