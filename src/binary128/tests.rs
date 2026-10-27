use super::*;
use crate::x87::{
    Integral,
    X87,
};

#[test]
fn exact_binary128_rounding_and_range() {
    let arena = Bump::new();
    for (text, bits) in [
        ("1.0q", 0x3FFF_0000_0000_0000_0000_0000_0000_0000),
        ("0.1Q", 0x3FFB_9999_9999_9999_9999_9999_9999_999A),
        (
            "0x1.0000000000000000000000000001p0q",
            0x3FFF_0000_0000_0000_0000_0000_0000_0001,
        ),
        (
            "0x1.00000000000000000000000000008p0q",
            0x3FFF_0000_0000_0000_0000_0000_0000_0000,
        ),
        (
            "0x1.00000000000000000000000000018p0q",
            0x3FFF_0000_0000_0000_0000_0000_0000_0002,
        ),
        ("0x1p-16494q", 1),
        ("0x1.8p-16495q", 1),
        (
            "0x1.ffffffffffffffffffffffffffffp16383q",
            0x7FFE_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF,
        ),
        ("0e999999999999999999999999q", 0),
    ] {
        assert_eq!(
            parse(text, &arena)
                .unwrap_or_else(|error| panic!("{text}: {error:?}"))
                .0
                .get(),
            bits,
            "{text}"
        );
    }
    for text in ["0x1p16384q", "1e5000q"] {
        assert!(
            matches!(
                parse(text, &arena),
                Err(ParseFloatError::OutOfRange(_, FloatRangeError::Overflow))
            ),
            "{text}"
        );
    }
    for text in ["0x1p-16495q", "1e-5000q"] {
        assert!(
            matches!(
                parse(text, &arena),
                Err(ParseFloatError::OutOfRange(_, FloatRangeError::Underflow))
            ),
            "{text}"
        );
    }
    for text in ["1q", "0x1q", ".q", "1..0q", "1.0f128", "1.0qe1", "1e+q"] {
        assert!(
            matches!(parse(text, &arena), Err(ParseFloatError::Invalid(_))),
            "{text}"
        );
    }
}

/// Operands and libgcc's results (`__addtf3`, `__subtf3`, `__multf3`,
/// `__divtf3`, glibc's `fmodf128` and `sqrtf128` of the first operand), a
/// sample of a 200,000-pair differential run against GCC 15 and glibc 2.41
/// on x86-64 that matched bit for bit: subnormals, infinities, quiet and
/// signaling NaNs, and values near one and near the range limits.
const LIBGCC_VECTORS: [[u128; 8]; 24] = [
    [
        0x3FF8_0975_FBDE_15B0_2A33_7357_AE2C_C59B,
        0x7FFF_0913_2716_87B2_F70A_BB34_1875_063D,
        0x7FFF_8913_2716_87B2_F70A_BB34_1875_063D,
        0x7FFF_8913_2716_87B2_F70A_BB34_1875_063D,
        0x7FFF_8913_2716_87B2_F70A_BB34_1875_063D,
        0x7FFF_8913_2716_87B2_F70A_BB34_1875_063D,
        0x7FFF_8913_2716_87B2_F70A_BB34_1875_063D,
        0x3FFB_70AA_F6CD_B91E_11DE_6481_DE52_1EB2,
    ],
    [
        0xC00E_EFE9_C561_0885_6EF1_4999_F811_4BD4,
        0xB494_D688_0418_A99E_9557_53B5_7993_3F4D,
        0xC00E_EFE9_C561_0885_6EF1_4999_F811_4BD4,
        0xC00E_EFE9_C561_0885_6EF1_4999_F811_4BD4,
        0x34A4_C7BF_5640_3763_0656_F546_9828_F4D5,
        0x4B79_0DCF_34D3_31BB_47BE_9891_31B3_F148,
        0xB491_FA1C_BDD8_66BB_81B8_F905_E159_0690,
        0xFFFF_8000_0000_0000_0000_0000_0000_0000,
    ],
    [
        0x9426_9B57_FB8D_31F9_BE1E_DEFC_FABD_75DA,
        0x7B59_BAF2_B613_ED82_8B4E_B781_7F86_EAD9,
        0x7B59_BAF2_B613_ED82_8B4E_B781_7F86_EAD9,
        0xFB59_BAF2_B613_ED82_8B4E_B781_7F86_EAD9,
        0xCF81_63DD_F307_12E1_59DF_FF8D_4078_5010,
        0x8000_0000_0000_0000_0000_0000_0000_0000,
        0x9426_9B57_FB8D_31F9_BE1E_DEFC_FABD_75DA,
        0xFFFF_8000_0000_0000_0000_0000_0000_0000,
    ],
    [
        0x3FFA_FF7A_A865_D7D4_6EDD_2256_2C4C_697B,
        0x3FFE_74A3_714B_9AD2_7DC2_AE94_E4DB_F967,
        0x3FFE_949B_1BD1_F84F_C4B0_80BA_47A0_BFFF,
        0xBFFE_54AB_C6C5_3D55_36D4_DC6F_8217_32CF,
        0x3FFA_7442_6514_B488_DF53_181D_9AA8_F4D7,
        0x3FFB_5F61_F089_6AA3_7DC8_AEC5_B898_3E7C,
        0x3FFA_FF7A_A865_D7D4_6EDD_2256_2C4C_697B,
        0x3FFC_FFBD_4FDB_18DB_BAF8_BEC7_B689_281F,
    ],
    [
        0xBFFC_A40A_022B_9B89_6C12_9AF7_F244_0EFE,
        0x0000_500C_CE12_6B79_3143_20AA_7DA5_B1EF,
        0xBFFC_A40A_022B_9B89_6C12_9AF7_F244_0EFE,
        0xBFFC_A40A_022B_9B89_6C12_9AF7_F244_0EFE,
        0x8000_106B_0468_802E_105C_F6A5_C597_31CB,
        0xFFFC_4FD2_4127_FC32_EEDC_8FD9_12DA_45E2,
        0x8000_2288_6F11_9236_611E_B7F8_2FF6_0991,
        0xFFFF_8000_0000_0000_0000_0000_0000_0000,
    ],
    [
        0x0000_21FE_CFEA_DE59_4453_EFE0_E843_BDA5,
        0xFFFF_E939_081A_3DF9_1B77_69DE_94BE_57C2,
        0xFFFF_E939_081A_3DF9_1B77_69DE_94BE_57C2,
        0xFFFF_E939_081A_3DF9_1B77_69DE_94BE_57C2,
        0xFFFF_E939_081A_3DF9_1B77_69DE_94BE_57C2,
        0xFFFF_E939_081A_3DF9_1B77_69DE_94BE_57C2,
        0xFFFF_E939_081A_3DF9_1B77_69DE_94BE_57C2,
        0x1FFE_7527_CC02_1825_9A2D_E1A0_5085_C046,
    ],
    [
        0xBFF8_63C5_8085_E876_40F7_CD92_EBBC_3E26,
        0x7FFF_074D_E3FA_4927_9EF4_F552_E761_1475,
        0x7FFF_874D_E3FA_4927_9EF4_F552_E761_1475,
        0x7FFF_874D_E3FA_4927_9EF4_F552_E761_1475,
        0x7FFF_874D_E3FA_4927_9EF4_F552_E761_1475,
        0x7FFF_874D_E3FA_4927_9EF4_F552_E761_1475,
        0x7FFF_874D_E3FA_4927_9EF4_F552_E761_1475,
        0xFFFF_8000_0000_0000_0000_0000_0000_0000,
    ],
    [
        0x7066_804E_A183_B565_B483_DFD4_BC1F_4B4F,
        0xFFFF_5863_C3F9_3CD5_141E_D10A_6A73_5BEC,
        0xFFFF_D863_C3F9_3CD5_141E_D10A_6A73_5BEC,
        0xFFFF_D863_C3F9_3CD5_141E_D10A_6A73_5BEC,
        0xFFFF_D863_C3F9_3CD5_141E_D10A_6A73_5BEC,
        0xFFFF_D863_C3F9_3CD5_141E_D10A_6A73_5BEC,
        0xFFFF_D863_C3F9_3CD5_141E_D10A_6A73_5BEC,
        0x5832_BB95_11FA_A65C_07A0_7301_2150_C03A,
    ],
    [
        0x7FFF_F090_1B9C_A499_E010_D027_7F31_9B90,
        0x4002_6C80_5E15_FF5E_7C5E_EFF7_E27D_C320,
        0x7FFF_F090_1B9C_A499_E010_D027_7F31_9B90,
        0x7FFF_F090_1B9C_A499_E010_D027_7F31_9B90,
        0x7FFF_F090_1B9C_A499_E010_D027_7F31_9B90,
        0x7FFF_F090_1B9C_A499_E010_D027_7F31_9B90,
        0x7FFF_F090_1B9C_A499_E010_D027_7F31_9B90,
        0x7FFF_F090_1B9C_A499_E010_D027_7F31_9B90,
    ],
    [
        0xFFFF_D932_F500_567F_57CF_B2FD_8816_2913,
        0xC0E1_9BEE_6571_3982_2DA6_EA65_3427_FB71,
        0xFFFF_D932_F500_567F_57CF_B2FD_8816_2913,
        0xFFFF_D932_F500_567F_57CF_B2FD_8816_2913,
        0xFFFF_D932_F500_567F_57CF_B2FD_8816_2913,
        0xFFFF_D932_F500_567F_57CF_B2FD_8816_2913,
        0xFFFF_D932_F500_567F_57CF_B2FD_8816_2913,
        0xFFFF_D932_F500_567F_57CF_B2FD_8816_2913,
    ],
    [
        0x9D07_A6F8_789D_D5A3_587B_33E3_D83D_E6C8,
        0x0000_C87B_C1DE_001B_E052_6C9D_3468_DADB,
        0x9D07_A6F8_789D_D5A3_587B_33E3_D83D_E6C8,
        0x9D07_A6F8_789D_D5A3_587B_33E3_D83D_E6C8,
        0x8000_0000_0000_0000_0000_0000_0000_0000,
        0xDD06_0E0C_6633_8F86_2796_4697_7679_0061,
        0x8000_B02F_3006_D418_CAE2_1053_D7D1_C7D0,
        0xFFFF_8000_0000_0000_0000_0000_0000_0000,
    ],
    [
        0x4004_2CA5_A4D5_B317_E37F_C02B_17F0_3DB1,
        0x8000_BA2D_5D1C_F2C5_2D82_D228_6A02_DA60,
        0x4004_2CA5_A4D5_B317_E37F_C02B_17F0_3DB1,
        0x4004_2CA5_A4D5_B317_E37F_C02B_17F0_3DB1,
        0x8005_B54B_4076_D397_4D53_5FF8_C994_CDB8,
        0xFFFF_0000_0000_0000_0000_0000_0000_0000,
        0x0000_4F46_E737_C1B0_96A4_66BC_56AC_5980,
        0x4001_8857_3D5E_3F28_1920_48F1_A2C8_486A,
    ],
    [
        0xFFFF_6DEB_F816_9729_7461_F780_5828_5047,
        0x0000_C6D1_272D_CD05_E2B6_4290_180E_77DF,
        0xFFFF_EDEB_F816_9729_7461_F780_5828_5047,
        0xFFFF_EDEB_F816_9729_7461_F780_5828_5047,
        0xFFFF_EDEB_F816_9729_7461_F780_5828_5047,
        0xFFFF_EDEB_F816_9729_7461_F780_5828_5047,
        0xFFFF_EDEB_F816_9729_7461_F780_5828_5047,
        0xFFFF_EDEB_F816_9729_7461_F780_5828_5047,
    ],
    [
        0x7FFF_67CC_D723_CF80_7C17_F3CE_2FB0_681F,
        0x7FFF_A48E_E209_1D11_7713_966E_88FE_6B6B,
        0x7FFF_A48E_E209_1D11_7713_966E_88FE_6B6B,
        0x7FFF_A48E_E209_1D11_7713_966E_88FE_6B6B,
        0x7FFF_A48E_E209_1D11_7713_966E_88FE_6B6B,
        0x7FFF_A48E_E209_1D11_7713_966E_88FE_6B6B,
        0x7FFF_A48E_E209_1D11_7713_966E_88FE_6B6B,
        0x7FFF_E7CC_D723_CF80_7C17_F3CE_2FB0_681F,
    ],
    [
        0x8000_42A9_B003_BCF7_91A2_6E64_588E_664E,
        0xFFFF_8BBC_2F7B_FFD0_3DD3_3A0E_0E7E_FC2F,
        0xFFFF_8BBC_2F7B_FFD0_3DD3_3A0E_0E7E_FC2F,
        0xFFFF_8BBC_2F7B_FFD0_3DD3_3A0E_0E7E_FC2F,
        0xFFFF_8BBC_2F7B_FFD0_3DD3_3A0E_0E7E_FC2F,
        0xFFFF_8BBC_2F7B_FFD0_3DD3_3A0E_0E7E_FC2F,
        0xFFFF_8BBC_2F7B_FFD0_3DD3_3A0E_0E7E_FC2F,
        0xFFFF_8000_0000_0000_0000_0000_0000_0000,
    ],
    [
        0x0000_483B_427D_DE82_5969_6089_9B20_C5BF,
        0x4009_4407_495C_86E3_AF24_3611_4E6F_E72E,
        0x4009_4407_495C_86E3_AF24_3611_4E6F_E72E,
        0xC009_4407_495C_86E3_AF24_3611_4E6F_E72E,
        0x0009_6DB4_39E4_817C_6CB6_1219_271C_5706,
        0x0000_000E_4446_0EF4_4BE2_3044_454B_8A37,
        0x0000_483B_427D_DE82_5969_6089_9B20_C5BF,
        0x1FFF_0FF7_139A_482E_3727_57E4_E249_12AE,
    ],
    [
        0x8000_5345_0F7A_BE13_F9D9_2D5C_5357_AFAF,
        0x0001_352A_A0DC_D926_EB17_24EC_8BA7_E914,
        0x0000_E1E5_9162_1B12_F13D_F790_3850_3965,
        0x8001_886F_B057_973A_E4F0_5248_DEFF_98C3,
        0x8000_0000_0000_0000_0000_0000_0000_0000,
        0xBFFD_13CC_E57C_8B88_2844_4BC3_66C1_E501,
        0x8000_5345_0F7A_BE13_F9D9_2D5C_5357_AFAF,
        0xFFFF_8000_0000_0000_0000_0000_0000_0000,
    ],
    [
        0xFFFF_000B_9FE5_74F8_2A3B_5F21_DF3A_8011,
        0x8003_A2F1_C7BD_ACB3_E2EA_6EAD_489B_9B2A,
        0xFFFF_800B_9FE5_74F8_2A3B_5F21_DF3A_8011,
        0xFFFF_800B_9FE5_74F8_2A3B_5F21_DF3A_8011,
        0xFFFF_800B_9FE5_74F8_2A3B_5F21_DF3A_8011,
        0xFFFF_800B_9FE5_74F8_2A3B_5F21_DF3A_8011,
        0xFFFF_800B_9FE5_74F8_2A3B_5F21_DF3A_8011,
        0xFFFF_800B_9FE5_74F8_2A3B_5F21_DF3A_8011,
    ],
    [
        0x7FFF_8695_E391_48F8_4EC5_CAA4_ADC9_5469,
        0xFFFF_3ADE_BE1D_6D38_CB84_91FD_1A59_19E2,
        0x7FFF_8695_E391_48F8_4EC5_CAA4_ADC9_5469,
        0x7FFF_8695_E391_48F8_4EC5_CAA4_ADC9_5469,
        0x7FFF_8695_E391_48F8_4EC5_CAA4_ADC9_5469,
        0x7FFF_8695_E391_48F8_4EC5_CAA4_ADC9_5469,
        0x7FFF_8695_E391_48F8_4EC5_CAA4_ADC9_5469,
        0x7FFF_8695_E391_48F8_4EC5_CAA4_ADC9_5469,
    ],
    [
        0x8000_AC52_87A6_CE55_4C85_86FE_7FC2_B689,
        0x4012_36D3_4B53_4BC7_929C_7DD9_A94F_FC90,
        0x4012_36D3_4B53_4BC7_929C_7DD9_A94F_FC90,
        0xC012_36D3_4B53_4BC7_929C_7DD9_A94F_FC90,
        0x8013_A274_55FC_17B3_722A_C02A_BB0C_5136,
        0x8000_0000_11BD_AA74_748A_97DD_BA3B_6903,
        0x8000_AC52_87A6_CE55_4C85_86FE_7FC2_B689,
        0xFFFF_8000_0000_0000_0000_0000_0000_0000,
    ],
    [
        0x0000_A023_C4E2_94B0_73C6_8DBE_126A_7D99,
        0x8000_8385_F74B_2A7C_4CF0_7D57_0D70_A328,
        0x0000_1C9D_CD97_6A34_26D6_1067_04F9_DA71,
        0x0001_23A9_BC2D_BF2C_C0B7_0B15_1FDB_20C1,
        0x8000_0000_0000_0000_0000_0000_0000_0000,
        0xBFFF_37B3_1CC9_D89C_567F_8A6F_09DE_5AC5,
        0x0000_1C9D_CD97_6A34_26D6_1067_04F9_DA71,
        0x1FFF_94F2_BFCB_C502_6BD5_C350_9206_08DA,
    ],
    [
        0x8000_8274_E0D1_40D1_2044_04E7_F97A_F610,
        0xFFFF_4E77_E585_E7F3_2F4C_3E30_3631_70FC,
        0xFFFF_CE77_E585_E7F3_2F4C_3E30_3631_70FC,
        0xFFFF_CE77_E585_E7F3_2F4C_3E30_3631_70FC,
        0xFFFF_CE77_E585_E7F3_2F4C_3E30_3631_70FC,
        0xFFFF_CE77_E585_E7F3_2F4C_3E30_3631_70FC,
        0xFFFF_CE77_E585_E7F3_2F4C_3E30_3631_70FC,
        0xFFFF_8000_0000_0000_0000_0000_0000_0000,
    ],
    [
        0x7FFF_3EEA_4BA4_5B78_9CED_4FE4_66D6_CDCE,
        0x8000_B5C1_A59A_F132_7A7E_ECE7_8EEB_C850,
        0x7FFF_BEEA_4BA4_5B78_9CED_4FE4_66D6_CDCE,
        0x7FFF_BEEA_4BA4_5B78_9CED_4FE4_66D6_CDCE,
        0x7FFF_BEEA_4BA4_5B78_9CED_4FE4_66D6_CDCE,
        0x7FFF_BEEA_4BA4_5B78_9CED_4FE4_66D6_CDCE,
        0x7FFF_BEEA_4BA4_5B78_9CED_4FE4_66D6_CDCE,
        0x7FFF_BEEA_4BA4_5B78_9CED_4FE4_66D6_CDCE,
    ],
    [
        0x8000_B4B9_FB39_4FB2_7494_B81C_4C30_91AD,
        0x0000_842D_615A_4239_697B_6AC8_94B5_58FD,
        0x8000_308C_99DF_0D79_0B19_4D53_B77B_38B0,
        0x8001_38E7_5C93_91EB_DE10_22E4_E0E5_EAAA,
        0x8000_0000_0000_0000_0000_0000_0000_0000,
        0xBFFF_5E07_A087_5ADB_B979_0648_227D_CCF7,
        0x8000_308C_99DF_0D79_0B19_4D53_B77B_38B0,
        0xFFFF_8000_0000_0000_0000_0000_0000_0000,
    ],
];

/// glibc's `strtod` family reports an underflow for a tiny inexact value,
/// with x86's tininess after rounding: just below the smallest normal, a
/// value that rounds to it at 113 bits is not tiny.
#[test]
fn underflow_follows_glibc_round_and_return() {
    let arena = Bump::new();
    for (text, underflow) in [
        ("0x1p-16494q", false),
        ("0x1.8p-16494q", true),
        ("0x1.8P-16494q", true),
        ("0x1p-16382q", false),
        ("0x1.ffffffffffffffffffffffffffffcp-16383q", false),
        ("0x1.ffffffffffffffffffffffffffff8p-16383q", false),
        ("0x1.ffffffffffffffffffffffffffff4p-16383q", true),
        ("0x0.ffffffffffffffffffffffffffff8p-16382q", true),
        ("1e-4940q", true),
        ("1.5q", false),
    ] {
        let (_, reported) = parse_reporting_underflow(text, &arena).unwrap();
        assert_eq!(reported, underflow, "{text}");
    }
}

#[test]
fn arithmetic_matches_libgcc_bit_for_bit() {
    for [a, b, sum, difference, product, quotient, remainder, root] in LIBGCC_VECTORS {
        let (x, y) = (Binary128::from_bits(a), Binary128::from_bits(b));
        assert_eq!(x.add(y).to_bits(), sum, "add {a:#034x} {b:#034x}");
        assert_eq!(x.sub(y).to_bits(), difference, "sub {a:#034x} {b:#034x}");
        assert_eq!(x.mul(y).to_bits(), product, "mul {a:#034x} {b:#034x}");
        assert_eq!(x.div(y).to_bits(), quotient, "div {a:#034x} {b:#034x}");
        assert_eq!(x.rem(y).to_bits(), remainder, "fmod {a:#034x} {b:#034x}");
        assert_eq!(x.sqrt().to_bits(), root, "sqrt {a:#034x}");
    }
}

/// Conversions from the same run: the first operand to binary64, binary32
/// and x87 (`__trunctfdf2`, `__trunctfsf2`, `__trunctfxf2`), the second
/// operand's low bits from binary64, binary32 and x87 (any x87 encoding,
/// unnormals included), and a 128-bit integer from signed and unsigned.
const CONVERSION_VECTORS: [[u128; 10]; 13] = [
    [
        0x3FF8_0975_FBDE_15B0_2A33_7357_AE2C_C59B,
        0x0000_0000_0000_0000_3F80_975F_BDE1_5B03,
        0x0000_0000_0000_0000_0000_0000_3C04_BAFE,
        0x0000_0000_0000_3FF8_84BA_FDEF_0AD8_151A,
        0x7FFF_0913_2716_87B2_F70A_BB34_1875_063D,
        0xC370_ABB3_4187_5063_D000_0000_0000_0000,
        0x3FB0_EA0C_7A00_0000_0000_0000_0000_0000,
        0x87B2_EE15_7668_30EA_0C7A_0000_0000_0000,
        0xFFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF_F422,
        0xC00A_7BC0_0000_0000_0000_0000_0000_0000,
    ],
    [
        0x3FF9_1032_F1EF_6926_397E_4F77_7E1A_3E74,
        0x0000_0000_0000_0000_3F91_032F_1EF6_9264,
        0x0000_0000_0000_0000_0000_0000_3C88_1979,
        0x0000_0000_0000_3FF9_8819_78F7_B493_1CBF,
        0x4010_2C31_942A_F3D0_D097_0501_07AA_5237,
        0xC109_7050_107A_A523_7000_0000_0000_0000,
        0x3F8F_54A4_6E00_0000_0000_0000_0000_0000,
        0xF3D0_A12E_0A02_0F54_A46E_0000_0000_0000,
        0x0678_B327_C108_253F_F71D_E3D9_CD2F_8285,
        0x4079_9E2C_C9F0_4209_4FFD_C778_F673_4BE1,
    ],
    [
        0x8000_42A9_B003_BCF7_91A2_6E64_588E_664E,
        0x0000_0000_0000_0000_8000_0000_0000_0000,
        0x0000_0000_0000_0000_0000_0000_8000_0000,
        0x0000_0000_0000_8000_2154_D801_DE7B_C8D1,
        0xFFFF_8BBC_2F7B_FFD0_3DD3_3A0E_0E7E_FC2F,
        0x3FDD_33A0_E0E7_EFC2_F000_0000_0000_0000,
        0x3F9C_FDF8_5E00_0000_0000_0000_0000_0000,
        0xFFD0_7BA6_741C_1CFD_F85E_0000_0000_0000,
        0xFFFF_FFFF_FFFF_FFFF_FFFF_FE56_45CB_21B0,
        0xC027_A9BA_34DE_5000_0000_0000_0000_0000,
    ],
    [
        0x8004_A607_4614_0AA0_A567_DFC5_59A4_8AB5,
        0x0000_0000_0000_0000_8000_0000_0000_0000,
        0x0000_0000_0000_0000_0000_0000_8000_0000,
        0x0000_0000_0000_8004_D303_A30A_0550_52B4,
        0xC000_4A52_257B_6A6F_E3BA_95AD_7C50_E77B,
        0xC23B_A95A_D7C5_0E77_B000_0000_0000_0000,
        0x4078_A1CE_F600_0000_0000_0000_0000_0000,
        0x6A6F_C775_2B5A_F8A1_CEF6_0000_0000_0000,
        0xFFFF_FFFF_FFFF_FFFF_FFFF_FF97_F8C7_CC7D,
        0xC025_A01C_E0CE_0C00_0000_0000_0000_0000,
    ],
    [
        0x54B7_C46D_CD93_F3E4_BF2A_1035_568E_AD03,
        0x0000_0000_0000_0000_7FF0_0000_0000_0000,
        0x0000_0000_0000_0000_0000_0000_7F80_0000,
        0x0000_0000_0000_54B7_E236_E6C9_F9F2_5F95,
        0x0000_9CA1_388B_A3F8_2E96_5E1D_B913_4ABF,
        0x3EE9_65E1_DB91_34AB_F000_0000_0000_0000,
        0xBFF2_2695_7E00_0000_0000_0000_0000_0000,
        0xA3F8_5D2C_BC3B_7226_957E_0000_0000_0000,
        0x0000_0000_0000_0000_021E_2364_859C_FF92,
        0x4038_0F11_B242_CE7F_C900_0000_0000_0000,
    ],
    [
        0xE09F_9378_896B_47FF_D477_F7FE_5737_8EB0,
        0x0000_0000_0000_0000_FFF0_0000_0000_0000,
        0x0000_0000_0000_0000_0000_0000_FF80_0000,
        0x0000_0000_0000_E09F_C9BC_44B5_A3FF_EA3C,
        0xFFFB_E4B1_5825_6A76_B317_2F28_64C5_7D22,
        0xBF31_72F2_864C_57D2_2000_0000_0000_0000,
        0x4049_8AFA_4400_0000_0000_0000_0000_0000,
        0x6A76_662E_5E50_C98A_FA44_0000_0000_0000,
        0xFA02_E970_BA90_EFBD_689F_638D_F85D_8D3A,
        0xC079_7F45_A3D1_5BC4_10A5_D827_1C81_E89D,
    ],
    [
        0xC009_2673_FAC1_C769_6677_F8C8_E8E6_BEA7,
        0x0000_0000_0000_0000_C092_673F_AC1C_7696,
        0x0000_0000_0000_0000_0000_0000_C493_39FD,
        0x0000_0000_0000_C009_9339_FD60_E3B4_B33C,
        0x15AD_6C0B_22C7_A977_19DA_B0CE_478C_9BE5,
        0x3D9D_AB0C_E478_C9BE_5000_0000_0000_0000,
        0x400F_1937_CA00_0000_0000_0000_0000_0000,
        0xA977_33B5_619C_8F19_37CA_0000_0000_0000,
        0xFFFF_FFFF_FFFF_FFFF_FFFF_FFD0_8599_93A4,
        0xC024_7BD3_3362_E000_0000_0000_0000_0000,
    ],
    [
        0x3FF6_7E52_996C_5294_368A_4714_B37A_AF31,
        0x0000_0000_0000_0000_3F67_E529_96C5_2943,
        0x0000_0000_0000_0000_0000_0000_3B3F_294D,
        0x0000_0000_0000_3FF6_BF29_4CB6_294A_1B45,
        0x400E_A0F2_AB84_B5C6_E0E8_56E5_231C_0D2D,
        0xC20E_856E_5231_C0D2_D000_0000_0000_0000,
        0x3FC6_381A_5A00_0000_0000_0000_0000_0000,
        0xB5C6_C1D0_ADCA_4638_1A5A_0000_0000_0000,
        0x0000_0000_0000_0000_0000_03F8_D4AA_3513,
        0x4028_FC6A_551A_8980_0000_0000_0000_0000,
    ],
    [
        0x7FFF_5B90_4258_2D3D_EE2F_799C_153E_7227,
        0x0000_0000_0000_0000_7FFD_B904_2582_D3DE,
        0x0000_0000_0000_0000_0000_0000_7FED_C821,
        0x0000_0000_0000_7FFF_EDC8_212C_169E_F717,
        0x8003_858E_906C_530D_0F46_855F_CDEF_E8EB,
        0x3CF4_6855_FCDE_FE8E_B000_0000_0000_0000,
        0xC01B_DFD1_D600_0000_0000_0000_0000_0000,
        0x530D_1E8D_0ABF_9BDF_D1D6_0000_0000_0000,
        0xFFFF_FFFF_FFFF_794A_1E89_04E6_163E_2E3D,
        0xC04E_0D6B_C2ED_F633_D383_A386_0000_0000,
    ],
    [
        0x7FFF_0A0A_B005_6284_0761_3C68_9AEF_4941,
        0x0000_0000_0000_0000_7FF8_A0AB_0056_2840,
        0x0000_0000_0000_0000_0000_0000_7FC5_0558,
        0x0000_0000_0000_7FFF_C505_5802_B142_03B0,
        0xC00D_9F17_03D9_6E9C_F71C_673F_00D4_FB41,
        0xC371_C673_F00D_4FB4_1000_0000_0000_0000,
        0x3F81_A9F6_8200_0000_0000_0000_0000_0000,
        0x6E9C_EE38_CE7E_01A9_F682_0000_0000_0000,
        0xFFFF_FFFF_FFFF_FFFF_FFFF_FFFF_F238_384D,
        0xC01A_B8F8_F660_0000_0000_0000_0000_0000,
    ],
    [
        0x0000_0529_C1CE_3A28_4E84_A466_7E3E_2C5C,
        0x0000_0000_0000_0000_0000_0000_0000_0000,
        0x0000_0000_0000_0000_0000_0000_0000_0000,
        0x0000_0000_0000_0000_0294_E0E7_1D14_2742,
        0xEBF3_0F0E_5D62_138A_4950_5771_B177_752D,
        0x4095_0577_1B17_7752_D000_0000_0000_0000,
        0xBFE2_EEEA_5A00_0000_0000_0000_0000_0000,
        0x138A_92A0_AEE3_62EE_EA5A_0000_0000_0000,
        0xFFFF_FFFF_FFFF_FFF5_C61A_6B17_E377_A6EA,
        0xC042_473C_B29D_0391_0B22_C000_0000_0000,
    ],
    [
        0x52E5_579E_F397_C47C_B019_02E3_3AE6_74F4,
        0x0000_0000_0000_0000_7FF0_0000_0000_0000,
        0x0000_0000_0000_0000_0000_0000_7F80_0000,
        0x0000_0000_0000_52E5_ABCF_79CB_E23E_580D,
        0x0000_61A6_6E25_B657_2B6D_7587_0CA8_88FB,
        0x3EB6_D758_70CA_888F_B000_0000_0000_0000,
        0x3F99_5111_F600_0000_0000_0000_0000_0000,
        0xB657_56DA_EB0E_1951_11F6_0000_0000_0000,
        0x0000_0000_0000_0000_0000_007D_5F58_4AEE,
        0x4025_F57D_612B_B800_0000_0000_0000_0000,
    ],
    [
        0x8004_1001_1039_6CF6_84C9_09DB_F682_E3AF,
        0x0000_0000_0000_0000_8000_0000_0000_0000,
        0x0000_0000_0000_0000_0000_0000_8000_0000,
        0x0000_0000_0000_8004_8800_881C_B67B_4265,
        0x4002_8FE1_9EA1_9DD7_707E_F5CF_5019_4B2C,
        0x4307_EF5C_F501_94B2_C000_0000_0000_0000,
        0x4020_3296_5800_0000_0000_0000_0000_0000,
        0x9DD7_E0FD_EB9E_A032_9658_0000_0000_0000,
        0xFFFF_F058_0994_2F8D_DE79_E3B2_E7A9_9010,
        0xC06A_F4FE_CD7A_0E44_30C3_89A3_0ACD_FE00,
    ],
];

#[test]
fn conversions_match_libgcc_bit_for_bit() {
    for [
        quad,
        double,
        single,
        extended,
        low,
        from_double,
        from_single,
        from_extended,
        integer,
        from_integer,
    ] in CONVERSION_VECTORS
    {
        let value = Binary128::from_bits(quad);
        assert_eq!(u128::from(value.to_f64().to_bits()), double, "{quad:#x}");
        assert_eq!(u128::from(value.to_f32().to_bits()), single, "{quad:#x}");
        assert_eq!(value.to_x87().to_bits(), extended, "{quad:#x}");
        let low64 = u64::try_from(low & u128::from(u64::MAX)).unwrap();
        let low32 = u32::try_from(low & u128::from(u32::MAX)).unwrap();
        assert_eq!(
            Binary128::from_f64(f64::from_bits(low64)).to_bits(),
            from_double,
            "{low:#x}"
        );
        assert_eq!(
            Binary128::from_f32(f32::from_bits(low32)).to_bits(),
            from_single,
            "{low:#x}"
        );
        let x87 = X87::from_bits(low & ((1 << 80) - 1));
        assert_eq!(
            Binary128::from_x87(x87).to_bits(),
            from_extended,
            "{low:#x}"
        );
        let signed = integer.cast_signed();
        assert_eq!(
            Binary128::from_integer(signed < 0, signed.unsigned_abs()).to_bits(),
            from_integer,
            "{integer:#x}"
        );
    }
}

#[test]
fn integer_conversions_truncate_and_check_range() {
    let arena = Bump::new();
    let value = |text: &str| {
        let (negative, text) = text
            .strip_prefix('-')
            .map_or((false, text), |text| (true, text));
        parse(text, &arena).unwrap().with_sign(negative)
    };
    assert_eq!(value("-7.75q").to_integer(true, 32), Some(0xFFFF_FFF9));
    assert_eq!(
        value("4294967295.9q").to_integer(false, 32),
        Some(0xFFFF_FFFF)
    );
    assert_eq!(value("4294967296.0q").to_integer(false, 32), None);
    assert_eq!(
        value("-2147483648.5q").to_integer(true, 32),
        Some(0x8000_0000)
    );
    assert_eq!(value("-2147483649.0q").to_integer(true, 32), None);
    assert_eq!(value("-0.5q").to_integer(false, 8), Some(0));
    assert_eq!(value("1e40q").to_integer(true, 128), None);
    assert_eq!(Binary128::DEFAULT_NAN.to_integer(true, 64), None);
}

#[test]
fn exact_functions_round_and_scale() {
    let arena = Bump::new();
    let value = |text: &str| {
        let (negative, text) = text
            .strip_prefix('-')
            .map_or((false, text), |text| (true, text));
        parse(text, &arena).unwrap().with_sign(negative)
    };
    for (text, floor, ceil, trunc, nearest, away) in [
        ("2.5q", "2.0q", "3.0q", "2.0q", "2.0q", "3.0q"),
        ("-3.5q", "-4.0q", "-3.0q", "-3.0q", "-4.0q", "-4.0q"),
        ("0.25q", "0.0q", "1.0q", "0.0q", "0.0q", "0.0q"),
        ("1e30q", "1e30q", "1e30q", "1e30q", "1e30q", "1e30q"),
    ] {
        let x = value(text);
        assert_eq!(
            x.round_integral(Integral::Floor),
            value(floor),
            "floor {text}"
        );
        assert_eq!(x.round_integral(Integral::Ceil), value(ceil), "ceil {text}");
        assert_eq!(
            x.round_integral(Integral::Truncate),
            value(trunc),
            "trunc {text}"
        );
        assert_eq!(
            x.round_integral(Integral::HalfEven),
            value(nearest),
            "rint {text}"
        );
        assert_eq!(x.round_half_away(), value(away), "round {text}");
    }
    assert_eq!(
        value("-0.25q").round_integral(Integral::Ceil).to_bits(),
        1 << 127,
        "a zero result keeps the sign"
    );
    assert_eq!(value("0x1p-16494q").scale(1).to_bits(), 2);
    assert_eq!(value("1.0q").scale(16384), Binary128::infinity(false));
    let (fraction, exponent) = value("-48.0q").decompose();
    assert_eq!((fraction, exponent), (value("-0.75q"), 6));
    assert_eq!(
        Binary128::nan(false, 0).to_bits(),
        0x7FFF_4000_0000_0000_0000_0000_0000_0000
    );
}
