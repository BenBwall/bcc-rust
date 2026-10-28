//! Runtime values (WebAssembly 2.0 §4.2.1). Floats are kept as their bits,
//! so a NaN's payload survives loads, stores and reinterpretation exactly.

use std::fmt;

use crate::module::ValType;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value {
    I32(i32),
    I64(i64),
    F32(u32),
    F64(u64),
    /// A function reference: a function index, or null.
    FuncRef(Option<u32>),
    /// A host reference; the stepper's host never makes one, so these are
    /// null unless a table starts with them.
    ExternRef(Option<u32>),
}

impl Value {
    /// The zero (or null) value of `ty`, which locals start with.
    pub fn default_of(ty: ValType) -> Self {
        match ty {
            | ValType::I32 => Self::I32(0),
            | ValType::I64 => Self::I64(0),
            | ValType::F32 => Self::F32(0),
            | ValType::F64 => Self::F64(0),
            | ValType::FuncRef => Self::FuncRef(None),
            | ValType::ExternRef => Self::ExternRef(None),
        }
    }

    pub fn ty(self) -> ValType {
        match self {
            | Self::I32(_) => ValType::I32,
            | Self::I64(_) => ValType::I64,
            | Self::F32(_) => ValType::F32,
            | Self::F64(_) => ValType::F64,
            | Self::FuncRef(_) => ValType::FuncRef,
            | Self::ExternRef(_) => ValType::ExternRef,
        }
    }

    pub fn f32(value: f32) -> Self {
        Self::F32(value.to_bits())
    }

    pub fn f64(value: f64) -> Self {
        Self::F64(value.to_bits())
    }
}

/// Values as the text format writes constants: integers in decimal, floats
/// in the shortest decimal that reads back exactly, `nan`, `inf` and `nan:0x…`
/// for a non-canonical NaN.
impl fmt::Display for Value {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            | Self::I32(value) => write!(formatter, "{value}"),
            | Self::I64(value) => write!(formatter, "{value}"),
            | Self::F32(bits) => {
                let value = f32::from_bits(bits);
                let text = format!("{value:?}");
                float(
                    formatter,
                    &text,
                    bits >> 31 == 1,
                    u64::from(bits & 0x007F_FFFF),
                    0x0040_0000,
                )
            },
            | Self::F64(bits) => {
                let value = f64::from_bits(bits);
                let text = format!("{value:?}");
                float(
                    formatter,
                    &text,
                    bits >> 63 == 1,
                    bits & 0x000F_FFFF_FFFF_FFFF,
                    0x0008_0000_0000_0000,
                )
            },
            | Self::FuncRef(None) | Self::ExternRef(None) => formatter.write_str("null"),
            | Self::FuncRef(Some(index)) => write!(formatter, "func {index}"),
            | Self::ExternRef(Some(index)) => write!(formatter, "extern {index}"),
        }
    }
}

/// `text` is Rust's rendering of the float: `NaN`, `inf`, `-inf` or a
/// shortest round-trip decimal.
fn float(
    formatter: &mut fmt::Formatter<'_>,
    text: &str,
    negative: bool,
    payload: u64,
    canonical: u64,
) -> fmt::Result {
    let sign = if negative { "-" } else { "" };
    if text == "NaN" {
        if payload == canonical {
            write!(formatter, "{sign}nan")
        } else {
            write!(formatter, "{sign}nan:{payload:#x}")
        }
    } else {
        formatter.write_str(text)
    }
}

#[cfg(test)]
mod tests {
    use super::Value;

    #[test]
    fn displays_values_as_the_text_format_writes_them() {
        assert_eq!(Value::I32(-7).to_string(), "-7");
        assert_eq!(Value::f32(1.5).to_string(), "1.5");
        assert_eq!(Value::f32(1.0).to_string(), "1.0");
        assert_eq!(Value::f64(0.1).to_string(), "0.1");
        assert_eq!(Value::f64(f64::NEG_INFINITY).to_string(), "-inf");
        assert_eq!(Value::F32(0x7FC0_0000).to_string(), "nan");
        assert_eq!(Value::F32(0xFFC0_0000).to_string(), "-nan");
        assert_eq!(Value::F32(0x7F80_0001).to_string(), "nan:0x1");
        assert_eq!(Value::FuncRef(None).to_string(), "null");
    }
}
