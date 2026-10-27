//! `cbrtl`, `hypotl` and `fmal`, as glibc's `sysdeps/ieee754/ldbl-96` C
//! code computes them (`s_cbrtl.c`, `e_hypotl.c`, `s_fmal.c`), on the x87
//! model. `fmal` computes the correctly rounded `x × y + z` by rounding to
//! odd; here its finite valid cases are rounded once from the exact value,
//! and the rest follow its explicit branches.
//! C99: §7.12.7.1, p. 229; PDF p. 241; §7.12.7.3, p. 229; PDF p. 241;
//! §7.12.13.1, p. 239; PDF p. 251.

use super::{
    exact::{
        biased,
        frexp,
        words,
    },
    expression::{
        ONE,
        double,
        equal,
        greater,
        greater_equal,
        is_finite,
        is_infinite,
        is_nan,
        less,
        less_equal,
    },
};
use crate::x87::{
    Class,
    X87,
};

/// `cbrtl`: a polynomial start in `double` coefficients scaled by the
/// cube root of 2^(e mod 3), then two Newton steps.
#[expect(
    clippy::excessive_precision,
    reason = "glibc's literals, which round to these doubles."
)]
pub(super) fn cbrt(x: X87) -> X87 {
    const CBRT2: f64 = 1.259_921_049_894_873_164_8;
    const SQR_CBRT2: f64 = 1.587_401_051_968_199_474_8;
    const FACTOR: [f64; 5] = [1.0 / SQR_CBRT2, 1.0 / CBRT2, 1.0, CBRT2, SQR_CBRT2];
    let (xm, xe) = frexp(x.with_sign(false));
    if xe == 0 && (is_nan(x) || is_infinite(x) || equal(x, X87::zero(false))) {
        return x.add(x);
    }
    let u = double(-1.346_611_047_335_952_065_51E-1)
        .mul(xm)
        .add(double(5.466_460_136_639_552_450_34E-1));
    let u = u.mul(xm).sub(double(9.543_822_477_150_944_652_50E-1));
    let u = u.mul(xm).add(double(1.139_998_335_471_729_327_37E0));
    let u = u.mul(xm).add(double(4.023_897_956_454_475_212_69E-1));
    let factor = usize::try_from(2 + xe % 3).expect("a remainder of three");
    let u = u.mul(double(FACTOR[factor]));
    let u = if greater(x, X87::zero(false)) {
        u
    } else {
        u.negate()
    };
    let mut u = u.scale(i64::from(xe / 3));
    for _ in 0..2 {
        u = u.sub(u.sub(x.div(u.mul(u))).mul(THIRD));
    }
    u
}

const THIRD: X87 = X87::from_bits(0x3FFD_AAAA_AAAA_AAAA_AAAB);

/// `e_hypotl.c`'s scaling bounds.
const SCALE: X87 = X87::from_bits(0x1FC1_8000_0000_0000_0000);
const LARGE: X87 = X87::from_bits(0x5FFE_B504_F333_F9DE_6484);
const TINY: X87 = X87::from_bits(0x2000_8000_0000_0000_0000);
const EPS: X87 = X87::from_bits(0x3FBE_8000_0000_0000_0000);

/// `hypotl`: √(x² + y²) with a correction step, scaling huge and tiny
/// operands. An infinity wins over a quiet NaN; its wrapper's overflow
/// value is the function's own.
pub(super) fn hypot(x: X87, y: X87) -> X87 {
    if !is_finite(x) || !is_finite(y) {
        if (is_infinite(x) || is_infinite(y)) && !is_signaling(x) && !is_signaling(y) {
            return X87::infinity(false);
        }
        return x.add(y);
    }
    let (x, y) = (x.with_sign(false), y.with_sign(false));
    let (ax, ay) = if less(x, y) { (y, x) } else { (x, y) };
    if greater(ax, LARGE) {
        if less_equal(ay, ax.mul(EPS)) {
            return ax.add(ay);
        }
        return hypot_kernel(ax.mul(SCALE), ay.mul(SCALE)).div(SCALE);
    }
    if less(ay, TINY) {
        if greater_equal(ax, ay.div(EPS)) {
            return ax.add(ay);
        }
        return hypot_kernel(ax.div(SCALE), ay.div(SCALE)).mul(SCALE);
    }
    if less_equal(ay, ax.mul(EPS)) {
        return ax.add(ay);
    }
    hypot_kernel(ax, ay)
}

/// `e_hypotl.c`'s kernel for ax >= ay >= 0 whose squares neither overflow
/// nor underflow.
fn hypot_kernel(ax: X87, ay: X87) -> X87 {
    let two = double(2.0);
    let h = ax.mul(ax).add(ay.mul(ay)).sqrt();
    let (t1, t2) = if less_equal(h, two.mul(ay)) {
        let delta = h.sub(ay);
        (
            ax.mul(two.mul(delta).sub(ax)),
            delta.sub(two.mul(ax.sub(ay))).mul(delta),
        )
    } else {
        let delta = h.sub(ax);
        (
            two.mul(delta).mul(ax.sub(two.mul(ay))),
            double(4.0).mul(delta).sub(ay).mul(ay).add(delta.mul(delta)),
        )
    };
    h.sub(t1.add(t2).div(two.mul(h)))
}

/// glibc's `issignalingl` for the x87 format: a NaN without its quiet bit,
/// or any pseudo number (a nonzero exponent without the integer bit).
fn is_signaling(x: X87) -> bool {
    let (se, high, low) = words(x);
    if biased(se) != 0 && high & 0x8000_0000 == 0 {
        return true;
    }
    let high = (high ^ 0x4000_0000) | ((low | low.wrapping_neg()) >> 31);
    biased(se) == 0x7FFF && high > 0xC000_0000
}

/// `fmal`.
#[expect(
    clippy::many_single_char_names,
    reason = "glibc's names, so that the port reads line by line against its source."
)]
pub(super) fn fma(x: X87, y: X87, z: X87) -> X87 {
    const BIAS: i32 = 0x3FFF;
    const DIGITS: i32 = 64;
    let exponent = |value: X87| i32::from(biased(words(value).0));
    let (u, v, w) = (exponent(x), exponent(y), exponent(z));
    let zero = X87::zero(false);
    if u + v >= 0x7FFF + BIAS - DIGITS
        || u >= 0x7FFF - DIGITS
        || v >= 0x7FFF - DIGITS
        || w >= 0x7FFF - DIGITS
        || u + v <= BIAS + DIGITS
    {
        if w == 0x7FFF && u != 0x7FFF && v != 0x7FFF {
            return z.add(x).add(y);
        }
        if equal(z, zero) && !equal(x, zero) && !equal(y, zero) {
            return x.mul(y);
        }
        if u == 0x7FFF || v == 0x7FFF || w == 0x7FFF || equal(x, zero) || equal(y, zero) {
            return x.mul(y).add(z);
        }
        if u + v > 0x7FFF + BIAS {
            return x.mul(y);
        }
        if u + v < BIAS - DIGITS - 2 {
            // The product is below a quarter of the smallest denormal: only
            // its sign matters.
            let tiny = X87::from_bits(0x0000_0000_0000_0000_0001)
                .with_sign(x.is_negative() != y.is_negative());
            if w >= 3 {
                return tiny.add(z);
            }
            let up = X87::from_integer(false, 1 << 65);
            return z.mul(up).add(tiny).mul(ONE.div(up));
        }
    }
    if (equal(x, zero) || equal(y, zero)) && equal(z, zero) {
        return x.mul(y).add(z);
    }
    if [x, y, z]
        .iter()
        .any(|value| value.class() == Class::Unsupported)
    {
        return X87::INDEFINITE;
    }
    x.mul_add(y, z)
}
