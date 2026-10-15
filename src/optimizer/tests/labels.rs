//! Labels and indirect branches in every pass: labeled blocks keep their
//! identity, `indirect_br` edges are never redirected, and passes that need
//! to redirect one leave the code alone.

use super::*;
use crate::backend::interpreter::RuntimeValue;

/// A labeled forwarding block and an `indirect_br` edge into another
/// forwarder: neither is threaded, while the plain forwarder still is.
#[test]
fn simplify_cfg_keeps_labeled_forwarders_and_indirect_edges() {
    let input = "\
function @f(ptr, i1) -> i32 external {
    label0 = block_label block2
    label1 = block_label block4
block0(v0: ptr, v1: i1):
    brif v1, block1, block2
block1:
    indirect_br v0, [block2, block4]
block2:
    jump block3
block3:
    jump block4
block4:
    v2 = iconst.i32 1
    return v2
}
";
    let expected = "\
function @f(ptr, i1) -> i32 external {
    label0 = block_label block2
    label1 = block_label block3
block0(v0: ptr, v1: i1):
    brif v1, block1, block2
block1:
    indirect_br v0, [block2, block3]
block2:
    jump block3
block3:
    v2 = iconst.i32 1
    return v2
}
";
    assert_pass(Pass::SimplifyCfg, input, expected);
}

/// A labeled block with one predecessor stays a block of its own, and one
/// that nothing reaches becomes an `unreachable` stub, which a second run
/// leaves alone.
#[test]
fn simplify_cfg_keeps_labeled_blocks() {
    let input = "\
function @f() -> ptr external {
    label0 = block_label block1
    label1 = block_label block2
block0:
    jump block1
block1:
    v0 = block_addr label1
    return v0
block2:
    v1 = iconst.i32 5
    jump block1
}
";
    let expected = "\
function @f() -> ptr external {
    label0 = block_label block1
    label1 = block_label block2
block0:
    jump block1
block1:
    v0 = block_addr label1
    return v0
block2:
    unreachable
}
";
    assert_pass(Pass::SimplifyCfg, input, expected);
    assert_pass_leaves(Pass::SimplifyCfg, expected);
    assert_pipeline(input, expected);
}

/// A labeled arm is not folded into a select.
#[test]
fn select_formation_keeps_labeled_arms() {
    let input = "\
function @f(i1, i32, i32) -> i32 external {
    label0 = block_label block1
block0(v0: i1, v1: i32, v2: i32):
    brif v0, block1, block2(v2)
block1:
    v3 = iadd.i32 v1, v2
    jump block2(v3)
block2(v4: i32):
    return v4
}
";
    assert_pass_leaves(Pass::SimplifyCfg, input);
}

/// An unused parameter of an `indirect_br` target loses its argument on
/// every edge, the indirect one included.
#[test]
fn dce_removes_parameters_through_indirect_edges() {
    let input = "\
function @f(ptr) -> i32 external {
    label0 = block_label block1
block0(v0: ptr):
    v1 = iconst.i32 1
    indirect_br v0, [block1(v1)]
block1(v2: i32):
    v3 = iconst.i32 0
    return v3
}
";
    let expected = "\
function @f(ptr) -> i32 external {
    label0 = block_label block1
block0(v0: ptr):
    indirect_br v0, [block1]
block1:
    v1 = iconst.i32 0
    return v1
}
";
    assert_pass(Pass::Dce, input, expected);
}

/// Every edge of an `indirect_br` is executable: a parameter that receives
/// one constant on all of them is that constant, and one that receives two
/// is not.
#[test]
fn sccp_follows_every_indirect_edge() {
    let input = "\
function @f(ptr) -> i32 external {
    label0 = block_label block1
    label1 = block_label block2
block0(v0: ptr):
    v1 = iconst.i32 4
    v2 = iconst.i32 6
    indirect_br v0, [block1(v1, v1), block2(v1, v2)]
block1(v3: i32, v4: i32):
    v5 = iadd.i32 v3, v4
    return v5
block2(v6: i32, v7: i32):
    v8 = iadd.i32 v6, v7
    return v8
}
";
    let expected = "\
function @f(ptr) -> i32 external {
    label0 = block_label block1
    label1 = block_label block2
block0(v0: ptr):
    indirect_br v0, [block1, block2]
block1:
    v1 = iconst.i32 8
    return v1
block2:
    v2 = iconst.i32 10
    return v2
}
";
    let (actual, _) = optimize_text(input, &only(&[Pass::Sccp, Pass::Dce]));
    pretty_assertions::assert_eq!(actual, expected);
}

/// Promotion gives an `indirect_br` target a parameter and passes the slot's
/// value on the indirect edge.
#[test]
fn promote_passes_values_on_indirect_edges() {
    let input = "\
function @f(ptr, i32) -> i32 external {
    slot0 = stack_slot 4, align 4
    label0 = block_label block1
    label1 = block_label block2
block0(v0: ptr, v1: i32):
    v2 = stack_addr slot0
    store.i32 v1, v2, align 4
    indirect_br v0, [block1, block2]
block1:
    v3 = iconst.i32 1
    store.i32 v3, v2, align 4
    jump block2
block2:
    v4 = load.i32 v2, align 4
    return v4
}
";
    let expected = "\
function @f(ptr, i32) -> i32 external {
    label0 = block_label block1
    label1 = block_label block2
block0(v0: ptr, v1: i32):
    indirect_br v0, [block1, block2(v1)]
block1:
    v2 = iconst.i32 1
    jump block2(v2)
block2(v3: i32):
    return v3
}
";
    assert_pass(Pass::Promote, input, expected);
}

/// A loop entered by an `indirect_br` has no preheader to hoist into, so its
/// invariant stays; LICM still hoists out of a loop entered by a jump.
#[test]
fn licm_skips_loops_entered_indirectly() {
    let input = "\
function @f(ptr, i32, i32) -> i32 external {
    label0 = block_label block1
block0(v0: ptr, v1: i32, v2: i32):
    v3 = iconst.i32 0
    indirect_br v0, [block1(v3)]
block1(v4: i32):
    v5 = imul.i32 v1, v2
    v6 = iadd.i32 v4, v5
    v7 = icmp.i32 slt v6, v1
    brif v7, block1(v6), block2
block2:
    return v6
}
";
    assert_pass_leaves(Pass::Licm, input);
    let jumped = input.replace("indirect_br v0, [block1(v3)]", "jump block1(v3)");
    let (hoisted, _) = optimize_text(&jumped, &only(&[Pass::Licm]));
    assert!(
        hoisted.contains("v4 = imul.i32 v1, v2\n    jump block1(v3)"),
        "{hoisted}"
    );
}

/// A callee whose block address is taken is not inlined; a caller with
/// labels still inlines other callees and keeps its labels.
#[test]
fn inline_refuses_labeled_callees() {
    let input = "\
function @labeled(ptr) -> i32 internal {
    label0 = block_label block1
block0(v0: ptr):
    indirect_br v0, [block1]
block1:
    v1 = iconst.i32 1
    return v1
}

function @leaf(i32) -> i32 internal {
block0(v0: i32):
    v1 = iconst.i32 2
    v2 = iadd.i32 v0, v1
    return v2
}

function @caller(ptr) -> i32 external {
    label0 = block_label block1
block0(v0: ptr):
    v1 = call @labeled(v0)
    indirect_br v0, [block1]
block1:
    v2 = call @leaf(v1)
    return v2
}
";
    let (actual, _) = optimize_text(input, &only(&[Pass::Inline]));
    assert!(actual.contains("v1 = call @labeled(v0)"), "{actual}");
    assert!(!actual.contains("call @leaf"), "{actual}");
    assert!(
        actual
            .contains("function @caller(ptr) -> i32 external {\n    label0 = block_label block1\n"),
        "{actual}"
    );
}

/// A threaded interpreter loop: each handler ends in an `indirect_br`
/// through a table of label addresses. The default pipeline keeps every
/// result and every bisect prefix agrees with the input.
#[test]
fn threaded_dispatch_survives_the_pipeline() {
    let input = "\
global @handlers internal constant size 24, align 8 = bytes \
                 \"000000000000000000000000000000000000000000000000\" relocs [0: \
                 blockaddress(@run, label0), 8: blockaddress(@run, label1), 16: \
                 blockaddress(@run, label2)]

function @run(i32) -> i32 external {
    slot0 = stack_slot 4, align 4
    slot1 = stack_slot 4, align 4
    label0 = block_label block1
    label1 = block_label block2
    label2 = block_label block3
block0(v0: i32):
    v1 = stack_addr slot0
    v2 = stack_addr slot1
    v3 = iconst.i32 0
    store.i32 v3, v1, align 4
    store.i32 v0, v2, align 4
    jump block4
block1:
    v4 = load.i32 v1, align 4
    v5 = iconst.i32 3
    v6 = iadd.i32 v4, v5
    store.i32 v6, v1, align 4
    jump block4
block2:
    v7 = load.i32 v1, align 4
    v8 = iconst.i32 2
    v9 = imul.i32 v7, v8
    store.i32 v9, v1, align 4
    jump block4
block3:
    v10 = load.i32 v1, align 4
    return v10
block4:
    v11 = load.i32 v2, align 4
    v12 = iconst.i32 1
    v13 = isub.i32 v11, v12
    store.i32 v13, v2, align 4
    v14 = iconst.i32 0
    v15 = icmp.i32 sle v11, v14
    v16 = iconst.i32 1
    v17 = and.i32 v11, v16
    v18 = iconst.i32 2
    v19 = select.i32 v15, v18, v17
    v20 = zext.i64 v19
    v21 = iconst.i64 8
    v22 = imul.i64 v20, v21
    v23 = global_addr @handlers
    v24 = ptr_add v23, v22
    v25 = load.ptr v24, align 8
    indirect_br v25, [block1, block2, block3]
}
";
    let run =
        |text: &str, steps: u128| equivalence::returned(text, "run", &[RuntimeValue::Int(steps)]);
    let expected: Vec<_> = (0..8).map(|steps| run(input, steps)).collect();
    assert_eq!(expected[3], RuntimeValue::Int(6));
    let options = OptimizerOptions {
        verify_each: true,
        ..OptimizerOptions::default()
    };
    let (optimized, report) = optimize_text(input, &options);
    assert!(!optimized.contains("stack_slot"), "{optimized}");
    for limit in 0..=report.transformations() {
        let options = OptimizerOptions {
            bisect_limit: Some(limit),
            verify_each: true,
            ..OptimizerOptions::default()
        };
        let (text, _) = optimize_text(input, &options);
        for (steps, value) in expected.iter().enumerate() {
            assert_eq!(
                run(&text, steps as u128),
                *value,
                "limit {limit}, {steps} steps"
            );
        }
    }
}
