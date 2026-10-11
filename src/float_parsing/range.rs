use super::{
    Debug,
    FloatTokenType,
    c_char,
};

/// Classifies overflow and underflow after host conversion.
/// C99: §7.20.1.3p10, p. 310; PDF p. 322.
pub(super) fn range_error(class: FloatClass, spelling: &str) -> Option<FloatRangeError> {
    match class {
        | FloatClass::Infinite => Some(FloatRangeError::Overflow),
        | FloatClass::Zero if significand_is_nonzero(spelling) => Some(FloatRangeError::Underflow),
        | FloatClass::Zero | FloatClass::Nonzero | FloatClass::NotANumber => None,
    }
}

/// Returns whether the significand of a floating constant's spelling has a
/// nonzero digit, i.e. whether its exact mathematical value is nonzero.
///
/// Only digits of the constant's base count, so a suffix such as the `f` of
/// an exponent-free `0.0f` is not mistaken for a hexadecimal digit.
/// C99: `floating-constant` significand §6.4.4.2p1-3, pp. 57-58; PDF pp. 69-70.
pub(super) fn significand_is_nonzero(spelling: &str) -> bool {
    let (digits, exponent_markers, is_digit): (&str, &[char], fn(&char) -> bool) = match spelling
        .strip_prefix("0x")
        .or_else(|| spelling.strip_prefix("0X"))
    {
        | Some(hex) => (hex, &['p', 'P'], char::is_ascii_hexdigit),
        | None => (spelling, &['e', 'E'], char::is_ascii_digit),
    };
    let significand = digits.split(exponent_markers).next().unwrap_or_default();
    significand.chars().any(|c| is_digit(&c) && c != '0')
}

pub(super) fn class_of(value: f64) -> FloatClass {
    if value.is_nan() {
        FloatClass::NotANumber
    } else if value.is_infinite() {
        FloatClass::Infinite
    } else if value == 0.0 {
        FloatClass::Zero
    } else {
        FloatClass::Nonzero
    }
}

/// Checks that the C conversion stopped exactly `suffix_bytes` before the
/// terminating NUL, i.e. that the whole spelling was consumed.
/// C99: `strtod` end pointer §7.20.1.3p4, pp. 308-309; PDF pp. 320-321.
pub(super) fn consumed_whole_spelling(s: &str, endptr: *const c_char, suffix_bytes: usize) -> bool {
    let expected = s.len() - 1 - suffix_bytes;
    endptr.addr() == s.as_ptr().addr() + expected
}

/// A phase-7 `floating-constant` rejected by conversion or range checks.
/// C99: §6.4.4.2p1-5, pp. 57-58; PDF pp. 69-70; §7.20.1.3p2-10, pp. 308-310;
/// PDF pp. 320-322.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum ParseFloatError {
    /// The spelling fails the `floating-constant` grammar.
    /// C99: §6.4.4.2p1, p. 57; PDF p. 69.
    Invalid(FloatTokenType),
    /// The host conversion reports an unrepresentable magnitude.
    /// C99: §7.20.1.3p10, p. 310; PDF p. 322.
    OutOfRange(FloatTokenType, FloatRangeError),
}

/// Why a floating constant's value could not be represented in its type.
///
/// Range errors are derived from the converted value and the spelling
/// instead of `errno`, whose underflow behavior is implementation-defined
/// (C99 §7.20.1.3p10, p. 310; PDF p. 322) and whose storage
/// differs between C runtimes. Conversion range follows
/// §6.4.4.2p3, p. 58; PDF p. 70.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum FloatRangeError {
    /// The magnitude exceeds the largest finite value; it becomes infinity.
    /// C99: §7.20.1.3p10, p. 310; PDF p. 322.
    Overflow,
    /// A nonzero constant is smaller than the least subnormal; it becomes
    /// zero.
    /// C99: §7.20.1.3p10, p. 310; PDF p. 322.
    Underflow,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(super) enum FloatClass {
    Nonzero,
    Zero,
    Infinite,
    NotANumber,
}
