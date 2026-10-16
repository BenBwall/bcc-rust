//! Select formation, speculation refusals, and interpreter equivalence.
use super::*;
use crate::{
    backend::interpreter::{
        Limits,
        RuntimeValue,
        run_text_with,
    },
    ir::Type,
};

const DIAMOND: &str = "function @f(i1, i32, i32) -> i32 external {
block0(v0: i1, v1: i32, v2: i32):
    brif v0, block1, block2
block1:
    v3 = iadd.i32 nsw v1, v2
    jump block3(v3)
block2:
    v4 = isub.i32 nuw v1, v2
    jump block3(v4)
block3(v5: i32):
    return v5
}
";

#[test]
fn diamond_moves_cheap_arms_and_drops_flags() {
    assert_pass(
        Pass::SimplifyCfg,
        DIAMOND,
        "function @f(i1, i32, i32) -> i32 external {
block0(v0: i1, v1: i32, v2: i32):
    v3 = iadd.i32 v1, v2
    v4 = isub.i32 v1, v2
    v5 = select.i32 v0, v3, v4
    return v5
}
",
    );
    equivalent(
        DIAMOND,
        "f",
        &[Type::I1, Type::I32, Type::I32],
        &[&[0, 2_147_483_647, 1], &[1, 7, 9], &[0, 9, 7]],
    );
}

#[test]
fn triangle_with_arm_parameters_and_multiple_join_parameters() {
    let text = "function @f(i1, i32, i32) -> i32 external {
block0(v0: i1, v1: i32, v2: i32):
    brif v0, block1(v1), block2(v1, v2)
block1(v3: i32):
    v4 = iadd.i32 nsw v3, v2
    jump block2(v4, v3)
block2(v5: i32, v6: i32):
    v7 = isub.i32 v5, v6
    return v7
}
";
    assert_pass(
        Pass::SimplifyCfg,
        text,
        "function @f(i1, i32, i32) -> i32 external {
block0(v0: i1, v1: i32, v2: i32):
    v3 = iadd.i32 v1, v2
    v4 = select.i32 v0, v3, v1
    v5 = select.i32 v0, v1, v2
    v6 = isub.i32 v4, v5
    return v6
}
",
    );
    for limit in 0..=3 {
        let options = OptimizerOptions {
            bisect_limit: Some(limit),
            ..only(&[Pass::SimplifyCfg])
        };
        let (out, report) = optimize_text(text, &options);
        assert!(report.transformations() <= limit);
        assert_eq!(
            out.matches("select.i32").count(),
            usize::try_from(limit.min(2)).unwrap()
        );
        compare(
            text,
            &out,
            "f",
            &[Type::I1, Type::I32, Type::I32],
            &[&[0, 7, 3], &[1, 7, 3]],
        );
    }
}

#[test]
fn one_zero_select_becomes_zext() {
    assert_pass(
        Pass::SimplifyCfg,
        "function @f(i1) -> i32 external {
block0(v0: i1):
    brif v0, block1, block2
block1:
    v1 = iconst.i32 1
    jump block3(v1)
block2:
    v2 = iconst.i32 0
    jump block3(v2)
block3(v3: i32):
    return v3
}
",
        "function @f(i1) -> i32 external {
block0(v0: i1):
    v1 = iconst.i32 1
    v2 = iconst.i32 0
    v3 = zext.i32 v0
    return v3
}
",
    );
}

#[test]
fn expensive_or_trapping_arms_are_refused() {
    for op in ["sdiv", "udiv", "srem", "urem", "fdiv"] {
        let text = if op == "fdiv" {
            DIAMOND
                .replace("i32", "f64")
                .replace("iadd.f64 nsw", "fdiv.f64")
                .replace("isub.f64 nuw", "fsub.f64")
        } else {
            DIAMOND.replace("iadd.i32 nsw", &format!("{op}.i32"))
        };
        assert_pass_leaves(Pass::SimplifyCfg, &text);
    }
    let text = DIAMOND.replace("v3 = iadd.i32 nsw v1, v2", "v3 = freeze.i32 v1");
    assert_pass_leaves(Pass::SimplifyCfg, &text);
    let text = DIAMOND.replace(
        "v3 = iadd.i32 nsw v1, v2",
        "v3 = iadd.i32 v1, v2
    v6 = iadd.i32 v3, v2
    v7 = iadd.i32 v6, v2
    v8 = iadd.i32 v7, v2
    v9 = iadd.i32 v8, v2",
    );
    let (out, _) = optimize_text(&text, &only(&[Pass::SimplifyCfg]));
    assert!(!out.contains("select"));
    let memory = "function @g(i32) -> i32 external

function @f(i1, ptr, i32) -> i32 external {
block0(v0: i1, v1: ptr, v2: i32):
    brif v0, block1, block2
block1:
    v3 = load.i32 v1, align 4
    jump block3(v3)
block2:
    v4 = iadd.i32 v2, v2
    jump block3(v4)
block3(v5: i32):
    return v5
}
";
    for arm in [
        "v3 = load.i32 v1, align 4",
        "v3 = load.i32 volatile v1, align 4",
        "store.i32 v2, v1, align 4
    v3 = iadd.i32 v2, v2",
        "v3 = call @g(v2)",
    ] {
        assert_pass_leaves(
            Pass::SimplifyCfg,
            &memory.replace("v3 = load.i32 v1, align 4", arm),
        );
    }
}

#[test]
fn shared_arms_or_joins_are_not_if_converted() {
    let text = "function @f(i1, i1, i32) -> i32 external {
block0(v0: i1, v1: i1, v2: i32):
    brif v0, block1, block2
block1:
    brif v1, block3, block4
block2:
    jump block3
block3:
    v3 = iadd.i32 v2, v2
    jump block5(v3)
block4:
    v4 = isub.i32 v2, v2
    jump block5(v4)
block5(v5: i32):
    return v5
}
";
    let (out, _) = optimize_text(text, &only(&[Pass::SimplifyCfg]));
    assert!(!out.contains("select"));
}

#[test]
fn levenshtein_style_minimum_recurrence() {
    let text = "function @f(i32, i32, i32) -> i32 external {
block0(v0: i32, v1: i32, v2: i32):
    v3 = iconst.i32 0
    v4 = iconst.i32 1
    jump block1(v3, v1)
block1(v5: i32, v6: i32):
    v7 = icmp.i32 slt v5, v0
    brif v7, block2, block6
block2:
    v8 = iadd.i32 v6, v4
    v9 = iadd.i32 v5, v2
    v10 = icmp.i32 slt v8, v9
    brif v10, block3, block4
block3:
    jump block5(v8)
block4:
    jump block5(v9)
block5(v11: i32):
    v12 = iadd.i32 v5, v4
    jump block1(v12, v11)
block6:
    return v6
}
";
    equivalent(
        text,
        "f",
        &[Type::I32; 3],
        &[&[0, 3, 7], &[20, 3, 7], &[11, -4, 9], &[30, 9, -2]],
    );
    let (out, _) = optimize_text(text, &only(&[Pass::SimplifyCfg]));
    assert!(out.contains("select.i32"));
}

#[test]
fn merge_loop_style_value_and_index_selection() {
    let text = "function @f(i32, i32, i32) -> i32 external {
block0(v0: i32, v1: i32, v2: i32):
    v3 = iconst.i32 0
    v4 = iconst.i32 1
    jump block1(v3, v3, v3)
block1(v5: i32, v6: i32, v7: i32):
    v8 = iadd.i32 v5, v6
    v9 = icmp.i32 slt v8, v0
    brif v9, block2, block6
block2:
    v10 = iadd.i32 v1, v5
    v11 = iadd.i32 v2, v6
    v12 = icmp.i32 slt v11, v10
    brif v12, block3, block4
block3:
    v13 = iadd.i32 nsw v6, v4
    jump block5(v5, v13, v11)
block4:
    v14 = iadd.i32 nsw v5, v4
    jump block5(v14, v6, v10)
block5(v15: i32, v16: i32, v17: i32):
    v18 = iadd.i32 v7, v17
    jump block1(v15, v16, v18)
block6:
    return v7
}
";
    equivalent(
        text,
        "f",
        &[Type::I32; 3],
        &[&[0, 0, 0], &[30, 0, 0], &[30, 1, 8], &[17, -8, 4]],
    );
    let (out, _) = optimize_text(text, &only(&[Pass::SimplifyCfg]));
    assert_eq!(out.matches("select.i32").count(), 3);
}

fn equivalent(text: &str, entry: &str, types: &[Type], inputs: &[&[i64]]) {
    for options in [only(&[Pass::SimplifyCfg]), OptimizerOptions::default()] {
        let (out, _) = optimize_text(text, &options);
        compare(text, &out, entry, types, inputs);
    }
}

pub(super) fn compare(before: &str, after: &str, entry: &str, types: &[Type], inputs: &[&[i64]]) {
    for input in inputs {
        let args: Vec<_> = input
            .iter()
            .zip(types)
            .map(|(&v, &ty)| RuntimeValue::int(ty, v as u128))
            .collect();
        let a = run_text_with(before, entry, &args, Limits::default());
        let b = run_text_with(after, entry, &args, Limits::default());
        assert!(a.result.is_ok(), "{a:?}");
        assert_eq!(
            a.result.unwrap().termination,
            b.result.unwrap().termination,
            "input {input:?}\n{after}"
        );
        assert_eq!(a.output, b.output);
    }
}
