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
