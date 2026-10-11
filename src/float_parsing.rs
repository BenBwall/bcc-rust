//! Translation-time floating-constant conversion in phase 7.
//!
//! C99: §5.1.1.2p7, p. 10; PDF p. 22; `floating-constant` §6.4.4.2p1-7,
//! pp. 57-58; PDF pp. 69-70. Conversion uses the host `strtof`, `strtod`, and
//! `strtold` interfaces (§7.20.1.3p1-10, pp. 308-310; PDF pp. 320-322).

mod ffi;

mod conversion;

mod range;

mod representation;

use std::{
    ffi::c_char,
    fmt::{
        Debug,
        Display,
        Formatter,
    },
};

pub(crate) use conversion::{
    string_to_binary64_long_double,
    string_to_double,
    string_to_float,
};
use range::{
    FloatClass,
    consumed_whole_spelling,
    range_error,
};
pub(crate) use range::{
    FloatRangeError,
    ParseFloatError,
};
#[cfg(test)]
use representation::LONG_DOUBLE_BYTES;
pub(crate) use representation::LongDouble;

use crate::{
    translation_phases::preprocessing::FloatTokenType,
    util::packed::Packed,
};

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

#[cfg(test)]
#[expect(
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod tests;
