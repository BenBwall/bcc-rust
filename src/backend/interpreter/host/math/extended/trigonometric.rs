//! `sinl`, `cosl` and `tanl` as glibc's `sysdeps/ieee754/ldbl-96` C code
//! computes them on x86-64 (`s_sinl.c`, `s_cosl.c`, `s_tanl.c` with the
//! kernels `k_sinl.c`, `k_cosl.c` and `k_tanl.c`): an argument beyond π/4
//! is reduced modulo π/2 by `e_rem_pio2l.c`, which splits its significand
//! into three 24-bit pieces for the `double` Payne-Hanek reduction of
//! `dbl-64/k_rem_pio2.c`, and the kernels evaluate polynomials, near zero
//! directly and further out around the nearest of 83 table points. Every
//! `long double` step runs on the x87 model and every `double` step in the
//! host's binary64 arithmetic, which is SSE2's, so the results are glibc's
//! bit for bit.
//! C99: §7.12.4.5-§7.12.4.7, pp. 220-221; PDF pp. 232-233.

use super::{
    exact::{
        biased,
        words,
    },
    expression::{
        ONE,
        double,
        equal,
        greater_equal,
        integer,
        less,
        truncate,
    },
    sincos_table::SINCOS,
};
use crate::x87::X87;

/// `sinl`. An infinity or NaN gives `x - x`.
pub(super) fn sin(x: X87) -> X87 {
    let (se, i0, _) = words(x);
    let se = biased(se);
    if se < 0x3FFE || (se == 0x3FFE && i0 <= 0xC90F_DAA2) {
        return kernel_sin(x, X87::zero(false), false);
    }
    if se == 0x7FFF {
        return x.sub(x);
    }
    let (n, y0, y1) = rem_pio2(x);
    match n & 3 {
        | 0 => kernel_sin(y0, y1, true),
        | 1 => kernel_cos(y0, y1),
        | 2 => kernel_sin(y0, y1, true).negate(),
        | _ => kernel_cos(y0, y1).negate(),
    }
}

/// `cosl`.
pub(super) fn cos(x: X87) -> X87 {
    let (se, i0, _) = words(x);
    let se = biased(se);
    if se < 0x3FFE || (se == 0x3FFE && i0 <= 0xC90F_DAA2) {
        return kernel_cos(x, X87::zero(false));
    }
    if se == 0x7FFF {
        return x.sub(x);
    }
    let (n, y0, y1) = rem_pio2(x);
    match n & 3 {
        | 0 => kernel_cos(y0, y1),
        | 1 => kernel_sin(y0, y1, true).negate(),
        | 2 => kernel_cos(y0, y1).negate(),
        | _ => kernel_sin(y0, y1, true),
    }
}

/// `tanl`.
pub(super) fn tan(x: X87) -> X87 {
    let se = biased(words(x).0);
    if se <= 0x3FFE {
        return kernel_tan(x, X87::zero(false), 1);
    }
    if se == 0x7FFF {
        return x.sub(x);
    }
    let (n, y0, y1) = rem_pio2(x);
    kernel_tan(y0, y1, 1 - ((n & 1) << 1))
}

/// The coefficients `k_sinl.c` and `k_cosl.c` share: cosine and sine of
/// the small offset from a table point.
const SCOS: [X87; 5] = [
    X87::from_bits(0xBFFE_8000_0000_0000_0000),
    X87::from_bits(0x3FFA_AAAA_AAAA_AAAA_AAAB),
    X87::from_bits(0xBFF5_B60B_60B6_0B60_B60B),
    X87::from_bits(0x3FEF_D00D_00D0_0CF7_016E),
    X87::from_bits(0xBFE9_93F2_7AEE_731B_E586),
];
const SSIN: [X87; 5] = [
    X87::from_bits(0xBFFC_AAAA_AAAA_AAAA_AAAB),
    X87::from_bits(0x3FF8_8888_8888_8888_8889),
    X87::from_bits(0xBFF2_D00D_00D0_0D00_D00D),
    X87::from_bits(0x3FEC_B8EF_1D2A_B632_0635),
    X87::from_bits(0xBFE5_D732_2774_8623_EE39),
];

/// `k_sinl.c`'s polynomial for |x| < 0.1484375.
const SIN: [X87; 8] = [
    X87::from_bits(0xBFFC_AAAA_AAAA_AAAA_AAAB),
    X87::from_bits(0x3FF8_8888_8888_8888_8889),
    X87::from_bits(0xBFF2_D00D_00D0_0D00_D00D),
    X87::from_bits(0x3FEC_B8EF_1D2A_B639_9C7D),
    X87::from_bits(0xBFE5_D732_2B3F_AA27_0B64),
    X87::from_bits(0x3FDE_B092_309D_408A_402A),
    X87::from_bits(0xBFD6_D73F_9EAD_7798_63DE),
    X87::from_bits(0x3FCE_CA88_08AD_55FC_3D67),
];

/// `k_cosl.c`'s polynomial for |x| < 0.1484375.
const COS: [X87; 8] = [
    X87::from_bits(0xBFFE_8000_0000_0000_0000),
    X87::from_bits(0x3FFA_AAAA_AAAA_AAAA_AAAB),
    X87::from_bits(0xBFF5_B60B_60B6_0B60_B60B),
    X87::from_bits(0x3FEF_D00D_00D0_0D00_D00D),
    X87::from_bits(0xBFE9_93F2_7DBB_C4FA_D541),
    X87::from_bits(0x3FE2_8F76_C77F_C40E_8F49),
    X87::from_bits(0xBFDA_C9CB_A4B4_9154_B00F),
    X87::from_bits(0x3FD2_D72F_C0CB_E5E6_E57C),
];

/// 0.1484375, where the kernels switch to the table.
const TABLE_START: X87 = X87::from_bits(0x3FFC_9800_0000_0000_0000);

/// `0x1p-33L`.
const TINY: X87 = X87::from_bits(0x3FDE_8000_0000_0000_0000);

/// Evaluates `c[0] + z × (c[1] + z × (… + z × c[n]))`, innermost first.
pub(super) fn horner(z: X87, coefficients: &[X87]) -> X87 {
    let (&last, rest) = coefficients.split_last().expect("a polynomial");
    rest.iter()
        .rev()
        .fold(last, |sum, &coefficient| coefficient.add(z.mul(sum)))
}

/// The table point at or below |`x`| (`h`), its entry's index, and the
/// offset `l` of `x + y` from it, as both kernels find them.
fn table_point(absx: X87) -> (X87, usize) {
    let offset = absx.sub(TABLE_START.sub(double(1.0 / 256.0)));
    let index = truncate(integer(128).mul(offset));
    let h = TABLE_START.add(double(f64::from(index) / 128.0));
    // `index *= 4` wraps for the integer indefinite of a NaN, leaving 0.
    let entry = usize::try_from(index.wrapping_mul(4)).unwrap_or(0);
    (h, entry)
}

/// One of the four table values of `entry`: the high and low parts of the
/// cosine, then of the sine.
fn table(entry: usize, part: usize) -> X87 {
    SINCOS
        .get(entry + part)
        .map_or(X87::INDEFINITE, |&bits| X87::from_bits(bits))
}

/// The sine and cosine-minus-one polynomials of the table offset `l`.
fn offset_terms(l: X87) -> (X87, X87) {
    let z = l.mul(l);
    let sin_l = l.mul(ONE.add(z.mul(horner(z, &SSIN))));
    let cos_l_m1 = z.mul(horner(z, &SCOS));
    (sin_l, cos_l_m1)
}

/// `__kernel_sinl`: the sine of `x + y` for |x| ≤ π/4, where `y` is the
/// tail of a reduced argument when `iy`.
#[expect(
    clippy::many_single_char_names,
    reason = "glibc's names, so that the port reads line by line against its source."
)]
fn kernel_sin(x: X87, y: X87, iy: bool) -> X87 {
    let absx = x.with_sign(false);
    if less(absx, TABLE_START) {
        if less(absx, TINY) && truncate(x) == 0 {
            return x;
        }
        let z = x.mul(x);
        return x.add(x.mul(z.mul(horner(z, &SIN))));
    }
    let (h, entry) = table_point(absx);
    let l = if iy {
        let y = if less(x, X87::zero(false)) {
            y.negate()
        } else {
            y
        };
        y.sub(h.sub(absx))
    } else {
        absx.sub(h)
    };
    let (sin_l, cos_l_m1) = offset_terms(l);
    let sin_hi = table(entry, 2);
    let z = sin_hi.add(
        table(entry, 3)
            .add(sin_hi.mul(cos_l_m1))
            .add(table(entry, 0).mul(sin_l)),
    );
    if less(x, X87::zero(false)) {
        z.negate()
    } else {
        z
    }
}

/// `__kernel_cosl`: the cosine of `x + y` for |x| ≤ π/4.
fn kernel_cos(x: X87, y: X87) -> X87 {
    let (x, y) = if x.is_negative() {
        (x.negate(), y.negate())
    } else {
        (x, y)
    };
    if less(x, TABLE_START) {
        if less(x, TINY) && truncate(x) == 0 {
            return ONE;
        }
        let z = x.mul(x);
        return ONE.add(z.mul(horner(z, &COS)));
    }
    let (h, entry) = table_point(x);
    let l = y.sub(h.sub(x));
    let (sin_l, cos_l_m1) = offset_terms(l);
    let cos_hi = table(entry, 0);
    cos_hi.add(table(entry, 1).sub(table(entry, 2).mul(sin_l).sub(cos_hi.mul(cos_l_m1))))
}

/// `k_tanl.c`'s π/4 in two parts and its rational approximation
/// `tan x = x + x^3 / 3 + x^5 T(x^2) / U(x^2)`.
const PIO4_HI: X87 = X87::from_bits(0x3FFE_C90F_DAA2_2168_C235);
const PIO4_LO: X87 = X87::from_bits(0xBFBC_ECE6_75D1_FC8F_8CBB);
const THIRD: X87 = X87::from_bits(0x3FFD_AAAA_AAAA_AAAA_AAAB);
const T: [X87; 5] = [
    X87::from_bits(0xC017_8A52_718F_0823_2E04),
    X87::from_bits(0x4013_A139_FFAE_192F_1C8F),
    X87::from_bits(0xC00D_CD37_8272_DC07_AEAB),
    X87::from_bits(0x4006_B075_13F2_F71B_996C),
    X87::from_bits(0xBFFD_AAA9_CEA6_E2AD_A158),
];
const U: [X87; 6] = [
    X87::from_bits(0xC01A_81AD_4A76_17A0_FB24),
    X87::from_bits(0x4018_F7BD_9693_9ED3_4BF7),
    X87::from_bits(0xC014_FF2C_CEB0_5F23_A68D),
    X87::from_bits(0x400F_9CDE_3804_F332_880D),
    X87::from_bits(0xC008_8514_0A46_A179_62B0),
    ONE,
];

/// 0.6743316650390625, above which `k_tanl.c` works with π/4 - x.
const TAN_SWITCH: X87 = X87::from_bits(0x3FFE_ACA1_0000_0000_0000);

/// `__kernel_tanl`: the tangent of `x + y` for |x| ≤ π/4 when `iy` is 1,
/// else minus its reciprocal.
#[expect(
    clippy::many_single_char_names,
    reason = "glibc's names, so that the port reads line by line against its source."
)]
fn kernel_tan(x: X87, y: X87, iy: i32) -> X87 {
    let absx = x.with_sign(false);
    if less(absx, TINY) && truncate(x) == 0 {
        return if equal(x, X87::zero(false)) && iy == -1 {
            ONE.div(absx)
        } else if iy == 1 {
            x
        } else {
            ONE.negate().div(x)
        };
    }
    let switched = greater_equal(absx, TAN_SWITCH);
    let (mut x, mut y, mut sign_negative) = (x, y, false);
    if switched {
        if x.is_negative() {
            (x, y, sign_negative) = (x.negate(), y.negate(), true);
        }
        let z = PIO4_HI.sub(x);
        let w = PIO4_LO.sub(y);
        x = z.add(w);
        y = X87::zero(false);
    }
    let z = x.mul(x);
    let r = horner(z, &T);
    let v = horner(z, &U);
    let r = r.div(v);
    let s = z.mul(x);
    let r = y.add(z.mul(s.mul(r).add(y)));
    let r = r.add(THIRD.mul(s));
    let w = x.add(r);
    if switched {
        let v = integer(i64::from(iy));
        let w = v.sub(double(2.0).mul(x.sub(w.mul(w).div(w.add(v)).sub(r))));
        return if sign_negative { w.negate() } else { w };
    }
    if iy == 1 {
        w
    } else {
        double(-1.0).div(x.add(r))
    }
}

/// `__ieee754_rem_pio2l`: `x` reduced modulo π/2 to `y0 + y1` with
/// |y0| ≤ π/4, and the low bits of the quotient. An unnormal gives a NaN.
fn rem_pio2(x: X87) -> (i32, X87, X87) {
    let (se, i0, i1) = words(x);
    let negative = se & 0x8000 != 0;
    let j0 = i32::from(biased(se)) - 0x3FFF;
    if j0 < -1 {
        return (0, x, X87::zero(false));
    }
    if i0 & 0x8000_0000 == 0 {
        let nan = X87::from_bits(0x7FFF_C000_0000_0000_0000);
        return (0, nan, nan);
    }
    let pieces = [
        f64::from(i0 >> 8),
        f64::from(((i0 << 16) | (i1 >> 16)) & 0x00FF_FFFF),
        f64::from((i1 << 8) & 0x00FF_FFFF),
    ];
    let (n, [t0, t1]) = kernel_rem_pio2(pieces, j0 - 23);
    let (t0, t1) = (double(t0), double(t1));
    if negative {
        let y0 = t0.add(t1).negate();
        (-n, y0, t1.negate().sub(y0.add(t0)))
    } else {
        let y0 = t0.add(t1);
        (n, y0, t1.sub(y0.sub(t0)))
    }
}

/// `__kernel_rem_pio2` (`dbl-64/k_rem_pio2.c`) for three 24-bit pieces
/// `x` of a value `x × 2^e0` and two `double` results: the quotient's low
/// three bits and the remainder in two parts, computed with 2/π's bits.
#[expect(
    clippy::cast_possible_truncation,
    clippy::many_single_char_names,
    reason = "glibc's algorithm truncates doubles to int32, and keeps its names."
)]
fn kernel_rem_pio2(x: [f64; 3], e0: i32) -> (i32, [f64; 2]) {
    const TWO24: f64 = 16_777_216.0;
    const TWON24: f64 = f64::from_bits(0x3E70_0000_0000_0000);
    const PIO2: [f64; 8] = [
        f64::from_bits(0x3FF9_21FB_4000_0000),
        f64::from_bits(0x3E74_442D_0000_0000),
        f64::from_bits(0x3CF8_4698_8000_0000),
        f64::from_bits(0x3B78_CC51_6000_0000),
        f64::from_bits(0x39F0_1B83_8000_0000),
        f64::from_bits(0x387A_2520_4000_0000),
        f64::from_bits(0x36E3_8222_8000_0000),
        f64::from_bits(0x3569_F31D_0000_0000),
    ];
    let scalbn = |value: f64, power: i32| {
        crate::backend::interpreter::host::math::native::scalbn(value, power)
    };
    let ipio2 = |index: i32| {
        usize::try_from(index)
            .ok()
            .and_then(|index| TWO_OVER_PI.get(index))
            .map_or(0.0, |&digits| f64::from(digits))
    };
    let at = |index: i32| usize::try_from(index).expect("a nonnegative index");
    let jk = 4;
    let jp = jk;
    let jx = 2;
    let jv = ((e0 - 3) / 24).max(0);
    let mut q0 = e0 - 24 * (jv + 1);
    let mut f = [0.0_f64; 20];
    let mut q = [0.0_f64; 20];
    let mut iq = [0_i32; 20];
    let mut fq = [0.0_f64; 20];
    for i in 0..=jx + jk {
        let j = jv - jx + i;
        f[at(i)] = if j < 0 { 0.0 } else { ipio2(j) };
    }
    for i in 0..=jk {
        let mut fw = 0.0;
        for j in 0..=jx {
            fw += x[at(j)] * f[at(jx + i - j)];
        }
        q[at(i)] = fw;
    }
    let mut jz = jk;
    loop {
        // Distil q[] into iq[], most significant last.
        let mut z = q[at(jz)];
        let mut i = 0;
        let mut j = jz;
        while j > 0 {
            let fw = f64::from((TWON24 * z) as i32);
            iq[at(i)] = (z - TWO24 * fw) as i32;
            z = q[at(j - 1)] + fw;
            i += 1;
            j -= 1;
        }
        z = scalbn(z, q0);
        z -= 8.0 * (z * 0.125).floor();
        let mut n = z as i32;
        z -= f64::from(n);
        let mut ih = 0;
        if q0 > 0 {
            let i = iq[at(jz - 1)] >> (24 - q0);
            n += i;
            iq[at(jz - 1)] -= i << (24 - q0);
            ih = iq[at(jz - 1)] >> (23 - q0);
        } else if q0 == 0 {
            ih = iq[at(jz - 1)] >> 23;
        } else if z >= 0.5 {
            ih = 2;
        }
        if ih > 0 {
            n += 1;
            let mut carry = 0;
            for i in 0..jz {
                let j = iq[at(i)];
                if carry == 0 {
                    if j != 0 {
                        carry = 1;
                        iq[at(i)] = 0x0100_0000 - j;
                    }
                } else {
                    iq[at(i)] = 0x00FF_FFFF - j;
                }
            }
            match q0 {
                | 1 => iq[at(jz - 1)] &= 0x007F_FFFF,
                | 2 => iq[at(jz - 1)] &= 0x003F_FFFF,
                | _ => {},
            }
            if ih == 2 {
                z = 1.0 - z;
                if carry != 0 {
                    z -= scalbn(1.0, q0);
                }
            }
        }
        if z == 0.0 {
            let mut j = 0;
            let mut i = jz - 1;
            while i >= jk {
                j |= iq[at(i)];
                i -= 1;
            }
            if j == 0 {
                // Recompute with more terms of 2/π.
                let mut k = 1;
                while iq[at(jk - k)] == 0 {
                    k += 1;
                }
                for i in jz + 1..=jz + k {
                    f[at(jx + i)] = ipio2(jv + i);
                    let mut fw = 0.0;
                    for j in 0..=jx {
                        fw += x[at(j)] * f[at(jx + i - j)];
                    }
                    q[at(i)] = fw;
                }
                jz += k;
                continue;
            }
        }
        // Chop off zero terms, or break z into 24-bit pieces.
        if z == 0.0 {
            jz -= 1;
            q0 -= 24;
            while iq[at(jz)] == 0 {
                jz -= 1;
                q0 -= 24;
            }
        } else {
            z = scalbn(z, -q0);
            if z >= TWO24 {
                let fw = f64::from((TWON24 * z) as i32);
                iq[at(jz)] = (z - TWO24 * fw) as i32;
                jz += 1;
                q0 += 24;
                iq[at(jz)] = fw as i32;
            } else {
                iq[at(jz)] = z as i32;
            }
        }
        // Convert the integer chunks to floating point.
        let mut fw = scalbn(1.0, q0);
        let mut i = jz;
        while i >= 0 {
            q[at(i)] = fw * f64::from(iq[at(i)]);
            fw *= TWON24;
            i -= 1;
        }
        // fq[] = PIO2[0..=jp] × q[jz..=0].
        let mut i = jz;
        while i >= 0 {
            let mut fw = 0.0;
            let mut k = 0;
            while k <= jp && k <= jz - i {
                fw += PIO2[at(k)] * q[at(i + k)];
                k += 1;
            }
            fq[at(jz - i)] = fw;
            i -= 1;
        }
        // Compress fq[] into two doubles.
        let mut fv = 0.0;
        let mut i = jz;
        while i >= 0 {
            fv += fq[at(i)];
            i -= 1;
        }
        let y0 = if ih == 0 { fv } else { -fv };
        fv = fq[0] - fv;
        for i in 1..=jz {
            fv += fq[at(i)];
        }
        let y1 = if ih == 0 { fv } else { -fv };
        return (n & 7, [y0, y1]);
    }
}

/// 2/π in 24-bit pieces (`two_over_pi` in `e_rem_pio2l.c`).
#[rustfmt::skip]
const TWO_OVER_PI: [u32; 938] = [
    0x00A2_F983, 0x006E_4E44, 0x0015_29FC, 0x0027_57D1, 0x00F5_34DD, 0x00C0_DB62,
    0x0095_993C, 0x0043_9041, 0x00FE_5163, 0x00AB_DEBB, 0x00C5_61B7, 0x0024_6E3A,
    0x0042_4DD2, 0x00E0_0649, 0x002E_EA09, 0x00D1_921C, 0x00FE_1DEB, 0x001C_B129,
    0x00A7_3EE8, 0x0082_35F5, 0x002E_BB44, 0x0084_E99C, 0x0070_26B4, 0x005F_7E41,
    0x0039_91D6, 0x0039_8353, 0x0039_F49C, 0x0084_5F8B, 0x00BD_F928, 0x003B_1FF8,
    0x0097_FFDE, 0x0005_980F, 0x00EF_2F11, 0x008B_5A0A, 0x006D_1F6D, 0x0036_7ECF,
    0x0027_CB09, 0x00B7_4F46, 0x003F_669E, 0x005F_EA2D, 0x0075_27BA, 0x00C7_EBE5,
    0x00F1_7B3D, 0x0007_39F7, 0x008A_5292, 0x00EA_6BFB, 0x005F_B11F, 0x008D_5D08,
    0x0056_0330, 0x0046_FC7B, 0x006B_ABF0, 0x00CF_BC20, 0x009A_F436, 0x001D_A9E3,
    0x0091_615E, 0x00E6_1B08, 0x0065_9985, 0x005F_14A0, 0x0068_408D, 0x00FF_D880,
    0x004D_7327, 0x0031_0606, 0x0015_56CA, 0x0073_A8C9, 0x0060_E27B, 0x00C0_8C6B,
    0x0047_C419, 0x00C3_67CD, 0x00DC_E809, 0x002A_8359, 0x00C4_768B, 0x0096_1CA6,
    0x00DD_AF44, 0x00D1_5719, 0x0005_3EA5, 0x00FF_0705, 0x003F_7E33, 0x00E8_32C2,
    0x00DE_4F98, 0x0032_7DBB, 0x00C3_3D26, 0x00EF_6B1E, 0x005E_F89F, 0x003A_1F35,
    0x00CA_F27F, 0x001D_87F1, 0x0021_907C, 0x007C_246A, 0x00FA_6ED5, 0x0077_2D30,
    0x0043_3B15, 0x00C6_14B5, 0x009D_19C3, 0x00C2_C4AD, 0x0041_4D2C, 0x005D_000C,
    0x0046_7D86, 0x002D_71E3, 0x009A_C69B, 0x0000_6233, 0x007C_D2B4, 0x0097_A7B4,
    0x00D5_5537, 0x00F6_3ED7, 0x0018_10A3, 0x00FC_764D, 0x002A_9D64, 0x00AB_D770,
    0x00F8_7C63, 0x0057_B07A, 0x00E7_1517, 0x0056_49C0, 0x00D9_D63B, 0x0038_84A7,
    0x00CB_2324, 0x0077_8AD6, 0x0023_545A, 0x00B9_1F00, 0x001B_0AF1, 0x00DF_CE19,
    0x00FF_319F, 0x006A_1E66, 0x0061_5799, 0x0047_FBAC, 0x00D8_7F7E, 0x00B7_6522,
    0x0089_E832, 0x0060_BFE6, 0x00CD_C4EF, 0x0009_366C, 0x00D4_3F5D, 0x00D7_DE16,
    0x00DE_3B58, 0x0092_9BDE, 0x0028_22D2, 0x00E8_8628, 0x004D_58E2, 0x0032_CAC6,
    0x0016_E308, 0x00CB_7DE0, 0x0050_C017, 0x00A7_1DF3, 0x005B_E018, 0x0034_132E,
    0x0062_1283, 0x0001_4883, 0x005B_8EF5, 0x007F_B0AD, 0x00F2_E91E, 0x0043_4A48,
    0x00D3_6710, 0x00D8_DDAA, 0x0042_5FAE, 0x00CE_616A, 0x00A4_280A, 0x00B4_99D3,
    0x00F2_A606, 0x007F_775C, 0x0083_C2A3, 0x0088_3C61, 0x0078_738A, 0x005A_8CAF,
    0x00BD_D76F, 0x0063_A62D, 0x00CB_BFF4, 0x00EF_818D, 0x0067_C126, 0x0045_CA55,
    0x0036_D9CA, 0x00D2_A828, 0x008D_61C2, 0x0077_C912, 0x0014_2604, 0x009B_4612,
    0x00C4_59C4, 0x0044_C5C8, 0x0091_B24D, 0x00F3_1700, 0x00AD_43D4, 0x00E5_4929,
    0x0010_D5FD, 0x00FC_BE00, 0x00CC_941E, 0x00EE_CE70, 0x00F5_3E13, 0x0080_F1EC,
    0x00C3_E7B3, 0x0028_F8C7, 0x0094_0593, 0x003E_71C1, 0x00B3_092E, 0x00F3_450B,
    0x009C_1288, 0x007B_20AB, 0x009F_B52E, 0x00C2_9247, 0x002F_327B, 0x006D_550C,
    0x0090_A772, 0x001F_E76B, 0x0096_CB31, 0x004A_1679, 0x00E2_7941, 0x0089_DFF4,
    0x0097_94E8, 0x0084_E6E2, 0x0097_3199, 0x006B_ED88, 0x0036_5F5F, 0x000E_FDBB,
    0x00B4_9A48, 0x006C_A467, 0x0042_7271, 0x0032_5D8D, 0x00B8_159F, 0x0009_E5BC,
    0x0025_318D, 0x0039_74F7, 0x001C_0530, 0x0001_0C0D, 0x0068_084B, 0x0058_EE2C,
    0x0090_AA47, 0x0002_E774, 0x0024_D6BD, 0x00A6_7DF7, 0x0072_486E, 0x00EF_169F,
    0x00A6_948E, 0x00F6_91B4, 0x0051_53D1, 0x00F2_0ACF, 0x0033_9820, 0x007E_4BF5,
    0x0068_63B2, 0x005F_3EDD, 0x0003_5D40, 0x007F_8985, 0x0029_5255, 0x00C0_6437,
    0x0010_D86D, 0x0032_4832, 0x0075_4C5B, 0x00D4_714E, 0x006E_5445, 0x00C1_090B,
    0x0069_F52A, 0x00D5_6614, 0x009D_0727, 0x0050_045D, 0x00DB_3BB4, 0x00C5_76EA,
    0x0017_F987, 0x007D_6B49, 0x00BA_271D, 0x0029_6996, 0x00AC_CCC6, 0x0054_14AD,
    0x006A_E290, 0x0089_D988, 0x0050_722C, 0x00BE_A404, 0x0094_0777, 0x0070_30F3,
    0x0027_FC00, 0x00A8_71EA, 0x0049_C266, 0x003D_E064, 0x0083_DD97, 0x0097_3FA3,
    0x00FD_9443, 0x008C_860D, 0x00DE_4131, 0x009D_3992, 0x008C_70DD, 0x00E7_B717,
    0x003B_DF08, 0x002B_3715, 0x00A0_805C, 0x0093_805A, 0x0092_1110, 0x00D8_E80F,
    0x00AF_806C, 0x004B_FFDB, 0x000F_9038, 0x0076_1859, 0x0015_A562, 0x00BB_CB61,
    0x00B9_89C7, 0x00BD_4010, 0x0004_F2D2, 0x0027_7549, 0x00F6_B6EB, 0x00BB_22DB,
    0x00AA_140A, 0x002F_2689, 0x0076_8364, 0x0033_3B09, 0x001A_940E, 0x00AA_3A51,
    0x00C2_A31D, 0x00AE_EDAF, 0x0012_265C, 0x004D_C26D, 0x009C_7A2D, 0x0097_56C0,
    0x0083_3F03, 0x00F6_F009, 0x008C_402B, 0x0099_316D, 0x0007_B439, 0x0015_200C,
    0x005B_C3D8, 0x00C4_92F5, 0x004B_ADC6, 0x00A5_CA4E, 0x00CD_37A7, 0x0036_A9E6,
    0x0094_92AB, 0x0068_42DD, 0x00DE_6319, 0x00EF_8C76, 0x0052_8B68, 0x0037_DBFC,
    0x00AB_A1AE, 0x0031_15DF, 0x00A1_AE00, 0x00DA_FB0C, 0x0066_4D64, 0x00B7_05ED,
    0x0030_6529, 0x00BF_5657, 0x003A_FF47, 0x00B9_F96A, 0x00F3_BE75, 0x00DF_9328,
    0x0030_80AB, 0x00F6_8C66, 0x0015_CB04, 0x0006_22FA, 0x001D_E4D9, 0x00A4_B33D,
    0x008F_1B57, 0x0009_CD36, 0x00E9_424E, 0x00A4_BE13, 0x00B5_2333, 0x001A_AAF0,
    0x00A8_654F, 0x00A5_C1D2, 0x000F_3F0B, 0x00CD_785B, 0x0076_F923, 0x0004_8B7B,
    0x0072_1789, 0x0053_A6C6, 0x00E2_6E6F, 0x0000_EBEF, 0x0058_4A9B, 0x00B7_DAC4,
    0x00BA_66AA, 0x00CF_CF76, 0x001D_02D1, 0x002D_F1B1, 0x00C1_998C, 0x0077_ADC3,
    0x00DA_4886, 0x00A0_5DF7, 0x00F4_80C6, 0x002F_F0AC, 0x009A_ECDD, 0x00BC_5C3F,
    0x006D_DED0, 0x001F_C790, 0x00B6_DB2A, 0x003A_25A3, 0x009A_AF00, 0x0093_53AD,
    0x0004_57B6, 0x00B4_2D29, 0x007E_804B, 0x00A7_07DA, 0x000E_AA76, 0x00A1_597B,
    0x002A_1216, 0x002D_B7DC, 0x00FD_E5FA, 0x00FE_DB89, 0x00FD_BE89, 0x006C_76E4,
    0x00FC_A906, 0x0070_803E, 0x0015_6E85, 0x00FF_87FD, 0x0007_3E28, 0x0033_6761,
    0x0086_182A, 0x00EA_BD4D, 0x00AF_E7B3, 0x006E_6D8F, 0x0039_6795, 0x005B_BF31,
    0x0048_D784, 0x0016_DF30, 0x0043_2DC7, 0x0035_6125, 0x00CE_70C9, 0x00B8_CB30,
    0x00FD_6CBF, 0x00A2_00A4, 0x00E4_6C05, 0x00A0_DD5A, 0x0047_6F21, 0x00D2_1262,
    0x0084_5CB9, 0x0049_6170, 0x00E0_566B, 0x0001_5299, 0x0037_5550, 0x00B7_D51E,
    0x00C4_F133, 0x005F_6E13, 0x00E4_305D, 0x00A9_2E85, 0x00C3_B21D, 0x0036_32A1,
    0x00A4_B708, 0x00D4_B1EA, 0x0021_F716, 0x00E4_698F, 0x0077_FF27, 0x0080_030C,
    0x002D_408D, 0x00A0_CD4F, 0x0099_A520, 0x00D3_A2B3, 0x000A_5D2F, 0x0042_F9B4,
    0x00CB_DA11, 0x00D0_BE7D, 0x00C1_DB9B, 0x00BD_17AB, 0x0081_A2CA, 0x005C_6A08,
    0x0017_552E, 0x0055_0027, 0x00F0_147F, 0x0086_07E1, 0x0064_0B14, 0x008D_4196,
    0x00DE_BE87, 0x002A_FDDA, 0x00B6_256B, 0x0034_897B, 0x00FE_F305, 0x009E_BFB9,
    0x004F_6A68, 0x00A8_2A4A, 0x005A_C44F, 0x00BC_F82D, 0x0098_5AD7, 0x0095_C7F4,
    0x008D_4D0D, 0x00A6_3A20, 0x005F_57A4, 0x00B1_3F14, 0x0095_3880, 0x0001_20CC,
    0x0086_DD71, 0x00B6_DEC9, 0x00F5_60BF, 0x0011_654D, 0x006B_0701, 0x00AC_B08C,
    0x00D0_C0B2, 0x0048_5551, 0x000E_FB1E, 0x00C3_7295, 0x003B_06A3, 0x0035_40C0,
    0x007B_DC06, 0x00CC_45E0, 0x00FA_294E, 0x00C8_CAD6, 0x0041_F3E8, 0x00DE_647C,
    0x00D8_649B, 0x0031_BED9, 0x00C3_97A4, 0x00D4_5877, 0x00C5_E369, 0x0013_DAF0,
    0x003C_3ABA, 0x0046_1846, 0x005F_7555, 0x00F5_BDD2, 0x00C6_926E, 0x005D_2EAC,
    0x00ED_440E, 0x0042_3E1C, 0x0087_C461, 0x00E9_FD29, 0x00F3_D6E7, 0x00CA_7C22,
    0x0035_916F, 0x00C5_E008, 0x008D_D7FF, 0x00E2_6A6E, 0x00C6_FDB0, 0x00C1_0893,
    0x0074_5D7C, 0x00B2_AD6B, 0x009D_6ECD, 0x007B_723E, 0x006A_11C6, 0x00A9_CFF7,
    0x00DF_7329, 0x00BA_C9B5, 0x0051_00B7, 0x000D_B2E2, 0x0024_BA74, 0x0060_7DE5,
    0x008A_D874, 0x002C_150D, 0x000C_1881, 0x0094_667E, 0x0016_2901, 0x0076_7A9F,
    0x00BE_FDFD, 0x00EF_4556, 0x0036_7ED9, 0x0013_D9EC, 0x00B9_BA8B, 0x00FC_97C4,
    0x0027_A831, 0x00C3_6EF1, 0x0036_C594, 0x0056_A8D8, 0x00B5_A8B4, 0x000E_CCCF,
    0x002D_8912, 0x0034_576F, 0x0089_562C, 0x00E3_CE99, 0x00B9_20D6, 0x00AA_5E6B,
    0x009C_2A3E, 0x00CC_5F11, 0x004A_0BFD, 0x00FB_F4E1, 0x006D_3B8E, 0x002C_86E2,
    0x0084_D4E9, 0x00A9_B4FC, 0x00D1_EEEF, 0x00C9_352E, 0x0061_392F, 0x0044_2138,
    0x00C8_D91B, 0x000A_FC81, 0x006A_4AFB, 0x00D8_1C2F, 0x0084_B453, 0x008C_994E,
    0x00CC_2254, 0x00DC_552A, 0x00D6_C6C0, 0x0096_190B, 0x00B8_701A, 0x0064_9569,
    0x0060_5A26, 0x00EE_523F, 0x000F_117F, 0x0011_B5F4, 0x00F5_CBFC, 0x002D_BC34,
    0x00EE_BC34, 0x00CC_5DE8, 0x0060_5EDD, 0x009B_8E67, 0x00EF_3392, 0x00B8_17C9,
    0x009B_5861, 0x00BC_57E1, 0x00C6_8351, 0x0010_3ED8, 0x0048_71DD, 0x00DD_1C2D,
    0x00A1_18AF, 0x0046_2C21, 0x00D7_F359, 0x0098_7AD9, 0x00C0_549E, 0x00FA_864F,
    0x00FC_0656, 0x00AE_79E5, 0x0036_2289, 0x0022_AD38, 0x00DC_9367, 0x00AA_E855,
    0x0038_2682, 0x009B_E7CA, 0x00A4_0D51, 0x00B1_3399, 0x000E_D7A9, 0x0048_0569,
    0x00F0_B265, 0x00A7_887F, 0x0097_4C88, 0x0036_D1F9, 0x00B3_9221, 0x004A_827B,
    0x0021_CF98, 0x00DC_9F40, 0x0055_47DC, 0x003A_74E1, 0x0042_EB67, 0x00DF_9DFE,
    0x005F_D45E, 0x00A4_677B, 0x007A_ACBA, 0x00A2_F655, 0x0023_882B, 0x0055_BA41,
    0x0008_6E59, 0x0086_2A21, 0x0083_4739, 0x00E6_E389, 0x00D4_9EE5, 0x0040_FB49,
    0x00E9_56FF, 0x00CA_0F1C, 0x008A_59C5, 0x002B_FA94, 0x00C5_C1D3, 0x00CF_C50F,
    0x00AE_5ADB, 0x0086_C547, 0x0062_4385, 0x003B_8621, 0x0094_792C, 0x0087_6110,
    0x007B_4C2A, 0x001A_2C80, 0x0012_BF43, 0x0090_2688, 0x0089_3C78, 0x00E4_C4A8,
    0x007B_DBE5, 0x00C2_3AC4, 0x00EA_F426, 0x008A_67F7, 0x00BF_920D, 0x002B_A365,
    0x00B1_933D, 0x000B_7CBD, 0x00DC_51A4, 0x0063_DD27, 0x00DD_E169, 0x0019_949A,
    0x0095_29A8, 0x0028_CE68, 0x00B4_ED09, 0x0020_9F44, 0x00CA_984E, 0x0063_8270,
    0x0023_7C7E, 0x0032_B90F, 0x008E_F5A7, 0x00E7_5614, 0x0008_F121, 0x002A_9DB5,
    0x004D_7E6F, 0x0051_19A5, 0x00AB_F9B5, 0x00D6_DF82, 0x0061_DD96, 0x0002_3616,
    0x009F_3AC4, 0x00A1_A283, 0x006D_ED72, 0x007A_8D39, 0x00A9_B882, 0x005C_326B,
    0x005B_2746, 0x00ED_3400, 0x0077_00D2, 0x0055_F4FC, 0x004D_5901, 0x0080_71E0,
    0x00E1_3F89, 0x00B2_95F3, 0x0064_A8F1, 0x00AE_A74B, 0x0038_FC4C, 0x00EA_B2BB,
    0x0047_270B, 0x00AB_C3A7, 0x0034_BA60, 0x0052_DD34, 0x00F8_563A, 0x00EB_7E8A,
    0x0031_BB36, 0x0058_95B7, 0x0047_F7A9, 0x0094_C3AA, 0x00D3_9225, 0x001E_7F3E,
    0x00D8_974E, 0x00BB_A94F, 0x00D8_AE01, 0x00E6_61B4, 0x0039_3D8E, 0x00A5_23AA,
    0x0033_068E, 0x0016_33B5, 0x003B_B188, 0x001D_3A9D, 0x0040_13D0, 0x00CC_1BE5,
    0x00F8_62E7, 0x003B_F28F, 0x0039_B5BF, 0x000B_C235, 0x0022_747E, 0x00A2_47C0,
    0x00D5_2D1F, 0x0019_ADD3, 0x0090_94DF, 0x0093_11D0, 0x00B4_2B25, 0x0049_6DB2,
    0x00E2_64B2, 0x005E_F135, 0x003B_C6A4, 0x001A_4AD0, 0x00AA_C92E, 0x0064_E886,
    0x0057_3091, 0x0098_2CFB, 0x0031_1B1A, 0x0008_728B, 0x00BD_CEE1, 0x0060_E142,
    0x00EB_641D, 0x00D0_BBA3, 0x00E5_59D4, 0x0059_7B8C, 0x002A_4483, 0x00F3_32BA,
    0x00F8_4867, 0x002C_8D1B, 0x002F_A9B0, 0x0050_F3DD, 0x00F9_F573, 0x00DB_61B4,
    0x00FE_233E, 0x006C_41A6, 0x00EE_A318, 0x0077_5A26, 0x00BC_5E5C, 0x00CE_A708,
    0x0094_DC57, 0x00E2_0196, 0x00F1_E839, 0x00BE_4851, 0x005D_2D2F, 0x004E_9555,
    0x00D9_6EC2, 0x00E7_D755, 0x0063_04E0, 0x00C0_2E0E, 0x00FC_40A0, 0x00BB_F9B3,
    0x0071_25A7, 0x0022_2DFB, 0x00F6_19D8, 0x0083_8C1C, 0x0066_19E6, 0x00B2_0D55,
    0x00BB_5137, 0x0079_E809, 0x00AF_9149, 0x000D_73DE, 0x000B_0DA5, 0x00CE_7F58,
    0x00AC_1934, 0x0072_4667, 0x007A_1A13, 0x009E_26BC, 0x0045_55E7, 0x0058_5CB5,
    0x0071_1D14, 0x0048_6991, 0x0048_0D60, 0x0056_ADAB, 0x00D6_2F64, 0x0096_EE0C,
    0x0021_2FF3, 0x005D_6D88, 0x00A6_7684, 0x0095_651E, 0x00AB_9E0A, 0x004D_DEFE,
    0x0057_1010, 0x0083_6A39, 0x00F8_EA31, 0x009E_381D, 0x00EA_C8B1, 0x00CA_C96B,
    0x0037_F21E, 0x00D5_05E9, 0x0098_4743, 0x009F_C56C, 0x0003_31B7, 0x003B_8BF8,
    0x0086_E56A, 0x008D_C343, 0x0062_30E7, 0x0093_CFD5, 0x006A_8F2D, 0x0073_3005,
    0x001A_F021, 0x00A0_9FCB, 0x0074_15A1, 0x00D5_6B23, 0x006F_F725, 0x002F_4BC7,
    0x00B8_A591, 0x007F_AC59, 0x005C_55DE, 0x0021_2C38, 0x00B1_3296, 0x005C_FF50,
    0x0036_6262, 0x00FA_7B16, 0x00F4_D9A6, 0x002A_CFE7, 0x00F0_7403, 0x00D4_D604,
    0x006F_D916, 0x0031_B1BF, 0x00CB_B450, 0x005B_D7C8, 0x000C_E194, 0x006B_D643,
    0x004F_D91C, 0x00DF_4543, 0x005F_3453, 0x00E2_B5AA, 0x00C9_AEC8, 0x0013_1485,
    0x00F9_D2BF, 0x00BA_DB9E, 0x0076_F5B9, 0x00AF_15CF, 0x00CA_3182, 0x0014_B56D,
    0x00E9_FE4D, 0x0050_FC35, 0x00F5_AED5, 0x00A2_D0C1, 0x00C9_6057, 0x0019_2EB6,
    0x00E9_1D92, 0x0007_D144, 0x00AE_A3C6, 0x0034_3566, 0x0026_D5B4, 0x0031_61E2,
    0x0037_F1A2, 0x0020_9EFF, 0x0095_8E23, 0x0049_3798, 0x0035_F4A6, 0x004B_DC02,
    0x00C2_BE13, 0x00BE_80A0, 0x000B_72A3, 0x0011_5C5F, 0x001E_1BD1, 0x000D_B4D3,
    0x0086_9E85, 0x0096_976B, 0x002A_C91F, 0x008A_26C2, 0x0030_70F0, 0x0004_1412,
    0x00FC_9FA5, 0x00F7_2A38, 0x009C_6878, 0x00E2_AA76, 0x0050_CFE1, 0x0055_9274,
    0x0093_4E38, 0x000A_92F7, 0x0055_33F0, 0x00A6_3DB4, 0x0039_9971, 0x00E2_B755,
    0x00A9_8A7C, 0x0000_8F19, 0x00AC_54D2, 0x002E_A0B4, 0x00F5_F3E0, 0x0060_C849,
    0x00FF_D269, 0x00AE_52CE, 0x007A_5FDD, 0x00E9_CE06, 0x00FB_0AE8, 0x00A5_0CCE,
    0x00EA_9D3E, 0x0037_66DD, 0x00B8_34F5, 0x000D_A090, 0x0084_6F88, 0x004A_E3D5,
    0x0009_9A03, 0x002E_AE2D, 0x00FC_B40A, 0x00FB_9B33, 0x00E2_81DD, 0x001B_16BA,
    0x00D8_C0AF, 0x00D9_6B97, 0x00B5_2DC9, 0x009C_277F, 0x0059_51D5, 0x0021_CCD6,
    0x00B6_496B, 0x0058_4562, 0x00B3_BAF2, 0x00A1_A5C4, 0x007C_A2CF, 0x00A9_B93D,
    0x007B_7B89, 0x0048_3D38,
];
