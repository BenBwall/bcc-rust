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

#[test]
fn noundef_return_area_pointer_survives_memory_abi_lowering() {
    let text = "function @memory(ptr result(size 24, align 8) noundef) external\n";
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        assert_eq!(
            lowered_signatures(text, target),
            "@memory(ptr sret(size 24, align 8) noundef)"
        );
    }
}

#[test]
fn noundef_contracts_follow_abi_indices_and_explicit_register_returns() {
    let text = "function @mixed(ptr result(size 16, align 8, [0: i64, 8: i64]), ptr \
                aggregate(size 16, align 8, [0: i64, 8: i64]) noundef, i8 signed noundef) \
                external\nfunction @wide(i128 noundef) -> i128 noundef external\n";
    let sysv = lowered_signatures(text, Target::LinuxGnu);
    assert!(
        sysv.contains("@mixed(ptr registers(i64, i64, align 8), i64, i64, i8 signext noundef)"),
        "{sysv}"
    );
    assert!(
        sysv.contains("@wide(i128 noundef) -> i128 noundef"),
        "{sysv}"
    );
    let win = lowered_signatures(text, Target::WindowsGnu);
    assert!(
        win.contains("@mixed(ptr sret(size 16, align 8), ptr noundef, i8 noundef)"),
        "{win}"
    );
    assert!(
        win.contains("@wide(ptr registers(i128, align 16), ptr noundef) -> noundef"),
        "{win}"
    );
    let plain = lower_text(
        "function @plain(ptr aggregate(size 24, align 8)) external\n",
        Target::LinuxGnu,
    );
    assert!(!plain.contains("noundef"), "{plain}");
}

#[test]
fn oversized_aggregate_is_refused_without_mutation_sysv() {
    check_oversized_aggregate(Target::LinuxGnu);
}

#[test]
fn oversized_aggregate_is_refused_without_mutation_win64() {
    check_oversized_aggregate(Target::WindowsGnu);
}

fn check_oversized_aggregate(target: Target) {
    let text = "\
function @small(ptr aggregate(size 8, align 8, [0: i64])) external
function @callee(ptr aggregate(size 4294967296, align 8)) external
function @caller(ptr) external {
block0(v0: ptr):
    call @callee(v0)
    return
}
";
    assert_refused_without_mutation(
        text,
        target,
        "callee",
        AbiErrorKind::AggregateTooLarge(4_294_967_296),
    );
}

#[test]
fn oversized_aggregate_storage_is_refused_in_every_signature() {
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        for size in [4_294_967_296, u64::MAX] {
            for kind in ["aggregate", "result"] {
                let param = format!("ptr {kind}(size {size}, align 8)");
                for variadic in ["", ", ..."] {
                    for argument in ["v0", "v1"] {
                        let text = format!(
                            "\
function @small(ptr aggregate(size 8, align 8, [0: i64])) external
function @callee({param}{variadic}) external {{
block0(v0: ptr):
    return
}}
function @caller(ptr) external {{
    slot0 = stack_slot 8, align 8
block0(v0: ptr):
    v1 = stack_addr slot0
    call @callee({argument})
    return
}}
"
                        );
                        assert_refused_without_mutation(
                            &text,
                            target,
                            "callee",
                            AbiErrorKind::AggregateTooLarge(size),
                        );
                    }
                    // This signature exists only at an indirect call, so
                    // planning must find it even without a declaration.
                    let text = format!(
                        "\
function @small(ptr aggregate(size 8, align 8, [0: i64])) external
function @caller(ptr, ptr) external {{
block0(v0: ptr, v1: ptr):
    call_indirect v0(v1) : ({param}{variadic})
    return
}}
"
                    );
                    assert_refused_without_mutation(
                        &text,
                        target,
                        "caller",
                        AbiErrorKind::AggregateTooLarge(size),
                    );
                }
            }
        }
    }
}

#[test]
fn maximum_stack_sized_aggregate_still_lowers() {
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for kind in ["aggregate", "result"] {
            for argument in ["v0", "v1"] {
                let text = format!(
                    "\
function @callee(ptr {kind}(size 4294967295, align 1), ...) external
function @caller(ptr) external {{
    slot0 = stack_slot 8, align 1
block0(v0: ptr):
    v1 = stack_addr slot0
    call @callee({argument})
    return
}}
"
                );
                let lowered = lower_text(&text, target);
                if target == Target::LinuxGnu && kind == "aggregate" {
                    assert!(
                        lowered.contains("stack_slot 4294967295, align 8"),
                        "{lowered}"
                    );
                }
            }
        }
    }
}

#[test]
fn aggregate_fetch_refusals_name_the_owning_function() {
    let text = "function @fetch(ptr) external {\nblock0(v0: ptr):\nv1 = va_arg_aggregate v0 : \
                (ptr aggregate(size 4294967296, align 1, []))\nreturn\n}\n";
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        assert_refused_without_mutation(
            text,
            target,
            "fetch",
            AbiErrorKind::AggregateTooLarge(4_294_967_296),
        );
    }
}

#[test]
fn classification_refusal_does_not_intern_earlier_signatures() {
    let text = "\
function @small(ptr aggregate(size 8, align 8, [0: i64])) external
function @callee(ptr aggregate(size 16, align 8, [8: i64])) external
";
    assert_refused_without_mutation(
        text,
        Target::LinuxGnu,
        "callee",
        AbiErrorKind::EmptyLowEightbyte,
    );
}

#[track_caller]
fn assert_refused_without_mutation(text: &str, target: Target, function: &str, kind: AbiErrorKind) {
    let arena = Bump::new();
    let mut module = parse_module(&arena, text).unwrap();
    assert_verified(&module, Profile::PreAbi, text);
    let before = module.to_string();
    let signatures = module.signature_count();
    let error = lower_abi(&mut module, target, &Bump::new()).unwrap_err();
    assert_eq!(error, AbiError { function, kind });
    if let AbiErrorKind::AggregateTooLarge(size) = kind {
        assert_eq!(
            error.to_string(),
            format!(
                "@{function}: aggregate size {size} exceeds ABI lowering's stack-slot limit of \
                 4294967295 bytes"
            ),
        );
    }
    assert_eq!(module.profile(), Profile::PreAbi);
    assert_eq!(module.signature_count(), signatures);
    assert_eq!(module.to_string(), before);
    assert_verified(&module, Profile::PreAbi, text);
}

#[test]
fn function_rewrite_and_verification_scratch_is_reclaimed() {
    let usage = |count| {
        let mut text = String::new();
        for i in 0..count {
            writeln!(
                text,
                "function @f{i}(i32) -> i32 external {{\nblock0(v0: i32):\n v1 = iconst.i32 1\n \
                 v2 = iadd.i32 v0, v1\n return v2\n}}"
            )
            .unwrap();
        }
        let arena = Bump::new();
        let scratch = Bump::new();
        let mut function_scratch = Bump::new();
        let mut module = parse_module(&arena, &text).unwrap();
        lower_abi_with_function_scratch(
            &mut module,
            Target::LinuxGnu,
            &scratch,
            &mut function_scratch,
        )
        .unwrap();
        assert_verified(&module, Profile::PostAbi, &text);
        assert_eq!(function_scratch.used(), 0);
        (scratch.high_water(), function_scratch.high_water())
    };
    let one = usage(1);
    let many = usage(1000);
    // Only module plans and the dense function/signature lists may grow.
    // Rewriting and debug verification must reuse their function arena.
    assert!(many.0 < one.0 + 64 * 1000, "{one:?} -> {many:?} bytes");
    assert!(one.1 > 0, "function scratch is exercised");
    assert_eq!(one.1, many.1, "per-function high water is bounded");
}

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
        // A blank line separates the targets, without one at the end.
        if !snapshot.is_empty() {
            snapshot.push('\n');
        }
        write!(snapshot, "; {suffix}\n{lowered}").unwrap();
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
