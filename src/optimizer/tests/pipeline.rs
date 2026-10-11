//! The pipeline: several passes together, bisecting, verification, printing
//! and reports.

use std::cell::RefCell;

use super::*;
use crate::ir::{
    Entity,
    Value,
};

/// A loop whose exit test is constant false: the body never runs.
const DEAD_LOOP: &str = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 0
    jump block1(v1)
block1(v2: i32):
    v3 = iconst.i1 0
    brif v3, block2, block3
block2:
    v4 = iadd.i32 nsw v2, v0
    jump block1(v4)
block3:
    return v2
}
";

/// Nested diamonds whose outer condition is a constant comparison.
const NESTED_DIAMONDS: &str = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 1
    v2 = icmp.i32 eq v1, v1
    brif v2, block1, block4
block1:
    v3 = icmp.i32 sgt v0, v1
    brif v3, block2, block3
block2:
    v4 = iadd.i32 v0, v1
    jump block5(v4)
block3:
    v5 = isub.i32 v0, v1
    jump block5(v5)
block4:
    v6 = imul.i32 v0, v0
    jump block6(v6)
block5(v7: i32):
    jump block6(v7)
block6(v8: i32):
    return v8
}
";

#[test]
fn a_loop_with_a_constant_false_branch_disappears() {
    let expected = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 0
    return v1
}
";
    assert_pipeline(DEAD_LOOP, expected);
}

#[test]
fn nested_diamonds_lose_the_dead_arm_and_the_forwarding_block() {
    let expected = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 1
    v2 = icmp.i32 sgt v0, v1
    brif v2, block1, block2
block1:
    v3 = iadd.i32 v0, v1
    jump block3(v3)
block2:
    v4 = isub.i32 v0, v1
    jump block3(v4)
block3(v5: i32):
    return v5
}
";
    assert_pipeline(NESTED_DIAMONDS, expected);
}

#[test]
fn dead_code_chains_vanish() {
    let input = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iadd.i32 v0, v0
    v2 = iadd.i32 v1, v1
    v3 = imul.i32 v2, v1
    v4 = isub.i32 v3, v2
    v5 = iconst.i32 0
    v6 = iadd.i32 v4, v5
    jump block1(v6)
block1(v7: i32):
    return v0
}
";
    let expected = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    return v0
}
";
    assert_pipeline(input, expected);
}

#[test]
fn a_straight_line_of_constants_becomes_one_return() {
    let input = "\
function @f() -> i32 external {
block0:
    v0 = iconst.i32 2
    v1 = iconst.i32 3
    v2 = iadd.i32 nsw v0, v1
    v3 = iconst.i32 4
    v4 = imul.i32 nsw v2, v3
    v5 = iconst.i32 20
    v6 = isub.i32 v4, v5
    jump block1
block1:
    return v6
}
";
    let expected = "\
function @f() -> i32 external {
block0:
    v0 = iconst.i32 0
    return v0
}
";
    assert_pipeline(input, expected);
}

#[test]
fn code_that_is_already_minimal_is_returned_unchanged() {
    assert_pipeline(tests_loop::COUNT_LOOP, tests_loop::COUNT_LOOP);
}

#[test]
fn a_constant_in_a_loop_moves_to_its_preheader() {
    let input = "\
function @count(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 0
    jump block1(v1, v1)
block1(v2: i32, v3: i32):
    v4 = icmp.i32 slt v2, v0
    brif v4, block2, block3
block2:
    v5 = iadd.i32 nsw v3, v2
    v6 = iconst.i32 1
    v7 = iadd.i32 nsw v2, v6
    jump block1(v7, v5)
block3:
    return v3
}
";
    assert_pipeline(input, tests_loop::COUNT_LOOP);
}

mod tests_loop {
    /// The loop of the middle-end plan, with its constant already hoisted.
    pub(super) const COUNT_LOOP: &str = "\
function @count(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 0
    v2 = iconst.i32 1
    jump block1(v1, v1)
block1(v3: i32, v4: i32):
    v5 = icmp.i32 slt v3, v0
    brif v5, block2, block3
block2:
    v6 = iadd.i32 nsw v4, v3
    v7 = iadd.i32 nsw v3, v2
    jump block1(v7, v6)
block3:
    return v4
}
";
}

#[test]
fn optimizing_twice_changes_nothing_more() {
    for input in [DEAD_LOOP, NESTED_DIAMONDS] {
        let options = OptimizerOptions {
            verify_each: true,
            ..OptimizerOptions::default()
        };
        let (once, _) = optimize_text(input, &options);
        let (twice, report) = optimize_text(&once, &options);
        assert_eq!(once, twice);
        assert_eq!(report.transformations(), 0);
    }
}

#[test]
fn declarations_globals_and_every_function_are_handled() {
    let input = "\
target triple = \"x86_64-unknown-linux-gnu\"
target datalayout = \"e-m:e-i64:64-n8:16:32:64-S128\"

global @counter internal size 4, align 4 = zero

function @puts(ptr) -> i32 external

function @one() -> i32 internal {
block0:
    v0 = iconst.i32 1
    v1 = iconst.i32 1
    v2 = iadd.i32 v0, v1
    return v2
}

function @two(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 0
    v2 = iadd.i32 v0, v1
    return v2
}
";
    let expected = "\
target triple = \"x86_64-unknown-linux-gnu\"
target datalayout = \"e-m:e-i64:64-n8:16:32:64-S128\"

global @counter internal size 4, align 4 = zero

function @puts(ptr) -> i32 external

function @one() -> i32 internal {
block0:
    v0 = iconst.i32 2
    return v0
}

function @two(i32) -> i32 external {
block0(v0: i32):
    return v0
}
";
    let options = OptimizerOptions {
        verify_each: true,
        ..OptimizerOptions::default()
    };
    let (text, report) = optimize_text(input, &options);
    assert_eq!(text, expected);
    assert_eq!(report.functions, 2);
}

#[test]
fn an_explicit_pass_list_runs_once_in_the_order_given() {
    // `dce` before `fold` cannot remove what `fold` has not yet made dead.
    let input = "function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 0
    v2 = iadd.i32 v0, v1
    return v2
}
";
    let fold_then_dce = "function @f(i32) -> i32 external {
block0(v0: i32):
    return v0
}
";
    let dce_then_fold = "function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 0
    return v0
}
";
    let (text, report) = optimize_text(input, &only(&[Pass::Fold, Pass::Dce]));
    assert_eq!(text, fold_then_dce);
    assert_eq!(report.rounds, 1);
    let (text, _) = optimize_text(input, &only(&[Pass::Dce, Pass::Fold]));
    assert_eq!(text, dce_then_fold);
    let (text, _) = optimize_text(input, &only(&[]));
    assert_eq!(text, input);
}

#[test]
fn bisecting_applies_exactly_the_first_n_rewrites() {
    let options = OptimizerOptions {
        verify_each: true,
        ..OptimizerOptions::default()
    };
    for input in [DEAD_LOOP, NESTED_DIAMONDS] {
        let (full_text, full) = optimize_text(input, &options);
        let total = full.transformations();
        assert!(total > 3, "the test input needs several rewrites");
        assert!(!full.limit_reached());
        for limit in 0..=total + 2 {
            let limited = OptimizerOptions {
                bisect_limit: Some(limit),
                verify_each: true,
                ..OptimizerOptions::default()
            };
            let (text, report) = optimize_text(input, &limited);
            assert_eq!(report.transformations(), limit.min(total), "limit {limit}");
            assert_eq!(report.limit_reached(), limit < total, "limit {limit}");
            if limit >= total {
                assert_eq!(text, full_text, "limit {limit}");
            } else if limit == 0 {
                assert_eq!(text, input);
            } else {
                assert_ne!(text, full_text, "limit {limit}");
            }
        }
    }
}

#[test]
fn bisecting_pins_the_rewrite_that_changes_the_output() {
    // Walking the limit up changes the printed function at every step, so the
    // rewrite a limit adds is the one to blame in a miscompile.
    let mut previous = String::new();
    let (_, full) = optimize_text(
        DEAD_LOOP,
        &OptimizerOptions {
            verify_each: true,
            ..OptimizerOptions::default()
        },
    );
    for limit in 0..=full.transformations() {
        let limited = OptimizerOptions {
            bisect_limit: Some(limit),
            verify_each: true,
            ..OptimizerOptions::default()
        };
        let (text, _) = optimize_text(DEAD_LOOP, &limited);
        assert_ne!(
            text, previous,
            "rewrite {limit} left the function as it was"
        );
        previous = text;
    }
}

/// Appends an argument to the first edge, which breaks the function.
fn break_an_edge(draft: &mut Draft<'_, '_>) -> bool {
    for block in &mut draft.blocks {
        if let Some(edge) = block.term.edges_mut().next() {
            edge.args.push(Value::new(0));
            return true;
        }
    }
    false
}

const TWO_BLOCKS: &str = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iadd.i32 v0, v0
    jump block1(v1)
block1(v2: i32):
    v3 = icmp.i32 eq v2, v0
    brif v3, block1(v2), block2
block2:
    return v2
}
";

#[test]
#[should_panic(expected = "does not pass the verifier after fold")]
fn verification_after_each_pass_catches_a_broken_pass() {
    let options = OptimizerOptions {
        verify_each: true,
        injected: Some(break_an_edge),
        ..only(&[Pass::Fold])
    };
    drop(optimize_text(TWO_BLOCKS, &options));
}

#[test]
fn without_verification_a_broken_pass_goes_unnoticed() {
    let arena = Bump::new();
    let scratch = Bump::new();
    let mut module = parse(&arena, TWO_BLOCKS);
    let options = OptimizerOptions {
        verify_each: false,
        injected: Some(break_an_edge),
        ..only(&[Pass::Fold])
    };
    _ = optimize(&mut module, &options, &scratch);
    let errors = verify_module(&module, Profile::PreAbi, &scratch);
    assert!(!errors.is_empty());
}

#[test]
#[should_panic(expected = "before optimization")]
fn the_input_is_verified_first() {
    // `v1` is used before its definition in a block that does not dominate.
    let input = "\
function @f(i1) -> i32 external {
block0(v0: i1):
    brif v0, block1, block2
block1:
    v1 = iconst.i32 1
    jump block2
block2:
    return v1
}
";
    let arena = Bump::new();
    let scratch = Bump::new();
    let mut module = parse(&arena, input);
    let options = OptimizerOptions {
        verify_each: true,
        ..only(&[Pass::Fold])
    };
    _ = optimize(&mut module, &options, &scratch);
}

#[test]
fn print_after_all_shows_the_function_after_each_pass() {
    let sink = RefCell::new(String::new());
    let options = OptimizerOptions {
        print_after_all: Some(&sink),
        ..only(&[Pass::Fold, Pass::SimplifyCfg, Pass::Dce])
    };
    let (final_text, _) = optimize_text(DEAD_LOOP, &options);
    let printed = sink.into_inner();
    let headers: Vec<&str> = printed
        .lines()
        .filter(|line| line.starts_with("***"))
        .collect();
    assert_eq!(
        headers,
        [
            "*** IR after fold of @f ***",
            "*** IR after simplify-cfg of @f ***",
            "*** IR after dce of @f ***"
        ]
    );
    assert!(printed.ends_with(&final_text), "{printed}");
    let after_fold = printed.split("*** IR after simplify-cfg").next().unwrap();
    assert!(after_fold.contains("    jump block3\n"), "{after_fold}");
}

#[test]
fn print_after_all_prints_even_when_a_pass_changes_nothing() {
    let sink = RefCell::new(String::new());
    let options = OptimizerOptions {
        print_after_all: Some(&sink),
        ..only(&[Pass::Dce])
    };
    let (text, _) = optimize_text(tests_loop::COUNT_LOOP, &options);
    assert_eq!(text, tests_loop::COUNT_LOOP);
    let printed = sink.into_inner();
    assert!(
        printed.starts_with("*** IR after dce of @count ***\n"),
        "{printed}"
    );
    assert!(printed.ends_with(tests_loop::COUNT_LOOP), "{printed}");
}

/// Two loops that give every pass work. The first keeps a flag in a slot:
/// `promote` turns the slot into a parameter, `fold` drops the `+ 0` and `dce`
/// drops the unused product. The second computes an invariant product twice,
/// for `gvn` and then `licm`, and its exit chain is for `simplify-cfg`.
const EVERY_PASS: &str = "\
function @f(i32) -> i32 external {
    slot0 = stack_slot 4, align 4
block0(v0: i32):
    v1 = stack_addr slot0
    v2 = iconst.i32 1
    store.i32 v2, v1, align 4
    v3 = iconst.i32 0
    jump block1(v3)
block1(v4: i32):
    v5 = load.i32 v1, align 4
    v6 = icmp.i32 slt v4, v0
    brif v6, block2, block3
block2:
    v7 = iadd.i32 v5, v3
    store.i32 v7, v1, align 4
    v8 = iadd.i32 nsw v4, v5
    jump block1(v8)
block3:
    v9 = imul.i32 v4, v4
    jump block4(v4)
block4(v10: i32):
    v11 = imul.i32 v0, v0
    v12 = imul.i32 v0, v0
    v13 = iadd.i32 v11, v12
    v14 = iadd.i32 v10, v13
    v15 = icmp.i32 slt v14, v0
    brif v15, block4(v14), block5
block5:
    jump block6
block6:
    v16 = iadd.i32 v14, v5
    return v16
}
";

#[test]
fn the_report_counts_rewrites_and_times_passes() {
    let (_, report) = optimize_text(
        EVERY_PASS,
        &OptimizerOptions {
            verify_each: true,
            ..OptimizerOptions::default()
        },
    );
    let by_pass: u64 = Pass::ALL
        .iter()
        .map(|&pass| report.stats(pass).changes)
        .sum();
    assert_eq!(by_pass, report.transformations());
    for pass in Pass::ALL {
        assert!(report.stats(pass).runs >= 1, "{pass} ran");
        assert!(report.stats(pass).changes >= 1, "{pass} rewrote something");
    }
    assert!(report.rounds >= 2, "a final round confirms nothing changes");
    assert!(report.total_time() >= report.stats(Pass::Fold).time);
    let table = report.to_string();
    for pass in Pass::ALL {
        assert!(table.contains(pass.name()), "{table}");
    }
}

#[test]
fn verification_defaults_on_in_debug_builds() {
    assert_eq!(
        OptimizerOptions::default().verify_each,
        cfg!(debug_assertions)
    );
    assert!(OptimizerOptions::default().passes.is_none());
    assert!(OptimizerOptions::default().bisect_limit.is_none());
}
