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

pub(super) const LONG_DOUBLE_HEX_CAPACITY: usize = ffi::LONG_DOUBLE_HEX_CAPACITY as _;

impl LongDouble {
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

    pub(crate) fn is_zero(self) -> bool {
        self.classify() == FloatClass::Zero
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
