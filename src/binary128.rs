//! IEEE binary128 values: conversion from decimal or hexadecimal floating
//! spellings, and arithmetic computed in software, bit for bit as libgcc's
//! soft-float routines compute it on x86-64 in the default rounding mode.
//!
//! Conversion reads a spelling, builds an exact integer significand, and
//! rounds it into the binary128 carrier; arena-backed integer limbs hold
//! intermediate values. For `0x1p+0Q`, conversion reads significand 1 and
//! exponent 0. The exact ratio 1/1 becomes the binary128 encoding of 1
//! without rounding loss.
//!
//! Arithmetic unpacks each operand into a [`Class`], computes the exact
//! result with integers (or with a sticky bit for an inexact quotient or
//! root), and rounds it once to 113 bits. For `1 / 3`, division forms 115
//! quotient bits with a sticky bit for the remainder, and rounding keeps
//! 113, giving `0x3FFD_5555_5555_5555_5555_5555_5555_5555`. Semantic
//! analysis folds constants with it, and the interpreter computes with it,
//! so both agree with programs that call libgcc.
//!
//! Read [`parse`], [`Binary128`], and [`arithmetic::Natural`], then
//! [`Class`] in `encoding.rs`, `operations.rs` and `conversion.rs`.
//!
//! Files by role:
//! - Integer arithmetic, exponent parsing, and display: `arithmetic.rs`.
//! - Encoding fields and classes: `encoding.rs`.
//! - Operations: `operations.rs` (add, subtract, multiply, divide, `fmod`,
//!   square root, comparison) and `functions.rs` (the exact `<math.h>`
//!   functions: rounding to integers, scaling, decomposition).
//! - Rounding and format conversion: `conversion.rs`.
//! - Conversion and arithmetic fixtures: `tests.rs`.
//!
//! C99: floating constants, §6.4.4.2 paragraphs 1-7, pp. 57-58;
//! PDF pp. 69-70; §5.2.4.2.2 paragraphs 2-4, pp. 22-24; PDF pp. 34-36
//! (floating model); §6.3.1.4-§6.3.1.5, pp. 43-44; PDF pp. 55-56
//! (conversions). Binary128 is a GNU floating-type extension (GCC manual,
//! "Additional Floating Types").

// Exact arithmetic
mod arithmetic;

// Encoding and operations
mod encoding;
mod functions;
mod operations;

// Rounding and format conversion
mod conversion;

use arithmetic::error;
pub(crate) use arithmetic::{
    Natural,
    exponent,
};
pub(crate) use encoding::Class;

use crate::{
    float_parsing::{
        FloatRangeError,
        ParseFloatError,
    },
    util::{
        bump::Bump,
        packed::Packed,
    },
};

/// Parses a suffixed spelling, retaining all significand digits before one
/// IEEE round-to-nearest-even conversion. Nonzero rounded-to-zero and infinity
/// use the existing range diagnostics (C99 §6.4.4p2, p. 54; PDF p. 66).
pub(crate) fn parse(text: &str, arena: &Bump) -> Result<Binary128, ParseFloatError> {
    parse_reporting_underflow(text, arena).map(|(value, _)| value)
}

/// As [`parse`], also saying whether a finite nonzero result underflowed
/// as glibc's `strtod` family reports it (`round_and_return` in
/// `stdlib/strtod_l.c`): the exact value is tiny and the result inexact.
/// x86 detects tininess after rounding, so a value below the smallest
/// normal that rounds to it at the full 113-bit precision is not tiny.
/// C99: §7.20.1.3 paragraph 10, p. 310; PDF p. 322 (`ERANGE` on underflow is
/// implementation-defined).
pub(crate) fn parse_reporting_underflow(
    text: &str,
    arena: &Bump,
) -> Result<(Binary128, bool), ParseFloatError> {
    let text = text.trim_end_matches('\0');
    let text = text.strip_suffix(['q', 'Q']).ok_or_else(|| error(None))?;
    let hex = text.starts_with("0x") || text.starts_with("0X");
    let text = if hex { &text[2..] } else { text };
    let split = text.find(if hex {
        &['p', 'P'][..]
    } else {
        &['e', 'E'][..]
    });
    let (digits, power) = if let Some(index) = split {
        (
            &text[..index],
            exponent(&text[index + 1..]).ok_or_else(|| error(None))?,
        )
    } else if hex {
        return Err(error(None));
    } else {
        (text, 0)
    };
    let radix = if hex { 16 } else { 10 };
    let mut point = false;
    let mut fractional = 0_i64;
    let mut count = 0_i64;
    let mut leading = 0_i64;
    let mut trailing = 0_i64;
    let mut nonzero = false;
    for byte in digits.bytes() {
        if byte == b'.' && !point {
            point = true;
            continue;
        }
        let digit = char::from(byte)
            .to_digit(radix)
            .ok_or_else(|| error(None))?;
        count += 1;
        fractional += i64::from(point);
        if digit == 0 {
            trailing += 1;
            if !nonzero {
                leading += 1;
            }
        } else {
            nonzero = true;
            trailing = 0;
        }
    }
    if count == 0 || (!hex && !point && split.is_none()) {
        return Err(error(None));
    }
    if !nonzero {
        return Ok((Binary128(Packed::new(0)), false));
    }
    let significant = count - leading - trailing;
    let factor = if hex { 4 } else { 1 };
    let scale = power
        .saturating_sub(fractional.saturating_mul(factor))
        .saturating_add(trailing.saturating_mul(factor));
    let estimate = significant.saturating_mul(factor).saturating_add(scale);
    let ceiling = if hex { 16520 } else { 5000 };
    if estimate > ceiling {
        return Err(error(Some(FloatRangeError::Overflow)));
    }
    if estimate < -ceiling {
        return Err(error(Some(FloatRangeError::Underflow)));
    }
    let mut numerator = Natural::new(arena, 0);
    let mut denominator = Natural::new(arena, 1);
    for byte in digits
        .bytes()
        .filter(|byte| *byte != b'.')
        .skip(usize::try_from(leading).map_err(|_| error(None))?)
        .take(usize::try_from(significant).map_err(|_| error(None))?)
    {
        numerator.mul_add(radix, char::from(byte).to_digit(radix).unwrap_or(0));
    }
    if !hex {
        let scaled = if scale >= 0 {
            &mut numerator
        } else {
            &mut denominator
        };
        for _ in 0..scale.unsigned_abs() {
            scaled.mul_add(5, 0);
        }
    }
    // N / D * 2^scale. Determine the binary exponent exactly, then divide
    // at 113 bits (or the fixed subnormal quantum, 2^-16494).
    let difference = numerator.bits() - denominator.bits();
    let mut comparison = if difference >= 0 {
        denominator.copy(arena)
    } else {
        numerator.copy(arena)
    };
    comparison.shift(difference.abs());
    let below = if difference >= 0 {
        numerator.compare(&comparison).is_lt()
    } else {
        comparison.compare(&denominator).is_lt()
    };
    let mut binary_exponent = difference + scale - i64::from(below);
    if binary_exponent > 16383 {
        return Err(error(Some(FloatRangeError::Overflow)));
    }
    if binary_exponent < -16495 {
        return Err(error(Some(FloatRangeError::Underflow)));
    }
    let shift = scale - binary_exponent.max(-16382) + 112;
    if shift >= 0 {
        numerator.shift(shift);
    } else {
        denominator.shift(-shift);
    }
    let quotient_bits = numerator.bits() - denominator.bits();
    let mut divisor = denominator.copy(arena);
    divisor.shift(quotient_bits.max(0));
    let mut quotient = 0_u128;
    for bit in (0..=quotient_bits).rev() {
        if !numerator.compare(&divisor).is_lt() {
            numerator.subtract(&divisor);
            quotient |= 1_u128 << bit;
        }
        divisor.halve();
    }
    // The remainder decides exactness. Just below the smallest normal, a
    // floor of 112 ones with at least three quarters left over rounds to it
    // at 113 bits, so it is not tiny.
    let inexact = numerator.bits() != 0;
    let rounds_to_normal = binary_exponent == -16383 && quotient == (1_u128 << 112) - 1 && {
        let mut remainder = numerator.copy(arena);
        remainder.shift(2);
        let mut three_quarters = denominator.copy(arena);
        three_quarters.mul_add(3, 0);
        !remainder.compare(&three_quarters).is_lt()
    };
    let underflow = inexact && binary_exponent < -16382 && !rounds_to_normal;
    numerator.shift(1);
    let rounding = numerator.compare(&denominator);
    if rounding.is_gt() || (rounding.is_eq() && quotient & 1 != 0) {
        quotient += 1;
    }
    if quotient == 0 {
        return Err(error(Some(FloatRangeError::Underflow)));
    }
    if quotient == 1_u128 << 113 {
        quotient >>= 1;
        binary_exponent += 1;
    }
    if binary_exponent > 16383 {
        return Err(error(Some(FloatRangeError::Overflow)));
    }
    let encoded_exponent = if binary_exponent < -16382 && quotient < 1_u128 << 112 {
        0
    } else {
        (binary_exponent.max(-16382) + 16383) as u128
    };
    Ok((
        Binary128(Packed::new(
            (encoded_exponent << 112) | (quotient & ((1_u128 << 112) - 1)),
        )),
        underflow,
    ))
}

/// IEEE binary128 bits at token alignment. Arithmetic is deliberately not
/// approximated through the host's binary64 or x87 long double.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Binary128(pub(crate) Packed<u128>);

// Tests
#[cfg(test)]
mod tests;
