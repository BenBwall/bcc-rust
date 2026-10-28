//! Numeric instructions, with values from the specification's test suite
//! (`i32.wast`, `i64.wast`, `f32.wast`, `conversions.wast` and friends).

use super::{
    eval_f32,
    eval_f64,
    eval_i32,
    eval_i64,
    trap,
};
use crate::{
    TrapKind,
    ValType,
};

#[test]
fn i32_arithmetic_wraps() {
    assert_eq!(eval_i32("i32.const 2 i32.const 3 i32.add"), 5);
    assert_eq!(
        eval_i32("i32.const 0x7fffffff i32.const 1 i32.add"),
        i32::MIN
    );
    assert_eq!(
        eval_i32("i32.const 0x80000000 i32.const 1 i32.sub"),
        i32::MAX
    );
    assert_eq!(eval_i32("i32.const 0x10000000 i32.const 4096 i32.mul"), 0);
    assert_eq!(
        eval_i32("i32.const 0x01234567 i32.const 0x76543210 i32.mul"),
        0x358E_7470
    );
}

#[test]
fn i32_division_traps_and_rounds_toward_zero() {
    assert_eq!(eval_i32("i32.const -7 i32.const 2 i32.div_s"), -3);
    assert_eq!(eval_i32("i32.const -7 i32.const 2 i32.div_u"), 0x7FFF_FFFC);
    assert_eq!(eval_i32("i32.const -7 i32.const 2 i32.rem_s"), -1);
    assert_eq!(eval_i32("i32.const 7 i32.const -2 i32.rem_s"), 1);
    assert_eq!(eval_i32("i32.const -7 i32.const 2 i32.rem_u"), 1);
    // The one overflowing quotient traps; the matching remainder is 0.
    assert_eq!(
        trap(ValType::I32, "i32.const 0x80000000 i32.const -1 i32.div_s"),
        TrapKind::IntegerOverflow
    );
    assert_eq!(eval_i32("i32.const 0x80000000 i32.const -1 i32.rem_s"), 0);
    for op in ["i32.div_s", "i32.div_u", "i32.rem_s", "i32.rem_u"] {
        assert_eq!(
            trap(ValType::I32, &format!("i32.const 1 i32.const 0 {op}")),
            TrapKind::DivideByZero,
            "{op}"
        );
    }
}

#[test]
fn i32_bits_shifts_and_rotations() {
    assert_eq!(eval_i32("i32.const 0 i32.clz"), 32);
    assert_eq!(eval_i32("i32.const 0x00008000 i32.clz"), 16);
    assert_eq!(eval_i32("i32.const 0 i32.ctz"), 32);
    assert_eq!(eval_i32("i32.const 0x00010000 i32.ctz"), 16);
    assert_eq!(eval_i32("i32.const -1 i32.popcnt"), 32);
    assert_eq!(
        eval_i32("i32.const 0xf0f0f0f0 i32.const 0x0ff00ff0 i32.and"),
        0x00F0_00F0
    );
    assert_eq!(
        eval_i32("i32.const 0xf0f0f0f0 i32.const 0x0ff00ff0 i32.or"),
        0xFFF0_FFF0_u32 as i32
    );
    assert_eq!(
        eval_i32("i32.const 0xf0f0f0f0 i32.const 0x0ff00ff0 i32.xor"),
        0xFF00_FF00_u32 as i32
    );
    // Shift counts are taken modulo 32.
    assert_eq!(eval_i32("i32.const 1 i32.const 33 i32.shl"), 2);
    assert_eq!(eval_i32("i32.const -1 i32.const 1 i32.shr_s"), -1);
    assert_eq!(eval_i32("i32.const -1 i32.const 1 i32.shr_u"), i32::MAX);
    assert_eq!(
        eval_i32("i32.const 0x80000000 i32.const 32 i32.shr_u"),
        i32::MIN
    );
    assert_eq!(
        eval_i32("i32.const 0xabcd9876 i32.const 1 i32.rotl"),
        0x579B_30ED
    );
    assert_eq!(
        eval_i32("i32.const 0xb0c1d2e3 i32.const 5 i32.rotr"),
        0x1D86_0E97
    );
    assert_eq!(eval_i32("i32.const 1 i32.const -1 i32.rotr"), 2);
}

#[test]
fn i32_comparisons_distinguish_signedness() {
    assert_eq!(eval_i32("i32.const 0 i32.eqz"), 1);
    assert_eq!(eval_i32("i32.const 7 i32.eqz"), 0);
    assert_eq!(eval_i32("i32.const -1 i32.const 1 i32.lt_s"), 1);
    assert_eq!(eval_i32("i32.const -1 i32.const 1 i32.lt_u"), 0);
    assert_eq!(eval_i32("i32.const -1 i32.const 1 i32.gt_u"), 1);
    assert_eq!(eval_i32("i32.const 1 i32.const 1 i32.le_s"), 1);
    assert_eq!(eval_i32("i32.const 0 i32.const -1 i32.ge_u"), 0);
    assert_eq!(eval_i32("i32.const 3 i32.const 3 i32.ne"), 0);
}

#[test]
fn i64_arithmetic_and_comparisons() {
    assert_eq!(
        eval_i64("i64.const 0x7fffffffffffffff i64.const 1 i64.add"),
        i64::MIN
    );
    assert_eq!(eval_i64("i64.const -7 i64.const 2 i64.div_s"), -3);
    assert_eq!(
        eval_i64("i64.const -7 i64.const 2 i64.div_u"),
        0x7FFF_FFFF_FFFF_FFFC
    );
    assert_eq!(
        trap(
            ValType::I64,
            "i64.const 0x8000000000000000 i64.const -1 i64.div_s"
        ),
        TrapKind::IntegerOverflow
    );
    assert_eq!(
        trap(ValType::I64, "i64.const 1 i64.const 0 i64.rem_u"),
        TrapKind::DivideByZero
    );
    assert_eq!(eval_i64("i64.const 1 i64.const 65 i64.shl"), 2);
    assert_eq!(eval_i64("i64.const 0 i64.clz"), 64);
    assert_eq!(eval_i64("i64.const 0x00008000 i64.popcnt"), 1);
    assert_eq!(
        eval_i64("i64.const 0xabcd987602468ace i64.const 1 i64.rotl"),
        0x579B_30EC_048D_159D
    );
    assert_eq!(eval_i32("i64.const -1 i64.const 1 i64.lt_u"), 0);
    assert_eq!(eval_i32("i64.const 0 i64.eqz"), 1);
}

#[test]
fn float_arithmetic_follows_ieee_754() {
    assert_eq!(
        f32::from_bits(eval_f32("f32.const 1.5 f32.const 2.25 f32.add")),
        3.75
    );
    assert_eq!(
        f64::from_bits(eval_f64("f64.const 1 f64.const 3 f64.div")),
        1.0 / 3.0
    );
    assert_eq!(
        eval_f32("f32.const 1 f32.const 0 f32.div"),
        f32::INFINITY.to_bits()
    );
    assert_eq!(
        eval_f32("f32.const 0 f32.const 0 f32.div"),
        0x7FC0_0000,
        "canonical NaN"
    );
    assert_eq!(
        f32::from_bits(eval_f32("f32.const 2 f32.sqrt")),
        2_f32.sqrt()
    );
    assert_eq!(eval_f32("f32.const -1 f32.sqrt"), 0x7FC0_0000);
}

#[test]
fn abs_neg_and_copysign_change_only_the_sign_bit() {
    // A NaN's payload survives.
    assert_eq!(eval_f32("f32.const bits:0xff800001 f32.abs"), 0x7F80_0001);
    assert_eq!(eval_f32("f32.const bits:0x7f800001 f32.neg"), 0xFF80_0001);
    assert_eq!(eval_f32("f32.const 0 f32.neg"), 0x8000_0000);
    assert_eq!(
        eval_f32("f32.const 1.5 f32.const -0 f32.copysign"),
        (-1.5_f32).to_bits()
    );
    assert_eq!(
        eval_f64("f64.const bits:0x7ff0000000000001 f64.const -1 f64.copysign"),
        0xFFF0_0000_0000_0001
    );
}

#[test]
fn min_and_max_propagate_nan_and_order_zeros() {
    assert_eq!(eval_f32("f32.const 0 f32.const -0 f32.min"), 0x8000_0000);
    assert_eq!(eval_f32("f32.const -0 f32.const 0 f32.max"), 0);
    assert_eq!(eval_f32("f32.const nan f32.const 1 f32.min"), 0x7FC0_0000);
    assert_eq!(
        eval_f64("f64.const 1 f64.const nan f64.max"),
        0x7FF8_0000_0000_0000
    );
    assert_eq!(
        f64::from_bits(eval_f64("f64.const -3 f64.const 2 f64.min")),
        -3.0
    );
}

#[test]
fn rounding_operators() {
    let f32_of = |code: &str| f32::from_bits(eval_f32(code));
    assert_eq!(f32_of("f32.const 2.5 f32.nearest"), 2.0, "ties to even");
    assert_eq!(f32_of("f32.const 3.5 f32.nearest"), 4.0);
    assert_eq!(
        eval_f32("f32.const -0.5 f32.nearest"),
        0x8000_0000,
        "keeps -0"
    );
    assert_eq!(f32_of("f32.const -1.5 f32.ceil"), -1.0);
    assert_eq!(f32_of("f32.const -1.5 f32.floor"), -2.0);
    assert_eq!(f32_of("f32.const -1.5 f32.trunc"), -1.0);
    assert_eq!(f64::from_bits(eval_f64("f64.const 0.5 f64.nearest")), 0.0);
}

#[test]
fn float_comparisons_are_false_with_nan() {
    assert_eq!(eval_i32("f32.const nan f32.const nan f32.eq"), 0);
    assert_eq!(eval_i32("f32.const nan f32.const nan f32.ne"), 1);
    assert_eq!(eval_i32("f64.const nan f64.const 1 f64.lt"), 0);
    assert_eq!(eval_i32("f64.const 0 f64.const -0 f64.eq"), 1);
    assert_eq!(eval_i32("f64.const 1 f64.const 2 f64.le"), 1);
}

#[test]
fn trapping_truncation_checks_its_range() {
    assert_eq!(eval_i32("f32.const -1.9 i32.trunc_f32_s"), -1);
    assert_eq!(
        eval_i32("f32.const -0.9 i32.trunc_f32_u"),
        0,
        "truncates to -0, in range"
    );
    assert_eq!(eval_i32("f64.const 4294967295.9 i32.trunc_f64_u"), -1);
    assert_eq!(
        eval_i32("f64.const -2147483648.9 i32.trunc_f64_s"),
        i32::MIN
    );
    assert_eq!(eval_i32("f32.const -2147483648 i32.trunc_f32_s"), i32::MIN);
    for (code, kind) in [
        (
            "f32.const 2147483648 i32.trunc_f32_s",
            TrapKind::IntegerOverflow,
        ),
        (
            "f64.const -2147483649 i32.trunc_f64_s",
            TrapKind::IntegerOverflow,
        ),
        (
            "f64.const 4294967296 i32.trunc_f64_u",
            TrapKind::IntegerOverflow,
        ),
        ("f32.const -1 i32.trunc_f32_u", TrapKind::IntegerOverflow),
        ("f32.const nan i32.trunc_f32_s", TrapKind::InvalidConversion),
        ("f32.const inf i32.trunc_f32_u", TrapKind::IntegerOverflow),
    ] {
        assert_eq!(trap(ValType::I32, code), kind, "{code}");
    }
    assert_eq!(
        eval_i64("f64.const -9223372036854775808 i64.trunc_f64_s"),
        i64::MIN
    );
    assert_eq!(
        trap(
            ValType::I64,
            "f64.const 9223372036854775808 i64.trunc_f64_s"
        ),
        TrapKind::IntegerOverflow
    );
    assert_eq!(
        eval_i64("f64.const 18446744073709549568 i64.trunc_f64_u"),
        -2048
    );
    assert_eq!(
        trap(
            ValType::I64,
            "f32.const 18446744073709551616 i64.trunc_f32_u"
        ),
        TrapKind::IntegerOverflow
    );
}

#[test]
fn saturating_truncation_clamps() {
    assert_eq!(eval_i32("f32.const nan i32.trunc_sat_f32_s"), 0);
    assert_eq!(eval_i32("f32.const 3e9 i32.trunc_sat_f32_s"), i32::MAX);
    assert_eq!(eval_i32("f32.const -3e9 i32.trunc_sat_f32_s"), i32::MIN);
    assert_eq!(eval_i32("f64.const -5 i32.trunc_sat_f64_u"), 0);
    assert_eq!(eval_i32("f64.const 5e10 i32.trunc_sat_f64_u"), -1);
    assert_eq!(eval_i64("f64.const inf i64.trunc_sat_f64_s"), i64::MAX);
    assert_eq!(eval_i64("f32.const -inf i64.trunc_sat_f32_u"), 0);
}

#[test]
fn integer_conversions_and_sign_extension() {
    assert_eq!(eval_i32("i64.const 0x100000002 i32.wrap_i64"), 2);
    assert_eq!(eval_i64("i32.const -1 i64.extend_i32_s"), -1);
    assert_eq!(eval_i64("i32.const -1 i64.extend_i32_u"), 0xFFFF_FFFF);
    assert_eq!(eval_i32("i32.const 0x80 i32.extend8_s"), -128);
    assert_eq!(eval_i32("i32.const 0x7f i32.extend8_s"), 127);
    assert_eq!(eval_i32("i32.const 0x18000 i32.extend16_s"), -32768);
    assert_eq!(
        eval_i64("i64.const 0x80000000 i64.extend32_s"),
        -0x8000_0000
    );
    assert_eq!(eval_i64("i64.const 0xff i64.extend8_s"), -1);
    assert_eq!(eval_i64("i64.const 0x7fff i64.extend16_s"), 0x7FFF);
}

#[test]
fn float_conversions_round_to_nearest_even() {
    // 2^24 + 1 is not an f32: it rounds to even, down.
    assert_eq!(
        f32::from_bits(eval_f32("i32.const 16777217 f32.convert_i32_s")),
        16_777_216.0
    );
    assert_eq!(
        f32::from_bits(eval_f32("i32.const -1 f32.convert_i32_u")),
        4_294_967_296.0
    );
    assert_eq!(
        f64::from_bits(eval_f64("i64.const -1 f64.convert_i64_u")),
        18_446_744_073_709_551_616.0
    );
    assert_eq!(
        f64::from_bits(eval_f64("i64.const -9007199254740993 f64.convert_i64_s")),
        -9_007_199_254_740_992.0
    );
    assert_eq!(
        f32::from_bits(eval_f32("f64.const 0.1 f32.demote_f64")),
        0.1_f32
    );
    assert_eq!(
        eval_f32("f64.const 1e300 f32.demote_f64"),
        f32::INFINITY.to_bits()
    );
    assert_eq!(
        f64::from_bits(eval_f64("f32.const 0.5 f64.promote_f32")),
        0.5
    );
}

#[test]
fn reinterpretation_keeps_every_bit() {
    assert_eq!(
        eval_i32("f32.const bits:0x7fa00001 i32.reinterpret_f32"),
        0x7FA0_0001
    );
    assert_eq!(
        eval_f32("i32.const 0xff800001 f32.reinterpret_i32"),
        0xFF80_0001
    );
    assert_eq!(eval_i64("f64.const -0 i64.reinterpret_f64"), i64::MIN);
    assert_eq!(eval_f64("i64.const 1 f64.reinterpret_i64"), 1);
}
