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
    let bytes_written =
        unsafe { ffi::long_double_to_string(ld, ptr.cast::<c_char>(), capacity) };
    if let Some(error) = NonZeroI32::new(errno()) {
        return Err(error);
    }
    unsafe {
        buffer.set_len(bytes_written);
    }
    Ok(unsafe { String::from_utf8_unchecked(buffer) })
}

pub(crate) fn long_double_to_operand(
    long_double: LongDouble,
) -> Result<PreprocessorExpressionOperand, NonZeroI32> {
    let mut error = 0;
    let ld = ffi::long_double_t {
        bytes: long_double.value,
    };
    let operand = unsafe { ffi::long_double_to_operand(ld, &mut error) };
    if let Some(error) = NonZeroI32::new(error) {
        return Err(error);
    }
    if operand.is_unsigned {
        unsafe {
            Ok(PreprocessorExpressionOperand::Unsigned(
                operand.value.unsigned_value,
            ))
        }
    } else {
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
    let long_double =
        unsafe { ffi::string_to_long_double(s.as_ptr().cast::<c_char>(), &mut endptr) };
    let error = errno();
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
    let double =
        unsafe { strtod(s.as_ptr().cast::<c_char>(), &mut endptr) };
    let error = errno();
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
    let float =
        unsafe { strtof(s.as_ptr().cast::<c_char>(), &mut endptr) };
    let error = errno();
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
