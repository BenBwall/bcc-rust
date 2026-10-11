//! Exact phase-7 binary128 literal conversion, independent of host floats.
//! GCC Additional Floating Types: `q`/`Q` denotes IEEE binary128.
//! C99: floating grammar §6.4.4.2, pp. 57-58; PDF pp. 69-70.
//! Decimal conversion uses arena integers and rounds once, ties to even.

mod arithmetic;

use std::{
    cmp::Ordering,
    fmt,
};

use arithmetic::{
    Natural,
    error,
    exponent,
};

use crate::{
    float_parsing::{
        FloatRangeError,
        ParseFloatError,
    },
    translation_phases::preprocessing::FloatTokenType,
    util::{
        bump::{
            ArenaVec,
            Bump,
        },
        packed::Packed,
    },
};

/// Parses a suffixed spelling, retaining all significand digits before one
/// IEEE round-to-nearest-even conversion. Nonzero rounded-to-zero and infinity
/// use the existing range diagnostics (C99 §6.4.4p2, p. 54; PDF p. 66).
pub(crate) fn parse(text: &str, arena: &Bump) -> Result<Binary128, ParseFloatError> {
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
        return Ok(Binary128(Packed::new(0)));
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
    Ok(Binary128(Packed::new(
        (encoded_exponent << 112) | (quotient & ((1_u128 << 112) - 1)),
    )))
}

/// IEEE binary128 bits at token alignment. Arithmetic is deliberately not
/// approximated through the host's binary64 or x87 long double.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Binary128(pub(crate) Packed<u128>);

#[cfg(test)]
mod tests;
