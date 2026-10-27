//! The x87 `long double` math functions, computed by the software x87 model
//! wherever glibc's x86-64 result is exact or correctly rounded. The
//! transcendental functions, whose glibc results depend on its own
//! polynomial approximations, and `fmal` stay unsupported and trap.
//!
//! Where glibc implements a function with x87 instructions (`floorl`,
//! `rintl`, `scalbnl`, `fmodl`, `remainderl`, `sqrtl`), the x87 model gives
//! its result, including the real indefinite for an invalid encoding. Where
//! glibc's `sysdeps/ieee754/ldbl-96` C code works on the representation's
//! words (`frexpl`, `roundl`, `lroundl`, `nextafterl`, `modfl`), this file
//! follows that code step by step, so unnormals, pseudo-denormals and
//! pseudo-infinities give glibc's bits too.
//! C99: §7.12.6-§7.12.13, pp. 223-239; PDF pp. 235-251.

use super::{
    Fault,
    Memory,
    Operation,
    RuntimeValue,
    Type,
    UbKind,
    Unsupported,
};
use crate::{
    ir::Align,
    x87::{
        Integral,
        X87,
    },
};

/// glibc's `FP_ILOGB0` and `FP_ILOGBNAN` on x86, both `INT_MIN`.
/// C99: §7.12 paragraph 8, p. 212; PDF p. 224 (implementation-defined).
const ILOGB_SPECIAL: i32 = i32::MIN;

/// Runs a `long double` function of the x87 format.
pub(super) fn extended(
    operation: Operation,
    args: &[RuntimeValue],
    memory: &mut Memory<'_>,
    long: Type,
    text: &[u8],
) -> Result<RuntimeValue, Fault> {
    let float = |index: usize| -> Result<X87, Fault> {
        match args.get(index) {
            | Some(RuntimeValue::F80(value)) => Ok(*value),
            | _ => Err(UbKind::PoisonInHostCall.into()),
        }
    };
    let int = |index: usize, ty: Type| -> Result<i128, Fault> {
        match args.get(index) {
            | Some(RuntimeValue::Int(bits)) =>
                Ok(crate::backend::interpreter::arithmetic::sext(ty, *bits)),
            | _ => Err(UbKind::PoisonInHostCall.into()),
        }
    };
    let store_int = |memory: &mut Memory<'_>, value: i32, address: RuntimeValue| {
        memory.store(
            Type::I32,
            RuntimeValue::int(Type::I32, value as u128),
            address,
            Align::BYTE,
        )
    };
    let integer = |ty: Type, mode: Integral| -> Result<RuntimeValue, Fault> {
        Ok(RuntimeValue::Int(
            float(0)?.to_rounded_integer(mode, ty.bits()),
        ))
    };
    let value = match operation {
        | Operation::Fabs => float(0)?.with_sign(false),
        | Operation::Copysign => float(0)?.with_sign(float(1)?.is_negative()),
        | Operation::Sqrt => float(0)?.sqrt(),
        | Operation::Ceil => float(0)?.round_integral(Integral::Ceil),
        | Operation::Floor => float(0)?.round_integral(Integral::Floor),
        | Operation::Trunc => float(0)?.round_integral(Integral::Truncate),
        | Operation::Round => round(float(0)?),
        | Operation::Rint | Operation::Nearbyint => float(0)?.round_integral(Integral::HalfEven),
        | Operation::Fmod => float(0)?.rem(float(1)?),
        | Operation::Remainder => float(0)?.remainder(float(1)?),
        | Operation::Remquo => {
            let (value, quotient) = remquo(float(0)?, float(1)?);
            if let Some(quotient) = quotient {
                store_int(memory, quotient, args[2])?;
            }
            value
        },
        | Operation::Nextafter | Operation::Nexttoward => next_after(float(0)?, float(1)?),
        | Operation::Fdim => {
            let (x, y) = (float(0)?, float(1)?);
            if x.compare(y).is_some_and(std::cmp::Ordering::is_le) {
                X87::zero(false)
            } else {
                x.sub(y)
            }
        },
        | Operation::Fmax | Operation::Fmin =>
            minimum_maximum(operation == Operation::Fmin, float(0)?, float(1)?),
        | Operation::Frexp => {
            let (fraction, exponent) = frexp(float(0)?);
            store_int(memory, exponent, args[1])?;
            fraction
        },
        | Operation::Ldexp | Operation::Scalbn | Operation::Scalbln => {
            let ty = if operation == Operation::Scalbln {
                long
            } else {
                Type::I32
            };
            let power = int(1, ty)?.clamp(i128::from(i64::MIN), i128::from(i64::MAX));
            float(0)?.scale(i64::try_from(power).expect("clamped to i64"))
        },
        | Operation::Modf => {
            let (fraction, integral) = modf(float(0)?);
            memory.store(Type::F80, RuntimeValue::F80(integral), args[1], Align::BYTE)?;
            fraction
        },
        | Operation::Ilogb => {
            let x = float(0)?;
            let exponent = match x.binary_exponent() {
                | Some(exponent) => exponent,
                | None if x
                    .compare(X87::zero(false))
                    .is_none_or(std::cmp::Ordering::is_eq) =>
                    ILOGB_SPECIAL,
                | None => i32::MAX,
            };
            return Ok(RuntimeValue::int(Type::I32, exponent as u128));
        },
        | Operation::Logb => {
            let x = float(0)?;
            match x.binary_exponent() {
                | Some(exponent) =>
                    X87::from_integer(exponent < 0, u128::from(exponent.unsigned_abs())),
                | None if x
                    .compare(X87::zero(false))
                    .is_some_and(std::cmp::Ordering::is_eq) =>
                    X87::infinity(true),
                | None if x.compare(x).is_none() => x.add(x),
                | None => X87::infinity(false),
            }
        },
        | Operation::Lrint => return integer(long, Integral::HalfEven),
        | Operation::Llrint => return integer(Type::I64, Integral::HalfEven),
        | Operation::Lround =>
            return Ok(RuntimeValue::int(
                long,
                round_to_integer(float(0)?, long.bits()),
            )),
        | Operation::Llround =>
            return Ok(RuntimeValue::int(
                Type::I64,
                round_to_integer(float(0)?, 64),
            )),
        | Operation::Nan => quiet_nan(text),
        | _ => return Err(Fault::Unsupported(Unsupported::WideFloat)),
    };
    Ok(RuntimeValue::F80(value))
}

/// `nexttoward` and `nexttowardf` on an x87 target: the next value of the
/// narrower format after `x` in the direction of the `long double` `y`.
/// C99: §7.12.11.4, p. 238; PDF p. 250.
pub(super) fn next_toward_extended(
    x: RuntimeValue,
    y: RuntimeValue,
) -> Result<RuntimeValue, Fault> {
    let RuntimeValue::F80(y) = y else {
        return Err(UbKind::PoisonInHostCall.into());
    };
    let wide = match x {
        | RuntimeValue::F32(x) => X87::from_f32(x),
        | RuntimeValue::F64(x) => X87::from_f64(x),
        | _ => return Err(UbKind::PoisonInHostCall.into()),
    };
    let order = wide.compare(y);
    let narrow = |value: X87| match x {
        | RuntimeValue::F32(_) => RuntimeValue::F32(value.to_f32()),
        | _ => RuntimeValue::F64(value.to_f64()),
    };
    let Some(order) = order else {
        return Ok(narrow(wide.add(y)));
    };
    if order.is_eq() {
        return Ok(narrow(y));
    }
    let up = order.is_lt();
    Ok(match x {
        | RuntimeValue::F32(x) => RuntimeValue::F32(super::native::nextafterf(
            x,
            if up { f32::INFINITY } else { f32::NEG_INFINITY },
        )),
        | RuntimeValue::F64(x) => RuntimeValue::F64(super::native::nextafter(
            x,
            if up { f64::INFINITY } else { f64::NEG_INFINITY },
        )),
        | _ => unreachable!("checked above"),
    })
}

/// glibc's `remquol` for the x87 format (`sysdeps/ieee754/ldbl-96`), step
/// by step: the remainder modulo `8p` and then the subtractions that give
/// the low quotient bits. Its quotient may be 8 (congruent to 0), and it is
/// not stored for a zero, infinite or NaN operand.
/// C99: §7.12.10.3 paragraphs 2-3, p. 236; PDF p. 248.
fn remquo(x: X87, p: X87) -> (X87, Option<i32>) {
    let ((ex, x_significand), (ep, p_significand)) = (x.magnitude_parts(), p.magnitude_parts());
    let negative = x.is_negative();
    let quotient_negative = negative != p.is_negative();
    let p_nan = ep == 0x7FFF && p_significand & !(1 << 63) != 0;
    if (ep == 0 && p_significand == 0) || ex == 0x7FFF || p_nan {
        let product = x.mul(p);
        return (product.div(product), None);
    }
    let constant = |value: u128| X87::from_integer(false, value);
    let mut x = x;
    if ep <= 0x7FFB {
        x = x.rem(constant(8).mul(p));
    }
    if (ex, x_significand) == (ep, p_significand) {
        let quotient = if quotient_negative { -1 } else { 1 };
        return (X87::zero(false).mul(x), Some(quotient));
    }
    let mut x = x.with_sign(false);
    let p = p.with_sign(false);
    let at_least = |a: X87, b: X87| a.compare(b).is_some_and(std::cmp::Ordering::is_ge);
    let above = |a: X87, b: X87| a.compare(b).is_some_and(std::cmp::Ordering::is_gt);
    let mut quotient = 0;
    if ep <= 0x7FFC && at_least(x, constant(4).mul(p)) {
        x = x.sub(constant(4).mul(p));
        quotient += 4;
    }
    if ep <= 0x7FFD && at_least(x, constant(2).mul(p)) {
        x = x.sub(constant(2).mul(p));
        quotient += 2;
    }
    if ep < 2 {
        if above(x.add(x), p) {
            x = x.sub(p);
            quotient += 1;
            if at_least(x.add(x), p) {
                x = x.sub(p);
                quotient += 1;
            }
        }
    } else {
        let half = X87::from_f64(0.5).mul(p);
        if above(x, half) {
            x = x.sub(p);
            quotient += 1;
            if at_least(x, half) {
                x = x.sub(p);
                quotient += 1;
            }
        }
    }
    if x.class() == crate::x87::Class::Zero {
        x = X87::zero(false);
    }
    let quotient = if quotient_negative {
        -quotient
    } else {
        quotient
    };
    (if negative { x.negate() } else { x }, Some(quotient))
}

/// glibc's x86-64 `fmaxl` and `fminl` (`s_fmaxl.S`, `s_fminl.S`): of two
/// ordered values, the larger (smaller), and for equal ones the second
/// (first). When one operand is unordered with itself (a NaN or an invalid
/// encoding), its significand's bit 62 decides: set, the other operand is
/// the result; clear, as for a signaling NaN, the sum. Two such operands
/// give their sum.
fn minimum_maximum(minimum: bool, x: X87, y: X87) -> X87 {
    match x.compare(y) {
        | Some(order) if minimum =>
            if order.is_le() {
                x
            } else {
                y
            },
        | Some(order) =>
            if order.is_gt() {
                x
            } else {
                y
            },
        | None => {
            let unordered = |value: X87| value.compare(value).is_none();
            let quiet = |value: X87| value.magnitude_parts().1 & 1 << 62 != 0;
            match (unordered(x), unordered(y)) {
                | (true, false) if quiet(x) => y,
                | (false, true) if quiet(y) => x,
                | _ => x.add(y),
            }
        },
    }
}

/// The sign-and-exponent word and the high and low significand words of an
/// x87 value, as glibc's `GET_LDOUBLE_WORDS` reads them.
fn words(value: X87) -> (u16, u32, u32) {
    let bits = value.to_bits();
    let word = |shift: u32| u32::try_from((bits >> shift) & 0xFFFF_FFFF).expect("masked");
    (
        u16::try_from(bits >> 64).expect("eighty bits"),
        word(32),
        word(0),
    )
}

/// The biased exponent in a sign-and-exponent word.
const fn biased(sign_exponent: u16) -> u16 {
    sign_exponent & 0x7FFF
}

/// glibc's `SET_LDOUBLE_WORDS`.
fn from_words(sign_exponent: u16, high: u32, low: u32) -> X87 {
    X87::from_bits(u128::from(sign_exponent) << 64 | u128::from(high) << 32 | u128::from(low))
}

/// glibc's `frexpl`: the exponent field of a nonzero finite encoding is
/// replaced by that of [0.5, 1), after scaling a denormal by 2^65; zeros,
/// infinities and NaNs (any all-ones exponent) give `x + x` and exponent 0.
/// C99: §7.12.6.4, p. 225; PDF p. 237.
fn frexp(x: X87) -> (X87, i32) {
    let (sign_exponent, high, low) = words(x);
    let biased = sign_exponent & 0x7FFF;
    if biased == 0x7FFF || (biased == 0 && high == 0 && low == 0) {
        return (x.add(x), 0);
    }
    let (x, offset) = if biased == 0 {
        (x.mul(X87::from_integer(false, 1 << 65)), -65)
    } else {
        (x, 0)
    };
    let (sign_exponent, high, low) = words(x);
    let exponent = offset + i32::from(sign_exponent & 0x7FFF) - 0x3FFE;
    (
        from_words((sign_exponent & 0x8000) | 0x3FFE, high, low),
        exponent,
    )
}

/// glibc's `roundl`: halfway cases away from zero, by adding half a unit
/// to the representation's words and clearing the fraction bits.
/// C99: §7.12.9.6, p. 233; PDF p. 245.
fn round(x: X87) -> X87 {
    let (mut sign_exponent, mut high, mut low) = words(x);
    let exponent = i32::from(sign_exponent & 0x7FFF) - 0x3FFF;
    if exponent < 31 {
        if exponent < 0 {
            sign_exponent &= 0x8000;
            (high, low) = (0, 0);
            if exponent == -1 {
                sign_exponent |= 0x3FFF;
                high = 0x8000_0000;
            }
        } else {
            let fraction = 0x7FFF_FFFF >> exponent;
            if (high & fraction) | low == 0 {
                return x;
            }
            let (sum, carry) = high.overflowing_add(0x4000_0000 >> exponent);
            if carry {
                sign_exponent += 1;
            }
            high = (sum & !fraction) | 0x8000_0000;
            low = 0;
        }
    } else if exponent > 62 {
        return if exponent == 0x4000 { x.add(x) } else { x };
    } else {
        let fraction = u32::MAX >> (exponent - 31);
        if low & fraction == 0 {
            return x;
        }
        let (sum, carry) = low.overflowing_add(1 << (62 - exponent));
        if carry {
            let (next, overflow) = high.overflowing_add(1);
            high = if overflow {
                sign_exponent += 1;
                next | 0x8000_0000
            } else {
                next
            };
        }
        low = sum & !fraction;
    }
    from_words(sign_exponent, high, low)
}

/// glibc's `llroundl` (and `lroundl`, for a `bits`-bit `long`): rounding
/// halfway away from zero on the words; a value too large for that is
/// converted as `fistp` truncates it, giving the integer indefinite when it
/// does not fit or is invalid.
/// C99: §7.12.9.7, p. 234; PDF p. 246.
fn round_to_integer(x: X87, bits: u32) -> u128 {
    let (sign_exponent, high, low) = words(x);
    let mut exponent = i32::from(sign_exponent & 0x7FFF) - 0x3FFF;
    let negative = sign_exponent & 0x8000 != 0;
    let mask = u128::MAX >> (128 - bits);
    let signed = |magnitude: u128| {
        if negative {
            magnitude.wrapping_neg() & mask
        } else {
            magnitude & mask
        }
    };
    let width = i32::try_from(bits).expect("an integer width");
    if exponent < 31 {
        if exponent < 0 {
            return signed(u128::from(exponent == -1));
        }
        let mut sum = u64::from(high) + u64::from(0x4000_0000_u32 >> exponent);
        if sum > u64::from(u32::MAX) {
            sum = (sum & u64::from(u32::MAX)) >> 1 | 0x8000_0000;
            exponent += 1;
        }
        signed(u128::from(sum >> (31 - exponent)))
    } else if exponent < width - 1 {
        let (sum, carry) = low.overflowing_add(0x8000_0000 >> (exponent - 31));
        let upper = u128::from(high) + u128::from(carry);
        let magnitude = if exponent == 31 {
            upper
        } else {
            (upper << (exponent - 31)) | u128::from(sum >> (63 - exponent))
        };
        signed(magnitude)
    } else {
        x.to_integer(true, bits).unwrap_or(1 << (bits - 1))
    }
}

/// glibc's `nextafterl`: one unit up or down in the representation's
/// words, deciding the direction by comparing words; a NaN operand (any
/// all-ones exponent with fraction bits) gives `x + y`, and `x == y` gives
/// `y`.
/// C99: §7.12.11.3, p. 237; PDF p. 249.
fn next_after(x: X87, y: X87) -> X87 {
    let ((x_exponent, x_high, x_low), (y_exponent, y_high, y_low)) = (words(x), words(y));
    let nan = |exponent: u16, high: u32, low: u32| {
        exponent & 0x7FFF == 0x7FFF && ((high & 0x7FFF_FFFF) | low) != 0
    };
    if nan(x_exponent, x_high, x_low) || nan(y_exponent, y_high, y_low) {
        return x.add(y);
    }
    if x.compare(y).is_some_and(std::cmp::Ordering::is_eq) {
        return y;
    }
    if biased(x_exponent) == 0 && x_high == 0 && x_low == 0 {
        return from_words(y_exponent & 0x8000, 0, 1);
    }
    // glibc reads the sign and exponent as a signed 16-bit field.
    let (sign_x, sign_y) = (x_exponent.cast_signed(), y_exponent.cast_signed());
    let x_larger = (sign_x, x_high, x_low) > (sign_y, y_high, y_low);
    let down = if sign_x >= 0 {
        x_larger
    } else {
        sign_y >= 0 || x_larger
    };
    let (mut exponent, mut high, mut low) = (sign_x, x_high, x_low);
    if down {
        if low == 0 {
            if high <= 0x8000_0000 {
                if biased(exponent.cast_unsigned()) == 0 {
                    high = high.wrapping_sub(1);
                } else {
                    exponent -= 1;
                    high = high.wrapping_sub(1);
                    if biased(exponent.cast_unsigned()) > 0 {
                        high |= 0x8000_0000;
                    }
                }
            } else {
                high -= 1;
            }
        }
        low = low.wrapping_sub(1);
    } else {
        low = low.wrapping_add(1);
        if low == 0 {
            high = high.wrapping_add(1);
            if high == 0 || (biased(exponent.cast_unsigned()) == 0 && high == 0x8000_0000) {
                exponent += 1;
                high |= 0x8000_0000;
            }
        }
    }
    from_words(exponent.cast_unsigned(), high, low)
}

/// glibc's `modfl`: the integral part from the words with the fraction bits
/// cleared, and `x` minus it; `|x| < 1` keeps `x` and gives a zero integral
/// part, and an exponent beyond the significand gives `x * 1` and a zero
/// fraction (or `x * 1` twice for a NaN).
/// C99: §7.12.6.12, p. 227; PDF p. 239.
fn modf(x: X87) -> (X87, X87) {
    let (sign_exponent, high, low) = words(x);
    let exponent = i32::from(sign_exponent & 0x7FFF) - 0x3FFF;
    let zero = from_words(sign_exponent & 0x8000, 0, 0);
    let one = X87::from_integer(false, 1);
    if exponent < 32 {
        if exponent < 0 {
            return (x, zero);
        }
        let fraction = 0x7FFF_FFFF >> exponent;
        if (high & fraction) | low == 0 {
            return (zero, x);
        }
        let integral = from_words(sign_exponent, high & !fraction, 0);
        (x.sub(integral), integral)
    } else if exponent > 63 {
        let integral = x.mul(one);
        if exponent == 0x4000 && ((high & 0x7FFF_FFFF) | low) != 0 {
            return (x.mul(one), integral);
        }
        (zero, integral)
    } else {
        let fraction = 0x7FFF_FFFF >> (exponent - 32);
        if low & fraction == 0 {
            return (zero, x);
        }
        let integral = from_words(sign_exponent, high, low & !fraction);
        (x.sub(integral), integral)
    }
}

/// `nanl`: the quiet NaN whose payload is the integer the text spells in
/// `strtoull`'s base 0, or zero when the text is not such an integer, as
/// glibc reads `NAN(n-char-sequence)`.
/// C99: §7.12.11.2 paragraph 2, p. 237; PDF p. 249; §7.20.1.3 paragraph 3,
/// p. 308; PDF p. 320.
fn quiet_nan(text: &[u8]) -> X87 {
    let text = text.strip_suffix(&[0]).unwrap_or(text);
    let text = std::str::from_utf8(text).unwrap_or("");
    let (digits, radix) =
        if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            (hex, 16)
        } else if text.len() > 1 && text.starts_with('0') {
            (&text[1..], 8)
        } else {
            (text, 10)
        };
    let payload = u64::from_str_radix(digits, radix).unwrap_or(0);
    X87::from_bits((0x7FFF << 64) | 0xC000_0000_0000_0000 | u128::from(payload & ((1 << 62) - 1)))
}
