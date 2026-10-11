use super::*;

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
