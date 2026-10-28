//! Floating constants as target bits: the folded values semantic analysis keeps
//! are host `long double`s, or binary128 values for binary128 constants.
//! Initializer checks and lowering need the exact bits of an `f32`, `f64`,
//! `f80` or `f128` constant.
//!
//! An x87 extended value is 80 bits: a 64-bit significand with an explicit
//! integer bit, a 15-bit biased exponent and a sign. [`constant_bits`]
//! decodes the host carrier through [`LongDouble::x87_parts`], rounds it to the
//! nearest value of the destination format (ties to even, as C99 Annex F
//! requires of conversions), and returns the bits `fconst` takes: the
//! destination format's own bits, with `f80` as the significand in the low
//! 64 bits and the sign and exponent above them. Semantic analysis already
//! rounded each folded value to its expression's type, so a conversion to
//! that type is exact; rounding matters only when an initializer converts a
//! constant to a narrower type.
//!
//! [`from_integer`] and [`to_integer`] convert between integers and
//! floating bits the same way, so integer constants fold into `fconst`
//! and floating constants into static integer initializers.
//!
//! C99: §6.3.1.4-§6.3.1.5, pp. 43-44; PDF pp. 55-56 (real floating and integer
//! conversions); §F.3 paragraph 1, pp. 445-447; PDF pp. 457-459.

use crate::{
    binary128::Binary128,
    float_parsing::LongDouble,
    half_precision::HalfFormat,
    ir::Type,
    x87::X87,
};

/// The exponent bias of an x87 extended value.
const X87_BIAS: i64 = 16383;

/// The sign, biased exponent and significand of an x87 value, if the
/// carrier holds a supported host representation.
fn parts(value: &LongDouble) -> Option<(bool, u16, u64)> {
    value.x87_parts()
}

/// The bits of `value` as a constant of the floating type `ty`, or `None`
/// for any other type or an unsupported host carrier.
pub(crate) fn constant_bits(value: &LongDouble, ty: Type) -> Option<u128> {
    constant_parts(parts(value)?, ty)
}

/// Converts decoded carrier parts to the target format.
/// C99: §6.3.1.5, p. 44; PDF p. 56.
pub(crate) fn constant_parts(
    (sign, exponent, significand): (bool, u16, u64),
    ty: Type,
) -> Option<u128> {
    let (mantissa_bits, exponent_bits) = match ty {
        | Type::F16 => (10, 5),
        | Type::BF16 => (7, 8),
        | Type::F32 => (23, 8),
        | Type::F64 => (52, 11),
        | Type::F80 => {
            return Some(
                u128::from(significand) | u128::from(u16::from(sign) << 15 | exponent) << 64,
            );
        },
        // Binary128 holds every x87 value exactly.
        | Type::F128 => {
            let x87 = u128::from(significand) | u128::from(u16::from(sign) << 15 | exponent) << 64;
            return Some(Binary128::from_x87(X87::from_bits(x87)).to_bits());
        },
        | _ => return None,
    };
    let format = Format {
        mantissa_bits,
        exponent_bits,
    };
    if exponent == 0x7FFF {
        // Infinity has only the integer bit; anything else is a NaN, which
        // keeps the top of its fraction, the quiet bit included, so a
        // signaling NaN stays one. A NaN whose payload a narrower format
        // cannot hold becomes quiet (C99 §F.3, conversions).
        let nan = significand << 1 != 0;
        let payload = if nan {
            let fraction = (significand << 1) >> (64 - mantissa_bits);
            if fraction == 0 {
                1 << (mantissa_bits - 1)
            } else {
                fraction
            }
        } else {
            0
        };
        return Some(format.signed(sign, format.infinity() | u128::from(payload)));
    }
    // A denormal or zero has exponent 1 for the scale of its significand.
    let scale = i64::from(exponent.max(1)) - X87_BIAS - 63;
    Some(format.signed(sign, format.round(u128::from(significand), scale)))
}

/// The bits of the integer `value` as a constant of the floating type `ty`,
/// rounded to nearest even; `None` for a type that is not `f32`, `f64` or
/// `f80`.
/// C99: §6.3.1.4 paragraph 2, pp. 43-44; PDF pp. 55-56.
pub(crate) fn from_integer(value: i128, ty: Type) -> Option<u128> {
    from_magnitude(value < 0, value.unsigned_abs(), ty)
}

/// Converts an unsigned magnitude without interpreting its high bit as a sign.
/// C99: §6.3.1.4 paragraph 2, pp. 43-44; PDF pp. 55-56.
pub(crate) fn from_unsigned(value: u128, ty: Type) -> Option<u128> {
    from_magnitude(false, value, ty)
}

fn from_magnitude(sign: bool, magnitude: u128, ty: Type) -> Option<u128> {
    let (mantissa_bits, exponent_bits) = match ty {
        | Type::F16 => (10, 5),
        | Type::BF16 => (7, 8),
        | Type::F32 => (23, 8),
        | Type::F64 => (52, 11),
        | Type::F80 => return Some(x87_from_magnitude(sign, magnitude)),
        | Type::F128 => return Some(Binary128::from_integer(sign, magnitude).to_bits()),
        | _ => return None,
    };
    let format = Format {
        mantissa_bits,
        exponent_bits,
    };
    Some(format.signed(sign, format.round(magnitude, 0)))
}

/// The x87 bits of an unsigned 128-bit magnitude, rounding the significand to
/// 64 bits.
#[expect(
    clippy::cast_possible_truncation,
    reason = "The halves of a 128-bit value are taken apart deliberately."
)]
fn x87_from_magnitude(sign: bool, magnitude: u128) -> u128 {
    if magnitude == 0 {
        return u128::from(sign) << 79;
    }
    let shift = magnitude.leading_zeros();
    let normal = magnitude << shift;
    // The leading bit is bit 127 of `normal`, so its value is 2^(127 - shift).
    let mut exponent = 127 - i64::from(shift);
    let mut significand = (normal >> 64) as u64;
    let rest = normal as u64;
    if rest > 1 << 63 || (rest == 1 << 63 && significand & 1 == 1) {
        let (rounded, carry) = significand.overflowing_add(1);
        significand = if carry { 1 << 63 } else { rounded };
        exponent += i64::from(carry);
    }
    let biased =
        u128::from(u16::try_from(exponent + X87_BIAS).unwrap_or(u16::MAX) | u16::from(sign) << 15);
    u128::from(significand) | biased << 64
}

/// The bits of the integer of `width` bits and the given signedness that a
/// floating constant truncates to (C99 §6.3.1.4 paragraph 1), or `None` for
/// a NaN, an infinity or a value out of the destination's range.
pub(crate) fn to_integer(value: &LongDouble, width: u32, signed: bool) -> Option<u128> {
    let (sign, exponent, significand) = parts(value)?;
    let bits = u128::from(significand) | u128::from(u16::from(sign) << 15 | exponent) << 64;
    X87::from_bits(bits).to_integer(signed, width)
}

/// The bits of the binary128 constant `value` converted to the floating
/// type `ty`, rounded to nearest even as libgcc's conversions round; `None`
/// for any other type.
/// C99: §6.3.1.5 paragraph 2, p. 44; PDF p. 56.
pub(crate) fn binary128_bits(value: Binary128, ty: Type) -> Option<u128> {
    Some(match ty {
        | Type::F16 => u128::from(HalfFormat::Binary16.round_binary128(value)),
        | Type::BF16 => u128::from(HalfFormat::BFloat16.round_binary128(value)),
        | Type::F32 => u128::from(value.to_f32().to_bits()),
        | Type::F64 => u128::from(value.to_f64().to_bits()),
        | Type::F80 => value.to_x87().to_bits(),
        | Type::F128 => value.to_bits(),
        | _ => return None,
    })
}

/// The bits of the integer of `width` bits and the given signedness that a
/// binary128 constant truncates to (C99 §6.3.1.4 paragraph 1), or `None`
/// for a NaN, an infinity or a value out of the destination's range.
pub(crate) fn binary128_to_integer(value: Binary128, width: u32, signed: bool) -> Option<u128> {
    value.to_integer(signed, width)
}

/// The bytes of a floating constant in memory: little-endian, `size` bytes,
/// zero padded past the format's own width.
pub(crate) fn bytes(bits: u128, size: usize) -> impl Iterator<Item = u8> {
    bits.to_le_bytes().into_iter().take(size)
}

/// An IEEE binary format, for rounding a binary significand into it.
#[derive(Clone, Copy)]
struct Format {
    mantissa_bits: u32,
    exponent_bits: u32,
}

impl Format {
    fn bias(self) -> i64 {
        (1 << (self.exponent_bits - 1)) - 1
    }

    /// The bits of infinity, without the sign.
    fn infinity(self) -> u128 {
        ((1_u128 << self.exponent_bits) - 1) << self.mantissa_bits
    }

    fn signed(self, sign: bool, bits: u128) -> u128 {
        bits | u128::from(sign) << (self.mantissa_bits + self.exponent_bits)
    }

    /// The bits, without the sign, of `significand * 2^scale` rounded to
    /// nearest even: zero, a subnormal, a normal value or infinity.
    fn round(self, significand: u128, scale: i64) -> u128 {
        if significand == 0 {
            return 0;
        }
        let shift = significand.leading_zeros();
        let normal = significand << shift;
        // The leading bit is bit 127 of `normal`: its value is 2^leading.
        let leading = scale - i64::from(shift) + 127;
        let precision = i64::from(self.mantissa_bits) + 1;
        let smallest = 1 - self.bias();
        // A subnormal keeps fewer bits the further below the smallest
        // normal exponent it is.
        let keep = if leading >= smallest {
            precision
        } else {
            precision - (smallest - leading)
        };
        if keep < 0 {
            return 0;
        }
        let dropped = 128 - keep;
        let (mut kept, rest, half) = if dropped == 128 {
            (0, normal, 1_u128 << 127)
        } else {
            (
                normal >> dropped,
                normal & ((1_u128 << dropped) - 1),
                1_u128 << (dropped - 1),
            )
        };
        if rest > half || (rest == half && kept & 1 == 1) {
            kept += 1;
        }
        let bits = if leading >= smallest {
            // `kept` carries the implicit bit, which adds one to the
            // exponent field; a carry out of the significand adds one more.
            (((leading + self.bias() - 1) as u128) << self.mantissa_bits) + kept
        } else {
            kept
        };
        bits.min(self.infinity())
    }
}
