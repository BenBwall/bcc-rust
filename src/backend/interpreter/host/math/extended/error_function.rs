//! `erfl` and `erfcl`, as glibc's `sysdeps/ieee754/ldbl-96/s_erfl.c`
//! computes them: rational approximations on four intervals, and beyond
//! 1.25 `exp(-x^2 - 0.5625 + R/S) / x` with the square split so that
//! `expl` (`exponential.rs`) sees exact arguments. Neither has a wrapper.
//! C99: §7.12.8.1-§7.12.8.2, p. 230; PDF p. 242.

use super::{
    exact::{
        biased,
        from_words,
        words,
    },
    exponential::{
        Base,
        exp,
    },
    expression::{
        HALF,
        ONE,
        TWO,
        double,
        integer,
    },
    trigonometric::horner,
};
use crate::{
    backend::interpreter::trap::Fault,
    x87::X87,
};

/// `erfl`.
pub(super) fn erf(x: X87) -> Result<X87, Fault> {
    let (se, i0, _) = words(x);
    let negative = se & 0x8000 != 0;
    if biased(se) == 0x7FFF {
        let i = i64::from(se >> 15) << 1;
        return Ok(integer(1 - i).add(ONE.div(x)));
    }
    let ix = (u32::from(biased(se)) << 16) | (i0 >> 16);
    if ix < 0x3FFE_D800 {
        if ix < 0x3FDE_8000 {
            if ix < 0x0008_0000 {
                return Ok(double(0.0625).mul(double(16.0).mul(x).add(EFX16.mul(x))));
            }
            return Ok(x.add(EFX.mul(x)));
        }
        let z = x.mul(x);
        let r = horner(z, &PP);
        let s = horner(z, &QQ);
        return Ok(x.add(x.mul(r.div(s))));
    }
    if ix < 0x3FFF_A000 {
        let (p, q) = near_one(x);
        return Ok(if negative {
            ERX.negate().sub(p.div(q))
        } else {
            ERX.add(p.div(q))
        });
    }
    if ix >= 0x4001_D555 {
        return Ok(if negative {
            TINY.sub(ONE)
        } else {
            ONE.sub(TINY)
        });
    }
    let x = x.with_sign(false);
    let s = ONE.div(x.mul(x));
    let (big_r, big_s) = if ix < 0x4000_B6DB {
        (horner(s, &RA), horner(s, &SA))
    } else {
        (horner(s, &RB), horner(s, &SB))
    };
    let (hx, i0, _) = words(x);
    let r = tail(x, from_words(hx, i0, 0), big_r, big_s)?;
    Ok(if negative {
        r.div(x).sub(ONE)
    } else {
        ONE.sub(r.div(x))
    })
}

/// `erfcl`.
pub(super) fn erfc(x: X87) -> Result<X87, Fault> {
    let (se, i0, _) = words(x);
    let negative = se & 0x8000 != 0;
    if biased(se) == 0x7FFF {
        return Ok(integer(i64::from(se >> 15) << 1).add(ONE.div(x)));
    }
    let ix = (u32::from(biased(se)) << 16) | (i0 >> 16);
    if ix < 0x3FFE_D800 {
        if ix < 0x3FBE_0000 {
            return Ok(ONE.sub(x));
        }
        let z = x.mul(x);
        let y = horner(z, &PP).div(horner(z, &QQ));
        if ix < 0x3FFD_8000 {
            return Ok(ONE.sub(x.add(x.mul(y))));
        }
        let r = x.mul(y).add(x.sub(HALF));
        return Ok(HALF.sub(r));
    }
    if ix < 0x3FFF_A000 {
        let (p, q) = near_one(x);
        return Ok(if negative {
            ONE.add(ERX.add(p.div(q)))
        } else {
            ONE.sub(ERX).sub(p.div(q))
        });
    }
    if ix >= 0x4005_D600 {
        return Ok(if negative {
            TWO.sub(TINY)
        } else {
            TINY.mul(TINY)
        });
    }
    let x = x.with_sign(false);
    let s = ONE.div(x.mul(x));
    let (big_r, big_s) = if ix < 0x4000_B6DB {
        (horner(s, &RA), horner(s, &SA))
    } else if ix < 0x4001_D555 {
        (horner(s, &RB), horner(s, &SB))
    } else {
        if negative {
            return Ok(TWO.sub(TINY));
        }
        (horner(s, &RC), horner(s, &SC))
    };
    let (hx, i0, _) = words(x);
    let r = tail(x, from_words(hx, i0 & 0xFFFF_FF00, 0), big_r, big_s)?;
    Ok(if negative {
        TWO.sub(r.div(x))
    } else {
        r.div(x)
    })
}

/// The rational approximation `P / Q` of `erf(1 + s) - erx` both functions
/// use for 0.84375 <= |x| < 1.25.
fn near_one(x: X87) -> (X87, X87) {
    let s = x.with_sign(false).sub(ONE);
    (horner(s, &PA), horner(s, &QA))
}

/// `exp(-z² - 0.5625) × exp((z - x)(z + x) + R/S)` for `z` the value `x`
/// with low significand bits cleared, so that `-z²` is exact.
fn tail(x: X87, z: X87, big_r: X87, big_s: X87) -> Result<X87, Fault> {
    let first = exp(z.negate().mul(z).sub(double(0.5625)), Base::E)?;
    let second = exp(z.sub(x).mul(z.add(x)).add(big_r.div(big_s)), Base::E)?;
    Ok(first.mul(second))
}

/// `s_erfl.c`'s constants: its tiny value, `erx` = (float) 0.84506291151,
/// `efx` = 2/√π - 1 and 16 times it.
const TINY: X87 = X87::from_bits(0x0002_BE5B_66EC_BCE0_B7B1);
const ERX: X87 = X87::from_bits(0x3FFE_D856_0B00_0000_0000);
const EFX: X87 = X87::from_bits(0x3FFC_8375_D410_A6DB_446C);
const EFX16: X87 = X87::from_bits(0x4000_8375_D410_A6DB_446C);

/// The rational approximations' coefficients, each denominator ending in
/// an implicit 1.
const PP: [X87; 6] = [
    X87::from_bits(0x4013_890D_FACE_C680_CB0C),
    X87::from_bits(0xC014_AB6B_5535_3EE2_51F9),
    X87::from_bits(0xC011_A1D5_1188_7DC5_E424),
    X87::from_bits(0xC00F_85C3_6C1D_991D_946E),
    X87::from_bits(0xC00A_A61D_1547_77EB_9519),
    X87::from_bits(0xC006_A587_F199_9B16_4FE8),
];

const QQ: [X87; 7] = [
    X87::from_bits(0x4016_8572_745F_3EF6_24EE),
    X87::from_bits(0x4014_E4A3_D90F_25C1_EEE9),
    X87::from_bits(0x4012_AC84_BE0D_9554_997B),
    X87::from_bits(0x400F_917C_A49D_3A9B_4669),
    X87::from_bits(0x400B_8CFC_AC01_3C37_575E),
    X87::from_bits(0x4006_88E3_EA4B_462E_BBD9),
    ONE,
];

const PA: [X87; 8] = [
    X87::from_bits(0xBFFF_89D9_9163_9982_CCAF),
    X87::from_bits(0x4006_BC7B_434E_A78A_EFB4),
    X87::from_bits(0xC004_D590_EFCB_D789_2BB3),
    X87::from_bits(0x4004_B16F_B9B1_509C_0C90),
    X87::from_bits(0x4003_86A8_55EF_ABD2_F1A5),
    X87::from_bits(0xC000_970E_1DE3_B589_EC74),
    X87::from_bits(0x3FFF_ED15_DFCC_1E1F_A493),
    X87::from_bits(0x3FFB_C068_D417_9735_4859),
];

const QA: [X87; 8] = [
    X87::from_bits(0x4007_E3F6_935D_7EB5_0540),
    X87::from_bits(0x4007_A476_6310_9BAF_ACB2),
    X87::from_bits(0x4007_8E4D_B557_4A8D_0A2C),
    X87::from_bits(0x4006_8BDF_2041_03DF_5006),
    X87::from_bits(0x4004_F268_5A65_A8D2_812C),
    X87::from_bits(0x4003_A64B_AFFE_C56E_6F95),
    X87::from_bits(0x4001_9485_4B0A_A180_6CA0),
    ONE,
];

const RA: [X87; 9] = [
    X87::from_bits(0x3FFC_8BA1_147F_2348_3020),
    X87::from_bits(0x4002_A2E9_9A0A_EEE0_4E8C),
    X87::from_bits(0x4006_BA3C_6651_2680_507E),
    X87::from_bits(0x4009_B073_EC3E_0C62_5EAF),
    X87::from_bits(0x400B_9F04_4EC3_FB20_0D87),
    X87::from_bits(0x400C_8B81_0197_59F3_B6EA),
    X87::from_bits(0x400B_E303_7CED_A547_7EA6),
    X87::from_bits(0x400A_9537_E11D_629D_5E93),
    X87::from_bits(0x4006_DE17_7760_39C0_8D90),
];

const SA: [X87; 10] = [
    X87::from_bits(0xC002_DD28_548B_26F4_9C21),
    X87::from_bits(0xC007_A5C8_2D56_2A7A_71C7),
    X87::from_bits(0xC00A_B851_FF71_4D68_3071),
    X87::from_bits(0xC00C_C2C8_E24B_636B_5DB4),
    X87::from_bits(0xC00D_D0D5_988E_04C9_308E),
    X87::from_bits(0xC00D_E105_654E_C58F_FCF7),
    X87::from_bits(0xC00C_E2A8_0256_FBB4_CDAD),
    X87::from_bits(0xC00A_B3A8_A2BD_1AE5_892A),
    X87::from_bits(0xC006_8C39_6054_B1AB_12AE),
    ONE,
];

const RB: [X87; 8] = [
    X87::from_bits(0xBFF0_CC3E_CBC7_9CC3_60D1),
    X87::from_bits(0xBFF7_840F_C34A_1215_833D),
    X87::from_bits(0xBFFB_C137_900C_35C6_FA33),
    X87::from_bits(0xBFFE_EE91_368B_082A_3D37),
    X87::from_bits(0xC001_88C2_DF6A_D5D3_AA0D),
    X87::from_bits(0xC002_8D7A_0504_50A2_CC9B),
    X87::from_bits(0xC001_E237_02E6_1492_A1B2),
    X87::from_bits(0xBFFF_B35B_767B_1FAF_5426),
];

const SB: [X87; 8] = [
    X87::from_bits(0x3FF7_A1C0_4ED1_59F6_AE68),
    X87::from_bits(0x3FFC_A225_643B_F00B_B255),
    X87::from_bits(0x3FFF_ECE2_088C_B3DE_BC0A),
    X87::from_bits(0x4002_9ED7_7F37_5E7A_F590),
    X87::from_bits(0x4003_CA88_8BA1_BCAA_BB46),
    X87::from_bits(0x4003_E594_8A03_597C_3A41),
    X87::from_bits(0x4002_BD21_2833_4D04_29EA),
    ONE,
];

const RC: [X87; 6] = [
    X87::from_bits(0xBFF1_AE0E_3B7F_13E2_9D19),
    X87::from_bits(0xBFF7_CC99_2C82_6973_D624),
    X87::from_bits(0xBFFB_E9D0_3DEA_0628_E5B6),
    X87::from_bits(0xBFFE_C08B_E0F3_198D_261E),
    X87::from_bits(0xBFFF_E1F6_11A6_8108_769A),
    X87::from_bits(0xBFFF_83C3_7E35_AEFE_08A5),
];

const SC: [X87; 6] = [
    X87::from_bits(0x3FF8_89D7_B459_1D2B_D98B),
    X87::from_bits(0x3FFC_D377_BBB2_7F6A_FCD4),
    X87::from_bits(0x3FFF_D1CC_E147_F28C_FE7F),
    X87::from_bits(0x4001_9DFA_2BCC_E78B_3201),
    X87::from_bits(0x4001_A02A_6A7B_20F6_5C06),
    ONE,
];
