//! The exponential, logarithm and power functions, as glibc's x86-64
//! assembly language computes them (`sysdeps/x86_64/fpu/e_expl.S`,
//! `e_exp2l.S`, `s_expm1l.S`, `e_logl.S`, `e_log10l.S`,
//! `e_log2l.S`, `s_log1pl.S` and `e_powl.S` with `sysdeps/x86/fpu/
//! powl_helper.c`): each splits its argument so that `f2xm1`, `fyl2x` or
//! `fyl2xp1` computes the transcendental part, and the processor runs those
//! instructions (`processor.rs`). Every other instruction runs on the x87
//! model, in the order the assembly language runs it, with the branches it
//! takes on `fxam` and comparison results; `powl` follows glibc 2.43, which
//! computes integral powers of magnitude below 4 as products (2.39 differed
//! for 3 and -3). The public names add the `w_*l_compat.c` wrappers' return
//! values where they differ from the functions': a logarithm of a negative
//! value is `NAN`.
//! C99: §7.12.6.1-§7.12.6.3, pp. 223-224; PDF pp. 235-236; §7.12.6.7-
//! §7.12.6.10, pp. 226-227; PDF pp. 238-239; §7.12.7.4, p. 230; PDF p. 242.

use super::{
    exact::{
        biased,
        frexp,
        from_words,
        round,
        words,
    },
    expression::{
        HALF,
        ONE,
        double,
        equal,
        greater,
        greater_equal,
        integer,
        is_nan,
        less,
        less_equal,
        truncate,
    },
    processor::{
        Examined,
        LN_2,
        LOG2_E,
        LOG10_2,
        compare,
        examine,
        f2xm1,
        fscale,
        fyl2x,
        fyl2xp1,
    },
};
use crate::{
    backend::interpreter::trap::Fault,
    x87::{
        Integral,
        X87,
    },
};

/// Which power of its argument an exponential computes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Base {
    E,
    Two,
}

/// `expl` and `exp2l`. Their wrappers only set `errno`.
pub(super) fn exp(x: X87, base: Base) -> Result<X87, Fault> {
    if base == Base::Two {
        return exp2(x);
    }
    let exponent = biased(words(x).0);
    let mut x = x;
    if exponent > 0x400D {
        // Overflow, underflow, an infinity or a NaN: saturate the finite
        // magnitudes (and invalid encodings) at 2^15.
        match examine(x) {
            | Examined::Infinity if x.is_negative() => return Ok(X87::zero(false)),
            | Examined::Infinity => return Ok(x),
            | Examined::Nan => return Ok(x.add(x)),
            | _ => x = SATURATED.with_sign(x.is_negative()),
        }
    } else if exponent < 0x3FBC {
        return Ok(x.add(ONE));
    }
    exp_core(x, false)
}

/// `expm1l`, which glibc exports without a wrapper; from 128 up it is
/// `expl`'s wrapper, and from -128 down it is -1.
pub(super) fn expm1(x: X87) -> Result<X87, Fault> {
    let sign_exponent = words(x).0;
    if sign_exponent ^ 0x8000 >= 0xC006 {
        return exp(x, Base::E);
    }
    if sign_exponent >= 0xC006 {
        return Ok(if examine(x) == Examined::Nan {
            x.add(x)
        } else {
            ONE.negate()
        });
    }
    // Zeros, values below 2^-64 and denormals are their own `expm1l`.
    if examine(x) == Examined::Zero || biased(sign_exponent) < 0x3FBF {
        return Ok(x);
    }
    exp_core(x, true)
}

/// glibc's `expl` constants: log2(e) in a high part of 32 bits and a low
/// part (`c0` and `c1`), and the saturation point 2^15.
const C0: X87 = X87::from_bits(0x3FFF_B8AA_0000_0000_0000);
const C1: X87 = X87::from_bits(0x3FED_ECA5_705F_C2EE_FA20);
const SATURATED: X87 = X87::from_bits(0x400E_8000_0000_0000_0000);

/// The common part of `e_expl.S`: `e^x = 2^i × 2^f` for `i` the nearest
/// integer to `x × log2(e)` and `f` the rest computed in extra precision,
/// or `e^x - 1` for `expm1l`.
fn exp_core(x: X87, minus_one: bool) -> Result<X87, Fault> {
    let (c0, c1) = (C0, C1);
    let i = LOG2_E.mul(x).round_integral(Integral::HalfEven);
    let xi = x.round_integral(Integral::HalfEven);
    let f = c0.mul(xi).sub(i);
    let f = f.add(c0.mul(x.sub(xi)));
    let f = f.add(c1.mul(x));
    let fraction = f2xm1(f)?;
    if minus_one {
        let scaled = fscale(fraction, i);
        let power = fscale(ONE, i).sub(ONE);
        return Ok(scaled.add(power));
    }
    Ok(fscale(fraction.add(ONE), i))
}

/// `exp2l` (`e_exp2l.S`): `2^i × 2^(x - i)` for `i` the nearest integer.
fn exp2(x: X87) -> Result<X87, Fault> {
    if examine(x) == Examined::Infinity {
        return Ok(if x.is_negative() { X87::zero(false) } else { x });
    }
    if biased(words(x).0) < 0x3FBE {
        return Ok(x.add(ONE));
    }
    let i = x.round_integral(Integral::HalfEven);
    let fraction = f2xm1(x.sub(i))?;
    Ok(fscale(fraction.add(ONE), i))
}

/// Which logarithm a function computes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Logarithm {
    Natural,
    Two,
    Ten,
}

/// `logl`, `log2l` and `log10l`, with their wrappers: a zero gives
/// `-HUGE_VAL` and a negative value `NAN` (`__kernel_standard` types 216-219,
/// 248 and 249), the rest the function's result.
pub(super) fn log(x: X87, logarithm: Logarithm) -> Result<X87, Fault> {
    if less_equal(x, X87::zero(false)) {
        return Ok(if equal(x, X87::zero(false)) {
            X87::infinity(true)
        } else {
            NAN
        });
    }
    log_ieee(x, logarithm)
}

/// The positive quiet NaN a wrapper returns for a domain error: glibc's
/// `NAN` as a `double` widened to `long double`.
pub(super) const NAN: X87 = X87::from_bits(0x7FFF_C000_0000_0000_0000);

/// 0.29 as a `double`: below it, `fyl2xp1` computes the logarithm of
/// `1 + (x - 1)`.
const LIMIT: f64 = 0.29;

/// `__ieee754_logl`, `__ieee754_log2l` and `__ieee754_log10l`: `fyl2x` with
/// the base's factor, or `fyl2xp1` of `x - 1` near 1 (a zero difference
/// made positive).
pub(super) fn log_ieee(x: X87, logarithm: Logarithm) -> Result<X87, Fault> {
    let factor = match logarithm {
        | Logarithm::Natural => LN_2,
        | Logarithm::Two => ONE,
        | Logarithm::Ten => LOG10_2,
    };
    if examine(x) == Examined::Nan {
        return Ok(x.add(x));
    }
    // `logl` keeps `x` for x <= -2, so that `-LDBL_MAX - 1` does not
    // overflow.
    let difference = if logarithm == Logarithm::Natural && words(x).0 >= 0xC000 {
        x
    } else {
        x.sub(ONE)
    };
    if compare(difference.with_sign(false), double(LIMIT)).above {
        return fyl2x(x, factor);
    }
    let difference = if examine(difference) == Examined::Zero {
        difference.with_sign(false)
    } else {
        difference
    };
    fyl2xp1(difference, factor)
}

/// The `long double` 0.29 `s_log1pl.S` compares with.
const LOG1P_LIMIT: X87 = X87::from_bits(0x3FFD_947A_E147_AE14_7AE1);

/// `log1pl` (`s_log1pl.S`): `fyl2xp1` within 0.29 of zero, else `fyl2x`
/// of `x + 1` (of `x` from 2^65 up). Its wrapper only sets `errno`.
pub(super) fn log1p(x: X87) -> Result<X87, Fault> {
    if examine(x) == Examined::Nan {
        return Ok(x.add(x));
    }
    if !compare(LOG1P_LIMIT, x.with_sign(false)).carry {
        return fyl2xp1(x, LN_2);
    }
    let operand = if words(x).0 ^ 0x8000 >= 0xC040 {
        x
    } else {
        x.add(ONE)
    };
    fyl2x(operand, LN_2)
}

/// `e_powl.S`'s bounds, as `double`s.
const P2: f64 = 4.0;
const P63: f64 = 9_223_372_036_854_775_808.0;
const P64: f64 = 18_446_744_073_709_551_616.0;
const P78: f64 = 302_231_454_903_657_293_676_544.0;
const PM79: f64 = f64::from_bits(0x3B00_0000_0000_0000);

/// `powl`. Its wrapper only sets `errno`: the values it substitutes are the
/// function's own.
pub(super) fn pow(x: X87, y: X87) -> Result<X87, Fault> {
    let y_examined = examine(y);
    let y_negative = y.is_negative();
    match y_examined {
        | Examined::Zero => return Ok(power_of_zero(x)),
        | Examined::Infinity => return Ok(infinite_power(x, y_negative)),
        | Examined::Nan => return Ok(nan_power(x, y)),
        | _ => {},
    }
    let x_negative = x.is_negative();
    match examine(x) {
        | Examined::Zero => return Ok(zero_base(x_negative, y)),
        | Examined::Infinity => return Ok(infinite_base(x_negative, y)),
        | Examined::Nan => return Ok(y.add(x)),
        | _ => {},
    }
    let magnitude = y.with_sign(false);
    if !compare(magnitude, double(P63)).carry {
        // A large integer: beyond 2^78 any finite nonzero x gives 0, 1 or
        // an infinity, so saturate there.
        let y = if compare(magnitude, double(P78)).carry {
            y
        } else {
            double(P78).with_sign(y_negative)
        };
        return pow_helper(x, y);
    }
    // `fistpll` rounds to nearest, storing the integer indefinite for an
    // invalid encoding.
    let n = rounded_integer(y);
    if !compare(integer(n), y).zero {
        // Tiny powers saturate at 2^-79, with the sign the reverse of y's
        // as glibc's code has it; the result is 1 either way.
        let y = if compare(magnitude, double(PM79)).carry {
            if y_negative {
                double(PM79)
            } else {
                double(PM79).negate()
            }
        } else {
            y
        };
        return pow_helper(x, y);
    }
    if !compare(magnitude, double(P2)).carry {
        return pow_helper(x, y);
    }
    // An integral power in [-3, 3], or the integer indefinite.
    let (x, n) = if n < 0 {
        (double(1.0).div(x), n.wrapping_neg())
    } else {
        (x, n)
    };
    Ok(match n & 0xFFFF_FFFF {
        | 0 => ONE,
        | 1 => x,
        | 2 => x.with_sign(false).mul(x.with_sign(false)),
        | _ => x.mul(x.with_sign(false)).mul(x.with_sign(false)),
    })
}

/// `fistpll`: the value rounded to the nearest 64-bit integer, or the
/// integer indefinite `i64::MIN` for a NaN, an invalid encoding or a value
/// out of range.
fn rounded_integer(x: X87) -> i64 {
    let bits = x.to_rounded_integer(Integral::HalfEven, 64);
    u64::try_from(bits)
        .expect("64-bit integer bits")
        .cast_signed()
}

/// `powl(x, ±0)`: 1, unless `x` is a signaling NaN.
fn power_of_zero(x: X87) -> X87 {
    if examine(x) == Examined::Nan && x.magnitude_parts().1 & 1 << 62 == 0 {
        return x.add(x);
    }
    ONE
}

/// `powl(x, ±inf)`: an infinity or a zero by whether |x| is above 1, 1 for
/// |x| = 1, and `x + x` for a NaN or invalid `x`.
fn infinite_power(x: X87, y_negative: bool) -> X87 {
    let flags = compare(x.with_sign(false), ONE);
    if flags.parity {
        return x.add(x);
    }
    if flags.zero {
        return ONE;
    }
    if flags.carry == y_negative {
        X87::infinity(false)
    } else {
        X87::zero(false)
    }
}

/// `powl(x, NaN)`: 1 for `x` = 1 and a quiet NaN, else the NaNs' sum.
fn nan_power(x: X87, y: X87) -> X87 {
    let flags = compare(ONE, x);
    if flags.zero && !flags.carry && y.magnitude_parts().1 & 1 << 62 != 0 {
        return x;
    }
    y.add(x)
}

/// Whether `y` (finite, below 2^64 in magnitude) is an odd or even integer
/// after `fprem` by 2^63 and `fistpll`, and the integer's sign; `None` when
/// it is not an integer.
fn integer_parity(y: X87) -> Option<(bool, bool)> {
    let reduced = y.rem(double(P63));
    let n = rounded_integer(reduced);
    if !compare(integer(n), reduced).zero {
        return None;
    }
    Some((n & 1 != 0, n < 0))
}

/// `powl(±0, y)`: an infinity for a negative `y`, negative only for -0 and
/// an odd integer, else a zero, negative likewise.
fn zero_base(x_negative: bool, y: X87) -> X87 {
    if !y.is_negative() {
        // `fcomi` compares y itself with 2^64 here, as y > 0.
        let odd = x_negative
            && compare(y, double(P64)).carry
            && integer_parity(y).is_some_and(|(odd, _)| odd);
        return X87::zero(odd);
    }
    let odd = x_negative
        && compare(y.with_sign(false), double(P64)).carry
        && integer_parity(y).is_some_and(|(odd, _)| odd);
    X87::infinity(odd)
}

/// `powl(±inf, y)`: for -inf and an integral y below 2^64, the sign of an
/// odd power and the integer's sign choose among -inf, -0, +inf and +0;
/// otherwise y's sign does, as for +inf.
fn infinite_base(x_negative: bool, y: X87) -> X87 {
    let by_sign = |negative: bool| {
        if negative {
            X87::zero(false)
        } else {
            X87::infinity(false)
        }
    };
    if !x_negative || !compare(y.with_sign(false), double(P64)).carry {
        // `fcom` with 0.0: below or unordered gives +0.
        return by_sign(compare(y, X87::zero(false)).carry);
    }
    match integer_parity(y) {
        | None => by_sign(y.is_negative()),
        | Some((true, negative)) =>
            if negative {
                X87::zero(true)
            } else {
                X87::infinity(true)
            },
        | Some((false, negative)) => by_sign(negative),
    }
}

/// `powl_helper.c`'s table: high and low parts of -log(k/16) for k from
/// 12 to 24.
const LOG_TABLE: [X87; 26] = [
    X87::from_bits(0x3FFD_934B_1089_A6DC_93C2),
    X87::from_bits(0xBFBB_8291_3127_EAAC_7ABA),
    X87::from_bits(0x3FFC_D49F_69E4_56CF_1B79),
    X87::from_bits(0x3FBB_BEA7_7A5C_80DC_CDCF),
    X87::from_bits(0x3FFC_88BC_7411_3F23_DEF2),
    X87::from_bits(0xBFBB_C74B_E038_D217_E1C4),
    X87::from_bits(0x3FFB_842C_C5AC_F1D0_3445),
    X87::from_bits(0x3FB8_FF66_FD40_CDCB_04BF),
    X87::from_bits(0x8000_0000_0000_0000_0000),
    X87::from_bits(0x0000_0000_0000_0000_0000),
    X87::from_bits(0xBFFA_F851_8600_8B15_330C),
    X87::from_bits(0x3FB7_CDA3_A445_3343_B396),
    X87::from_bits(0xBFFB_F138_3B71_5797_2F4F),
    X87::from_bits(0xBFBA_A87F_FE1F_E9E1_55DC),
    X87::from_bits(0xBFFC_AFF9_8385_3C9E_9E44),
    X87::from_bits(0x3FBB_C1DF_5F8D_EDC4_5019),
    X87::from_bits(0xBFFC_E47F_BE3C_D4D1_0D61),
    X87::from_bits(0xBFBA_BB03_DE5F_F734_495C),
    X87::from_bits(0xBFFD_8B3A_E55D_5D30_701D),
    X87::from_bits(0x3FBA_CE0A_A3BE_4747_DC10),
    X87::from_bits(0xBFFD_A30C_5E10_E2F6_13E8),
    X87::from_bits(0xBFBC_B7B3_7B33_C734_415E),
    X87::from_bits(0xBFFD_B9CE_BFB5_DE80_34E7),
    X87::from_bits(0xBFBA_9314_FEB4_FBDE_5AAE),
    X87::from_bits(0xBFFD_CF99_1F65_FCC2_5F96),
    X87::from_bits(0x3FBC_9728_990B_FADD_E7E6),
];

/// log2(e) in a high part of 32 bits and the rest, and in full.
const LOG2E_HI: X87 = X87::from_bits(0x3FFF_B8AA_3B29_0000_0000);
const LOG2E_LO: X87 = X87::from_bits(0x3FDE_B82F_E177_7D0F_FDA1);
const LOG2E: X87 = X87::from_bits(0x3FFF_B8AA_3B29_5C17_F0BC);

/// 2/3 rounded down, and the high and low parts of 1/3.
const TWO_THIRDS: X87 = X87::from_bits(0x3FFE_AAAA_AAAA_AAAA_AAAA);
const THIRD_HI: X87 = X87::from_bits(0x3FFD_AAAA_0000_0000_0000);
const THIRD_LO: X87 = X87::from_bits(0x3FED_AAAA_AAAA_AAAA_AAAA);

/// The value `hi + lo` stored as two parts, as `acc_split` keeps it.
#[derive(Clone, Copy, Debug)]
struct Split {
    hi: X87,
    lo: X87,
}

impl Split {
    /// `acc_split`: adds `x` to the value.
    fn add(self, x: X87) -> Self {
        let thi = self.hi.add(x);
        let tlo = self.hi.sub(thi).add(x).add(self.lo);
        let hi = thi.add(tlo);
        Self {
            hi,
            lo: thi.sub(hi).add(tlo),
        }
    }
}

/// The value with its significand's low 32 bits cleared.
fn high_32(x: X87) -> X87 {
    let (se, i0, _) = words(x);
    from_words(se, i0, 0)
}

/// `__powl_helper`: `x^y` for finite nonzero (or NaN) `x` and finite
/// nonzero 2^-79 <= |y| <= 2^78, as `2^(y × log2(x))` with the logarithm
/// computed to 78 bits from the table and a power series.
fn pow_helper(x: X87, y: X87) -> Result<X87, Fault> {
    let through_logarithm = |x: X87| exp(y.mul(log_ieee(x, Logarithm::Natural)?), Base::E);
    if is_nan(x) {
        return through_logarithm(x);
    }
    let mut negate = false;
    let mut x = x;
    if less(x, X87::zero(false)) {
        let absy = y.with_sign(false);
        if !greater_equal(absy, double(P64)) {
            let truncated = absy.round_integral(Integral::Truncate);
            if !equal(truncated, absy) {
                return through_logarithm(x);
            }
            negate = truncated.to_integer(false, 64).is_some_and(|n| n & 1 != 0);
        }
        x = x.with_sign(false);
    }
    let (mut x_frac, mut exponent) = frexp(x);
    if less_equal(x_frac, TWO_THIRDS) {
        x_frac = x_frac.mul(double(2.0));
        exponent -= 1;
    }
    let k = truncate(integer(16).div(x_frac).add(HALF));
    let entry = |offset: i32| {
        usize::try_from(2 * k - offset)
            .ok()
            .and_then(|index| LOG_TABLE.get(index).copied())
            .unwrap_or(X87::INDEFINITE)
    };
    let mut log = Split {
        hi: entry(24),
        lo: entry(23),
    };
    let x_frac_low = if k == 16 {
        X87::zero(false)
    } else {
        // Clear the low five bits so that the multiplication by k/16 is
        // exact; their logarithm is added at the end.
        let (se, i0, i1) = words(x_frac);
        let masked = from_words(se, i0, i1 & 0xFFFF_FFE0);
        let low = x_frac.sub(masked).div(masked);
        x_frac = masked.mul(integer(i64::from(k)).div(integer(16)));
        low
    };
    let w = x_frac.sub(ONE);
    let (se, i0, _) = words(w);
    let w_hi = from_words(se, i0 & 0xFFFF_0000, 0);
    let w_lo = w.sub(w_hi);
    let mut wp = w_hi;
    log = log.add(wp);
    wp = wp.mul(w_hi.negate());
    log = log.add(wp.div(double(2.0)));
    wp = wp.mul(w_hi.negate());
    log = log.add(wp.mul(THIRD_HI));
    log = log.add(wp.mul(THIRD_LO));
    wp = wp.mul(w_hi.negate());
    log = log.add(wp.div(double(4.0)));
    for i in 5..=17 {
        wp = wp.mul(w_hi.negate());
        log = log.add(wp.div(integer(i)));
    }
    // To base 2.
    let hi32 = high_32(log.hi);
    let lo64 = log.hi.sub(hi32).add(log.lo);
    let hi1 = hi32.mul(LOG2E_HI);
    let lo1 = lo64.mul(LOG2E_HI).add(log.hi.mul(LOG2E_LO));
    let hi = hi1.add(lo1);
    let mut base_two = Split {
        hi,
        lo: hi1.sub(hi).add(lo1),
    };
    let correction = fyl2xp1(w_lo.div(ONE.add(w_hi)), ONE)?;
    base_two = base_two.add(correction);
    base_two = base_two.add(x_frac_low.mul(LOG2E));
    let exponent = integer(i64::from(exponent));
    let log2_x_hi = exponent.add(base_two.hi);
    let log2_x_lo = exponent.sub(log2_x_hi).add(base_two.hi).add(base_two.lo);
    // Times y.
    let log2_x_hi32 = high_32(log2_x_hi);
    let log2_x_lo64 = log2_x_hi.sub(log2_x_hi32).add(log2_x_lo);
    let y_hi32 = high_32(y);
    let y_lo32 = y.sub(y_hi32);
    let res_hi = log2_x_hi32.mul(y_hi32);
    let res_lo = log2_x_hi32.mul(y_lo32).add(log2_x_lo64.mul(y));
    let res_int = round(res_hi);
    let mut res_frac = res_hi.sub(res_int).add(res_lo);
    if greater(res_int.with_sign(false), integer(16500)) {
        res_frac = X87::zero(false);
    }
    let mut result = f2xm1(res_frac)?.add(ONE);
    if negate {
        result = result.negate();
    }
    Ok(fscale(result, res_int))
}
