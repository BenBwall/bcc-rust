//! Native long-double bytes support arithmetic, comparison, hexadecimal
//! formatting, and classification through the C bridge.

use std::{
    ffi::c_char,
    fmt::{
        Debug,
        Display,
        Formatter,
    },
};

use super::{
    ffi,
    range::FloatClass,
};

/// A host `long double` stored as its object representation with any
/// padding bytes zeroed, so equality and hashing depend only on the value.
#[derive(PartialEq, Eq, Clone, Copy)]
pub(crate) struct LongDouble {
    pub(crate) value: [u8; LONG_DOUBLE_BYTES],
}

pub(super) const LONG_DOUBLE_BYTES: usize = ffi::LONG_DOUBLE_BYTES as _;

const LONG_DOUBLE_HEX_CAPACITY: usize = ffi::LONG_DOUBLE_HEX_CAPACITY as _;

impl LongDouble {
    /// Decodes the host carrier into exact x87 parts for target conversion.
    /// C99: §6.3.1.5, p. 44; PDF p. 56 (real floating conversions).
    pub(crate) fn x87_parts(self) -> Option<(bool, u16, u64)> {
        Self::decode_x87(&self.value[..ffi::LONG_DOUBLE_VALUE_BYTES as usize])
    }

    /// Decodes either supported host object representation, without padding.
    pub(crate) fn decode_x87(bytes: &[u8]) -> Option<(bool, u16, u64)> {
        if let Ok(bytes) = <[u8; 8]>::try_from(bytes) {
            let bits = u64::from_le_bytes(bytes);
            let sign = bits >> 63 != 0;
            let exponent = (bits >> 52) & 0x7FF;
            let fraction = bits & ((1 << 52) - 1);
            let (exponent, significand) = if exponent == 0x7FF {
                // Preserve the payload and the quiet bit, which lands on the
                // x87 quiet bit, so a signaling NaN stays one; a NaN's
                // fraction is nonzero, so it stays a NaN. Infinity has only
                // the explicit integer bit.
                (0x7FFF, (1 << 63) | (fraction << 11))
            } else if exponent == 0 && fraction == 0 {
                (0, 0)
            } else {
                let significand = fraction | if exponent == 0 { 0 } else { 1 << 52 };
                let shift = significand.leading_zeros();
                let biased = i64::try_from(exponent.max(1)).ok()? - 1023 - 52 + 63
                    - i64::from(shift)
                    + 16383;
                (u16::try_from(biased).ok()?, significand << shift)
            };
            return Some((sign, exponent, significand));
        }
        let bytes = <[u8; 10]>::try_from(bytes).ok()?;
        let significand = u64::from_le_bytes(bytes[..8].try_into().ok()?);
        let top = u16::from_le_bytes(bytes[8..10].try_into().ok()?);
        Some((top >> 15 == 1, top & 0x7FFF, significand))
    }

    /// C99: §6.6p4, p. 95; PDF p. 107; §6.3.1.8, pp. 44-45;
    /// PDF pp. 56-57. Native arithmetic shares the literal carrier, and
    /// infinite or NaN results are the IEC 60559 values of Annex F.
    pub(crate) fn arithmetic(self, right: Self, operation: i32, precision: i32) -> Self {
        // SAFETY: Both carriers are initialized native representations; the C
        // function returns a fully initialized carrier by value.
        let result = unsafe {
            ffi::long_double_arithmetic(self.to_ffi(), right.to_ffi(), operation, precision)
        };
        // SAFETY: C initializes all bytes, including padding.
        Self {
            // SAFETY: The C function initializes all carrier bytes.
            value: unsafe { result.bytes },
        }
    }

    /// -1, 0 or 1 for ordered operands, and 2 when either is a NaN.
    pub(crate) fn compare(self, right: Self) -> i32 {
        // SAFETY: Both carriers contain valid native values and are passed by
        // copy.
        unsafe { ffi::long_double_compare(self.to_ffi(), right.to_ffi()) }
    }

    pub(super) fn classify(self) -> FloatClass {
        // SAFETY: The bytes encode a host `long double` (every value comes
        // from `ZERO` or a C helper), and the function only reads the copy.
        let class = i64::from(unsafe { ffi::long_double_classify(self.to_ffi()) });
        // Bindgen types anonymous enum constants per platform, so compare
        // through a common width.
        if class == i64::from(ffi::FLOAT_CLASS_ZERO) {
            FloatClass::Zero
        } else if class == i64::from(ffi::FLOAT_CLASS_INFINITE) {
            FloatClass::Infinite
        } else if class == i64::from(ffi::FLOAT_CLASS_NAN) {
            FloatClass::NotANumber
        } else {
            FloatClass::Nonzero
        }
    }

    pub(super) fn to_ffi(self) -> ffi::long_double_t {
        ffi::long_double_t { bytes: self.value }
    }

    /// The additive inverse, keeping the sign of zero (Annex F.3).
    pub(crate) fn negate(self) -> Self {
        self.arithmetic(Self::ZERO, 5, 3)
    }

    pub(crate) fn from_double(value: f64) -> Self {
        // SAFETY: The C function accepts a scalar and initializes all carrier
        // bytes.
        let result = unsafe { ffi::long_double_from_double(value) };
        // SAFETY: All union bytes are initialized by C.
        Self {
            // SAFETY: The C function initializes all carrier bytes.
            value: unsafe { result.bytes },
        }
    }

    /// Constructs a quiet NaN with the target precision's low payload bits.
    /// The x87 carrier stores binary32/64 payloads in the corresponding high
    /// significand positions so narrowing preserves the requested bits.
    /// C99: §7.12.11.2p2, p. 237; PDF p. 249 (implementation-defined payload).
    pub(crate) fn quiet_nan(payload: u64, precision: i32) -> Self {
        let fraction = match precision {
            | 1 => (payload & ((1 << 22) - 1)) << 29,
            | _ => payload & ((1 << 51) - 1),
        };
        if precision != 3 || ffi::LONG_DOUBLE_VALUE_BYTES == 8 {
            return Self::from_double(f64::from_bits(0x7FF8_0000_0000_0000 | fraction));
        }
        let mut value = Self::ZERO;
        let significand = (1 << 63) | (1 << 62) | (payload & ((1 << 62) - 1));
        value.value[..8].copy_from_slice(&significand.to_le_bytes());
        value.value[8..10].copy_from_slice(&0x7FFF_u16.to_le_bytes());
        value
    }

    /// Constructs a signaling NaN with the target precision's low payload
    /// bits, as GCC's `__builtin_nans` family does: the quiet bit is clear,
    /// and a payload of zero sets the bit below it, so the value stays a
    /// NaN. The carrier bytes are written directly, since a load through
    /// the x87 unit would quiet the NaN; binary32 and binary64 payloads sit
    /// in the high significand positions their format's fraction occupies,
    /// as for [`Self::quiet_nan`].
    /// C99: §7.12.11.2p2, p. 237; PDF p. 249 (implementation-defined payload);
    /// signaling NaNs, §5.2.4.2.2 paragraph 3, p. 24; PDF p. 36.
    pub(crate) fn signaling_nan(payload: u64, precision: i32) -> Self {
        let binary64_carrier = ffi::LONG_DOUBLE_VALUE_BYTES == 8;
        // Fraction bits of the destination format; the top one is the
        // quiet bit.
        let fraction_bits = match precision {
            | 1 => 23,
            | 2 => 52,
            | _ if binary64_carrier => 52,
            | _ => 63,
        };
        let mut payload = payload & ((1 << (fraction_bits - 1)) - 1);
        if payload == 0 {
            payload = 1 << (fraction_bits - 2);
        }
        let mut value = Self::ZERO;
        if binary64_carrier {
            let bits = 0x7FF0_0000_0000_0000 | payload << (52 - fraction_bits);
            value.value[..8].copy_from_slice(&bits.to_le_bytes());
        } else {
            let significand = (1 << 63) | payload << (63 - fraction_bits);
            value.value[..8].copy_from_slice(&significand.to_le_bytes());
            value.value[8..10].copy_from_slice(&0x7FFF_u16.to_le_bytes());
        }
        value
    }

    pub(crate) fn is_zero(self) -> bool {
        self.classify() == FloatClass::Zero
    }

    /// C99 §7.12.11.1, p. 236; PDF p. 248: copysign changes only the
    /// representation's sign, including zero and NaN. Both host carriers
    /// are little-endian; padding remains untouched.
    pub(crate) fn with_sign(mut self, sign: bool) -> Self {
        let byte = if ffi::LONG_DOUBLE_VALUE_BYTES == 8 {
            7
        } else {
            9
        };
        self.value[byte] = (self.value[byte] & 0x7F) | if sign { 0x80 } else { 0 };
        self
    }
}

impl LongDouble {
    pub(crate) const ZERO: Self = Self {
        value: [0; LONG_DOUBLE_BYTES],
    };
}

/// Prints the exact value as a C99 hexadecimal floating constant such as
/// `0x1.8p+0`. The digits come from exact arithmetic rather than the C
/// library's `printf`, so the text is the same on every host that shares a
/// `long double` format.
/// C99: hexadecimal `floating-constant` §6.4.4.2p1-3, pp. 57-58; PDF pp. 69-70.
impl Display for LongDouble {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut buffer = [0_u8; LONG_DOUBLE_HEX_CAPACITY];
        // SAFETY: The bytes encode a host `long double`. The buffer is
        // writable for its full length, which is passed as the capacity; the
        // function writes at most that many bytes and keeps no pointer.
        let length = unsafe {
            ffi::long_double_to_hex(
                self.to_ffi(),
                buffer.as_mut_ptr().cast::<c_char>(),
                buffer.len(),
            )
        };
        let text = buffer.get(..length).unwrap_or_default();
        f.write_str(std::str::from_utf8(text).unwrap_or("<long double>"))
    }
}

impl Debug for LongDouble {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "LongDouble({self})")
    }
}
