//! The `<math.h>` functions whose x87 results are exact or correctly
//! rounded, so software computes the C library's result bit for bit:
//! rounding to integers, scaling and decomposition, IEEE remainders and
//! neighbouring values. Each follows glibc's x86-64 `long double` version.

use super::{
    Class,
    X87,
};

/// How a value is rounded to an integer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Integral {
    /// Toward zero (`trunc`).
    Truncate,
    /// Toward negative infinity (`floor`).
    Floor,
    /// Toward positive infinity (`ceil`).
    Ceil,
    /// To nearest, halfway cases to even (`rint` and `nearbyint` in the
    /// default rounding mode).
    HalfEven,
}

impl X87 {
    /// The value rounded to an integer in `mode`, keeping the sign of a
    /// zero result. NaNs are quieted; infinities are returned.
    /// C99: §7.12.9, pp. 232-235; PDF pp. 244-247.
    pub(crate) fn round_integral(self, mode: Integral) -> Self {
        let negative = self.is_negative();
        let (exponent, significand) = match self.class() {
            | Class::Finite {
                exponent,
                significand,
            } => (exponent, significand),
            | Class::Nan { .. } => return self.quieted(),
            | Class::Unsupported => return Self::INDEFINITE,
            | Class::Zero | Class::Infinite => return self,
        };
        if exponent >= 0 {
            return Self::rounded(negative, exponent, u128::from(significand));
        }
        let shift = exponent.unsigned_abs();
        let significand = u128::from(significand);
        let (integer, fraction) = if shift >= 128 {
            (0, significand)
        } else {
            (significand >> shift, significand & ((1 << shift) - 1))
        };
        // The fraction compared with one half; a shift of 128 or more leaves
        // a fraction below 2^64, far less than half.
        let (above, at) = if shift >= 128 {
            (false, false)
        } else {
            let half = 1 << (shift - 1);
            (fraction > half, fraction == half)
        };
        let up = match mode {
            | Integral::Truncate => false,
            | Integral::Floor => negative && fraction != 0,
            | Integral::Ceil => !negative && fraction != 0,
            | Integral::HalfEven => above || (at && integer & 1 != 0),
        };
        Self::from_integer(negative, integer + u128::from(up))
    }

    /// The value as an integer of `bits` bits rounded in `mode`, or the
    /// integer indefinite (only the sign bit set) that `fistp` stores for a
    /// NaN or a value out of range, as glibc's `lrintl` and `lroundl` give.
    /// C99: §7.12.9.5, p. 233; PDF p. 245; §7.12.9.7, p. 234; PDF p. 246.
    pub(crate) fn to_rounded_integer(self, mode: Integral, bits: u32) -> u128 {
        self.round_integral(mode)
            .to_integer(true, bits)
            .unwrap_or(1 << (bits - 1))
    }

    /// `fscale`, and so `scalbnl`: the value times 2^`power`, rounded once.
    /// An invalid encoding gives the real indefinite, as `fscale` does.
    /// C99: §7.12.6.13, p. 228; PDF p. 240.
    pub(crate) fn scale(self, power: i64) -> Self {
        match self.class() {
            | Class::Finite {
                exponent,
                significand,
            } => {
                // Beyond this, every result has overflowed or underflowed.
                let power = power.clamp(-0x10000, 0x10000);
                let power = i32::try_from(power).expect("clamped to a small range");
                Self::rounded(
                    self.is_negative(),
                    exponent + power,
                    u128::from(significand),
                )
            },
            | Class::Nan { .. } => self.quieted(),
            | Class::Unsupported => Self::INDEFINITE,
            | Class::Zero | Class::Infinite => self,
        }
    }

    /// The exponent of the leading bit of a finite nonzero value, as
    /// `ilogbl` and `logbl` give it.
    /// C99: §7.12.6.5, p. 226; PDF p. 238; §7.12.6.11, p. 227; PDF p. 239.
    pub(crate) fn binary_exponent(self) -> Option<i32> {
        match self.class() {
            | Class::Finite {
                exponent,
                significand,
            } => Some(exponent + 63 - significand.leading_zeros() as i32),
            | _ => None,
        }
    }

    /// The IEEE remainder `self - n × other` for `n` the quotient rounded
    /// to nearest even, as `fprem1` and so `remainderl` give it.
    /// C99: §7.12.10.2 paragraphs 2-3, p. 235; PDF p. 247.
    pub(crate) fn remainder(self, other: Self) -> Self {
        let truncated = self.rem(other);
        let finite = |value: Self| matches!(value.class(), Class::Finite { .. });
        if !finite(self)
            || !finite(other)
            || !(finite(truncated) || truncated.class() == Class::Zero)
        {
            return truncated;
        }
        let divisor = other.with_sign(false);
        let quotient = low_quotient_bits(self.with_sign(false), divisor);
        let remainder = truncated.with_sign(false);
        // Twice the remainder is exact unless it overflows, when it exceeds
        // the divisor anyway; the difference is exact by Sterbenz's lemma.
        let order = remainder
            .scale(1)
            .compare(divisor)
            .expect("finite values are ordered");
        if order.is_gt() || (order.is_eq() && quotient & 1 != 0) {
            divisor.sub(remainder).with_sign(!self.is_negative())
        } else {
            remainder.with_sign(self.is_negative())
        }
    }

    /// The biased exponent and the significand, without the sign.
    pub(crate) const fn magnitude_parts(self) -> (u16, u64) {
        (self.biased_exponent(), self.significand)
    }
}

/// The low 64 bits of `a / b` truncated, for finite positive `a` and `b`;
/// the IEEE remainder needs the parity.
#[expect(
    clippy::cast_possible_truncation,
    reason = "Only the quotient's low bits are kept."
)]
fn low_quotient_bits(a: X87, b: X87) -> u64 {
    let (
        Class::Finite {
            exponent: a_exponent,
            significand: a,
        },
        Class::Finite {
            exponent: b_exponent,
            significand: b,
        },
    ) = (a.class(), b.class())
    else {
        return 0;
    };
    let (a_exponent, a) = (
        a_exponent - a.leading_zeros() as i32,
        a << a.leading_zeros(),
    );
    let (b_exponent, b) = (
        b_exponent - b.leading_zeros() as i32,
        b << b.leading_zeros(),
    );
    let b = u128::from(b);
    if a_exponent < b_exponent {
        return 0;
    }
    let mut remainder = u128::from(a);
    let mut quotient = (remainder / b) as u64;
    remainder %= b;
    let mut difference = (a_exponent - b_exponent).unsigned_abs();
    while difference > 0 {
        let step = difference.min(64);
        let shifted = remainder << step;
        // A shift by all 64 bits leaves none of the old quotient.
        let kept = quotient.checked_shl(step).unwrap_or(0);
        quotient = kept.wrapping_add((shifted / b) as u64);
        remainder = shifted % b;
        difference -= step;
    }
    quotient
}
