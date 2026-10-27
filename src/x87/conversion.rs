//! Rounding an exact magnitude into a format, and the conversions between
//! x87 values, integers and the IEEE binary formats.
//!
//! Every operation hands [`round`] its exact result, or one whose discarded
//! low bits are summarized by a sticky bit below the rounding position.
//! Rounding is to nearest, ties to even, at the format's precision; a
//! magnitude below the smallest normal is first denormalized, so a denormal
//! result is rounded once at its own, smaller precision.

use super::{
    BIAS,
    Class,
    INTEGER_BIT,
    MAX_BIASED,
    QUIET_BIT,
    X87,
};

/// A binary floating format's precision and exponent range.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Binary {
    /// Significand bits, including the leading (hidden or explicit) bit.
    precision:    u32,
    /// The exponent of the leading bit of the smallest normal value.
    min_exponent: i32,
    /// The exponent of the leading bit of the largest finite value.
    max_exponent: i32,
}

impl Binary {
    /// bfloat16, GCC's and Clang's `__bf16`: binary32's exponent range with
    /// eight significand bits.
    pub(crate) const BFLOAT: Self = Self {
        precision:    8,
        min_exponent: -126,
        max_exponent: 127,
    };
    /// IEEE binary64, C's `double`.
    pub(super) const DOUBLE: Self = Self {
        precision:    53,
        min_exponent: -1022,
        max_exponent: 1023,
    };
    /// x87 double-extended: 64 significand bits.
    pub(super) const EXTENDED: Self = Self {
        precision:    64,
        min_exponent: 1 - BIAS,
        max_exponent: BIAS,
    };
    /// IEEE binary16, C23's `_Float16`.
    pub(crate) const HALF: Self = Self {
        precision:    11,
        min_exponent: -14,
        max_exponent: 15,
    };
    /// IEEE binary32, C's `float`.
    pub(super) const SINGLE: Self = Self {
        precision:    24,
        min_exponent: -126,
        max_exponent: 127,
    };

    /// The total width of an IEEE interchange encoding of this format.
    const fn width(self) -> u32 {
        match self.precision {
            | 8 | 11 => 16,
            | 24 => 32,
            | _ => 64,
        }
    }

    const fn bias(self) -> i32 {
        self.max_exponent
    }
}

/// A magnitude rounded into a format.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Rounded {
    Zero,
    /// `significand × 2^quantum`, with `significand < 2^precision`; normal
    /// when its leading bit is at `precision - 1`, else denormal.
    Finite {
        quantum:     i32,
        significand: u64,
    },
    Infinite,
}

/// Rounds `magnitude × 2^exponent` into `format`, to nearest with ties to
/// even. A nonzero bit below the rounding position may stand for any
/// nonzero remainder (a sticky bit).
/// C99: §5.2.4.2.2 paragraph 7, p. 24; PDF p. 36 (`FLT_ROUNDS` 1).
pub(super) fn round(format: Binary, exponent: i32, magnitude: u128) -> Rounded {
    if magnitude == 0 {
        return Rounded::Zero;
    }
    let leading_zeros = magnitude.leading_zeros();
    let normalized = magnitude << leading_zeros;
    // `normalized × 2^exponent` is the value, with its leading bit at 127.
    let exponent = exponent - leading_zeros as i32;
    let precision = format.precision as i32;
    let leading = exponent + 127;
    let mut quantum = (leading - (precision - 1)).max(format.min_exponent - (precision - 1));
    // At least 128 - precision bits are dropped.
    let shift = quantum - exponent;
    let (mut significand, half, sticky) = match shift {
        | 129.. => (0, false, true),
        | 128 => (0, normalized >> 127 != 0, normalized << 1 != 0),
        | _ => {
            let shift = shift.unsigned_abs();
            (
                normalized >> shift,
                (normalized >> (shift - 1)) & 1 != 0,
                normalized & ((1 << (shift - 1)) - 1) != 0,
            )
        },
    };
    if half && (sticky || significand & 1 != 0) {
        significand += 1;
        if significand == 1 << precision {
            significand >>= 1;
            quantum += 1;
        }
    }
    if significand == 0 {
        Rounded::Zero
    } else if quantum + precision - 1 > format.max_exponent {
        Rounded::Infinite
    } else {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "The significand has at most 64 bits."
        )]
        let significand = significand as u64;
        Rounded::Finite {
            quantum,
            significand,
        }
    }
}

impl X87 {
    /// The x87 value nearest `magnitude × 2^exponent` with the given sign.
    pub(crate) fn rounded(negative: bool, exponent: i32, magnitude: u128) -> Self {
        match round(Binary::EXTENDED, exponent, magnitude) {
            | Rounded::Zero => Self::zero(negative),
            | Rounded::Infinite => Self::infinity(negative),
            | Rounded::Finite {
                quantum,
                significand,
            } => {
                let biased = if significand & INTEGER_BIT == 0 {
                    0
                } else {
                    quantum + 63 + BIAS
                };
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "Rounding checked the exponent range."
                )]
                let biased = biased as u16;
                Self::from_parts(negative, biased, significand)
            },
        }
    }

    /// The value of a signed or unsigned integer, as `fild` gives it for
    /// up to 64 bits; wider integers round to nearest.
    /// C99: §6.3.1.4 paragraph 2, p. 43; PDF p. 55.
    pub(crate) fn from_integer(negative: bool, magnitude: u128) -> Self {
        Self::rounded(negative, 0, magnitude)
    }

    /// The value truncated toward zero as an integer of `bits` bits, or
    /// `None` for a NaN, an infinity, an invalid encoding or a value out of
    /// range. The result is the integer's two's-complement bits.
    /// C99: §6.3.1.4 paragraph 1, p. 43; PDF p. 55.
    pub(crate) fn to_integer(self, signed: bool, bits: u32) -> Option<u128> {
        let magnitude = match self.class() {
            | Class::Zero => 0,
            | Class::Finite {
                exponent,
                significand,
            } => {
                let significand = u128::from(significand);
                match exponent {
                    | ..=-64 => 0,
                    | ..0 => significand >> exponent.unsigned_abs(),
                    | _ if significand.leading_zeros() < exponent.unsigned_abs() => return None,
                    | _ => significand << exponent,
                }
            },
            | _ => return None,
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

    /// The exact value of an IEEE `format` encoding, as `fld` loads it: a
    /// signaling NaN is quieted, keeping its payload in the high fraction
    /// bits.
    /// C99: §6.3.1.5 paragraph 1, p. 44; PDF p. 56.
    pub(crate) fn from_binary(format: Binary, bits: u64) -> Self {
        let width = format.width();
        let fraction_bits = format.precision - 1;
        let negative = (bits >> (width - 1)) & 1 != 0;
        let max_biased = (1 << (width - fraction_bits - 1)) - 1;
        let biased = (bits >> fraction_bits) & max_biased;
        let fraction = bits & ((1 << fraction_bits) - 1);
        if biased == max_biased {
            return if fraction == 0 {
                Self::infinity(negative)
            } else {
                let payload = fraction << (63 - fraction_bits);
                Self::from_parts(negative, MAX_BIASED, INTEGER_BIT | QUIET_BIT | payload)
            };
        }
        let offset = i64::from(format.bias());
        let (exponent, significand) = if biased == 0 {
            (1 - offset, fraction)
        } else {
            #[expect(clippy::cast_possible_wrap, reason = "The exponent field is small.")]
            let biased = biased as i64;
            (biased - offset, fraction | 1 << fraction_bits)
        };
        #[expect(
            clippy::cast_possible_truncation,
            reason = "The exponent field is small."
        )]
        let exponent = (exponent - i64::from(fraction_bits)) as i32;
        Self::rounded(negative, exponent, u128::from(significand))
    }

    pub(crate) fn from_f64(value: f64) -> Self {
        Self::from_binary(Binary::DOUBLE, value.to_bits())
    }

    pub(crate) fn from_f32(value: f32) -> Self {
        Self::from_binary(Binary::SINGLE, u64::from(value.to_bits()))
    }

    /// The IEEE `format` encoding nearest this value, as `fstp` stores it: a
    /// NaN is quieted and keeps the high bits of its payload, and an invalid
    /// encoding gives the format's default NaN, whose sign bit is set.
    /// C99: §6.3.1.5 paragraph 2, p. 44; PDF p. 56.
    pub(crate) fn to_binary(self, format: Binary) -> u64 {
        let width = format.width();
        let fraction_bits = format.precision - 1;
        let sign = u64::from(self.is_negative()) << (width - 1);
        let max_biased = (1_u64 << (width - fraction_bits - 1)) - 1;
        let infinity = max_biased << fraction_bits;
        let quiet = 1 << (fraction_bits - 1);
        match self.class() {
            | Class::Zero => sign,
            | Class::Infinite => sign | infinity,
            | Class::Nan { .. } => {
                let payload = (self.significand & !INTEGER_BIT) >> (63 - fraction_bits);
                sign | infinity | quiet | payload
            },
            | Class::Unsupported => (1 << (width - 1)) | infinity | quiet,
            | Class::Finite {
                exponent,
                significand,
            } => encode(
                format,
                self.is_negative(),
                round(format, exponent, u128::from(significand)),
            ),
        }
    }

    pub(crate) fn to_f64(self) -> f64 {
        f64::from_bits(self.to_binary(Binary::DOUBLE))
    }

    pub(crate) fn to_f32(self) -> f32 {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "A binary32 encoding has 32 bits."
        )]
        let bits = self.to_binary(Binary::SINGLE) as u32;
        f32::from_bits(bits)
    }
}

/// The IEEE binary32 (`single`) or binary64 encoding nearest the exact
/// value `magnitude × 2^exponent` with the given sign, rounded once, as
/// `strtof` and `strtod` read a hexadecimal floating constant. Bits below
/// the rounding position may be summarized by a sticky bit.
/// C99: §7.20.1.3 paragraph 8, p. 309; PDF p. 321.
pub(crate) fn nearest_binary(single: bool, negative: bool, exponent: i32, magnitude: u128) -> u64 {
    let format = if single {
        Binary::SINGLE
    } else {
        Binary::DOUBLE
    };
    nearest(format, negative, exponent, magnitude)
}

/// The IEEE `format` encoding nearest the exact value `magnitude ×
/// 2^exponent` with the given sign, rounded once.
/// C99: §6.3.1.4 paragraph 2, pp. 43-44; PDF pp. 55-56; §6.3.1.5 paragraph
/// 2, p. 44; PDF p. 56.
pub(crate) fn nearest(format: Binary, negative: bool, exponent: i32, magnitude: u128) -> u64 {
    encode(format, negative, round(format, exponent, magnitude))
}

/// The IEEE `format` encoding of a rounded magnitude with the given sign.
fn encode(format: Binary, negative: bool, rounded: Rounded) -> u64 {
    let width = format.width();
    let fraction_bits = format.precision - 1;
    let sign = u64::from(negative) << (width - 1);
    let max_biased = (1_u64 << (width - fraction_bits - 1)) - 1;
    match rounded {
        | Rounded::Zero => sign,
        | Rounded::Infinite => sign | max_biased << fraction_bits,
        | Rounded::Finite {
            quantum,
            significand,
        } => {
            let leading = 1 << fraction_bits;
            let biased = if significand & leading == 0 {
                0
            } else {
                #[expect(
                    clippy::cast_sign_loss,
                    reason = "A normal result has a positive biased exponent."
                )]
                let biased = (quantum + fraction_bits as i32 + format.bias()) as u64;
                biased
            };
            sign | biased << fraction_bits | (significand & (leading - 1))
        },
    }
}
