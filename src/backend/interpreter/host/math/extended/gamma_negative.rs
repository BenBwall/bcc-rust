//! `__lgamma_negl` (`sysdeps/ieee754/ldbl-96/lgamma_negl.c`) and
//! `__lgamma_productl` (`lgamma_productl.c`): `lgammal` for -33 < x < -2,
//! which `gamma.rs` hands over. Near each zero of lgamma in that range the
//! result is computed from the zero, so that it is accurate where it is
//! small: between -3 and -2 from polynomial approximations, and below -3 as
//! `log(sin(πx0) / sin(πx)) + log(Γ(1 - x0) / Γ(1 - x))` with Stirling's
//! series.
//! C99: §7.12.8.3, p. 230; PDF p. 242.

use super::{
    exponential::{
        Logarithm,
        log_ieee,
        log1p,
    },
    expression::{
        HALF,
        ONE,
        double,
        equal,
        integer,
        less,
        less_equal,
        truncate,
    },
    gamma::{
        PI,
        mul_split,
    },
    trigonometric::{
        cos,
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

/// `lgammal` of `x`, -33 < x < -2, and the sign of `Γ(x)`.
#[expect(
    clippy::similar_names,
    reason = "glibc's names, so that the port reads line by line against its source."
)]
pub(super) fn lgamma_negative(x: X87) -> Result<(X87, i32), Fault> {
    // The half-integer interval `x` lies in, the exact integers, and the
    // sign of the result.
    let minus_twice = integer(-2).mul(x);
    let mut i = truncate(minus_twice.round_integral(Integral::Floor));
    if i & 1 == 0 && equal(integer(i64::from(i)), minus_twice) {
        return Ok((X87::infinity(false), 1));
    }
    let xn = integer(i64::from(if i & 1 == 0 { -i / 2 } else { (-i - 1) / 2 }));
    i -= 4;
    let sign = if i & 2 == 0 { -1 } else { 1 };
    let index = usize::try_from(i).expect("an interval below -2");
    // Expand around the zero x0 = x0_hi + x0_lo.
    let (x0_hi, x0_lo) = ZEROS[index];
    let xdiff = x.sub(x0_hi).sub(x0_lo);
    // From -3 to -2: polynomial approximations to an adjusted gamma.
    if i < 2 {
        let j = truncate(integer(-8).mul(x).round_integral(Integral::Floor)) - 16;
        let j = usize::try_from(j).expect("an eighth of [-3, -2]");
        let xm = integer(-33 - 2 * j as i64).mul(double(0.0625));
        let x_adj = x.sub(xm);
        let (degree, end) = (POLY_DEG[j], POLY_END[j]);
        let mut g = POLY_COEFF[end];
        for k in 1..=degree {
            g = g.mul(x_adj).add(POLY_COEFF[end - k]);
        }
        return Ok((log1p(g.mul(xdiff).div(x.sub(xn)))?, sign));
    }
    // log(sinpi(x0) / sinpi(x)) + log(gamma(1 - x0) / gamma(1 - x)).
    let x_idiff = xn.sub(x).with_sign(false);
    let x0_idiff = xn.sub(x0_hi).sub(x0_lo).with_sign(false);
    let log_sinpi_ratio = if less(x0_idiff, x_idiff.mul(HALF)) {
        // log, not log1p, near -1.
        log_ieee(sin_pi(x0_idiff).div(sin_pi(x_idiff)), Logarithm::Natural)?
    } else {
        // log1p, not log, near 1; `x0diff2` is positive when x0 is
        // further from xn than x is.
        let x0diff2 = if i & 1 == 0 { xdiff } else { xdiff.negate() }.mul(HALF);
        let sx0d2 = sin_pi(x0diff2);
        let cx0d2 = cos_pi(x0diff2);
        let cot = cos_pi(x_idiff).div(sin_pi(x_idiff));
        log1p(
            double(2.0)
                .mul(sx0d2)
                .mul(sx0d2.negate().add(cx0d2.mul(cot))),
        )?
    };
    let mut y0 = ONE.sub(x0_hi);
    let mut y0_eps = x0_hi.negate().add(ONE.sub(y0)).sub(x0_lo);
    let mut y = ONE.sub(x);
    let mut y_eps = x.negate().add(ONE.sub(y));
    // log(gamma(y0 + y0_eps) / gamma(y + y_eps)) by Stirling's
    // approximation, after moving into the range where it is accurate.
    let mut log_gamma_adj = X87::zero(false);
    if i < 8 {
        let n_up = (9 - i) / 2;
        let up = integer(i64::from(n_up));
        let ny0 = y0.add(up);
        let ny0_eps = y0.sub(ny0.sub(up)).add(y0_eps);
        (y0, y0_eps) = (ny0, ny0_eps);
        let ny = y.add(up);
        let ny_eps = y.sub(ny.sub(up)).add(y_eps);
        (y, y_eps) = (ny, ny_eps);
        let prodm1 = product(xdiff, y.sub(up), y_eps, n_up);
        log_gamma_adj = log1p(prodm1)?.negate();
    }
    let log_gamma_high = xdiff
        .mul(log1p(y0.sub(E_HI).sub(E_LO).add(y0_eps).div(E_HI))?)
        .add(y.sub(HALF).add(y_eps).mul(log1p(xdiff.div(y))?))
        .add(log_gamma_adj);
    // The sum of (B_2k / 2k(2k-1))(y0^-(2k-1) - y^-(2k-1)).
    let (y0r, yr) = (ONE.div(y0), ONE.div(y));
    let (y0r2, yr2) = (y0r.mul(y0r), yr.mul(yr));
    let rdiff = xdiff.negate().div(y.mul(y0));
    let mut bterm = [X87::zero(false); LGAMMA_COEFF.len()];
    let (mut dlast, mut elast) = (rdiff, rdiff.mul(yr).mul(yr.add(y0r)));
    bterm[0] = dlast.mul(LGAMMA_COEFF[0]);
    for j in 1..LGAMMA_COEFF.len() {
        let dnext = dlast.mul(y0r2).add(elast);
        let enext = elast.mul(yr2);
        bterm[j] = dnext.mul(LGAMMA_COEFF[j]);
        (dlast, elast) = (dnext, enext);
    }
    let log_gamma_low = bterm
        .iter()
        .rev()
        .fold(X87::zero(false), |sum, &term| sum.add(term));
    let log_gamma_ratio = log_gamma_high.add(log_gamma_low);
    Ok((log_sinpi_ratio.add(log_gamma_ratio), sign))
}

/// `lg_sinpi`: sin(πx) for -0.25 <= x <= 0.5.
fn sin_pi(x: X87) -> X87 {
    if less_equal(x, double(0.25)) {
        sin(PI.mul(x))
    } else {
        cos(PI.mul(HALF.sub(x)))
    }
}

/// `lg_cospi`: cos(πx) for -0.25 <= x <= 0.5.
fn cos_pi(x: X87) -> X87 {
    if less_equal(x, double(0.25)) {
        cos(PI.mul(x))
    } else {
        sin(PI.mul(HALF.sub(x)))
    }
}

/// `__lgamma_productl`: the product of `1 + t / (x + x_eps + k)` for k
/// from 0 to n - 1, minus 1, keeping the low parts of each step.
fn product(t: X87, x: X87, x_eps: X87, n: i32) -> X87 {
    let (mut ret, mut ret_eps) = (X87::zero(false), X87::zero(false));
    for i in 0..n {
        let xi = x.add(integer(i64::from(i)));
        let quot = t.div(xi);
        let (mhi, mlo) = mul_split(quot, xi);
        let quot_lo = t
            .sub(mhi)
            .sub(mlo)
            .div(xi)
            .sub(t.mul(x_eps).div(xi.mul(xi)));
        // (1 + ret + ret_eps) × (1 + quot + quot_lo) - 1.
        let (rhi, rlo) = mul_split(ret, quot);
        let rpq = ret.add(quot);
        let rpq_eps = ret.sub(rpq).add(quot);
        let nret = rpq.add(rhi);
        let nret_eps = rpq.sub(nret).add(rhi);
        ret_eps = ret_eps.add(
            rpq_eps
                .add(nret_eps)
                .add(rlo)
                .add(ret_eps.mul(quot))
                .add(quot_lo)
                .add(quot_lo.mul(ret.add(ret_eps))),
        );
        ret = nret;
    }
    ret.add(ret_eps)
}

/// `poly_deg` and `poly_end`.
const POLY_DEG: [usize; 8] = [13, 13, 14, 15, 16, 15, 14, 13];
const POLY_END: [usize; 8] = [13, 27, 42, 58, 75, 91, 106, 120];

/// The zeros of lgamma between -33 and -2, each as a high and a low part.
const ZEROS: [(X87, X87); 62] = [
    (
        X87::from_bits(0xC000_9D3F_E4B0_07C3_60AB),
        X87::from_bits(0x3FBB_9B06_7507_2FC7_69E6),
    ),
    (
        X87::from_bits(0xC000_AFDA_0850_DEC8_065E),
        X87::from_bits(0xBFBF_CAD2_5A32_0F57_5FA6),
    ),
    (
        X87::from_bits(0xC000_C930_6DE4_F2CD_7BEE),
        X87::from_bits(0xBFBE_BE69_9AD3_D9BA_6545),
    ),
    (
        X87::from_bits(0xC000_FD23_8AA3_E17F_285C),
        X87::from_bits(0xBFBE_D456_1291_236C_C321),
    ),
    (
        X87::from_bits(0xC001_8142_73C2_CCAC_0618),
        X87::from_bits(0xBFC0_E7C2_C378_6044_AB06),
    ),
    (
        X87::from_bits(0xC001_9FBA_BBD3_7757_E6A2),
        X87::from_bits(0xBFBE_D0AA_4076_9885_01D8),
    ),
    (
        X87::from_bits(0xC001_A043_52BF_85B6_C865),
        X87::from_bits(0xBFC0_9315_B965_4E53_7B32),
    ),
    (
        X87::from_bits(0xC001_BFF4_97AC_8FA0_6AFC),
        X87::from_bits(0x3FC0_AC4A_74D0_B384_81C7),
    ),
    (
        X87::from_bits(0xC001_C00B_592C_4BE4_676C),
        X87::from_bits(0xBFBD_F85B_2DA2_C70B_970D),
    ),
    (
        X87::from_bits(0xC001_DFFE_5FBB_5C37_7FE8),
        X87::from_bits(0x3FC0_A765_268F_700B_7830),
    ),
    (
        X87::from_bits(0xC001_E001_9FEF_6FF0_F5BF),
        X87::from_bits(0x3FC0_EDF4_8C91_9E1F_5536),
    ),
    (
        X87::from_bits(0xC001_FFFF_CBFC_0ACE_7879),
        X87::from_bits(0xBFC0_A7A0_AD48_AC32_A74C),
    ),
    (
        X87::from_bits(0xC002_8000_1A01_459F_C9F6),
        X87::from_bits(0xBFBE_CB3C_EC1C_EC85_7667),
    ),
    (
        X87::from_bits(0xC002_8FFF_FD1C_425E_8100),
        X87::from_bits(0x3FC0_DE6C_5AA2_DB69_8607),
    ),
    (
        X87::from_bits(0xC002_9000_02E3_BB47_D86D),
        X87::from_bits(0xBFC1_DB08_7FDB_86A3_BD6F),
    ),
    (
        X87::from_bits(0xC002_9FFF_FFB6_06BD_FDCD),
        X87::from_bits(0xBFBD_C55C_EF4A_0A8F_8D3A),
    ),
    (
        X87::from_bits(0xC002_A000_0049_F93B_B992),
        X87::from_bits(0xBFC1_F68B_B2BC_2A88_3C06),
    ),
    (
        X87::from_bits(0xC002_AFFF_FFF9_466E_9F1B),
        X87::from_bits(0xBFC0_DB6B_34AB_6F46_3417),
    ),
    (
        X87::from_bits(0xC002_B000_0006_B991_5316),
        X87::from_bits(0x3FC0_9A69_6400_56FC_6D05),
    ),
    (
        X87::from_bits(0xC002_BFFF_FFFF_7089_3874),
        X87::from_bits(0x3FC1_F043_7CA6_7858_6D0F),
    ),
    (
        X87::from_bits(0xC002_C000_0000_8F76_C773),
        X87::from_bits(0xBFBF_AB3E_0781_2879_C3C9),
    ),
    (
        X87::from_bits(0xC002_CFFF_FFFF_F4F6_DCF6),
        X87::from_bits(0xBFBF_BFCB_D2FF_E3AB_EAA4),
    ),
    (
        X87::from_bits(0xC002_D000_0000_0B09_230A),
        X87::from_bits(0x3FC0_FE65_F08B_91BF_0724),
    ),
    (
        X87::from_bits(0xC002_DFFF_FFFF_FF36_345B),
        X87::from_bits(0x3FC1_8C3C_F6B8_3EC5_DD12),
    ),
    (
        X87::from_bits(0xC002_E000_0000_00C9_CBA5),
        X87::from_bits(0xBFC1_8BD2_9CEB_D8AE_31EF),
    ),
    (
        X87::from_bits(0xC002_EFFF_FFFF_FFF2_8C06),
        X87::from_bits(0xBFBE_C660_4EF3_0371_F89D),
    ),
    (
        X87::from_bits(0xC002_F000_0000_000D_73FA),
        X87::from_bits(0x3FBE_C664_2F1B_DF07_A161),
    ),
    (
        X87::from_bits(0xC002_FFFF_FFFF_FFFF_28C0),
        X87::from_bits(0xBFC1_C18C_C43E_A25C_E5CA),
    ),
    (
        X87::from_bits(0xC003_8000_0000_0000_6BA0),
        X87::from_bits(0x3FC1_C18C_C4BD_7B68_0D89),
    ),
    (
        X87::from_bits(0xC003_87FF_FFFF_FFFF_F9AB),
        X87::from_bits(0xBFC2_9C47_E7A9_3E1C_46A1),
    ),
    (
        X87::from_bits(0xC003_8800_0000_0000_0655),
        X87::from_bits(0x3FC2_9C47_E7A9_7778_935A),
    ),
    (
        X87::from_bits(0xC003_8FFF_FFFF_FFFF_FFA6),
        X87::from_bits(0x3FBF_9E18_EE5F_6697_BA6A),
    ),
    (
        X87::from_bits(0xC003_9000_0000_0000_005A),
        X87::from_bits(0xBFBF_9E18_EE5F_6526_1D98),
    ),
    (
        X87::from_bits(0xC003_97FF_FFFF_FFFF_FFFB),
        X87::from_bits(0xBFC2_85B2_5CBF_5F54_5CEE),
    ),
    (
        X87::from_bits(0xC003_9800_0000_0000_0005),
        X87::from_bits(0x3FC2_85B2_5CBF_5F54_7E48),
    ),
    (
        X87::from_bits(0xC003_A000_0000_0000_0000),
        X87::from_bits(0x3FC1_F2A1_5D20_1011_2853),
    ),
    (
        X87::from_bits(0xC003_A000_0000_0000_0000),
        X87::from_bits(0xBFC1_F2A1_5D20_1011_2828),
    ),
    (
        X87::from_bits(0xC003_A800_0000_0000_0000),
        X87::from_bits(0x3FBD_B8DC_77B6_E7AB_8C60),
    ),
    (
        X87::from_bits(0xC003_A800_0000_0000_0000),
        X87::from_bits(0xBFBD_B8DC_77B6_E7AB_8C5F),
    ),
    (
        X87::from_bits(0xC003_B000_0000_0000_0000),
        X87::from_bits(0x3FB9_8671_CB6D_BFC2_94A3),
    ),
    (
        X87::from_bits(0xC003_B000_0000_0000_0000),
        X87::from_bits(0xBFB9_8671_CB6D_BFC2_94A2),
    ),
    (
        X87::from_bits(0xC003_B800_0000_0000_0000),
        X87::from_bits(0x3FB4_BB0D_A098_B1C0_CECC),
    ),
    (
        X87::from_bits(0xC003_B800_0000_0000_0000),
        X87::from_bits(0xBFB4_BB0D_A098_B1C0_CECC),
    ),
    (
        X87::from_bits(0xC003_C000_0000_0000_0000),
        X87::from_bits(0x3FAF_F967_80CB_97AB_BE65),
    ),
    (
        X87::from_bits(0xC003_C000_0000_0000_0000),
        X87::from_bits(0xBFAF_F967_80CB_97AB_BE65),
    ),
    (
        X87::from_bits(0xC003_C800_0000_0000_0000),
        X87::from_bits(0x3FAB_9F9E_66E8_B2FD_46A7),
    ),
    (
        X87::from_bits(0xC003_C800_0000_0000_0000),
        X87::from_bits(0xBFAB_9F9E_66E8_B2FD_46A7),
    ),
    (
        X87::from_bits(0xC003_D000_0000_0000_0000),
        X87::from_bits(0x3FA6_C474_2FE3_5272_CD1C),
    ),
    (
        X87::from_bits(0xC003_D000_0000_0000_0000),
        X87::from_bits(0xBFA6_C474_2FE3_5272_CD1C),
    ),
    (
        X87::from_bits(0xC003_D800_0000_0000_0000),
        X87::from_bits(0x3FA1_E8D5_8E16_E675_1905),
    ),
    (
        X87::from_bits(0xC003_D800_0000_0000_0000),
        X87::from_bits(0xBFA1_E8D5_8E16_E675_1905),
    ),
    (
        X87::from_bits(0xC003_E000_0000_0000_0000),
        X87::from_bits(0x3F9D_850C_5131_A842_E9BA),
    ),
    (
        X87::from_bits(0xC003_E000_0000_0000_0000),
        X87::from_bits(0xBF9D_850C_5131_A842_E9BA),
    ),
    (
        X87::from_bits(0xC003_E800_0000_0000_0000),
        X87::from_bits(0x3F98_92CF_CC5A_1AC5_6BD6),
    ),
    (
        X87::from_bits(0xC003_E800_0000_0000_0000),
        X87::from_bits(0xBF98_92CF_CC5A_1AC5_6BD6),
    ),
    (
        X87::from_bits(0xC003_F000_0000_0000_0000),
        X87::from_bits(0x3F93_9C99_6282_3EB0_7306),
    ),
    (
        X87::from_bits(0xC003_F000_0000_0000_0000),
        X87::from_bits(0xBF93_9C99_6282_3EB0_7306),
    ),
    (
        X87::from_bits(0xC003_F800_0000_0000_0000),
        X87::from_bits(0x3F8E_A1A6_973C_1FAD_E217),
    ),
    (
        X87::from_bits(0xC003_F800_0000_0000_0000),
        X87::from_bits(0xBF8E_A1A6_973C_1FAD_E217),
    ),
    (
        X87::from_bits(0xC004_8000_0000_0000_0000),
        X87::from_bits(0x3F89_A1A6_973C_1FAD_E217),
    ),
    (
        X87::from_bits(0xC004_8000_0000_0000_0000),
        X87::from_bits(0xBF89_A1A6_973C_1FAD_E217),
    ),
    (
        X87::from_bits(0xC004_8400_0000_0000_0000),
        X87::from_bits(0x3F84_9CC0_92A6_E86A_8DAA),
    ),
];
/// The coefficients `B_2k / 2k(2k-1)` of Stirling's series for lgamma.
const LGAMMA_COEFF: [X87; 16] = [
    X87::from_bits(0x3FFB_AAAA_AAAA_AAAA_AAAB),
    X87::from_bits(0xBFF6_B60B_60B6_0B60_B60B),
    X87::from_bits(0x3FF4_D00D_00D0_0D00_D00D),
    X87::from_bits(0xBFF4_9C09_C09C_09C0_9C0A),
    X87::from_bits(0x3FF4_DCA8_F158_C7F9_1AB8),
    X87::from_bits(0xBFF5_FB55_86CC_C9E3_E410),
    X87::from_bits(0x3FF7_D20D_20D2_0D20_D20D),
    X87::from_bits(0xBFF9_F214_3658_7A9C_BEE1),
    X87::from_bits(0x3FFC_B7F4_B1C0_F033_FFD1),
    X87::from_bits(0xBFFF_B23B_3808_C0F9_CF6E),
    X87::from_bits(0x4002_D672_2191_6700_2D3A),
    X87::from_bits(0xC006_9CD9_292E_6660_D55B),
    X87::from_bits(0x400A_8911_A740_DA74_0DA7),
    X87::from_bits(0xC00E_8D0C_C570_E255_BF5A),
    X87::from_bits(0x4012_A8D1_044D_3708_D1C2),
    X87::from_bits(0xC016_E884_4D8A_169A_BBC4),
];
/// The polynomial approximations to `(|Γ(x)| - 1)(x - n)/(x - x0)` on the
/// eighths of [-3, -2], in terms of `x - xm` for the eighth's midpoint
/// `xm`, each ending at [`POLY_END`] with degree [`POLY_DEG`].
const POLY_COEFF: [X87; 121] = [
    X87::from_bits(0xBFFF_85B8_E2E2_A6A1_75B6),
    X87::from_bits(0xBFFE_C73A_1DC0_5F34_9517),
    X87::from_bits(0xBFFB_F642_0A04_2945_B5D7),
    X87::from_bits(0xBFFE_E37C_9DA2_6FC3_B492),
    X87::from_bits(0xBFFB_81E6_C3E2_8BC4_C8D0),
    X87::from_bits(0xBFFE_EAE9_ADA6_5ECE_2F39),
    X87::from_bits(0x3FFA_9B11_8550_5EDA_C18D),
    X87::from_bits(0xBFFE_EF28_C130_B54D_3CB2),
    X87::from_bits(0x3FFC_9BB0_599B_3D12_98EC),
    X87::from_bits(0xBFFE_F57C_B277_4193_BBD5),
    X87::from_bits(0x3FFD_8B5C_C8CE_3483_6388),
    X87::from_bits(0xBFFE_FF48_EA8B_5BD3_A7CE),
    X87::from_bits(0x3FFD_CFAE_6F11_51A6_1DEB),
    X87::from_bits(0xBFFF_88F0_725A_835E_9397),
    X87::from_bits(0xBFFE_F293_0890_D7D6_75A8),
    X87::from_bits(0xBFFE_CA5C_FDE0_54EA_B5CD),
    X87::from_bits(0x3FFC_E727_83F7_AEC1_9DB9),
    X87::from_bits(0xBFFF_8152_D69A_B02F_86C6),
    X87::from_bits(0x3FFE_96E9_B118_5D0B_92ED),
    X87::from_bits(0xBFFF_A6C1_9979_EB51_CAC8),
    X87::from_bits(0x3FFF_8E06_4656_6867_69F5),
    X87::from_bits(0xBFFF_E4D3_7AC9_533D_8B14),
    X87::from_bits(0x3FFF_EBF4_A3B7_CB55_25EB),
    X87::from_bits(0xC000_A487_3B6D_222E_CCC6),
    X87::from_bits(0x4000_BA2C_FF5B_2864_F932),
    X87::from_bits(0xC000_F2DA_7675_98A3_9288),
    X87::from_bits(0x4001_92BE_258E_76AB_0C70),
    X87::from_bits(0xC001_BA72_5A17_2F80_556C),
    X87::from_bits(0xBFFE_D7D2_8D50_5D61_8122),
    X87::from_bits(0xBFFE_E696_49A3_0409_8532),
    X87::from_bits(0x3FFE_B0D7_4A28_27D0_55C5),
    X87::from_bits(0xBFFF_C925_8491_4298_E007),
    X87::from_bits(0x3FFF_EA4D_895E_6772_7C44),
    X87::from_bits(0xC000_C226_2EDF_6EC8_7916),
    X87::from_bits(0x4001_840F_4D95_ADF4_2144),
    X87::from_bits(0xC001_C73D_CC61_68C1_275B),
    X87::from_bits(0x4002_8E2E_2521_1A3F_B5CC),
    X87::from_bits(0xC002_D0E8_5CCD_8E79_C080),
    X87::from_bits(0x4003_9722_C415_E0BF_CF0B),
    X87::from_bits(0xC003_DC5B_7420_C0AF_F98A),
    X87::from_bits(0x4004_9FFE_2FDD_413E_8137),
    X87::from_bits(0xC004_F064_BA72_40CD_4A5D),
    X87::from_bits(0x4005_B008_16EB_E9DE_0FF3),
    X87::from_bits(0xBFFE_B74E_A1BC_FFF9_4B2C),
    X87::from_bits(0xBFFF_9541_5EAC_861B_A9C2),
    X87::from_bits(0x3FFF_C401_07C1_45CB_431A),
    X87::from_bits(0xC000_CC89_E7C1_03AE_03E9),
    X87::from_bits(0x4001_AAF5_904A_2EB2_8631),
    X87::from_bits(0xC002_9C2A_EDCF_E10F_129E),
    X87::from_bits(0x4003_8960_9979_6F81_440F),
    X87::from_bits(0xC003_F54A_7136_05B7_FD30),
    X87::from_bits(0x4004_D9AD_2A2E_C0A4_004C),
    X87::from_bits(0xC005_C19E_E6BC_03EB_3216),
    X87::from_bits(0x4006_AC10_A8DB_7AE9_9343),
    X87::from_bits(0xC007_98F6_D909_598A_FF75),
    X87::from_bits(0x4008_87D1_E164_8A62_CFAC),
    X87::from_bits(0xC008_F151_B696_55EB_39B3),
    X87::from_bits(0x4009_E1C7_A540_4AF3_7496),
    X87::from_bits(0xC00A_CB1E_A961_C393_02F8),
    X87::from_bits(0xBFFC_F440_4230_9FAE_BEB5),
    X87::from_bits(0x3FFF_E6AA_BE57_FBE9_5901),
    X87::from_bits(0x4000_E066_D215_B4E6_540D),
    X87::from_bits(0x4001_D0A0_B975_A07B_A77B),
    X87::from_bits(0x4002_BC1B_2E65_3AA0_B924),
    X87::from_bits(0x4003_A852_9D1C_782F_B96B),
    X87::from_bits(0x4004_95EB_8032_F41B_BECD),
    X87::from_bits(0x4005_8562_AC60_EF4A_EFD2),
    X87::from_bits(0x4005_ED31_3DAF_2027_14FF),
    X87::from_bits(0x4006_D2DD_2841_7B4F_8406),
    X87::from_bits(0x4007_BB70_DB8F_8388_401D),
    X87::from_bits(0x4008_A69E_8258_952B_00C8),
    X87::from_bits(0x4009_941C_C213_C540_72F1),
    X87::from_bits(0x400A_837E_A17A_93C8_77B2),
    X87::from_bits(0x400A_E950_6A64_1143_612B),
    X87::from_bits(0x400B_DB40_76A7_51C3_6A90),
    X87::from_bits(0x400C_CA28_84C3_2378_2BA1),
    X87::from_bits(0xBFFD_D6BA_4A54_ADD1_4EA9),
    X87::from_bits(0x3FFF_946B_01C1_ED1D_6390),
    X87::from_bits(0x3FFF_EDB2_89D6_D453_381D),
    X87::from_bits(0x4000_B885_C463_E74D_2A9F),
    X87::from_bits(0x4001_88A0_2258_B97A_C4AC),
    X87::from_bits(0x4001_C95F_322A_3D2E_5F24),
    X87::from_bits(0x4002_92DB_598B_5B18_3CD6),
    X87::from_bits(0x4002_D62B_EF9C_9ADC_FF6A),
    X87::from_bits(0x4003_9BCF_9486_BA1E_CBBA),
    X87::from_bits(0x4003_E2C1_38FF_C11E_5513),
    X87::from_bits(0x4004_A4EA_1C6E_1E81_B9CC),
    X87::from_bits(0x4004_EFE7_6D98_40F5_FB26),
    X87::from_bits(0x4005_AE64_8F82_23F7_E32E),
    X87::from_bits(0x4005_FD91_732E_7744_FA01),
    X87::from_bits(0x4006_BECA_5F96_19B3_9C03),
    X87::from_bits(0x4007_8C79_7232_08E3_C58E),
    X87::from_bits(0xBFFE_8A41_B1E4_F36F_F88E),
    X87::from_bits(0x3FFE_CDA8_7D3B_69DC_0F34),
    X87::from_bits(0x3FFF_8A3A_56AE_1B0A_C569),
    X87::from_bits(0x3FFF_BB0F_65C8_62AA_9CCB),
    X87::from_bits(0x3FFF_E93C_DFFC_D711_A7C0),
    X87::from_bits(0x4000_9397_4015_685B_1505),
    X87::from_bits(0x4000_B55D_5169_E0E4_0BE3),
    X87::from_bits(0x4000_E145_3BB0_98EA_A7E6),
    X87::from_bits(0x4001_8A46_BC67_048B_EDFD),
    X87::from_bits(0x4001_AAC5_63DE_4016_4AD9),
    X87::from_bits(0x4001_D1D9_2F05_7277_AACA),
    X87::from_bits(0x4002_814B_AF48_3648_3508),
    X87::from_bits(0x4002_9EFA_F35D_C712_EA79),
    X87::from_bits(0x4002_C843_1F6A_2265_07A9),
    X87::from_bits(0x4002_F803_5828_9A76_8401),
    X87::from_bits(0xBFFE_A046_D667_E468_F3E4),
    X87::from_bits(0x3FFE_970B_88DC_C006_C216),
    X87::from_bits(0x3FFE_AA8A_3942_1C86_CE90),
    X87::from_bits(0x3FFE_D2F4_D136_3F32_1E89),
    X87::from_bits(0x3FFE_DCA9_AA1A_3AB2_F438),
    X87::from_bits(0x3FFE_FCF0_9C31_F05D_02CB),
    X87::from_bits(0x3FFF_8258_99D0_CAB4_351C),
    X87::from_bits(0x3FFF_915A_A3CC_E803_9012),
    X87::from_bits(0x3FFF_962C_015C_34D1_B574),
    X87::from_bits(0x3FFF_A556_F918_2AEB_882F),
    X87::from_bits(0x3FFF_ABCA_03C6_EA2E_2AEB),
    X87::from_bits(0x3FFF_BBAC_834E_D0C5_E785),
    X87::from_bits(0x3FFF_C733_9677_D231_1F9A),
    X87::from_bits(0x3FFF_D956_7D19_60B8_A2F3),
];
/// e as a high and a low part.
const E_HI: X87 = X87::from_bits(0x4000_ADF8_5458_A2BB_4A9B);
const E_LO: X87 = X87::from_bits(0xBFBF_A047_53BF_B185_861C);
