//! libgcc's `__multc3` and `__divtc3`: binary128 complex multiplication and
//! division as `libgcc2.c` computes them, each operation rounded once by
//! the soft-float routines (`crate::binary128`), so the results are libgcc's
//! bit for bit, NaN payloads and signs included.
//!
//! Multiplication forms the four products and recovers an infinite result
//! that computed as NaN + iNaN (C99 Annex G.5.1 paragraph 6). Division
//! scales by the larger part of the divisor (Smith's method with the
//! underflow and overflow corrections GCC 12 added) and recovers the
//! infinite and zero results of Annex G.5.1 paragraph 8.
//!
//! A NaN result keeps the sign and payload of the operand libgcc's
//! soft-float routine picks, which depends on the order of a sum's or a
//! product's operands. GCC is free to swap those, and does: where libgcc as
//! GCC 13 and 15 compile it on x86-64 evaluates `c * b` for the source's
//! `b * c`, or `x + a` for `a + x`, these functions follow the compiled
//! order.
//!
//! C99: Annex G.5.1 paragraphs 2-8, pp. 469-472; PDF pp. 481-484; the
//! helpers' names and algorithms are libgcc's (GCC internals manual,
//! "Routines for floating point emulation").

use std::cmp::Ordering;

use crate::binary128::Binary128 as Q;

/// `(a + ib) × (c + id)`, as `__multc3`.
/// C99: Annex G.5.1 paragraph 6, p. 470; PDF p. 482.
#[expect(
    clippy::many_single_char_names,
    reason = "libgcc's names, so that the port reads line by line against its source."
)]
pub(super) fn multiply([mut a, mut b]: [Q; 2], [mut c, mut d]: [Q; 2]) -> [Q; 2] {
    let (ac, bd, ad, bc) = (a.mul(c), b.mul(d), a.mul(d), c.mul(b));
    let mut x = ac.sub(bd);
    let mut y = ad.add(bc);
    if x.is_nan() && y.is_nan() {
        // Recover infinities that computed as NaN + iNaN.
        let mut recalculate = false;
        if is_infinite(a) || is_infinite(b) {
            // `z` is infinite: box the infinity and change NaNs in the
            // other factor to 0.
            a = boxed(a);
            b = boxed(b);
            c = nan_to_zero(c);
            d = nan_to_zero(d);
            recalculate = true;
        }
        if is_infinite(c) || is_infinite(d) {
            c = boxed(c);
            d = boxed(d);
            a = nan_to_zero(a);
            b = nan_to_zero(b);
            recalculate = true;
        }
        if !recalculate
            && (is_infinite(ac) || is_infinite(bd) || is_infinite(ad) || is_infinite(bc))
        {
            // Recover infinities from overflow by changing NaNs to 0.
            a = nan_to_zero(a);
            b = nan_to_zero(b);
            c = nan_to_zero(c);
            d = nan_to_zero(d);
            recalculate = true;
        }
        if recalculate {
            let infinity = Q::infinity(false);
            x = infinity.mul(a.mul(c).sub(b.mul(d)));
            y = infinity.mul(a.mul(d).add(b.mul(c)));
        }
    }
    [x, y]
}

/// `(a + ib) / (c + id)`, as `__divtc3`.
/// C99: Annex G.5.1 paragraph 8, pp. 471-472; PDF pp. 483-484.
#[expect(
    clippy::many_single_char_names,
    reason = "libgcc's names, so that the port reads line by line against its source."
)]
pub(super) fn divide([mut a, mut b]: [Q; 2], [mut c, mut d]: [Q; 2]) -> [Q; 2] {
    let (x, y);
    // Scale by the larger of `c` and `d` so the denominator does not
    // overflow.
    if less(c.with_sign(false), d.with_sign(false)) {
        // Prevent underflow when the denominator is near the largest value.
        if at_least(d.with_sign(false), RBIG) {
            [a, b, c, d] = [a, b, c, d].map(|part| part.div(TWO));
        }
        // Scale small parts up, which cannot overflow below `RMIN2`.
        if less(d.with_sign(false), RMIN2) || tiny_numerator(a, b, d) {
            [a, b, c, d] = [a, b, c, d].map(|part| part.mul(RMINSCAL));
        }
        let ratio = c.div(d);
        let denominator = c.mul(ratio).add(d);
        // Choose another order of computation if the ratio is subnormal.
        if less(RMIN, ratio.with_sign(false)) {
            x = a.mul(ratio).add(b).div(denominator);
            y = b.mul(ratio).sub(a).div(denominator);
        } else {
            x = a.div(d).mul(c).add(b).div(denominator);
            y = b.div(d).mul(c).sub(a).div(denominator);
        }
    } else {
        if at_least(c.with_sign(false), RBIG) {
            [a, b, c, d] = [a, b, c, d].map(|part| part.div(TWO));
        }
        if less(c.with_sign(false), RMIN2) || tiny_numerator(a, b, c) {
            [a, b, c, d] = [a, b, c, d].map(|part| part.mul(RMINSCAL));
        }
        let ratio = d.div(c);
        let denominator = d.mul(ratio).add(c);
        if less(RMIN, ratio.with_sign(false)) {
            x = b.mul(ratio).add(a).div(denominator);
            y = b.sub(a.mul(ratio)).div(denominator);
        } else {
            // The source's `a + d * (b / c)`.
            x = b.div(c).mul(d).add(a).div(denominator);
            y = b.sub(a.div(c).mul(d)).div(denominator);
        }
    }
    // Recover infinities and zeros that computed as NaN + iNaN; the only
    // cases are nonzero/zero, infinite/finite and finite/infinite.
    if x.is_nan() && y.is_nan() {
        let zero = Q::zero(false);
        if equal(c, zero) && equal(d, zero) && (!a.is_nan() || !b.is_nan()) {
            let infinity = Q::infinity(c.is_negative());
            return [infinity.mul(a), infinity.mul(b)];
        }
        if (is_infinite(a) || is_infinite(b)) && is_finite(c) && is_finite(d) {
            let (a, b) = (boxed(a), boxed(b));
            let infinity = Q::infinity(false);
            return [
                infinity.mul(a.mul(c).add(b.mul(d))),
                infinity.mul(b.mul(c).sub(a.mul(d))),
            ];
        }
        if (is_infinite(c) || is_infinite(d)) && is_finite(a) && is_finite(b) {
            let (c, d) = (boxed(c), boxed(d));
            return [
                zero.mul(a.mul(c).add(b.mul(d))),
                zero.mul(b.mul(c).sub(a.mul(d))),
            ];
        }
    }
    [x, y]
}

/// libgcc's second scaling test: a numerator part below `RMIN` with the
/// other part and the divisor's larger part below `RMAX2`.
fn tiny_numerator(a: Q, b: Q, larger: Q) -> bool {
    let (a, b, larger) = (
        a.with_sign(false),
        b.with_sign(false),
        larger.with_sign(false),
    );
    (less(a, RMIN) && less(b, RMAX2) && less(larger, RMAX2))
        || (less(b, RMIN) && less(a, RMAX2) && less(larger, RMAX2))
}

/// `TF_MAX / 2`, below which the divisor's larger part needs no halving.
const RBIG: Q = Q::from_bits(0x7FFD_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF);
/// `TF_MIN`, the smallest normal value.
const RMIN: Q = Q::from_bits(0x0001_0000_0000_0000_0000_0000_0000_0000);
/// `TF_EPSILON`, 2^-112.
const RMIN2: Q = Q::from_bits(0x3F8F_0000_0000_0000_0000_0000_0000_0000);
/// `1 / TF_EPSILON`, 2^112.
const RMINSCAL: Q = Q::from_bits(0x406F_0000_0000_0000_0000_0000_0000_0000);
/// `RBIG * RMIN2`.
const RMAX2: Q = Q::from_bits(0x7F8D_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF);
const TWO: Q = Q::from_bits(0x4000_0000_0000_0000_0000_0000_0000_0000);

/// `x < y`, false when either is a NaN (`__lttf2`).
fn less(x: Q, y: Q) -> bool {
    x.compare(y) == Some(Ordering::Less)
}

/// `x >= y`, false when either is a NaN (`__getf2`).
fn at_least(x: Q, y: Q) -> bool {
    matches!(x.compare(y), Some(Ordering::Greater | Ordering::Equal))
}

/// `x == y`, false when either is a NaN (`__eqtf2`).
fn equal(x: Q, y: Q) -> bool {
    x.compare(y) == Some(Ordering::Equal)
}

fn is_infinite(x: Q) -> bool {
    matches!(x.class(), crate::binary128::Class::Infinite)
}

fn is_finite(x: Q) -> bool {
    !is_infinite(x) && !x.is_nan()
}

/// `COPYSIGN (isinf (x) ? 1 : 0, x)`: an infinity boxed to a unit.
fn boxed(x: Q) -> Q {
    let magnitude = if is_infinite(x) {
        Q::from_bits(0x3FFF_u128 << 112)
    } else {
        Q::zero(false)
    };
    magnitude.with_sign(x.is_negative())
}

/// `if (isnan (x)) x = COPYSIGN (0, x)`.
fn nan_to_zero(x: Q) -> Q {
    if x.is_nan() {
        Q::zero(x.is_negative())
    } else {
        x
    }
}
