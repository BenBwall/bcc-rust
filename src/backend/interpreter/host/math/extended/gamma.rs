//! `lgammal`, `lgammal_r` and `tgammal`, as glibc computes them for the x87
//! format in C (`sysdeps/ieee754/ldbl-96/e_lgammal_r.c`, `e_gammal_r.c`
//! and `gamma_productl.c`), operation by operation on the x87 model:
//! `lgammal` by rational approximations on intervals below 8, Stirling's
//! series above it, the reflection formula for negative arguments and, for
//! -33 < x < -2, `__lgamma_negl` (`gamma_negative.rs`); `tgammal` from
//! `expl` of `lgammal` below 7.5 and Stirling's formula with the power of
//! two factored out above it. The exponentials, logarithms and powers they
//! call are glibc's (`exponential.rs`), as are `sinl` and `cosl`
//! (`trigonometric.rs`).
//!
//! glibc 2.39 and 2.43 compute both alike. `lgammal`'s wrapper only sets
//! `errno` and the sign (`signgam`), while `tgammal`'s replaces the value
//! of a pole, a negative integer and an overflow (`__kernel_standard_l`
//! types 240, 241 and 250), as [`tgamma`] does.
//! C99: §7.12.8.3-§7.12.8.4, pp. 230-231; PDF pp. 242-243.

use super::{
    exact::{
        biased,
        frexp,
        round,
        words,
    },
    exponential::{
        Base,
        Logarithm,
        NAN,
        exp,
        expm1,
        log_ieee,
        pow,
    },
    expression::{
        HALF,
        MAX,
        ONE,
        TWO,
        double,
        equal,
        greater,
        greater_equal,
        integer,
        is_finite,
        is_infinite,
        less,
        less_equal,
        truncate,
    },
    gamma_negative::lgamma_negative,
    trigonometric::{
        cos,
        horner,
        sin,
    },
};
use crate::{
    backend::interpreter::trap::Fault,
    x87::{
        Integral,
        X87,
    },
};

/// `__ieee754_lgammal_r`: `log |Γ(x)|` and the sign of `Γ(x)`. The
/// wrappers (`lgammal`, `lgammal_r`, `gammal`) return the same value.
#[expect(
    clippy::many_single_char_names,
    reason = "glibc's names, so that the port reads line by line against its source."
)]
pub(in crate::backend::interpreter::host::math) fn lgamma(x: X87) -> Result<(X87, i32), Fault> {
    let mut sign = 1;
    let (se, i0, i1) = words(x);
    let negative = se & 0x8000 != 0;
    let exponent = u32::from(biased(se));
    if exponent | i0 | i1 == 0 {
        if negative {
            sign = -1;
        }
        return Ok((ONE.div(x.with_sign(false)), sign));
    }
    let ix = (exponent << 16) | (i0 >> 16);
    // Infinities and NaNs.
    if ix >= 0x7FFF_0000 {
        return Ok((x.mul(x), sign));
    }
    // |x| < 2^-63: -log(|x|).
    if ix < 0x3FC0_8000 {
        if negative {
            return Ok((log_ieee(x.negate(), Logarithm::Natural)?.negate(), -1));
        }
        return Ok((log_ieee(x, Logarithm::Natural)?.negate(), sign));
    }
    let mut x = x;
    let mut nadj = X87::zero(false);
    if negative {
        if less(x, integer(-2)) && greater(x, integer(-33)) {
            return lgamma_negative(x);
        }
        let t = sin_pi(x);
        if equal(t, X87::zero(false)) {
            // A negative integer.
            return Ok((ONE.div(t.with_sign(false)), sign));
        }
        nadj = log_ieee(PI.div(t.mul(x).with_sign(false)), Logarithm::Natural)?;
        if less(t, X87::zero(false)) {
            sign = -1;
        }
        x = x.negate();
    }
    // glibc's test for 1 and 2, which the explicit integer bit in `i0`
    // keeps from ever succeeding.
    let r = if (ix.wrapping_sub(0x3FFF_8000) | i0 | i1) == 0
        || (ix.wrapping_sub(0x4000_8000) | i0 | i1) == 0
    {
        X87::zero(false)
    } else if ix < 0x4000_8000 {
        below_two(x, ix)?
    } else if ix < 0x4002_8000 {
        below_eight(x)?
    } else if ix < 0x4041_8000 {
        // 8 <= x < 2^66: Stirling's series in 1/x.
        let t = log_ieee(x, Logarithm::Natural)?;
        let z = ONE.div(x);
        let y = z.mul(z);
        let w = W0.add(z.mul(horner(y, &W)));
        x.sub(HALF).mul(t.sub(ONE)).add(w)
    } else {
        x.mul(log_ieee(x, Logarithm::Natural)?.sub(ONE))
    };
    Ok((if negative { nadj.sub(r) } else { r }, sign))
}

/// `lgammal` for 0 < x < 2, from `lgamma(x + 1) - log(x)` below 0.9.
fn below_two(x: X87, ix: u32) -> Result<X87, Fault> {
    let (mut r, y, interval);
    if ix <= 0x3FFE_E666 {
        r = log_ieee(x, Logarithm::Natural)?.negate();
        (y, interval) = if ix >= 0x3FFE_BB4A {
            (x.sub(ONE), 0)
        } else if ix >= 0x3FFC_ED33 {
            (x.sub(TC.sub(ONE)), 1)
        } else {
            (x, 2)
        };
    } else {
        r = X87::zero(false);
        (y, interval) = if ix >= 0x3FFF_DDA6 {
            (x.sub(double(2.0)), 0)
        } else if ix >= 0x3FFF_9DA6 {
            (x.sub(TC), 1)
        } else {
            (x.sub(ONE), 2)
        };
    }
    match interval {
        | 0 => {
            let p1 = horner(y, &A);
            let p2 = monic(y, &B);
            r = r.add(HALF.mul(y).add(y.mul(p1).div(p2)));
        },
        | 1 => {
            let p1 = horner(y, &G);
            let p2 = monic(y, &H);
            let p = TT.add(y.mul(p1).div(p2));
            r = r.add(TF.add(p));
        },
        | _ => {
            let p1 = y.mul(horner(y, &U));
            let p2 = monic(y, &V);
            r = r.add(HALF.negate().mul(y).add(p1.div(p2)));
        },
    }
    Ok(r)
}

/// `lgammal` for 2 <= x < 8: `lgamma(2 + y)` and the logarithm of the
/// product `(y + 2)(y + 3)…` that brings the argument down.
#[expect(
    clippy::many_single_char_names,
    reason = "glibc's names, so that the port reads line by line against its source."
)]
fn below_eight(x: X87) -> Result<X87, Fault> {
    let i = truncate(x);
    let y = x.sub(integer(i64::from(i)));
    let p = y.mul(horner(y, &S));
    let q = monic(y, &R);
    let mut r = HALF.mul(y).add(p.div(q));
    if (3..=7).contains(&i) {
        let mut z = ONE;
        for k in (2..i).rev() {
            z = z.mul(y.add(integer(i64::from(k))));
        }
        r = r.add(log_ieee(z, Logarithm::Natural)?);
    }
    Ok(r)
}

/// `c[0] + y × (c[1] + … + y × (c[n-1] + y))`: a polynomial whose leading
/// coefficient, 1, the source leaves out.
fn monic(y: X87, coefficients: &[X87]) -> X87 {
    let (&last, rest) = coefficients.split_last().expect("a polynomial");
    rest.iter()
        .rev()
        .fold(last.add(y), |sum, &coefficient| coefficient.add(y.mul(sum)))
}

/// `sin_pi`: `-sin(πx)` for negative `x`, reduced so that an integer gives
/// an exact zero.
fn sin_pi(x: X87) -> X87 {
    let (se, i0, _) = words(x);
    let ix = (u32::from(biased(se)) << 16) | (i0 >> 16);
    if ix < 0x3FFD_8000 {
        return sin(PI.mul(x));
    }
    let mut y = x.negate();
    let mut z = y.round_integral(Integral::Floor);
    let n;
    if z.compare(y) == Some(std::cmp::Ordering::Equal) {
        if ix >= 0x403F_8000 {
            // 2^64 and up: even.
            y = X87::zero(false);
            n = 0;
        } else {
            if ix < 0x403E_8000 {
                z = y.add(TWO63);
            }
            let low = words(z).2;
            let odd = (low & 1).cast_signed();
            y = integer(i64::from(odd));
            n = odd << 2;
        }
    } else {
        y = y.mul(double(0.5));
        y = double(2.0).mul(y.sub(y.round_integral(Integral::Floor)));
        n = truncate(y.mul(double(4.0)));
    }
    let y = match n {
        | 0 => sin(PI.mul(y)),
        | 1 | 2 => cos(PI.mul(HALF.sub(y))),
        | 3 | 4 => sin(PI.mul(ONE.sub(y))),
        | 5 | 6 => cos(PI.mul(y.sub(double(1.5)))).negate(),
        | _ => sin(PI.mul(y.sub(double(2.0)))),
    };
    y.negate()
}

/// `tgammal`, with its wrapper's values: ±∞ for ±0 and for an overflow
/// (with the argument's sign), `NAN` for a negative integer and -∞, and
/// the sign applied otherwise (`math/w_tgammal_compat.c`).
pub(super) fn tgamma(x: X87) -> Result<X87, Fault> {
    let (y, sign) = gamma(x)?;
    let infinite_or_zero = !is_finite(y) || equal(y, X87::zero(false));
    if infinite_or_zero && (is_finite(x) || (is_infinite(x) && less(x, X87::zero(false)))) {
        if equal(x, X87::zero(false)) {
            return Ok(X87::infinity(x.is_negative()));
        }
        if equal(x.round_integral(Integral::Floor), x) && less(x, X87::zero(false)) {
            return Ok(NAN);
        }
        if !equal(y, X87::zero(false)) {
            return Ok(X87::infinity(x.is_negative()));
        }
    }
    Ok(if sign < 0 { y.negate() } else { y })
}

/// `__ieee754_gammal_r`: `|Γ(x)|` or `Γ(x)`, and the sign to apply (zero
/// when the value has it).
fn gamma(x: X87) -> Result<(X87, i32), Fault> {
    let (se, hx, lx) = words(x);
    let es = u32::from(se);
    if (es & 0x7FFF) | hx | lx == 0 {
        return Ok((double(1.0).div(x), 0));
    }
    // glibc's test for -∞, which the 16-bit sign and exponent word keeps
    // from ever succeeding: -∞ takes the next branch.
    if es == 0xFFFF_FFFF && ((hx & 0x7FFF_FFFF) | lx) == 0 {
        return Ok((x.sub(x), 0));
    }
    if es & 0x7FFF == 0x7FFF {
        return Ok((x.add(x), 0));
    }
    if es & 0x8000 != 0 && equal(x.round_integral(Integral::HalfEven), x) {
        let difference = x.sub(x);
        return Ok((difference.div(difference), 0));
    }
    if greater_equal(x, integer(1756)) {
        return Ok((MAX.mul(MAX), 0));
    }
    let mut sign = 0;
    let ret = if greater(x, X87::zero(false)) {
        let (ret, exp2_adj) = positive(x)?;
        ret.scale(i64::from(exp2_adj))
    } else if greater_equal(x, EPSILON.negate().div(double(4.0))) {
        ONE.div(x)
    } else {
        let tx = x.round_integral(Integral::Truncate);
        let half = tx.div(double(2.0)).round_integral(Integral::Truncate);
        sign = if equal(tx, double(2.0).mul(half)) {
            -1
        } else {
            1
        };
        if less_equal(x, integer(-1766)) {
            // Underflow.
            MIN.mul(MIN)
        } else {
            let mut frac = tx.sub(x);
            if greater(frac, HALF) {
                frac = ONE.sub(frac);
            }
            let sinpix = if less_equal(frac, double(0.25)) {
                sin(PI.mul(frac))
            } else {
                cos(PI.mul(HALF.sub(frac)))
            };
            let (g, exp2_adj) = positive(x.negate())?;
            let ret = PI.div(x.negate().mul(sinpix).mul(g));
            ret.scale(-i64::from(exp2_adj))
        }
    };
    let saturated = |bound: X87| {
        let product = bound.with_sign(ret.is_negative());
        if sign < 0 {
            product.negate().mul(bound).negate()
        } else {
            product.mul(bound)
        }
    };
    Ok(if is_infinite(ret) && !equal(x, X87::zero(false)) {
        (saturated(MAX), sign)
    } else if equal(ret, X87::zero(false)) {
        (saturated(MIN), sign)
    } else {
        (ret, sign)
    })
}

/// `gammal_positive`: `Γ(x)` for 0 < x < 1756 as `R × 2^exp2_adj`.
#[expect(
    clippy::similar_names,
    reason = "glibc's names, so that the port reads line by line against its source."
)]
fn positive(x: X87) -> Result<(X87, i32), Fault> {
    if less(x, HALF) {
        let (log, _) = lgamma(x.add(ONE))?;
        return Ok((exp(log, Base::E)?.div(x), 0));
    }
    if less_equal(x, double(1.5)) {
        let (log, _) = lgamma(x)?;
        return Ok((exp(log, Base::E)?, 0));
    }
    if less(x, double(7.5)) {
        // Adjust into the range for exp(lgamma).
        let n = x.sub(double(1.5)).round_integral(Integral::Ceil);
        let x_adj = x.sub(n);
        let (prod, eps) = product(x_adj, X87::zero(false), truncate(n));
        let (log, _) = lgamma(x_adj)?;
        return Ok((exp(log, Base::E)?.mul(prod).mul(ONE.add(eps)), 0));
    }
    let (mut eps, mut x_eps, mut x_adj, mut prod) = (X87::zero(false), X87::zero(false), x, ONE);
    if less(x, double(13.0)) {
        // Adjust into the range for Stirling's approximation.
        let n = double(13.0).sub(x).round_integral(Integral::Ceil);
        x_adj = x.add(n);
        x_eps = x.sub(x_adj.sub(n));
        (prod, eps) = product(x_adj.sub(n), x_eps, truncate(n));
    }
    // Γ(x_adj + x_eps) / (prod × (1 + eps)), starting from
    // pow(x_adj, x_adj) with a power of two factored out.
    let mut exp_adj = eps.negate();
    let x_adj_int = round(x_adj);
    let x_adj_frac = x_adj.sub(x_adj_int);
    let (mut x_adj_mant, mut x_adj_log2) = frexp(x_adj);
    if less(x_adj_mant, SQRT1_2) {
        x_adj_log2 -= 1;
        x_adj_mant = x_adj_mant.mul(TWO);
    }
    let exp2_adj = x_adj_log2.wrapping_mul(truncate(x_adj_int));
    let ret = pow(x_adj_mant, x_adj)?
        .mul(exp(
            integer(i64::from(x_adj_log2)).mul(x_adj_frac),
            Base::Two,
        )?)
        .mul(exp(x_adj.negate(), Base::E)?)
        .mul(TWO_PI.div(x_adj).sqrt())
        .div(prod);
    exp_adj = exp_adj.add(x_eps.mul(log_ieee(x_adj, Logarithm::Natural)?));
    let (&last, rest) = GAMMA_COEFF.split_last().expect("coefficients");
    let x_adj2 = x_adj.mul(x_adj);
    let bsum = rest
        .iter()
        .rev()
        .fold(last, |sum, &coefficient| sum.div(x_adj2).add(coefficient));
    exp_adj = exp_adj.add(bsum.div(x_adj));
    Ok((ret.add(ret.mul(expm1(exp_adj)?)), exp2_adj))
}

/// `__gamma_productl`: the product of `x + x_eps`, `x + x_eps + 1`, …,
/// `x + x_eps + n - 1` as `R × (1 + eps)`.
fn product(x: X87, x_eps: X87, n: i32) -> (X87, X87) {
    let mut ret = x;
    let mut eps = x_eps.div(x);
    for i in 1..n {
        let xi = x.add(integer(i64::from(i)));
        eps = eps.add(x_eps.div(xi));
        let (high, low) = mul_split(ret, xi);
        ret = high;
        eps = eps.add(low.div(ret));
    }
    (ret, eps)
}

/// `mul_splitl` without a fused multiply-add: `x × y` exactly as
/// `high + low`, by Dekker's algorithm.
pub(super) fn mul_split(x: X87, y: X87) -> (X87, X87) {
    let high = x.mul(y);
    let x1 = x.mul(SPLIT);
    let y1 = y.mul(SPLIT);
    let x1 = x.sub(x1).add(x1);
    let y1 = y.sub(y1).add(y1);
    let x2 = x.sub(x1);
    let y2 = y.sub(y1);
    let low = x1
        .mul(y1)
        .sub(high)
        .add(x1.mul(y2))
        .add(x2.mul(y1))
        .add(x2.mul(y2));
    (high, low)
}

/// `(1LL << (LDBL_MANT_DIG + 1) / 2) + 1`, Dekker's splitting constant.
const SPLIT: X87 = X87::from_bits(0x401F_8000_0000_8000_0000);

/// `M_PIl` (`pi` in `e_lgammal_r.c`), `2 × M_PIl`, `M_SQRT1_2l`,
/// `LDBL_EPSILON`, `LDBL_MIN` and `2^63`.
pub(super) const PI: X87 = X87::from_bits(0x4000_C90F_DAA2_2168_C235);
const TWO_PI: X87 = X87::from_bits(0x4001_C90F_DAA2_2168_C235);
const SQRT1_2: X87 = X87::from_bits(0x3FFE_B504_F333_F9DE_6484);
const EPSILON: X87 = X87::from_bits(0x3FC0_8000_0000_0000_0000);
const MIN: X87 = X87::from_bits(0x0001_8000_0000_0000_0000);
const TWO63: X87 = X87::from_bits(0x403E_8000_0000_0000_0000);

/// `tc`, the minimum of Γ on the positive axis, `lgamma(tc)` as a `double`
/// (`tf`) and its tail (`tt`), and `w0`, `LS2PI - 0.5`.
const TC: X87 = X87::from_bits(0x3FFF_BB16_C31A_B5F1_FB71);
const TF: X87 = X87::from_bits(0xBFFB_F8CD_CDE6_1C52_1000);
const TT: X87 = X87::from_bits(0x3FC4_F84A_E446_AD36_0AC8);
const W0: X87 = X87::from_bits(0x3FFD_D67F_1C86_4BEB_4A69);

/// The rational approximations' coefficients (`a0`-`a5` over `b0`-`b4`,
/// `g` over `h`, `u` over `v`, `s` over `r`; each denominator's leading
/// 1 left out) and Stirling's `w1`-`w7`.
const A: [X87; 6] = [
    X87::from_bits(0xC008_9E94_C730_2246_C2A8),
    X87::from_bits(0x4009_E811_ED79_A706_3DDF),
    X87::from_bits(0x400A_964B_BAC9_559E_AC31),
    X87::from_bits(0x4008_DC1A_CEEC_4B3B_A538),
    X87::from_bits(0x4005_E312_8057_2629_A7F8),
    X87::from_bits(0x4000_F115_D0E1_3AEA_5AAC),
];
const B: [X87; 5] = [
    X87::from_bits(0x400C_805B_E515_45C7_6426),
    X87::from_bits(0x400C_A05D_BD87_D12E_640B),
    X87::from_bits(0x400B_8E4A_B327_2A55_A21D),
    X87::from_bits(0x4008_D4AC_A520_4E78_D8C0),
    X87::from_bits(0x4004_F1B2_AA9F_4861_B5CB),
];
const G: [X87; 7] = [
    X87::from_bits(0x3FC5_867F_0E7C_2934_0760),
    X87::from_bits(0x400B_A035_3CB5_5C12_1785),
    X87::from_bits(0x400C_89F2_6A0F_C9F1_1345),
    X87::from_bits(0x400B_AAC1_7DCD_6744_4A43),
    X87::from_bits(0x4009_B5ED_AD4A_2B6A_F60D),
    X87::from_bits(0x4006_9A2C_6D7D_A4A7_143C),
    X87::from_bits(0x4001_8ABC_66E1_056E_BB97),
];
const H: [X87; 6] = [
    X87::from_bits(0x400C_A58F_65AF_2D63_52AF),
    X87::from_bits(0x400D_A7CE_6EDD_A997_9471),
    X87::from_bits(0x400D_805C_4B9F_648B_E3D4),
    X87::from_bits(0x400B_B768_2D0B_D004_1DB4),
    X87::from_bits(0x4008_F41B_2AA4_D08C_27AA),
    X87::from_bits(0x4005_80D9_8684_9A09_4970),
];
const U: [X87; 7] = [
    X87::from_bits(0xC005_B1B9_6F00_70C4_A41B),
    X87::from_bits(0x4008_AB00_B430_2BB3_CBF1),
    X87::from_bits(0x4009_FF54_090C_5581_BF2A),
    X87::from_bits(0x4009_EEF7_2A37_8D76_FD0C),
    X87::from_bits(0x4008_BA2D_37BF_660E_26EB),
    X87::from_bits(0x4005_E273_8851_DB57_6BEE),
    X87::from_bits(0x4001_8F80_3217_7068_1672),
];
const V: [X87; 6] = [
    X87::from_bits(0x4009_8FDA_96EE_5615_5B39),
    X87::from_bits(0x400A_D47B_1380_1C9A_BE6B),
    X87::from_bits(0x400A_ECAA_1B77_60BA_7C84),
    X87::from_bits(0x4009_F5CE_6768_5BED_2E20),
    X87::from_bits(0x4007_ED11_6565_A2F2_67A3),
    X87::from_bits(0x4004_B45C_0DD3_D2AA_7F6A),
];
const S: [X87; 7] = [
    X87::from_bits(0x4013_B194_321B_770C_93B4),
    X87::from_bits(0xC014_EE1F_D18F_72CB_B4CD),
    X87::from_bits(0xC015_C89C_0165_7BB2_F73A),
    X87::from_bits(0xC014_CA94_3F86_A403_F6BE),
    X87::from_bits(0xC012_AD37_1282_903E_B2E4),
    X87::from_bits(0xC00E_F4AA_4437_06E8_10E3),
    X87::from_bits(0xC009_D29D_A60F_5DFA_05EF),
];
const R: [X87; 7] = [
    X87::from_bits(0xC017_8FBC_72CD_BD6A_A4C7),
    X87::from_bits(0xC017_D6C8_7E69_FFDB_436A),
    X87::from_bits(0xC016_F42D_96F3_D84B_ACAE),
    X87::from_bits(0xC015_838B_FC9A_902F_B887),
    X87::from_bits(0xC012_87D5_CC4A_A7C1_EF90),
    X87::from_bits(0xC00D_EC8A_B16E_428C_6E2C),
    X87::from_bits(0xC007_E119_8B5A_F4A1_810D),
];
const W: [X87; 7] = [
    X87::from_bits(0x3FFB_AAAA_AAAA_AAAA_9FCC),
    X87::from_bits(0xBFF6_B60B_60B6_03A8_4D88),
    X87::from_bits(0x3FF4_D00D_0092_30E5_F8F2),
    X87::from_bits(0xBFF4_9C09_844E_9FCE_8B20),
    X87::from_bits(0x3FF4_DC88_D492_AAD1_3BDC),
    X87::from_bits(0xBFF5_F685_3DA1_0304_3D91),
    X87::from_bits(0x3FF7_A012_91C2_CC08_D984),
];

/// The coefficients `B_2k / 2k(2k-1)` of Stirling's series for Γ.
const GAMMA_COEFF: [X87; 8] = [
    X87::from_bits(0x3FFB_AAAA_AAAA_AAAA_AAAB),
    X87::from_bits(0xBFF6_B60B_60B6_0B60_B60B),
    X87::from_bits(0x3FF4_D00D_00D0_0D00_D00D),
    X87::from_bits(0xBFF4_9C09_C09C_09C0_9C0A),
    X87::from_bits(0x3FF4_DCA8_F158_C7F9_1AB8),
    X87::from_bits(0xBFF5_FB55_86CC_C9E3_E410),
    X87::from_bits(0x3FF7_D20D_20D2_0D20_D20D),
    X87::from_bits(0xBFF9_F214_3658_7A9C_BEE1),
];
