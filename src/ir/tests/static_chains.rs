//! GNU nested functions in the IR: a signature's static chain, written
//! `chain` on its last fixed parameter, and the `trampoline` and
//! `adjust_trampoline` instructions that make a nested function callable
//! through a plain pointer.

use super::*;

const NESTED: &str = "\
function @add.0(i32, ptr chain) -> i32 internal {
block0(v0: i32, v1: ptr):
    v2 = load.ptr v1, align 8
    v3 = load.i32 v2, align 4
    v4 = iadd.i32 v0, v3
    return v4
}

function @apply(ptr, i32) -> i32 external

function @main() -> i32 external {
    slot0 = stack_slot 8, align 8
    slot1 = stack_slot 32, align 16
block0:
    v0 = stack_addr slot0
    v1 = iconst.i32 1
    v2 = call @add.0(v1, v0)
    v3 = stack_addr slot1
    v4 = func_addr @add.0
    trampoline v3, v4, v0, align 16
    v5 = adjust_trampoline v3
    v6 = call @apply(v5, v1)
    return v6
}
";

#[test]
fn static_chains_and_trampolines_round_trip_and_verify() {
    assert_round_trip(NESTED);
    let arena = Bump::new();
    let module = parse(&arena, NESTED);
    assert_eq!(verify(&module, Profile::PreAbi), Vec::<String>::new());
}

/// Only the last fixed parameter can be the chain, and it is a `ptr`.
#[test]
fn a_static_chain_is_the_last_fixed_pointer() {
    let arena = Bump::new();
    let error = parse_module(&arena, "function @f(ptr chain, i32) external\n").unwrap_err();
    assert!(
        error
            .to_string()
            .contains("`chain` on the last fixed parameter"),
        "{error}"
    );
    let module = parse(&arena, "function @f(i32, i64 chain) external\n");
    assert_eq!(
        verify(&module, Profile::PreAbi),
        ["@f: a static chain must be a plain `ptr` last fixed parameter"]
    );
}
