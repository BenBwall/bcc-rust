//! Preprocessing-token formation in translation phase 3.
//!
//! C99: §5.1.1.2p3, p. 10; PDF p. 22; lexical categories and maximal munch are
//! §6.4p1-4, pp. 49-50; PDF pp. 61-62. This lexer does not form header-name
//! tokens; `#include` handling interprets their source spelling in phase 4
//! (§6.4p4, p. 50; PDF p. 62; §6.4.7, pp. 64-65; PDF pp. 76-77).

mod batch;
mod replay;
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod tests;
mod token_source;
pub(crate) mod ucn;

use std::fmt::{
    self,
    Display,
};

pub(crate) use batch::{
    LogicalCharacter,
    logical_characters,
    position_after,
};
pub(crate) use token_source::{
    LexedFiles,
    TokenSource,
};

use super::{
    Context,
    ErrorSeverity,
    GetPosition,
    GetSeverity,
    GetSourceVectors,
    SourcePosition,
    SourceVector,
    SourceVectors,
};
use crate::{
    diagnostics::{
        Diagnostic,
        Explanation,
        ToDiagnostic,
        format_in,
        quote_spelling,
    },
    util::{
        bump::Bump,
        string_cache::StringCacheId,
    },
};

/// Phase-3 lexical failures and partial tokens.
/// C99: §5.1.1.2p3, p. 10; PDF p. 22; §6.4p2-3, p. 49; PDF p. 61; §6.4.9p1-2,
/// p. 66; PDF p. 78.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) enum PreprocessorTokenizerErrorType {
    /// An other preprocessing token cannot become a phase-7 token.
    /// C99: §6.4p2-3, p. 49; PDF p. 61.
    UnknownToken,
    /// A source file ends in a partial comment.
    /// C99: §5.1.1.2p3, p. 10; PDF p. 22.
    UnterminatedBlockComment,
    /// A source file ends in a partial `character-constant`.
    /// C99: §5.1.1.2p3, p. 10; PDF p. 22; §6.4.4.4p1, p. 59; PDF p. 71.
    UnterminatedCharacter,
    /// A source file ends in a partial `string-literal`.
    /// C99: §5.1.1.2p3, p. 10; PDF p. 22; §6.4.5p1, p. 62; PDF p. 74.
    UnterminatedString,
    /// A new-line cannot occur in a `c-char`.
    /// C99: §6.4.4.4p1, p. 59; PDF p. 71.
    NewlineInCharacter,
    /// A new-line cannot occur in an `s-char`.
    /// C99: §6.4.5p1, p. 62; PDF p. 74.
    NewlineInString,
}

impl PreprocessorTokenizerErrorType {
    /// Describes the error; `spelling` is the source text it points at.
    pub(crate) fn explain_in<'d>(self, arena: &'d Bump, spelling: Option<&str>) -> Explanation<'d> {
        let new = |message: &'d str| Explanation::new(arena, message);
        match self {
            | Self::UnknownToken => {
                let character = spelling.and_then(|spelling| spelling.chars().next());
                let quoted = fmt::from_fn(|f| match character {
                    | Some('`') => f.write_str("'`'"),
                    | Some(c) if c.is_control() => write!(f, "U+{:04X}", u32::from(c)),
                    | Some(c) => write!(f, "`{c}`"),
                    | None => f.write_str("character"),
                });
                new(format_in!(arena, "unexpected character {quoted} in source"))
                    .label("no C token starts with this character")
                    .note(
                        "C99 §6.4: a preprocessing token that survives replacement must be \
                         convertible to a C token",
                    )
            },
            | Self::UnterminatedBlockComment => new("unterminated block comment")
                .label("the file ends before the closing `*/`")
                .note("C99 §5.1.1.2p3: a source file shall not end in a partial comment")
                .help("close the comment with `*/`"),
            | Self::UnterminatedCharacter =>
                new("unterminated character constant").label("the file ends before the closing `'`"),
            | Self::UnterminatedString =>
                new("unterminated string literal").label("the file ends before the closing `\"`"),
            | Self::NewlineInCharacter => new("unterminated character constant")
                .label("the line ends before the closing `'`")
                .note("C99 §6.4.4.4: a character constant cannot span lines")
                .help("write `\\n` for a newline character"),
            | Self::NewlineInString => new("unterminated string literal")
                .label("the line ends before the closing `\"`")
                .note("C99 §6.4.5: a string literal cannot span lines")
                .help(
                    "close the literal on this line and start another on the next; adjacent \
                     literals are concatenated",
                ),
        }
    }
}

impl Display for PreprocessorTokenizerErrorType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let arena = Bump::new();
        f.write_str(self.explain_in(&arena, None).message)
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PreprocessorTokenizerError {
    source_vector: SourceVector,
    error_type:    PreprocessorTokenizerErrorType,
    /// For an unknown token, the character after phases 1 and 2 that no
    /// token starts with; its raw spelling may be a trigraph.
    character:     Option<char>,
}

impl PreprocessorTokenizerError {
    pub(crate) fn is_unclosed_header_string_at(&self, source: &SourceVector) -> bool {
        matches!(
            self.error_type,
            PreprocessorTokenizerErrorType::UnterminatedString
                | PreprocessorTokenizerErrorType::NewlineInString
        ) && self.source_vector == *source
    }

    /// Reports a phase-3 character that survived preprocessing but cannot
    /// become a C token in phase 7.
    pub(crate) fn unknown_character(source_vector: SourceVector, character: char) -> Self {
        Self {
            source_vector,
            error_type: PreprocessorTokenizerErrorType::UnknownToken,
            character: Some(character),
        }
    }
}

impl std::error::Error for PreprocessorTokenizerError {}

impl GetPosition for PreprocessorTokenizerError {
    #[inline(always)]
    fn position(&self, context: &Context<'_>) -> SourcePosition {
        self.source_vector.position(context)
    }
}

impl GetSourceVectors for PreprocessorTokenizerError {
    #[inline(always)]
    fn source_vectors(&self, context: &mut Context<'_>) -> SourceVectors {
        context.create_source_vectors(
            self.source_vector.position(context),
            self.source_vector.source_file_index,
            self.source_vector.length as usize,
        )
    }
}

impl GetSeverity for PreprocessorTokenizerError {
    fn severity(&self) -> ErrorSeverity {
        match self.error_type {
            | PreprocessorTokenizerErrorType::UnknownToken
            | PreprocessorTokenizerErrorType::UnterminatedBlockComment
            | PreprocessorTokenizerErrorType::UnterminatedCharacter
            | PreprocessorTokenizerErrorType::UnterminatedString
            | PreprocessorTokenizerErrorType::NewlineInCharacter
            | PreprocessorTokenizerErrorType::NewlineInString => ErrorSeverity::Error,
        }
    }
}

impl Display for PreprocessorTokenizerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.error_type)
    }
}

impl ToDiagnostic for PreprocessorTokenizerError {
    fn diagnostic_in<'d>(
        &self,
        context: &Context<'_>,
        source: SourceVectors,
        arena: &'d Bump,
    ) -> Diagnostic<'d> {
        let mut buffer = [0; 4];
        let spelling = match self.character {
            | Some(character) => Some(&*character.encode_utf8(&mut buffer)),
            | None => context.source_spelling(source),
        };
        self.error_type
            .explain_in(arena, spelling)
            .at(self.severity(), source)
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PreprocessorToken {
    pub(crate) kind:           PreprocessorTokenType,
    pub(crate) source_vectors: SourceVectors,
    pub(crate) contents:       StringCacheId,
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

impl Default for PreprocessorToken {
    fn default() -> Self {
        Self {
            kind:           PreprocessorTokenType::WideGeneratedString,
            source_vectors: SourceVectors::empty(),
            contents:       StringCacheId::from_u32(u32::MAX),
        }
    }
}
