//! The fields of a binary128 encoding and how an operation reads them.
//!
//! An encoding is a sign bit, a 15-bit biased exponent and a 112-bit
//! fraction below a hidden leading bit. Exponent zero holds the zeros and
//! subnormals, whose hidden bit is clear and whose exponent is that of the
//! smallest normal; the all-ones exponent holds the infinities (a zero
//! fraction) and the NaNs, quiet when the top fraction bit is set.

use super::Binary128;
use crate::util::packed::Packed;

/// What an encoding means to an operation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[expect(
    variant_size_differences,
    reason = "A class is a short-lived copy taken apart at once."
)]
pub(crate) enum Class {
    Zero,
    /// A nonzero finite magnitude, `significand × 2^exponent`, normal or
    /// subnormal; a subnormal's significand is not normalized.
    Finite {
        exponent:    i32,
        significand: u128,
    },
    Infinite,
    Nan {
        signaling: bool,
    },
}

/// The exponent bias.
pub(super) const BIAS: i32 = 16383;
/// The all-ones biased exponent of infinities and NaNs.
pub(super) const MAX_BIASED: u128 = 0x7FFF;
/// The fraction's width; a significand has one more bit.
pub(super) const FRACTION_BITS: u32 = 112;
/// The hidden leading bit of a normal significand.
pub(super) const HIDDEN_BIT: u128 = 1 << FRACTION_BITS;
/// The top fraction bit, set in a quiet NaN.
pub(super) const QUIET_BIT: u128 = 1 << (FRACTION_BITS - 1);
const FRACTION_MASK: u128 = HIDDEN_BIT - 1;
const SIGN_BIT: u128 = 1 << 127;

impl Binary128 {
    /// The quiet NaN an invalid operation produces in libgcc's soft-float
    /// routines on x86-64: sign set and only the quiet bit in the fraction,
    /// as SSE's default NaN.
    pub(crate) const DEFAULT_NAN: Self = Self::from_bits(SIGN_BIT | MAX_BIASED << 112 | QUIET_BIT);

    /// The value with object representation `bits`.
    pub(crate) const fn from_bits(bits: u128) -> Self {
        Self(Packed::new(bits))
    }

    /// The 128-bit object representation.
    pub(crate) const fn to_bits(self) -> u128 {
        self.0.get()
    }

    /// How an operation reads this encoding.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "The biased exponent has fifteen bits."
    )]
    pub(crate) const fn class(self) -> Class {
        let biased = self.biased_exponent();
        let fraction = self.fraction();
        match biased {
            | 0 if fraction == 0 => Class::Zero,
            | 0 => Class::Finite {
                exponent:    1 - BIAS - FRACTION_BITS as i32,
                significand: fraction,
            },
            | MAX_BIASED if fraction == 0 => Class::Infinite,
            | MAX_BIASED => Class::Nan {
                signaling: fraction & QUIET_BIT == 0,
            },
            | _ => Class::Finite {
                exponent:    biased as i32 - BIAS - FRACTION_BITS as i32,
                significand: fraction | HIDDEN_BIT,
            },
        }
    }

    /// Whether the sign bit is set, for zeros and NaNs too.
    pub(crate) const fn is_negative(self) -> bool {
        self.to_bits() & SIGN_BIT != 0
    }

    /// Whether the encoding is a NaN.
    pub(crate) const fn is_nan(self) -> bool {
        matches!(self.class(), Class::Nan { .. })
    }

    /// Whether the value is a zero of either sign.
    pub(crate) const fn is_zero(self) -> bool {
        self.to_bits() & !SIGN_BIT == 0
    }

    /// The same encoding with the sign bit flipped, NaNs included.
    pub(crate) const fn negate(self) -> Self {
        Self::from_bits(self.to_bits() ^ SIGN_BIT)
    }

    /// The same encoding with the sign bit set to `negative`.
    pub(crate) const fn with_sign(self, negative: bool) -> Self {
        Self::from_bits(self.to_bits() & !SIGN_BIT | if negative { SIGN_BIT } else { 0 })
    }

    pub(crate) const fn zero(negative: bool) -> Self {
        Self::from_parts(negative, 0, 0)
    }

    pub(crate) const fn infinity(negative: bool) -> Self {
        Self::from_parts(negative, MAX_BIASED, 0)
    }

    /// The NaN GCC's `__builtin_nanf128` (`quiet`) or `__builtin_nansf128`
    /// builds from a tag's payload: the payload's low bits below the quiet
    /// bit; a signaling NaN whose payload is zero gets the bit below the
    /// quiet bit, so it stays a NaN.
    /// C99: §7.12.11.2 paragraph 2, p. 237; PDF p. 249
    /// (implementation-defined payload).
    pub(crate) const fn nan(quiet: bool, payload: u64) -> Self {
        let payload = payload as u128 & (QUIET_BIT - 1);
        let fraction = if quiet {
            QUIET_BIT | payload
        } else if payload == 0 {
            QUIET_BIT >> 1
        } else {
            payload
        };
        Self::from_parts(false, MAX_BIASED, fraction)
    }

    /// The NaN with the payload of this one and the quiet bit set.
    pub(super) const fn quieted(self) -> Self {
        Self::from_bits(self.to_bits() | QUIET_BIT)
    }

    pub(super) const fn from_parts(negative: bool, biased: u128, fraction: u128) -> Self {
        Self::from_bits(if negative { SIGN_BIT } else { 0 } | biased << 112 | fraction)
    }

    pub(super) const fn biased_exponent(self) -> u128 {
        (self.to_bits() >> 112) & MAX_BIASED
    }

    pub(super) const fn fraction(self) -> u128 {
        self.to_bits() & FRACTION_MASK
    }
}
