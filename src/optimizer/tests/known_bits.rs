//! Dominating conditions, sign propagation and division/remainder rewrites.
use super::*;
use crate::ir::Type;

const EVEN: &str = "function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 1
    v2 = iconst.i32 0
    v3 = iconst.i32 2
    v4 = and.i32 v0, v1
    v5 = icmp.i32 eq v4, v2
    brif v5, block1, block2
block1:
    v6 = sdiv.i32 v0, v3
    return v6
block2:
    return v0
}
";

#[test]
fn even_condition_gives_exact_signed_shift_with_unknown_sign() {
    assert_pass(
        Pass::KnownBits,
        EVEN,
        "function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 1
    v2 = iconst.i32 1
    v3 = iconst.i32 0
    v4 = iconst.i32 2
    v5 = and.i32 v0, v2
    v6 = icmp.i32 eq v5, v3
    brif v6, block1, block2
block1:
    v7 = ashr.i32 exact v0, v1
    return v7
block2:
    return v0
}
",
    );
    equivalent(EVEN, &[&[-2_147_483_648], &[-18], &[-3], &[0], &[7], &[8]]);
}

#[test]
fn nonnegative_condition_rewrites_division_and_remainder() {
    for (op, result) in [
        ("sdiv", "lshr"),
        ("udiv", "lshr"),
        ("srem", "and"),
        ("urem", "and"),
    ] {
        let text = EVEN
            .replace("icmp.i32 eq v4, v2", "icmp.i32 sge v0, v2")
            .replace("sdiv.i32", &format!("{op}.i32"));
        let (out, report) = optimize_text(&text, &only(&[Pass::KnownBits]));
        assert!(out.contains(&format!("{result}.i32 v0, v1")), "{out}");
        assert_eq!(report.transformations(), 1);
        equivalent(&text, &[&[-19], &[0], &[1], &[19], &[2_147_483_647]]);
    }
}

#[test]
fn facts_do_not_leak_to_siblings_joins_or_loop_headers() {
    assert_pass_leaves(Pass::KnownBits, &EVEN.replace("icmp.i32 eq", "icmp.i32 ne"));
    assert_pass_leaves(
        Pass::KnownBits,
        &EVEN.replace("iconst.i32 2", "iconst.i32 4"),
    );
    assert_pass_leaves(
        Pass::KnownBits,
        &EVEN.replace("iconst.i32 2", "iconst.i32 -2"),
    );
    let text = EVEN.replace("return v6", "jump block3").replace(
        "return v0
}",
        "jump block3
block3:
    v7 = sdiv.i32 v0, v3
    return v7
}",
    );
    let (out, _) = optimize_text(&text, &only(&[Pass::KnownBits]));
    assert_eq!(out.matches("ashr.i32 exact").count(), 1);
    assert_eq!(out.matches("sdiv.i32").count(), 1);
    let loop_text = "function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 0
    v2 = iconst.i32 2
    jump block1(v0)
block1(v3: i32):
    v4 = sdiv.i32 v3, v2
    v5 = icmp.i32 sge v3, v1
    brif v5, block1(v4), block2
block2:
    return v4
}
";
    assert_pass_leaves(Pass::KnownBits, loop_text);
}

#[test]
fn false_swapped_and_combined_conditions() {
    let text = EVEN
        .replace("icmp.i32 eq v4, v2", "icmp.i32 sgt v2, v0")
        .replace("brif v5, block1, block2", "brif v5, block2, block1");
    let (out, _) = optimize_text(&text, &only(&[Pass::KnownBits]));
    assert!(out.contains("lshr.i32"));
    equivalent(&text, &[&[-9], &[0], &[11]]);
    // Two nested equality guards contribute disjoint bits of the same value.
    let text = "function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 1
    v2 = iconst.i32 2
    v3 = iconst.i32 0
    v4 = iconst.i32 4
    v5 = and.i32 v0, v1
    v6 = icmp.i32 eq v5, v3
    brif v6, block1, block3
block1:
    v7 = and.i32 v0, v2
    v8 = icmp.i32 eq v3, v7
    brif v8, block2, block3
block2:
    v9 = sdiv.i32 v0, v4
    return v9
block3:
    return v0
}
";
    let (out, _) = optimize_text(text, &only(&[Pass::KnownBits]));
    assert!(out.contains("ashr.i32 exact"));
    equivalent(text, &[&[-16], &[-6], &[8], &[11], &[14]]);
}

#[test]
fn ordered_nsw_difference_is_nonnegative() {
    let text = "function @f(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = iconst.i32 2
    v3 = icmp.i32 slt v0, v1
    brif v3, block1, block2
block1:
    v4 = isub.i32 nsw v1, v0
    v5 = sdiv.i32 v4, v2
    return v5
block2:
    return v0
}
";
    let (out, _) = optimize_text(text, &only(&[Pass::KnownBits]));
    assert!(out.contains("lshr.i32"));
    assert_pass_leaves(Pass::KnownBits, &text.replace("isub.i32 nsw", "isub.i32"));
    select::compare(
        text,
        &out,
        "f",
        &[Type::I32; 2],
        &[&[2, 9], &[-9, -2], &[9, 2]],
    );
}

#[test]
fn collatz_loop_rewrites_even_arm_then_forms_select() {
    let text = "function @f(i64) -> i64 external {
block0(v0: i64):
    v1 = iconst.i64 1
    v2 = iconst.i64 0
    v3 = iconst.i64 2
    v4 = iconst.i64 3
    jump block1(v0, v2)
block1(v5: i64, v6: i64):
    v7 = icmp.i64 sgt v5, v1
    brif v7, block2, block6
block2:
    v8 = and.i64 v5, v1
    v9 = icmp.i64 ne v8, v2
    brif v9, block3, block4
block3:
    v10 = imul.i64 nsw v5, v4
    v11 = iadd.i64 nsw v10, v1
    jump block5(v11)
block4:
    v12 = sdiv.i64 v5, v3
    jump block5(v12)
block5(v13: i64):
    v14 = iadd.i64 v6, v1
    jump block1(v13, v14)
block6:
    return v6
}
";
    let (out, _) = optimize_text(text, &only(&[Pass::KnownBits, Pass::SimplifyCfg]));
    assert!(out.contains("ashr.i64"));
    assert!(!out.contains("exact")); // speculating removes the path's proof.
    assert!(out.contains("select.i64"));
    assert!(!out.contains("sdiv"));
    let inputs = [
        &[1][..],
        &[2][..],
        &[7][..],
        &[27][..],
        &[97][..],
        &[0][..],
        &[-2][..],
    ];
    select::compare(text, &out, "f", &[Type::I64], &inputs);
    let (out, _) = optimize_text(text, &OptimizerOptions::default());
    select::compare(text, &out, "f", &[Type::I64], &inputs);
}

#[test]
fn rewrite_bisect_limit_and_local_masks() {
    let text = "function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 15
    v2 = iconst.i32 4
    v3 = and.i32 v0, v1
    v4 = sdiv.i32 v3, v2
    v5 = srem.i32 v3, v2
    v6 = iadd.i32 v4, v5
    return v6
}
";
    for limit in 0..=2 {
        let (out, report) = optimize_text(
            text,
            &OptimizerOptions {
                bisect_limit: Some(limit),
                ..only(&[Pass::KnownBits])
            },
        );
        assert_eq!(report.transformations(), limit);
        select::compare(text, &out, "f", &[Type::I32], &[&[-99], &[0], &[19]]);
    }
}

fn equivalent(text: &str, inputs: &[&[i64]]) {
    for options in [only(&[Pass::KnownBits]), OptimizerOptions::default()] {
        let (out, _) = optimize_text(text, &options);
        select::compare(text, &out, "f", &[Type::I32], inputs);
    }
}

#[test]
fn single_predecessor_parameters_inherit_guard_facts() {
    let text = EVEN
        .replace("brif v5, block1, block2", "brif v5, block1(v0), block2")
        .replace(
            "block1:
    v6 = sdiv.i32 v0, v3",
            "block1(v7: i32):
    v6 = sdiv.i32 v7, v3",
        );
    let (out, _) = optimize_text(&text, &only(&[Pass::KnownBits]));
    assert!(out.contains("ashr.i32 exact"));
    equivalent(&text, &[&[-16], &[-3], &[8]]);
}

#[test]
fn unsigned_high_bit_divisor_and_exact_flags() {
    let text = "function @f(i8) -> i8 external {
block0(v0: i8):
    v1 = iconst.i8 -128
    v2 = udiv.i8 exact v0, v1
    v3 = urem.i8 v0, v1
    v4 = iadd.i8 v2, v3
    return v4
}
";
    let (out, _) = optimize_text(text, &only(&[Pass::KnownBits]));
    assert!(out.contains("lshr.i8 exact"));
    assert!(out.contains("and.i8"));
    select::compare(text, &out, "f", &[Type::I8], &[&[0], &[128]]);
    assert_pass_leaves(
        Pass::KnownBits,
        &text.replace("udiv", "sdiv").replace("urem", "srem"),
    );
}
