//! The `<math.h>` functions whose binary128 results are exact: rounding to
//! integers, scaling and decomposition. Each gives glibc's `_Float128`
//! result bit for bit; NaN operands are quieted.

use super::{
    Binary128,
    conversion::Wide,
    encoding::Class,
};
use crate::x87::Integral;

impl Binary128 {
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
            | Class::Zero | Class::Infinite => return self,
        };
        if exponent >= 0 {
            return self;
        }
        let shift = exponent.unsigned_abs();
        let (integer, fraction) = if shift >= 128 {
            (0, significand)
        } else {
            (significand >> shift, significand & ((1 << shift) - 1))
        };
        // The fraction compared with one half; a shift of 128 or more leaves
        // a fraction below 2^113, far less than half.
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

    /// `roundf128`: the nearest integer, halfway cases away from zero.
    /// C99: §7.12.9.6, p. 234; PDF p. 246.
    pub(crate) fn round_half_away(self) -> Self {
        let truncated = self.round_integral(Integral::Truncate);
        let Class::Finite { .. } = self.class() else {
            return truncated;
        };
        let half = Self::from_parts(false, 0x3FFE, 0);
        let fraction = self.sub(truncated).with_sign(false);
        if fraction
            .compare(half)
            .is_some_and(std::cmp::Ordering::is_ge)
        {
            truncated.add(Self::from_integer(self.is_negative(), 1))
        } else {
            truncated
        }
    }

    /// The value times 2^`power`, rounded once, as `scalbnf128` and
    /// `ldexpf128` give it.
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
                Self::rounded(self.is_negative(), exponent + power, Wide::new(significand))
            },
            | Class::Nan { .. } => self.quieted(),
            | Class::Zero | Class::Infinite => self,
        }
    }

    /// `frexpf128`: a fraction in [0.5, 1) with the sign of the value, and
    /// the power of two that scales it back. Zeros, infinities and NaNs
    /// return themselves with exponent zero.
    /// C99: §7.12.6.4, p. 225; PDF p. 237.
    pub(crate) fn decompose(self) -> (Self, i32) {
        match self.class() {
            | Class::Finite {
                exponent,
                significand,
            } => {
                let leading = exponent + 127 - significand.leading_zeros() as i32;
                (self.scale(-i64::from(leading) - 1), leading + 1)
            },
            | Class::Nan { .. } => (self.quieted(), 0),
            | Class::Zero | Class::Infinite => (self, 0),
        }
    }
}
