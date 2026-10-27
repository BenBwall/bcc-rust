use super::*;

/// libgcc's results on x86-64 (GCC 15): a source encoding, then its
/// binary16 and bfloat16 conversions. The binary32 rows end with NaNs,
/// signaling and quiet, and the overflow, subnormal and rounding boundaries.
const SINGLE: &[(u32, u16, u16)] = &[
    (0xB9E2_F6E1, 0x8F18, 0xB9E3),
    (0xCD4F_2A8F, 0xFC00, 0xCD4F),
    (0x54BD_A9BF, 0x7C00, 0x54BE),
    (0xB70D_186E, 0x808D, 0xB70D),
    (0xB289_0EB4, 0x8000, 0xB289),
    (0x40D1_AE0D, 0x468D, 0x40D2),
    (0x385B_BFC3, 0x036F, 0x385C),
    (0xB1B8_75B6, 0x8000, 0xB1B8),
    (0x8244_A367, 0x8000, 0x8245),
    (0x62A0_FDE2, 0x7C00, 0x62A1),
    (0x792A_646A, 0x7C00, 0x792A),
    (0x4E8C_F373, 0x7C00, 0x4E8D),
    (0x49CE_662F, 0x7C00, 0x49CE),
    (0xCEA7_9058, 0xFC00, 0xCEA8),
    (0x7F80_0001, 0x7E00, 0x7FC0),
    (0x7FC0_0000, 0x7E00, 0x7FC0),
    (0xFFA0_0000, 0xFF00, 0xFFE0),
    (0x7F80_2000, 0x7E01, 0x7FC0),
    (0x7FFF_FFFF, 0x7FFF, 0x7FFF),
    (0xFF80_0400, 0xFE00, 0xFFC0),
    (0x7F80_0000, 0x7C00, 0x7F80),
    (0x477F_F000, 0x7C00, 0x4780),
    (0x477F_EFFF, 0x7BFF, 0x4780),
    (0x477F_E000, 0x7BFF, 0x4780),
    (0x3380_0000, 0x0001, 0x3380),
    (0x3300_0000, 0x0000, 0x3300),
    (0x3300_0001, 0x0001, 0x3300),
    (0x337F_FFFF, 0x0001, 0x3380),
    (0x387F_C000, 0x03FF, 0x3880),
    (0x387F_E000, 0x0400, 0x3880),
];
const DOUBLE: &[(u64, u16, u16)] = &[
    (0x3E8D_61F6_A8B7_ADBA, 0x0004, 0x346B),
    (0x4739_E8C7_EEB4_A5B9, 0x7C00, 0x79CF),
    (0x88D1_80A3_CD8D_5C7E, 0x8000, 0x8000),
    (0xDD9B_E575_0BA4_3C1F, 0xFC00, 0xFF80),
    (0xBF7F_9081_B82B_5D26, 0x9FE4, 0xBBFD),
    (0x80FE_CB0E_83C8_67F0, 0x8000, 0x8000),
    (0x9EA9_FF3B_8C1A_D929, 0x8000, 0x8000),
    (0xC585_F4A8_F8E4_2EAD, 0xFC00, 0xEC30),
    (0xBDA3_5A9C_E859_D922, 0x8000, 0xAD1B),
    (0xC036_0AC7_6213_6F89, 0xCD83, 0xC1B0),
    (0x43D1_2AE9_8CD7_CE03, 0x7C00, 0x5E89),
    (0xBE6E_BACA_B4CF_A0B4, 0x8001, 0xB376),
    (0xF928_5354_C95F_196E, 0xFC00, 0xFF80),
    (0x3CB7_8CE6_3A31_BA80, 0x0000, 0x25BC),
];
const EXTENDED: &[(u128, u16, u16)] = &[
    (0x3FFB_8A1B_003B_C5AA_C96A, 0x2C51, 0x3D8A),
    (0x4046_8D37_7AA2_2331_CAC7, 0x7C00, 0x630D),
    (0xBFF8_B65C_117F_C98A_0EB4, 0xA1B3, 0xBC36),
    (0x3FFD_D143_2F13_2901_DED5, 0x368A, 0x3ED1),
    (0x3FDF_84F7_05F4_BB83_4F81, 0x0000, 0x2F85),
    (0xBFF8_81BE_55BA_0000_0000, 0xA00E, 0xBC02),
    (0xBFE7_8DA1_3080_689E_5C56, 0x8001, 0xB38E),
    (0xC00B_B5AA_D24E_C109_347E, 0xEDAD, 0xC5B6),
    (0xBFF7_F6F5_A56A_0000_0000, 0x9FB8, 0xBBF7),
    (0xC00A_C736_B86B_FF3B_3761, 0xEA3A, 0xC547),
    (0x4003_B34D_1A25_C845_B90C, 0x4D9A, 0x41B3),
    (0xBFF8_F8F1_D046_ABBF_6927, 0xA3C8, 0xBC79),
];
const QUAD: &[(u128, u16, u16)] = &[
    (0x3FCF_C927_EEDB_AD8F_5222_EC2C_5074_9914, 0x0000, 0x27E5),
    (0x3FE5_2EC3_3284_E3F9_796E_394E_E15A_347E, 0x0000, 0x3297),
    (0xBFE8_0D7B_7061_A25D_18A8_28F5_0610_5659, 0x8002, 0xB407),
    (0xBFF8_5E23_2692_11BE_0FEF_325D_B256_9A1D, 0xA179, 0xBC2F),
    (0x3FED_4B1D_37E1_0714_9BA9_5261_D519_801A, 0x0053, 0x36A6),
    (0xBFE2_D34B_287E_5E77_992F_67C7_D461_DF0B, 0x8000, 0xB16A),
    (0x3FE6_A251_B17A_A832_8F58_1E3E_3A70_11E2, 0x0001, 0x3351),
    (0x4011_EA5D_757A_30AB_654B_6268_CB1D_8E0A, 0x7C00, 0x48F5),
    (0x3FEC_3510_C2EC_7ED4_3BE6_364E_F1FE_9329, 0x0027, 0x361B),
    (0x3FF4_0EDF_1D10_D687_C7E5_7F38_901B_B6EA, 0x103B, 0x3A07),
    (0x3FF5_7825_A7EB_429E_1046_270E_9911_F39B, 0x15E1, 0x3ABC),
    (0xBFF6_C75D_4937_838A_18D4_F6CF_D22E_4E0D, 0x9B1D, 0xBB64),
];

#[test]
fn narrowing_matches_libgcc_bit_for_bit() {
    use HalfFormat::{
        BFloat16,
        Binary16,
    };
    for &(bits, half, bfloat) in SINGLE {
        let value = f32::from_bits(bits);
        assert_eq!(Binary16.round_f32(value), half, "{bits:#x}");
        assert_eq!(BFloat16.round_f32(value), bfloat, "{bits:#x}");
    }
    for &(bits, half, bfloat) in DOUBLE {
        let value = f64::from_bits(bits);
        assert_eq!(Binary16.round_f64(value), half, "{bits:#x}");
        assert_eq!(BFloat16.round_f64(value), bfloat, "{bits:#x}");
    }
    for &(bits, half, bfloat) in EXTENDED {
        let value = X87::from_bits(bits);
        assert_eq!(Binary16.round_x87(value), half, "{bits:#x}");
        assert_eq!(BFloat16.round_x87(value), bfloat, "{bits:#x}");
    }
    for &(bits, half, bfloat) in QUAD {
        let value = Binary128::from_bits(bits);
        assert_eq!(Binary16.round_binary128(value), half, "{bits:#x}");
        assert_eq!(BFloat16.round_binary128(value), bfloat, "{bits:#x}");
    }
}

#[test]
fn widening_is_exact_and_quiets_signaling_nans() {
    use HalfFormat::{
        BFloat16,
        Binary16,
    };
    for (bits, single) in [
        (0x3C00, 0x3F80_0000),
        (0x0001, 0x3380_0000),
        (0x03FF, 0x387F_C000),
        (0x7BFF, 0x477F_E000),
        (0xFC00, 0xFF80_0000),
        (0x8000, 0x8000_0000),
        (0x7C01, 0x7FC0_2000),
        (0xFE00, 0xFFC0_0000),
    ] {
        assert_eq!(Binary16.widen(bits).to_bits(), single, "{bits:#x}");
    }
    for (bits, single) in [
        (0x3F80, 0x3F80_0000),
        (0x0001, 0x0001_0000),
        (0x7F7F, 0x7F7F_0000),
        (0x7F81, 0x7FC1_0000),
        (0xFFC0, 0xFFC0_0000),
    ] {
        assert_eq!(BFloat16.widen(bits).to_bits(), single, "{bits:#x}");
    }
    // Every value is a binary32 value that converts back to it.
    for bits in 0..=u16::MAX {
        if !Binary16.is_nan(bits) {
            assert_eq!(Binary16.round_f32(Binary16.widen(bits)), bits);
        }
        if !BFloat16.is_nan(bits) {
            assert_eq!(BFloat16.round_f32(BFloat16.widen(bits)), bits);
        }
    }
}

#[test]
fn integers_round_once() {
    use HalfFormat::{
        BFloat16,
        Binary16,
    };
    for (negative, magnitude, half, bfloat) in [
        (false, 2049, 0x6800, 0x4500),
        (false, 2051, 0x6802, 0x4500),
        (true, 65519, 0xFBFF, 0xC780),
        (false, 65520, 0x7C00, 0x4780),
        (false, u128::MAX, 0x7C00, 0x7F80),
        // 2^127 + 2^119 + 1 is just over a bfloat16 midpoint; rounding it
        // to 64 bits first would make it a tie that rounds to even.
        (false, (1 << 127) | (1 << 119) | 1, 0x7C00, 0x7F01),
    ] {
        assert_eq!(Binary16.round_integer(negative, magnitude), half);
        assert_eq!(BFloat16.round_integer(negative, magnitude), bfloat);
    }
    assert_eq!(Binary16.to_integer(0xFBFF, true, 32), Some(0xFFFF_0020));
    assert_eq!(Binary16.to_integer(0x7C00, true, 32), None);
    assert_eq!(Binary16.to_integer(0x3BFF, false, 8), Some(0));
}
