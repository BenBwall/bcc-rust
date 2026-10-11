//! Calls: indirect calls through function pointers, unknown functions, and
//! variadic arguments.

use super::*;

const ADD: &str = "\
function @add(i32, i32) -> i32 internal {
block0(v0: i32, v1: i32):
    v2 = iadd.i32 v0, v1
    return v2
}
";

#[test]
fn indirect_calls_go_through_function_addresses() {
    let text = format!(
        "global @handlers internal constant size 8, align 8 = bytes \"0000000000000000\" relocs \
         [0: @add]

{ADD}
function @f() -> i32 external {{
block0:
    v0 = func_addr @add
    v1 = iconst.i32 2
    v2 = iconst.i32 3
    v3 = call_indirect v0(v1, v2) : (i32, i32) -> i32
    v4 = global_addr @handlers
    v5 = load.ptr v4, align 8
    v6 = call_indirect v5(v3, v3) : (i32, i32) -> i32
    v7 = icmp.ptr eq v0, v5
    v8 = zext.i32 v7
    v9 = iadd.i32 v6, v8
    return v9
}}
"
    );
    assert_eq!(returned(&text, "f"), int(11));
}

#[test]
fn bad_indirect_calls_are_undefined() {
    let call = |callee: &str, signature: &str| {
        let text = format!(
            "{ADD}
function @f() -> i32 external {{
    slot0 = stack_slot 4, align 4
block0:
    v0 = {callee}
    v1 = iconst.i32 1
    v2 = call_indirect v0(v1, v1) : {signature}
    return v1
}}
"
        );
        undefined(&text, "f")
    };
    let signature = "(i32, i32) -> i32";
    assert_eq!(
        call("func_addr @add", "(i32, i32) -> i64"),
        UbKind::SignatureMismatch
    );
    assert_eq!(
        call("stack_addr slot0", signature),
        UbKind::CallThroughNonFunction
    );
    assert_eq!(
        call("poison.ptr", signature),
        UbKind::CallThroughNonFunction
    );
    assert_eq!(call("null", signature), UbKind::CallThroughNonFunction);
}

#[test]
fn calls_the_host_lacks_trap_with_the_callee() {
    let text = "\
function @mystery(i32) -> i32 external

function @f() -> i32 external {
block0:
    v0 = iconst.i32 1
    v1 = func_addr @mystery
    v2 = call_indirect v1(v0) : (i32) -> i32
    return v2
}
";
    let run = run_text(text, "f");
    assert!(
        matches!(run.result, Err(Trap::UnknownFunction(callee, _)) if callee.as_u32() == 0),
        "{:?}",
        run.result
    );
    assert_eq!(
        run.message.as_deref(),
        Some("call to @mystery, which has no definition (in @f at inst2)")
    );
}

/// `@sum(n, ...)` adds its `n` variadic `i32` arguments, reading them in
/// `@vsum` through a `va_list` it passes on, and `@first(...)` checks
/// `va_copy`.
const VARIADIC: &str = "\
function @vsum(ptr, i32) -> i32 internal {
block0(v0: ptr, v1: i32):
    v2 = iconst.i32 0
    jump block1(v1, v2)
block1(v3: i32, v4: i32):
    v5 = iconst.i32 0
    v6 = icmp.i32 eq v3, v5
    brif v6, block3, block2
block2:
    v7 = va_arg.i32 v0
    v8 = iadd.i32 v4, v7
    v9 = iconst.i32 1
    v10 = isub.i32 v3, v9
    jump block1(v10, v8)
block3:
    return v4
}

function @sum(i32, ...) -> i32 internal {
    slot0 = stack_slot 24, align 8
    slot1 = stack_slot 24, align 8
block0(v0: i32):
    v1 = stack_addr slot0
    va_start v1
    v2 = va_arg.i32 v1
    v3 = stack_addr slot1
    va_copy v3, v1
    va_end v1
    v4 = iconst.i32 1
    v5 = isub.i32 v0, v4
    v6 = call @vsum(v3, v5)
    va_end v3
    v7 = iadd.i32 v2, v6
    return v7
}
";

#[test]
fn varargs_walk_the_extra_arguments() {
    let text = format!(
        "{VARIADIC}
function @f() -> i32 external {{
block0:
    v0 = iconst.i32 3
    v1 = iconst.i32 10
    v2 = iconst.i32 20
    v3 = iconst.i32 30
    v4 = call @sum(v0, v1, v2, v3)
    v5 = iconst.i32 1
    v6 = call @sum(v5, v0)
    v7 = iadd.i32 v4, v6
    return v7
}}
"
    );
    assert_eq!(returned(&text, "f"), int(63));
}

#[test]
fn varargs_misuse_is_undefined() {
    let misuse = |args: &str, body: &str| {
        let text = format!(
            "function @g(i32, ...) -> i32 internal {{
    slot0 = stack_slot 8, align 8
block0(v0: i32):
    v1 = stack_addr slot0
    {body}
    return v0
}}

function @f() -> i32 external {{
block0:
    v0 = iconst.i32 1
    v1 = iconst.i64 2
    v2 = call @g({args})
    return v2
}}
"
        );
        undefined(&text, "f")
    };
    assert_eq!(
        misuse(
            "v0, v0",
            "va_start v1\n    v2 = va_arg.i32 v1\n    v3 = va_arg.i32 v1"
        ),
        UbKind::VaArgPastEnd
    );
    assert_eq!(
        misuse("v0, v1", "va_start v1\n    v2 = va_arg.i32 v1"),
        UbKind::VaArgTypeMismatch
    );
    assert_eq!(
        misuse(
            "v0, v0",
            "va_start v1\n    va_end v1\n    v2 = va_arg.i32 v1"
        ),
        UbKind::InvalidVaList
    );
    assert_eq!(
        misuse("v0", "v2 = va_arg.i32 v1"),
        UbKind::InvalidVaList,
        "the va_list was never started, so its bytes are poison"
    );
    let text = "\
function @start(ptr, ...) internal {
block0(v0: ptr):
    va_start v0
    return
}

function @nonvariadic(ptr) internal {
block0(v0: ptr):
    va_start v0
    return
}

function @stale() -> i32 external {
    slot0 = stack_slot 8, align 8
block0:
    v0 = stack_addr slot0
    v1 = iconst.i32 5
    call @start(v0, v1)
    v2 = va_arg.i32 v0
    return v2
}

function @outside() -> i32 external {
    slot0 = stack_slot 8, align 8
block0:
    v0 = stack_addr slot0
    call @nonvariadic(v0)
    v1 = iconst.i32 0
    return v1
}
";
    assert_eq!(undefined(text, "stale"), UbKind::InvalidVaList);
    assert_eq!(undefined(text, "outside"), UbKind::VaStartOutsideVariadic);
}
