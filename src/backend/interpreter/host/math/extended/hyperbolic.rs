//! The hyperbolic functions and their inverses, as glibc's
//! `sysdeps/ieee754/ldbl-96` C code computes them (`e_sinhl.c`,
//! `e_coshl.c`, `s_tanhl.c`, `s_asinhl.c`, `e_acoshl.c`, `e_atanhl.c`): on
//! the x87 model, from `expl`, `expm1l`, `logl` and `log1pl`
//! (`exponential.rs`), whose transcendental steps run on the processor.
//! The wrappers' substitutes equal these functions' results.
//! C99: §7.12.5.1-§7.12.5.6, pp. 221-223; PDF pp. 233-235.

use super::{
    exact::{
        biased,
        words,
    },
    exponential::{
        Base,
        Logarithm,
        exp,
        expm1,
        log_ieee,
        log1p,
    },
    expression::{
        HALF,
        ONE,
        TWO,
        double,
        greater,
        less,
    },
};
use crate::{
    backend::interpreter::trap::Fault,
    x87::X87,
};

/// `sinhl`'s and `asinhl`'s large constants and `tanhl`'s tiny one.
const SINH_HUGE: X87 = X87::from_bits(0x7FFB_AC23_C221_16CB_F8A4);
const HUGE: X87 = X87::from_bits(0x7F94_AE92_0427_5937_A4C1);
const TINY: X87 = X87::from_bits(0x0069_BBB4_DF56_BAF6_2972);

/// ln 2.
const LN2: X87 = X87::from_bits(0x3FFE_B172_17F7_D1CF_79AC);

/// `sinhl`.
pub(super) fn sinh(x: X87) -> Result<X87, Fault> {
    let (jx, i0, i1) = words(x);
    let ix = biased(jx);
    if ix == 0x7FFF {
        return Ok(x.add(x));
    }
    let h = if jx & 0x8000 != 0 {
        HALF.negate()
    } else {
        HALF
    };
    let absx = x.with_sign(false);
    if ix < 0x4003 || (ix == 0x4003 && i0 <= 0xC800_0000) {
        if ix < 0x3FDF && greater(SINH_HUGE.add(x), ONE) {
            return Ok(x);
        }
        let t = expm1(absx)?;
        if ix < 0x3FFF {
            return Ok(h.mul(double(2.0).mul(t).sub(t.mul(t).div(t.add(ONE)))));
        }
        return Ok(h.mul(t.add(t.div(t.add(ONE)))));
    }
    if ix < 0x400C || (ix == 0x400C && i0 < 0xB172_17F7) {
        return Ok(h.mul(exp(absx, Base::E)?));
    }
    if ix < 0x400C
        || (ix == 0x400C && (i0 < 0xB174_DDC0 || (i0 == 0xB174_DDC0 && i1 <= 0x31AE_C0EA)))
    {
        let w = exp(double(0.5).mul(absx), Base::E)?;
        let t = h.mul(w);
        return Ok(t.mul(w));
    }
    Ok(x.mul(SINH_HUGE))
}

/// `coshl`.
pub(super) fn cosh(x: X87) -> Result<X87, Fault> {
    let (ex, mx, lx) = words(x);
    let ex = biased(ex);
    let absx = x.with_sign(false);
    if ex < 0x4003 || (ex == 0x4003 && mx < 0xB000_0000) {
        if ex < 0x3FFD || (ex == 0x3FFD && mx < 0xB172_17F7) {
            if ex < 0x3FBC {
                return Ok(ONE);
            }
            let t = expm1(absx)?;
            let w = ONE.add(t);
            return Ok(ONE.add(t.mul(t).div(w.add(w))));
        }
        let t = exp(absx, Base::E)?;
        return Ok(HALF.mul(t).add(HALF.div(t)));
    }
    if ex < 0x400C || (ex == 0x400C && mx < 0xB170_0000) {
        return Ok(HALF.mul(exp(absx, Base::E)?));
    }
    if ex == 0x400C && (mx < 0xB174_DDC0 || (mx == 0xB174_DDC0 && lx < 0x31AE_C0EB)) {
        let w = exp(HALF.mul(absx), Base::E)?;
        let t = HALF.mul(w);
        return Ok(t.mul(w));
    }
    if ex == 0x7FFF {
        return Ok(x.mul(x));
    }
    Ok(HUGE.mul(HUGE))
}

/// `tanhl`.
pub(super) fn tanh(x: X87) -> Result<X87, Fault> {
    let (se, j0, j1) = words(x);
    let ix = biased(se);
    if ix == 0x7FFF {
        let reciprocal = ONE.div(x);
        return Ok(if se & 0x8000 != 0 {
            reciprocal.sub(ONE)
        } else {
            reciprocal.add(ONE)
        });
    }
    let z = if ix < 0x4003 || (ix == 0x4003 && j0 < 0xB800_0000) {
        if (u32::from(ix) | j0 | j1) == 0 {
            return Ok(x);
        }
        if ix < 0x3FC8 {
            return Ok(x.mul(ONE.add(TINY)));
        }
        if ix >= 0x3FFF {
            let t = expm1(TWO.mul(x.with_sign(false)))?;
            ONE.sub(TWO.div(t.add(TWO)))
        } else {
            let t = expm1(TWO.negate().mul(x.with_sign(false)))?;
            t.negate().div(t.add(TWO))
        }
    } else {
        ONE.sub(TINY)
    };
    Ok(if se & 0x8000 != 0 { z.negate() } else { z })
}

/// `asinhl`.
pub(super) fn asinh(x: X87) -> Result<X87, Fault> {
    let ix = biased(words(x).0);
    if ix < 0x3FDE && greater(HUGE.add(x), ONE) {
        return Ok(x);
    }
    let w = if ix > 0x4020 {
        if ix == 0x7FFF {
            return Ok(x.add(x));
        }
        log_ieee(x.with_sign(false), Logarithm::Natural)?.add(LN2)
    } else {
        let xa = x.with_sign(false);
        if ix > 0x4000 {
            let root = xa.mul(xa).add(ONE).sqrt();
            log_ieee(
                double(2.0).mul(xa).add(ONE.div(root.add(xa))),
                Logarithm::Natural,
            )?
        } else {
            let t = xa.mul(xa);
            log1p(xa.add(t.div(ONE.add(ONE.add(t).sqrt()))))?
        }
    };
    Ok(w.with_sign(x.is_negative()))
}

/// `acoshl`, with its wrapper: `0.0 / 0.0` in `double` for x < 1, the
/// default NaN.
pub(super) fn acosh(x: X87) -> Result<X87, Fault> {
    if less(x, ONE) {
        return Ok(X87::INDEFINITE);
    }
    let (se, i0, i1) = words(x);
    if se < 0x3FFF || se & 0x8000 != 0 {
        let difference = x.sub(x);
        return Ok(difference.div(difference));
    }
    if se >= 0x401D {
        if se >= 0x7FFF {
            return Ok(x.add(x));
        }
        return Ok(log_ieee(x, Logarithm::Natural)?.add(LN2));
    }
    if ((u32::from(se) - 0x3FFF) | (i0 ^ 0x8000_0000) | i1) == 0 {
        return Ok(X87::zero(false));
    }
    if se > 0x4000 {
        let t = x.mul(x);
        let root = t.sub(ONE).sqrt();
        return log_ieee(
            double(2.0).mul(x).sub(ONE.div(x.add(root))),
            Logarithm::Natural,
        );
    }
    let t = x.sub(ONE);
    log1p(t.add(double(2.0).mul(t).add(t.mul(t)).sqrt()))
}

/// `atanhl`, with its wrapper: `0.0 / 0.0` in `double` (the default NaN)
/// for |x| > 1 and `x / 0.0` for |x| = 1, which are the function's own
/// results.
pub(super) fn atanh(x: X87) -> Result<X87, Fault> {
    let (se, i0, i1) = words(x);
    let ix = u32::from(biased(se));
    let fraction = (i0 & 0x7FFF_FFFF) | i1;
    if ix + ((fraction | fraction.wrapping_neg()) >> 31) > 0x3FFF {
        let difference = x.sub(x);
        return Ok(difference.div(difference));
    }
    if ix == 0x3FFF {
        return Ok(x.div(X87::zero(false)));
    }
    if ix < 0x3FDF {
        return Ok(x);
    }
    let x = x.with_sign(false);
    let t = if ix < 0x3FFE {
        let t = x.add(x);
        double(0.5).mul(log1p(t.add(t.mul(x).div(ONE.sub(x))))?)
    } else {
        double(0.5).mul(log1p(x.add(x).div(ONE.sub(x)))?)
    };
    Ok(if se & 0x8000 == 0 { t } else { t.negate() })
}
