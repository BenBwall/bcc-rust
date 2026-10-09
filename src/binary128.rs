//! Exact phase-7 binary128 literal conversion, independent of host floats.
//! GCC Additional Floating Types: `q`/`Q` denotes IEEE binary128.
//! C99: floating grammar §6.4.4.2, pp. 57-58; PDF pp. 69-70.
//! Decimal conversion uses arena integers and rounds once, ties to even.

use std::{
    cmp::Ordering,
    fmt,
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

/// IEEE binary128 bits at token alignment. Arithmetic is deliberately not
/// approximated through the host's binary64 or x87 long double.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Binary128(pub(crate) Packed<u128>);

impl fmt::Display for Binary128 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bits = self.0.get();
        let exponent = (bits >> 112) & 0x7FFF;
        let fraction = bits & ((1_u128 << 112) - 1);
        if exponent == 0x7FFF {
            return f.write_str("inf");
        }
        if exponent == 0 && fraction == 0 {
            return f.write_str("0x0p+0");
        }
        let power = if exponent == 0 {
            -16382
        } else {
            exponent as i32 - 16383
        };
        write!(f, "0x{}.{fraction:028x}p{power:+}", u8::from(exponent != 0))
    }
}

/// Nonnegative little-endian base-2^32 integer; storage belongs to scratch.
struct Natural<'a>(ArenaVec<'a, u32>);

#[expect(
    clippy::cast_possible_truncation,
    reason = "Base-2^32 limb stores discard high bits; arena extents fit usize on the required \
              64-bit host."
)]
impl<'a> Natural<'a> {
    fn new(arena: &'a Bump, value: u32) -> Self {
        let mut words = ArenaVec::new_in(arena);
        if value != 0 {
            words.push(value);
        }
        Self(words)
    }

    fn trim(&mut self) {
        while self.0.last() == Some(&0) {
            _ = self.0.pop();
        }
    }

    fn mul_add(&mut self, multiplier: u32, addend: u32) {
        let mut carry = u64::from(addend);
        for word in &mut self.0 {
            let value = u64::from(*word) * u64::from(multiplier) + carry;
            *word = value as u32;
            carry = value >> 32;
        }
        if carry != 0 {
            self.0.push(carry as u32);
        }
    }

    fn bits(&self) -> i64 {
        self.0.last().map_or(0, |word| {
            (self.0.len() as i64 - 1) * 32 + i64::from(32 - word.leading_zeros())
        })
    }

    fn shift(&mut self, count: i64) {
        if self.0.is_empty() || count == 0 {
            return;
        }
        let words = (count / 32) as usize;
        let bits = (count % 32) as u32;
        let old = self.0.len();
        self.0.resize(old + words + 1, 0);
        self.0.copy_within(..old, words);
        self.0[..words].fill(0);
        let mut carry = 0_u64;
        for word in &mut self.0[words..] {
            let value = (u64::from(*word) << bits) | carry;
            *word = value as u32;
            carry = value >> 32;
        }
        self.trim();
    }

    fn halve(&mut self) {
        let mut carry = 0;
        for word in self.0.iter_mut().rev() {
            let next = *word << 31;
            *word = (*word >> 1) | carry;
            carry = next;
        }
        self.trim();
    }

    fn compare(&self, other: &Self) -> Ordering {
        self.0
            .len()
            .cmp(&other.0.len())
            .then_with(|| self.0.iter().rev().cmp(other.0.iter().rev()))
    }

    fn subtract(&mut self, other: &Self) {
        let mut borrow = 0_u64;
        for (index, word) in self.0.iter_mut().enumerate() {
            let sub = u64::from(other.0.get(index).copied().unwrap_or(0)) + borrow;
            let value = u64::from(*word);
            *word = value.wrapping_sub(sub) as u32;
            borrow = u64::from(value < sub);
        }
        self.trim();
    }

    fn copy(&self, arena: &'a Bump) -> Self {
        let mut words = ArenaVec::new_in(arena);
        words.extend_from_slice(&self.0);
        Self(words)
    }
}

fn error(range: Option<FloatRangeError>) -> ParseFloatError {
    let bits = if matches!(range, Some(FloatRangeError::Overflow)) {
        0x7FFF_u128 << 112
    } else {
        0
    };
    let value = FloatTokenType::Float128(Binary128(Packed::new(bits)));
    range.map_or(ParseFloatError::Invalid(value), |range| {
        ParseFloatError::OutOfRange(value, range)
    })
}

fn exponent(text: &str) -> Option<i64> {
    let (negative, digits) = if let Some(rest) = text.strip_prefix('-') {
        (true, rest)
    } else {
        (false, text.strip_prefix('+').unwrap_or(text))
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let mut result = 0_i64;
    for digit in digits.bytes() {
        result = result
            .saturating_mul(10)
            .saturating_add(i64::from(digit - b'0'));
    }
    Some(if negative { -result } else { result })
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_binary128_rounding_and_range() {
        let arena = Bump::new();
        for (text, bits) in [
            ("1.0q", 0x3FFF_0000_0000_0000_0000_0000_0000_0000),
            ("0.1Q", 0x3FFB_9999_9999_9999_9999_9999_9999_999A),
            (
                "0x1.0000000000000000000000000001p0q",
                0x3FFF_0000_0000_0000_0000_0000_0000_0001,
            ),
            (
                "0x1.00000000000000000000000000008p0q",
                0x3FFF_0000_0000_0000_0000_0000_0000_0000,
            ),
            (
                "0x1.00000000000000000000000000018p0q",
                0x3FFF_0000_0000_0000_0000_0000_0000_0002,
            ),
            ("0x1p-16494q", 1),
            ("0x1.8p-16495q", 1),
            (
                "0x1.ffffffffffffffffffffffffffffp16383q",
                0x7FFE_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF,
            ),
            ("0e999999999999999999999999q", 0),
        ] {
            assert_eq!(
                parse(text, &arena)
                    .unwrap_or_else(|error| panic!("{text}: {error:?}"))
                    .0
                    .get(),
                bits,
                "{text}"
            );
        }
        for text in ["0x1p16384q", "1e5000q"] {
            assert!(
                matches!(
                    parse(text, &arena),
                    Err(ParseFloatError::OutOfRange(_, FloatRangeError::Overflow))
                ),
                "{text}"
            );
        }
        for text in ["0x1p-16495q", "1e-5000q"] {
            assert!(
                matches!(
                    parse(text, &arena),
                    Err(ParseFloatError::OutOfRange(_, FloatRangeError::Underflow))
                ),
                "{text}"
            );
        }
        for text in ["1q", "0x1q", ".q", "1..0q", "1.0f128", "1.0qe1", "1e+q"] {
            assert!(
                matches!(parse(text, &arena), Err(ParseFloatError::Invalid(_))),
                "{text}"
            );
        }
    }
}
