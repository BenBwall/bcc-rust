//! C99 real math functions for the default interpreter host.
//! Native libm computes f32/f64 results in their own precision. Target long
//! double uses that target's IR representation: binary64 goes to native libm,
//! and x87 `f80` to the software x87 model in `math/extended.rs`, which
//! supports the functions with exact results. Guest pointers are never passed
//! to native library code.
//! C99: §7.12.4-§7.12.13, pp. 218-239; PDF pp. 230-251.

use super::{
    Fault,
    HostCall,
    Memory,
    RuntimeValue,
    Type,
    UbKind,
    Unsupported,
};
use crate::ir::Align;

// The x87 long double functions and the errno they set
mod extended;
mod extended_errno;

/// Calls a known math function after validating its complete C signature:
/// its value and the `errno` it sets, or zero. glibc's `math_errhandling`
/// is `MATH_ERRNO | MATH_ERREXCEPT`: a domain error sets `EDOM`, and a pole
/// error or a range error `ERANGE`. Native libm reports its own `errno` for
/// `float` and `double` (glibc's, on GNU/Linux); the x87 functions follow
/// glibc's wrappers' rules (`math/extended_errno.rs`).
/// C99: §7.12.1 paragraphs 2-4, pp. 214-215; PDF pp. 226-227.
pub(super) fn call(
    call: HostCall<'_>,
    memory: &mut Memory<'_>,
    long_double: Type,
    long_width: u32,
    text: &mut crate::util::bump::ArenaVec<'_, u8>,
) -> Option<Result<(RuntimeValue, i32), Fault>> {
    let (operation, precision) = lookup(call.name, long_double)?;
    Some(evaluate(
        call,
        memory,
        operation,
        precision,
        long_double,
        long_width,
        text,
    ))
}

fn evaluate(
    call: HostCall<'_>,
    memory: &mut Memory<'_>,
    operation: Operation,
    precision: Type,
    long_double: Type,
    long_width: u32,
    text: &mut crate::util::bump::ArenaVec<'_, u8>,
) -> Result<(RuntimeValue, i32), Fault> {
    super::check_signature(call, long_width, long_double)?;
    let long = if long_width == 32 {
        Type::I32
    } else {
        Type::I64
    };
    if call.args.len() != call.arg_types.len() {
        return Err(UbKind::SignatureMismatch.into());
    }
    if call.args.iter().any(|value| value.is_poison()) {
        return Err(UbKind::PoisonInHostCall.into());
    }
    if operation == Operation::Nexttoward && long_double == Type::F80 && precision != Type::F80 {
        let value = extended::next_toward_extended(call.args[0], call.args[1])?;
        return Ok((
            value,
            extended_errno::next_toward(call.args[0], call.args[1], value),
        ));
    }
    if operation == Operation::Nan {
        text.clear();
        memory.read_c_string(call.args[0], text)?;
        text.push(0);
    }
    match precision {
        | Type::F32 => {
            let (value, code) =
                native::with_errno(|| single(operation, call.args, memory, long, text));
            Ok((value?, code))
        },
        | Type::F64 => {
            let (value, code) =
                native::with_errno(|| double(operation, call.args, memory, long, text));
            Ok((value?, code))
        },
        | Type::F80 => {
            let value = extended::extended(operation, call.args, memory, long, text)?;
            Ok((value, extended_errno::errno(operation, call.args, value)))
        },
        | _ => Err(Fault::Unsupported(Unsupported::WideFloat)),
    }
}

// Generate the lookup and exhaustive test inventory from the same entries.
macro_rules! math_functions {
    ($long_double:ident; $(| $name:literal => $value:expr,)*) => {
        fn lookup(name: &str, $long_double: Type) -> Option<(Operation, Type)> {
            Some(match name {
                $($name => $value,)*
                _ => return None,
            })
        }
        #[cfg(test)]
        const NAMES: &[&str] = &[$($name,)*];
    };
}

math_functions! { long_double;
        | "acos" => (Operation::Acos, Type::F64),
        | "acosf" => (Operation::Acos, Type::F32),
        | "acosl" => (Operation::Acos, long_double),
        | "asin" => (Operation::Asin, Type::F64),
        | "asinf" => (Operation::Asin, Type::F32),
        | "asinl" => (Operation::Asin, long_double),
        | "atan" => (Operation::Atan, Type::F64),
        | "atanf" => (Operation::Atan, Type::F32),
        | "atanl" => (Operation::Atan, long_double),
        | "cos" => (Operation::Cos, Type::F64),
        | "cosf" => (Operation::Cos, Type::F32),
        | "cosl" => (Operation::Cos, long_double),
        | "sin" => (Operation::Sin, Type::F64),
        | "sinf" => (Operation::Sin, Type::F32),
        | "sinl" => (Operation::Sin, long_double),
        | "tan" => (Operation::Tan, Type::F64),
        | "tanf" => (Operation::Tan, Type::F32),
        | "tanl" => (Operation::Tan, long_double),
        | "acosh" => (Operation::Acosh, Type::F64),
        | "acoshf" => (Operation::Acosh, Type::F32),
        | "acoshl" => (Operation::Acosh, long_double),
        | "asinh" => (Operation::Asinh, Type::F64),
        | "asinhf" => (Operation::Asinh, Type::F32),
        | "asinhl" => (Operation::Asinh, long_double),
        | "atanh" => (Operation::Atanh, Type::F64),
        | "atanhf" => (Operation::Atanh, Type::F32),
        | "atanhl" => (Operation::Atanh, long_double),
        | "cosh" => (Operation::Cosh, Type::F64),
        | "coshf" => (Operation::Cosh, Type::F32),
        | "coshl" => (Operation::Cosh, long_double),
        | "sinh" => (Operation::Sinh, Type::F64),
        | "sinhf" => (Operation::Sinh, Type::F32),
        | "sinhl" => (Operation::Sinh, long_double),
        | "tanh" => (Operation::Tanh, Type::F64),
        | "tanhf" => (Operation::Tanh, Type::F32),
        | "tanhl" => (Operation::Tanh, long_double),
        | "exp" => (Operation::Exp, Type::F64),
        | "expf" => (Operation::Exp, Type::F32),
        | "expl" => (Operation::Exp, long_double),
        | "exp2" => (Operation::Exp2, Type::F64),
        | "exp2f" => (Operation::Exp2, Type::F32),
        | "exp2l" => (Operation::Exp2, long_double),
        | "expm1" => (Operation::Expm1, Type::F64),
        | "expm1f" => (Operation::Expm1, Type::F32),
        | "expm1l" => (Operation::Expm1, long_double),
        | "log" => (Operation::Log, Type::F64),
        | "logf" => (Operation::Log, Type::F32),
        | "logl" => (Operation::Log, long_double),
        | "log10" => (Operation::Log10, Type::F64),
        | "log10f" => (Operation::Log10, Type::F32),
        | "log10l" => (Operation::Log10, long_double),
        | "log1p" => (Operation::Log1p, Type::F64),
        | "log1pf" => (Operation::Log1p, Type::F32),
        | "log1pl" => (Operation::Log1p, long_double),
        | "log2" => (Operation::Log2, Type::F64),
        | "log2f" => (Operation::Log2, Type::F32),
        | "log2l" => (Operation::Log2, long_double),
        | "logb" => (Operation::Logb, Type::F64),
        | "logbf" => (Operation::Logb, Type::F32),
        | "logbl" => (Operation::Logb, long_double),
        | "cbrt" => (Operation::Cbrt, Type::F64),
        | "cbrtf" => (Operation::Cbrt, Type::F32),
        | "cbrtl" => (Operation::Cbrt, long_double),
        | "fabs" => (Operation::Fabs, Type::F64),
        | "fabsf" => (Operation::Fabs, Type::F32),
        | "fabsl" => (Operation::Fabs, long_double),
        | "sqrt" => (Operation::Sqrt, Type::F64),
        | "sqrtf" => (Operation::Sqrt, Type::F32),
        | "sqrtl" => (Operation::Sqrt, long_double),
        | "erf" => (Operation::Erf, Type::F64),
        | "erff" => (Operation::Erf, Type::F32),
        | "erfl" => (Operation::Erf, long_double),
        | "erfc" => (Operation::Erfc, Type::F64),
        | "erfcf" => (Operation::Erfc, Type::F32),
        | "erfcl" => (Operation::Erfc, long_double),
        | "lgamma" => (Operation::Lgamma, Type::F64),
        | "lgammaf" => (Operation::Lgamma, Type::F32),
        | "lgammal" => (Operation::Lgamma, long_double),
        | "tgamma" => (Operation::Tgamma, Type::F64),
        | "tgammaf" => (Operation::Tgamma, Type::F32),
        | "tgammal" => (Operation::Tgamma, long_double),
        | "ceil" => (Operation::Ceil, Type::F64),
        | "ceilf" => (Operation::Ceil, Type::F32),
        | "ceill" => (Operation::Ceil, long_double),
        | "floor" => (Operation::Floor, Type::F64),
        | "floorf" => (Operation::Floor, Type::F32),
        | "floorl" => (Operation::Floor, long_double),
        | "nearbyint" => (Operation::Nearbyint, Type::F64),
        | "nearbyintf" => (Operation::Nearbyint, Type::F32),
        | "nearbyintl" => (Operation::Nearbyint, long_double),
        | "rint" => (Operation::Rint, Type::F64),
        | "rintf" => (Operation::Rint, Type::F32),
        | "rintl" => (Operation::Rint, long_double),
        | "round" => (Operation::Round, Type::F64),
        | "roundf" => (Operation::Round, Type::F32),
        | "roundl" => (Operation::Round, long_double),
        | "trunc" => (Operation::Trunc, Type::F64),
        | "truncf" => (Operation::Trunc, Type::F32),
        | "truncl" => (Operation::Trunc, long_double),
        | "atan2" => (Operation::Atan2, Type::F64),
        | "atan2f" => (Operation::Atan2, Type::F32),
        | "atan2l" => (Operation::Atan2, long_double),
        | "hypot" => (Operation::Hypot, Type::F64),
        | "hypotf" => (Operation::Hypot, Type::F32),
        | "hypotl" => (Operation::Hypot, long_double),
        | "pow" => (Operation::Pow, Type::F64),
        | "powf" => (Operation::Pow, Type::F32),
        | "powl" => (Operation::Pow, long_double),
        | "fmod" => (Operation::Fmod, Type::F64),
        | "fmodf" => (Operation::Fmod, Type::F32),
        | "fmodl" => (Operation::Fmod, long_double),
        | "remainder" => (Operation::Remainder, Type::F64),
        | "remainderf" => (Operation::Remainder, Type::F32),
        | "remainderl" => (Operation::Remainder, long_double),
        | "copysign" => (Operation::Copysign, Type::F64),
        | "copysignf" => (Operation::Copysign, Type::F32),
        | "copysignl" => (Operation::Copysign, long_double),
        | "nextafter" => (Operation::Nextafter, Type::F64),
        | "nextafterf" => (Operation::Nextafter, Type::F32),
        | "nextafterl" => (Operation::Nextafter, long_double),
        | "fdim" => (Operation::Fdim, Type::F64),
        | "fdimf" => (Operation::Fdim, Type::F32),
        | "fdiml" => (Operation::Fdim, long_double),
        | "fmax" => (Operation::Fmax, Type::F64),
        | "fmaxf" => (Operation::Fmax, Type::F32),
        | "fmaxl" => (Operation::Fmax, long_double),
        | "fmin" => (Operation::Fmin, Type::F64),
        | "fminf" => (Operation::Fmin, Type::F32),
        | "fminl" => (Operation::Fmin, long_double),
        | "frexp" => (Operation::Frexp, Type::F64),
        | "frexpf" => (Operation::Frexp, Type::F32),
        | "frexpl" => (Operation::Frexp, long_double),
        | "ilogb" => (Operation::Ilogb, Type::F64),
        | "ilogbf" => (Operation::Ilogb, Type::F32),
        | "ilogbl" => (Operation::Ilogb, long_double),
        | "ldexp" => (Operation::Ldexp, Type::F64),
        | "ldexpf" => (Operation::Ldexp, Type::F32),
        | "ldexpl" => (Operation::Ldexp, long_double),
        | "modf" => (Operation::Modf, Type::F64),
        | "modff" => (Operation::Modf, Type::F32),
        | "modfl" => (Operation::Modf, long_double),
        | "scalbn" => (Operation::Scalbn, Type::F64),
        | "scalbnf" => (Operation::Scalbn, Type::F32),
        | "scalbnl" => (Operation::Scalbn, long_double),
        | "scalbln" => (Operation::Scalbln, Type::F64),
        | "scalblnf" => (Operation::Scalbln, Type::F32),
        | "scalblnl" => (Operation::Scalbln, long_double),
        | "lrint" => (Operation::Lrint, Type::F64),
        | "lrintf" => (Operation::Lrint, Type::F32),
        | "lrintl" => (Operation::Lrint, long_double),
        | "llrint" => (Operation::Llrint, Type::F64),
        | "llrintf" => (Operation::Llrint, Type::F32),
        | "llrintl" => (Operation::Llrint, long_double),
        | "lround" => (Operation::Lround, Type::F64),
        | "lroundf" => (Operation::Lround, Type::F32),
        | "lroundl" => (Operation::Lround, long_double),
        | "llround" => (Operation::Llround, Type::F64),
        | "llroundf" => (Operation::Llround, Type::F32),
        | "llroundl" => (Operation::Llround, long_double),
        | "remquo" => (Operation::Remquo, Type::F64),
        | "remquof" => (Operation::Remquo, Type::F32),
        | "remquol" => (Operation::Remquo, long_double),
        | "nan" => (Operation::Nan, Type::F64),
        | "nanf" => (Operation::Nan, Type::F32),
        | "nanl" => (Operation::Nan, long_double),
        | "nexttoward" => (Operation::Nexttoward, Type::F64),
        | "nexttowardf" => (Operation::Nexttoward, Type::F32),
        | "nexttowardl" => (Operation::Nexttoward, long_double),
        | "fma" => (Operation::Fma, Type::F64),
        | "fmaf" => (Operation::Fma, Type::F32),
        | "fmal" => (Operation::Fma, long_double),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Operation {
    Acos,
    Asin,
    Atan,
    Cos,
    Sin,
    Tan,
    Acosh,
    Asinh,
    Atanh,
    Cosh,
    Sinh,
    Tanh,
    Exp,
    Exp2,
    Expm1,
    Log,
    Log10,
    Log1p,
    Log2,
    Logb,
    Cbrt,
    Fabs,
    Sqrt,
    Erf,
    Erfc,
    Lgamma,
    Tgamma,
    Ceil,
    Floor,
    Nearbyint,
    Rint,
    Round,
    Trunc,
    Atan2,
    Hypot,
    Pow,
    Fmod,
    Remainder,
    Copysign,
    Nextafter,
    Fdim,
    Fmax,
    Fmin,
    Frexp,
    Ilogb,
    Ldexp,
    Modf,
    Scalbn,
    Scalbln,
    Lrint,
    Llrint,
    Lround,
    Llround,
    Remquo,
    Nan,
    Nexttoward,
    Fma,
}

fn single(
    operation: Operation,
    args: &[RuntimeValue],
    memory: &mut Memory<'_>,
    long: Type,
    text: &[u8],
) -> Result<RuntimeValue, Fault> {
    let float = |index: usize| -> Result<f32, Fault> {
        match args.get(index) {
            | Some(RuntimeValue::F32(value)) => Ok(*value),
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
    let value = match operation {
        | Operation::Acos => native::acosf(float(0)?),
        | Operation::Asin => native::asinf(float(0)?),
        | Operation::Atan => native::atanf(float(0)?),
        | Operation::Cos => native::cosf(float(0)?),
        | Operation::Sin => native::sinf(float(0)?),
        | Operation::Tan => native::tanf(float(0)?),
        | Operation::Acosh => native::acoshf(float(0)?),
        | Operation::Asinh => native::asinhf(float(0)?),
        | Operation::Atanh => native::atanhf(float(0)?),
        | Operation::Cosh => native::coshf(float(0)?),
        | Operation::Sinh => native::sinhf(float(0)?),
        | Operation::Tanh => native::tanhf(float(0)?),
        | Operation::Exp => native::expf(float(0)?),
        | Operation::Exp2 => native::exp2f(float(0)?),
        | Operation::Expm1 => native::expm1f(float(0)?),
        | Operation::Log => native::logf(float(0)?),
        | Operation::Log10 => native::log10f(float(0)?),
        | Operation::Log1p => native::log1pf(float(0)?),
        | Operation::Log2 => native::log2f(float(0)?),
        | Operation::Logb => native::logbf(float(0)?),
        | Operation::Cbrt => native::cbrtf(float(0)?),
        | Operation::Fabs => native::fabsf(float(0)?),
        | Operation::Sqrt => native::sqrtf(float(0)?),
        | Operation::Erf => native::erff(float(0)?),
        | Operation::Erfc => native::erfcf(float(0)?),
        | Operation::Lgamma => native::lgammaf(float(0)?),
        | Operation::Tgamma => native::tgammaf(float(0)?),
        | Operation::Ceil => native::ceilf(float(0)?),
        | Operation::Floor => native::floorf(float(0)?),
        | Operation::Nearbyint => native::nearbyintf(float(0)?),
        | Operation::Rint => native::rintf(float(0)?),
        | Operation::Round => native::roundf(float(0)?),
        | Operation::Trunc => native::truncf(float(0)?),
        | Operation::Atan2 => native::atan2f(float(0)?, float(1)?),
        | Operation::Hypot => native::hypotf(float(0)?, float(1)?),
        | Operation::Pow => native::powf(float(0)?, float(1)?),
        | Operation::Fmod => native::fmodf(float(0)?, float(1)?),
        | Operation::Remainder => native::remainderf(float(0)?, float(1)?),
        | Operation::Copysign => native::copysignf(float(0)?, float(1)?),
        | Operation::Nextafter => native::nextafterf(float(0)?, float(1)?),
        | Operation::Fdim => native::fdimf(float(0)?, float(1)?),
        | Operation::Fmax => native::fmaxf(float(0)?, float(1)?),
        | Operation::Fmin => native::fminf(float(0)?, float(1)?),
        | Operation::Fma => native::fmaf(float(0)?, float(1)?, float(2)?),
        | Operation::Frexp => {
            let mut exponent = 0;
            let value = native::frexpf(float(0)?, &mut exponent);
            memory.store(
                Type::I32,
                RuntimeValue::int(Type::I32, exponent as u128),
                args[1],
                Align::BYTE,
            )?;
            value
        },
        | Operation::Modf => {
            let mut integral = 0.0;
            let value = native::modff(float(0)?, &mut integral);
            memory.store(Type::F32, RuntimeValue::F32(integral), args[1], Align::BYTE)?;
            value
        },
        | Operation::Remquo => {
            let mut quotient = 0;
            let value = native::remquof(float(0)?, float(1)?, &mut quotient);
            memory.store(
                Type::I32,
                RuntimeValue::int(Type::I32, quotient as u128),
                args[2],
                Align::BYTE,
            )?;
            value
        },
        | Operation::Ldexp | Operation::Scalbn | Operation::Scalbln => {
            let ty = if operation == Operation::Scalbln {
                long
            } else {
                Type::I32
            };
            // Exponents beyond int already overflow/underflow every supported
            // floating format. Clamping avoids narrowing a target LP64 long
            // through the host's possibly LLP64 C interface.
            let exponent = int(1, ty)?.clamp(i128::from(i32::MIN), i128::from(i32::MAX));
            let exponent = i32::try_from(exponent).map_err(|_| UbKind::SignatureMismatch)?;
            native::scalbnf(float(0)?, exponent)
        },
        | Operation::Ilogb =>
            return Ok(RuntimeValue::int(
                Type::I32,
                native::ilogbf(float(0)?) as u128,
            )),
        | Operation::Lrint | Operation::Llrint => {
            let ty = if operation == Operation::Lrint {
                long
            } else {
                Type::I64
            };
            return Ok(RuntimeValue::int(ty, native::llrintf(float(0)?) as u128));
        },
        | Operation::Lround | Operation::Llround => {
            let ty = if operation == Operation::Lround {
                long
            } else {
                Type::I64
            };
            return Ok(RuntimeValue::int(ty, native::llroundf(float(0)?) as u128));
        },
        | Operation::Nan => native::nanf(text),
        | Operation::Nexttoward => {
            let x = float(0)?;
            let Some(RuntimeValue::F64(y)) = args.get(1) else {
                return Err(UbKind::PoisonInHostCall.into());
            };
            // C99 nexttoward returns y when x == y, including signed zero.
            #[expect(
                clippy::float_cmp,
                reason = "nexttoward requires exact numeric equality, not a tolerance."
            )]
            if f64::from(x) == *y || y.is_nan() {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "C99 nexttowardf returns float, including signed zero and NaN."
                )]
                {
                    *y as f32
                }
            } else {
                native::nextafterf(
                    x,
                    if f64::from(x) < *y {
                        f32::INFINITY
                    } else {
                        f32::NEG_INFINITY
                    },
                )
            }
        },
    };
    Ok(RuntimeValue::F32(value))
}

fn double(
    operation: Operation,
    args: &[RuntimeValue],
    memory: &mut Memory<'_>,
    long: Type,
    text: &[u8],
) -> Result<RuntimeValue, Fault> {
    let float = |index: usize| -> Result<f64, Fault> {
        match args.get(index) {
            | Some(RuntimeValue::F64(value)) => Ok(*value),
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
    let value = match operation {
        | Operation::Acos => native::acos(float(0)?),
        | Operation::Asin => native::asin(float(0)?),
        | Operation::Atan => native::atan(float(0)?),
        | Operation::Cos => native::cos(float(0)?),
        | Operation::Sin => native::sin(float(0)?),
        | Operation::Tan => native::tan(float(0)?),
        | Operation::Acosh => native::acosh(float(0)?),
        | Operation::Asinh => native::asinh(float(0)?),
        | Operation::Atanh => native::atanh(float(0)?),
        | Operation::Cosh => native::cosh(float(0)?),
        | Operation::Sinh => native::sinh(float(0)?),
        | Operation::Tanh => native::tanh(float(0)?),
        | Operation::Exp => native::exp(float(0)?),
        | Operation::Exp2 => native::exp2(float(0)?),
        | Operation::Expm1 => native::expm1(float(0)?),
        | Operation::Log => native::log(float(0)?),
        | Operation::Log10 => native::log10(float(0)?),
        | Operation::Log1p => native::log1p(float(0)?),
        | Operation::Log2 => native::log2(float(0)?),
        | Operation::Logb => native::logb(float(0)?),
        | Operation::Cbrt => native::cbrt(float(0)?),
        | Operation::Fabs => native::fabs(float(0)?),
        | Operation::Sqrt => native::sqrt(float(0)?),
        | Operation::Erf => native::erf(float(0)?),
        | Operation::Erfc => native::erfc(float(0)?),
        | Operation::Lgamma => native::lgamma(float(0)?),
        | Operation::Tgamma => native::tgamma(float(0)?),
        | Operation::Ceil => native::ceil(float(0)?),
        | Operation::Floor => native::floor(float(0)?),
        | Operation::Nearbyint => native::nearbyint(float(0)?),
        | Operation::Rint => native::rint(float(0)?),
        | Operation::Round => native::round(float(0)?),
        | Operation::Trunc => native::trunc(float(0)?),
        | Operation::Atan2 => native::atan2(float(0)?, float(1)?),
        | Operation::Hypot => native::hypot(float(0)?, float(1)?),
        | Operation::Pow => native::pow(float(0)?, float(1)?),
        | Operation::Fmod => native::fmod(float(0)?, float(1)?),
        | Operation::Remainder => native::remainder(float(0)?, float(1)?),
        | Operation::Copysign => native::copysign(float(0)?, float(1)?),
        | Operation::Nextafter | Operation::Nexttoward => native::nextafter(float(0)?, float(1)?),
        | Operation::Fdim => native::fdim(float(0)?, float(1)?),
        | Operation::Fmax => native::fmax(float(0)?, float(1)?),
        | Operation::Fmin => native::fmin(float(0)?, float(1)?),
        | Operation::Fma => native::fma(float(0)?, float(1)?, float(2)?),
        | Operation::Frexp => {
            let mut exponent = 0;
            let value = native::frexp(float(0)?, &mut exponent);
            memory.store(
                Type::I32,
                RuntimeValue::int(Type::I32, exponent as u128),
                args[1],
                Align::BYTE,
            )?;
            value
        },
        | Operation::Modf => {
            let mut integral = 0.0;
            let value = native::modf(float(0)?, &mut integral);
            memory.store(Type::F64, RuntimeValue::F64(integral), args[1], Align::BYTE)?;
            value
        },
        | Operation::Remquo => {
            let mut quotient = 0;
            let value = native::remquo(float(0)?, float(1)?, &mut quotient);
            memory.store(
                Type::I32,
                RuntimeValue::int(Type::I32, quotient as u128),
                args[2],
                Align::BYTE,
            )?;
            value
        },
        | Operation::Ldexp | Operation::Scalbn | Operation::Scalbln => {
            let ty = if operation == Operation::Scalbln {
                long
            } else {
                Type::I32
            };
            // Exponents beyond int already overflow/underflow every supported
            // floating format. Clamping avoids narrowing a target LP64 long
            // through the host's possibly LLP64 C interface.
            let exponent = int(1, ty)?.clamp(i128::from(i32::MIN), i128::from(i32::MAX));
            let exponent = i32::try_from(exponent).map_err(|_| UbKind::SignatureMismatch)?;
            native::scalbn(float(0)?, exponent)
        },
        | Operation::Ilogb =>
            return Ok(RuntimeValue::int(
                Type::I32,
                native::ilogb(float(0)?) as u128,
            )),
        | Operation::Lrint | Operation::Llrint => {
            let ty = if operation == Operation::Lrint {
                long
            } else {
                Type::I64
            };
            return Ok(RuntimeValue::int(ty, native::llrint(float(0)?) as u128));
        },
        | Operation::Lround | Operation::Llround => {
            let ty = if operation == Operation::Lround {
                long
            } else {
                Type::I64
            };
            return Ok(RuntimeValue::int(ty, native::llround(float(0)?) as u128));
        },
        | Operation::Nan => native::nan(text),
    };
    Ok(RuntimeValue::F64(value))
}

// Safe scalar interfaces; pointer outputs always refer to Rust temporaries,
// never to guest memory. Library state (errno/fenv) is the native host's.
pub(super) mod native {
    use std::ffi::{
        c_char,
        c_int,
        c_longlong,
    };

    // Rust references in the output signatures have C pointer ABI. They
    // guarantee writable temporary storage before crossing this boundary.
    macro_rules! library {
        ($($name:ident($($arg:ident: $ty:ty),*) -> $result:ty;)*) => {
            mod ffi {
                use super::{c_int, c_longlong};
                #[cfg_attr(unix, link(name = "m"))]
                unsafe extern "C" {
                    $(pub(super) fn $name($($arg: $ty),*) -> $result;)*
                }
            }
            $(pub(in crate::backend::interpreter::host) fn $name($($arg: $ty),*) -> $result {
                // SAFETY: Scalars have their declared C types; any output
                // reference names writable Rust storage, never guest memory.
                unsafe { ffi::$name($($arg),*) }
            })*
        };
    }

    library! {
        acosf(x: f32) -> f32;
        asinf(x: f32) -> f32;
        atanf(x: f32) -> f32;
        cosf(x: f32) -> f32;
        sinf(x: f32) -> f32;
        tanf(x: f32) -> f32;
        acoshf(x: f32) -> f32;
        asinhf(x: f32) -> f32;
        atanhf(x: f32) -> f32;
        coshf(x: f32) -> f32;
        sinhf(x: f32) -> f32;
        tanhf(x: f32) -> f32;
        expf(x: f32) -> f32;
        exp2f(x: f32) -> f32;
        expm1f(x: f32) -> f32;
        logf(x: f32) -> f32;
        log10f(x: f32) -> f32;
        log1pf(x: f32) -> f32;
        log2f(x: f32) -> f32;
        logbf(x: f32) -> f32;
        cbrtf(x: f32) -> f32;
        fabsf(x: f32) -> f32;
        sqrtf(x: f32) -> f32;
        erff(x: f32) -> f32;
        erfcf(x: f32) -> f32;
        lgammaf(x: f32) -> f32;
        tgammaf(x: f32) -> f32;
        ceilf(x: f32) -> f32;
        floorf(x: f32) -> f32;
        nearbyintf(x: f32) -> f32;
        rintf(x: f32) -> f32;
        roundf(x: f32) -> f32;
        truncf(x: f32) -> f32;
        atan2f(x: f32, y: f32) -> f32;
        hypotf(x: f32, y: f32) -> f32;
        powf(x: f32, y: f32) -> f32;
        fmodf(x: f32, y: f32) -> f32;
        remainderf(x: f32, y: f32) -> f32;
        copysignf(x: f32, y: f32) -> f32;
        nextafterf(x: f32, y: f32) -> f32;
        fdimf(x: f32, y: f32) -> f32;
        fmaxf(x: f32, y: f32) -> f32;
        fminf(x: f32, y: f32) -> f32;
        fmaf(x: f32, y: f32, z: f32) -> f32;
        frexpf(x: f32, out: &mut c_int) -> f32;
        modff(x: f32, out: &mut f32) -> f32;
        remquof(x: f32, y: f32, out: &mut c_int) -> f32;
        scalbnf(x: f32, n: c_int) -> f32;
        ilogbf(x: f32) -> c_int;
        llrintf(x: f32) -> c_longlong;
        llroundf(x: f32) -> c_longlong;
        acos(x: f64) -> f64;
        asin(x: f64) -> f64;
        atan(x: f64) -> f64;
        cos(x: f64) -> f64;
        sin(x: f64) -> f64;
        tan(x: f64) -> f64;
        acosh(x: f64) -> f64;
        asinh(x: f64) -> f64;
        atanh(x: f64) -> f64;
        cosh(x: f64) -> f64;
        sinh(x: f64) -> f64;
        tanh(x: f64) -> f64;
        exp(x: f64) -> f64;
        exp2(x: f64) -> f64;
        expm1(x: f64) -> f64;
        log(x: f64) -> f64;
        log10(x: f64) -> f64;
        log1p(x: f64) -> f64;
        log2(x: f64) -> f64;
        logb(x: f64) -> f64;
        cbrt(x: f64) -> f64;
        fabs(x: f64) -> f64;
        sqrt(x: f64) -> f64;
        erf(x: f64) -> f64;
        erfc(x: f64) -> f64;
        lgamma(x: f64) -> f64;
        tgamma(x: f64) -> f64;
        ceil(x: f64) -> f64;
        floor(x: f64) -> f64;
        nearbyint(x: f64) -> f64;
        rint(x: f64) -> f64;
        round(x: f64) -> f64;
        trunc(x: f64) -> f64;
        atan2(x: f64, y: f64) -> f64;
        hypot(x: f64, y: f64) -> f64;
        pow(x: f64, y: f64) -> f64;
        fmod(x: f64, y: f64) -> f64;
        remainder(x: f64, y: f64) -> f64;
        copysign(x: f64, y: f64) -> f64;
        nextafter(x: f64, y: f64) -> f64;
        fdim(x: f64, y: f64) -> f64;
        fmax(x: f64, y: f64) -> f64;
        fmin(x: f64, y: f64) -> f64;
        fma(x: f64, y: f64, z: f64) -> f64;
        frexp(x: f64, out: &mut c_int) -> f64;
        modf(x: f64, out: &mut f64) -> f64;
        remquo(x: f64, y: f64, out: &mut c_int) -> f64;
        scalbn(x: f64, n: c_int) -> f64;
        ilogb(x: f64) -> c_int;
        llrint(x: f64) -> c_longlong;
        llround(x: f64) -> c_longlong;
    }

    #[cfg_attr(unix, link(name = "m"))]
    unsafe extern "C" {
        #[link_name = "nan"]
        fn native_nan(tag: *const c_char) -> f64;
        #[link_name = "nanf"]
        fn native_nanf(tag: *const c_char) -> f32;
    }

    /// Runs `operation`, which calls the host's C library, and returns its
    /// result with the `errno` it left (zero if it set none). On GNU/Linux
    /// the host's library is glibc, whose math functions and conversions
    /// set `errno` as the program's would.
    pub(in crate::backend::interpreter::host) fn with_errno<T>(
        operation: impl FnOnce() -> T,
    ) -> (T, c_int) {
        let location = errno_location();
        // SAFETY: The C library's `errno` is this thread's, valid for as
        // long as the thread runs.
        unsafe { location.write(0) };
        let result = operation();
        // SAFETY: As above.
        (result, unsafe { location.read() })
    }

    /// The address of this thread's C `errno`.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn errno_location() -> *mut c_int {
        // SAFETY: `__errno_location` has no preconditions.
        unsafe { libc::__errno_location() }
    }

    #[cfg(target_vendor = "apple")]
    fn errno_location() -> *mut c_int {
        // SAFETY: `__error` has no preconditions.
        unsafe { libc::__error() }
    }

    #[cfg(windows)]
    fn errno_location() -> *mut c_int {
        unsafe extern "C" {
            fn _errno() -> *mut c_int;
        }
        // SAFETY: The C runtime's `_errno` has no preconditions.
        unsafe { _errno() }
    }

    pub(super) fn nan(tag: &[u8]) -> f64 {
        debug_assert_eq!(tag.last(), Some(&0), "host tags have a terminating null");
        // SAFETY: The host copied the complete tag and appended its null.
        unsafe { native_nan(tag.as_ptr().cast()) }
    }

    pub(super) fn nanf(tag: &[u8]) -> f32 {
        debug_assert_eq!(tag.last(), Some(&0), "host tags have a terminating null");
        // SAFETY: The host copied the complete tag and appended its null.
        unsafe { native_nanf(tag.as_ptr().cast()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backend::interpreter::host::HostVaList,
        util::bump::{
            ArenaVec,
            Bump,
        },
    };

    #[test]
    fn every_host_math_name_rejects_malformed_signatures() {
        for (long_width, long_double) in [(64, Type::F80), (32, Type::F64)] {
            for &name in NAMES {
                let signature = crate::builtins::external_signature(name)
                    .unwrap_or_else(|| panic!("missing host math signature: {name}"));
                let arena = Bump::new();
                let mut params = ArenaVec::new_in(&arena);
                params.extend(
                    signature
                        .parameters
                        .iter()
                        .map(|&p| super::super::external_type(p, long_width, long_double).unwrap()),
                );
                let result = super::super::external_type(signature.result, long_width, long_double);
                let mut values = ArenaVec::new_in(&arena);
                values.resize(params.len(), RuntimeValue::Poison);
                let valid = HostCall {
                    name,
                    args: &values,
                    arg_types: &params,
                    result,
                    va_list: HostVaList::None,
                };
                assert!(
                    super::super::check_signature(valid, long_width, long_double).is_ok(),
                    "{name}"
                );
                // A wrong parameter must be diagnosed before poison operands.
                for index in 0..params.len() {
                    let arena = Bump::new();
                    let mut wrong = ArenaVec::new_in(&arena);
                    wrong.extend_from_slice(&params);
                    wrong[index] = Type::I128;
                    let mut memory = Memory::new(&arena, 1024);
                    let mut text = ArenaVec::new_in(&arena);
                    assert!(
                        matches!(
                            call(
                                HostCall {
                                    arg_types: &wrong,
                                    ..valid
                                },
                                &mut memory,
                                long_double,
                                long_width,
                                &mut text
                            ),
                            Some(Err(Fault::Ub(UbKind::SignatureMismatch)))
                        ),
                        "{name} parameter {index}"
                    );
                }
                // Run the actual dispatch with malformed calls, before
                // wide-float refusal, pointer access or native
                // libm can hide a mismatch.
                for malformed in [
                    HostCall {
                        result: Some(Type::Ptr),
                        ..valid
                    },
                    HostCall {
                        args: &[],
                        arg_types: &[],
                        ..valid
                    },
                ] {
                    // nan takes one pointer: the wrong result also covers it;
                    // every host math operation has at least one parameter.
                    let mut memory = Memory::new(&arena, 1024);
                    let mut text = ArenaVec::new_in(&arena);
                    assert!(
                        matches!(
                            call(malformed, &mut memory, long_double, long_width, &mut text),
                            Some(Err(Fault::Ub(UbKind::SignatureMismatch)))
                        ),
                        "{name}"
                    );
                }
            }
        }
    }

    #[test]
    fn unknown_signature_fails_closed() {
        let call = HostCall {
            name:      "unknown_math",
            args:      &[],
            arg_types: &[],
            result:    Some(Type::F64),
            va_list:   HostVaList::None,
        };
        assert!(matches!(
            super::super::check_signature(call, 64, Type::F80),
            Err(Fault::Ub(UbKind::SignatureMismatch))
        ));
    }
}
