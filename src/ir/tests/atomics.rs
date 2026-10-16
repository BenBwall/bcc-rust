//! Text and verifier boundaries for atomic memory instructions.
use super::*;

#[test]
fn atomic_text_round_trips_and_preserves_memory_metadata() {
    let text = "function @atomic(ptr, i32, i32) -> i32 external {\nblock0(v0: ptr, v1: i32, v2: \
                i32):\n    v3 = atomic.i32 volatile load acquire relaxed align 4 (v0)\n    \
                atomic.i32 store release relaxed align 4 (v0, v1)\n    v4 = atomic.i32 add \
                acq_rel relaxed align 4 (v0, v1)\n    v5 = atomic.i32 compare_exchange seq_cst \
                acquire align 4 (v0, v1, v2)\n    atomic.i8 thread_fence seq_cst relaxed align 1 \
                ()\n    atomic.i8 signal_fence acquire relaxed align 1 ()\n    return v5\n}\n";
    assert_round_trip(text);
    let no_errors: [String; 0] = [];
    assert_eq!(verify_text(text), no_errors);
}

#[test]
fn atomic_verifier_rejects_illegal_memory_orders_and_arity() {
    for operation in [
        "atomic.i32 load release relaxed align 4 (v0)",
        "atomic.i32 load acq_rel relaxed align 4 (v0)",
        "atomic.i32 exchange relaxed acquire align 4 (v0, v1)",
        "atomic.i32 compare_exchange relaxed acquire align 4 (v0, v1, v1)",
        "atomic.i32 compare_exchange acquire seq_cst align 4 (v0, v1, v1)",
        "atomic.i32 compare_exchange seq_cst release align 4 (v0, v1, v1)",
        "atomic.i32 load acquire relaxed align 4 (v0, v1)",
        "atomic.i32 compare_exchange seq_cst acquire align 4 (v0, v1)",
    ] {
        let text = format!(
            "function @f(ptr, i32) -> i32 external {{\nblock0(v0: ptr, v1: i32):\n    v2 = \
             {operation}\n    return v2\n}}\n"
        );
        assert!(
            verify_text(&text)
                .iter()
                .any(|error| error.contains("atomic")),
            "{operation}"
        );
    }
    for order in ["acquire", "acq_rel"] {
        let text = format!(
            "function @f(ptr, i32) external {{\nblock0(v0: ptr, v1: i32):\n    atomic.i32 store \
             {order} relaxed align 4 (v0, v1)\n    return\n}}\n"
        );
        assert!(
            verify_text(&text)
                .iter()
                .any(|error| error.contains("atomic"))
        );
    }
}

#[test]
fn atomic_verifier_rejects_operand_and_operation_type_mismatches() {
    for text in [
        "function @f(i32) -> i32 external {\nblock0(v0: i32):\n    v1 = atomic.i32 load relaxed \
         relaxed align 4 (v0)\n    return v1\n}\n",
        "function @f(ptr, i16) -> i32 external {\nblock0(v0: ptr, v1: i16):\n    v2 = atomic.i32 \
         add seq_cst relaxed align 4 (v0, v1)\n    return v2\n}\n",
        "function @f(ptr) -> i1 external {\nblock0(v0: ptr):\n    v1 = atomic.i1 load seq_cst \
         relaxed align 1 (v0)\n    return v1\n}\n",
        "function @f(ptr, f32) -> f32 external {\nblock0(v0: ptr, v1: f32):\n    v2 = atomic.f32 \
         add seq_cst relaxed align 4 (v0, v1)\n    return v2\n}\n",
    ] {
        assert!(!verify_text(text).is_empty(), "{text}");
    }
}

#[test]
fn atomic_integer_widths_and_float_rmw_types_verify() {
    for (ty, align) in [("i8", 1), ("i16", 2), ("i32", 4), ("i64", 8), ("i128", 16)] {
        for operation in [
            "load",
            "exchange",
            "add",
            "sub",
            "and",
            "or",
            "xor",
            "nand",
            "min",
            "max",
            "umin",
            "umax",
            "compare_exchange",
        ] {
            let operands = if operation == "load" {
                "v0"
            } else if operation == "compare_exchange" {
                "v0, v1, v1"
            } else {
                "v0, v1"
            };
            let text = format!(
                "function @f(ptr, {ty}) -> {ty} external {{\nblock0(v0: ptr, v1: {ty}):\n    v2 = \
                 atomic.{ty} {operation} seq_cst relaxed align {align} ({operands})\n    return \
                 v2\n}}\n"
            );
            assert!(verify_text(&text).is_empty(), "{text}");
            assert_round_trip(&text);
        }
    }
    for (ty, align) in [("f32", 4), ("f64", 8)] {
        for operation in ["fadd", "fsub", "fmin", "fmax"] {
            let text = format!(
                "function @f(ptr, {ty}) -> {ty} external {{\nblock0(v0: ptr, v1: {ty}):\n    v2 = \
                 atomic.{ty} {operation} acq_rel relaxed align {align} (v0, v1)\n    return \
                 v2\n}}\n"
            );
            assert!(verify_text(&text).is_empty(), "{text}");
            assert_round_trip(&text);
        }
    }
}

#[test]
fn atomic_buffer_operations_round_trip() {
    let text = "function @f(ptr, ptr, ptr, i64) -> i8 external {\nblock0(v0: ptr, v1: ptr, v2: \
                ptr, v3: i64):\n    atomic.i8 load_bytes acquire relaxed align 1 (v0, v1, v3)\n    \
                atomic.i8 store_bytes release relaxed align 1 (v0, v1, v3)\n    atomic.i8 \
                exchange_bytes acq_rel relaxed align 1 (v0, v1, v2, v3)\n    v4 = atomic.i8 \
                compare_exchange_bytes seq_cst acquire align 1 (v0, v1, v2, v3)\n    return \
                v4\n}\n";
    let no_errors: [String; 0] = [];
    assert_eq!(verify_text(text), no_errors);
    assert_round_trip(text);
}
