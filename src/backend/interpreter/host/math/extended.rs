//! The x87 `long double` math functions, computed by the software x87 model
//! wherever glibc's x86-64 result is exact or correctly rounded. The
//! transcendental functions, whose glibc results depend on its own
//! polynomial approximations, and `fmal` stay unsupported and trap.
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
        | Operation::Round => float(0)?.round_integral(Integral::HalfAway),
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
        | Operation::Nextafter | Operation::Nexttoward => float(0)?.next_toward(float(1)?),
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
            let (fraction, exponent) = float(0)?.frexp();
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
            let x = float(0)?;
            let integral = x.round_integral(Integral::Truncate);
            let fraction = if integral.compare(integral).is_none() {
                integral
            } else if x.compare(integral).is_some_and(std::cmp::Ordering::is_eq) {
                X87::zero(x.is_negative())
            } else {
                x.sub(integral)
            };
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
        | Operation::Lround => return integer(long, Integral::HalfAway),
        | Operation::Llround => return integer(Type::I64, Integral::HalfAway),
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

/// glibc's x86-64 `fmaxl` and `fminl`: of two ordered values, the larger
/// (smaller), and for equal ones the second (first); a quiet NaN gives the
/// other operand, and a signaling one the propagated NaN.
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
            let signals =
                |value: X87| matches!(value.class(), crate::x87::Class::Nan { signaling: true });
            let nan = |value: X87| value.compare(value).is_none();
            match (nan(x), nan(y)) {
                | (true, false) if !signals(x) => y,
                | (false, true) if !signals(y) => x,
                | _ => x.add(y),
            }
        },
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
