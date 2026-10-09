//! Target scalar representations shared by token conversion and semantic
//! analysis. C99: implementation-defined data model §6.2.5, pp. 33-37; PDF pp.
//! 45-49. The default is x86-64 System V LP64, independent of the compiler host
//! ABI.

mod predefined;

/// Distinct fundamental types, even when representation is identical.
/// C99: §6.2.5, pp. 33-37; PDF pp. 45-49.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Scalar {
    Void,
    Bool,
    Char,
    SignedChar,
    UnsignedChar,
    Short,
    UnsignedShort,
    Int,
    UnsignedInt,
    Long,
    UnsignedLong,
    LongLong,
    UnsignedLongLong,
    Float,
    Double,
    LongDouble,
    ComplexFloat,
    ComplexDouble,
    ComplexLongDouble,
}

impl Scalar {
    pub(crate) fn spelling(self) -> &'static str {
        match self {
            | Self::Void => "void",
            | Self::Bool => "_Bool",
            | Self::Char => "char",
            | Self::SignedChar => "signed char",
            | Self::UnsignedChar => "unsigned char",
            | Self::Short => "short",
            | Self::UnsignedShort => "unsigned short",
            | Self::Int => "int",
            | Self::UnsignedInt => "unsigned int",
            | Self::Long => "long",
            | Self::UnsignedLong => "unsigned long",
            | Self::LongLong => "long long",
            | Self::UnsignedLongLong => "unsigned long long",
            | Self::Float => "float",
            | Self::Double => "double",
            | Self::LongDouble => "long double",
            | Self::ComplexFloat => "float _Complex",
            | Self::ComplexDouble => "double _Complex",
            | Self::ComplexLongDouble => "long double _Complex",
        }
    }

    /// C99: §6.2.5p17, p. 35; PDF p. 47.
    pub(crate) fn integer(self) -> bool {
        matches!(
            self,
            Self::Bool
                | Self::Char
                | Self::SignedChar
                | Self::UnsignedChar
                | Self::Short
                | Self::UnsignedShort
                | Self::Int
                | Self::UnsignedInt
                | Self::Long
                | Self::UnsignedLong
                | Self::LongLong
                | Self::UnsignedLongLong
        )
    }
}

/// Size and alignment in target bytes, independent of the host Rust ABI.
/// C99: §6.5.3.4, pp. 80-81; PDF pp. 92-93.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Layout {
    pub(crate) size:  u64,
    pub(crate) align: u64,
}

/// The explicit x86-64 System V LP64 data model. Future targets replace this
/// value. C99: implementation-defined representations §6.2.5, pp. 33-37; PDF
/// pp. 45-49.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TargetLayout {
    pub(crate) char_bit:    u32,
    pub(crate) mb_len_max:  u32,
    pub(crate) wint_t:      Scalar,
    pub(crate) va_list:     Layout,
    pub(crate) scalars:     [Option<Layout>; 19],
    pub(crate) pointer:     Layout,
    pub(crate) char_signed: bool,
    pub(crate) size_t:      Scalar,
    pub(crate) ptrdiff_t:   Scalar,
    pub(crate) wchar_t:     Scalar,
}

impl TargetLayout {
    pub(crate) const LP64: Self = Self {
        char_bit:    8,
        mb_len_max:  1,
        wint_t:      Scalar::UnsignedInt,
        va_list:     Layout {
            size:  24,
            align: 8,
        },
        scalars:     [
            None,
            Some(Layout { size: 1, align: 1 }),
            Some(Layout { size: 1, align: 1 }),
            Some(Layout { size: 1, align: 1 }),
            Some(Layout { size: 1, align: 1 }),
            Some(Layout { size: 2, align: 2 }),
            Some(Layout { size: 2, align: 2 }),
            Some(Layout { size: 4, align: 4 }),
            Some(Layout { size: 4, align: 4 }),
            Some(Layout { size: 8, align: 8 }),
            Some(Layout { size: 8, align: 8 }),
            Some(Layout { size: 8, align: 8 }),
            Some(Layout { size: 8, align: 8 }),
            Some(Layout { size: 4, align: 4 }),
            Some(Layout { size: 8, align: 8 }),
            Some(Layout {
                size:  16,
                align: 16,
            }),
            Some(Layout { size: 8, align: 4 }),
            Some(Layout {
                size:  16,
                align: 8,
            }),
            Some(Layout {
                size:  32,
                align: 16,
            }),
        ],
        pointer:     Layout { size: 8, align: 8 },
        char_signed: true,
        size_t:      Scalar::UnsignedLong,
        ptrdiff_t:   Scalar::Long,
        wchar_t:     Scalar::Int,
    };

    pub(crate) fn scalar(self, scalar: Scalar) -> Option<Layout> {
        self.scalars[scalar as usize]
    }

    /// C99: §6.2.5p2-9, pp. 33-34; PDF pp. 45-46.
    pub(crate) fn integer(self, scalar: Scalar) -> Option<(u32, bool)> {
        if !scalar.integer() {
            return None;
        }
        let signed = match scalar {
            | Scalar::Char => self.char_signed,
            | Scalar::Bool
            | Scalar::UnsignedChar
            | Scalar::UnsignedShort
            | Scalar::UnsignedInt
            | Scalar::UnsignedLong
            | Scalar::UnsignedLongLong => false,
            | _ => true,
        };
        Some((
            u32::try_from(self.scalar(scalar)?.size * u64::from(self.char_bit)).ok()?,
            signed,
        ))
    }
}
