//! IEEE binary16 (`_Float16`) and bfloat16 (`__bf16`) values computed in
//! software: exact widening, and narrowing rounded once to nearest with
//! ties to even, bit for bit as libgcc's `__truncsfhf2`, `__truncdfhf2`,
//! `__truncxfhf2`, `__trunctfhf2` and their bfloat16 counterparts give it on
//! x86-64.
//!
//! Neither format computes on its own: GCC and Clang on x86-64 evaluate their
//! arithmetic in binary32 and round the result, so the only operations here
//! are conversions. Every narrowing reads the source's exact magnitude and
//! hands it to the x87 module's rounding core with the format's precision
//! and exponent range, so a subnormal result is rounded once at its own
//! precision and an overflow becomes an infinity. A NaN keeps its sign and
//! the top of its payload and is quieted; widening a binary16 signaling
//! NaN quiets it too, as libgcc's soft-float routines do, while a bfloat16
//! widens to binary32 by a shift, as GCC and Clang widen it inline.
//!
//! For `0.1` as a `double`, the magnitude `0x1999999999999A × 2^-56` keeps
//! its top 11 bits, `0x666`, and the discarded bits are just over half an
//! ulp, so the binary16 result is `0x2E66`, about 0.099975586.
//!
//! Read [`HalfFormat`] and its conversions; tests are in `tests.rs`.
//!
//! C99: §6.3.1.4-§6.3.1.5, pp. 43-44; PDF pp. 55-56 (conversions); §5.2.4.2.2
//! paragraphs 2-4, pp. 22-24; PDF pp. 34-36 (floating model). binary16 is a
//! C23 interchange format (Annex H, §H.2.2 paragraph 1, p. 553; PDF p. 566);
//! bfloat16 is a GNU and Clang extension (GCC manual, "Half-Precision
//! Floating Point").

use crate::{
    binary128::{
        Binary128,
        Class,
    },
    x87::{
        self,
        Binary,
        X87,
    },
};

/// A 16-bit floating format. A value is its encoding, a `u16`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(crate) enum HalfFormat {
    /// IEEE binary16: 5 exponent bits and 10 fraction bits.
    Binary16,
    /// bfloat16, the top half of a binary32 encoding: 8 exponent bits and 7
    /// fraction bits, the format of GCC's and Clang's `__bf16`.
    BFloat16,
}

impl HalfFormat {
    /// The exact binary32 value of an encoding. A binary16 signaling NaN is
    /// quieted, as `__extendhfsf2` quiets it; a bfloat16 encoding becomes
    /// the top half of the binary32 one, signaling NaNs included, as GCC
    /// and Clang widen it inline (only libgcc's `__extendbfsf2`, which GCC
    /// calls before arithmetic, quiets one, and arithmetic quiets it anyway).
    /// C99: §6.3.1.5 paragraph 1, p. 44; PDF p. 56.
    pub(crate) fn widen(self, bits: u16) -> f32 {
        match self {
            | Self::Binary16 => self.to_x87(bits).to_f32(),
            | Self::BFloat16 => f32::from_bits(u32::from(bits) << 16),
        }
    }

    /// The exact x87 value of an encoding; a signaling NaN is quieted.
    pub(crate) fn to_x87(self, bits: u16) -> X87 {
        X87::from_binary(self.binary(), u64::from(bits))
    }

    /// The exact binary128 value of an encoding; a signaling NaN is quieted.
    pub(crate) fn to_binary128(self, bits: u16) -> Binary128 {
        Binary128::from_x87(self.to_x87(bits))
    }

    /// The encoding nearest a binary32 value, as `__truncsfhf2` and
    /// `__truncsfbf2` give it.
    /// C99: §6.3.1.5 paragraph 2, p. 44; PDF p. 56.
    pub(crate) fn round_f32(self, value: f32) -> u16 {
        self.round_x87(X87::from_f32(value))
    }

    /// The encoding nearest a binary64 value, rounded once, as
    /// `__truncdfhf2` and `__truncdfbf2` give it.
    pub(crate) fn round_f64(self, value: f64) -> u16 {
        self.round_x87(X87::from_f64(value))
    }

    /// The encoding nearest an x87 value, as `__truncxfhf2` and
    /// `__truncxfbf2` give it; an invalid encoding gives the default NaN.
    pub(crate) fn round_x87(self, value: X87) -> u16 {
        narrow(u128::from(value.to_binary(self.binary())))
    }

    /// The encoding nearest a binary128 value, as `__trunctfhf2` and
    /// `__trunctfbf2` give it.
    pub(crate) fn round_binary128(self, value: Binary128) -> u16 {
        let format = self.binary();
        let fraction_bits = self.fraction_bits();
        let negative = value.is_negative();
        let sign = u16::from(negative) << 15;
        let infinity = self.infinity();
        match value.class() {
            | Class::Zero => sign,
            | Class::Infinite => sign | infinity,
            | Class::Nan { .. } => {
                let payload = value.to_bits() >> (112 - fraction_bits) & ((1 << fraction_bits) - 1);
                sign | infinity | 1 << (fraction_bits - 1) | narrow(payload)
            },
            | Class::Finite {
                exponent,
                significand,
            } => narrow(u128::from(x87::nearest(
                format,
                negative,
                exponent,
                significand,
            ))),
        }
    }

    /// The encoding nearest a signed or unsigned integer's value, rounded
    /// once however wide the integer is.
    /// C99: §6.3.1.4 paragraph 2, pp. 43-44; PDF pp. 55-56.
    pub(crate) fn round_integer(self, negative: bool, magnitude: u128) -> u16 {
        narrow(u128::from(x87::nearest(
            self.binary(),
            negative,
            0,
            magnitude,
        )))
    }

    /// The value truncated toward zero as an integer of `width` bits, or
    /// `None` for a NaN, an infinity or a value out of range.
    /// C99: §6.3.1.4 paragraph 1, p. 43; PDF p. 55.
    pub(crate) fn to_integer(self, bits: u16, signed: bool, width: u32) -> Option<u128> {
        self.to_x87(bits).to_integer(signed, width)
    }

    /// Whether an encoding is a NaN.
    #[cfg(test)]
    pub(crate) const fn is_nan(self, bits: u16) -> bool {
        bits & 0x7FFF > self.infinity()
    }

    /// The encoding of positive infinity.
    pub(crate) const fn infinity(self) -> u16 {
        match self {
            | Self::Binary16 => 0x7C00,
            | Self::BFloat16 => 0x7F80,
        }
    }

    const fn fraction_bits(self) -> u32 {
        match self {
            | Self::Binary16 => 10,
            | Self::BFloat16 => 7,
        }
    }

    const fn binary(self) -> Binary {
        match self {
            | Self::Binary16 => Binary::HALF,
            | Self::BFloat16 => Binary::BFLOAT,
        }
    }
}

/// A 16-bit encoding from the low bits of a wider carrier.
#[expect(
    clippy::cast_possible_truncation,
    reason = "Every 16-bit format's encodings fit 16 bits."
)]
const fn narrow(bits: u128) -> u16 {
    bits as u16
}

// Tests
#[cfg(test)]
mod tests;
