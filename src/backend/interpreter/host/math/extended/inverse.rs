//! The inverse trigonometric functions. glibc's x86-64 `atanl`, `atan2l`
//! and `acosl` are the i386 `fpatan` code (`sysdeps/i386/fpu/s_atanl.c`,
//! `e_atan2l.c` and `e_acosl.c`), so the processor computes their angle
//! (`processor.rs`); `asinl` is `sysdeps/ieee754/ldbl-96/e_asinl.c`, a
//! rational approximation computed here on the x87 model. The wrappers of
//! `acosl` and `asinl` (`w_acosl_compat.c`, `w_asinl_compat.c`) return
//! `NAN` for |x| > 1.
//! C99: §7.12.4.1-§7.12.4.4, pp. 218-220; PDF pp. 230-232.

use super::{
    exact::{
        from_words,
        words,
    },
    exponential::NAN,
    expression::{
        ONE,
        double,
        greater,
    },
    processor::fpatan,
};
use crate::{
    backend::interpreter::trap::Fault,
    x87::X87,
};

/// `atanl`: `fpatan` of `x / 1`.
pub(super) fn atan(x: X87) -> Result<X87, Fault> {
    fpatan(ONE, x)
}

/// `atan2l`; its wrapper only sets `errno`.
pub(super) fn atan2(y: X87, x: X87) -> Result<X87, Fault> {
    fpatan(x, y)
}

/// `acosl`: the angle of (x, √((1 - x)(1 + x))), or `NAN` for |x| > 1.
pub(super) fn acos(x: X87) -> Result<X87, Fault> {
    if greater(x.with_sign(false), ONE) {
        return Ok(NAN);
    }
    let product = ONE.sub(x).mul(ONE.add(x));
    fpatan(x, product.sqrt().with_sign(false))
}

/// `e_asinl.c`'s π/2 and π/4 to 64 bits, and its rational approximation
/// `asin(x) = x + x^3 pS(x^2) / qS(x^2)` for 0 <= x <= 0.5.
const PIO2_HI: X87 = X87::from_bits(0x3FFF_C90F_DAA2_2168_C235);
const PIO2_LO: X87 = X87::from_bits(0xBFBD_ECE6_75D1_FC8F_8CBB);
const PIO4_HI: X87 = X87::from_bits(0x3FFE_C90F_DAA2_2168_C235);
const HUGE: X87 = X87::from_bits(0x7FFE_D72C_B2A9_5C7E_F6CD);
const P_S: [X87; 6] = [
    X87::from_bits(0xC002_A164_F3CF_C6A9_EA38),
    X87::from_bits(0x4003_BA84_4EA5_9E25_3B0F),
    X87::from_bits(0xC003_950D_B77B_8D86_9BD4),
    X87::from_bits(0x4001_BDC5_D4DC_6810_8BF5),
    X87::from_bits(0xBFFE_9CB4_7FAB_93E6_BE97),
    X87::from_bits(0x3FF6_F786_7009_3509_59D1),
];
const Q_S: [X87; 6] = [
    X87::from_bits(0xC004_F217_6DB7_A9FE_DED7),
    X87::from_bits(0x4006_A71F_7754_1355_4C69),
    X87::from_bits(0xC006_AAC8_B4FD_BE0F_E11A),
    X87::from_bits(0x4005_9D67_E943_F36F_B5C3),
    X87::from_bits(0xC002_FAF3_09E9_8516_1229),
    ONE,
];

/// `asinl`, or `NAN` for |x| > 1.
#[expect(
    clippy::many_single_char_names,
    reason = "glibc's names, so that the port reads line by line against its source."
)]
pub(super) fn asin(x: X87) -> X87 {
    if greater(x.with_sign(false), ONE) {
        return NAN;
    }
    let (se, i0, i1) = words(x);
    let ix = (u32::from(se & 0x7FFF) << 16) | (i0 >> 16);
    if ix >= 0x3FFF_8000 {
        if ix == 0x3FFF_8000 && (i0.wrapping_sub(0x8000_0000) | i1) == 0 {
            return x.mul(PIO2_HI).add(x.mul(PIO2_LO));
        }
        let difference = x.sub(x);
        return difference.div(difference);
    }
    let rational = |t: X87| {
        let p = t.mul(super::trigonometric::horner(t, &P_S));
        let q = super::trigonometric::horner(t, &Q_S);
        (p, q)
    };
    if ix < 0x3FFE_8000 {
        if ix < 0x3FDE_8000 {
            if greater(HUGE.add(x), ONE) {
                return x;
            }
        } else {
            let t = x.mul(x);
            let (p, q) = rational(t);
            return x.add(x.mul(p.div(q)));
        }
    }
    let w = ONE.sub(x.with_sign(false));
    let t = w.mul(double(0.5));
    let (p, q) = rational(t);
    let s = t.sqrt();
    let t = if ix >= 0x3FFE_F999 {
        let w = p.div(q);
        PIO2_HI.sub(double(2.0).mul(s.add(s.mul(w))).sub(PIO2_LO))
    } else {
        let (k, i0, _) = words(s);
        let w = from_words(k, i0, 0);
        let c = t.sub(w.mul(w)).div(s.add(w));
        let r = p.div(q);
        let p = double(2.0)
            .mul(s)
            .mul(r)
            .sub(PIO2_LO.sub(double(2.0).mul(c)));
        let q = PIO4_HI.sub(double(2.0).mul(w));
        PIO4_HI.sub(p.sub(q))
    };
    if se & 0x8000 == 0 { t } else { t.negate() }
}
