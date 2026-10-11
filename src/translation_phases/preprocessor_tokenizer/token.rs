use std::fmt::{
    self,
    Display,
};

use super::{
    Context,
    SourceVectors,
};
use crate::{
    diagnostics::quote_spelling,
    util::string_cache::StringCacheId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PreprocessorToken {
    pub(crate) kind:           PreprocessorTokenType,
    pub(crate) source_vectors: SourceVectors,
    pub(crate) contents:       StringCacheId,
}

/// Phase-3 preprocessing-token categories plus phase-4 internal markers.
/// C99: §6.4p1-3, p. 49; PDF p. 61; placemarkers §6.10.3.3, p. 154; PDF p. 166.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) enum PreprocessorTokenType {
    // Identifiers: C99 §6.4.2.1p1, p. 51; PDF p. 63.
    Identifier,
    /// Suppressed during rescan; remains unavailable in later rescans
    /// C99: §6.10.3.4p2, p. 155; PDF p. 167.
    UnavailableIdentifier,
    /// Canonical identity is in contents; this payload preserves source
    /// spelling.
    UniversalIdentifier,
    UnavailableUniversalIdentifier,
    // `pp-number`: C99 §6.4.8p1, p. 65; PDF p. 77.
    Number,
    // Quoted preprocessing tokens: C99 §6.4.5p1, p. 62; PDF p. 74;
    // §6.4.4.4p1, p. 59; PDF p. 71.
    String,
    Character,

    /// A non-whitespace character outside the other pp-token categories
    /// (C99 §6.4p3). It can be discarded or stringified in phase 4.
    Other,

    // Stringification: C99 §6.10.3.2p2, p. 153; PDF p. 165.
    // Expanded from hash operator
    GeneratedString,
    // A generated string with an L, u, U or u8 prefix formed by pasting.
    WideGeneratedString,

    // Placemarkers are internal to phase 4: C99 §6.10.3.3p2,
    // p. 154; PDF p. 166.
    // Generated when a macro argument generated no tokens
    Placeholder,

    // Phase-3 whitespace: C99 §5.1.1.2p3, p. 10; PDF p. 22.
    // Whitespace
    Newline,
    Whitespace,

    // The `defined` operator: C99 §6.10.1p1, pp. 147-148;
    // PDF pp. 159-160.
    // Phase-4 conditional operator
    Defined,

    // Punctuation: C99 §6.4.6p1, p. 63; PDF p. 75; digraphs
    // §6.4.6p3, p. 64; PDF p. 76.
    // In order of appearance in the C99 standard.
    OpeningSquareBracket,
    ClosingSquareBracket,
    OpeningParenthesis,
    ClosingParenthesis,
    OpeningCurlyBrace,
    ClosingCurlyBrace,
    Period,
    Arrow,
    PlusPlus,
    MinusMinus,
    Ampersand,
    Asterisk,
    Plus,
    Minus,
    Tilde,
    ExclamationMark,
    ForwardSlash,
    Percent,
    LessThanLessThan,
    GreaterThanGreaterThan,
    LessThan,
    GreaterThan,
    LessThanEquals,
    GreaterThanEquals,
    EqualsEquals,
    ExclamationMarkEquals,
    Caret,
    Pipe,
    AmpersandAmpersand,
    PipePipe,
    QuestionMark,
    Colon,
    SemiColon,
    Ellipsis,
    Equals,
    AsteriskEquals,
    ForwardSlashEquals,
    PercentEquals,
    PlusEquals,
    MinusEquals,
    LessThanLessThanEquals,
    GreaterThanGreaterThanEquals,
    AmpersandEquals,
    CaretEquals,
    PipeEquals,
    Comma,
    Hash,
    HashHash,
}

impl PreprocessorToken {
    pub(crate) fn identifier_id(self, context: &Context<'_>) -> StringCacheId {
        match self.kind {
            | PreprocessorTokenType::UniversalIdentifier
            | PreprocessorTokenType::UnavailableUniversalIdentifier =>
                context.canonical_identifiers[&self.contents],
            | _ => self.contents,
        }
    }
}

impl PreprocessorTokenType {
    /// Names the token kind for use in a message, such as "identifier" or
    /// "`(`".
    pub(crate) fn description(self) -> &'static str {
        match self {
            | Self::Identifier
            | Self::UnavailableIdentifier
            | Self::UnavailableUniversalIdentifier
            | Self::UniversalIdentifier => "identifier",
            | Self::Number => "number",
            | Self::String | Self::GeneratedString => "string literal",
            | Self::WideGeneratedString => "wide string literal",
            | Self::Character => "character constant",
            | Self::Other => "character",
            | Self::Placeholder => "empty macro argument",
            | Self::Newline => "end of line",
            | Self::Whitespace => "whitespace",
            | Self::Defined => "`defined`",
            | Self::OpeningSquareBracket => "`[`",
            | Self::ClosingSquareBracket => "`]`",
            | Self::OpeningParenthesis => "`(`",
            | Self::ClosingParenthesis => "`)`",
            | Self::OpeningCurlyBrace => "`{`",
            | Self::ClosingCurlyBrace => "`}`",
            | Self::Period => "`.`",
            | Self::Arrow => "`->`",
            | Self::PlusPlus => "`++`",
            | Self::MinusMinus => "`--`",
            | Self::Ampersand => "`&`",
            | Self::Asterisk => "`*`",
            | Self::Plus => "`+`",
            | Self::Minus => "`-`",
            | Self::Tilde => "`~`",
            | Self::ExclamationMark => "`!`",
            | Self::ForwardSlash => "`/`",
            | Self::Percent => "`%`",
            | Self::LessThanLessThan => "`<<`",
            | Self::GreaterThanGreaterThan => "`>>`",
            | Self::LessThan => "`<`",
            | Self::GreaterThan => "`>`",
            | Self::LessThanEquals => "`<=`",
            | Self::GreaterThanEquals => "`>=`",
            | Self::EqualsEquals => "`==`",
            | Self::ExclamationMarkEquals => "`!=`",
            | Self::Caret => "`^`",
            | Self::Pipe => "`|`",
            | Self::AmpersandAmpersand => "`&&`",
            | Self::PipePipe => "`||`",
            | Self::QuestionMark => "`?`",
            | Self::Colon => "`:`",
            | Self::SemiColon => "`;`",
            | Self::Ellipsis => "`...`",
            | Self::Equals => "`=`",
            | Self::AsteriskEquals => "`*=`",
            | Self::ForwardSlashEquals => "`/=`",
            | Self::PercentEquals => "`%=`",
            | Self::PlusEquals => "`+=`",
            | Self::MinusEquals => "`-=`",
            | Self::LessThanLessThanEquals => "`<<=`",
            | Self::GreaterThanGreaterThanEquals => "`>>=`",
            | Self::AmpersandEquals => "`&=`",
            | Self::CaretEquals => "`^=`",
            | Self::PipeEquals => "`|=`",
            | Self::Comma => "`,`",
            | Self::Hash => "`#`",
            | Self::HashHash => "`##`",
        }
    }

    pub(crate) fn is_identifier(self) -> bool {
        matches!(
            self,
            Self::Identifier
                | Self::UniversalIdentifier
                | Self::UnavailableIdentifier
                | Self::UnavailableUniversalIdentifier
        )
    }

    /// Describes a found token, quoting its spelling when the kind alone
    /// does not say what was written.
    pub(crate) fn found(self, spelling: Option<&str>) -> impl Display {
        let spelled = matches!(
            self,
            Self::Identifier
                | Self::Number
                | Self::String
                | Self::GeneratedString
                | Self::WideGeneratedString
                | Self::Character
                | Self::Other
        );
        fmt::from_fn(move |f| match spelling {
            | Some(spelling) if spelled && !spelling.is_empty() =>
                write!(f, "{} {}", self.description(), quote_spelling(spelling)),
            | _ => f.write_str(self.description()),
        })
    }
}

impl Default for PreprocessorToken {
    fn default() -> Self {
        Self {
            kind:           PreprocessorTokenType::WideGeneratedString,
            source_vectors: SourceVectors::empty(),
            contents:       StringCacheId::from_u32(u32::MAX),
        }
    }
}
