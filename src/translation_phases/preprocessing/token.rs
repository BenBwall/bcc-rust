//! Tokens produced for the parser.
//!
//! These are the tokens of translation phase 7: keywords, identifiers,
//! constants, string literals, and punctuators. C99: §5.1.1.2 paragraph 1
//! item 7, p. 10; PDF p. 22, and the `token` categories of §6.4 paragraphs
//! 1 and 3, p. 49; PDF p. 61 (also §A.1.1, p. 403; PDF p. 415).
//!
//! Types and values assume an LP64 target: `int` has 32 bits, `long` and
//! `long long` have 64, and `char` has 8. Those widths are
//! implementation-defined (§5.2.4.2.1 paragraph 1, pp. 21-22; PDF
//! pp. 33-34).

use std::fmt::{
    Debug,
    Display,
    Formatter,
    Result as FmtResult,
};

use crate::{
    configuration::{
        CStandard,
        CompilerConfiguration,
        Feature,
        FeatureOrigin,
    },
    diagnostics::quote_spelling,
    float_parsing::LongDouble,
    translation_phases::{
        Context,
        GetPosition,
        GetSourceVectors,
        SourcePosition,
        SourceVectors,
    },
    util::{
        packed::Packed,
        string_cache::StringCacheId,
    },
};

/// A signed type an integer-constant widening warning names.
///
/// C99: §6.4.4.1 paragraph 5, pp. 55-56; PDF pp. 67-68.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum SignedIntegerLiteralType {
    Int,
    Long,
}

/// An unsigned type an integer constant can have.
///
/// C99: §6.4.4.1 paragraph 5, pp. 55-56; PDF pp. 67-68.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
#[expect(
    clippy::enum_variant_names,
    reason = "We are repeating the word 'unsigned' a lot here, but I think it's clearer this way."
)]
pub(crate) enum UnsignedIntegerLiteralType {
    UnsignedInt,
    UnsignedLong,
    UnsignedLongLong,
}

/// An `integer-suffix`; `u` and `l` may come in either order and case,
/// but `ll` and `LL` may not mix cases.
///
/// C99: §6.4.4.1 paragraph 1, p. 55; PDF p. 67.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum IntegerSuffix {
    Unsigned,
    Long,
    LongLong,
    UnsignedLong,
    UnsignedLongLong,
}

/// A phase-7 `token`, with its spelling and provenance.
///
/// C99: §6.4 paragraph 1, p. 49; PDF p. 61.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct Token {
    pub(crate) kind:           TokenType,
    pub(crate) source_vectors: SourceVectors,
    pub(crate) contents:       StringCacheId,
}

impl GetPosition for Token {
    #[inline(always)]
    fn position(&self, context: &Context<'_>) -> SourcePosition {
        self.source_vectors.position(context)
    }
}

impl GetSourceVectors for Token {
    #[inline(always)]
    fn source_vectors(&self, _context: &mut Context<'_>) -> SourceVectors {
        self.source_vectors
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
/// 8-byte values are [`Packed`] so tokens and constants stay 4-byte aligned.
///
/// An `integer-constant` with the type its value and suffix give it.
///
/// C99: §6.4.4.1 paragraph 5, pp. 55-56; PDF pp. 67-68.
pub(crate) enum IntegerTokenType {
    Int(i32),
    Long(Packed<i64>),
    LongLong(Packed<i64>),
    UnsignedInt(u32),
    UnsignedLong(Packed<u64>),
    UnsignedLongLong(Packed<u64>),
    /// C23 bit-precise suffix: magnitude, minimum width, unsignedness.
    BitInt(Packed<u64>, u8, bool),
    /// GNU imaginary integer constant.
    Imaginary(Packed<u64>, ImaginaryIntegerKind),
}

/// GNU imaginary integer component types (extension to C99 §6.4.4.1).
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum ImaginaryIntegerKind {
    Int,
    Long,
    LongLong,
    UnsignedInt,
    UnsignedLong,
    UnsignedLongLong,
}
impl ImaginaryIntegerKind {
    pub(crate) fn type_name(self) -> &'static str {
        match self {
            | Self::Int => "int _Complex",
            | Self::Long => "long _Complex",
            | Self::LongLong => "long long _Complex",
            | Self::UnsignedInt => "unsigned int _Complex",
            | Self::UnsignedLong => "unsigned long _Complex",
            | Self::UnsignedLongLong => "unsigned long long _Complex",
        }
    }
}
impl From<IntegerTokenType> for i128 {
    fn from(v: IntegerTokenType) -> Self {
        match v {
            | IntegerTokenType::UnsignedLong(v)
            | IntegerTokenType::UnsignedLongLong(v)
            | IntegerTokenType::BitInt(v, _, _)
            | IntegerTokenType::Imaginary(v, _) => i128::from(v.get()),
            | IntegerTokenType::Long(v) | IntegerTokenType::LongLong(v) => i128::from(v.get()),
            | IntegerTokenType::UnsignedInt(v) => i128::from(v),
            | IntegerTokenType::Int(v) => i128::from(v),
        }
    }
}

/// A `floating-constant`: `double` unsuffixed, `float` with `f` or `F`, and
/// `long double` with `l` or `L`.
///
/// C99: §6.4.4.2 paragraph 4, p. 58; PDF p. 70.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum FloatTokenType {
    Float(f32),
    Double(Packed<f64>),
    LongDouble(LongDouble),
    /// GNU imaginary floating constants preserve their component precision.
    ImaginaryFloat(f32),
    ImaginaryDouble(Packed<f64>),
    ImaginaryLongDouble(LongDouble),
}

impl FloatTokenType {
    /// The C type named by this constant's suffix.
    pub(crate) fn type_name(&self) -> &'static str {
        match self {
            | Self::Float(_) => "float",
            | Self::Double(_) => "double",
            | Self::LongDouble(_) => "long double",
            | Self::ImaginaryFloat(_) => "float _Complex",
            | Self::ImaginaryDouble(_) => "double _Complex",
            | Self::ImaginaryLongDouble(_) => "long double _Complex",
        }
    }
}

impl Display for FloatTokenType {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            | Self::Float(v) => write!(f, "{v}"),
            | Self::ImaginaryFloat(v) => write!(f, "{v}i"),
            | Self::Double(v) => write!(f, "{v}"),
            | Self::ImaginaryDouble(v) => write!(f, "{v}i"),
            | Self::LongDouble(v) => write!(f, "{v}"),
            | Self::ImaginaryLongDouble(v) => write!(f, "{v}i"),
        }
    }
}

/// A `keyword`.
///
/// C99: §6.4.1 paragraph 1, p. 50; PDF p. 62.
/// C11: §6.4.1p1, p. 58; PDF p. 76. C23: §6.4.1p1, p. 53;
/// PDF p. 66. GNU/MSVC entries and C2y `_Countof` are extensions.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum KeywordTokenType {
    Auto,
    Break,
    Case,
    Char,
    Const,
    Continue,
    Default,
    Do,
    Double,
    Else,
    Enum,
    Extern,
    Float,
    For,
    Goto,
    If,
    Inline,
    Int,
    Long,
    Register,
    Restrict,
    Return,
    Short,
    Signed,
    Sizeof,
    Static,
    Struct,
    Switch,
    Typedef,
    Union,
    Unsigned,
    Void,
    Volatile,
    While,
    Bool,
    Complex,
    Imaginary,
    Alignas,
    Alignof,
    Atomic,
    Generic,
    Noreturn,
    StaticAssert,
    ThreadLocal,
    BitInt,
    Decimal32,
    Decimal64,
    Decimal128,
    Constexpr,
    True,
    False,
    Nullptr,
    Typeof,
    TypeofUnqual,
    Countof,
    Attribute,
    Asm,
    Extension,
    BuiltinVaArg,
    BuiltinOffsetof,
    BuiltinTypesCompatible,
    BuiltinChooseExpr,
    LocalLabel,
    Int128,
    AutoType,
    Real,
    Imag,
    Declspec,
    Int8,
    Int16,
    Int32,
    Int64,
    Cdecl,
    Stdcall,
    Fastcall,
    Vectorcall,
    Thiscall,
    Ptr32,
    Ptr64,
    Unaligned,
    W64,
    Sptr,
    Uptr,
    Forceinline,
    Try,
    Except,
    Finally,
    Leave,
    MsAsm,
    Pragma,
}

/// A `punctuator`. Digraphs map to the punctuators they behave as (§6.4.6
/// paragraph 3, p. 64; PDF p. 76). `#` and `##` are absent: no phase-7
/// grammar uses them, so they never become tokens.
///
/// C99: §6.4.6 paragraph 1, p. 63; PDF p. 75.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum OperatorTokenType {
    Plus,
    Minus,
    Asterisk,
    ForwardSlash,
    Percent,
    LessThanLessThan,
    GreaterThanGreaterThan,
    LessThan,
    LessThanEquals,
    GreaterThan,
    GreaterThanEquals,
    EqualsEquals,
    ExclamationMarkEquals,
    Ampersand,
    Caret,
    Pipe,
    AmpersandAmpersand,
    PipePipe,
    QuestionMark,
    Colon,
    Semicolon,
    OpeningParenthesis,
    ClosingParenthesis,
    OpeningSquareBracket,
    ClosingSquareBracket,
    OpeningCurlyBrace,
    ClosingCurlyBrace,
    Period,
    Arrow,
    PlusPlus,
    MinusMinus,
    Comma,
    Tilde,
    ExclamationMark,
    Equals,
    PlusEquals,
    MinusEquals,
    AsteriskEquals,
    ForwardSlashEquals,
    PercentEquals,
    LessThanLessThanEquals,
    GreaterThanGreaterThanEquals,
    AmpersandEquals,
    CaretEquals,
    PipeEquals,
    Ellipsis,
}

/// Literal values live outside the UTF-8 source-spelling interner.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct LiteralId(pub(crate) u32);

/// Preserve numeric execution codes separately from source characters. This
/// also retains their meaning when phase 6 concatenates narrow/wide literals.
///
/// C99: a unit is one element after phase-5 conversion, §5.1.1.2 paragraph 1
/// item 5, p. 10; PDF p. 22. `Numeric` is the value of an octal or
/// hexadecimal escape, §6.4.4.4 paragraphs 5-6, p. 60; PDF p. 72.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum LiteralUnit {
    Character(char),
    Numeric(u32),
}

/// Literal encodings introduced by C11 §6.4.5p1 and C23 §6.4.4.4p1.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum LiteralEncoding {
    Utf8,
    Utf16,
    Utf32,
}
impl LiteralEncoding {
    pub(crate) fn prefix(self) -> &'static str {
        match self {
            | Self::Utf8 => "u8",
            | Self::Utf16 => "u",
            | Self::Utf32 => "U",
        }
    }

    pub(crate) fn type_name(self) -> &'static str {
        match self {
            | Self::Utf8 => "char8_t",
            | Self::Utf16 => "char16_t",
            | Self::Utf32 => "char32_t",
        }
    }
}

/// A character or wide `string-literal`, decoded into literal units.
///
/// C99: §6.4.5 paragraphs 1-2, p. 62; PDF p. 74.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum StringTokenType {
    String(LiteralId),
    WideString(LiteralId),
    /// C11/C23 encoding-prefixed literal, retaining code-point/numeric units.
    EncodedString(LiteralId, LiteralEncoding),
}

/// A `character-constant` and its value.
///
/// C99: §6.4.4.4 paragraphs 1-2, pp. 59-60; PDF pp. 71-72, with values from
/// paragraphs 10-11, p. 61; PDF p. 73.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum CharacterTokenType {
    /// A narrow constant of one byte, held as that byte.
    Char(char),
    WideChar(u32),
    EncodedChar(u32, LiteralEncoding),
    /// Packed integer value of an ordinary multi-character constant.
    ///
    /// The value is implementation-defined (§6.4.4.4 paragraph 10): each
    /// byte is shifted in after the ones before it, as GCC does.
    MultiChar(i32),
}

/// The value `#if` gives a character constant. A one-byte narrow constant is
/// never negative here, which C99 leaves implementation-defined (§6.10.1
/// paragraph 4, p. 148; PDF p. 160). Whether plain `char` is signed in
/// phase 7 (§6.2.5 paragraph 15, p. 35; PDF p. 47) is left to semantic
/// analysis.
impl From<CharacterTokenType> for i64 {
    fn from(v: CharacterTokenType) -> Self {
        match v {
            | CharacterTokenType::Char(c) => i64::from(u32::from(c)),
            | CharacterTokenType::WideChar(c) | CharacterTokenType::EncodedChar(c, _) =>
                i64::from(c),
            | CharacterTokenType::MultiChar(value) => i64::from(value),
        }
    }
}

/// The category of a phase-7 token.
///
/// C99: §6.4 paragraph 3, p. 49; PDF p. 61. An `enumeration-constant` is an
/// identifier until declarations are analyzed (§6.4.4.3, p. 59; PDF p. 71).
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum TokenType {
    Integer(IntegerTokenType),
    Float(FloatTokenType),
    Identifier,
    Keyword(KeywordTokenType),
    Operator(OperatorTokenType),
    String(StringTokenType),
    Character(CharacterTokenType),
}

impl KeywordTokenType {
    /// Alias spellings occupy the rest of the reserved interner prefix.
    pub(crate) const ALIASES: &'static [&'static str] = &[
        "bool",
        "alignas",
        "alignof",
        "static_assert",
        "thread_local",
        "__inline",
        "__inline__",
        "__restrict",
        "__restrict__",
        "__const",
        "__const__",
        "__volatile",
        "__volatile__",
        "__signed",
        "__signed__",
        "__alignof",
        "__alignof__",
        "__complex",
        "__complex__",
        "__real",
        "__imag",
        "__typeof",
        "__typeof__",
        "__typeof_unqual",
        "__typeof_unqual__",
        "__attribute",
        "asm",
        "_asm",
    ];
    /// Contexts reserve this contiguous prefix before interning source text.
    /// Keep this in discriminant order; the constructor and regression test
    /// verify that each spelling has its well-known ID.
    pub(crate) const ALL: &[Self] = &[
        Self::Auto,
        Self::Break,
        Self::Case,
        Self::Char,
        Self::Const,
        Self::Continue,
        Self::Default,
        Self::Do,
        Self::Double,
        Self::Else,
        Self::Enum,
        Self::Extern,
        Self::Float,
        Self::For,
        Self::Goto,
        Self::If,
        Self::Inline,
        Self::Int,
        Self::Long,
        Self::Register,
        Self::Restrict,
        Self::Return,
        Self::Short,
        Self::Signed,
        Self::Sizeof,
        Self::Static,
        Self::Struct,
        Self::Switch,
        Self::Typedef,
        Self::Union,
        Self::Unsigned,
        Self::Void,
        Self::Volatile,
        Self::While,
        Self::Bool,
        Self::Complex,
        Self::Imaginary,
        Self::Alignas,
        Self::Alignof,
        Self::Atomic,
        Self::Generic,
        Self::Noreturn,
        Self::StaticAssert,
        Self::ThreadLocal,
        Self::BitInt,
        Self::Decimal32,
        Self::Decimal64,
        Self::Decimal128,
        Self::Constexpr,
        Self::True,
        Self::False,
        Self::Nullptr,
        Self::Typeof,
        Self::TypeofUnqual,
        Self::Countof,
        Self::Attribute,
        Self::Asm,
        Self::Extension,
        Self::BuiltinVaArg,
        Self::BuiltinOffsetof,
        Self::BuiltinTypesCompatible,
        Self::BuiltinChooseExpr,
        Self::LocalLabel,
        Self::Int128,
        Self::AutoType,
        Self::Real,
        Self::Imag,
        Self::Declspec,
        Self::Int8,
        Self::Int16,
        Self::Int32,
        Self::Int64,
        Self::Cdecl,
        Self::Stdcall,
        Self::Fastcall,
        Self::Vectorcall,
        Self::Thiscall,
        Self::Ptr32,
        Self::Ptr64,
        Self::Unaligned,
        Self::W64,
        Self::Sptr,
        Self::Uptr,
        Self::Forceinline,
        Self::Try,
        Self::Except,
        Self::Finally,
        Self::Leave,
        Self::MsAsm,
        Self::Pragma,
    ];

    /// Classification is an integer-index lookup and cheap configuration tests.
    /// C99: §6.4.2.1p4, p. 51; PDF p. 63. Later/non-ISO keywords are
    /// extensions; reserved aliases retain their own diagnostic origin.
    pub(crate) fn classify(
        id: StringCacheId,
        configuration: CompilerConfiguration,
    ) -> Option<KeywordClassification> {
        let index = id.to_u32().checked_sub(1)? as usize;
        if let Some(&kind) = Self::ALL.get(index) {
            // The reserved GNU alias overlaps MSVC's statement introducer.
            // With MS assembly enabled, its grammar owner selects the origin.
            if kind == Self::MsAsm {
                let msvc = configuration.accepts(Feature::MsAsm);
                return Some(KeywordClassification {
                    kind:     if msvc { Self::MsAsm } else { Self::Asm },
                    origin:   if msvc { None } else { Some(FeatureOrigin::Gnu) },
                    spelling: kind.spelling(),
                });
            }
            let (enabled, origin) = match kind {
                | Self::Inline => (
                    configuration.accepts(Feature::Inline),
                    Some(Feature::Inline.origin()),
                ),
                | Self::Restrict => (
                    configuration.accepts(Feature::Restrict),
                    Some(Feature::Restrict.origin()),
                ),
                | Self::Bool => (true, Some(Feature::Bool.origin())),
                | Self::Complex => (true, Some(Feature::Complex.origin())),
                | Self::Imaginary => (true, Some(Feature::Imaginary.origin())),
                | Self::Alignas => (
                    configuration.accepts(Feature::Alignas),
                    Some(Feature::Alignas.origin()),
                ),
                | Self::Alignof => (
                    configuration.accepts(Feature::Alignof),
                    Some(Feature::Alignof.origin()),
                ),
                | Self::Atomic => (
                    configuration.accepts(Feature::Atomic),
                    Some(Feature::Atomic.origin()),
                ),
                | Self::Generic => (
                    configuration.accepts(Feature::Generic),
                    Some(Feature::Generic.origin()),
                ),
                | Self::Noreturn => (
                    configuration.accepts(Feature::Noreturn),
                    Some(Feature::Noreturn.origin()),
                ),
                | Self::StaticAssert => (
                    configuration.accepts(Feature::StaticAssert),
                    Some(Feature::StaticAssert.origin()),
                ),
                | Self::ThreadLocal => (
                    configuration.accepts(Feature::ThreadLocal),
                    Some(Feature::ThreadLocal.origin()),
                ),
                | Self::BitInt => (
                    configuration.accepts(Feature::BitInt),
                    Some(Feature::BitInt.origin()),
                ),
                | Self::Decimal32 | Self::Decimal64 | Self::Decimal128 => (
                    configuration.accepts(Feature::DecimalTypes),
                    Some(Feature::DecimalTypes.origin()),
                ),

                | Self::Constexpr
                | Self::True
                | Self::False
                | Self::Nullptr
                | Self::TypeofUnqual => (
                    configuration.accepts(Feature::C23Keywords),
                    Some(Feature::C23Keywords.origin()),
                ),

                | Self::Typeof => (
                    configuration.accepts(Feature::C23Keywords) || configuration.gnu_extensions(),
                    Some(Feature::C23Keywords.origin()),
                ),

                | Self::Countof => (
                    configuration.accepts(Feature::Countof),
                    Some(Feature::Countof.origin()),
                ),
                | Self::Attribute => (
                    configuration.accepts(Feature::GnuAttribute),
                    Some(Feature::GnuAttribute.origin()),
                ),
                | Self::Asm => (
                    configuration.accepts(Feature::GnuAsm),
                    Some(Feature::GnuAsm.origin()),
                ),
                | Self::Extension => (
                    configuration.accepts(Feature::ExtensionMarker),
                    Some(Feature::ExtensionMarker.origin()),
                ),
                | Self::BuiltinVaArg => (
                    configuration.accepts(Feature::BuiltinVaArg),
                    Some(Feature::BuiltinVaArg.origin()),
                ),
                | Self::BuiltinOffsetof => (
                    configuration.accepts(Feature::BuiltinOffsetof),
                    Some(Feature::BuiltinOffsetof.origin()),
                ),
                | Self::BuiltinTypesCompatible => (
                    configuration.accepts(Feature::BuiltinTypesCompatible),
                    Some(Feature::BuiltinTypesCompatible.origin()),
                ),
                | Self::BuiltinChooseExpr => (
                    configuration.accepts(Feature::BuiltinChooseExpr),
                    Some(Feature::BuiltinChooseExpr.origin()),
                ),
                | Self::LocalLabel => (
                    configuration.accepts(Feature::LocalLabels),
                    Some(Feature::LocalLabels.origin()),
                ),
                | Self::Int128 => (
                    configuration.accepts(Feature::Int128),
                    Some(Feature::Int128.origin()),
                ),
                | Self::AutoType => (
                    configuration.accepts(Feature::AutoType),
                    Some(Feature::AutoType.origin()),
                ),
                | Self::Real | Self::Imag => (
                    configuration.accepts(Feature::RealImag),
                    Some(Feature::RealImag.origin()),
                ),

                | Self::Declspec => (
                    configuration.accepts(Feature::MsDeclspec),
                    Some(Feature::MsDeclspec.origin()),
                ),
                | Self::Int8 | Self::Int16 | Self::Int32 | Self::Int64 => (
                    configuration.accepts(Feature::MsIntTypes),
                    Some(Feature::MsIntTypes.origin()),
                ),

                | Self::Cdecl
                | Self::Stdcall
                | Self::Fastcall
                | Self::Vectorcall
                | Self::Thiscall => (
                    configuration.accepts(Feature::MsCallingConventions),
                    Some(Feature::MsCallingConventions.origin()),
                ),

                | Self::Ptr32
                | Self::Ptr64
                | Self::Unaligned
                | Self::W64
                | Self::Sptr
                | Self::Uptr => (
                    configuration.accepts(Feature::MsTypeQualifiers),
                    Some(Feature::MsTypeQualifiers.origin()),
                ),

                | Self::Forceinline => (
                    configuration.accepts(Feature::MsInline),
                    Some(Feature::MsInline.origin()),
                ),
                | Self::Try | Self::Except | Self::Finally | Self::Leave => (
                    configuration.accepts(Feature::MsSeh),
                    Some(Feature::MsSeh.origin()),
                ),

                | Self::MsAsm => (
                    configuration.accepts(Feature::MsAsm),
                    Some(Feature::MsAsm.origin()),
                ),
                | Self::Pragma => (
                    configuration.accepts(Feature::MsPragma),
                    Some(Feature::MsPragma.origin()),
                ),
                | _ => (true, None),
            };
            return enabled.then_some(KeywordClassification {
                kind,
                origin,
                spelling: kind.spelling(),
            });
        }
        let (kind, enabled, origin) = match index.checked_sub(Self::ALL.len())? {
            | 0 => (
                Self::Bool,
                configuration.accepts(Feature::C23Keywords),
                FeatureOrigin::Standard(CStandard::C23),
            ),
            | 1 => (
                Self::Alignas,
                configuration.accepts(Feature::C23Keywords),
                FeatureOrigin::Standard(CStandard::C23),
            ),
            | 2 => (
                Self::Alignof,
                configuration.accepts(Feature::C23Keywords),
                FeatureOrigin::Standard(CStandard::C23),
            ),
            | 3 => (
                Self::StaticAssert,
                configuration.accepts(Feature::C23Keywords),
                FeatureOrigin::Standard(CStandard::C23),
            ),
            | 4 => (
                Self::ThreadLocal,
                configuration.accepts(Feature::C23Keywords),
                FeatureOrigin::Standard(CStandard::C23),
            ),
            | 5 | 6 => (Self::Inline, true, FeatureOrigin::Gnu),

            | 7 | 8 => (Self::Restrict, true, FeatureOrigin::Gnu),

            | 9 | 10 => (Self::Const, true, FeatureOrigin::Gnu),

            | 11 | 12 => (Self::Volatile, true, FeatureOrigin::Gnu),

            | 13 | 14 => (Self::Signed, true, FeatureOrigin::Gnu),

            | 15 | 16 => (Self::Alignof, true, FeatureOrigin::Gnu),

            | 17 | 18 => (Self::Complex, true, FeatureOrigin::Gnu),

            | 19 => (Self::Real, true, FeatureOrigin::Gnu),
            | 20 => (Self::Imag, true, FeatureOrigin::Gnu),
            | 21 | 22 => (Self::Typeof, true, FeatureOrigin::Gnu),

            | 23 | 24 => (Self::TypeofUnqual, true, FeatureOrigin::Gnu),

            | 25 => (Self::Attribute, true, FeatureOrigin::Gnu),
            | 26 => (
                Self::Asm,
                configuration.gnu_extensions(),
                FeatureOrigin::Gnu,
            ),
            | 27 => (
                Self::MsAsm,
                configuration.accepts(Feature::MsAsm),
                Feature::MsAsm.origin(),
            ),
            | _ => return None,
        };
        enabled.then_some(KeywordClassification {
            kind,
            origin: Some(origin),
            spelling: Self::ALIASES[index - Self::ALL.len()],
        })
    }

    pub(crate) const fn cache_id(self) -> StringCacheId {
        StringCacheId::from_u32(self as u32 + 1)
    }

    /// Only identifiers are classified here, after preprocessing has finished.
    /// No cache access or string comparison is needed for ordinary identifiers.
    ///
    /// C99: a token that could be a keyword or an identifier is a keyword,
    /// §6.4.2.1 paragraph 4, p. 51; PDF p. 63.
    #[cfg(test)]
    pub(crate) fn from_cache_id(id: StringCacheId) -> Option<Self> {
        Self::ALL.get((id.to_u32() - 1) as usize).copied()
    }

    /// The keyword as written in C source.
    pub(crate) fn spelling(self) -> &'static str {
        match self {
            | Self::Auto => "auto",
            | Self::Break => "break",
            | Self::Case => "case",
            | Self::Char => "char",
            | Self::Const => "const",
            | Self::Continue => "continue",
            | Self::Default => "default",
            | Self::Do => "do",
            | Self::Double => "double",
            | Self::Else => "else",
            | Self::Enum => "enum",
            | Self::Extern => "extern",
            | Self::Float => "float",
            | Self::For => "for",
            | Self::Goto => "goto",
            | Self::If => "if",
            | Self::Inline => "inline",
            | Self::Int => "int",
            | Self::Long => "long",
            | Self::Register => "register",
            | Self::Restrict => "restrict",
            | Self::Return => "return",
            | Self::Short => "short",
            | Self::Signed => "signed",
            | Self::Sizeof => "sizeof",
            | Self::Static => "static",
            | Self::Struct => "struct",
            | Self::Switch => "switch",
            | Self::Typedef => "typedef",
            | Self::Union => "union",
            | Self::Unsigned => "unsigned",
            | Self::Void => "void",
            | Self::Volatile => "volatile",
            | Self::While => "while",
            | Self::Bool => "_Bool",
            | Self::Complex => "_Complex",
            | Self::Imaginary => "_Imaginary",
            | Self::Alignas => "_Alignas",
            | Self::Alignof => "_Alignof",
            | Self::Atomic => "_Atomic",
            | Self::Generic => "_Generic",
            | Self::Noreturn => "_Noreturn",
            | Self::StaticAssert => "_Static_assert",
            | Self::ThreadLocal => "_Thread_local",
            | Self::BitInt => "_BitInt",
            | Self::Decimal32 => "_Decimal32",
            | Self::Decimal64 => "_Decimal64",
            | Self::Decimal128 => "_Decimal128",
            | Self::Constexpr => "constexpr",
            | Self::True => "true",
            | Self::False => "false",
            | Self::Nullptr => "nullptr",
            | Self::Typeof => "typeof",
            | Self::TypeofUnqual => "typeof_unqual",
            | Self::Countof => "_Countof",
            | Self::Attribute => "__attribute__",
            | Self::Asm => "__asm__",
            | Self::Extension => "__extension__",
            | Self::BuiltinVaArg => "__builtin_va_arg",
            | Self::BuiltinOffsetof => "__builtin_offsetof",
            | Self::BuiltinTypesCompatible => "__builtin_types_compatible_p",
            | Self::BuiltinChooseExpr => "__builtin_choose_expr",
            | Self::LocalLabel => "__label__",
            | Self::Int128 => "__int128",
            | Self::AutoType => "__auto_type",
            | Self::Real => "__real__",
            | Self::Imag => "__imag__",
            | Self::Declspec => "__declspec",
            | Self::Int8 => "__int8",
            | Self::Int16 => "__int16",
            | Self::Int32 => "__int32",
            | Self::Int64 => "__int64",
            | Self::Cdecl => "__cdecl",
            | Self::Stdcall => "__stdcall",
            | Self::Fastcall => "__fastcall",
            | Self::Vectorcall => "__vectorcall",
            | Self::Thiscall => "__thiscall",
            | Self::Ptr32 => "__ptr32",
            | Self::Ptr64 => "__ptr64",
            | Self::Unaligned => "__unaligned",
            | Self::W64 => "__w64",
            | Self::Sptr => "__sptr",
            | Self::Uptr => "__uptr",
            | Self::Forceinline => "__forceinline",
            | Self::Try => "__try",
            | Self::Except => "__except",
            | Self::Finally => "__finally",
            | Self::Leave => "__leave",
            | Self::MsAsm => "__asm",
            | Self::Pragma => "__pragma",
        }
    }
}

impl OperatorTokenType {
    /// The punctuator as written in C source (never a digraph).
    pub(crate) fn spelling(self) -> &'static str {
        match self {
            | Self::Plus => "+",
            | Self::Minus => "-",
            | Self::Asterisk => "*",
            | Self::ForwardSlash => "/",
            | Self::Percent => "%",
            | Self::LessThanLessThan => "<<",
            | Self::GreaterThanGreaterThan => ">>",
            | Self::LessThan => "<",
            | Self::LessThanEquals => "<=",
            | Self::GreaterThan => ">",
            | Self::GreaterThanEquals => ">=",
            | Self::EqualsEquals => "==",
            | Self::ExclamationMarkEquals => "!=",
            | Self::Ampersand => "&",
            | Self::Caret => "^",
            | Self::Pipe => "|",
            | Self::AmpersandAmpersand => "&&",
            | Self::PipePipe => "||",
            | Self::QuestionMark => "?",
            | Self::Colon => ":",
            | Self::Semicolon => ";",
            | Self::OpeningParenthesis => "(",
            | Self::ClosingParenthesis => ")",
            | Self::OpeningSquareBracket => "[",
            | Self::ClosingSquareBracket => "]",
            | Self::OpeningCurlyBrace => "{",
            | Self::ClosingCurlyBrace => "}",
            | Self::Period => ".",
            | Self::Arrow => "->",
            | Self::PlusPlus => "++",
            | Self::MinusMinus => "--",
            | Self::Comma => ",",
            | Self::Tilde => "~",
            | Self::ExclamationMark => "!",
            | Self::Equals => "=",
            | Self::PlusEquals => "+=",
            | Self::MinusEquals => "-=",
            | Self::AsteriskEquals => "*=",
            | Self::ForwardSlashEquals => "/=",
            | Self::PercentEquals => "%=",
            | Self::LessThanLessThanEquals => "<<=",
            | Self::GreaterThanGreaterThanEquals => ">>=",
            | Self::AmpersandEquals => "&=",
            | Self::CaretEquals => "^=",
            | Self::PipeEquals => "|=",
            | Self::Ellipsis => "...",
        }
    }
}

impl TokenType {
    /// Describes a found token for a message, such as "keyword `int`",
    /// "`;`", or "identifier `count`". `spelling` is the token's source text
    /// when known.
    pub(crate) fn found(self, spelling: Option<&str>) -> impl Display {
        std::fmt::from_fn(move |f| {
            let kind = match self {
                | Self::Keyword(keyword) =>
                    return write!(
                        f,
                        "keyword `{}`",
                        spelling
                            .filter(|text| !text.is_empty())
                            .unwrap_or_else(|| keyword.spelling())
                    ),
                | Self::Operator(operator) => return write!(f, "`{}`", operator.spelling()),
                | Self::Identifier => "identifier",
                | Self::Integer(_) => "integer constant",
                | Self::Float(_) => "floating constant",
                | Self::Character(_) => "character constant",
                | Self::String(_) => "string literal",
            };
            match spelling.filter(|spelling| !spelling.is_empty()) {
                | Some(spelling) => write!(f, "{kind} {}", quote_spelling(spelling)),
                | None => f.write_str(kind),
            }
        })
    }
}

impl SignedIntegerLiteralType {
    pub(super) fn spelling(self) -> &'static str {
        match self {
            | Self::Int => "int",
            | Self::Long => "long",
        }
    }
}

impl UnsignedIntegerLiteralType {
    pub(super) fn spelling(self) -> &'static str {
        match self {
            | Self::UnsignedInt => "unsigned int",
            | Self::UnsignedLong => "unsigned long",
            | Self::UnsignedLongLong => "unsigned long long",
        }
    }
}

/// Classification retains spelling origin independently of the parser kind.
#[derive(Debug, Clone, Copy)]
pub(crate) struct KeywordClassification {
    pub(crate) spelling: &'static str,
    pub(crate) kind:     KeywordTokenType,
    pub(crate) origin:   Option<FeatureOrigin>,
}
