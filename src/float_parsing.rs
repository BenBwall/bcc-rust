use std::{
    ffi::c_char,
    fmt::{
        Display,
        Formatter,
    },
    io::Error as IoError,
    mem::ManuallyDrop,
    num::NonZeroI32,
};

use crate::{
    float_parsing::ffi::ERANGE,
    translation_phases::preprocessing::{
        FloatTokenType,
        PreprocessorExpressionOperand,
    },
};

mod ffi {
    #![allow(non_upper_case_globals)]
    #![allow(non_camel_case_types)]
    #![allow(non_snake_case)]
    #![allow(dead_code)]
    #![allow(clippy::all)]
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

/// Calls libc::errno() and returns the value.
fn errno() -> i32 {
    // `std::io::Error::last_os_error().raw_os_error()` is guaranteed to
    // return Some(i32).
    // We use ManuallyDrop here in order to avoid an unnecessary branch when calling
    // the destructor `std::io::Error`.
    ManuallyDrop::new(IoError::last_os_error())
        .raw_os_error()
        .unwrap()
}

extern "C" {
    fn strtod(s: *const c_char, endptr: *mut *mut c_char) -> f64;
    fn strtof(s: *const c_char, endptr: *mut *mut c_char) -> f32;
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) struct LongDouble {
    pub(crate) value: [u8; LONG_DOUBLE_BYTES],
}

const LONG_DOUBLE_BYTES: usize = ffi::LONG_DOUBLE_BYTES as _;

impl Display for LongDouble {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        type T = std::fmt::Error;

        let s = long_double_to_string(*self).expect("Converting long_double to string failed.");
        write!(f, "{s}")
    }
}

fn long_double_to_string_get_size(long_double: LongDouble) -> Result<usize, NonZeroI32> {
    let ld = ffi::long_double_t {
        bytes: long_double.value,
    };
    // SAFETY: long_double_to_string_get_size is safe to call with any initialized
    // input.
    let bytes = unsafe { ffi::long_double_to_string_get_size(ld) };
    match NonZeroI32::new(errno()) {
        | Some(error) => Err(error),
        | None => Ok(bytes),
    }
}

fn long_double_to_string(long_double: LongDouble) -> Result<String, NonZeroI32> {
    let bytes = long_double_to_string_get_size(long_double)?;
    let mut buffer = Vec::with_capacity(bytes);
    let capacity = buffer.capacity();
    let ld = ffi::long_double_t {
        bytes: long_double.value,
    };
    let ptr: *mut u8 = buffer.as_mut_ptr();
    // SAFETY: long_double_to_string is safe to call because we're passing a pointer
    // to a valid buffer, and we're also passing the capacity of the buffer.
    let bytes_written = unsafe { ffi::long_double_to_string(ld, ptr.cast::<c_char>(), capacity) };
    if let Some(error) = NonZeroI32::new(errno()) {
        return Err(error);
    }
    // SAFETY: We trust the C function to have written the correct number of bytes.
    unsafe {
        buffer.set_len(bytes_written);
    }
    // SAFETY: We trust the C function to have only written valid ASCII bytes into
    // the buffer.
    Ok(unsafe { String::from_utf8_unchecked(buffer) })
}

pub(crate) fn long_double_to_operand(
    long_double: LongDouble,
) -> Result<PreprocessorExpressionOperand, NonZeroI32> {
    let mut error = 0;
    let ld = ffi::long_double_t {
        bytes: long_double.value,
    };
    // SAFETY: long_double_to_operand is safe to call because we're  passing a
    // pointer to a valid error variable.
    let operand = unsafe { ffi::long_double_to_operand(ld, &mut error) };
    if let Some(error) = NonZeroI32::new(error) {
        return Err(error);
    }
    if operand.is_unsigned {
        // SAFETY: We know that the operand is unsigned because the C function told us
        // so. We also know that the operand is initialized because it was returned by a
        // C function.
        unsafe {
            Ok(PreprocessorExpressionOperand::Unsigned(
                operand.value.unsigned_value,
            ))
        }
    } else {
        // SAFETY: We know that the operand is signed because the C function told us so.
        // We also know that the operand is initialized because it was returned by a C
        // function.
        unsafe {
            Ok(PreprocessorExpressionOperand::Signed(
                operand.value.signed_value,
            ))
        }
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum ParseFloatError {
    Overflow(FloatTokenType),
    Invalid(FloatTokenType),
}

pub(crate) fn string_to_long_double(s: &str) -> Result<LongDouble, ParseFloatError> {
    assert!(
        s.ends_with('\0'),
        "string_to_long_double: string must end with null byte. Was: {s:?}"
    );
    let mut endptr = std::ptr::null_mut();
    // SAFETY: string_to_long_double is safe to call because our string is
    // null-terminated, and we're also a pointer to a null pointer, which is
    // what you're supposed to do.
    let long_double =
        unsafe { ffi::string_to_long_double(s.as_ptr().cast::<c_char>(), &mut endptr) };
    let error = errno();
    // SAFETY: Pointer arithmetic is safe because we know that the string contains
    // at least one byte (the null terminator), so adding a len - 1 is guaranteed to
    // be in bounds.
    unsafe {
        if endptr.cast_const().cast() != s.as_ptr().add(s.len() - 2) {
            return Err(ParseFloatError::Invalid(FloatTokenType::LongDouble(
                LongDouble {
                    value: [0; LONG_DOUBLE_BYTES],
                },
            )));
        }
    }
    let ret = LongDouble {
        // SAFETY: We know long_double is initialized because it was returned by a C function.
        value: unsafe { long_double.bytes },
    };
    if error == ERANGE as i32 {
        return Err(ParseFloatError::Overflow(FloatTokenType::LongDouble(ret)));
    }

    Ok(ret)
}

pub(crate) fn string_to_double(s: &str) -> Result<f64, ParseFloatError> {
    assert!(
        s.ends_with('\0'),
        "string_to_double: string must end with null byte. Was: {s:?}"
    );
    let mut endptr = std::ptr::null_mut();
    // SAFETY: strtod is safe to call because our string is null-terminated, and
    // we're also a pointer to a null pointer, which is what you're supposed to do.
    let double = unsafe { strtod(s.as_ptr().cast::<c_char>(), &mut endptr) };
    let error = errno();
    // SAFETY: Pointer arithmetic is safe because we know that the string contains
    // at least one byte (the null terminator), so adding a len - 1 is guaranteed to
    // be in bounds.
    unsafe {
        if endptr.cast_const().cast() != s.as_ptr().add(s.len() - 1) {
            return Err(ParseFloatError::Invalid(FloatTokenType::Double(0.0)));
        }
    }
    if error == ERANGE as i32 {
        return Err(ParseFloatError::Overflow(FloatTokenType::Double(double)));
    }

    Ok(double)
}

pub(crate) fn string_to_float(s: &str) -> Result<f32, ParseFloatError> {
    assert!(
        s.ends_with('\0'),
        "string_to_float: string must end with null byte. Was: {s:?}"
    );
    let mut endptr = std::ptr::null_mut();
    // SAFETY: strtof is safe to call because our string is null-terminated, and
    // we're also a pointer to a null pointer, which is what you're supposed to do.
    let float = unsafe { strtof(s.as_ptr().cast::<c_char>(), &mut endptr) };
    let error = errno();
    // SAFETY: Pointer arithmetic is safe because we know that the string contains
    // at least one byte (the null terminator), so adding a len - 1 is guaranteed to
    // be in bounds.
    unsafe {
        if endptr.cast_const().cast() != s.as_ptr().add(s.len() - 2) {
            return Err(ParseFloatError::Invalid(FloatTokenType::Float(0.0)));
        }
    }
    if error == ERANGE as i32 {
        return Err(ParseFloatError::Overflow(FloatTokenType::Float(float)));
    }
    Ok(float)
}
