//! Converts float and double spellings and adapts binary64 results to the
//! long-double carrier used by Microsoft targets.

use std::ffi::c_char;

use super::{
    LongDouble,
    ParseFloatError,
    ffi,
    range::{
        class_of,
        consumed_whole_spelling,
        range_error,
    },
};
use crate::{
    translation_phases::preprocessing::FloatTokenType,
    util::packed::Packed,
};

/// Converts a NUL-terminated unsuffixed `double` constant spelling.
/// C99: unsuffixed type §6.4.4.2p4, p. 58; PDF p. 70; `strtod` §7.20.1.3p1-4,
/// pp. 308-309; PDF pp. 320-321.
pub(crate) fn string_to_double(s: &str) -> Result<f64, ParseFloatError> {
    parse_double(s, 0)
}

fn parse_double(s: &str, suffix_bytes: usize) -> Result<f64, ParseFloatError> {
    assert!(
        s.ends_with('\0'),
        "string_to_double: string must end with null byte. Was: {s:?}"
    );
    let mut endptr = std::ptr::null_mut();
    // SAFETY: The string is NUL-terminated and the end pointer is writable.
    let double = unsafe { ffi::string_to_double(s.as_ptr().cast::<c_char>(), &raw mut endptr) };
    if !consumed_whole_spelling(s, endptr, suffix_bytes) {
        return Err(ParseFloatError::Invalid(FloatTokenType::Double(
            Packed::new(0.0),
        )));
    }
    match range_error(class_of(double), s) {
        | Some(error) => Err(ParseFloatError::OutOfRange(
            FloatTokenType::Double(Packed::new(double)),
            error,
        )),
        | None => Ok(double),
    }
}

/// Converts a NUL-terminated `float` constant spelling (with its `f`
/// suffix).
/// C99: type suffix §6.4.4.2p4, p. 58; PDF p. 70; `strtof` §7.20.1.3p1-4,
/// pp. 308-309; PDF pp. 320-321.
pub(crate) fn string_to_float(s: &str) -> Result<f32, ParseFloatError> {
    assert!(
        s.ends_with('\0'),
        "string_to_float: string must end with null byte. Was: {s:?}"
    );
    let mut endptr = std::ptr::null_mut();
    // SAFETY: The string is NUL-terminated and the end pointer is writable.
    let float = unsafe { ffi::string_to_float(s.as_ptr().cast::<c_char>(), &raw mut endptr) };
    if !consumed_whole_spelling(s, endptr, 1) {
        return Err(ParseFloatError::Invalid(FloatTokenType::Float(0.0)));
    }
    match range_error(class_of(f64::from(float)), s) {
        | Some(error) => Err(ParseFloatError::OutOfRange(
            FloatTokenType::Float(float),
            error,
        )),
        | None => Ok(float),
    }
}

/// MSVC's `long double` is binary64 even when the host uses x87.
/// C99: implementation-defined representation §6.2.5p10, p. 34; PDF p. 46;
/// suffix §6.4.4.2p4, p. 58; PDF p. 70.
pub(crate) fn string_to_binary64_long_double(s: &str) -> Result<LongDouble, ParseFloatError> {
    parse_double(s, 1)
        .map(LongDouble::from_double)
        .map_err(|error| match error {
            | ParseFloatError::Invalid(_) =>
                ParseFloatError::Invalid(FloatTokenType::LongDouble(LongDouble::ZERO)),
            | ParseFloatError::OutOfRange(FloatTokenType::Double(value), range) =>
                ParseFloatError::OutOfRange(
                    FloatTokenType::LongDouble(LongDouble::from_double(value.get())),
                    range,
                ),
            | _ => unreachable!("double conversion returns a double error"),
        })
}

/// Converts the longest prefix of the NUL-terminated `text` the host C
/// library's `strtod` reads: the value and how many bytes it read (zero when
/// it read no number).
/// C99: §7.20.1.3 paragraphs 2-10, pp. 308-310; PDF pp. 320-322.
#[cfg(feature = "native")]
pub(crate) fn host_strtod(text: &[u8]) -> (f64, usize) {
    assert_eq!(text.last(), Some(&0), "host_strtod: text must end with a null byte");
    let mut endptr = std::ptr::null_mut();
    // SAFETY: The text is NUL-terminated and the end pointer is writable.
    let value = unsafe { ffi::string_to_double(text.as_ptr().cast::<c_char>(), &raw mut endptr) };
    (value, read_length(text, endptr))
}

/// As [`host_strtod`] for `strtof`.
#[cfg(feature = "native")]
pub(crate) fn host_strtof(text: &[u8]) -> (f32, usize) {
    assert_eq!(text.last(), Some(&0), "host_strtof: text must end with a null byte");
    let mut endptr = std::ptr::null_mut();
    // SAFETY: The text is NUL-terminated and the end pointer is writable.
    let value = unsafe { ffi::string_to_float(text.as_ptr().cast::<c_char>(), &raw mut endptr) };
    (value, read_length(text, endptr))
}

/// As [`host_strtod`] for `strtold`, in the host's `long double` carrier.
#[cfg(feature = "native")]
pub(crate) fn host_strtold(text: &[u8]) -> (LongDouble, usize) {
    assert_eq!(text.last(), Some(&0), "host_strtold: text must end with a null byte");
    let mut endptr = std::ptr::null_mut();
    // SAFETY: The text is NUL-terminated and the end pointer is writable.
    let value =
        unsafe { ffi::string_to_long_double(text.as_ptr().cast::<c_char>(), &raw mut endptr) };
    let value = LongDouble {
        // SAFETY: The C function initializes every byte of the union.
        value: unsafe { value.bytes },
    };
    (value, read_length(text, endptr))
}

/// How far into `text` the C conversion's end pointer points.
#[cfg(feature = "native")]
fn read_length(text: &[u8], endptr: *mut c_char) -> usize {
    endptr.addr().saturating_sub(text.as_ptr().addr())
}
