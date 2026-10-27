//! The `errno` glibc's x86-64 `long double` functions set, from their
//! arguments and result. Most go through glibc's SVID-compatible wrappers
//! (`math/w_*l_compat.c`), whose `__kernel_standard_l` sets `EDOM` for a
//! domain error and `ERANGE` for a pole or range error in POSIX mode; the
//! newer functions through the type-generic wrappers
//! (`math/w_*_template.c`, `math/s_ldexp_template.c`,
//! `math/s_fdim_template.c`); and a few set it themselves: `sinl`, `cosl`
//! and `tanl` for an infinity (`sysdeps/ieee754/ldbl-96`), `erfcl` when it
//! underflows to zero, and `nextafterl` and `nexttoward` when they leave the
//! normal range; `expm1l` reports an overflow through `expl`. The rest
//! (`atanl`, `tanhl`, `asinhl`, `cbrtl`, `erfl`, rounding,
//! `frexpl`, `fmal`, ...) never set it; `scalbnl` is libm's `ldexpl`. glibc
//! 2.39 and 2.43 agree.
//!
//! An encoding the processor rejects (an unnormal, pseudo-infinity or
//! pseudo-NaN) compares unordered, as a NaN does, so a wrapper's `isless`
//! and `==` tests see it as one; this file treats it as a NaN throughout.
//!
//! C99: §7.12.1 paragraphs 2-4, pp. 214-215; PDF pp. 226-227.

use std::cmp::Ordering;

use super::{
    super::errno::{
        EDOM,
        ERANGE,
    },
    Operation,
    RuntimeValue,
};
use crate::x87::{
    Class,
    X87,
};

/// The `errno` the x87 `long double` form of `operation` sets for `args`
/// and its `result`, or zero.
pub(super) fn errno(operation: Operation, args: &[RuntimeValue], result: RuntimeValue) -> i32 {
    let argument = |index: usize| match args.get(index) {
        | Some(&RuntimeValue::F80(value)) => value,
        | _ => X87::INDEFINITE,
    };
    let (x, y) = (argument(0), argument(1));
    let z = match result {
        | RuntimeValue::F80(value) => value,
        | _ => X87::INDEFINITE,
    };
    let one = X87::from_integer(false, 1);
    let domain = |error: bool| if error { EDOM } else { 0 };
    let range = |error: bool| if error { ERANGE } else { 0 };
    // An overflow or underflow from a finite argument.
    let out_of_range = |argument: X87| finite(argument) && (!finite(z) || is_zero(z));
    match operation {
        | Operation::Acos | Operation::Asin => domain(greater(magnitude(x), one)),
        | Operation::Acosh => domain(less(x, one)),
        | Operation::Atanh => match magnitude(x).compare(one) {
            | Some(Ordering::Greater) => EDOM,
            | Some(Ordering::Equal) => ERANGE,
            | _ => 0,
        },
        | Operation::Atan2 => range(is_zero(z) && !is_zero(x) && finite(y)),
        | Operation::Cosh | Operation::Sinh | Operation::Lgamma => range(!finite(z) && finite(x)),
        | Operation::Exp | Operation::Exp2 => range(out_of_range(x)),
        // `expm1l` hands an argument of 128 or more to `expl`, whose
        // wrapper reports the overflow.
        | Operation::Expm1 =>
            range(finite(x) && !less(x, X87::from_integer(false, 128)) && !finite(z)),
        | Operation::Fmod => domain((infinite(x) || is_zero(y)) && !is_nan(x) && !is_nan(y)),
        | Operation::Remainder => domain((is_zero(y) && !is_nan(x)) || (infinite(x) && !is_nan(y))),
        | Operation::Hypot => range(!finite(z) && finite(x) && finite(y)),
        | Operation::Log | Operation::Log10 | Operation::Log2 =>
            pole_or_domain(x, X87::zero(false)),
        | Operation::Log1p => pole_or_domain(x, one.negate()),
        | Operation::Sqrt => domain(less(x, X87::zero(false))),
        | Operation::Pow => {
            if finite(z) {
                range(is_zero(z) && finite(x) && !is_zero(x) && finite(y))
            } else if !finite(x) || !finite(y) {
                0
            } else if is_nan(z) {
                EDOM
            } else {
                // A pole (zero to a negative power) or an overflow.
                ERANGE
            }
        },
        | Operation::Tgamma => {
            let minus_infinity = infinite(x) && x.is_negative();
            if (finite(z) && !is_zero(z)) || !(finite(x) || minus_infinity) {
                0
            } else if is_zero(x) {
                ERANGE
            } else if less(x, X87::zero(false))
                && x.round_integral(crate::x87::Integral::Floor) == x
            {
                EDOM
            } else {
                ERANGE
            }
        },
        | Operation::Sin | Operation::Cos | Operation::Tan => domain(x.class() == Class::Infinite),
        | Operation::Erfc => range(finite(x) && greater(x, X87::zero(false)) && is_zero(z)),
        | Operation::Ilogb => range_of_ilogb(result),
        // libm's `scalbnl` is its `ldexpl` wrapper.
        | Operation::Ldexp | Operation::Scalbn | Operation::Scalbln =>
            range(finite(x) && !is_zero(x) && out_of_range(x)),
        | Operation::Fdim => range(infinite(z) && !infinite(x) && !infinite(y)),
        // From zero, both step to the least subnormal without setting it.
        | Operation::Nextafter | Operation::Nexttoward => range(
            !is_nan(x) && !is_nan(y) && !is_zero(x) && x.compare(y) != Some(Ordering::Equal) && {
                let (biased, _) = z.magnitude_parts();
                biased == 0 || biased == 0x7FFF
            },
        ),
        | _ => 0,
    }
}

/// The `errno` of `nexttoward` and `nexttowardf` on an x87 target, which
/// glibc's `s_nexttoward.c` and `s_nexttowardf.c` set to `ERANGE` when the
/// next value is infinite, subnormal or zero, unless it is a step from zero.
pub(super) fn next_toward(x: RuntimeValue, y: RuntimeValue, result: RuntimeValue) -> i32 {
    let leaves_normal_range = match (x, result) {
        | (RuntimeValue::F32(x), RuntimeValue::F32(next)) =>
            !x.is_nan() && x != 0.0 && !next.is_normal(),
        | (RuntimeValue::F64(x), RuntimeValue::F64(next)) =>
            !x.is_nan() && x != 0.0 && !next.is_normal(),
        | _ => false,
    };
    let moved = match (x, y) {
        | (RuntimeValue::F32(x), RuntimeValue::F80(y)) =>
            X87::from_f32(x).compare(y).is_some_and(Ordering::is_ne),
        | (RuntimeValue::F64(x), RuntimeValue::F80(y)) =>
            X87::from_f64(x).compare(y).is_some_and(Ordering::is_ne),
        | _ => false,
    };
    if leaves_normal_range && moved {
        ERANGE
    } else {
        0
    }
}

/// `ilogb`'s `EDOM` for zero, a NaN or an infinity, whose results are
/// `FP_ILOGB0`, `FP_ILOGBNAN` (both `INT_MIN`) and `INT_MAX`
/// (`math/w_ilogb_template.c`).
fn range_of_ilogb(result: RuntimeValue) -> i32 {
    let RuntimeValue::Int(bits) = result else {
        return 0;
    };
    #[expect(clippy::cast_possible_truncation, reason = "`ilogb` returns an int.")]
    let value = bits as u32 as i32;
    if value == i32::MIN || value == i32::MAX {
        EDOM
    } else {
        0
    }
}

/// A logarithm's error at or below `pole`: `ERANGE` at it, `EDOM` below.
fn pole_or_domain(x: X87, pole: X87) -> i32 {
    match x.compare(pole) {
        | Some(Ordering::Equal) => ERANGE,
        | Some(Ordering::Less) => EDOM,
        | _ => 0,
    }
}

fn finite(value: X87) -> bool {
    matches!(value.class(), Class::Zero | Class::Finite { .. })
}

fn infinite(value: X87) -> bool {
    value.class() == Class::Infinite
}

fn is_nan(value: X87) -> bool {
    matches!(value.class(), Class::Nan { .. } | Class::Unsupported)
}

fn is_zero(value: X87) -> bool {
    value.class() == Class::Zero
}

fn magnitude(value: X87) -> X87 {
    value.with_sign(false)
}

fn less(left: X87, right: X87) -> bool {
    left.compare(right) == Some(Ordering::Less)
}

fn greater(left: X87, right: X87) -> bool {
    left.compare(right) == Some(Ordering::Greater)
}
