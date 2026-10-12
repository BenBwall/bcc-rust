//! `va_arg` expansion: lowered modules keep no `va_arg`, run in the
//! interpreter with the target's `va_list` to the same result as before,
//! and reject the types the expansion does not read.

use std::fmt::Write as _;

use super::{
    rewrite::main_result,
    *,
};
use crate::test_support::assert_snapshot;

#[test]
fn repeated_va_start_keeps_backing_memory_bounded_and_lists_independent() {
    let program = "function @restart(i32, ...) -> i32 internal {
    slot0 = stack_slot 24, align 8
    slot1 = stack_slot 24, align 8
    slot2 = stack_slot 24, align 8
block0(v0: i32):
    v1 = stack_addr slot0
    v2 = stack_addr slot1
    v3 = stack_addr slot2
    v4 = iconst.i32 0
    v5 = iconst.i32 1
    jump block1(v4, v4)
block1(v6: i32, v7: i32):
    va_start v1
    va_start v2
    v8 = va_arg.i32 v1
    va_copy v3, v1
    v9 = va_arg.i32 v1
    v10 = va_arg.i32 v2
    v11 = va_arg.i32 v3
    va_end v1
    va_end v2
    va_end v3
    v12 = iadd.i32 v8, v9
    v13 = iadd.i32 v12, v10
    v14 = iadd.i32 v13, v11
    v15 = iadd.i32 v7, v14
    v16 = iadd.i32 v6, v5
    v17 = icmp.i32 slt v16, v0
    brif v17, block1(v16, v15), block2
block2:
    return v15
}
function @main() -> i32 external {
block0:
    v0 = iconst.i32 100000
    v1 = iconst.i32 3
    v2 = iconst.i32 7
    v3 = call @restart(v0, v1, v2, v1, v2, v1, v2, v1, v2)
    return v3
}
";
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        let text = format!("target triple = \"{}\"\n{program}", target.triple());
        let text = lower_text(&text, target);
        let ir = Bump::new();
        let module = parse_module(&ir, &text).unwrap();
        let arena = Bump::new();
        let mut host = crate::backend::interpreter::DefaultHost::for_target(&arena, target);
        let outcome = crate::backend::interpreter::run(&module, "main", &[], &mut host).unwrap();
        assert_eq!(
            outcome.termination,
            crate::backend::interpreter::Termination::Returned(Some(
                crate::backend::interpreter::RuntimeValue::Int(2_000_000)
            )),
            "{target:?}"
        );
        eprintln!(
            "{target:?}: {} retained cells after 100,000 restarts",
            outcome.memory_cells
        );
        assert!(
            outcome.memory_cells < 1024,
            "{target:?}: {} retained cells",
            outcome.memory_cells
        );
    }
}

/// `@sum` adds `n` variadic `i64`s; `@mixed` reads an `i32`, a `double`, a
/// pointer and an `i64` repeatedly, through a copy of its `va_list` for the
/// second half; `@main` passes enough of each that both register classes
/// overflow onto the stack.
const VARIADIC: &str = "\
function @sum(i32, ...) -> i64 internal {
    slot0 = stack_slot 24, align 8
block0(v0: i32):
    v1 = stack_addr slot0
    va_start v1
    v2 = iconst.i32 0
    v3 = iconst.i64 0
    jump block1(v2, v3)
block1(v4: i32, v5: i64):
    v6 = icmp.i32 slt v4, v0
    brif v6, block2, block3
block2:
    v7 = va_arg.i64 v1
    v8 = iadd.i64 v5, v7
    v9 = iconst.i32 1
    v10 = iadd.i32 v4, v9
    jump block1(v10, v8)
block3:
    va_end v1
    return v5
}

function @mixed(i32, ...) -> i64 internal {
    slot0 = stack_slot 24, align 8
    slot1 = stack_slot 24, align 8
block0(v0: i32):
    v1 = stack_addr slot0
    v2 = stack_addr slot1
    va_start v1
    v3 = va_arg.i32 v1
    v4 = va_arg.f64 v1
    v5 = va_arg.ptr v1
    v6 = va_arg.i64 v1
    va_copy v2, v1
    va_end v1
    v7 = va_arg.i32 v2
    v8 = va_arg.f64 v2
    v9 = va_arg.ptr v2
    v10 = va_arg.i64 v2
    va_end v2
    v11 = sext.i64 v3
    v12 = fptosi.i64 v4
    v13 = load.i64 v5, align 8
    v14 = sext.i64 v7
    v15 = fptosi.i64 v8
    v16 = load.i64 v9, align 8
    v17 = iadd.i64 v11, v12
    v18 = iadd.i64 v17, v13
    v19 = iadd.i64 v18, v6
    v20 = iadd.i64 v19, v14
    v21 = iadd.i64 v20, v15
    v22 = iadd.i64 v21, v16
    v23 = iadd.i64 v22, v10
    return v23
}

function @main() -> i64 external {
    slot0 = stack_slot 8, align 8
block0:
    v0 = iconst.i64 1
    v1 = iconst.i64 2
    v2 = iconst.i64 3
    v3 = iconst.i64 4
    v4 = iconst.i64 5
    v5 = iconst.i64 6
    v6 = iconst.i64 7
    v7 = iconst.i64 8
    v8 = iconst.i32 8
    v9 = call @sum(v8, v0, v1, v2, v3, v4, v5, v6, v7)
    v10 = stack_addr slot0
    v11 = iconst.i64 1000
    store.i64 v11, v10, align 8
    v12 = iconst.i32 2
    v13 = iconst.i32 10
    v14 = fconst.f64 0x4059000000000000
    v15 = fconst.f64 0x408f400000000000
    v16 = call @mixed(v12, v13, v14, v10, v4, v13, v15, v10, v6)
    v17 = iadd.i64 v9, v16
    return v17
}
";

/// [`VARIADIC`] for `target`.
fn variadic_for(target: Target) -> String {
    format!("target triple = \"{}\"\n\n{VARIADIC}", target.triple())
}

#[test]
fn va_arg_walks_the_targets_va_list() {
    // 36 from @sum; 10 + 100 + 1000 + 5 + 10 + 1000 + 1000 + 7 from @mixed.
    let expected = 36 + 3132;
    let mut snapshot = String::new();
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        let text = variadic_for(target);
        assert_eq!(
            main_result(&text),
            expected,
            "{target:?} before ABI lowering"
        );
        let lowered = lower_text(&text, target);
        assert!(!lowered.contains("va_arg"), "{lowered}");
        assert_eq!(main_result(&lowered), expected, "{target:?}:\n{lowered}");
        if target != Target::WindowsMsvc {
            let sum = lowered
                .split("\n\n")
                .find(|section| section.starts_with("function @sum"))
                .unwrap();
            writeln!(snapshot, "; {}\n{sum}", target.triple()).unwrap();
        }
    }
    assert_snapshot("tests/fixtures/abi/va_arg.ir", &snapshot);
}

/// More variadic `double`s than System V's eight vector registers, after
/// fixed arguments that already took some of each class.
#[test]
fn fixed_arguments_use_registers_first() {
    let text = "\
function @last(i64, f64, i64, f64, i32, ...) -> i64 internal {
    slot0 = stack_slot 24, align 8
block0(v0: i64, v1: f64, v2: i64, v3: f64, v4: i32):
    v5 = stack_addr slot0
    va_start v5
    v6 = iconst.i32 0
    v7 = iconst.i64 0
    jump block1(v6, v7)
block1(v8: i32, v9: i64):
    v10 = icmp.i32 slt v8, v4
    brif v10, block2, block3
block2:
    v11 = va_arg.f64 v5
    v12 = fptosi.i64 v11
    v13 = va_arg.i64 v5
    v14 = imul.i64 v12, v13
    v15 = iadd.i64 v9, v14
    v16 = iconst.i32 1
    v17 = iadd.i32 v8, v16
    jump block1(v17, v15)
block3:
    v18 = iadd.i64 v9, v0
    v19 = iadd.i64 v18, v2
    return v19
}

function @main() -> i64 external {
block0:
    v0 = iconst.i64 1
    v1 = fconst.f64 0x4000000000000000
    v2 = iconst.i32 9
    v3 = iconst.i64 3
    v4 = call @last(v0, v1, v0, v1, v2, v1, v3, v1, v3, v1, v3, v1, v3, v1, v3, v1, v3, v1, v3, \
                v1, v3, v1, v3)
    return v4
}
";
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        let text = format!("target triple = \"{}\"\n\n{text}", target.triple());
        assert_eq!(main_result(&text), 56);
        assert_eq!(main_result(&lower_text(&text, target)), 56, "{target:?}");
    }
}

#[test]
fn unsupported_va_arg_types_are_reported_before_any_change() {
    let text = |ty: &str| {
        format!(
            "\
function @f(i32, ...) -> {ty} external {{
    slot0 = stack_slot 24, align 8
block0(v0: i32):
    v1 = stack_addr slot0
    va_start v1
    v2 = va_arg.{ty} v1
    va_end v1
    return v2
}}
"
        )
    };
    for (target, ty) in [
        (Target::LinuxGnu, "i1"),
        (Target::WindowsGnu, "i128"),
        (Target::WindowsMsvc, "f80"),
    ] {
        let text = text(ty);
        let arena = Bump::new();
        let mut module = parse_module(&arena, &text).unwrap();
        let before = module.to_string();
        let scratch = Bump::new();
        let error = lower_abi(&mut module, target, &scratch).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("@f: va_arg of type {ty} is not yet supported by ABI lowering for this target")
        );
        assert_eq!(module.to_string(), before);
    }
    for ty in ["i128", "f80", "f128", "f32"] {
        drop(lower_text(&text(ty), Target::LinuxGnu));
    }
}

/// Distinct nonzero halves expose either a split-register mistake or an
/// overflow cursor rounded to eight bytes instead of sixteen.
pub(super) const WIDE_VARIADIC: &str = r"
function @b_wide(i32, ...) -> i32 external {
    slot0 = stack_slot 24, align 8
block0(v0: i32):
    v1 = stack_addr slot0
    va_start v1
    v2 = iconst.i32 0
    jump block1(v2)
block1(v3: i32):
    v4 = icmp.i32 slt v3, v0
    brif v4, block2, block3
block2:
    v5 = va_arg.i64 v1
    v6 = iconst.i32 1
    v7 = iadd.i32 v3, v6
    jump block1(v7)
block3:
    v8 = va_arg.i128 v1
    v9 = iconst.i128 22774453838368691933757882222884355841
    v10 = icmp.i128 ne v8, v9
    v11 = va_arg.i64 v1
    v12 = iconst.i64 999
    v13 = icmp.i64 ne v11, v12
    v14 = zext.i32 v10
    v15 = zext.i32 v13
    v16 = or.i32 v14, v15
    va_end v1
    return v16
}
";

#[test]
fn review_wide_varargs_execute_with_register_exhaustion_and_stack_alignment() {
    for count in [0, 3, 4, 5, 6] {
        let mut main = format!(
            "target triple = \"x86_64-unknown-linux-gnu\"\n{WIDE_VARIADIC}\nfunction @main() -> \
             i32 external {{\nblock0:\n v0 = iconst.i32 {count}\n"
        );
        let mut args = vec!["v0".to_string()];
        for i in 0..count {
            writeln!(main, " v{} = iconst.i64 {}", i + 1, i + 1).unwrap();
            args.push(format!("v{}", i + 1));
        }
        writeln!(
            main,
            " v{} = iconst.i128 22774453838368691933757882222884355841",
            count + 1
        )
        .unwrap();
        writeln!(main, " v{} = iconst.i64 999", count + 2).unwrap();
        args.push(format!("v{}", count + 1));
        args.push(format!("v{}", count + 2));
        writeln!(
            main,
            " v{} = call @b_wide({})\n return v{}\n}}",
            count + 3,
            args.join(", "),
            count + 3
        )
        .unwrap();
        assert_eq!(main_result(&main), 0, "{count} before ABI lowering");
        let lowered = lower_text(&main, Target::LinuxGnu);
        assert_eq!(main_result(&lowered), 0, "{count} after ABI lowering");
    }
}
