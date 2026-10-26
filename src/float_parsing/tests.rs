use std::hint::black_box;

use super::{
    FloatRangeError,
    ParseFloatError,
    ffi,
    representation::LONG_DOUBLE_BYTES,
    string_to_double,
    string_to_float,
    string_to_long_double,
};

/// Leaves recognizable garbage in stack memory that a following FFI call
/// may reuse for its return slot.
#[inline(never)]
fn dirty_stack() {
    let garbage = black_box([0xA5_u8; 4096]);
    let _ = black_box(&garbage);
}

#[test]
fn long_double_padding_is_deterministic() {
    dirty_stack();
    let first = string_to_long_double("1.0L\0").unwrap();
    dirty_stack();
    let second = string_to_long_double("1.0L\0").unwrap();

    assert_eq!(first.value, second.value);
    let value_bytes = ffi::LONG_DOUBLE_VALUE_BYTES as usize;
    assert!(value_bytes <= LONG_DOUBLE_BYTES);
    assert!(
        first.value[value_bytes..].iter().all(|&byte| byte == 0),
        "long double padding bytes must be zero: {:?}",
        first.value
    );
}

#[test]
fn long_double_prints_as_exact_hexadecimal() {
    for (spelling, expected) in [
        ("1.5L\0", "0x1.8p+0"),
        ("0.0L\0", "0x0p+0"),
        ("0x1p-3L\0", "0x1p-3"),
        ("2.5L\0", "0x1.4p+1"),
    ] {
        let value = string_to_long_double(spelling).unwrap();
        assert_eq!(value.to_string(), expected, "{spelling:?}");
        assert_eq!(format!("{value:?}"), format!("LongDouble({expected})"));
    }
}

#[test]
fn range_errors_do_not_depend_on_errno() {
    // Stale errno state from earlier library calls must not matter.
    drop(std::fs::metadata("this path does not exist"));
    assert_eq!(string_to_double("1.5\0"), Ok(1.5));
    assert_eq!(string_to_float("1.5f\0"), Ok(1.5));
    assert_eq!(
        string_to_long_double("1.5L\0").unwrap().to_string(),
        "0x1.8p+0"
    );
    assert_eq!(string_to_double("0.0\0"), Ok(0.0));
    assert_eq!(string_to_double("0x0p-99999\0"), Ok(0.0));

    for result in [
        string_to_double("1e999\0").map(|_| ()),
        string_to_float("1e999f\0").map(|_| ()),
        string_to_long_double("1e99999L\0").map(|_| ()),
    ] {
        assert!(
            matches!(
                result,
                Err(ParseFloatError::OutOfRange(_, FloatRangeError::Overflow))
            ),
            "{result:?}"
        );
    }
    for result in [
        string_to_double("1e-999\0").map(|_| ()),
        string_to_float("1e-999f\0").map(|_| ()),
        string_to_long_double("1e-99999L\0").map(|_| ()),
    ] {
        assert!(
            matches!(
                result,
                Err(ParseFloatError::OutOfRange(_, FloatRangeError::Underflow))
            ),
            "{result:?}"
        );
    }
}

/// GCC's `__builtin_nans` constants, bit for bit: the quiet bit clear, the
/// payload in the low fraction bits, and the bit below the quiet bit when
/// the payload is zero. A narrowing conversion that loses the payload
/// gives a quiet NaN instead of an infinity.
#[test]
fn signaling_nans_keep_their_format_and_payload() {
    use crate::{
        float_parsing::{
            LongDouble,
            bits::constant_bits,
        },
        ir::Type,
    };
    let bits = |payload, precision, ty| {
        constant_bits(&LongDouble::signaling_nan(payload, precision), ty).unwrap()
    };
    assert_eq!(bits(0, 1, Type::F32), 0x7FA0_0000);
    assert_eq!(bits(7, 1, Type::F32), 0x7F80_0007);
    assert_eq!(bits(0, 2, Type::F64), 0x7FF4_0000_0000_0000);
    assert_eq!(bits(5, 2, Type::F64), 0x7FF0_0000_0000_0005);
    assert_eq!(bits(5, 2, Type::F32), 0x7FC0_0000);
    if ffi::LONG_DOUBLE_VALUE_BYTES != 8 {
        assert_eq!(bits(3, 3, Type::F80), 0x7FFF_8000_0000_0000_0003);
        assert_eq!(bits(0, 3, Type::F80), 0x7FFF_A000_0000_0000_0000);
    }
    // Quiet NaNs keep their quiet bit and payload.
    assert_eq!(
        constant_bits(&LongDouble::quiet_nan(5, 2), Type::F64).unwrap(),
        0x7FF8_0000_0000_0005
    );
}

/// A binary64 carrier, as on hosts whose `long double` is 8 bytes (MSVC),
/// decodes a signaling NaN without setting its quiet bit, so it converts to
/// the same bits as the x87 carrier gives.
#[test]
fn binary64_carriers_keep_signaling_nans() {
    use crate::{
        float_parsing::{
            LongDouble,
            bits::constant_parts,
        },
        ir::Type,
    };
    let convert =
        |bits: u64, ty| constant_parts(LongDouble::decode_x87(&bits.to_le_bytes()).unwrap(), ty);
    // `__builtin_nansf32("7")` as the binary64 carrier holds it.
    assert_eq!(
        convert(0x7FF0_0000_0000_0000 | 7 << 29, Type::F32),
        Some(0x7F80_0007)
    );
    assert_eq!(
        convert(0x7FF4_0000_0000_0000, Type::F64),
        Some(0x7FF4_0000_0000_0000)
    );
    assert_eq!(
        convert(0x7FF0_0000_0000_0005, Type::F64),
        Some(0x7FF0_0000_0000_0005)
    );
    // Quiet NaNs and infinities are unchanged.
    assert_eq!(
        convert(0x7FF8_0000_0000_0005, Type::F64),
        Some(0x7FF8_0000_0000_0005)
    );
    assert_eq!(
        convert(0xFFF0_0000_0000_0000, Type::F64),
        Some(0xFFF0_0000_0000_0000)
    );
    assert_eq!(convert(0x7FF8_0000_0000_0000, Type::F32), Some(0x7FC0_0000));
}
