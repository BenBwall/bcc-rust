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
}

impl From<IntegerTokenType> for i128 {
    fn from(v: IntegerTokenType) -> Self {
        match v {
            | IntegerTokenType::UnsignedLong(v) | IntegerTokenType::UnsignedLongLong(v) =>
                i128::from(v.get()),
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
}

impl FloatTokenType {
    /// The C type named by this constant's suffix.
    pub(crate) fn type_name(&self) -> &'static str {
        match self {
            | Self::Float(_) => "float",
            | Self::Double(_) => "double",
            | Self::LongDouble(_) => "long double",
        }
    }
}

impl Display for FloatTokenType {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            | Self::Float(v) => write!(f, "{v}"),
            | Self::Double(v) => write!(f, "{v}"),
            | Self::LongDouble(v) => write!(f, "{v}"),
        }
    }
}

/// A `keyword`.
///
/// C99: §6.4.1 paragraph 1, p. 50; PDF p. 62.
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

/// A character or wide `string-literal`, decoded into literal units.
///
/// C99: §6.4.5 paragraphs 1-2, p. 62; PDF p. 74.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum StringTokenType {
    String(LiteralId),
    WideString(LiteralId),
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
            | CharacterTokenType::WideChar(c) => i64::from(c),
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
    ];

    pub(crate) const fn cache_id(self) -> StringCacheId {
        StringCacheId::from_u32(self as u32 + 1)
    }

    /// Only identifiers are classified here, after preprocessing has finished.
    /// No cache access or string comparison is needed for ordinary identifiers.
    ///
    /// C99: a token that could be a keyword or an identifier is a keyword,
    /// §6.4.2.1 paragraph 4, p. 51; PDF p. 63.
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
                | Self::Keyword(keyword) => return write!(f, "keyword `{}`", keyword.spelling()),
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
