use std::ffi::c_char;

/// A deterministic pseudo-random source (xorshift64*).
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    /// An x87 encoding, biased towards the cases that need care: zeros,
    /// infinities, NaNs, subnormals, values at the ends of the range, and
    /// values that are binary32 or binary64 values.
    fn x87(&mut self) -> [u8; 10] {
        let sign = u16::from(self.below(2) == 0) << 15;
        let random = self.next();
        let (exponent, significand): (u16, u64) = match self.below(16) {
            | 0 => (0, 0),
            | 1 => (0x7FFF, 1 << 63),
            | 2 => (0x7FFF, (1 << 63) | (1 << 62) | (random >> 30)),
            | 3 => (0x7FFF, (1 << 63) | (random >> 2) | 1),
            | 4 => (0, random >> self.below(64)),
            | 5 => (0x7FFE - self.below(3) as u16, random | (1 << 63)),
            | 6 => (1 + self.below(3) as u16, random | (1 << 63)),
            | 7 => (
                16383 - 126 + self.below(254) as u16,
                (random | (1 << 63)) & !((1 << 40) - 1),
            ),
            | 8 => (
                16383 - 1022 + self.below(2046) as u16,
                (random | (1 << 63)) & !((1 << 11) - 1),
            ),
            | 9 => (
                16383 - 20 + self.below(40) as u16,
                (1 << 63) | (random & 0xFF00_0000_0000_0000),
            ),
            | _ => (16383 - 200 + self.below(400) as u16, random | (1 << 63)),
        };
        let mut bytes = [0; 10];
        bytes[..8].copy_from_slice(&significand.to_le_bytes());
        bytes[8..].copy_from_slice(&(exponent | sign).to_le_bytes());
        bytes
    }

    fn f64(&mut self) -> f64 {
        let random = self.next();
        f64::from_bits(match self.below(10) {
            | 0 => random & (1 << 63),
            | 1 => (random & (1 << 63)) | 0x7FF0_0000_0000_0000,
            | 2 => random | 0x7FF0_0000_0000_0001,
            | 3 => random & 0x800F_FFFF_FFFF_FFFF,
            | 4 => (random & 0x800F_FFFF_FFFF_FFFF) | 0x3FF0_0000_0000_0000,
            | _ => random,
        })
    }

    fn f32(&mut self) -> f32 {
        let random = self.next() as u32;
        f32::from_bits(match self.below(10) {
            | 0 => random & (1 << 31),
            | 1 => (random & (1 << 31)) | 0x7F80_0000,
            | 2 => random | 0x7F80_0001,
            | 3 => random & 0x807F_FFFF,
            | 4 => (random & 0x807F_FFFF) | 0x3F80_0000,
            | _ => random,
        })
    }

    /// A floating constant's spelling, possibly with a trailing suffix or
    /// stray characters.
    fn spelling(&mut self) -> String {
        let mut text = String::new();
        let hex = self.below(3) == 0;
        if hex {
            text.push_str(if self.below(2) == 0 { "0x" } else { "0X" });
        }
        let digits = b"0123456789abcdefABCDEF";
        let radix = if hex { 22 } else { 10 };
        let length = match self.below(4) {
            | 0 => self.below(3),
            | 1 => 1 + self.below(25),
            | _ => 1 + self.below(60),
        };
        let point = self.below(length + 2);
        for index in 0..length {
            if index == point {
                text.push('.');
            }
            // Mostly nines or zeros for halfway and carry cases.
            let digit = match self.below(6) {
                | 0 => b'0',
                | 1 if !hex => b'9',
                | 1 => b'f',
                | _ => digits[self.below(radix) as usize],
            };
            text.push(char::from(digit));
        }
        if point == length {
            text.push('.');
        }
        if self.below(3) != 0 {
            text.push(match (hex, self.below(2)) {
                | (true, 0) => 'p',
                | (true, _) => 'P',
                | (false, 0) => 'e',
                | (false, _) => 'E',
            });
            match self.below(3) {
                | 0 => text.push('-'),
                | 1 => text.push('+'),
                | _ => {},
            }
            let magnitude = match self.below(5) {
                | 0 => self.below(10),
                | 1 => self.below(400),
                | 2 => 4900 + self.below(100),
                | 3 => 16300 + self.below(200),
                | _ => self.below(2000),
            };
            if self.below(20) != 0 {
                text.push_str(&magnitude.to_string());
            }
        }
        match self.below(6) {
            | 0 => text.push('L'),
            | 1 => text.push('f'),
            | 2 => text.push_str("e+"),
            | _ => {},
        }
        text.push('\0');
        text
    }
}

/// A carrier's bytes from an x87 encoding, with zero padding.
fn carrier(value: [u8; 10]) -> [u8; 16] {
    let mut bytes = [0; 16];
    bytes[..10].copy_from_slice(&value);
    bytes
}

/// The bytes of several carriers.
type Carriers<const N: usize> = [[u8; 16]; N];

/// One implementation of the bridge's interface, as safe functions over
/// carrier bytes.
struct Bridge {
    strtod:              fn(&str) -> (usize, u64),
    strtof:              fn(&str) -> (usize, u32),
    strtold:             fn(&str) -> (usize, [u8; 16]),
    arithmetic:          fn([u8; 16], [u8; 16], i32, i32) -> [u8; 16],
    compare:             fn([u8; 16], [u8; 16]) -> i32,
    classify:            fn([u8; 16]) -> i32,
    hex:                 fn([u8; 16]) -> String,
    from_double:         fn(f64) -> [u8; 16],
    complex_float:       fn([f32; 4], i32) -> [f32; 2],
    complex_double:      fn([f64; 4], i32) -> [f64; 2],
    complex_long_double: fn(Carriers<4>, i32, i32) -> Carriers<2>,
}

/// The [`Bridge`] over the module at `$path`: the stand-in, or the C
/// bridge's bindings, which share every name.
macro_rules! bridge {
    ($($path:tt)*) => {
        Bridge {
            strtod: |text| {
                let mut end = std::ptr::null_mut();
                // SAFETY: The text is NUL-terminated and the end pointer is
                // writable.
                let value = unsafe { $($path)*::string_to_double(text.as_ptr().cast(), &raw mut end) };
                (end.addr() - text.as_ptr().addr(), value.to_bits())
            },
            strtof: |text| {
                let mut end = std::ptr::null_mut();
                // SAFETY: As for `strtod`.
                let value = unsafe { $($path)*::string_to_float(text.as_ptr().cast(), &raw mut end) };
                (end.addr() - text.as_ptr().addr(), value.to_bits())
            },
            strtold: |text| {
                let mut end = std::ptr::null_mut();
                let value =
                    // SAFETY: As for `strtod`.
                    unsafe { $($path)*::string_to_long_double(text.as_ptr().cast(), &raw mut end) };
                // SAFETY: Every carrier byte is initialized.
                (end.addr() - text.as_ptr().addr(), unsafe { value.bytes })
            },
            arithmetic: |left, right, operation, precision| {
                let carrier = |bytes| $($path)*::long_double_t { bytes };
                // SAFETY: Both carriers are initialized.
                let result = unsafe {
                    $($path)*::long_double_arithmetic(carrier(left), carrier(right), operation, precision)
                };
                // SAFETY: Every carrier byte is initialized.
                unsafe { result.bytes }
            },
            compare: |left, right| {
                let carrier = |bytes| $($path)*::long_double_t { bytes };
                // SAFETY: Both carriers are initialized.
                unsafe { $($path)*::long_double_compare(carrier(left), carrier(right)) }
            },
            classify: |value| {
                // SAFETY: The carrier is initialized.
                unsafe { $($path)*::long_double_classify($($path)*::long_double_t { bytes: value }) }
            },
            hex: |value| {
                let mut buffer: [c_char; 64] = [0; 64];
                let carrier = $($path)*::long_double_t { bytes: value };
                let length =
                    // SAFETY: The buffer is writable for its length.
                    unsafe { $($path)*::long_double_to_hex(carrier, buffer.as_mut_ptr(), buffer.len()) };
                buffer[..length].iter().map(|&byte| char::from(byte as u8)).collect()
            },
            from_double: |value| {
                // SAFETY: The function takes a scalar.
                let result = unsafe { $($path)*::long_double_from_double(value) };
                // SAFETY: Every carrier byte is initialized.
                unsafe { result.bytes }
            },
            complex_float: |[ar, ai, br, bi], operation| {
                let (mut real, mut imag) = (0.0, 0.0);
                // SAFETY: The outputs are writable.
                unsafe {
                    $($path)*::bcc_complex_float(ar, ai, br, bi, operation, &raw mut real, &raw mut imag);
                }
                [real, imag]
            },
            complex_double: |[ar, ai, br, bi], operation| {
                let (mut real, mut imag) = (0.0, 0.0);
                // SAFETY: The outputs are writable.
                unsafe {
                    $($path)*::bcc_complex_double(ar, ai, br, bi, operation, &raw mut real, &raw mut imag);
                }
                [real, imag]
            },
            complex_long_double: |parts, operation, precision| {
                let [ar, ai, br, bi] = parts.map(|bytes| $($path)*::long_double_t { bytes });
                let mut real = $($path)*::long_double_t { bytes: [0; 16] };
                let mut imag = $($path)*::long_double_t { bytes: [0; 16] };
                // SAFETY: The carriers are initialized and the outputs writable.
                unsafe {
                    $($path)*::long_double_complex_arithmetic(
                        ar, ai, br, bi, operation, precision, &raw mut real, &raw mut imag,
                    );
                }
                // SAFETY: The function initialized every byte of both outputs.
                [unsafe { real.bytes }, unsafe { imag.bytes }]
            },
        }
    };
}

const SOFT: Bridge = bridge!(super);

#[test]
fn conversions_round_once_and_stop_where_strtod_does() {
    for (text, consumed, bits) in [
        ("0.1\0", 3, 0x3FB9_9999_9999_999A),
        ("1e-324\0", 6, 0),
        ("2.4703282292062328e-324\0", 23, 1),
        ("0x1.8p1\0", 7, 0x4008_0000_0000_0000),
        ("0x1p\0", 3, 0x3FF0_0000_0000_0000),
        ("0x.p1\0", 1, 0),
        ("1e+\0", 1, 0x3FF0_0000_0000_0000),
        ("1.5f\0", 3, 0x3FF8_0000_0000_0000),
        ("1e999\0", 5, 0x7FF0_0000_0000_0000),
        (".\0", 0, 0),
    ] {
        assert_eq!((SOFT.strtod)(text), (consumed, bits), "{text:?}");
    }
    for (text, expected) in [
        ("0.1L\0", "0x1.999999999999999ap-4"),
        ("1e4932L\0", "0x1.ae596552b8fded9ap+16383"),
        ("3.6451995318824746025e-4951L\0", "0x1p-16445"),
        ("1e-5000L\0", "0x0p+0"),
        ("0x1.00000000000000008p0L\0", "0x1p+0"),
        ("0x1.00000000000000018p0L\0", "0x1.0000000000000002p+0"),
    ] {
        assert_eq!((SOFT.hex)((SOFT.strtold)(text).1), expected, "{text:?}");
    }
}

#[test]
fn x87_arithmetic_rounds_to_the_requested_precision() {
    let value = |text: &str| (SOFT.strtold)(text).1;
    let (tenth, fifth) = (value("0.1L\0"), value("0.2L\0"));
    let [sum, single, double] =
        [3, 1, 2].map(|precision| (SOFT.arithmetic)(tenth, fifth, 0, precision));
    assert_eq!((SOFT.hex)(sum), "0x1.3333333333333334p-2");
    assert_eq!((SOFT.hex)(single), "0x1.333334p-2");
    assert_eq!((SOFT.hex)(double), "0x1.3333333333334p-2");
    let quotient = (SOFT.arithmetic)(value("1.0L\0"), value("3.0L\0"), 3, 3);
    assert_eq!((SOFT.hex)(quotient), "0x1.5555555555555556p-2");
    let invalid = (SOFT.arithmetic)(value("0.0L\0"), value("0.0L\0"), 3, 3);
    assert_eq!(invalid[..10], [0, 0, 0, 0, 0, 0, 0, 0xC0, 0xFF, 0xFF]);
}

/// The stand-in and the C bridge agree on every operation.
#[cfg(feature = "native")]
mod differential {
    use super::*;

    const C: Bridge = bridge!(super::super::super::ffi);

    #[test]
    fn conversions_match_the_c_library() {
        let mut random = Random(0x9E37_79B9_7F4A_7C15);
        for _ in 0..40_000 {
            let text = random.spelling();
            assert_eq!((SOFT.strtod)(&text), (C.strtod)(&text), "strtod {text:?}");
            assert_eq!((SOFT.strtof)(&text), (C.strtof)(&text), "strtof {text:?}");
            assert_eq!(
                (SOFT.strtold)(&text),
                (C.strtold)(&text),
                "strtold {text:?}"
            );
        }
    }

    #[test]
    fn arithmetic_comparison_and_formatting_match_the_bridge() {
        let mut random = Random(0x0123_4567_89AB_CDEF);
        for _ in 0..200_000 {
            let (left, right) = (carrier(random.x87()), carrier(random.x87()));
            let operation = random.below(7) as i32;
            let precision = 1 + random.below(3) as i32;
            let operands = format!("{:x?} {:x?}", &left[..10], &right[..10]);
            assert_eq!(
                (SOFT.arithmetic)(left, right, operation, precision),
                (C.arithmetic)(left, right, operation, precision),
                "{operands}: op {operation} at precision {precision}"
            );
            assert_eq!(
                (SOFT.compare)(left, right),
                (C.compare)(left, right),
                "{operands}"
            );
            assert_eq!((SOFT.classify)(left), (C.classify)(left), "{operands}");
            assert_eq!((SOFT.hex)(left), (C.hex)(left), "{operands}");
            let double = random.f64();
            assert_eq!(
                (SOFT.from_double)(double),
                (C.from_double)(double),
                "{:x}",
                double.to_bits()
            );
        }
    }

    /// Any NaN, whose payload a complex routine's operand order picks.
    fn nan_or<T>(nan: bool, value: T) -> Option<T> {
        (!nan).then_some(value)
    }

    #[test]
    fn complex_arithmetic_matches_the_bridge() {
        let mut random = Random(0xDEAD_BEEF_CAFE_F00D);
        let float = |part: f32| nan_or(part.is_nan(), part.to_bits());
        let double = |part: f64| nan_or(part.is_nan(), part.to_bits());
        let x87 = |bytes: [u8; 16]| {
            let exponent = u16::from_le_bytes([bytes[8], bytes[9]]) & 0x7FFF;
            let significand = u64::from_le_bytes(bytes[..8].try_into().unwrap());
            nan_or(exponent == 0x7FFF && significand << 1 != 0, bytes)
        };
        for _ in 0..100_000 {
            let operation = random.below(4) as i32;
            let parts = [(); 4].map(|()| random.f32());
            assert_eq!(
                (SOFT.complex_float)(parts, operation).map(float),
                (C.complex_float)(parts, operation).map(float),
                "float {parts:?} op {operation}"
            );
            let parts = [(); 4].map(|()| random.f64());
            assert_eq!(
                (SOFT.complex_double)(parts, operation).map(double),
                (C.complex_double)(parts, operation).map(double),
                "double {parts:?} op {operation}"
            );
            let parts = [(); 4].map(|()| carrier(random.x87()));
            let precision = 1 + random.below(3) as i32;
            assert_eq!(
                (SOFT.complex_long_double)(parts, operation, precision).map(x87),
                (C.complex_long_double)(parts, operation, precision).map(x87),
                "long double {:x?} op {operation} at precision {precision}",
                parts.map(SOFT.hex)
            );
        }
    }
}
