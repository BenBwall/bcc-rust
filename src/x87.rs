//! x87 double-extended values computed in software, bit for bit as an
//! x86-64 processor computes them with its default control word: 64-bit
//! precision, round to nearest even, and every exception masked.
//!
//! An [`X87`] keeps the 80-bit object representation: a sign, a 15-bit biased
//! exponent and a 64-bit significand whose top bit is the explicit integer
//! bit. [`X87::class`] reads an encoding the way the processor does; each
//! operation then computes the exact result with integer arithmetic and
//! [`conversion::round`] rounds it once into the destination format, so
//! results match the hardware without depending on the host's `long double`.
//!
//! For `1.0 / 3.0`, both operands are finite: division normalizes the two
//! significands, forms 128 quotient bits with a sticky bit for the
//! remainder, and rounding keeps the top 64, giving
//! `0x3FFD_AAAA_AAAA_AAAA_AAAB`.
//!
//! The hardware rules this follows (Intel SDM volume 1, §8.2.2 and §4.8.3):
//! - Pseudo-denormals (exponent zero, integer bit set) are valid operands with
//!   the value a denormal of the same significand has; results are never
//!   pseudo-denormal.
//! - Unnormals, pseudo-infinities and pseudo-NaNs are invalid operands, as on
//!   every processor since the 80387: arithmetic gives the default "real
//!   indefinite" NaN and comparisons are unordered.
//! - A signaling NaN operand is quieted. Of two NaN operands the result is the
//!   quiet one, or, when both are quiet or both signaling, the one with the
//!   larger significand.
//! - An invalid operation (`inf - inf`, `0 * inf`, `0 / 0`, `inf / inf`) gives
//!   the real indefinite NaN, whose sign bit is set.
//! - Underflow denormalizes before rounding; overflow rounds to infinity.
//!
//! Read [`X87`] and [`Class`], then `arithmetic.rs` for the operations and
//! `conversion.rs` for rounding and conversions to and from integers and the
//! IEEE binary formats.
//!
//! Files by role:
//! - Operations: `arithmetic.rs` (add, subtract, multiply, divide, `fmod`,
//!   square root, comparison) and `functions.rs` (the exact `<math.h>`
//!   functions: rounding to integers, scaling, remainders, neighbours).
//! - Rounding and format conversion: `conversion.rs`.
//! - Fixtures, including a cross-check against the host's x87 unit: `tests.rs`.
//!
//! C99: §5.2.4.2.2 paragraphs 2-4, pp. 22-24; PDF pp. 34-36 (floating
//! model); §6.3.1.4-§6.3.1.5, pp. 43-44; PDF pp. 55-56 (conversions); Annex
//! F.2, p. 444; PDF p. 456 (`long double` as an IEC 60559 extended format,
//! an implementation-defined choice of the x86 targets).

// Operations
mod arithmetic;
mod functions;

// Rounding and format conversion
mod conversion;

// Tests
#[cfg(test)]
mod tests;

pub(crate) use conversion::nearest_binary;
pub(crate) use functions::Integral;

/// An x87 double-extended value as its 80-bit object representation.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct X87 {
    /// The significand, with the explicit integer bit at bit 63.
    significand:   u64,
    /// The sign at bit 15 and the biased exponent below it.
    sign_exponent: u16,
}

/// What an encoding means to the processor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[expect(
    variant_size_differences,
    reason = "A class is a short-lived copy taken apart at once."
)]
pub(crate) enum Class {
    Zero,
    /// A nonzero finite magnitude, `significand × 2^exponent`, including
    /// denormals and pseudo-denormals; the significand need not be
    /// normalized.
    Finite {
        exponent:    i32,
        significand: u64,
    },
    Infinite,
    Nan {
        signaling: bool,
    },
    /// An unnormal, pseudo-infinity or pseudo-NaN.
    Unsupported,
}

/// The exponent bias.
const BIAS: i32 = 16383;
/// The all-ones biased exponent of infinities and NaNs.
const MAX_BIASED: u16 = 0x7FFF;
const INTEGER_BIT: u64 = 1 << 63;
const QUIET_BIT: u64 = 1 << 62;

impl X87 {
    /// The "real indefinite" quiet NaN an invalid operation produces.
    pub(crate) const INDEFINITE: Self = Self {
        significand:   INTEGER_BIT | QUIET_BIT,
        sign_exponent: 0x8000 | MAX_BIASED,
    };

    /// The value whose object representation is the low 80 bits of `bits`.
    pub(crate) const fn from_bits(bits: u128) -> Self {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "The fields are taken apart."
        )]
        Self {
            significand:   bits as u64,
            sign_exponent: (bits >> 64) as u16,
        }
    }

    /// The 80-bit object representation in the low bits.
    pub(crate) const fn to_bits(self) -> u128 {
        ((self.sign_exponent as u128) << 64) | self.significand as u128
    }

    /// How the processor reads this encoding.
    pub(crate) const fn class(self) -> Class {
        let biased = self.biased_exponent();
        let integer = self.significand & INTEGER_BIT != 0;
        match biased {
            | 0 if self.significand == 0 => Class::Zero,
            | 0 => Class::Finite {
                exponent:    1 - BIAS - 63,
                significand: self.significand,
            },
            | MAX_BIASED if !integer => Class::Unsupported,
            | MAX_BIASED if self.significand == INTEGER_BIT => Class::Infinite,
            | MAX_BIASED => Class::Nan {
                signaling: self.significand & QUIET_BIT == 0,
            },
            | _ if !integer => Class::Unsupported,
            | _ => Class::Finite {
                exponent:    biased as i32 - BIAS - 63,
                significand: self.significand,
            },
        }
    }

    /// Whether the sign bit is set, for zeros and NaNs too.
    pub(crate) const fn is_negative(self) -> bool {
        self.sign_exponent & 0x8000 != 0
    }

    /// The same encoding with the sign bit flipped, as `fchs` does, NaNs
    /// included.
    pub(crate) const fn negate(self) -> Self {
        Self {
            sign_exponent: self.sign_exponent ^ 0x8000,
            ..self
        }
    }

    /// The same encoding with the sign bit set to `negative`.
    pub(crate) const fn with_sign(self, negative: bool) -> Self {
        Self {
            sign_exponent: (self.sign_exponent & 0x7FFF) | if negative { 0x8000 } else { 0 },
            ..self
        }
    }

    pub(crate) const fn zero(negative: bool) -> Self {
        Self::from_parts(negative, 0, 0)
    }

    pub(crate) const fn infinity(negative: bool) -> Self {
        Self::from_parts(negative, MAX_BIASED, INTEGER_BIT)
    }

    /// The NaN with the payload of this one and the quiet bit set.
    const fn quieted(self) -> Self {
        Self {
            significand: self.significand | QUIET_BIT,
            ..self
        }
    }

    const fn from_parts(negative: bool, biased: u16, significand: u64) -> Self {
        Self {
            significand,
            sign_exponent: if negative { 0x8000 } else { 0 } | biased,
        }
    }

    const fn biased_exponent(self) -> u16 {
        self.sign_exponent & MAX_BIASED
    }
}

impl std::fmt::Debug for X87 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "X87({:#06X}_{:016X})",
            self.sign_exponent, self.significand
        )
    }
}
