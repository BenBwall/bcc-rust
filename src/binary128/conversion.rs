//! Rounding an exact magnitude into binary128, and the conversions between
//! binary128, integers, the narrower IEEE formats and the x87 format.
//!
//! Every operation hands [`Binary128::rounded`] its exact result as a
//! 256-bit [`Wide`] magnitude, or one whose discarded low bits are
//! summarized by a sticky bit below the rounding position. Rounding is to
//! nearest, ties to even, at 113 bits; a magnitude below the smallest
//! normal is first denormalized, so a subnormal result is rounded once at
//! its own, smaller precision.
//!
//! NaNs convert as libgcc's soft-float routines convert them on x86-64
//! (`__extenddftf2`, `__trunctfdf2` …): the sign and the top of the payload
//! are kept, aligned at the top of the fraction, and the quiet bit is set.

use super::{
    Binary128,
    encoding::{
        BIAS,
        Class,
        FRACTION_BITS,
        HIDDEN_BIT,
        MAX_BIASED,
        QUIET_BIT,
    },
};
use crate::x87::{
    self,
    X87,
};

/// The exponent of the lowest significand bit of the smallest subnormal,
/// 2^-16494.
const MIN_QUANTUM: i32 = 1 - BIAS - FRACTION_BITS as i32;

/// A 256-bit natural number: an exact product of two significands, or a
/// magnitude with a sticky bit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Wide {
    pub(super) high: u128,
    pub(super) low:  u128,
}

impl Wide {
    pub(super) const fn new(value: u128) -> Self {
        Self {
            high: 0,
            low:  value,
        }
    }

    /// The exact product `a × b`.
    pub(super) const fn product(a: u128, b: u128) -> Self {
        const MASK: u128 = u64::MAX as u128;
        let (a_high, a_low) = (a >> 64, a & MASK);
        let (b_high, b_low) = (b >> 64, b & MASK);
        let (middle, middle_carry) = (a_high * b_low).overflowing_add(a_low * b_high);
        let (low, low_carry) = (a_low * b_low).overflowing_add(middle << 64);
        let high = (a_high * b_high)
            .wrapping_add(middle >> 64)
            .wrapping_add((middle_carry as u128) << 64)
            .wrapping_add(low_carry as u128);
        Self { high, low }
    }

    const fn is_zero(self) -> bool {
        self.high == 0 && self.low == 0
    }

    const fn leading_zeros(self) -> u32 {
        if self.high == 0 {
            128 + self.low.leading_zeros()
        } else {
            self.high.leading_zeros()
        }
    }

    /// Bit `index`, below 256.
    const fn bit(self, index: u32) -> bool {
        if index >= 128 {
            (self.high >> (index - 128)) & 1 != 0
        } else {
            (self.low >> index) & 1 != 0
        }
    }

    /// Whether any of the `count` lowest bits is set, for `count` up to 256.
    const fn any_below(self, count: u32) -> bool {
        match count {
            | 0 => false,
            | 1..128 => self.low & ((1 << count) - 1) != 0,
            | 128 => self.low != 0,
            | 129..256 => self.low != 0 || self.high & ((1 << (count - 128)) - 1) != 0,
            | _ => !self.is_zero(),
        }
    }

    /// The bits from `shift` up, for a result known to fit 128 bits.
    const fn above(self, shift: u32) -> u128 {
        match shift {
            | 0 => self.low,
            | 1..128 => (self.high << (128 - shift)) | (self.low >> shift),
            | 128..256 => self.high >> (shift - 128),
            | _ => 0,
        }
    }
}

impl Binary128 {
    /// The binary128 value nearest `magnitude × 2^exponent` with the given
    /// sign, to nearest with ties to even. A nonzero bit below the rounding
    /// position may stand for any nonzero remainder (a sticky bit).
    /// C99: §5.2.4.2.2 paragraph 7, p. 24; PDF p. 36 (`FLT_ROUNDS` 1).
    pub(super) fn rounded(negative: bool, exponent: i32, magnitude: Wide) -> Self {
        if magnitude.is_zero() {
            return Self::zero(negative);
        }
        // The leading bit's exponent.
        let leading = exponent + 255 - magnitude.leading_zeros() as i32;
        let mut quantum = (leading - FRACTION_BITS as i32).max(MIN_QUANTUM);
        let mut significand = if quantum <= exponent {
            // Exact: the leading bit lands at or below the hidden bit.
            magnitude.low << (exponent - quantum)
        } else {
            let shift = (quantum - exponent).unsigned_abs();
            let kept = magnitude.above(shift);
            let half = shift <= 256 && magnitude.bit(shift - 1);
            let sticky = magnitude.any_below((shift - 1).min(256));
            kept + u128::from(half && (sticky || kept & 1 != 0))
        };
        if significand == HIDDEN_BIT << 1 {
            significand >>= 1;
            quantum += 1;
        }
        if significand == 0 {
            return Self::zero(negative);
        }
        if quantum + FRACTION_BITS as i32 > BIAS {
            return Self::infinity(negative);
        }
        let biased = if significand & HIDDEN_BIT == 0 {
            0
        } else {
            #[expect(
                clippy::cast_sign_loss,
                reason = "A normal result has a positive biased exponent."
            )]
            let biased = (quantum + FRACTION_BITS as i32 + BIAS) as u128;
            biased
        };
        Self::from_parts(negative, biased, significand & (HIDDEN_BIT - 1))
    }

    /// The value of a signed or unsigned integer, rounded to nearest (a
    /// magnitude of more than 113 bits can be inexact).
    /// C99: §6.3.1.4 paragraph 2, p. 43; PDF p. 55.
    pub(crate) fn from_integer(negative: bool, magnitude: u128) -> Self {
        Self::rounded(negative, 0, Wide::new(magnitude))
    }

    /// The value truncated toward zero as an integer of `bits` bits, or
    /// `None` for a NaN, an infinity or a value out of range. The result is
    /// the integer's two's-complement bits.
    /// C99: §6.3.1.4 paragraph 1, p. 43; PDF p. 55.
    pub(crate) fn to_integer(self, signed: bool, bits: u32) -> Option<u128> {
        let magnitude = match self.class() {
            | Class::Zero => 0,
            | Class::Finite {
                exponent,
                significand,
            } => match exponent {
                | ..=-128 => 0,
                | ..0 => significand >> exponent.unsigned_abs(),
                | _ if significand.leading_zeros() < exponent.unsigned_abs() => return None,
                | _ => significand << exponent,
            },
            | Class::Infinite | Class::Nan { .. } => return None,
        };
        let negative = self.is_negative() && magnitude != 0;
        let limit = |width: u32| {
            1_u128
                .checked_shl(width)
                .map_or(u128::MAX, |limit| limit - 1)
        };
        let fits = match (signed, negative) {
            | (true, true) => magnitude - 1 <= limit(bits - 1),
            | (true, false) => magnitude <= limit(bits - 1),
            | (false, true) => false,
            | (false, false) => magnitude <= limit(bits),
        };
        let mask = limit(bits);
        fits.then(|| {
            if negative {
                magnitude.wrapping_neg() & mask
            } else {
                magnitude
            }
        })
    }

    /// The exact value of an IEEE binary32 or binary64 encoding with
    /// `fraction_bits` fraction bits and a `width`-bit encoding, as
    /// `__extendsftf2` and `__extenddftf2` give it: a NaN keeps its payload
    /// at the top of the fraction and is quieted.
    /// C99: §6.3.1.5 paragraph 1, p. 44; PDF p. 56.
    fn from_binary(width: u32, fraction_bits: u32, bits: u64) -> Self {
        let negative = (bits >> (width - 1)) & 1 != 0;
        let max_biased = (1 << (width - fraction_bits - 1)) - 1;
        let biased = (bits >> fraction_bits) & max_biased;
        let fraction = u128::from(bits & ((1 << fraction_bits) - 1));
        if biased == max_biased {
            return if fraction == 0 {
                Self::infinity(negative)
            } else {
                let payload = fraction << (FRACTION_BITS - fraction_bits);
                Self::from_parts(negative, MAX_BIASED, payload | QUIET_BIT)
            };
        }
        let offset = i32::try_from(max_biased >> 1).expect("a small bias");
        let (exponent, significand) = if biased == 0 {
            (1 - offset, fraction)
        } else {
            let biased = i32::try_from(biased).expect("a small exponent field");
            (biased - offset, fraction | 1 << fraction_bits)
        };
        Self::rounded(
            negative,
            exponent - fraction_bits as i32,
            Wide::new(significand),
        )
    }

    pub(crate) fn from_f64(value: f64) -> Self {
        Self::from_binary(64, 52, value.to_bits())
    }

    pub(crate) fn from_f32(value: f32) -> Self {
        Self::from_binary(32, 23, u64::from(value.to_bits()))
    }

    /// The value of an x87 encoding, as `__extendxftf2` gives it: the
    /// explicit integer bit is ignored and the other fields are moved across,
    /// so an unnormal reads as the normal value of its fraction, a
    /// pseudo-denormal as the denormal without its integer bit, a
    /// pseudo-infinity as an infinity and a pseudo-NaN as a NaN. Both
    /// formats have the same exponent range, so every value is exact.
    /// C99: §6.3.1.5 paragraph 1, p. 44; PDF p. 56.
    pub(crate) fn from_x87(value: X87) -> Self {
        let bits = value.to_bits();
        let negative = value.is_negative();
        let biased = (bits >> 64) & MAX_BIASED;
        let fraction = bits & u128::from(u64::MAX >> 1);
        match biased {
            | MAX_BIASED if fraction == 0 => Self::infinity(negative),
            | MAX_BIASED => Self::from_parts(negative, MAX_BIASED, fraction << 49 | QUIET_BIT),
            | _ => Self::from_parts(negative, biased, fraction << 49),
        }
    }

    /// The x87 value nearest this one, as `__trunctfxf2` gives it.
    /// C99: §6.3.1.5 paragraph 2, p. 44; PDF p. 56.
    pub(crate) fn to_x87(self) -> X87 {
        let negative = self.is_negative();
        match self.class() {
            | Class::Zero => X87::zero(negative),
            | Class::Infinite => X87::infinity(negative),
            | Class::Nan { .. } => {
                let payload = (self.fraction() | QUIET_BIT) >> 49;
                X87::from_bits((u128::from(negative) << 79) | (0x7FFF << 64) | 1 << 63 | payload)
            },
            | Class::Finite {
                exponent,
                significand,
            } => X87::rounded(negative, exponent, significand),
        }
    }

    /// The IEEE binary32 or binary64 encoding nearest this value, as
    /// `__trunctfsf2` and `__trunctfdf2` give it: a NaN keeps the top of
    /// its payload and is quieted.
    /// C99: §6.3.1.5 paragraph 2, p. 44; PDF p. 56.
    fn to_binary(self, single: bool) -> u64 {
        let (width, fraction_bits) = if single { (32, 23) } else { (64, 52) };
        let negative = self.is_negative();
        let sign = u64::from(negative) << (width - 1);
        let infinity = ((1_u64 << (width - fraction_bits - 1)) - 1) << fraction_bits;
        match self.class() {
            | Class::Zero => sign,
            | Class::Infinite => sign | infinity,
            | Class::Nan { .. } => {
                let payload = (self.fraction() | QUIET_BIT) >> (FRACTION_BITS - fraction_bits);
                sign | infinity | u64::try_from(payload).expect("a narrower fraction")
            },
            | Class::Finite {
                exponent,
                significand,
            } => x87::nearest_binary(single, negative, exponent, significand),
        }
    }

    pub(crate) fn to_f64(self) -> f64 {
        f64::from_bits(self.to_binary(false))
    }

    pub(crate) fn to_f32(self) -> f32 {
        f32::from_bits(u32::try_from(self.to_binary(true)).expect("a binary32 encoding"))
    }
}
