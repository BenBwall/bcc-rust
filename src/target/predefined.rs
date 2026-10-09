//! Reserved target-description macros for the x86-64 System V target.
//! C99: implementation-defined limits §5.2.4.2, pp. 21-27; PDF pp. 33-39.
//! Floating spellings are the exact LLVM 23 Clang Linux-target spellings.

use std::fmt::Write;

use super::{
    Scalar,
    TargetLayout,
};
use crate::util::bump::{
    ArenaString,
    Bump,
};

impl Scalar {
    fn macro_type(self) -> &'static str {
        match self {
            | Self::Long => "long int",
            | Self::UnsignedLong => "long unsigned int",
            | _ => self.spelling(),
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            | Self::Long => "L",
            | Self::UnsignedLong => "UL",
            | Self::LongLong => "LL",
            | Self::UnsignedLongLong => "ULL",
            | Self::UnsignedInt => "U",
            | _ => "",
        }
    }
}

impl TargetLayout {
    /// Preprocessing source is built in the TU arena and read before user
    /// input. Reserved implementation names are available in every language
    /// mode. C99: §7.1.3p1, p. 166; PDF p. 178; limits §5.2.4.2 and §7.18,
    /// pp. 21-27, 255-261; PDF pp. 33-39, 267-273.
    pub(crate) fn predefined_macros(self, arena: &Bump) -> &str {
        use Scalar as S;
        let mut out = ArenaString::new_in(arena);
        writeln!(out, "#define __CHAR_BIT__ {}", self.char_bit).unwrap();
        for (name, scalar) in [
            ("SHORT", S::Short),
            ("INT", S::Int),
            ("LONG", S::Long),
            ("LONG_LONG", S::LongLong),
            ("FLOAT", S::Float),
            ("DOUBLE", S::Double),
            ("LONG_DOUBLE", S::LongDouble),
            ("SIZE_T", self.size_t),
            ("PTRDIFF_T", self.ptrdiff_t),
            ("WCHAR_T", self.wchar_t),
            ("WINT_T", self.wint_t),
        ] {
            writeln!(
                out,
                "#define __SIZEOF_{name}__ {}",
                self.scalar(scalar).unwrap().size
            )
            .unwrap();
        }
        writeln!(out, "#define __SIZEOF_POINTER__ {}", self.pointer.size).unwrap();
        for (name, scalar) in [
            ("SCHAR", S::SignedChar),
            ("SHRT", S::Short),
            ("INT", S::Int),
            ("LONG", S::Long),
            ("LONG_LONG", S::LongLong),
            ("SIZE", self.size_t),
            ("PTRDIFF", self.ptrdiff_t),
            ("WCHAR", self.wchar_t),
            ("WINT", self.wint_t),
            ("INTMAX", S::Long),
            ("UINTMAX", S::UnsignedLong),
            ("INTPTR", self.ptrdiff_t),
            ("UINTPTR", self.size_t),
            ("SIG_ATOMIC", S::Int),
        ] {
            self.integer_macros(&mut out, name, scalar);
        }
        for (bits, signed, unsigned) in [
            (8, S::SignedChar, S::UnsignedChar),
            (16, S::Short, S::UnsignedShort),
            (32, S::Int, S::UnsignedInt),
            (64, S::Long, S::UnsignedLong),
        ] {
            for (prefix, scalar) in [("INT", signed), ("UINT", unsigned)] {
                for middle in ["", "_LEAST", "_FAST"] {
                    let mut name = ArenaString::new_in(arena);
                    write!(name, "{prefix}{middle}{bits}").unwrap();
                    self.integer_macros(&mut out, &name, scalar);
                }
                Self::constant_macro(&mut out, &format_args!("{prefix}{bits}"), scalar);
            }
        }
        Self::constant_macro(&mut out, &"INTMAX", S::Long);
        Self::constant_macro(&mut out, &"UINTMAX", S::UnsignedLong);
        out.push_str(
            "#define __WCHAR_MIN__ (-__WCHAR_MAX__ - 1)\n#define __WINT_MIN__ 0U\n#define \
             __SIG_ATOMIC_MIN__ (-__SIG_ATOMIC_MAX__ - 1)\n",
        );
        out.push_str(
            "#define __ORDER_LITTLE_ENDIAN__ 1234\n#define __ORDER_BIG_ENDIAN__ 4321\n#define \
             __ORDER_PDP_ENDIAN__ 3412\n#define __BYTE_ORDER__ __ORDER_LITTLE_ENDIAN__\n#define \
             __LITTLE_ENDIAN__ 1\n#define __LP64__ 1\n#define _LP64 1\n#define __x86_64__ \
             1\n#define __x86_64 1\n",
        );
        out.push_str(FLOAT_MACROS);
        out.into_str()
    }

    fn integer_macros(self, out: &mut ArenaString<'_>, name: &str, scalar: Scalar) {
        let (bits, signed) = self.integer(scalar).unwrap();
        let max = if signed {
            (1_u64 << (bits - 1)) - 1
        } else {
            u64::MAX >> (64 - bits)
        };
        writeln!(out, "#define __{name}_MAX__ {max}{}", scalar.suffix()).unwrap();
        // Fundamental-type maxima have no corresponding Clang type macro.
        if !matches!(name, "SCHAR" | "SHRT" | "INT" | "LONG" | "LONG_LONG") {
            writeln!(out, "#define __{name}_TYPE__ {}", scalar.macro_type()).unwrap();
        }
    }

    fn constant_macro(out: &mut ArenaString<'_>, name: &impl std::fmt::Display, scalar: Scalar) {
        let suffix = scalar.suffix();
        writeln!(out, "#define __{name}_C_SUFFIX__ {suffix}").unwrap();
        writeln!(
            out,
            "#define __{name}_C(c) c{}{suffix}",
            if suffix.is_empty() { "" } else { "##" }
        )
        .unwrap();
    }
}

/// IEEE binary32/binary64 and x87 extended precision target representations.
/// C99: §5.2.4.2.2, pp. 23-27; PDF pp. 35-39.
const FLOAT_MACROS: &str = include_str!("floating-macros.h");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_definition_matches_linux_clang_dm_exactly() {
        let arena = Bump::new();
        let actual = TargetLayout::LP64.predefined_macros(&arena);
        let expected = include_str!("../../tests/fixtures/freestanding/target-macros.h");
        assert_eq!(actual.lines().count(), expected.lines().count());
        for line in actual.lines() {
            assert!(
                expected
                    .lines()
                    .any(|expected| expected.trim_end() == line.trim_end()),
                "{line}"
            );
        }
    }
}
