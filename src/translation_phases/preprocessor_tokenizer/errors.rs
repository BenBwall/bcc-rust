//! Diagnostics recorded by translation phase 3. Lexical errors describe
//! partial comments, partial quoted tokens, and characters that cannot become
//! C tokens. The lexer records locations; token sources decide when to replay
//! diagnostics. Literal values and other phase-7 constraints are checked later.
//!
//! C99: §5.1.1.2 paragraph 1 (phase 3), p. 10; PDF p. 22;
//! §6.4 paragraphs 2-3, p. 49; PDF p. 61;
//! comments §6.4.9 paragraphs 1-2, p. 66; PDF p. 78;
//! character constants §6.4.4.4 paragraph 1, p. 59; PDF p. 71;
//! string literals §6.4.5 paragraph 1, p. 62; PDF p. 74.

use std::fmt::{
    self,
    Display,
};

use crate::{
    diagnostics::{
        Diagnostic,
        Explanation,
        ToDiagnostic,
        format_in,
    },
    translation_phases::{
        Context,
        ErrorSeverity,
        GetPosition,
        GetSeverity,
        GetSourceVectors,
        SourcePosition,
        SourceVector,
        SourceVectors,
    },
    util::{
        bump::Bump,
        raw_bytes,
    },
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PreprocessorTokenizerError {
    pub(super) source_vector: SourceVector,
    pub(super) error_type:    PreprocessorTokenizerErrorType,
    /// For an unknown token, the character after phases 1 and 2 that no
    /// token starts with; its raw spelling may be a trigraph.
    pub(super) character:     Option<char>,
}

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
    /// A `'` or `"` with no closing quote before the line or the file ends,
    /// in the text of phase 4 (`-E`), where the rest of the line is kept as
    /// it was written. Phase 7 never converts it, so only a directive that
    /// reads it as a literal rejects it; the lexer warns, as GCC and Clang
    /// do (Clang `-Winvalid-pp-token`).
    /// C99: a `'` or `"` that begins no token is undefined, §6.4p3, p. 49;
    /// PDF p. 61.
    MissingTerminatingQuote(u8),
}

impl PreprocessorTokenizerErrorType {
    /// The quote of a character constant or string literal that closes on
    /// no line, which this diagnoses.
    pub(crate) const fn unclosed_quote(self) -> Option<u8> {
        match self {
            | Self::UnterminatedCharacter | Self::NewlineInCharacter => Some(b'\''),
            | Self::UnterminatedString | Self::NewlineInString => Some(b'"'),
            | Self::MissingTerminatingQuote(quote) => Some(quote),
            | Self::UnknownToken | Self::UnterminatedBlockComment => None,
        }
    }

    /// Describes the error; `spelling` is the source text it points at.
    pub(crate) fn explain_in<'d>(self, arena: &'d Bump, spelling: Option<&str>) -> Explanation<'d> {
        let new = |message: &'d str| Explanation::new(arena, message);
        match self {
            | Self::UnknownToken => {
                let character = spelling.and_then(|spelling| spelling.chars().next());
                if let Some(byte) = character.and_then(raw_bytes::raw_byte) {
                    return new(format_in!(arena, "stray byte <{byte:02X}> in source"))
                        .label("this byte begins no UTF-8 character")
                        .note("C99 §6.4: no C token is spelled with this byte");
                }
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
            | Self::MissingTerminatingQuote(quote) => new(if quote == b'"' {
                "missing terminating `\"` character"
            } else {
                "missing terminating `'` character"
            })
            .label("the rest of the line is kept as written")
            .note("C99 §6.4p3: a `'` or `\"` that begins no token is undefined"),
        }
    }
}

impl PreprocessorTokenizerError {
    pub(crate) fn is_unclosed_header_string_at(&self, source: &SourceVector) -> bool {
        matches!(
            self.error_type,
            PreprocessorTokenizerErrorType::UnterminatedString
                | PreprocessorTokenizerErrorType::NewlineInString
                | PreprocessorTokenizerErrorType::MissingTerminatingQuote(b'"')
        ) && self.source_vector == *source
    }

    /// Reports a literal with no closing quote that a directive reads, in a
    /// text of phase 4, where the lexer kept it as written.
    pub(crate) fn unclosed_literal(source_vector: SourceVector, quote: u8) -> Self {
        Self {
            source_vector,
            error_type: if quote == b'"' {
                PreprocessorTokenizerErrorType::NewlineInString
            } else {
                PreprocessorTokenizerErrorType::NewlineInCharacter
            },
            character: None,
        }
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
            | PreprocessorTokenizerErrorType::MissingTerminatingQuote(_) => ErrorSeverity::Warning,
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

impl Display for PreprocessorTokenizerErrorType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let arena = Bump::new();
        f.write_str(self.explain_in(&arena, None).message)
    }
}
