//! Exact arithmetic on the values of the floating types: an unsigned
//! integer of any width ([`Big`]), the exact decimal expansion of a binary
//! value, the conversions of `printf`'s `%f %e %g %a`, and the digits of a
//! decimal floating constant.
//!
//! Every value of a binary floating type is a fraction with a power of two
//! below it, so it has a finite decimal expansion: m × 2^-k is m × 5^k /
//! 10^k. The machine writes that expansion exactly and rounds it as glibc
//! does, to the nearest, ties to even, so `printf("%.2f", 0.125)` is
//! `0.12` and `printf("%.20f", 0.1)` shows the digits of the double that
//! 0.1 became.
//!
//! C99: §7.19.6.1 paragraph 8, pp. 277-280; PDF pp. 289-292 (the f, e, g
//! and a conversions).
//! C99: §F.5 paragraph 1, p. 447; PDF p. 459 (binary-decimal conversion).

use std::{
    cmp::Ordering,
    fmt::Write,
};

/// An unsigned integer of any width, least significant 32-bit limb first,
/// without leading zero limbs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct Big {
    limbs: Vec<u32>,
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "Each cast keeps the low 32 bits of a wider intermediate as one limb, as intended."
)]
impl Big {
    pub(super) fn from_u128(mut value: u128) -> Self {
        let mut limbs = Vec::new();
        while value != 0 {
            limbs.push(value as u32);
            value >>= 32;
        }
        Self { limbs }
    }

    pub(super) fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }

    /// The number of bits up to and including the highest set one.
    pub(super) fn bits(&self) -> u32 {
        self.limbs.last().map_or(0, |&top| {
            u32::try_from(self.limbs.len() - 1).unwrap_or(u32::MAX) * 32
                + (32 - top.leading_zeros())
        })
    }

    /// Bit `index`, counting from the least significant.
    pub(super) fn bit(&self, index: u32) -> bool {
        self.limbs
            .get((index / 32) as usize)
            .is_some_and(|limb| limb >> (index % 32) & 1 == 1)
    }

    /// Whether any bit below `index` is set.
    pub(super) fn any_below(&self, index: u32) -> bool {
        let whole = (index / 32) as usize;
        self.limbs.iter().take(whole).any(|&limb| limb != 0)
            || self.limbs.get(whole).is_some_and(|&limb| {
                !index.is_multiple_of(32) && limb & ((1 << (index % 32)) - 1) != 0
            })
    }

    /// The value, when it fits in 128 bits.
    pub(super) fn to_u128(&self) -> Option<u128> {
        if self.limbs.len() > 4 {
            return None;
        }
        Some(
            self.limbs
                .iter()
                .rev()
                .fold(0_u128, |value, &limb| (value << 32) | u128::from(limb)),
        )
    }

    pub(super) fn shl(&self, by: u32) -> Self {
        if self.is_zero() {
            return Self::default();
        }
        let (whole, part) = ((by / 32) as usize, by % 32);
        let mut limbs = vec![0; whole];
        let mut carry = 0_u32;
        for &limb in &self.limbs {
            if part == 0 {
                limbs.push(limb);
            } else {
                limbs.push((limb << part) | carry);
                carry = limb >> (32 - part);
            }
        }
        if carry != 0 {
            limbs.push(carry);
        }
        Self { limbs }
    }

    /// The value shifted right by `by` bits, the bits shifted out dropped.
    pub(super) fn shr(&self, by: u32) -> Self {
        let (whole, part) = ((by / 32) as usize, by % 32);
        if whole >= self.limbs.len() {
            return Self::default();
        }
        let source = &self.limbs[whole..];
        let mut limbs = Vec::with_capacity(source.len());
        for (index, &limb) in source.iter().enumerate() {
            let next = source.get(index + 1).copied().unwrap_or(0);
            limbs.push(if part == 0 {
                limb
            } else {
                (limb >> part) | (next << (32 - part))
            });
        }
        let mut result = Self { limbs };
        result.trim();
        result
    }

    pub(super) fn add(&self, other: &Self) -> Self {
        let mut limbs = Vec::with_capacity(self.limbs.len().max(other.limbs.len()) + 1);
        let mut carry = 0_u64;
        for index in 0..self.limbs.len().max(other.limbs.len()) {
            let sum = u64::from(self.limbs.get(index).copied().unwrap_or(0))
                + u64::from(other.limbs.get(index).copied().unwrap_or(0))
                + carry;
            limbs.push(sum as u32);
            carry = sum >> 32;
        }
        if carry != 0 {
            limbs.push(carry as u32);
        }
        Self { limbs }
    }

    /// `self - other`, for `other` no larger than `self`.
    pub(super) fn sub(&self, other: &Self) -> Self {
        let mut limbs = Vec::with_capacity(self.limbs.len());
        let mut borrow = 0_i64;
        for (index, &limb) in self.limbs.iter().enumerate() {
            let mut difference =
                i64::from(limb) - i64::from(other.limbs.get(index).copied().unwrap_or(0)) - borrow;
            borrow = i64::from(difference < 0);
            if difference < 0 {
                difference += 1 << 32;
            }
            limbs.push(difference as u32);
        }
        let mut result = Self { limbs };
        result.trim();
        result
    }

    pub(super) fn mul(&self, other: &Self) -> Self {
        if self.is_zero() || other.is_zero() {
            return Self::default();
        }
        let mut limbs = vec![0_u32; self.limbs.len() + other.limbs.len()];
        for (i, &a) in self.limbs.iter().enumerate() {
            let mut carry = 0_u64;
            for (j, &b) in other.limbs.iter().enumerate() {
                let product = u64::from(a) * u64::from(b) + u64::from(limbs[i + j]) + carry;
                limbs[i + j] = product as u32;
                carry = product >> 32;
            }
            limbs[i + other.limbs.len()] = carry as u32;
        }
        let mut result = Self { limbs };
        result.trim();
        result
    }

    pub(super) fn mul_small(&self, factor: u32) -> Self {
        self.mul(&Self::from_u128(u128::from(factor)))
    }

    /// `base^exponent`.
    pub(super) fn power(base: u32, exponent: u32) -> Self {
        let mut result = Self::from_u128(1);
        let mut square = Self::from_u128(u128::from(base));
        let mut rest = exponent;
        while rest > 0 {
            if rest & 1 == 1 {
                result = result.mul(&square);
            }
            rest >>= 1;
            if rest > 0 {
                square = square.mul(&square);
            }
        }
        result
    }

    /// The quotient and remainder of `self / divisor`, for a nonzero
    /// divisor, by binary long division.
    pub(super) fn div_rem(&self, divisor: &Self) -> (Self, Self) {
        if self.cmp(divisor) == Ordering::Less {
            return (Self::default(), self.clone());
        }
        let mut quotient = vec![0_u32; self.limbs.len()];
        let mut remainder = Self::default();
        for index in (0..self.bits()).rev() {
            remainder = remainder.shl(1);
            if self.bit(index) {
                if remainder.limbs.is_empty() {
                    remainder.limbs.push(1);
                } else {
                    remainder.limbs[0] |= 1;
                }
            }
            if remainder.cmp(divisor) != Ordering::Less {
                remainder = remainder.sub(divisor);
                quotient[(index / 32) as usize] |= 1 << (index % 32);
            }
        }
        let mut quotient = Self { limbs: quotient };
        quotient.trim();
        (quotient, remainder)
    }

    /// The decimal digits of the value; `"0"` for zero.
    pub(super) fn decimal(&self) -> String {
        if self.is_zero() {
            return String::from("0");
        }
        // Divide by 10^9 repeatedly, collecting nine digits at a time.
        let mut chunks = Vec::new();
        let mut rest = self.limbs.clone();
        while !rest.is_empty() {
            let mut remainder = 0_u64;
            for limb in rest.iter_mut().rev() {
                let current = (remainder << 32) | u64::from(*limb);
                *limb = (current / 1_000_000_000) as u32;
                remainder = current % 1_000_000_000;
            }
            while rest.last() == Some(&0) {
                let _zero = rest.pop();
            }
            chunks.push(remainder as u32);
        }
        let mut text = String::new();
        for (index, chunk) in chunks.iter().rev().enumerate() {
            if index == 0 {
                let _ = write!(text, "{chunk}");
            } else {
                let _ = write!(text, "{chunk:09}");
            }
        }
        text
    }

    /// The value of a string of decimal digits.
    pub(super) fn from_decimal(digits: &str) -> Self {
        let mut value = Self::default();
        for chunk in digits.as_bytes().chunks(9) {
            let mut part = 0_u32;
            for &digit in chunk {
                part = part * 10 + u32::from(digit - b'0');
            }
            let scale = 10_u32.pow(u32::try_from(chunk.len()).unwrap_or(9));
            value = value
                .mul_small(scale)
                .add(&Self::from_u128(u128::from(part)));
        }
        value
    }

    fn trim(&mut self) {
        while self.limbs.last() == Some(&0) {
            let _zero = self.limbs.pop();
        }
    }
}

impl PartialOrd for Big {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Big {
    fn cmp(&self, other: &Self) -> Ordering {
        self.limbs
            .len()
            .cmp(&other.limbs.len())
            .then_with(|| self.limbs.iter().rev().cmp(other.limbs.iter().rev()))
    }
}

/// The exact decimal expansion of the finite value `significand` ×
/// 2^`exponent`, as its digits and the number of them after the decimal
/// point: the value is `digits` × 10^-`scale`.
pub(super) fn expansion(significand: u128, exponent: i32) -> (String, usize) {
    let significand = Big::from_u128(significand);
    if exponent >= 0 {
        return (significand.shl(exponent.unsigned_abs()).decimal(), 0);
    }
    // m × 2^-k = m × 5^k / 10^k.
    let k = exponent.unsigned_abs();
    (significand.mul(&Big::power(5, k)).decimal(), k as usize)
}

/// The exact decimal value of `digits` × 10^-`scale`, as C would write it:
/// `0.1000000000000000055511151231257827021181583404541015625`, `3`.
pub(super) fn exact_text(digits: &str, scale: usize) -> String {
    let (whole, fraction) = split(digits, scale);
    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() {
        whole
    } else {
        format!("{whole}.{fraction}")
    }
}

/// The integral and fractional digits of `digits` × 10^-`scale`.
fn split(digits: &str, scale: usize) -> (String, String) {
    if digits.len() > scale {
        let (whole, fraction) = digits.split_at(digits.len() - scale);
        (String::from(whole), String::from(fraction))
    } else {
        (
            String::from("0"),
            format!("{}{digits}", "0".repeat(scale - digits.len())),
        )
    }
}

/// `digits` rounded to its first `keep` digits, to the nearest and ties to
/// even, as glibc rounds in the default rounding mode; `true` when the
/// rounding carried into a new leading digit (`999` → `1000`).
pub(super) fn round_digits(digits: &str, keep: usize) -> (String, bool) {
    if digits.len() <= keep {
        return (
            format!("{digits}{}", "0".repeat(keep - digits.len())),
            false,
        );
    }
    let bytes = digits.as_bytes();
    let (kept, dropped) = bytes.split_at(keep);
    let up = match dropped[0] {
        | b'6'..=b'9' => true,
        | b'5' =>
            dropped[1..].iter().any(|&digit| digit != b'0')
                || kept.last().is_some_and(|&digit| (digit - b'0') % 2 == 1),
        | _ => false,
    };
    let mut kept = kept.to_vec();
    if !up {
        return (String::from_utf8(kept).unwrap_or_default(), false);
    }
    for digit in kept.iter_mut().rev() {
        if *digit == b'9' {
            *digit = b'0';
        } else {
            *digit += 1;
            return (String::from_utf8(kept).unwrap_or_default(), false);
        }
    }
    kept.insert(0, b'1');
    (String::from_utf8(kept).unwrap_or_default(), true)
}

/// `%f` (`%F`) with `precision` digits after the point, of the value
/// `digits` × 10^-`scale`, without its sign.
///
/// C99: §7.19.6.1 paragraph 8, p. 278; PDF p. 290.
pub(super) fn fixed(digits: &str, scale: usize, precision: usize) -> String {
    let (whole, fraction) = split(digits, scale);
    let all = format!("{whole}{fraction}");
    let keep = whole.len() + precision;
    let (rounded, carried) = round_digits(&all, keep);
    let whole_length = whole.len() + usize::from(carried);
    let (whole, fraction) = rounded.split_at(whole_length);
    let whole = whole.trim_start_matches('0');
    let whole = if whole.is_empty() { "0" } else { whole };
    if precision == 0 {
        String::from(whole)
    } else {
        format!("{whole}.{fraction}")
    }
}

/// `%e` (`%E` when `upper`) with `precision` digits after the point.
///
/// C99: §7.19.6.1 paragraph 8, p. 278; PDF p. 290.
pub(super) fn scientific(digits: &str, scale: usize, precision: usize, upper: bool) -> String {
    let (mantissa, exponent) = significant(digits, scale, precision + 1);
    let (first, rest) = mantissa.split_at(1);
    let point = if precision == 0 { "" } else { "." };
    let e = if upper { 'E' } else { 'e' };
    let sign = if exponent < 0 { '-' } else { '+' };
    format!(
        "{first}{point}{rest}{e}{sign}{:02}",
        exponent.unsigned_abs()
    )
}

/// The first `count` significant digits of `digits` × 10^-`scale`,
/// rounded, and the decimal exponent of the first: `("125", -1)` for 0.125
/// to three digits. Zero is all zeros with exponent 0.
fn significant(digits: &str, scale: usize, count: usize) -> (String, i64) {
    let trimmed = digits.trim_start_matches('0');
    if trimmed.is_empty() {
        return ("0".repeat(count), 0);
    }
    let exponent = i64::try_from(trimmed.len()).unwrap_or(i64::MAX)
        - 1
        - i64::try_from(scale).unwrap_or(i64::MAX);
    let (rounded, carried) = round_digits(trimmed, count);
    if carried {
        (String::from(&rounded[..count]), exponent + 1)
    } else {
        (rounded, exponent)
    }
}

/// `%g` (`%G`) with `precision` significant digits (0 counts as 1),
/// trailing zeros removed.
///
/// C99: §7.19.6.1 paragraph 8, p. 278; PDF p. 290.
pub(super) fn general(digits: &str, scale: usize, precision: usize, upper: bool) -> String {
    let precision = precision.max(1);
    let (_, exponent) = significant(digits, scale, precision);
    let precision = i64::try_from(precision).unwrap_or(i64::MAX);
    let text = if precision > exponent && exponent >= -4 {
        fixed(
            digits,
            scale,
            usize::try_from(precision - 1 - exponent).unwrap_or(0),
        )
    } else {
        scientific(
            digits,
            scale,
            usize::try_from(precision - 1).unwrap_or(0),
            upper,
        )
    };
    // Without the # flag, trailing zeros are removed from the fraction,
    // and the point too when no digit follows it.
    let (number, exponent) = match text.find(['e', 'E']) {
        | Some(at) => text.split_at(at),
        | None => (text.as_str(), ""),
    };
    let number = if number.contains('.') {
        number.trim_end_matches('0').trim_end_matches('.')
    } else {
        number
    };
    format!("{number}{exponent}")
}

/// `%a` (`%A`) of a finite value, without its sign: the hexadecimal
/// digits of `significand`, of which `fraction` follow the point, scaled
/// by 2^`exponent`; all of them unless a precision is given. A rounding
/// carry that leaves more than one leading digit is written as glibc
/// writes it for each format: `0x2.0p+0` from double's leading 1,
/// `0x1.0p+4` renormalized from long double's leading `f`.
///
/// C99: §7.19.6.1 paragraph 8, p. 279; PDF p. 291.
pub(super) fn hexadecimal(
    significand: u64,
    fraction: u32,
    exponent: i32,
    precision: Option<usize>,
    renormalize: bool,
    upper: bool,
) -> String {
    let digits = (fraction / 4) as usize;
    let mut lead = significand >> fraction;
    let mut rest = significand & ((1_u64 << fraction) - 1);
    let mut exponent = exponent;
    let mut shown = digits;
    if let Some(precision) = precision
        && precision < digits
    {
        let drop = u32::try_from(4 * (digits - precision)).unwrap_or(fraction);
        let kept = rest >> drop;
        let dropped = rest & ((1_u64 << drop) - 1);
        let half = 1_u64 << (drop - 1);
        let up = dropped > half
            || (dropped == half && (kept & 1 == 1 || (precision == 0 && lead & 1 == 1)));
        let mut kept = kept + u64::from(up);
        if kept >> (4 * precision) != 0 {
            kept &= (1_u64 << (4 * precision)) - 1;
            lead += 1;
        }
        if renormalize && lead > 0xF {
            lead = 1;
            kept = 0;
            exponent += 4;
        }
        rest = kept;
        shown = precision;
    }
    let mut text = if upper {
        format!("0X{lead:X}")
    } else {
        format!("0x{lead:x}")
    };
    let mut fraction_text = String::new();
    for index in (0..shown).rev() {
        let digit = (rest >> (4 * index)) & 0xF;
        let _ = if upper {
            write!(fraction_text, "{digit:X}")
        } else {
            write!(fraction_text, "{digit:x}")
        };
    }
    match precision {
        | None => fraction_text = String::from(fraction_text.trim_end_matches('0')),
        | Some(precision) if precision > shown =>
            fraction_text.push_str(&"0".repeat(precision - shown)),
        | Some(_) => {},
    }
    if !fraction_text.is_empty() || precision.is_some_and(|precision| precision > 0) {
        text.push('.');
        text.push_str(&fraction_text);
    }
    let p = if upper { 'P' } else { 'p' };
    let _ = write!(text, "{p}{exponent:+}");
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn big_arithmetic_is_exact() {
        let a = Big::from_u128(u128::MAX);
        let b = Big::from_u128(12345);
        assert_eq!(a.add(&b).sub(&b), a);
        assert_eq!(a.mul(&b).div_rem(&b), (a.clone(), Big::default()));
        assert_eq!(Big::power(5, 3).decimal(), "125");
        assert_eq!(
            Big::from_decimal("123456789012345678901234567890").decimal(),
            "123456789012345678901234567890"
        );
        assert_eq!(Big::from_u128(1).shl(100).shr(99).to_u128(), Some(2));
        assert!(Big::from_u128(0b1000).any_below(4));
        assert!(!Big::from_u128(0b1000).any_below(3));
    }

    #[test]
    fn decimal_conversions_round_ties_to_even() {
        // 0.125 = 125 × 10^-3.
        assert_eq!(fixed("125", 3, 2), "0.12");
        assert_eq!(fixed("25", 1, 0), "2");
        assert_eq!(fixed("35", 1, 0), "4");
        assert_eq!(fixed("5", 1, 0), "0");
        assert_eq!(fixed("9995", 3, 2), "10.00");
        assert_eq!(scientific("125", 3, 2, false), "1.25e-01");
        assert_eq!(scientific("0", 0, 6, false), "0.000000e+00");
        assert_eq!(scientific("12345", 0, 0, false), "1e+04");
        assert_eq!(general("100000", 0, 6, false), "100000");
        assert_eq!(general("1000000", 0, 6, false), "1e+06");
        assert_eq!(general("1", 4, 6, false), "0.0001");
        assert_eq!(general("1", 5, 6, false), "1e-05");
        assert_eq!(general("0", 0, 6, false), "0");
        assert_eq!(exact_text("1000", 3), "1");
        assert_eq!(exact_text("15", 1), "1.5");
    }

    #[test]
    fn hexadecimal_matches_glibc() {
        // 0.1 as a double: 0x1.999999999999ap-4.
        let tenth = 0x0019_9999_9999_999A_u64;
        assert_eq!(
            hexadecimal(tenth, 52, -4, None, false, false),
            "0x1.999999999999ap-4"
        );
        assert_eq!(hexadecimal(1 << 52, 52, 0, None, false, false), "0x1p+0");
        let one = 1_u64 << 52;
        assert_eq!(
            hexadecimal(one | 0xF8 << 44, 52, 0, Some(1), false, false),
            "0x2.0p+0"
        );
        assert_eq!(
            hexadecimal(one | 0x28 << 44, 52, 0, Some(1), false, false),
            "0x1.2p+0"
        );
        assert_eq!(
            hexadecimal(one | 0x8 << 48, 52, 0, Some(0), false, false),
            "0x2p+0"
        );
        // 0xf.f8p+0L to one digit renormalizes.
        assert_eq!(
            hexadecimal(0xFF80_0000_0000_0000, 60, 0, Some(1), true, false),
            "0x1.0p+4"
        );
    }
}
