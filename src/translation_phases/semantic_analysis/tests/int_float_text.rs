//! The value an inexact integer-to-floating conversion gives, as Clang's
//! `-Wimplicit-const-int-float-conversion` prints it.

use super::super::value_warnings::int_float::{
    Format,
    TEXT_CAPACITY,
    rounded_text,
};

const FLOAT: Format = Format {
    precision:    24,
    max_exponent: 128,
};
const DOUBLE: Format = Format {
    precision:    53,
    max_exponent: 1024,
};
const X87: Format = Format {
    precision:    64,
    max_exponent: 16384,
};
const HALF: Format = Format {
    precision:    11,
    max_exponent: 16,
};

fn says(negative: bool, magnitude: u128, format: Format, expected: Option<&str>) {
    let mut buffer = [0; TEXT_CAPACITY];
    assert_eq!(
        rounded_text(negative, magnitude, format, &mut buffer),
        expected,
        "{magnitude}"
    );
}

/// Each expectation is Clang 23's text for the same conversion.
#[test]
fn inexact_conversions_print_as_clang_prints_them() {
    let cases: [(bool, u128, Format, &str); 11] = [
        (false, 16_777_217, FLOAT, "16777216"),
        (true, 16_777_217, FLOAT, "-16777216"),
        (false, 2_147_483_647, FLOAT, "2147483648"),
        (false, 1_234_567_891, FLOAT, "1234567936"),
        (false, 100_000_000_001, FLOAT, "99999997952"),
        (false, u128::from(u64::MAX), FLOAT, "18446744073709551616"),
        (false, 9_007_199_254_740_993, DOUBLE, "9007199254740992"),
        (false, 100_000_000_000_000_001, DOUBLE, "1.0E+17"),
        (false, 123_456_789_012_345_678, DOUBLE, "123456789012345680"),
        (
            false,
            (1 << 127) + 1,
            FLOAT,
            "1.70141183460469231731687E+38",
        ),
        (
            false,
            (1 << 100) | 1,
            X87,
            "1267650600228229401496703205376",
        ),
    ];
    for (negative, magnitude, format, expected) in cases {
        says(negative, magnitude, format, Some(expected));
    }
}

#[test]
fn exact_conversions_and_overflow() {
    says(false, 16_777_216, FLOAT, None);
    says(false, 1 << 127, FLOAT, None);
    says(false, 0, FLOAT, None);
    // A tie rounds to the even significand.
    says(false, 16_777_219, FLOAT, Some("16777220"));
    says(false, u128::MAX, FLOAT, Some("+Inf"));
    says(true, u128::MAX, FLOAT, Some("-Inf"));
    says(false, 65_521, HALF, Some("+Inf"));
    says(false, 2_049, HALF, Some("2048"));
}
