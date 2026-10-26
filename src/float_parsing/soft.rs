//! A pure-Rust stand-in for the C floating bridge
//! (`build_support/float_parsing.c` behind `ffi.rs`), for builds without a C
//! toolchain such as the explorer's WebAssembly one. It has the bridge's
//! interface, so the rest of `float_parsing` calls either the same way, and
//! it reproduces the bridge as built for x86-64 Linux: `long double` is x87
//! double-extended precision in a 16-byte carrier with six zero padding
//! bytes, conversions round as glibc's `strtod` family does, arithmetic
//! rounds and chooses NaNs as the x87 FPU and SSE do, and complex operations
//! follow Clang and the runtime routines it calls. One thing is not
//! reproduced: which NaN operand a complex routine's result carries, since
//! the C compiler may commute the routine's additions and multiplications.
//!
//! Each operation unpacks its operands into a [`value::Soft`], computes the
//! exact result with integers, rounds it once to the precision the bridge
//! would have used, and packs it again. For `0.1L + 0.2L` at precision 3,
//! the two significands are aligned with 62 guard bits, added, and rounded to
//! 64 bits, as the x87 `fadd` does.
//!
//! Read [`long_double_arithmetic`], then `value.rs` and `arithmetic.rs`.
//!
//! Files by role:
//! - Unpacked values and rounding: `value.rs`.
//! - Arithmetic and the runtime's math helpers: `arithmetic.rs`.
//! - Decimal and hexadecimal conversion: `conversion.rs`.
//! - Complex multiplication and division: `complex.rs`.
//! - Differential and unit tests: `tests.rs`.
//!
//! C99: floating constants, §6.4.4.2 paragraphs 3-5, p. 58; PDF p. 70;
//! `strtod`, §7.20.1.3, pp. 307-310; PDF pp. 319-322; IEC 60559
//! arithmetic, Annex F, pp. 444-467; PDF pp. 456-479.

#![allow(
    non_camel_case_types,
    dead_code,
    clippy::allow_attributes,
    reason = "The stand-in keeps the generated bindings' names, and not every build uses all of \
              them."
)]
#![expect(
    clippy::cast_possible_truncation,
    reason = "Encodings are split into fields by masking and shifting before each cast."
)]

// Values
mod arithmetic;
mod value;

// Operations
mod complex;
mod conversion;

use std::{
    ffi::{
        CStr,
        c_char,
        c_int,
    },
    fmt::{
        self,
        Write as _,
    },
};

use arithmetic::{
    Operation,
    apply,
    compare,
};
use value::{
    F32,
    F64,
    Format,
    Soft,
    Value,
    X87,
};

/// `sizeof(long double)` on x86-64 Linux.
pub(super) const LONG_DOUBLE_BYTES: u32 = 16;
/// The x87 value bytes; the rest of the carrier is padding.
pub(super) const LONG_DOUBLE_VALUE_BYTES: u32 = 10;
pub(super) const LONG_DOUBLE_HEX_CAPACITY: u32 = 64;
pub(super) const FLOAT_CLASS_NONZERO: u32 = 0;
pub(super) const FLOAT_CLASS_ZERO: u32 = 1;
pub(super) const FLOAT_CLASS_INFINITE: u32 = 2;
pub(super) const FLOAT_CLASS_NAN: u32 = 3;

/// The bridge's `long double` carrier.
#[derive(Clone, Copy)]
#[repr(C)]
pub(super) union long_double_t {
    pub(super) bytes: [u8; LONG_DOUBLE_BYTES as usize],
}

impl long_double_t {
    fn soft(self) -> Soft {
        // SAFETY: The union's only field is plain bytes.
        let bytes = unsafe { self.bytes };
        let mut value = [0; 10];
        value.copy_from_slice(&bytes[..10]);
        Soft::from_x87(value)
    }

    /// Stores an x87 value with zero padding.
    fn store(value: Soft) -> Self {
        let mut bytes = [0; LONG_DOUBLE_BYTES as usize];
        bytes[..10].copy_from_slice(&value.to_x87());
        Self { bytes }
    }
}

/// `strtold`.
///
/// # Safety
///
/// `s` must point to a NUL-terminated string, and `endptr` to writable
/// storage for a pointer.
pub(super) unsafe fn string_to_long_double(
    s: *const c_char,
    endptr: *mut *mut c_char,
) -> long_double_t {
    // SAFETY: The caller passes a NUL-terminated string and a writable end
    // pointer.
    let value = unsafe { convert(s, endptr, X87) };
    long_double_t::store(value)
}

/// `strtod`.
///
/// # Safety
///
/// As for [`string_to_long_double`].
pub(super) unsafe fn string_to_double(s: *const c_char, endptr: *mut *mut c_char) -> f64 {
    // SAFETY: As the caller guarantees.
    let value = unsafe { convert(s, endptr, F64) };
    f64::from_bits(value.to_ieee(F64))
}

/// `strtof`.
///
/// # Safety
///
/// As for [`string_to_long_double`].
pub(super) unsafe fn string_to_float(s: *const c_char, endptr: *mut *mut c_char) -> f32 {
    // SAFETY: As the caller guarantees.
    let value = unsafe { convert(s, endptr, F32) };
    f32::from_bits(value.to_ieee(F32) as u32)
}

/// Converts the string at `s` and points `endptr` past what it consumed.
///
/// # Safety
///
/// As for [`string_to_long_double`].
unsafe fn convert(s: *const c_char, endptr: *mut *mut c_char, format: Format) -> Soft {
    // SAFETY: The caller passes a NUL-terminated string.
    let text = unsafe { CStr::from_ptr(s) }.to_bytes();
    let (consumed, value) = conversion::convert(text, format);
    // SAFETY: `consumed` is within the string.
    let end = unsafe { s.add(consumed) };
    // SAFETY: The caller's end pointer is writable.
    unsafe {
        endptr.write(end.cast_mut());
    }
    value
}

/// # Safety
///
/// None; the bridge's signature is unsafe.
pub(super) unsafe fn long_double_classify(value: long_double_t) -> c_int {
    let value = value.soft();
    let class = match value.value {
        | Value::Zero => FLOAT_CLASS_ZERO,
        | Value::Finite { .. } => FLOAT_CLASS_NONZERO,
        | Value::Infinity => FLOAT_CLASS_INFINITE,
        | Value::Nan { .. } | Value::Unsupported { .. } => FLOAT_CLASS_NAN,
    };
    class as c_int
}

/// Translation-time arithmetic with the bridge's operation and precision
/// codes: x87 arithmetic rounded to binary32 for precision 1, SSE binary64
/// arithmetic on operands rounded to binary64 for precision 2 (Microsoft
/// targets), and x87 arithmetic for precision 3.
/// C99: §6.6p4, p. 95; PDF p. 107; §6.3.1.8, pp. 44-45; PDF pp. 56-57.
///
/// # Safety
///
/// None; the bridge's signature is unsafe.
pub(super) unsafe fn long_double_arithmetic(
    left: long_double_t,
    right: long_double_t,
    operation: c_int,
    precision: c_int,
) -> long_double_t {
    // An x87 negation or copy (`fchs`, `fld` and `fstp` of the ten-byte
    // form) moves the encoding as it is, even one arithmetic would refuse
    // or normalize.
    if precision == 3 && !(0..=3).contains(&operation) {
        // SAFETY: The union's only field is plain bytes.
        let mut bytes = unsafe { left.bytes };
        bytes[10..].fill(0);
        if operation == 5 {
            bytes[9] ^= 0x80;
        }
        return long_double_t { bytes };
    }
    let (left, right) = (left.soft(), right.soft());
    let result = if precision == 2 {
        operate(operation, left.convert(F64), right.convert(F64), F64).convert(X87)
    } else {
        let result = operate(operation, left, right, X87);
        if precision == 1 {
            result.convert(F32).convert(X87)
        } else {
            result
        }
    };
    long_double_t::store(result)
}

/// One of the bridge's operation codes: add, subtract, multiply, divide,
/// negate the left operand (5), or keep it.
fn operate(operation: c_int, left: Soft, right: Soft, format: Format) -> Soft {
    let operation = match operation {
        | 0 => Operation::Add,
        | 1 => Operation::Subtract,
        | 2 => Operation::Multiply,
        | 3 => Operation::Divide,
        | 5 => return left.negated(),
        | _ => return left,
    };
    apply(operation, left, right, format)
}

/// # Safety
///
/// None; the bridge's signature is unsafe.
pub(super) unsafe fn long_double_from_double(value: f64) -> long_double_t {
    long_double_t::store(Soft::from_ieee(value.to_bits(), F64).convert(X87))
}

/// -1, 0 or 1 for ordered operands; 2 when either is a NaN.
///
/// # Safety
///
/// None; the bridge's signature is unsafe.
pub(super) unsafe fn long_double_compare(left: long_double_t, right: long_double_t) -> c_int {
    compare(left.soft(), right.soft()).map_or(2, |ordering| ordering as c_int)
}

/// Writes the value as a C99 hexadecimal floating constant, `0x1.8p+0` for
/// 1.5, and returns its length, truncated to fit `buffer_size` with its
/// terminator.
///
/// # Safety
///
/// `buffer` must be writable for `buffer_size` bytes.
pub(super) unsafe fn long_double_to_hex(
    value: long_double_t,
    buffer: *mut c_char,
    buffer_size: usize,
) -> usize {
    let value = value.soft();
    let mut text = HexText {
        bytes:  [0; LONG_DOUBLE_HEX_CAPACITY as usize],
        length: 0,
    };
    if value.negative {
        text.push(b"-");
    }
    match value.value {
        | Value::Nan { .. } | Value::Unsupported { .. } => text.push(b"nan"),
        | Value::Infinity => text.push(b"inf"),
        | Value::Zero => text.push(b"0x0p+0"),
        | Value::Finite {
            exponent,
            significand,
        } => {
            text.push(b"0x1");
            // The fraction below the integer bit, one hexadecimal digit at a
            // time until no bits remain.
            let mut fraction = significand << 1;
            if fraction != 0 {
                text.push(b".");
            }
            while fraction != 0 {
                text.push(&[b"0123456789abcdef"[(fraction >> 60) as usize]]);
                fraction <<= 4;
            }
            _ = write!(text, "p{exponent:+}");
        },
    }
    if buffer_size == 0 {
        return 0;
    }
    let length = text.length.min(buffer_size - 1);
    // SAFETY: The caller's buffer holds `buffer_size` bytes, more than
    // `length`, and does not overlap the local text.
    unsafe {
        std::ptr::copy_nonoverlapping(text.bytes.as_ptr().cast::<c_char>(), buffer, length);
    }
    // SAFETY: `length` is below `buffer_size`.
    let terminator = unsafe { buffer.add(length) };
    // SAFETY: The terminator lies inside the caller's writable buffer.
    unsafe {
        terminator.write(0);
    }
    length
}

/// The text of a hexadecimal constant, which always fits its capacity.
struct HexText {
    bytes:  [u8; LONG_DOUBLE_HEX_CAPACITY as usize],
    length: usize,
}

impl HexText {
    fn push(&mut self, bytes: &[u8]) {
        let end = (self.length + bytes.len()).min(self.bytes.len());
        self.bytes[self.length..end].copy_from_slice(&bytes[..end - self.length]);
        self.length = end;
    }
}

impl fmt::Write for HexText {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push(s.as_bytes());
        Ok(())
    }
}

/// `float _Complex` arithmetic.
///
/// # Safety
///
/// `real` and `imag` must be writable.
pub(super) unsafe fn bcc_complex_float(
    ar: f32,
    ai: f32,
    br: f32,
    bi: f32,
    operation: c_int,
    real: *mut f32,
    imag: *mut f32,
) {
    let soft = |value: f32| Soft::from_ieee(u64::from(value.to_bits()), F32);
    let [r, i] = complex::arithmetic([soft(ar), soft(ai)], [soft(br), soft(bi)], operation, F32);
    // SAFETY: The caller's outputs are writable.
    unsafe {
        real.write(f32::from_bits(r.to_ieee(F32) as u32));
    }
    // SAFETY: As above.
    unsafe {
        imag.write(f32::from_bits(i.to_ieee(F32) as u32));
    }
}

/// `double _Complex` arithmetic.
///
/// # Safety
///
/// `real` and `imag` must be writable.
pub(super) unsafe fn bcc_complex_double(
    ar: f64,
    ai: f64,
    br: f64,
    bi: f64,
    operation: c_int,
    real: *mut f64,
    imag: *mut f64,
) {
    let soft = |value: f64| Soft::from_ieee(value.to_bits(), F64);
    let [r, i] = complex::arithmetic([soft(ar), soft(ai)], [soft(br), soft(bi)], operation, F64);
    // SAFETY: The caller's outputs are writable.
    unsafe {
        real.write(f64::from_bits(r.to_ieee(F64)));
    }
    // SAFETY: As above.
    unsafe {
        imag.write(f64::from_bits(i.to_ieee(F64)));
    }
}

/// Complex arithmetic on `long double` carriers in the component precision:
/// the operands are rounded to binary32 or binary64 for precisions 1 and 2.
///
/// # Safety
///
/// `real` and `imag` must be writable.
#[expect(
    clippy::too_many_arguments,
    reason = "The bridge passes each part separately."
)]
pub(super) unsafe fn long_double_complex_arithmetic(
    ar: long_double_t,
    ai: long_double_t,
    br: long_double_t,
    bi: long_double_t,
    operation: c_int,
    precision: c_int,
    real: *mut long_double_t,
    imag: *mut long_double_t,
) {
    let format = match precision {
        | 1 => F32,
        | 2 => F64,
        | _ => X87,
    };
    // An x87 operand is loaded as it is; the others are rounded to their
    // component type first.
    let part = |value: long_double_t| {
        if format == X87 {
            value.soft()
        } else {
            value.soft().convert(format)
        }
    };
    let [r, i] = complex::arithmetic(
        [part(ar), part(ai)],
        [part(br), part(bi)],
        operation,
        format,
    );
    // SAFETY: The caller's outputs are writable.
    unsafe {
        real.write(long_double_t::store(r.convert(X87)));
    }
    // SAFETY: As above.
    unsafe {
        imag.write(long_double_t::store(i.convert(X87)));
    }
}

// Tests
#[cfg(test)]
#[expect(
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod tests;
