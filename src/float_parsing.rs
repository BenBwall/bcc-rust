//! Floating-constant conversion checks that the host conversion routines
//! consume the complete spelling and reports overflow or underflow. Native
//! `long double` values keep their initialized object representation, including
//! zeroed padding, for later arithmetic and comparison.
//!
//! For `1.5L`, supplied with a NUL terminator, host conversion reads 1.5.
//! The suffix check accepts `L`, and the finite nonzero result needs no range
//! diagnostic.
//!
//! Read [`string_to_long_double`], [`string_to_double`], and
//! [`range::range_error`], then [`LongDouble`].
//!
//! Files by role:
//! - Other floating carriers: `conversion.rs`.
//! - Native long-double representation: `representation.rs`.
//! - Classification and conversion errors: `range.rs`.
//! - Generated native bindings: `ffi.rs`.
//! - Conversion fixtures: `tests.rs`.
//!
//! C99: phase 7, §5.1.1.2 paragraph 1, p. 10; PDF p. 22.
//! Floating constants: §6.4.4.2 paragraphs 1-7, pp. 57-58;
//! PDF pp. 69-70. Host conversion interfaces: §7.20.1.3 paragraphs 1-10,
//! pp. 308-310; PDF pp. 320-322.

// Conversion
mod conversion;
mod range;

// Native representations
mod ffi;
mod representation;

use std::ffi::c_char;

pub(crate) use conversion::{
    string_to_binary64_long_double,
    string_to_double,
    string_to_float,
};
pub(crate) use range::{
    FloatRangeError,
    ParseFloatError,
};
use range::{
    consumed_whole_spelling,
    range_error,
};
pub(crate) use representation::LongDouble;

use crate::translation_phases::preprocessing::FloatTokenType;

/// Converts a NUL-terminated `long double` constant spelling (with its `L`
/// suffix) to the host `long double`.
/// C99: type suffix §6.4.4.2p4, p. 58; PDF p. 70; `strtold` §7.20.1.3p1-4,
/// pp. 308-309; PDF pp. 320-321.
pub(crate) fn string_to_long_double(s: &str) -> Result<LongDouble, ParseFloatError> {
    assert!(
        s.ends_with('\0'),
        "string_to_long_double: string must end with null byte. Was: {s:?}"
    );
    let mut endptr = std::ptr::null_mut();
    // SAFETY: The string is NUL-terminated and the end pointer is writable.
    let long_double =
        unsafe { ffi::string_to_long_double(s.as_ptr().cast::<c_char>(), &raw mut endptr) };
    if !consumed_whole_spelling(s, endptr, 1) {
        return Err(ParseFloatError::Invalid(FloatTokenType::LongDouble(
            LongDouble::ZERO,
        )));
    }
    let ret = LongDouble {
        // SAFETY: The C function initializes every byte of the union.
        value: unsafe { long_double.bytes },
    };
    match range_error(ret.classify(), s) {
        | Some(error) => Err(ParseFloatError::OutOfRange(
            FloatTokenType::LongDouble(ret),
            error,
        )),
        | None => Ok(ret),
    }
}

// Tests
#[cfg(test)]
#[expect(
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod tests;
