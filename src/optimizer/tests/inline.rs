//! Call expansion, refusal policy, bottom-up ordering and interpreter checks.

use std::fmt::Write as _;

use super::{
    equivalence::{
        assert_equivalent,
        returned,
    },
    *,
};
use crate::{
    backend::interpreter::RuntimeValue,
    ir::Type,
};

const LEAF: &str = "\
function @twice(i32) -> i32 internal {
block0(v0: i32):
    v1 = iadd.i32 nsw v0, v0
    return v1
}

function @main(i32) -> i32 external {
block0(v0: i32):
    v1 = call @twice(v0)
    v2 = iadd.i32 v1, v0
    return v2
}
";

#[test]
fn dynamic_alloca_callee_in_a_loop_retains_its_call_and_bounded_storage() {
    // GNU Stack Allocation: each allocation dies when the callee returns.
    // <https://gcc.gnu.org/onlinedocs/gcc/Stack-Allocation.html>
    let text = "\
function @allocate(i64) -> i32 internal {
block0(v0: i64):
    v1 = dyn_alloca.i64 v0
    v2 = iconst.i32 7
    store.i32 v2, v1, align 4
    v3 = load.i32 v1, align 4
    return v3
}

function @main(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 0
    v2 = iconst.i32 1
    v3 = iconst.i64 256
    jump block1(v1, v1)
block1(v4: i32, v5: i32):
    v6 = icmp.i32 slt v4, v0
    brif v6, block2, block3
block2:
    v7 = call @allocate(v3)
    v8 = iadd.i32 v5, v7
    v9 = iadd.i32 v4, v2
    jump block1(v9, v8)
block3:
    return v5
}
";
    let (output, report) = optimize_text(text, &only(&[Pass::Inline]));
    assert!(output.contains("call @allocate"), "{output}");
    assert_eq!(report.stats(Pass::Inline).changes, 0);
    for program in [text, output.as_str()] {
        for iterations in [1_u32, 10_000] {
            let run = crate::backend::interpreter::run_text_with(
                program,
                "main",
                &[RuntimeValue::int(Type::I32, u128::from(iterations))],
                crate::backend::interpreter::Limits::default(),
            );
            let outcome = run.result.unwrap();
            assert_eq!(
                outcome.termination,
                crate::backend::interpreter::Termination::Returned(Some(RuntimeValue::int(
                    Type::I32,
                    u128::from(iterations * 7)
                )))
            );
            assert!(
                outcome.memory_cells <= 512,
                "{iterations} iterations retained {} byte cells",
                outcome.memory_cells
            );
        }
    }
}

#[test]
fn leaf_splits_block_and_replaces_result_with_continuation_parameter() {
    assert_pass(
        Pass::Inline,
        LEAF,
        "\
function @twice(i32) -> i32 internal {
block0(v0: i32):
    v1 = iadd.i32 nsw v0, v0
    return v1
}

function @main(i32) -> i32 external {
block0(v0: i32):
    jump block2
block1(v1: i32):
    v2 = iadd.i32 v1, v0
    return v2
block2:
    v3 = iadd.i32 nsw v0, v0
    jump block1(v3)
}
",
    );
    assert_equivalent(LEAF, "main", &[&[0], &[1], &[-7], &[12345]]);
}

#[test]
fn default_inline_runs_once_while_cleanup_repeats() {
    let (_, report) = optimize_text(LEAF, &OptimizerOptions::default());
    assert_eq!(report.stats(Pass::Inline).runs, 2);
    assert!(report.stats(Pass::Fold).runs > report.stats(Pass::Inline).runs);
    let (_, explicit) = optimize_text(LEAF, &only(&[Pass::Inline, Pass::Inline]));
    assert_eq!(explicit.stats(Pass::Inline).runs, 4);
}

#[test]
fn nested_forward_calls_are_inlined_bottom_up() {
    let text = "\
function @main(i32) -> i32 external {
block0(v0: i32):
    v1 = call @middle(v0)
    return v1
}

function @middle(i32) -> i32 internal {
block0(v0: i32):
    v1 = call @leaf(v0)
    return v1
}

function @leaf(i32) -> i32 external {
block0(v0: i32):
    v1 = iadd.i32 v0, v0
    return v1
}
";
    let (output, report) = optimize_text(text, &only(&[Pass::Inline]));
    assert!(!output.contains("call @"), "{output}");
    assert_eq!(report.stats(Pass::Inline).changes, 2);
    assert_equivalent(text, "main", &[&[-10], &[0], &[42]]);
}

#[test]
fn multiple_returns_join_and_two_calls_in_one_block_expand() {
    let text = "\
function @choose(i32) -> i32 internal {
block0(v0: i32):
    v1 = iconst.i32 0
    v2 = icmp.i32 slt v0, v1
    brif v2, block1, block2
block1:
    v3 = isub.i32 v1, v0
    return v3
block2:
    return v0
}

function @main(i32) -> i32 external {
block0(v0: i32):
    v1 = call @choose(v0)
    v2 = call @choose(v1)
    return v2
}
";
    let (output, report) = optimize_text(text, &only(&[Pass::Inline]));
    assert_eq!(report.stats(Pass::Inline).changes, 2);
    assert!(!output.contains("call @choose"));
    assert_eq!(output.matches("jump block1(").count(), 2);
    assert_equivalent(text, "main", &[&[-101], &[0], &[77]]);
}

#[test]
fn void_pointer_calls_expose_address_taken_caller_slots_to_promotion() {
    let text = "\
function @write(ptr, i32) internal {
block0(v0: ptr, v1: i32):
    store.i32 v1, v0, align 4
    return
}

function @main(i32) -> i32 external {
    slot0 = stack_slot 4, align 4
block0(v0: i32):
    v1 = stack_addr slot0
    call @write(v1, v0)
    v2 = load.i32 v1, align 4
    return v2
}
";
    let (output, _) = optimize_text(text, &only(&[Pass::Inline]));
    assert!(!output.contains("call @write"));
    assert!(output.contains("block1:\n")); // void continuation has no parameter
    let (cleaned, _) = optimize_text(text, &OptimizerOptions::default());
    assert!(!cleaned.contains("stack_slot"), "{cleaned}");
    assert_equivalent(text, "main", &[&[-12], &[0], &[55]]);
}

#[test]
fn copied_slots_are_independent_per_site_and_promotable() {
    let text = "\
function @local(i32) -> i32 internal {
    slot0 = stack_slot 4, align 4
block0(v0: i32):
    v1 = stack_addr slot0
    store.i32 v0, v1, align 4
    v2 = load.i32 v1, align 4
    return v2
}

function @main(i32) -> i32 external {
    slot0 = stack_slot 8, align 8
block0(v0: i32):
    v1 = call @local(v0)
    v2 = call @local(v1)
    return v2
}
";
    let (output, _) = optimize_text(text, &only(&[Pass::Inline]));
    assert!(output.contains("slot1 = stack_slot 4, align 4"));
    assert!(output.contains("slot2 = stack_slot 4, align 4"));
    let (promoted, _) = optimize_text(text, &only(&[Pass::Inline, Pass::Promote]));
    assert!(!promoted.contains("stack_slot 4"), "{promoted}");
    assert_equivalent(text, "main", &[&[-9], &[0], &[81]]);
}

#[test]
fn self_and_mutual_recursion_are_skipped_but_other_leaf_calls_expand() {
    let text = "\
function @self(i32) -> i32 internal {
block0(v0: i32):
    v1 = call @self(v0)
    return v1
}

function @a(i32) -> i32 internal {
block0(v0: i32):
    v1 = call @b(v0)
    return v1
}

function @b(i32) -> i32 internal {
block0(v0: i32):
    v1 = call @a(v0)
    v2 = call @leaf(v1)
    return v2
}

function @leaf(i32) -> i32 internal {
block0(v0: i32):
    return v0
}

function @main(i32) -> i32 external {
block0(v0: i32):
    v1 = call @a(v0)
    return v1
}
";
    let (output, report) = optimize_text(text, &only(&[Pass::Inline]));
    assert_eq!(report.stats(Pass::Inline).changes, 1);
    for name in ["self", "a", "b"] {
        assert!(output.contains(&format!("call @{name}")));
    }
    assert!(!output.contains("call @leaf"));
}

#[test]
fn declarations_variadic_and_returns_twice_wrappers_are_refused() {
    let text = "\
function @setjmp(ptr) -> i32 external

function @variadic(i32, ...) -> i32 internal {
block0(v0: i32):
    return v0
}

function @wrapper(ptr) -> i32 internal {
block0(v0: ptr):
    v1 = call @setjmp(v0)
    return v1
}

function @main(ptr) -> i32 external {
block0(v0: ptr):
    v1 = call @wrapper(v0)
    v2 = call @variadic(v1)
    return v2
}
";
    let (_, report) = optimize_text(text, &only(&[Pass::Inline]));
    assert_eq!(report.stats(Pass::Inline).changes, 0);
}

#[test]
fn callee_threshold_and_caller_growth_cap_are_enforced() {
    let mut callee = String::from("function @large(i32) -> i32 internal {\nblock0(v0: i32):\n");
    for n in 1..101 {
        _ = writeln!(callee, "    v{n} = iadd.i32 v{}, v0", n - 1);
    }
    callee.push_str(
        "    return v100\n}\n\nfunction @main(i32) -> i32 external {\nblock0(v0: i32):\n    v1 = \
         call @large(v0)\n    return v1\n}\n",
    );
    let (_, report) = optimize_text(&callee, &only(&[Pass::Inline]));
    assert_eq!(report.stats(Pass::Inline).changes, 0);
    let mut many = String::from("function @leaf(i32) -> i32 internal {\nblock0(v0: i32):\n");
    for n in 1..30 {
        _ = writeln!(many, "    v{n} = iadd.i32 v{}, v0", n - 1);
    }
    many.push_str("    return v29\n}\n\nfunction @main(i32) -> i32 external {\nblock0(v0: i32):\n");
    for n in 1..101 {
        _ = writeln!(many, "    v{n} = call @leaf(v{})", n - 1);
    }
    many.push_str("    return v100\n}\n");
    let (output, report) = optimize_text(&many, &only(&[Pass::Inline]));
    assert_eq!(report.stats(Pass::Inline).changes, 8);
    assert_eq!(output.matches("call @leaf").count(), 92);
}

#[test]
fn bisect_counts_each_complete_site_as_one_rewrite() {
    let text = LEAF.replace("    v2 = iadd.i32 v1, v0", "    v2 = call @twice(v1)");
    for limit in 0..=2 {
        let mut options = only(&[Pass::Inline]);
        options.bisect_limit = Some(limit);
        let (output, report) = optimize_text(&text, &options);
        assert_eq!(report.transformations(), limit);
        assert_eq!(
            output.matches("call @twice").count(),
            2 - usize::try_from(limit).unwrap()
        );
        assert_eq!(
            returned(&text, "main", &[RuntimeValue::int(Type::I32, 9)]),
            returned(&output, "main", &[RuntimeValue::int(Type::I32, 9)])
        );
    }
}

#[test]
fn foreign_wide_integer_and_float_constants_keep_their_bits() {
    let text = "\
function @wide() -> i128 internal {
block0:
    v0 = iconst.i128 18446744073709551617
    return v0
}

function @floating() -> f128 internal {
block0:
    v0 = fconst.f128 0x3fff0000000000000000000000000000
    return v0
}

function @main() -> i128 external {
block0:
    v0 = call @wide()
    return v0
}

function @float_main() -> f128 external {
block0:
    v0 = call @floating()
    return v0
}
";
    for options in [only(&[Pass::Inline]), OptimizerOptions::default()] {
        let (output, _) = optimize_text(text, &options);
        assert!(!output.contains("call @"));
        assert_eq!(returned(text, "main", &[]), returned(&output, "main", &[]));
        // The interpreter does not execute wide floats; verify their exact bits
        // and two owned copies (the definition and its inlined call site).
        assert_eq!(
            output
                .matches("fconst.f128 0x3FFF0000000000000000000000000000")
                .count(),
            2
        );
    }
}

#[test]
fn switch_edges_and_copied_indirect_call_arguments_are_remapped() {
    let text = "\
function @leaf(i32) -> i32 internal {
block0(v0: i32):
    v1 = iadd.i32 v0, v0
    return v1
}

function @dispatch(ptr, i32) -> i32 internal {
block0(v0: ptr, v1: i32):
    v2 = call_indirect v0(v1) : (i32) -> i32
    switch v1, block1(v2), [0: block2(v2), 1: block3(v2)]
block1(v3: i32):
    return v3
block2(v4: i32):
    v5 = iconst.i32 7
    v6 = iadd.i32 v4, v5
    return v6
block3(v7: i32):
    v8 = iconst.i32 9
    v9 = iadd.i32 v7, v8
    return v9
}

function @main(i32) -> i32 external {
block0(v0: i32):
    v1 = func_addr @leaf
    v2 = call @dispatch(v1, v0)
    v3 = call @dispatch(v1, v2)
    return v3
}
";
    let (output, report) = optimize_text(text, &only(&[Pass::Inline]));
    assert_eq!(report.stats(Pass::Inline).changes, 2);
    assert_eq!(output.matches("call_indirect").count(), 3);
    assert!(!output.contains("call @dispatch"));
    assert_equivalent(text, "main", &[&[-11], &[0], &[1], &[32]]);
}

#[test]
fn definitions_in_later_layout_blocks_and_callee_loops_are_copied() {
    let text = "\
function @sum(i32) -> i32 internal {
block0(v0: i32):
    jump block2
block1:
    return v3
block2:
    v1 = iconst.i32 0
    jump block3(v1, v1)
block3(v2: i32, v3: i32):
    v4 = icmp.i32 slt v2, v0
    brif v4, block4, block1
block4:
    v5 = iconst.i32 1
    v6 = iadd.i32 v2, v5
    v7 = iadd.i32 v3, v2
    jump block3(v6, v7)
}

function @main(i32) -> i32 external {
block0(v0: i32):
    v1 = call @sum(v0)
    return v1
}
";
    let (output, report) = optimize_text(text, &only(&[Pass::Inline]));
    assert_eq!(report.stats(Pass::Inline).changes, 1);
    assert!(!output.contains("call @sum"));
    assert_equivalent(text, "main", &[&[-1], &[0], &[1], &[10]]);
}

#[test]
fn deep_call_graph_uses_an_explicit_stack() {
    let mut text = String::new();
    for n in 0..2000 {
        _ = write!(
            text,
            "function @f{n}(i32) -> i32 internal {{\nblock0(v0: i32):\n"
        );
        if n == 1999 {
            text.push_str("    return v0\n}\n\n");
        } else {
            _ = write!(text, "    v1 = call @f{}(v0)\n    return v1\n}}\n\n", n + 1);
        }
    }
    let (output, report) = optimize_text(&text, &OptimizerOptions::default());
    assert_eq!(report.stats(Pass::Inline).changes, 1999);
    assert!(!output.contains("call @"));
}
