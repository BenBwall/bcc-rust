//! How LLVM IR text spells types, symbol names and constants.

use std::fmt::{
    self,
    Write as _,
};

use crate::ir::Type;

/// The LLVM type of an IR type: `f80` is `x86_fp80` and `f128` is `fp128`.
pub(super) const fn type_name(ty: Type) -> &'static str {
    match ty {
        | Type::I1 => "i1",
        | Type::I8 => "i8",
        | Type::I16 => "i16",
        | Type::I32 => "i32",
        | Type::I64 => "i64",
        | Type::I128 => "i128",
        | Type::F32 => "float",
        | Type::F64 => "double",
        | Type::F80 => "x86_fp80",
        | Type::F128 => "fp128",
        | Type::Ptr => "ptr",
    }
}

/// `@name`, quoted as `@"..."` with `\XX` escapes unless every character is
/// one LLVM allows in a bare name (`[-a-zA-Z$._][-a-zA-Z$._0-9]*`).
pub(super) fn symbol(name: &str) -> impl fmt::Display + '_ {
    fmt::from_fn(move |f| {
        let bare = |byte: u8| byte.is_ascii_alphanumeric() || b"-$._".contains(&byte);
        let plain = name.bytes().all(bare)
            && name
                .bytes()
                .next()
                .is_some_and(|first| !first.is_ascii_digit());
        if plain {
            return write!(f, "@{name}");
        }
        f.write_str("@\"")?;
        for byte in name.bytes() {
            escaped_byte(f, byte)?;
        }
        f.write_char('"')
    })
}

/// A byte array constant, `c"..."`, escaping all but printable ASCII.
pub(super) fn byte_string(bytes: &[u8]) -> impl fmt::Display + '_ {
    fmt::from_fn(move |f| {
        f.write_str("c\"")?;
        for &byte in bytes {
            escaped_byte(f, byte)?;
        }
        f.write_char('"')
    })
}

fn escaped_byte(f: &mut fmt::Formatter<'_>, byte: u8) -> fmt::Result {
    if (b' '..=b'~').contains(&byte) && byte != b'"' && byte != b'\\' {
        f.write_char(char::from(byte))
    } else {
        write!(f, "\\{byte:02X}")
    }
}

/// An integer constant of `ty` from its low bits: `true` or `false` for
/// `i1`, else the signed decimal value, which LLVM reads for every width.
pub(super) fn int(bits: u128, ty: Type) -> impl fmt::Display {
    fmt::from_fn(move |f| {
        if ty == Type::I1 {
            return f.write_str(if bits & 1 == 0 { "false" } else { "true" });
        }
        let shift = 128 - ty.bits();
        write!(f, "{}", ((bits << shift) as i128) >> shift)
    })
}

/// A floating constant of `ty` from its bits, in LLVM's exact hexadecimal
/// forms: `0x` and the 16 digits of a `double` (also for `float`, whose
/// value LLVM reads as a `double`), `0xK` and 20 digits for `x86_fp80`, and
/// `0xL` with the low 64 bits before the high 64 for `fp128`.
pub(super) fn float(bits: u128, ty: Type) -> impl fmt::Display {
    fmt::from_fn(move |f| {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "The halves are taken apart deliberately."
        )]
        let (low, high) = (bits as u64, (bits >> 64) as u64);
        match ty {
            | Type::F32 => write!(f, "0x{:016X}", float_as_double_bits(low)),
            | Type::F80 => write!(f, "0xK{:04X}{low:016X}", high & 0xFFFF),
            | Type::F128 => write!(f, "0xL{low:016X}{high:016X}"),
            | _ => write!(f, "0x{low:016X}"),
        }
    })
}

/// The `double` bits of the same value as the `float` bits. A NaN keeps its
/// sign and payload, shifted to the top of the wider significand, which is
/// how LLVM narrows it back.
fn float_as_double_bits(bits: u64) -> u64 {
    if (bits >> 23) & 0xFF == 0xFF {
        let sign = (bits >> 31 & 1) << 63;
        let significand = (bits & 0x7F_FFFF) << 29;
        sign | (0x7FF << 52) | significand
    } else {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "A float's bits are the low 32."
        )]
        let float = f32::from_bits(bits as u32);
        f64::from(float).to_bits()
    }
}
