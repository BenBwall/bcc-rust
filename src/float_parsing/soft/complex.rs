//! Complex multiplication and division as the bridge's Clang build computes
//! them. Clang multiplies inline and calls the runtime's `__mul?c3` only
//! when both parts come out NaN, and always calls `__div?c3`. The x86-64
//! link resolves those routines to libgcc's (GCC 12 and later): binary32
//! division computes in binary64, and the wider formats divide by Smith's
//! method with guards against overflow and underflow.
//!
//! C99: multiplicative operators, §6.5.5, pp. 82-83; PDF pp. 94-95; complex
//! arithmetic, Annex G.5.1, pp. 469-472; PDF pp. 481-484.

use super::{
    arithmetic::{
        add_of,
        div_of,
        less,
        max_finite,
        mul_of,
        power_of_two,
        sub_of,
    },
    value::{
        F32,
        F64,
        Format,
        Soft,
    },
};

/// The operation codes the bridge's complex helpers take.
const ADD: i32 = 0;
const SUBTRACT: i32 = 1;
const MULTIPLY: i32 = 2;

/// `(a + bi) operation (c + di)` in `format`; any operation code other than
/// add, subtract and multiply divides.
pub(super) fn arithmetic(
    [a, b]: [Soft; 2],
    [c, d]: [Soft; 2],
    operation: i32,
    format: Format,
) -> [Soft; 2] {
    match operation {
        | ADD => [add_of(a, c, format), add_of(b, d, format)],
        | SUBTRACT => [sub_of(a, c, format), sub_of(b, d, format)],
        | MULTIPLY => {
            let product = products(a, b, c, d, format);
            if product[0].is_nan() && product[1].is_nan() {
                multiply_recovering(a, b, c, d, format)
            } else {
                product
            }
        },
        | _ if format == F32 => divide_promoted(a, b, c, d),
        | _ => divide_smith(a, b, c, d, format),
    }
}

/// `(ac - bd) + (ad + bc)i`.
fn products(a: Soft, b: Soft, c: Soft, d: Soft, format: Format) -> [Soft; 2] {
    let ac = mul_of(a, c, format);
    let bd = mul_of(b, d, format);
    let ad = mul_of(a, d, format);
    let bc = mul_of(b, c, format);
    [sub_of(ac, bd, format), add_of(ad, bc, format)]
}

/// `__mulsc3`, `__muldc3` and `__mulxc3`, which agree: recover the
/// infinities that computed as NaN + NaN i (C99 Annex G.5.1 example 1,
/// p. 471; PDF p. 483).
fn multiply_recovering(
    mut a: Soft,
    mut b: Soft,
    mut c: Soft,
    mut d: Soft,
    format: Format,
) -> [Soft; 2] {
    let ac = mul_of(a, c, format);
    let bd = mul_of(b, d, format);
    let ad = mul_of(a, d, format);
    let bc = mul_of(b, c, format);
    let product = [sub_of(ac, bd, format), add_of(ad, bc, format)];
    if !(product[0].is_nan() && product[1].is_nan()) {
        return product;
    }
    let mut recalculate = false;
    if a.is_infinite() || b.is_infinite() {
        a = box_infinity(a);
        b = box_infinity(b);
        c = zero_nan(c);
        d = zero_nan(d);
        recalculate = true;
    }
    if c.is_infinite() || d.is_infinite() {
        c = box_infinity(c);
        d = box_infinity(d);
        a = zero_nan(a);
        b = zero_nan(b);
        recalculate = true;
    }
    if !recalculate
        && (ac.is_infinite() || bd.is_infinite() || ad.is_infinite() || bc.is_infinite())
    {
        a = zero_nan(a);
        b = zero_nan(b);
        c = zero_nan(c);
        d = zero_nan(d);
        recalculate = true;
    }
    if !recalculate {
        return product;
    }
    let infinity = Soft::infinity(false);
    let real = sub_of(mul_of(a, c, format), mul_of(b, d, format), format);
    let imaginary = add_of(mul_of(a, d, format), mul_of(b, c, format), format);
    [
        mul_of(infinity, real, format),
        mul_of(infinity, imaginary, format),
    ]
}

/// libgcc's `__divsc3`: the textbook formula in binary64, rounded to
/// binary32 once at the end, then the shared recovery of infinities.
fn divide_promoted(a: Soft, b: Soft, c: Soft, d: Soft) -> [Soft; 2] {
    let [wide_a, wide_b, wide_c, wide_d] = [a, b, c, d].map(|x| x.convert(F64));
    let denominator = add_of(
        mul_of(wide_c, wide_c, F64),
        mul_of(wide_d, wide_d, F64),
        F64,
    );
    let real = add_of(
        mul_of(wide_a, wide_c, F64),
        mul_of(wide_b, wide_d, F64),
        F64,
    );
    let imaginary = sub_of(
        mul_of(wide_b, wide_c, F64),
        mul_of(wide_a, wide_d, F64),
        F64,
    );
    let quotient = [real, imaginary].map(|part| div_of(part, denominator, F64).convert(F32));
    recover_quotient(quotient, a, b, c, d, F32)
}

/// libgcc's `__divdc3` and `__divxc3`: Smith's method, with the operands halved
/// when the divisor nears overflow and scaled up when they are small enough to
/// underflow.
fn divide_smith(a: Soft, b: Soft, c: Soft, d: Soft, format: Format) -> [Soft; 2] {
    // RBIG, RMIN, RMIN2 (the epsilon), RMINSCAL and RMAX2 of libgcc2.c.
    let big = div_of(max_finite(format), power_of_two(1), format);
    let least = power_of_two(format.min_exponent);
    let epsilon = power_of_two(1 - format.precision as i32);
    let enlargement = power_of_two(format.precision as i32 - 1);
    let big_scaled = mul_of(big, epsilon, format);
    let (mut a, mut b, mut c, mut d) = (a, b, c, d);
    // Whether `d` is the divisor part of greater magnitude.
    let by_d = less(c.abs(), d.abs());
    let large = if by_d { d } else { c };
    if !less(large.abs(), big) && !large.is_nan() {
        let two = power_of_two(1);
        [a, b, c, d] = [a, b, c, d].map(|x| div_of(x, two, format));
    }
    let large = if by_d { d } else { c };
    let small_dividend = (less(a.abs(), least) && less(b.abs(), big_scaled))
        || (less(b.abs(), least) && less(a.abs(), big_scaled));
    if less(large.abs(), epsilon) || (small_dividend && less(large.abs(), big_scaled)) {
        [a, b, c, d] = [a, b, c, d].map(|x| mul_of(x, enlargement, format));
    }
    let [real, imaginary] = if by_d {
        let ratio = div_of(c, d, format);
        let denominator = add_of(mul_of(c, ratio, format), d, format);
        if less(least, ratio.abs()) {
            [
                add_of(mul_of(a, ratio, format), b, format),
                sub_of(mul_of(b, ratio, format), a, format),
            ]
        } else {
            [
                add_of(mul_of(c, div_of(a, d, format), format), b, format),
                sub_of(mul_of(c, div_of(b, d, format), format), a, format),
            ]
        }
        .map(|part| div_of(part, denominator, format))
    } else {
        let ratio = div_of(d, c, format);
        let denominator = add_of(mul_of(d, ratio, format), c, format);
        if less(least, ratio.abs()) {
            [
                add_of(mul_of(b, ratio, format), a, format),
                sub_of(b, mul_of(a, ratio, format), format),
            ]
        } else {
            [
                add_of(a, mul_of(d, div_of(b, c, format), format), format),
                sub_of(b, mul_of(d, div_of(a, c, format), format), format),
            ]
        }
        .map(|part| div_of(part, denominator, format))
    };
    recover_quotient([real, imaginary], a, b, c, d, format)
}

/// Recovers the infinities and zeros a quotient computed as NaN + NaN i:
/// a nonzero dividend over zero, an infinite dividend over a finite
/// divisor, and a finite dividend over an infinite divisor (C99 Annex
/// G.5.1 example 2, p. 472; PDF p. 484).
fn recover_quotient(
    quotient: [Soft; 2],
    a: Soft,
    b: Soft,
    c: Soft,
    d: Soft,
    format: Format,
) -> [Soft; 2] {
    if !(quotient[0].is_nan() && quotient[1].is_nan()) {
        return quotient;
    }
    if c.is_zero() && d.is_zero() && (!a.is_nan() || !b.is_nan()) {
        let infinity = Soft::infinity(false).with_sign_of(c);
        [mul_of(infinity, a, format), mul_of(infinity, b, format)]
    } else if (a.is_infinite() || b.is_infinite()) && c.is_finite() && d.is_finite() {
        recover_infinite_dividend(a, b, c, d, format)
    } else if (c.is_infinite() || d.is_infinite()) && a.is_finite() && b.is_finite() {
        recover_infinite_divisor(a, b, c, d, format)
    } else {
        quotient
    }
}

/// An infinite dividend over a finite divisor is infinite.
fn recover_infinite_dividend(a: Soft, b: Soft, c: Soft, d: Soft, format: Format) -> [Soft; 2] {
    let a = box_infinity(a);
    let b = box_infinity(b);
    let infinity = Soft::infinity(false);
    [
        mul_of(
            infinity,
            add_of(mul_of(a, c, format), mul_of(b, d, format), format),
            format,
        ),
        mul_of(
            infinity,
            sub_of(mul_of(b, c, format), mul_of(a, d, format), format),
            format,
        ),
    ]
}

/// A finite dividend over an infinite divisor is zero.
fn recover_infinite_divisor(a: Soft, b: Soft, c: Soft, d: Soft, format: Format) -> [Soft; 2] {
    let c = box_infinity(c);
    let d = box_infinity(d);
    let zero = Soft::zero(false);
    [
        mul_of(
            zero,
            add_of(mul_of(a, c, format), mul_of(b, d, format), format),
            format,
        ),
        mul_of(
            zero,
            sub_of(mul_of(b, c, format), mul_of(a, d, format), format),
            format,
        ),
    ]
}

/// `copysign(isinf(x) ? 1 : 0, x)`.
fn box_infinity(value: Soft) -> Soft {
    if value.is_infinite() {
        Soft::one(value.negative)
    } else {
        Soft::zero(value.negative)
    }
}

/// `isnan(x) ? copysign(0, x) : x`.
fn zero_nan(value: Soft) -> Soft {
    if value.is_nan() {
        Soft::zero(value.negative)
    } else {
        value
    }
}
