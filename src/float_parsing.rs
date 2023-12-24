use std::{
    ffi::c_char,
    fmt::{
        Display,
        Formatter,
    },
};

use crate::{
    float_parsing::ffi::ERANGE,
    translation_phases::phase_4_preprocessing::FloatTokenType,
};

mod ffi {
    #![allow(non_upper_case_globals)]
    #![allow(non_camel_case_types)]
    #![allow(non_snake_case)]
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) struct LongDouble {
    pub(crate) value: [u8; 16],
}

impl Display for LongDouble {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let s = long_double_to_string(*self).unwrap();
        write!(f, "{s}")
    }
}

fn long_double_to_string_get_size(long_double: LongDouble) -> Result<usize, i32> {
    let ld = ffi::long_double_t {
        bytes: long_double.value,
    };
    let mut error = 0;
    let bytes = unsafe { ffi::long_double_to_string_get_size(ld, &mut error) };
    if error == 0 {
        Ok(bytes)
    } else {
        Err(error)
    }
}

fn long_double_to_string(long_double: LongDouble) -> Result<String, i32> {
    let bytes = long_double_to_string_get_size(long_double)?;
    let mut buffer = Vec::with_capacity(bytes);
    let capacity = buffer.capacity();
    let ld = ffi::long_double_t {
        bytes: long_double.value,
    };
    let mut error = 0;
    let ptr: *mut u8 = buffer.as_mut_ptr();
    let bytes_written =
        unsafe { ffi::long_double_to_string(ld, ptr.cast::<c_char>(), capacity, &mut error) };
    if error != 0 {
        return Err(error);
    }
    unsafe {
        buffer.set_len(bytes_written);
    }
    Ok(unsafe { String::from_utf8_unchecked(buffer) })
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum ParseFloatError {
    Overflow(FloatTokenType),
    Invalid,
}

pub(crate) fn string_to_long_double(s: &str) -> Result<LongDouble, ParseFloatError> {
    assert!(
        s.ends_with('\0'),
        "string_to_long_double: string must end with null byte. Was: {s:?}"
    );
    let mut endptr = std::ptr::null_mut();
    let mut error = 0;
    let long_double =
        unsafe { ffi::string_to_long_double(s.as_ptr().cast::<c_char>(), &mut endptr, &mut error) };

    unsafe {
        if endptr.cast_const().cast() != s.as_ptr().add(s.len() - 2) {
            return Err(ParseFloatError::Invalid);
        }
    }
    let ret = LongDouble {
        value: long_double.bytes,
    };
    if error == ERANGE as i32 {
        return Err(ParseFloatError::Overflow(FloatTokenType::LongDouble(ret)));
    }

    Ok(LongDouble {
        value: long_double.bytes,
    })
}

pub(crate) fn string_to_double(s: &str) -> Result<f64, ParseFloatError> {
    assert!(
        s.ends_with('\0'),
        "string_to_double: string must end with null byte. Was: {s:?}"
    );
    let mut endptr = std::ptr::null_mut();
    let mut error = 0;
    let double =
        unsafe { ffi::string_to_double(s.as_ptr().cast::<c_char>(), &mut endptr, &mut error) };
    unsafe {
        if endptr.cast_const().cast() != s.as_ptr().add(s.len() - 1) {
            return Err(ParseFloatError::Invalid);
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
    let mut error = 0;
    let float =
        unsafe { ffi::string_to_float(s.as_ptr().cast::<c_char>(), &mut endptr, &mut error) };
    unsafe {
        if endptr.cast_const().cast() != s.as_ptr().add(s.len() - 2) {
            return Err(ParseFloatError::Invalid);
        }
    }
    if error == ERANGE as i32 {
        return Err(ParseFloatError::Overflow(FloatTokenType::Float(float)));
    }
    Ok(float)
}
