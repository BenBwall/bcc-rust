//! `strtof`, `strtod` and `strtold` over the subject sequences a floating
//! constant's spelling can start with: a decimal or hexadecimal significand
//! with an optional exponent. Each conversion is exact and rounds once, as
//! glibc's are: binary32 and binary64 decimal spellings go through Rust's
//! correctly rounded parser, hexadecimal spellings through a 124-bit
//! accumulator with a sticky bit, and double-extended decimal spellings
//! through exact integer arithmetic.
//!
//! C99: `strtod`, `strtof` and `strtold`, §7.20.1.3 paragraphs 3-5 and 8-10,
//! pp. 308-310; PDF pp. 320-322. A spelling that starts with a sign, white
//! space, `INF` or `NAN` never reaches the bridge.

use super::value::{
    F32,
    F64,
    Format,
    Soft,
    round,
};
use crate::{
    binary128::{
        Natural,
        exponent,
    },
    util::bump::Bump,
};

/// Converts the longest subject sequence at the start of `text`, returning
/// the bytes it spans (zero when there is none, with a zero value) and the
/// value rounded to `format`.
pub(super) fn convert(text: &[u8], format: Format) -> (usize, Soft) {
    let Some(subject) = Subject::scan(text) else {
        return (0, Soft::zero(false));
    };
    let value = if subject.hexadecimal {
        hexadecimal(&subject, format)
    } else if format == F32 || format == F64 {
        decimal_by_std(text, &subject, format)
    } else {
        decimal_exactly(&subject, format)
    };
    (subject.end, value)
}

/// The parts of a subject sequence.
struct Subject<'a> {
    hexadecimal: bool,
    /// The significand's digits and at most one period.
    digits:      &'a [u8],
    /// The exponent's value, saturated; zero without one.
    power:       i64,
    /// Where the subject sequence ends.
    end:         usize,
}

impl<'a> Subject<'a> {
    fn scan(text: &'a [u8]) -> Option<Self> {
        let hex_prefix = text.len() > 2 && text[0] == b'0' && matches!(text[1], b'x' | b'X');
        if hex_prefix && let Some(digits) = significand(&text[2..], 16) {
            let (power, end) = exponent_part(text, 2 + digits, *b"pP");
            return Some(Self {
                hexadecimal: true,
                digits: &text[2..2 + digits],
                power,
                end,
            });
        }
        // Without hexadecimal digits, `0x` is the decimal `0` and a stray
        // `x`.
        let digits = significand(text, 10)?;
        let (power, end) = exponent_part(text, digits, *b"eE");
        Some(Self {
            hexadecimal: false,
            digits: &text[..digits],
            power,
            end,
        })
    }
}

/// The length of a nonempty digit sequence in `radix` with at most one
/// period at the start of `text`.
fn significand(text: &[u8], radix: u32) -> Option<usize> {
    let mut period = false;
    let mut digits = 0;
    let mut end = 0;
    for &byte in text {
        if byte == b'.' && !period {
            period = true;
        } else if char::from(byte).is_digit(radix) {
            digits += 1;
        } else {
            break;
        }
        end += 1;
    }
    (digits > 0).then_some(end)
}

/// An exponent part at `start`, if a complete one is there: its value and
/// the subject's new end. An incomplete one is not part of the subject.
fn exponent_part(text: &[u8], start: usize, markers: [u8; 2]) -> (i64, usize) {
    if text.get(start).is_none_or(|byte| !markers.contains(byte)) {
        return (0, start);
    }
    let mut end = start + 1;
    if matches!(text.get(end), Some(b'+' | b'-')) {
        end += 1;
    }
    let digits_start = end;
    while text.get(end).is_some_and(u8::is_ascii_digit) {
        end += 1;
    }
    if end == digits_start {
        return (0, start);
    }
    let spelling = std::str::from_utf8(&text[start + 1..end]).unwrap_or("0");
    (exponent(spelling).unwrap_or(0), end)
}

fn hexadecimal(subject: &Subject<'_>, format: Format) -> Soft {
    let mut significand = 0_u128;
    let mut sticky = false;
    // Hexadecimal digits before the period, and kept digits.
    let mut integer_digits = 0_i64;
    let mut kept = 0_i64;
    let mut period = false;
    for &byte in subject.digits {
        if byte == b'.' {
            period = true;
            continue;
        }
        let digit = char::from(byte).to_digit(16).unwrap_or(0);
        integer_digits += i64::from(!period);
        if significand >> 120 == 0 {
            significand = significand * 16 + u128::from(digit);
            kept += 1;
        } else {
            sticky |= digit != 0;
        }
    }
    let exponent = subject
        .power
        .saturating_add(4 * (integer_digits - kept))
        .clamp(-100_000, 100_000) as i32;
    round(false, significand, exponent, sticky, format)
}

/// Binary32 and binary64 decimal spellings, which Rust's parser rounds
/// correctly from the same grammar.
fn decimal_by_std(text: &[u8], subject: &Subject<'_>, format: Format) -> Soft {
    let spelling = std::str::from_utf8(&text[..subject.end]).unwrap_or("0");
    if format == F32 {
        Soft::from_ieee(
            u64::from(spelling.parse::<f32>().unwrap_or(0.0).to_bits()),
            F32,
        )
    } else {
        Soft::from_ieee(spelling.parse::<f64>().unwrap_or(0.0).to_bits(), F64)
    }
}

/// A decimal spelling as the exact ratio of two integers times a power of
/// two, divided out to two bits beyond the result's precision with the
/// remainder as the sticky bit.
fn decimal_exactly(subject: &Subject<'_>, format: Format) -> Soft {
    let mut period = false;
    let mut fractional = 0_i64;
    let mut leading = true;
    let arena = Bump::new();
    let mut numerator = Natural::new(&arena, 0);
    let mut digits = 0_i64;
    for &byte in subject.digits {
        if byte == b'.' {
            period = true;
            continue;
        }
        let digit = u32::from(byte - b'0');
        fractional += i64::from(period);
        leading &= digit == 0;
        if !leading {
            numerator.mul_add(10, digit);
            digits += 1;
        }
    }
    if digits == 0 {
        return Soft::zero(false);
    }
    let scale = subject.power.saturating_sub(fractional);
    // About the decimal logarithm; the extended range ends near 10^±4951.
    let estimate = digits.saturating_add(scale);
    if estimate > 5000 {
        return Soft::infinity(false);
    }
    if estimate < -5000 {
        return Soft::zero(false);
    }
    // N / D × 2^scale with N and D carrying the powers of five.
    let mut denominator = Natural::new(&arena, 1);
    {
        let scaled = if scale >= 0 {
            &mut numerator
        } else {
            &mut denominator
        };
        for _ in 0..scale.unsigned_abs() {
            scaled.mul_add(5, 0);
        }
    }
    // floor(log2(N / D)), exactly.
    let difference = numerator.bits() - denominator.bits();
    let mut comparison = if difference >= 0 {
        denominator.copy(&arena)
    } else {
        numerator.copy(&arena)
    };
    comparison.shift(difference.abs());
    let below = if difference >= 0 {
        numerator.compare(&comparison).is_lt()
    } else {
        comparison.compare(&denominator).is_lt()
    };
    let magnitude = difference + scale - i64::from(below);
    if magnitude > i64::from(format.max_exponent) {
        return Soft::infinity(false);
    }
    // Quotient bits down to two below the result's last significand bit.
    let quantum =
        magnitude.max(i64::from(format.min_exponent)) - i64::from(format.precision - 1) - 2;
    if magnitude < quantum {
        return Soft::zero(false);
    }
    let shift = scale - quantum;
    if shift >= 0 {
        numerator.shift(shift);
    } else {
        denominator.shift(-shift);
    }
    let quotient_bits = numerator.bits() - denominator.bits();
    let mut divisor = denominator.copy(&arena);
    divisor.shift(quotient_bits.max(0));
    let mut quotient = 0_u128;
    for bit in (0..=quotient_bits).rev() {
        if !numerator.compare(&divisor).is_lt() {
            numerator.subtract(&divisor);
            quotient |= 1_u128 << bit;
        }
        divisor.halve();
    }
    let sticky = numerator.bits() != 0;
    round(false, quotient, quantum as i32, sticky, format)
}
