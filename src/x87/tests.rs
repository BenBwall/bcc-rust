use std::cmp::Ordering;

use super::{
    Class,
    X87,
};

const ONE: X87 = X87::from_bits(0x3FFF_8000_0000_0000_0000);
const TWO: X87 = X87::from_bits(0x4000_8000_0000_0000_0000);
const THREE: X87 = X87::from_bits(0x4000_C000_0000_0000_0000);
/// The smallest positive denormal, 2^-16445.
const TINY: X87 = X87::from_bits(1);
const LARGEST: X87 = X87::from_bits(0x7FFE_FFFF_FFFF_FFFF_FFFF);
const QUIET_NAN: X87 = X87::from_bits(0x7FFF_C000_0000_0000_0001);
const SIGNALING_NAN: X87 = X87::from_bits(0x7FFF_8000_0000_0000_0001);

#[test]
fn classifies_every_kind_of_encoding() {
    assert_eq!(X87::zero(true).class(), Class::Zero);
    assert_eq!(
        ONE.class(),
        Class::Finite {
            exponent:    -63,
            significand: 1 << 63,
        }
    );
    assert_eq!(
        TINY.class(),
        Class::Finite {
            exponent:    -16445,
            significand: 1,
        }
    );
    // A pseudo-denormal has the value of the smallest normal exponent.
    assert_eq!(
        X87::from_bits(0x0000_8000_0000_0000_0000).class(),
        Class::Finite {
            exponent:    -16445,
            significand: 1 << 63,
        }
    );
    assert_eq!(X87::infinity(false).class(), Class::Infinite);
    assert_eq!(QUIET_NAN.class(), Class::Nan { signaling: false });
    assert_eq!(SIGNALING_NAN.class(), Class::Nan { signaling: true });
    // Unnormal, pseudo-infinity and pseudo-NaN.
    for bits in [
        0x3FFF_4000_0000_0000_0000,
        0x7FFF_0000_0000_0000_0000,
        0x7FFF_4000_0000_0000_0000,
    ] {
        assert_eq!(X87::from_bits(bits).class(), Class::Unsupported);
    }
}

#[test]
fn rounds_to_nearest_even_at_64_bits() {
    let third = ONE.div(THREE);
    assert_eq!(third.to_bits(), 0x3FFD_AAAA_AAAA_AAAA_AAAB);
    // 2^64 + 1 is a tie between 2^64 and 2^64 + 2: even wins.
    assert_eq!(
        X87::from_integer(false, (1 << 64) + 1).to_bits(),
        0x403F_8000_0000_0000_0000
    );
    assert_eq!(
        X87::from_integer(false, (1 << 64) + 3).to_bits(),
        0x403F_8000_0000_0000_0002
    );
    assert_eq!(
        X87::from_integer(true, 1).to_bits(),
        0xBFFF_8000_0000_0000_0000
    );
    assert_eq!(TWO.sqrt().to_bits(), 0x3FFF_B504_F333_F9DE_6484);
}

#[test]
fn handles_overflow_underflow_and_invalid_operations() {
    assert_eq!(LARGEST.add(LARGEST), X87::infinity(false));
    assert_eq!(TINY.div(TWO), X87::zero(false));
    assert_eq!(TINY.mul(THREE).div(TWO).to_bits(), 2);
    assert_eq!(
        X87::infinity(false).sub(X87::infinity(false)),
        X87::INDEFINITE
    );
    assert_eq!(X87::zero(false).div(X87::zero(true)), X87::INDEFINITE);
    assert_eq!(ONE.div(X87::zero(true)), X87::infinity(true));
    assert_eq!(ONE.sub(ONE), X87::zero(false));
    assert_eq!(X87::zero(true).add(X87::zero(true)), X87::zero(true));
    assert_eq!(SIGNALING_NAN.add(ONE), QUIET_NAN);
    assert_eq!(
        X87::from_bits(0x3FFF_4000_0000_0000_0000).add(ONE),
        X87::INDEFINITE
    );
}

#[test]
fn computes_fmod_exactly() {
    let seven = X87::from_integer(false, 7);
    assert_eq!(seven.rem(THREE), ONE);
    assert_eq!(seven.negate().rem(THREE), ONE.negate());
    assert_eq!(LARGEST.rem(THREE).to_integer(false, 32), Some(0));
    assert_eq!(ONE.rem(X87::infinity(true)), ONE);
    assert_eq!(X87::infinity(false).rem(ONE), X87::INDEFINITE);
}

#[test]
fn compares_with_unordered_nans() {
    assert_eq!(ONE.compare(TWO), Some(Ordering::Less));
    assert_eq!(TWO.negate().compare(ONE.negate()), Some(Ordering::Less));
    assert_eq!(
        X87::zero(true).compare(X87::zero(false)),
        Some(Ordering::Equal)
    );
    assert_eq!(
        X87::infinity(true).compare(LARGEST.negate()),
        Some(Ordering::Less)
    );
    assert_eq!(QUIET_NAN.compare(QUIET_NAN), None);
    assert_eq!(
        X87::from_bits(0x0000_8000_0000_0000_0000)
            .compare(X87::from_bits(0x0001_8000_0000_0000_0000)),
        Some(Ordering::Equal)
    );
}

#[test]
fn converts_to_and_from_binary_formats_and_integers() {
    assert_eq!(X87::from_f64(1.5).to_bits(), 0x3FFF_C000_0000_0000_0000);
    assert_eq!(X87::from_f64(f64::from_bits(1)).to_f64().to_bits(), 1);
    assert_eq!(X87::from_f32(-0.0).to_bits(), 0x8000_0000_0000_0000_0000);
    assert_eq!(ONE.div(THREE).to_f64(), 1.0 / 3.0);
    assert_eq!(ONE.div(THREE).to_f32(), 1.0 / 3.0);
    assert_eq!(LARGEST.to_f64(), f64::INFINITY);
    assert_eq!(X87::INDEFINITE.to_f64().to_bits(), 0xFFF8_0000_0000_0000);
    assert_eq!(
        X87::from_bits(0x3FFF_4000_0000_0000_0000)
            .to_f64()
            .to_bits(),
        0xFFF8_0000_0000_0000
    );
    assert_eq!(X87::from_f64(-2.75).to_integer(true, 32), Some(0xFFFF_FFFE));
    assert_eq!(X87::from_f64(-0.5).to_integer(false, 32), Some(0));
    assert_eq!(X87::from_f64(-1.0).to_integer(false, 32), None);
    assert_eq!(X87::from_integer(false, 1 << 63).to_integer(true, 64), None);
    assert_eq!(
        X87::from_integer(true, 1 << 63).to_integer(true, 64),
        Some(1 << 63)
    );
    assert_eq!(
        X87::from_integer(false, u128::from(u64::MAX)).to_integer(false, 64),
        Some(u128::from(u64::MAX))
    );
    assert_eq!(QUIET_NAN.to_integer(true, 32), None);
}

/// Compares every operation against the processor's own x87 unit on
/// random encodings of every class, near and far apart.
#[cfg(target_arch = "x86_64")]
mod hardware {
    use std::arch::asm;

    use super::super::X87;

    /// Runs an x87 instruction sequence with the default control word:
    /// 64-bit precision, round to nearest, exceptions masked. The sequence
    /// reads and writes only the memory its operands name and leaves the
    /// x87 stack empty; pending exception flags are cleared and the thread's
    /// control word restored afterwards.
    macro_rules! x87 {
        ($($template:literal),* ; $($operands:tt)*) => {{
            let mut saved = 0_u16;
            let control = 0x037F_u16;
            // SAFETY: See the macro's description; every operand points to
            // live, writable or readable storage of the accessed size.
            unsafe {
                asm!(
                    "fnstcw [{saved}]",
                    "fldcw [{control}]",
                    $($template,)*
                    "fnclex",
                    "fldcw [{saved}]",
                    saved = in(reg) &raw mut saved,
                    control = in(reg) &raw const control,
                    $($operands)*
                );
            }
        }};
    }

    fn binary(operation: u8, a: X87, b: X87) -> X87 {
        let (a, b) = (a.to_bits().to_le_bytes(), b.to_bits().to_le_bytes());
        let mut out = [0_u8; 16];
        let (a, b, result) = (a.as_ptr(), b.as_ptr(), out.as_mut_ptr());
        match operation {
            | 0 => x87!("fld tbyte ptr [{b}]", "fld tbyte ptr [{a}]", "fadd st(0), st(1)",
                "fstp tbyte ptr [{out}]", "fstp st(0)";
                a = in(reg) a, b = in(reg) b, out = in(reg) result, out("st(0)") _, out("st(1)") _),
            | 1 => x87!("fld tbyte ptr [{b}]", "fld tbyte ptr [{a}]", "fsub st(0), st(1)",
                "fstp tbyte ptr [{out}]", "fstp st(0)";
                a = in(reg) a, b = in(reg) b, out = in(reg) result, out("st(0)") _, out("st(1)") _),
            | 2 => x87!("fld tbyte ptr [{b}]", "fld tbyte ptr [{a}]", "fmul st(0), st(1)",
                "fstp tbyte ptr [{out}]", "fstp st(0)";
                a = in(reg) a, b = in(reg) b, out = in(reg) result, out("st(0)") _, out("st(1)") _),
            | 3 => x87!("fld tbyte ptr [{b}]", "fld tbyte ptr [{a}]", "fdiv st(0), st(1)",
                "fstp tbyte ptr [{out}]", "fstp st(0)";
                a = in(reg) a, b = in(reg) b, out = in(reg) result, out("st(0)") _, out("st(1)") _),
            | 4 => x87!("fld tbyte ptr [{b}]", "fld tbyte ptr [{a}]", "2:", "fprem", "fnstsw ax",
                "test ah, 4", "jnz 2b", "fstp tbyte ptr [{out}]", "fstp st(0)";
                a = in(reg) a, b = in(reg) b, out = in(reg) result, out("ax") _, out("st(0)") _,
                out("st(1)") _),
            | _ => x87!("fld tbyte ptr [{b}]", "fld tbyte ptr [{a}]", "2:", "fprem1", "fnstsw ax",
                "test ah, 4", "jnz 2b", "fstp tbyte ptr [{out}]", "fstp st(0)";
                a = in(reg) a, b = in(reg) b, out = in(reg) result, out("ax") _, out("st(0)") _,
                out("st(1)") _),
        }
        X87::from_bits(u128::from_le_bytes(out))
    }

    /// One operand through `instruction`, from and to ten bytes.
    fn unary(instruction: u8, a: X87) -> X87 {
        let a = a.to_bits().to_le_bytes();
        let mut out = [0_u8; 16];
        let (a, result) = (a.as_ptr(), out.as_mut_ptr());
        if instruction == 0 {
            x87!("fld tbyte ptr [{a}]", "fsqrt", "fstp tbyte ptr [{out}]";
                a = in(reg) a, out = in(reg) result, out("st(0)") _);
        } else {
            x87!("fld tbyte ptr [{a}]", "frndint", "fstp tbyte ptr [{out}]";
                a = in(reg) a, out = in(reg) result, out("st(0)") _);
        }
        X87::from_bits(u128::from_le_bytes(out))
    }

    /// `fucomip` flags as an ordering: `None` when unordered.
    fn compare(a: X87, b: X87) -> Option<std::cmp::Ordering> {
        let (a, b) = (a.to_bits().to_le_bytes(), b.to_bits().to_le_bytes());
        let (mut zero, mut parity, mut carry) = (0_u8, 0_u8, 0_u8);
        x87!("fld tbyte ptr [{b}]", "fld tbyte ptr [{a}]", "fucomip st(0), st(1)", "setz {z}",
            "setp {p}", "setc {c}", "fstp st(0)";
            a = in(reg) a.as_ptr(), b = in(reg) b.as_ptr(), z = out(reg_byte) zero,
            p = out(reg_byte) parity, c = out(reg_byte) carry, out("st(0)") _, out("st(1)") _);
        match (zero, parity, carry) {
            | (_, 1, _) => None,
            | (1, ..) => Some(std::cmp::Ordering::Equal),
            | (_, _, 1) => Some(std::cmp::Ordering::Less),
            | _ => Some(std::cmp::Ordering::Greater),
        }
    }

    fn to_f64(a: X87) -> u64 {
        let a = a.to_bits().to_le_bytes();
        let mut out = 0_u64;
        x87!("fld tbyte ptr [{a}]", "fstp qword ptr [{out}]";
            a = in(reg) a.as_ptr(), out = in(reg) &raw mut out, out("st(0)") _);
        out
    }

    fn to_f32(a: X87) -> u32 {
        let a = a.to_bits().to_le_bytes();
        let mut out = 0_u32;
        x87!("fld tbyte ptr [{a}]", "fstp dword ptr [{out}]";
            a = in(reg) a.as_ptr(), out = in(reg) &raw mut out, out("st(0)") _);
        out
    }

    fn from_f64(bits: u64) -> X87 {
        let mut out = [0_u8; 16];
        x87!("fld qword ptr [{a}]", "fstp tbyte ptr [{out}]";
            a = in(reg) &raw const bits, out = in(reg) out.as_mut_ptr(), out("st(0)") _);
        X87::from_bits(u128::from_le_bytes(out))
    }

    fn from_f32(bits: u32) -> X87 {
        let mut out = [0_u8; 16];
        x87!("fld dword ptr [{a}]", "fstp tbyte ptr [{out}]";
            a = in(reg) &raw const bits, out = in(reg) out.as_mut_ptr(), out("st(0)") _);
        X87::from_bits(u128::from_le_bytes(out))
    }

    fn from_i64(value: i64) -> X87 {
        let mut out = [0_u8; 16];
        x87!("fild qword ptr [{a}]", "fstp tbyte ptr [{out}]";
            a = in(reg) &raw const value, out = in(reg) out.as_mut_ptr(), out("st(0)") _);
        X87::from_bits(u128::from_le_bytes(out))
    }

    /// `fisttp`: truncation, or the integer indefinite when out of range.
    fn to_i64(a: X87) -> i64 {
        let a = a.to_bits().to_le_bytes();
        let mut out = 0_i64;
        x87!("fld tbyte ptr [{a}]", "fisttp qword ptr [{out}]";
            a = in(reg) a.as_ptr(), out = in(reg) &raw mut out, out("st(0)") _);
        out
    }

    /// A deterministic generator of encodings biased toward the edges.
    struct Encodings(u64);

    impl Encodings {
        fn next_u64(&mut self) -> u64 {
            // xorshift64*
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn exponent(&mut self) -> u16 {
            let random = self.next_u64();
            #[expect(clippy::cast_possible_truncation, reason = "Masked to 15 bits.")]
            match random % 10 {
                | 0 => 0,
                | 1 => 1,
                | 2 => 0x7FFF,
                | 3 => 0x7FFE,
                | 4 => (random >> 8) as u16 % 64,
                | 5 => 0x7FFE - (random >> 8) as u16 % 64,
                | _ => 0x3FFF - 128 + (random >> 8) as u16 % 256,
            }
        }

        fn significand(&mut self) -> u64 {
            let random = self.next_u64();
            let bits = self.next_u64();
            match random % 12 {
                | 0 => 0,
                | 1 => 1 << 63,
                | 2 => u64::MAX,
                | 3 => bits >> 1,
                | 4 => (1 << 63) | (1 << 62) | (bits & 0xFF),
                | 5 => (1 << 63) | (bits & 0xFFFF),
                | 6 => bits & 0xFFFF,
                | 7 => (1 << 63) | (bits << 40),
                | _ => (1 << 63) | bits,
            }
        }

        fn value(&mut self) -> X87 {
            let sign = u128::from(self.next_u64() & 1) << 79;
            let exponent = u128::from(self.exponent()) << 64;
            X87::from_bits(sign | exponent | u128::from(self.significand()))
        }

        /// A second operand, often with an exponent close to `a`'s.
        fn partner(&mut self, a: X87) -> X87 {
            let b = self.value();
            if self.next_u64().is_multiple_of(2) {
                return b;
            }
            let a_exponent = (a.to_bits() >> 64) & 0x7FFF;
            let delta = i128::from(self.next_u64() & 0x7F) - 64;
            let exponent = (i128::try_from(a_exponent).unwrap() + delta).clamp(0, 0x7FFF);
            let exponent = u128::try_from(exponent).unwrap();
            X87::from_bits((b.to_bits() & !(0x7FFF << 64)) | exponent << 64)
        }
    }

    #[test]
    fn arithmetic_matches_the_processor() {
        let mut encodings = Encodings(0x9E37_79B9_7F4A_7C15);
        for _ in 0..200_000 {
            let a = encodings.value();
            let b = encodings.partner(a);
            assert_eq!(a.add(b), binary(0, a, b), "{a:?} + {b:?}");
            assert_eq!(a.sub(b), binary(1, a, b), "{a:?} - {b:?}");
            assert_eq!(a.mul(b), binary(2, a, b), "{a:?} * {b:?}");
            assert_eq!(a.div(b), binary(3, a, b), "{a:?} / {b:?}");
            assert_eq!(a.compare(b), compare(a, b), "{a:?} <=> {b:?}");
            assert_eq!(a.sqrt(), unary(0, a), "sqrt {a:?}");
        }
    }

    #[test]
    fn fmod_matches_the_processor() {
        let mut encodings = Encodings(0x0123_4567_89AB_CDEF);
        for _ in 0..20_000 {
            let a = encodings.value();
            let b = encodings.partner(a);
            assert_eq!(a.rem(b), binary(4, a, b), "fmod({a:?}, {b:?})");
            assert_eq!(a.remainder(b), binary(5, a, b), "remainder({a:?}, {b:?})");
            assert_eq!(
                a.round_integral(super::super::Integral::HalfEven),
                unary(1, a),
                "rint {a:?}"
            );
        }
    }

    #[test]
    fn conversions_match_the_processor() {
        let mut encodings = Encodings(0xDEAD_BEEF_CAFE_F00D);
        for _ in 0..200_000 {
            let a = encodings.value();
            assert_eq!(a.to_f64().to_bits(), to_f64(a), "{a:?} as f64");
            assert_eq!(a.to_f32().to_bits(), to_f32(a), "{a:?} as f32");
            let expected = to_i64(a);
            match a.to_integer(true, 64) {
                | Some(bits) => assert_eq!(
                    u64::try_from(bits).unwrap().cast_signed(),
                    expected,
                    "{a:?} as i64"
                ),
                | None => assert_eq!(expected, i64::MIN, "{a:?} as i64"),
            }
            let bits = encodings.next_u64();
            let narrow = f64::from_bits(bits);
            assert_eq!(X87::from_f64(narrow), from_f64(bits), "{bits:#X} as f80");
            let narrow = f32::from_bits(u32::try_from(bits >> 32).unwrap());
            assert_eq!(
                X87::from_f32(narrow),
                from_f32(narrow.to_bits()),
                "{narrow:?} as f80"
            );
            let integer = bits.cast_signed() >> (bits % 64);
            assert_eq!(
                X87::from_integer(integer < 0, u128::from(integer.unsigned_abs())),
                from_i64(integer),
                "{integer} as f80"
            );
        }
    }
}
