//! Scalar types and target layouts describe C representations independently
//! of the host Rust data model.

/// Scalar layout and implementation-defined ABI choices. The System V base
/// is shared by the Linux targets. C99: representations §6.2.5, pp. 33-37; PDF
/// pp. 45-49.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TargetLayout {
    pub(crate) fixed_enum_type:    Option<Scalar>,
    pub(crate) va_list_is_pointer: bool,
    pub(crate) ms_bitfields:       bool,
    pub(crate) intmax_t:           Scalar,
    pub(crate) uintmax_t:          Scalar,
    pub(crate) char_bit:           u32,
    pub(crate) mb_len_max:         u32,
    pub(crate) wint_t:             Scalar,
    pub(crate) va_list:            Layout,
    pub(crate) scalars:            [Option<Layout>; 24],
    pub(crate) pointer:            Layout,
    pub(crate) char_signed:        bool,
    pub(crate) size_t:             Scalar,
    pub(crate) ptrdiff_t:          Scalar,
    pub(crate) wchar_t:            Scalar,
    /// Whether `long double` is IEEE binary128 rather than what its layout
    /// implies; see [`TargetLayout::long_double_is_binary128`].
    pub(crate) long_double_format: LongDoubleFormat,
}

impl TargetLayout {
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
            | Scalar::UnsignedLongLong
            | Scalar::UnsignedInt128 => false,
            | _ => true,
        };
        Some((
            u32::try_from(self.scalar(scalar)?.size * u64::from(self.char_bit)).ok()?,
            signed,
        ))
    }

    pub(crate) fn scalar(self, scalar: Scalar) -> Option<Layout> {
        self.scalars[scalar.representation() as usize]
    }

    /// C99: implementation-defined wide encoding §6.4.5p5, pp. 62-63;
    /// PDF pp. 74-75. The Windows targets use UTF-16 code units.
    pub(crate) fn wide_utf16(self) -> bool {
        self.wchar_t == Scalar::UnsignedShort
    }

    /// C99: implementation-defined floating representations §6.2.5p10,
    /// p. 34; PDF p. 46. MSVC uses binary64 for both distinct types.
    pub(crate) fn long_double_is_double(self) -> bool {
        self.scalar(Scalar::LongDouble) == self.scalar(Scalar::Double)
    }

    /// C99: implementation-defined floating representations §6.2.5p10,
    /// p. 34; PDF p. 46. The WebAssembly targets use IEEE binary128 for
    /// `long double`, as Clang does there.
    pub(crate) fn long_double_is_binary128(self) -> bool {
        self.long_double_format == LongDoubleFormat::Binary128
    }

    /// The class of `long double`'s set of values in
    /// [`Scalar::floating_rank`]: 2 for binary64, 3 for the x87 format and 4
    /// for binary128.
    /// C99: §6.3.1.8 paragraph 1, pp. 44-45; PDF pp. 56-57.
    pub(crate) fn long_double_class(self) -> u8 {
        if self.long_double_is_double() {
            2
        } else if self.long_double_is_binary128() {
            4
        } else {
            3
        }
    }

    /// Whether `long double` is the x87 80-bit extended format: neither
    /// binary64 (MSVC) nor binary128 (WebAssembly).
    /// C99: §6.2.5p10, p. 34; PDF p. 46.
    pub(crate) fn long_double_is_x87(self) -> bool {
        !self.long_double_is_double() && !self.long_double_is_binary128()
    }
}

impl TargetLayout {
    /// The WebAssembly `wasm32` data model, as Clang's
    /// `WebAssembly32TargetInfo` defines it: ILP32 with 64-bit `long long`,
    /// binary128 `long double` and `__int128` aligned to 16 bytes, a
    /// `char *` `va_list`, signed `char`, and `int` for `wchar_t` and
    /// `wint_t`.
    /// C99: representations §6.2.5, pp. 33-37; PDF pp. 45-49.
    pub(crate) const ILP32_WASM: Self = {
        let mut layout = Self::LP64;
        layout.va_list = Layout { size: 4, align: 4 };
        layout.va_list_is_pointer = true;
        layout.pointer = Layout { size: 4, align: 4 };
        layout.intmax_t = Scalar::LongLong;
        layout.uintmax_t = Scalar::UnsignedLongLong;
        layout.wint_t = Scalar::Int;
        layout.scalars[Scalar::Long as usize] = Some(Layout { size: 4, align: 4 });
        layout.scalars[Scalar::UnsignedLong as usize] = Some(Layout { size: 4, align: 4 });
        layout.long_double_format = LongDoubleFormat::Binary128;
        layout
    };
    pub(crate) const LP64: Self = Self {
        fixed_enum_type:    None,
        va_list_is_pointer: false,
        ms_bitfields:       false,
        intmax_t:           Scalar::Long,
        uintmax_t:          Scalar::UnsignedLong,
        char_bit:           8,
        mb_len_max:         4,
        wint_t:             Scalar::UnsignedInt,
        va_list:            Layout {
            size:  24,
            align: 8,
        },
        scalars:            [
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
            Some(Layout {
                size:  16,
                align: 16,
            }),
            Some(Layout {
                size:  16,
                align: 16,
            }),
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
            Some(Layout {
                size:  16,
                align: 16,
            }),
            Some(Layout {
                size:  32,
                align: 16,
            }),
            Some(Layout { size: 2, align: 2 }),
        ],
        pointer:            Layout { size: 8, align: 8 },
        char_signed:        true,
        size_t:             Scalar::UnsignedLong,
        ptrdiff_t:          Scalar::Long,
        wchar_t:            Scalar::Int,
        long_double_format: LongDoubleFormat::Layout,
    };
}

/// How `long double` is represented. C99 leaves it to the implementation
/// (§6.2.5 paragraph 10, p. 34; PDF p. 46).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LongDoubleFormat {
    /// As its layout implies: binary64 when it is `double`'s (MSVC), else
    /// the x87 80-bit format in 16 bytes.
    Layout,
    /// IEEE binary128 (WebAssembly).
    Binary128,
}

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
    /// GNU integer extension; rank exceeds long long.
    /// C99: §6.3.1.1p1, p. 42; PDF p. 54.
    Int128,
    UnsignedInt128,
    Float,
    Double,
    LongDouble,
    ComplexFloat,
    ComplexDouble,
    ComplexLongDouble,
    /// GCC Additional Floating Types: IEEE binary128, also spelled
    /// `_Float128`.
    Float128,
    ComplexFloat128,
    /// The interchange type `_Float16`, IEEE binary16. GCC and Clang compute
    /// with it in `float` on x86-64.
    /// C23: Annex H, §H.2.2 paragraph 1, p. 553; PDF p. 566. Extension to
    /// C99 §6.2.5 paragraph 10, p. 34; PDF p. 46.
    Float16,
    /// The interchange types `_Float32` and `_Float64` and the extended types
    /// `_Float32x` and `_Float64x`, with their complex types. Each is a
    /// distinct type with the representation of the standard type
    /// [`Self::representation`] names.
    /// C23: Annex H, §H.2.2 paragraph 3, p. 553; PDF p. 566; §H.2.4
    /// paragraph 2, p. 554; PDF p. 567. Extension to C99 §6.2.5 paragraph
    /// 10, p. 34; PDF p. 46.
    Float32,
    Float64,
    Float32x,
    Float64x,
    ComplexFloat32,
    ComplexFloat64,
    ComplexFloat32x,
    ComplexFloat64x,
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
            | Self::Int128 => "__int128",
            | Self::UnsignedInt128 => "unsigned __int128",
            | Self::Float => "float",
            | Self::Double => "double",
            | Self::LongDouble => "long double",
            | Self::ComplexFloat => "float _Complex",
            | Self::ComplexDouble => "double _Complex",
            | Self::ComplexLongDouble => "long double _Complex",
            | Self::Float128 => "__float128",
            | Self::ComplexFloat128 => "__float128 _Complex",
            | Self::Float16 => "_Float16",
            | Self::Float32 => "_Float32",
            | Self::Float64 => "_Float64",
            | Self::Float32x => "_Float32x",
            | Self::Float64x => "_Float64x",
            | Self::ComplexFloat32 => "_Float32 _Complex",
            | Self::ComplexFloat64 => "_Float64 _Complex",
            | Self::ComplexFloat32x => "_Float32x _Complex",
            | Self::ComplexFloat64x => "_Float64x _Complex",
        }
    }

    /// The standard or binary128 type whose representation this type
    /// shares: binary32 for `_Float32`, binary64 for `_Float64` and
    /// `_Float32x`, and `long double` for `_Float64x`, which needs a format
    /// wider than binary64 (the x87 format on the GNU targets). Every other
    /// type is its own representation. Layout, constant folding and
    /// lowering read the representation; type identity, compatibility and
    /// the usual arithmetic conversions read the type itself.
    /// C23: §H.2.2 paragraph 3, p. 553; PDF p. 566; §H.2.4 paragraphs 2
    /// and 4, p. 554; PDF p. 567.
    pub(crate) const fn representation(self) -> Self {
        match self {
            | Self::Float32 => Self::Float,
            | Self::Float64 | Self::Float32x => Self::Double,
            | Self::Float64x => Self::LongDouble,
            | Self::ComplexFloat32 => Self::ComplexFloat,
            | Self::ComplexFloat64 | Self::ComplexFloat32x => Self::ComplexDouble,
            | Self::ComplexFloat64x => Self::ComplexLongDouble,
            | _ => self,
        }
    }

    /// Whether this is a complex floating type.
    /// C99: §6.2.5 paragraph 11, p. 34; PDF p. 46.
    pub(crate) const fn complex(self) -> bool {
        matches!(
            self,
            Self::ComplexFloat
                | Self::ComplexDouble
                | Self::ComplexLongDouble
                | Self::ComplexFloat128
                | Self::ComplexFloat32
                | Self::ComplexFloat64
                | Self::ComplexFloat32x
                | Self::ComplexFloat64x
        )
    }

    /// The corresponding real type of a complex type; any other type is its
    /// own.
    /// C99: §6.2.5 paragraph 11, p. 34; PDF p. 46.
    pub(crate) const fn real_type(self) -> Self {
        match self {
            | Self::ComplexFloat => Self::Float,
            | Self::ComplexDouble => Self::Double,
            | Self::ComplexLongDouble => Self::LongDouble,
            | Self::ComplexFloat128 => Self::Float128,
            | Self::ComplexFloat32 => Self::Float32,
            | Self::ComplexFloat64 => Self::Float64,
            | Self::ComplexFloat32x => Self::Float32x,
            | Self::ComplexFloat64x => Self::Float64x,
            | _ => self,
        }
    }

    /// The complex type whose corresponding real type is this real floating
    /// type; any other type is its own.
    /// C99: §6.2.5 paragraph 11, p. 34; PDF p. 46.
    pub(crate) const fn complex_type(self) -> Self {
        match self {
            | Self::Float => Self::ComplexFloat,
            | Self::Double => Self::ComplexDouble,
            | Self::LongDouble => Self::ComplexLongDouble,
            | Self::Float128 => Self::ComplexFloat128,
            | Self::Float32 => Self::ComplexFloat32,
            | Self::Float64 => Self::ComplexFloat64,
            | Self::Float32x => Self::ComplexFloat32x,
            | Self::Float64x => Self::ComplexFloat64x,
            | _ => self,
        }
    }

    /// Whether this is a real or complex floating type.
    /// C99: §6.2.5 paragraphs 10-11, p. 34; PDF p. 46.
    pub(crate) const fn floating(self) -> bool {
        self.floating_rank(3) != 0
    }

    /// Orders the floating types for the usual arithmetic conversions; 0 for
    /// every other type. The set of values comes first: binary16, binary32,
    /// binary64, the x87 format, then binary128, each class holding the types
    /// with that representation, so `long double` joins the class
    /// `long_double_class` names: binary64 (2) on the MSVC target, x87 (3) on
    /// the other x86-64 targets, binary128 (4) on WebAssembly. Within a class
    /// of equal sets of values an interchange type is chosen, then
    /// `long double`, then `double` or `float`, then an extended type.
    /// C99: §6.3.1.8 paragraph 1, pp. 44-45; PDF pp. 56-57. C23: §H.4.3
    /// paragraph 1, pp. 558-559; PDF pp. 571-572.
    pub(crate) const fn floating_rank(self, long_double_class: u8) -> u8 {
        let (class, preference) = match self.real_type() {
            | Self::Float16 => (0, 3),
            | Self::Float => (1, 1),
            | Self::Float32 => (1, 3),
            | Self::Double => (2, 1),
            | Self::Float64 => (2, 3),
            | Self::Float32x => (2, 0),
            | Self::LongDouble => (long_double_class, 2),
            | Self::Float64x => (3, 0),
            | Self::Float128 => (4, 3),
            | _ => return 0,
        };
        class * 4 + preference
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
                | Self::Int128
                | Self::UnsignedInt128
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
