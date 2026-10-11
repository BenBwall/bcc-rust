//! Diagnostics recorded by translation phases 1-3. Lexical errors describe
//! partial comments, partial quoted tokens, and characters that cannot become
//! C tokens. [`InitialProcessorError`] describes the final physical newline.
//! The lexer records locations; token sources decide when to replay
//! diagnostics. Literal values and other phase-7 constraints are checked later.
//!
//! C99: §5.1.1.2 paragraph 1 (phases 2-3), p. 10; PDF p. 22;
//! §6.4 paragraphs 2-3, p. 49; PDF p. 61;
//! comments §6.4.9 paragraphs 1-2, p. 66; PDF p. 78;
//! character constants §6.4.4.4 paragraph 1, p. 59; PDF p. 71;
//! string literals §6.4.5 paragraph 1, p. 62; PDF p. 74.

use std::fmt::{
    self,
    Display,
};

use thiserror::Error;

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
    util::bump::Bump,
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
}

/// Violations of the required final physical newline before line splicing.
/// C99: §5.1.1.2p2, p. 10; PDF p. 22.
#[derive(Debug, Error)]
pub(crate) enum InitialProcessorError {
    /// A nonempty source file ends without a new-line character.
    /// C99: §5.1.1.2p2, p. 10; PDF p. 22.
    #[error("no newline at end of file")]
    MissingFinalNewline(SourceVector),
    /// A backslash immediately precedes the final physical newline.
    /// C99: §5.1.1.2p2, p. 10; PDF p. 22.
    #[error("final newline is escaped")]
    EscapedFinalNewline(SourceVector),
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

impl Display for PreprocessorTokenizerErrorType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let arena = Bump::new();
        f.write_str(self.explain_in(&arena, None).message)
    }
}

impl ToDiagnostic for InitialProcessorError {
    fn diagnostic_in<'d>(
        &self,
        _context: &Context<'_>,
        source: SourceVectors,
        arena: &'d Bump,
    ) -> Diagnostic<'d> {
        let message = format_in!(arena, "{self}");
        match self {
            | Self::EscapedFinalNewline(_) => Explanation::new(arena, message)
                .label("this splice removes the final physical newline")
                .note(
                    "C99 5.1.1.2p2: the final newline shall not be immediately preceded by a \
                     backslash before splicing",
                )
                .help("add an unescaped newline at the end of the file")
                .at(self.severity(), source),
            | Self::MissingFinalNewline(_) => Explanation::new(arena, message)
                .label("the file ends without a newline")
                .note("C99 §5.1.1.2p2: a nonempty source file shall end in a new-line character")
                .help("add a newline at the end of the file")
                .at(self.severity(), source),
        }
    }
}

impl GetPosition for InitialProcessorError {
    #[inline(always)]
    fn position(&self, context: &Context<'_>) -> SourcePosition {
        match self {
            | Self::MissingFinalNewline(vector) | Self::EscapedFinalNewline(vector) =>
                vector.position(context),
        }
    }
}

impl GetSeverity for InitialProcessorError {
    fn severity(&self) -> ErrorSeverity {
        match self {
            | Self::MissingFinalNewline(_) | Self::EscapedFinalNewline(_) => ErrorSeverity::Warning,
        }
    }
}

impl GetSourceVectors for InitialProcessorError {
    fn source_vectors(&self, context: &mut Context<'_>) -> SourceVectors {
        match self {
            | Self::MissingFinalNewline(vector) | Self::EscapedFinalNewline(vector) => context
                .create_source_vectors(
                    vector.position(context),
                    vector.source_file_index,
                    vector.length as usize,
                ),
        }
    }
}
