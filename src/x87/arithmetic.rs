//! The x87 operations: `fadd`, `fsub`, `fmul`, `fdiv`, `fprem` as C's
//! `fmodl` uses it, `fsqrt`, and `fucomi` comparison.
//!
//! Each operation settles NaN and invalid operands first
//! ([`special_operands`]), then the infinite and zero cases, and computes a
//! finite result exactly, or with a sticky bit for an inexact quotient,
//! before rounding it once.

use std::cmp::Ordering;

use super::{
    Class,
    X87,
};

impl X87 {
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
        if let Some(result) = special_operands(self, other) {
            return result;
        }
        let negative = self.is_negative() != other.is_negative();
        match (self.class(), other.class()) {
            | (Class::Infinite, Class::Zero) | (Class::Zero, Class::Infinite) => Self::INDEFINITE,
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
            ) => Self::rounded(
                negative,
                a_exponent + b_exponent,
                u128::from(a) * u128::from(b),
            ),
            | _ => unreachable!("special operands were handled"),
        }
    }

    /// `self / other`.
    /// C99: §6.5.5 paragraph 5, p. 82; PDF p. 94; Annex F.3, p. 445; PDF
    /// p. 457 (a nonzero finite value divided by zero is infinite).
    pub(crate) fn div(self, other: Self) -> Self {
        if let Some(result) = special_operands(self, other) {
            return result;
        }
        let negative = self.is_negative() != other.is_negative();
        match (self.class(), other.class()) {
            | (Class::Infinite, Class::Infinite) | (Class::Zero, Class::Zero) => Self::INDEFINITE,
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
                // Both significands lie in [2^63, 2^64), so a first quotient
                // of `a × 2^63` or `a × 2^64` has exactly 64 bits; the next
                // 64 come from the remainder, and what is left after them
                // becomes the sticky bit.
                let shift = if a >= b { 63 } else { 64 };
                let (a, b) = (u128::from(a), u128::from(b));
                let numerator = a << shift;
                let (high, remainder) = (numerator / b, numerator % b);
                let numerator = remainder << 64;
                let (low, remainder) = (numerator / b, numerator % b);
                let quotient = (high << 64) | low | u128::from(remainder != 0);
                Self::rounded(negative, a_exponent - b_exponent - shift - 64, quotient)
            },
            | _ => unreachable!("special operands were handled"),
        }
    }

    /// The remainder `fmodl` gives: `self - n × other` for `n` the quotient
    /// truncated toward zero, which is always exact, with the sign of
    /// `self`. An infinite dividend or a zero divisor is invalid.
    /// C99: §7.12.10.1 paragraphs 2-3, p. 235; PDF p. 247.
    pub(crate) fn rem(self, other: Self) -> Self {
        if let Some(result) = special_operands(self, other) {
            return result;
        }
        let negative = self.is_negative();
        match (self.class(), other.class()) {
            | (Class::Infinite, _) | (_, Class::Zero) => Self::INDEFINITE,
            | (Class::Zero, _) => self,
            | (
                Class::Finite {
                    exponent,
                    significand,
                },
                Class::Infinite,
            ) => Self::rounded(negative, exponent, u128::from(significand)),
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
                    return Self::rounded(negative, a_exponent, u128::from(a));
                }
                // Reduce `a × 2^(a_exponent - b_exponent)` modulo `b`, 64 bits
                // of the exponent difference at a time.
                let b = u128::from(b);
                let mut remainder = u128::from(a) % b;
                let mut difference = (a_exponent - b_exponent).unsigned_abs();
                while difference > 0 {
                    let step = difference.min(64);
                    remainder = (remainder << step) % b;
                    difference -= step;
                }
                Self::rounded(negative, b_exponent, remainder)
            },
            | _ => unreachable!("special operands were handled"),
        }
    }

    /// The correctly rounded square root, as `fsqrt` and so `sqrtl` give
    /// it. The root of a negative nonzero value is invalid; `-0` is its own
    /// root.
    /// C99: §7.12.7.5 paragraphs 2-3, p. 228; PDF p. 240.
    pub(crate) fn sqrt(self) -> Self {
        match self.class() {
            | Class::Unsupported => Self::INDEFINITE,
            | Class::Nan { .. } => self.quieted(),
            | Class::Zero => self,
            | _ if self.is_negative() => Self::INDEFINITE,
            | Class::Infinite => self,
            | Class::Finite {
                exponent,
                significand,
            } => {
                let (exponent, significand) = normalize(exponent, significand);
                // An even exponent keeps the root's exponent whole; the
                // radicand then lies in [2^126, 2^128), so its integer root
                // has exactly 64 bits.
                let shift = if exponent % 2 == 0 { 64 } else { 63 };
                let radicand = u128::from(significand) << shift;
                let root = radicand.isqrt();
                let remainder = radicand - root * root;
                // The root is never a midpoint, so the bit below the 64 kept
                // is set exactly when the remainder exceeds the root.
                let below = if remainder > root {
                    3
                } else {
                    u128::from(remainder != 0)
                };
                Self::rounded(false, (exponent - shift) / 2 - 2, (root << 2) | below)
            },
        }
    }

    /// The order of two values, or `None` when either is a NaN or an
    /// invalid encoding. Zeros of either sign are equal.
    /// C99: §6.5.8 paragraphs 3-6, p. 85; PDF p. 97; §6.5.9 paragraph 3,
    /// p. 86; PDF p. 98.
    pub(crate) fn compare(self, other: Self) -> Option<Ordering> {
        let (a, b) = (magnitude(self)?, magnitude(other)?);
        let (a_negative, b_negative) = (
            self.is_negative() && a != (0, 0),
            other.is_negative() && b != (0, 0),
        );
        Some(match (a_negative, b_negative) {
            | (false, false) => a.cmp(&b),
            | (true, true) => b.cmp(&a),
            | (false, true) => Ordering::Greater,
            | (true, false) => Ordering::Less,
        })
    }
}

impl X87 {
    /// `self × other + addend` rounded once, as a correctly rounded `fma`
    /// gives it, for finite or zero operands; others give the product's
    /// sum.
    /// C99: §7.12.13.1 paragraph 2, p. 239; PDF p. 251.
    pub(crate) fn mul_add(self, other: Self, addend: Self) -> Self {
        let (
            Class::Finite {
                exponent: a_exponent,
                significand: a,
            },
            Class::Finite {
                exponent: b_exponent,
                significand: b,
            },
        ) = (self.class(), other.class())
        else {
            return self.mul(other).add(addend);
        };
        let product_negative = self.is_negative() != other.is_negative();
        let (c_exponent, c) = match addend.class() {
            | Class::Zero => return self.mul(other),
            | Class::Finite {
                exponent,
                significand,
            } => (exponent, significand),
            | _ => return self.mul(other).add(addend),
        };
        // Both terms as 256-bit magnitudes with their leading bit at 254,
        // leaving a bit for the carry of a sum; the smaller one shifted to
        // the larger's scale, its lost bits kept as a sticky bit.
        let product = u128::from(a) * u128::from(b);
        let product_shift = product.leading_zeros();
        let product_exponent = a_exponent + b_exponent - product_shift as i32 - 127;
        let product = Wide::from_high(product << product_shift).shift_right(1);
        let c_shift = c.leading_zeros();
        let c_exponent = c_exponent - c_shift as i32 - 63 - 128;
        let c = Wide::from_high(u128::from(c << c_shift) << 64).shift_right(1);
        let ((high_exponent, high, high_negative), (low_exponent, low, low_negative)) =
            if product_exponent >= c_exponent {
                (
                    (product_exponent, product, product_negative),
                    (c_exponent, c, addend.is_negative()),
                )
            } else {
                (
                    (c_exponent, c, addend.is_negative()),
                    (product_exponent, product, product_negative),
                )
            };
        let low = low.shift_right_sticky((high_exponent - low_exponent).unsigned_abs());
        let (negative, sum) = if high_negative == low_negative {
            (high_negative, high.add(low))
        } else {
            match high.cmp(&low) {
                | Ordering::Greater => (high_negative, high.sub(low)),
                | Ordering::Less => (low_negative, low.sub(high)),
                | Ordering::Equal => return Self::zero(false),
            }
        };
        let (exponent, magnitude) = sum.narrow(high_exponent);
        Self::rounded(negative, exponent, magnitude)
    }
}

/// A 256-bit magnitude, for the exact sum `mul_add` rounds.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Wide {
    high: u128,
    low:  u128,
}

impl Wide {
    const fn from_high(high: u128) -> Self {
        Self { high, low: 0 }
    }

    /// Shifts right by `shift` bits, discarding them.
    fn shift_right(self, shift: u32) -> Self {
        match shift {
            | 0 => self,
            | 1..128 => Self {
                high: self.high >> shift,
                low:  (self.low >> shift) | (self.high << (128 - shift)),
            },
            | 128..256 => Self {
                high: 0,
                low:  self.high >> (shift - 128),
            },
            | _ => Self { high: 0, low: 0 },
        }
    }

    /// Shifts right by `shift` bits, setting the lowest bit when any
    /// nonzero bit is lost.
    fn shift_right_sticky(self, shift: u32) -> Self {
        let shifted = self.shift_right(shift);
        let lost = if shift >= 256 {
            self != Self::from_high(0)
        } else {
            shifted.shift_left(shift) != self
        };
        Self {
            low: shifted.low | u128::from(lost),
            ..shifted
        }
    }

    fn shift_left(self, shift: u32) -> Self {
        match shift {
            | 0 => self,
            | 1..128 => Self {
                high: (self.high << shift) | (self.low >> (128 - shift)),
                low:  self.low << shift,
            },
            | 128..256 => Self {
                high: self.low << (shift - 128),
                low:  0,
            },
            | _ => Self { high: 0, low: 0 },
        }
    }

    fn add(self, other: Self) -> Self {
        let (low, carry) = self.low.overflowing_add(other.low);
        Self {
            high: self.high + other.high + u128::from(carry),
            low,
        }
    }

    fn sub(self, other: Self) -> Self {
        let (low, borrow) = self.low.overflowing_sub(other.low);
        Self {
            high: self.high - other.high - u128::from(borrow),
            low,
        }
    }

    /// The nonzero magnitude `self × 2^exponent` as its leading 128 bits
    /// and their exponent, the lost low bits kept as a sticky bit.
    fn narrow(self, exponent: i32) -> (i32, u128) {
        let leading = if self.high == 0 {
            128 + self.low.leading_zeros()
        } else {
            self.high.leading_zeros()
        };
        let normalized = self.shift_left(leading);
        (
            exponent + 128 - leading as i32,
            normalized.high | u128::from(normalized.low != 0),
        )
    }
}

/// `a + b`, or `a - b` when `subtract`.
fn add(a: X87, b: X87, subtract: bool) -> X87 {
    if let Some(result) = special_operands(a, b) {
        return result;
    }
    let a_negative = a.is_negative();
    let b_negative = b.is_negative() != subtract;
    match (a.class(), b.class()) {
        | (Class::Infinite, Class::Infinite) if a_negative != b_negative => X87::INDEFINITE,
        | (Class::Infinite, _) => X87::infinity(a_negative),
        | (_, Class::Infinite) => X87::infinity(b_negative),
        // C99 Annex F.8.3: `-0 + -0` is the only sum of zeros that is `-0`
        // when rounding to nearest.
        | (Class::Zero, Class::Zero) => X87::zero(a_negative && b_negative),
        | (
            Class::Zero,
            Class::Finite {
                exponent,
                significand,
            },
        ) => X87::rounded(b_negative, exponent, u128::from(significand)),
        | (
            Class::Finite {
                exponent,
                significand,
            },
            Class::Zero,
        ) => X87::rounded(a_negative, exponent, u128::from(significand)),
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
            // 62 guard bits below each significand and two above leave room
            // for the carry of a sum; an operand shifted out of the guard
            // bits only matters through its sticky bit.
            let (mut high, mut low) = ((a_exponent, a_negative, a), (b_exponent, b_negative, b));
            if high.0 < low.0 {
                std::mem::swap(&mut high, &mut low);
            }
            let exponent = high.0 - 62;
            let high_magnitude = u128::from(high.2) << 62;
            let difference = (high.0 - low.0).unsigned_abs();
            let low_magnitude = u128::from(low.2) << 62;
            let low_magnitude = if difference >= 128 {
                1
            } else {
                let shifted = low_magnitude >> difference;
                shifted | u128::from(shifted << difference != low_magnitude)
            };
            if high.1 == low.1 {
                X87::rounded(high.1, exponent, high_magnitude + low_magnitude)
            } else {
                match high_magnitude.cmp(&low_magnitude) {
                    | Ordering::Greater =>
                        X87::rounded(high.1, exponent, high_magnitude - low_magnitude),
                    | Ordering::Less =>
                        X87::rounded(low.1, exponent, low_magnitude - high_magnitude),
                    // An exact zero difference is +0 when rounding to
                    // nearest (IEC 60559 §6.3).
                    | Ordering::Equal => X87::zero(false),
                }
            }
        },
        | _ => unreachable!("special operands were handled"),
    }
}

/// The result of a two-operand operation that an invalid encoding or a
/// NaN decides: the real indefinite for an invalid encoding, else the NaN
/// the processor propagates, quieted.
fn special_operands(a: X87, b: X87) -> Option<X87> {
    let (a_class, b_class) = (a.class(), b.class());
    if a_class == Class::Unsupported || b_class == Class::Unsupported {
        return Some(X87::INDEFINITE);
    }
    Some(match (a_class, b_class) {
        | (
            Class::Nan {
                signaling: a_signals,
            },
            Class::Nan {
                signaling: b_signals,
            },
        ) =>
            if a_signals == b_signals {
                larger_nan(a, b).quieted()
            } else if a_signals {
                b
            } else {
                a
            },
        | (Class::Nan { .. }, _) => a.quieted(),
        | (_, Class::Nan { .. }) => b.quieted(),
        | _ => return None,
    })
}

/// Of two NaNs of the same kind, the one with the larger significand; for
/// equal significands, the positive one, else the second.
fn larger_nan(a: X87, b: X87) -> X87 {
    let (a_payload, b_payload) = (a.significand | 1 << 62, b.significand | 1 << 62);
    let take_a = match a_payload.cmp(&b_payload) {
        | Ordering::Greater => true,
        | Ordering::Less => false,
        | Ordering::Equal => !a.is_negative(),
    };
    if take_a { a } else { b }
}

/// A nonzero significand shifted so its top bit is set, with its exponent.
const fn normalize(exponent: i32, significand: u64) -> (i32, u64) {
    let shift = significand.leading_zeros();
    (exponent - shift as i32, significand << shift)
}

/// A key that orders the magnitudes of comparable values: `(0, 0)` for
/// zero, the leading bit's biased exponent and the normalized significand
/// for finite values, and a key above every finite one for infinity.
fn magnitude(value: X87) -> Option<(i32, u64)> {
    match value.class() {
        | Class::Zero => Some((0, 0)),
        | Class::Finite {
            exponent,
            significand,
        } => {
            let (exponent, significand) = normalize(exponent, significand);
            Some((exponent + 63 + super::BIAS + 64, significand))
        },
        | Class::Infinite => Some((i32::MAX, 0)),
        | Class::Nan { .. } | Class::Unsupported => None,
    }
}
