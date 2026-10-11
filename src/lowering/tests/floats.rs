#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::unreadable_literal,
    reason = "The tests compare the conversions with Rust's own casts, which truncate and round."
)]

//! The conversions between host `long double` constants, integers and the
//! bits of IR floating constants, checked against Rust's own correctly
//! rounded `as` casts.

use super::super::floats::{
    constant_bits,
    from_integer,
    from_unsigned,
    to_integer,
};
use crate::{
    float_parsing::LongDouble,
    ir::Type,
};

/// A deterministic sequence of 64-bit patterns.
fn patterns() -> impl Iterator<Item = u64> {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    std::iter::repeat_with(move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        state ^ (state >> 29)
    })
    .take(20_000)
}

fn interesting_doubles() -> Vec<f64> {
    let mut values = vec![
        0.0,
        -0.0,
        1.0,
        -1.0,
        0.5,
        1.5,
        3.0,
        0.1,
        1.0 / 3.0,
        f64::MAX,
        f64::MIN,
        f64::MIN_POSITIVE,
        f64::MIN_POSITIVE / 2.0,
        5e-324,
        -5e-324,
        f64::EPSILON,
        f64::INFINITY,
        f64::NEG_INFINITY,
        16_777_217.0,
        4_503_599_627_370_497.5,
        1e300,
        1e-300,
        3.4028235e38,
        3.4028236e38,
        1e-45,
        7e-46,
        1.17549435e-38,
    ];
    values.extend(patterns().map(f64::from_bits));
    values
}

#[test]
fn doubles_survive_the_trip_through_x87() {
    for value in interesting_doubles() {
        let wide = LongDouble::from_double(value);
        let bits = constant_bits(&wide, Type::F64).expect("an f64 constant");
        if value.is_nan() {
            let converted = f64::from_bits(bits as u64);
            assert!(converted.is_nan(), "{value:e}");
        } else {
            assert_eq!(bits, u128::from(value.to_bits()), "{value:e}");
        }
    }
}

#[test]
fn doubles_round_to_float_to_nearest_even() {
    for value in interesting_doubles() {
        let wide = LongDouble::from_double(value);
        let bits = constant_bits(&wide, Type::F32).expect("an f32 constant");
        let expected = value as f32;
        if value.is_nan() {
            assert!(f32::from_bits(bits as u32).is_nan(), "{value:e}");
        } else {
            assert_eq!(bits, u128::from(expected.to_bits()), "{value:e}");
        }
    }
}

#[test]
fn extended_constants_keep_the_x87_bits() {
    let bits = |value: f64| constant_bits(&LongDouble::from_double(value), Type::F80).unwrap();
    assert_eq!(bits(1.0), 0x3FFF_8000_0000_0000_0000);
    assert_eq!(bits(-2.0), 0xC000_8000_0000_0000_0000);
    assert_eq!(bits(0.0), 0);
    assert_eq!(bits(-0.0), 1 << 79);
    assert_eq!(bits(f64::INFINITY), 0x7FFF_8000_0000_0000_0000);
    assert_eq!(bits(0.75), 0x3FFE_C000_0000_0000_0000);
}

#[test]
fn integers_convert_to_the_nearest_floating_value() {
    let mut values = vec![
        0_i128,
        1,
        -1,
        7,
        -7,
        16_777_216,
        16_777_217,
        16_777_219,
        i128::from(u32::MAX),
        i128::from(i64::MAX),
        i128::from(i64::MIN),
        i128::from(u64::MAX),
        (1 << 53) + 1,
        (1 << 53) + 3,
        (1 << 63) + (1 << 10),
        i128::MAX,
        i128::MIN,
    ];
    values.extend(patterns().map(|bits| i128::from(bits as i64) << (bits % 64)));
    for value in values {
        assert_eq!(
            from_integer(value, Type::F64),
            Some(u128::from((value as f64).to_bits())),
            "{value} as f64"
        );
        assert_eq!(
            from_integer(value, Type::F32),
            Some(u128::from((value as f32).to_bits())),
            "{value} as f32"
        );
    }
}

#[test]
fn integers_convert_to_extended_bits() {
    assert_eq!(from_integer(1, Type::F80), Some(0x3FFF_8000_0000_0000_0000));
    assert_eq!(
        from_integer(-3, Type::F80),
        Some(0xC000_C000_0000_0000_0000)
    );
    assert_eq!(from_integer(0, Type::F80), Some(0));
    // 2^64 + 1 needs 65 bits and rounds to 2^64.
    assert_eq!(
        from_integer((1 << 64) + 1, Type::F80),
        Some(0x403F_8000_0000_0000_0000)
    );
    // Rounding up to the next power of two carries into the exponent.
    assert_eq!(
        from_integer(i128::from(u64::MAX) + 1, Type::F80),
        Some(0x403F_8000_0000_0000_0000)
    );
    assert_eq!(
        from_integer(i128::MAX, Type::F80),
        Some(0x407E_8000_0000_0000_0000)
    );
    // Round-trips against the host conversion for small values.
    for value in [5_i128, -123_456, 1 << 40, -(1 << 62)] {
        assert_eq!(
            from_integer(value, Type::F80),
            constant_bits(&LongDouble::from_double(value as f64), Type::F80)
        );
    }
}

#[test]
fn other_types_have_no_floating_bits() {
    assert_eq!(from_integer(1, Type::I32), None);
    assert_eq!(constant_bits(&LongDouble::ZERO, Type::Ptr), None);
}

#[test]
fn floating_constants_truncate_toward_zero_as_integers() {
    for value in [
        0.0,
        0.9,
        -0.9,
        1.0,
        -1.0,
        2.75,
        -2.75,
        123456.789,
        1e18,
        -1e18,
        9.007199254740993e15,
    ] {
        assert_eq!(
            to_integer(&LongDouble::from_double(value)),
            Some(value as i128),
            "{value}"
        );
    }
    assert_eq!(to_integer(&LongDouble::from_double(f64::NAN)), None);
    assert_eq!(to_integer(&LongDouble::from_double(f64::INFINITY)), None);
    assert_eq!(to_integer(&LongDouble::from_double(1e300)), None);
}

#[test]
fn unsigned_128_magnitudes_convert_without_a_sign_bit() {
    assert_eq!(
        from_unsigned(1_u128 << 127, Type::F64),
        Some(0x47E0_0000_0000_0000)
    );
    assert_eq!(
        from_unsigned(1_u128 << 127, Type::F80),
        Some(0x407E_8000_0000_0000_0000)
    );
    assert_eq!(
        from_unsigned(u128::from(u64::MAX), Type::F80),
        Some(0x403E_FFFF_FFFF_FFFF_FFFF)
    );
}
