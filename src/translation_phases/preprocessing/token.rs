//! Tokens produced for the parser.

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

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum SignedIntegerLiteralType {
    Int,
    Long,
    LongLong,
}

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

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum IntegerSuffix {
    Unsigned,
    Long,
    LongLong,
    UnsignedLong,
    UnsignedLongLong,
}

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
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum LiteralUnit {
    Character(char),
    Numeric(u32),
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum StringTokenType {
    String(LiteralId),
    WideString(LiteralId),
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum CharacterTokenType {
    Char(char),
    WideChar(u32),
    /// Packed integer value of an ordinary multi-character constant.
    MultiChar(i32),
}

impl From<CharacterTokenType> for i64 {
    fn from(v: CharacterTokenType) -> Self {
        match v {
            | CharacterTokenType::Char(c) => i64::from(u32::from(c)),
            | CharacterTokenType::WideChar(c) => i64::from(c),
            | CharacterTokenType::MultiChar(value) => i64::from(value),
        }
    }
}

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
            | Self::LongLong => "long long",
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
