//! The binary128 operations: addition, subtraction, multiplication,
//! division, `fmodf128`, square root and comparison.
//!
//! Each operation settles NaN operands first ([`nan_operands`]), then the
//! infinite and zero cases, and computes a finite result exactly, or with a
//! sticky bit for an inexact quotient or root, before rounding it once.
//! NaNs propagate as libgcc's soft-float routines propagate them on x86-64
//! (`_FP_CHOOSENAN` in `config/i386/sfp-machine.h`): of two NaNs the one
//! with the larger fraction wins, a tie going to the first operand of `+`
//! and `*` and to the second of `-` and `/`; the result is quieted and keeps
//! its sign, and an invalid operation gives [`Binary128::DEFAULT_NAN`].

use std::cmp::Ordering;

use super::{
    Binary128,
    conversion::Wide,
    encoding::{
        Class,
        FRACTION_BITS,
    },
};

impl Binary128 {
    /// `self + other`.
    /// C99: §6.5.6 paragraph 5, p. 82; PDF p. 94.
    pub(crate) fn add(self, other: Self) -> Self {
        add(self, other, false)
    }

    /// `self - other`.
    /// C99: §6.5.6 paragraph 6, p. 82; PDF p. 94.
    pub(crate) fn sub(self, other: Self) -> Self {
        add(self, other, true)
    }

    /// `self * other`.
    /// C99: §6.5.5 paragraph 4, p. 82; PDF p. 94.
    pub(crate) fn mul(self, other: Self) -> Self {
        if let Some(result) = nan_operands(self, other, true) {
            return result;
        }
        let negative = self.is_negative() != other.is_negative();
        match (self.class(), other.class()) {
            | (Class::Infinite, Class::Zero) | (Class::Zero, Class::Infinite) => Self::DEFAULT_NAN,
            | (Class::Infinite, _) | (_, Class::Infinite) => Self::infinity(negative),
            | (Class::Zero, _) | (_, Class::Zero) => Self::zero(negative),
            | (
                Class::Finite {
                    exponent: a_exponent,
                    significand: a,
                },
                Class::Finite {
                    exponent: b_exponent,
                    significand: b,
                },
            ) => Self::rounded(negative, a_exponent + b_exponent, Wide::product(a, b)),
            | _ => unreachable!("NaN operands were handled"),
        }
    }

    /// `self / other`.
    /// C99: §6.5.5 paragraph 5, p. 82; PDF p. 94; Annex F.3, p. 445; PDF
    /// p. 457 (a nonzero finite value divided by zero is infinite).
    pub(crate) fn div(self, other: Self) -> Self {
        if let Some(result) = nan_operands(self, other, false) {
            return result;
        }
        let negative = self.is_negative() != other.is_negative();
        match (self.class(), other.class()) {
            | (Class::Infinite, Class::Infinite) | (Class::Zero, Class::Zero) => Self::DEFAULT_NAN,
            | (Class::Infinite, _) | (_, Class::Zero) => Self::infinity(negative),
            | (_, Class::Infinite) | (Class::Zero, _) => Self::zero(negative),
            | (
                Class::Finite {
                    exponent: a_exponent,
                    significand: a,
                },
                Class::Finite {
                    exponent: b_exponent,
                    significand: b,
                },
            ) => {
                let (a_exponent, a) = normalize(a_exponent, a);
                let (b_exponent, b) = normalize(b_exponent, b);
                // With `a` at least `b` and below `2b`, long division gives
                // 115 quotient bits, the first one set; what is left becomes
                // the sticky bit.
                let (a_exponent, mut remainder) = if a < b {
                    (a_exponent - 1, a << 1)
                } else {
                    (a_exponent, a)
                };
                let mut quotient = 0_u128;
                for _ in 0..115 {
                    quotient <<= 1;
                    if remainder >= b {
                        remainder -= b;
                        quotient |= 1;
                    }
                    remainder <<= 1;
                }
                Self::rounded(
                    negative,
                    a_exponent - b_exponent - 114,
                    Wide::new(quotient | u128::from(remainder != 0)),
                )
            },
            | _ => unreachable!("NaN operands were handled"),
        }
    }

    /// The remainder `fmodf128` gives: `self - n × other` for `n` the
    /// quotient truncated toward zero, which is always exact, with the sign
    /// of `self`. An infinite dividend or a zero divisor is invalid; glibc
    /// returns `(x * y) / (x * y)` for those and for NaN operands.
    /// C99: §7.12.10.1 paragraphs 2-3, p. 235; PDF p. 247.
    pub(crate) fn rem(self, other: Self) -> Self {
        if let Some(result) = nan_operands(self, other, true) {
            return result;
        }
        let negative = self.is_negative();
        match (self.class(), other.class()) {
            | (Class::Infinite, _) | (_, Class::Zero) => Self::DEFAULT_NAN,
            | (Class::Zero | Class::Finite { .. }, Class::Infinite) | (Class::Zero, _) => self,
            | (
                Class::Finite {
                    exponent: a_exponent,
                    significand: a,
                },
                Class::Finite {
                    exponent: b_exponent,
                    significand: b,
                },
            ) => {
                let (a_exponent, a) = normalize(a_exponent, a);
                let (b_exponent, b) = normalize(b_exponent, b);
                if a_exponent < b_exponent {
                    return self;
                }
                // Reduce `a × 2^(a_exponent - b_exponent)` modulo `b`, 15 bits
                // of the exponent difference at a time, which keeps the
                // shifted remainder within 128 bits.
                let mut remainder = a % b;
                let mut difference = (a_exponent - b_exponent).unsigned_abs();
                while difference > 0 {
                    let step = difference.min(15);
                    remainder = (remainder << step) % b;
                    difference -= step;
                }
                Self::rounded(negative, b_exponent, Wide::new(remainder))
            },
            | _ => unreachable!("NaN operands were handled"),
        }
    }

    /// The correctly rounded square root, as `sqrtf128` gives it. The root
    /// of a negative nonzero value is invalid; `-0` is its own root.
    /// C99: §7.12.7.5 paragraphs 2-3, p. 228; PDF p. 240.
    pub(crate) fn sqrt(self) -> Self {
        match self.class() {
            | Class::Nan { .. } => self.quieted(),
            | Class::Zero => self,
            | _ if self.is_negative() => Self::DEFAULT_NAN,
            | Class::Infinite => self,
            | Class::Finite {
                exponent,
                significand,
            } => {
                let (exponent, significand) = normalize(exponent, significand);
                // A radicand of 230 or 231 bits with an even exponent has an
                // integer root of 115 or 116 bits.
                let shift = if (exponent - 117) % 2 == 0 { 117 } else { 118 };
                let radicand = Wide {
                    high: significand >> (128 - shift),
                    low:  significand << shift,
                };
                let (root, inexact) = square_root(radicand);
                Self::rounded(
                    false,
                    (exponent - shift) / 2,
                    Wide::new(root | u128::from(inexact)),
                )
            },
        }
    }

    /// The order of two values, or `None` when either is a NaN. Zeros of
    /// either sign are equal.
    /// C99: §6.5.8 paragraphs 3-6, p. 85; PDF p. 97; §6.5.9 paragraph 3,
    /// p. 86; PDF p. 98.
    pub(crate) fn compare(self, other: Self) -> Option<Ordering> {
        if self.is_nan() || other.is_nan() {
            return None;
        }
        // Magnitudes order as their encodings without the sign do.
        let key = |value: Self| {
            let magnitude = value.with_sign(false).to_bits().cast_signed();
            if value.is_negative() {
                -magnitude
            } else {
                magnitude
            }
        };
        Some(key(self).cmp(&key(other)))
    }
}

/// `a + b`, or `a - b` when `subtract`.
fn add(a: Binary128, b: Binary128, subtract: bool) -> Binary128 {
    if let Some(result) = nan_operands(a, b, !subtract) {
        return result;
    }
    let a_negative = a.is_negative();
    let b_negative = b.is_negative() != subtract;
    match (a.class(), b.class()) {
        | (Class::Infinite, Class::Infinite) if a_negative != b_negative => Binary128::DEFAULT_NAN,
        | (Class::Infinite, _) => Binary128::infinity(a_negative),
        | (_, Class::Infinite) => Binary128::infinity(b_negative),
        // C99 Annex F.8.3: `-0 + -0` is the only sum of zeros that is `-0`
        // when rounding to nearest.
        | (Class::Zero, Class::Zero) => Binary128::zero(a_negative && b_negative),
        | (Class::Zero, _) => b.with_sign(b_negative),
        | (_, Class::Zero) => a,
        | (
            Class::Finite {
                exponent: a_exponent,
                significand: a,
            },
            Class::Finite {
                exponent: b_exponent,
                significand: b,
            },
        ) => {
            // 13 guard bits below each significand and two above leave room
            // for the carry of a sum; an operand shifted out of the guard
            // bits only matters through its sticky bit.
            let (mut high, mut low) = ((a_exponent, a_negative, a), (b_exponent, b_negative, b));
            if high.0 < low.0 {
                std::mem::swap(&mut high, &mut low);
            }
            let exponent = high.0 - 13;
            let high_magnitude = high.2 << 13;
            let difference = (high.0 - low.0).unsigned_abs();
            let low_magnitude = low.2 << 13;
            let low_magnitude = if difference >= 128 {
                1
            } else {
                let shifted = low_magnitude >> difference;
                shifted | u128::from(shifted << difference != low_magnitude)
            };
            let (negative, magnitude) = if high.1 == low.1 {
                (high.1, high_magnitude + low_magnitude)
            } else {
                match high_magnitude.cmp(&low_magnitude) {
                    | Ordering::Greater => (high.1, high_magnitude - low_magnitude),
                    | Ordering::Less => (low.1, low_magnitude - high_magnitude),
                    // An exact zero difference is +0 when rounding to
                    // nearest (IEC 60559 §6.3).
                    | Ordering::Equal => return Binary128::zero(false),
                }
            };
            Binary128::rounded(negative, exponent, Wide::new(magnitude))
        },
        | _ => unreachable!("NaN operands were handled"),
    }
}

/// The result of a two-operand operation that a NaN decides, quieted: the
/// NaN operand, or of two the one with the larger fraction, a tie going to
/// `a` when `first_on_tie`.
fn nan_operands(a: Binary128, b: Binary128, first_on_tie: bool) -> Option<Binary128> {
    Some(match (a.is_nan(), b.is_nan()) {
        | (true, true) => {
            let take_a = match a.fraction().cmp(&b.fraction()) {
                | Ordering::Greater => true,
                | Ordering::Less => false,
                | Ordering::Equal => first_on_tie,
            };
            if take_a { a } else { b }.quieted()
        },
        | (true, false) => a.quieted(),
        | (false, true) => b.quieted(),
        | (false, false) => return None,
    })
}

/// A nonzero significand shifted so its leading bit is the hidden bit, with
/// its exponent.
const fn normalize(exponent: i32, significand: u128) -> (i32, u128) {
    let shift = significand.leading_zeros() as i32 - (127 - FRACTION_BITS as i32);
    (exponent - shift, significand << shift)
}

/// The integer square root of `radicand`, below 2^128, and whether it is
/// inexact, computed two radicand bits at a time.
fn square_root(radicand: Wide) -> (u128, bool) {
    let mut root = 0_u128;
    let mut remainder = 0_u128;
    for pair in (0..128).rev() {
        let index = 2 * pair;
        let bits = if index >= 128 {
            (radicand.high >> (index - 128)) & 3
        } else {
            (radicand.low >> index) & 3
        };
        remainder = (remainder << 2) | bits;
        let trial = (root << 2) | 1;
        root <<= 1;
        if remainder >= trial {
            remainder -= trial;
            root |= 1;
        }
    }
    (root, remainder != 0)
}
