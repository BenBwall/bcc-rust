//! The x87 `long double` math functions, bit for bit as glibc's x86-64
//! versions compute them.
//!
//! glibc builds each function either from x87 instructions or from C code
//! over `long double` arithmetic, and each file here follows one family's
//! code step by step on the software x87 model (`crate::x87`), including
//! how it treats unnormals, pseudo-denormals, pseudo-infinities and
//! pseudo-NaNs. Where glibc uses an instruction the model computes exactly
//! (`frndint`, `fscale`, `fprem`, `fsqrt`), the model gives its result;
//! where the instruction's result comes from the processor's microcode
//! (`f2xm1`, `fyl2x`, `fyl2xp1`, `fpatan`), the host's own x87 unit runs
//! it, so the exponentials, logarithms, powers, arctangents and the
//! functions built on them give the bits glibc gives on the same
//! processor, and trap as unsupported on a host without one. A public name
//! that glibc wraps for SVID error handling (`w_*l_compat.c`) returns the
//! wrapper's value where it differs from the function's; `errno` is not
//! set here.
//!
//! Reading order: [`extended`] dispatches; `exact.rs` holds the exact
//! functions on the representation's words, `trigonometric.rs` (with the
//! table in `sincos_table.rs`) the sine, cosine and tangent, `inverse.rs`
//! their inverses, `exponential.rs` the exponentials, logarithms and
//! powers, `hyperbolic.rs` the hyperbolic functions and their inverses,
//! `error_function.rs` `erfl` and `erfcl`, `gamma.rs` (with
//! `gamma_negative.rs`) `lgammal` and `tgammal`, and `algebraic.rs` `cbrtl`,
//! `hypotl` and `fmal`. `processor.rs` runs the microcoded instructions
//! and models `fxam` and the comparisons, and `expression.rs` gives C's
//! comparisons and conversions on the model. `tests.rs` compares every
//! function with the host's glibc.
//! C99: §7.12.4-§7.12.13, pp. 218-239; PDF pp. 230-251.

// Function families
mod algebraic;
mod error_function;
mod exact;
mod exponential;
mod gamma;
mod gamma_negative;
mod hyperbolic;
mod inverse;
mod sincos_table;
mod trigonometric;

// The processor and C's expressions
mod expression;
mod processor;

// Tests against the host's glibc
#[cfg(test)]
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
mod tests;

use exact::{
    frexp,
    minimum_maximum,
    modf,
    next_after,
    quiet_nan,
    remquo,
    round,
    round_to_integer,
};
use exponential::{
    Base,
    Logarithm,
};
pub(super) use gamma::lgamma;

use super::{
    Fault,
    Memory,
    Operation,
    RuntimeValue,
    Type,
    UbKind,
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
        | Operation::Sin => trigonometric::sin(float(0)?),
        | Operation::Cos => trigonometric::cos(float(0)?),
        | Operation::Tan => trigonometric::tan(float(0)?),
        | Operation::Asin => inverse::asin(float(0)?),
        | Operation::Acos => inverse::acos(float(0)?)?,
        | Operation::Atan => inverse::atan(float(0)?)?,
        | Operation::Atan2 => inverse::atan2(float(0)?, float(1)?)?,
        | Operation::Exp => exponential::exp(float(0)?, Base::E)?,
        | Operation::Exp2 => exponential::exp(float(0)?, Base::Two)?,
        | Operation::Expm1 => exponential::expm1(float(0)?)?,
        | Operation::Log => exponential::log(float(0)?, Logarithm::Natural)?,
        | Operation::Log2 => exponential::log(float(0)?, Logarithm::Two)?,
        | Operation::Log10 => exponential::log(float(0)?, Logarithm::Ten)?,
        | Operation::Log1p => exponential::log1p(float(0)?)?,
        | Operation::Pow => exponential::pow(float(0)?, float(1)?)?,
        | Operation::Sinh => hyperbolic::sinh(float(0)?)?,
        | Operation::Cosh => hyperbolic::cosh(float(0)?)?,
        | Operation::Tanh => hyperbolic::tanh(float(0)?)?,
        | Operation::Asinh => hyperbolic::asinh(float(0)?)?,
        | Operation::Acosh => hyperbolic::acosh(float(0)?)?,
        | Operation::Atanh => hyperbolic::atanh(float(0)?)?,
        | Operation::Cbrt => algebraic::cbrt(float(0)?),
        | Operation::Hypot => algebraic::hypot(float(0)?, float(1)?),
        | Operation::Fma => algebraic::fma(float(0)?, float(1)?, float(2)?),
        | Operation::Erf => error_function::erf(float(0)?)?,
        | Operation::Erfc => error_function::erfc(float(0)?)?,
        | Operation::Lgamma => lgamma(float(0)?)?.0,
        | Operation::Tgamma => gamma::tgamma(float(0)?)?,
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
