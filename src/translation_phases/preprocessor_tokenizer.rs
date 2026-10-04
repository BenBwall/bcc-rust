mod batch;
mod replay;
#[cfg(test)]
mod tests;
mod token_source;
pub(crate) mod ucn;

use std::fmt::Display;

pub(crate) use batch::{
    LogicalCharacter,
    logical_characters,
    position_after,
};
pub(crate) use token_source::TokenSource;

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
        quote_spelling,
    },
    util::string_cache::StringCacheId,
};

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) enum PreprocessorTokenizerErrorType {
    UnknownToken,
    UnterminatedBlockComment,
    UnterminatedCharacter,
    UnterminatedString,
    NewlineInCharacter,
    NewlineInString,
}

impl PreprocessorTokenizerErrorType {
    /// Describes the error; `spelling` is the source text it points at.
    pub(crate) fn explain(self, spelling: Option<&str>) -> Explanation {
        match self {
            | Self::UnknownToken => {
                let character = spelling.and_then(|spelling| spelling.chars().next());
                let quoted = match character {
                    | Some('`') => "'`'".to_owned(),
                    | Some(c) if c.is_control() => format!("U+{:04X}", u32::from(c)),
                    | Some(c) => format!("`{c}`"),
                    | None => "character".to_owned(),
                };
                Explanation::new(format!("unexpected character {quoted} in source"))
                    .label("no C token starts with this character")
                    .note(
                        "C99 §6.4: a preprocessing token that survives replacement must be \
                         convertible to a C token",
                    )
            },
            | Self::UnterminatedBlockComment => Explanation::new("unterminated block comment")
                .label("the file ends before the closing `*/`")
                .note("C99 §5.1.1.2p3: a source file shall not end in a partial comment")
                .help("close the comment with `*/`"),
            | Self::UnterminatedCharacter => Explanation::new("unterminated character constant")
                .label("the file ends before the closing `'`"),
            | Self::UnterminatedString => Explanation::new("unterminated string literal")
                .label("the file ends before the closing `\"`"),
            | Self::NewlineInCharacter => Explanation::new("unterminated character constant")
                .label("the line ends before the closing `'`")
                .note("C99 §6.4.4.4: a character constant cannot span lines")
                .help("write `\\n` for a newline character"),
            | Self::NewlineInString => Explanation::new("unterminated string literal")
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
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.explain(None).message)
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
    pub(crate) fn found(self, spelling: Option<&str>) -> String {
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
        match spelling {
            | Some(spelling) if spelled && !spelling.is_empty() =>
                format!("{} {}", self.description(), quote_spelling(spelling)),
            | _ => self.description().to_owned(),
        }
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
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.error_type)
    }
}

impl ToDiagnostic for PreprocessorTokenizerError {
    fn to_diagnostic(&self, context: &Context<'_>, source: SourceVectors) -> Diagnostic {
        let mut buffer = [0; 4];
        let spelling = match self.character {
            | Some(character) => Some(&*character.encode_utf8(&mut buffer)),
            | None => context.source_spelling(source),
        };
        self.error_type
            .explain(spelling)
            .at(self.severity(), source)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) enum PreprocessorTokenType {
    // Literals
    Identifier,
    /// Suppressed during rescan; remains unavailable in later rescans
    /// (6.10.3.4p2).
    UnavailableIdentifier,
    /// Canonical identity is in contents; this payload preserves source
    /// spelling.
    UniversalIdentifier,
    UnavailableUniversalIdentifier,
    Number,
    String,
    Character,

    /// A non-whitespace character outside the other pp-token categories
    /// (C99 §6.4p3). It can be discarded or stringified in phase 4.
    Other,

    // Expanded from hash operator
    GeneratedString,
    WideGeneratedString,

    // Generated when a macro argument generated no tokens
    Placeholder,

    // Whitespace
    Newline,
    Whitespace,

    // Keywords
    Defined,

    // Punctuation
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
