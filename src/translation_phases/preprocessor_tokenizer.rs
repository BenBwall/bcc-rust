use std::{
    fmt::Display,
    mem::replace,
    num::NonZeroU32,
};

use arrayvec::ArrayVec;
use proptest::prelude::Rng;
use smallstr::SmallString;
use thiserror::Error;

use super::{
    initial_processing::InitialProcessor,
    Context,
    ErrorSeverity,
    GetPosition,
    GetSeverity,
    SavePoint as ISavePoint,
    SourcePosition,
    SourceVector,
    SourceVectors,
    TokenString,
    TranslationPhase,
};
use crate::util::{
    small_queue::SmallQueue,
    string_cache::{
        Interner,
        StringCacheId,
    },
};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy)]
pub(crate) struct PreprocessorTokenizer {
    initial_processor:   InitialProcessor,
    contents:            TokenString,
    current_token_start: SourcePosition,
}

impl GetPosition for PreprocessorTokenizer {
    fn position(&self) -> SourcePosition {
        self.initial_processor.position()
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) enum PreprocessorTokenizerErrorType {
    UnknownToken,
    UnterminatedCharacter,
    UnterminatedString,
    UnterminatedIncludeString,
    NewlineInCharacter,
    NewlineInString,
    NewlineInIncludeString,
    MissingSignInExponent,
}

impl Display for PreprocessorTokenizerErrorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            | Self::UnknownToken => write!(f, "Unknown token"),
            | Self::UnterminatedCharacter => write!(f, "Unterminated character literal"),
            | Self::UnterminatedString => write!(f, "Unterminated string literal"),

            | Self::UnterminatedIncludeString => write!(f, "Unterminated header include string"),
            | Self::NewlineInCharacter => write!(
                f,
                "Unescaped newlines are not allowed in character literals"
            ),
            | Self::NewlineInString =>
                write!(f, "Unescaped newlines are not allowed in string literals"),
            | Self::NewlineInIncludeString => write!(
                f,
                "Unescaped newlines are not allowed in header include strings"
            ),
            | Self::MissingSignInExponent =>
                write!(f, "Exponent in floating point number is missing a sign"),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PreprocessorTokenizerError {
    source_vector: SourceVector,
    error_type:    PreprocessorTokenizerErrorType,
}

impl std::error::Error for PreprocessorTokenizerError {}

impl GetSeverity for PreprocessorTokenizerError {
    fn severity(&self) -> ErrorSeverity {
        match self.error_type {
            | PreprocessorTokenizerErrorType::UnknownToken
            | PreprocessorTokenizerErrorType::UnterminatedCharacter
            | PreprocessorTokenizerErrorType::UnterminatedString
            | PreprocessorTokenizerErrorType::UnterminatedIncludeString
            | PreprocessorTokenizerErrorType::NewlineInCharacter
            | PreprocessorTokenizerErrorType::NewlineInString
            | PreprocessorTokenizerErrorType::NewlineInIncludeString
            | PreprocessorTokenizerErrorType::MissingSignInExponent => ErrorSeverity::Error,
        }
    }
}

impl Display for PreprocessorTokenizerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.error_type)
    }
}

impl TranslationPhase for PreprocessorTokenizer {
    type Item = PreprocessorToken;

    fn next_item(&mut self, context: &mut Context) -> Option<PreprocessorToken> {
        let start_index = self.initial_processor.position().index;
        let start_column = self.initial_processor.position().column;
        let start_line = self.initial_processor.position().line;
        loop {
            self.contents.clear();
            self.contents.shrink_to_fit();
            self.current_token_start = self.initial_processor.position();
            let Some(input) = self.initial_processor.next_item(context) else {
                break None;
            };
            self.contents.push(input);
            break Some(match input {
                | '\n' => self.generate_token(context, PreprocessorTokenType::Newline),
                | '0'..='9' => self.tokenize_number(context),
                | '{' => self.generate_token(context, PreprocessorTokenType::OpeningCurlyBrace),
                | '}' => self.generate_token(context, PreprocessorTokenType::ClosingCurlyBrace),
                | '(' => self.generate_token(context, PreprocessorTokenType::OpeningParenthesis),
                | ')' => self.generate_token(context, PreprocessorTokenType::ClosingParenthesis),
                | '[' => self.generate_token(context, PreprocessorTokenType::OpeningSquareBracket),
                | ']' => self.generate_token(context, PreprocessorTokenType::ClosingSquareBracket),
                | 'L' => self.tokenize_wide_string_or_identifier(context),
                | c if c.is_alphabetic() || c == '_' => self.tokenize_identifier(context),
                | '.' => self.tokenize_period_or_number(context),
                | '"' =>
                    if context.is_tokenizing_include_string {
                        self.tokenize_include_string(context)
                    } else {
                        self.tokenize_string(context)
                    },
                | '\'' => self.tokenize_char(context),
                | '#' => self.tokenize_hash(context),
                | ' ' => self.tokenize_whitespace(context),
                | '/' => self.tokenize_forward_slash(context),
                | '%' => self.tokenize_percent(context),
                | '<' =>
                    if self.is_tokenizing_include_string {
                        match self.tokenize_angle_bracket_string(context) {
                            | Some(token) => token,
                            | None => {
                                context.source.index = start_index;
                                context.source.column = start_column;
                                context.source.line = start_line;
                                context.is_tokenizing_include_string = false;
                                self.contents.clear();
                                self.contents.shrink_to_fit();
                                continue;
                            },
                        }
                    } else {
                        self.tokenize_left_angle_bracket(context)
                    },
                | '>' => self.tokenize_right_angle_bracket(context),
                | ',' => self.generate_token(context, PreprocessorTokenType::Comma),
                | ';' => self.generate_token(context, PreprocessorTokenType::SemiColon),
                | '?' => self.generate_token(context, PreprocessorTokenType::QuestionMark),
                | '~' => self.generate_token(context, PreprocessorTokenType::Tilde),
                | ':' => self.tokenize_colon(context),
                | '+' => self.tokenize_plus(context),
                | '-' => self.tokenize_minus(context),
                | '*' => self.tokenize_asterisk(context),
                | '^' => self.tokenize_caret(context),
                | '&' => self.tokenize_ampersand(context),
                | '|' => self.tokenize_pipe(context),
                | '!' => self.tokenize_exclamation_mark(context),
                | '=' => self.tokenize_equals(context),
                | _ => {
                    self.generate_error(context, PreprocessorTokenizerErrorType::UnknownToken);
                    continue;
                },
            });
        }
    }
}

impl PreprocessorTokenizer {
    fn generate_error(
        &mut self,
        context: &mut Context,
        error_type: PreprocessorTokenizerErrorType,
    ) {
        let start_position = self.current_token_start;

        context.tokenizer_error(PreprocessorTokenizerError::TokenizerError(
            InnerPreprocessorTokenizerError {
                start_position,
                length: self.position().index - start_position.index,
                error_type,
                contents: context.string_cache.intern(self.contents.as_str()),
            },
        ));
    }

    fn generate_token_with_position(
        &mut self,
        context: &mut Context,
        token_type: PreprocessorTokenType,
        source_position: SourcePosition,
        length: u32,
    ) -> PreprocessorToken {
        let source_vector = context.push_source_vector(source_position, length);
        PreprocessorToken {
            source_vectors: SourceVectors {
                start_index: source_vector,
                length:      1,
            },
            kind:           token_type,
            contents:       context.string_cache.intern(self.contents.as_str()),
        }
    }

    fn generate_token(
        &mut self,
        context: &mut Context,
        token_type: PreprocessorTokenType,
    ) -> PreprocessorToken {
        let current_token_start = self.current_token_start;
        let current_position = context.position();
        let length = current_position.index - current_token_start.index;

        self.generate_token_with_position(context, token_type, current_token_start, length)
    }

    fn tokenize_whitespace(&mut self, context: &mut Context) -> PreprocessorToken {
        let mut current;
        let mut last_index = self.initial_processor.position().index;
        let mut last_column = self.initial_processor.position().column;
        loop {
            current = self.initial_processor.next_item(context);
            match current {
                | None => {
                    last_index = self.initial_processor.position().index;
                    last_column = self.initial_processor.position().column;
                    break;
                },
                | Some(i) if i.is_whitespace() && i != '\n' => {
                    last_index = self.initial_processor.position().index;
                    last_column = self.initial_processor.position().column;
                },
                | Some(_) => break,
            }
        }
        context.source.index = last_index;
        context.source.column = last_column;
        self.contents.clear();
        self.contents.push(' ');
        self.generate_token(context, PreprocessorTokenType::Whitespace)
    }

    fn tokenize_forward_slash(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_index = self.initial_processor.position().index;
        let last_column = self.initial_processor.position().column;
        let last_line = self.initial_processor.position().line;
        let current = self.initial_processor.next_item(context);
        match current {
            | Some('=') => {
                self.contents.push('=');
                self.generate_token(context, PreprocessorTokenType::ForwardSlashEquals)
            },
            | Some(_) | None => {
                context.source.index = last_index;
                context.source.column = last_column;
                context.source.line = last_line;
                self.generate_token(context, PreprocessorTokenType::ForwardSlash)
            },
        }
    }

    pub(crate) fn new() -> Self {
        Self {
            initial_processor:   InitialProcessor::new(),
            contents:            TokenString::new(),
            current_token_start: SourcePosition::default(),
        }
    }

    fn tokenize_hash(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_index = self.initial_processor.position().index;
        let last_column = self.initial_processor.position().column;
        let last_line = self.initial_processor.position().line;
        let current = self.initial_processor.next_item(context);
        match current {
            | Some('#') => {
                self.contents.push('#');
                self.generate_token(context, PreprocessorTokenType::HashHash)
            },
            | _ => {
                context.source.index = last_index;
                context.source.column = last_column;
                context.source.line = last_line;
                self.generate_token(context, PreprocessorTokenType::Hash)
            },
        }
    }

    fn tokenize_identifier(&mut self, context: &mut Context) -> PreprocessorToken {
        loop {
            let last_index = self.initial_processor.position().index;
            let last_column = self.initial_processor.position().column;
            let last_line = self.initial_processor.position().line;
            let current = self.initial_processor.next_item(context);
            match current {
                | Some(c) if c.is_alphanumeric() || c == '_' => self.contents.push(c),
                | _ => {
                    context.source.index = last_index;
                    context.source.column = last_column;
                    context.source.line = last_line;
                    break self.generate_token(context, PreprocessorTokenType::Identifier);
                },
            }
        }
    }

    fn tokenize_keyword_or_identifier(&mut self, context: &mut Context) -> PreprocessorToken {
        let mut res = self.tokenize_identifier(context);
        if context.string_cache.get(res.contents) == Some("defined") {
            res.kind = PreprocessorTokenType::Defined;
        }
        res
    }

    fn tokenize_wide_string_or_identifier(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_index = self.initial_processor.position().index;
        let last_column = self.initial_processor.position().column;
        let last_line = self.initial_processor.position().line;
        let current = self.initial_processor.next_item(context);
        match current {
            | Some('"') => self.tokenize_string(context),
            | Some('\'') => self.tokenize_char(context),
            | None | Some(Ok(_)) => {
                context.source.index = last_index;
                context.source.column = last_column;
                context.source.line = last_line;
                self.generate_token(context, PreprocessorTokenType::Identifier)
            },
        }
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn tokenize_string_like(
        &mut self,
        context: &mut Context,
        end_char: char,
        ignore_escapes: bool,
        return_token_type: PreprocessorTokenType,
        unterminated_error_type: PreprocessorTokenizerErrorType,
        newline_error_type: PreprocessorTokenizerErrorType,
    ) -> PreprocessorToken {
        loop {
            let last_index = self.initial_processor.position().index;
            let last_column = self.initial_processor.position().column;
            let last_line = self.initial_processor.position().line;
            let Some(current) = self.initial_processor.next_item(context) else {
                self.generate_error(context, unterminated_error_type);
                break;
            };
            if !ignore_escapes {
                let last_index = self.initial_processor.position().index;
                let last_column = self.initial_processor.position().column;
                let last_line = self.initial_processor.position().line;
                let next = self.initial_processor.next_item(context);
                if current == '\\' && next == Some(end_char) {
                    self.contents.push('\\');
                    self.contents.push(end_char);
                    continue;
                }
                context.source.index = last_index;
                context.source.column = last_column;
                context.source.line = last_line;
            }
            if current == end_char {
                self.contents.push(end_char);
                break;
            }
            if current == '\n' {
                self.contents.push(end_char);
                self.generate_error(context, newline_error_type);
                break;
            }
            self.contents.push(current);
        }

        self.generate_token(context, return_token_type)
    }

    fn tokenize_string(&mut self, context: &mut Context) -> PreprocessorToken {
        self.tokenize_string_like(
            context,
            '"',
            false,
            PreprocessorTokenType::String,
            PreprocessorTokenizerErrorType::UnterminatedString,
            PreprocessorTokenizerErrorType::NewlineInString,
        )
    }

    fn tokenize_char(&mut self, context: &mut Context) -> PreprocessorToken {
        self.tokenize_string_like(
            context,
            '\'',
            false,
            PreprocessorTokenType::Character,
            PreprocessorTokenizerErrorType::UnterminatedCharacter,
            PreprocessorTokenizerErrorType::NewlineInCharacter,
        )
    }

    fn tokenize_angle_bracket_string(
        &mut self,
        context: &mut Context,
    ) -> Option<PreprocessorToken> {
        loop {
            let Some(current) = self.initial_processor.next_item(context) else {
                return None;
            };
            self.contents.push(current);
            if current == '>' {
                break;
            }
            if current == '\n' {
                return None;
            }
        }
        Some(self.generate_token(context, PreprocessorTokenType::AngleBracketString))
    }

    fn tokenize_include_string(&mut self, context: &mut Context) -> PreprocessorToken {
        self.tokenize_string_like(
            context,
            '"',
            true,
            PreprocessorTokenType::IncludeString,
            PreprocessorTokenizerErrorType::UnterminatedIncludeString,
            PreprocessorTokenizerErrorType::NewlineInIncludeString,
        )
    }

    fn tokenize_period_or_number(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_index = self.initial_processor.position().index;
        let last_column = self.initial_processor.position().column;
        let last_line = self.initial_processor.position().line;
        match self.initial_processor.next_item(context) {
            | Some('.') => self.tokenize_ellipsis(context, last_index, last_colunn, last_line),
            | Some(v) if v.is_ascii_digit() => {
                self.contents.push(v);
                self.tokenize_number(context)
            },
            | _ => {
                context.source.index = last_index;
                context.source.column = last_column;
                context.source.line = last_line;
                self.generate_token(context, PreprocessorTokenType::Period)
            },
        }
    }

    #[inline(never)]
    #[cold]
    fn tokenize_ellipsis(
        &mut self,
        context: &mut Context,
        last_index: u32,
        last_column: NonZeroU32,
        last_line: u32,
    ) -> PreprocessorToken {
        match self.initial_processor.next_item(context) {
            | Some('.') => {
                self.contents.push_str("..");
                self.generate_token(context, PreprocessorTokenType::Ellipsis)
            },
            | None | Some(_) => {
                context.source.index = last_index;
                context.source.column = last_column;
                context.source.line = last_line;
                self.generate_token(context, PreprocessorTokenType::Period)
            },
        }
    }

    fn tokenize_number(&mut self, context: &mut Context) -> PreprocessorToken {
        loop {
            let last_index = self.initial_processor.position().index;
            let last_column = self.initial_processor.position().column;
            let last_line = self.initial_processor.position().line;
            let Some(current) = self.initial_processor.next_item(context) else {
                break;
            };
            if matches!(current, 'e' | 'E' | 'p' | 'P') {
                match self.initial_processor.next_item(context) {
                    | Some('+' | '-') => {
                        self.contents.push(current);
                        self.contents.push('+');
                        continue;
                    },
                    | None | Some(_) => {
                        self.generate_error(
                            context,
                            PreprocessorTokenizerErrorType::MissingSignInExponent,
                        );
                        context.source.index = last_index;
                        context.source.column = last_column;
                        context.source.line = last_line;
                        break;
                    },
                }
            }

            if current.is_alphanumeric() || current == '.' {
                self.contents.push(current);
                continue;
            }
            context.source.index = last_index;
            context.source.column = last_column;
            context.source.line = last_line;
            break;
        }
        self.generate_token(context, PreprocessorTokenType::Number)
    }

    fn tokenize_percent(&mut self, context: &mut Context) -> PreprocessorToken {
        match self.initial_processor.next_item(context) {
            | Some('=') => {
                self.contents.push('=');
                self.generate_token(context, PreprocessorTokenType::PercentEquals)
            },
            // Digraph.
            | Some('>') => {
                self.contents.push('>');
                self.generate_token(context, PreprocessorTokenType::ClosingCurlyBrace)
            },
            | Some(':') => self.tokenize_hash_digraph(context),
            | None | Some(_) => {
                context.source.index = last_index;
                context.source.column = last_column;
                context.source.line = last_line;
                self.generate_token(context, PreprocessorTokenType::Percent)
            },
        }
    }

    #[inline(never)]
    #[cold]
    fn tokenize_hash_digraph(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_index = self.initial_processor.position().index;
        let last_column = self.initial_processor.position().column;
        let last_line = self.initial_processor.position().line;
        match self.initial_processor.next_item(context) {
            | Some('%') =>
                self.tokenize_double_hash_digraph(context, last_index, last_column, last_line),
            | None | Some(_) => {
                context.source.index = last_index;
                context.source.column = last_column;
                context.source.line = last_line;
                self.generate_token(context, PreprocessorTokenType::Hash)
            },
        }
    }

    fn tokenize_double_hash_digraph(
        &mut self,
        context: &mut Context,
        last_index: u32,
        last_column: NonZeroU32,
        last_line: u32,
    ) -> PreprocessorToken {
        match self.initial_processor.next_item(context) {
            | Some(':') => {
                self.contents.push_str(":%:");
                self.generate_token(context, PreprocessorTokenType::HashHash)
            },
            | _ => {
                context.source.index = last_index;
                context.source.column = last_column;
                context.source.line = last_line;
                self.generate_token(context, PreprocessorTokenType::Hash)
            },
        }
    }

    fn tokenize_left_angle_bracket(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_index = self.initial_processor.position().index;
        let last_column = self.initial_processor.position().column;
        let last_line = self.initial_processor.position().line;

        match self.initial_processor.next_item(context) {
            | Some(':') => {
                self.contents.push(':');
                self.generate_token(context, PreprocessorTokenType::OpeningSquareBracket)
            },
            | Some('%') => {
                self.contents.push('%');
                self.generate_token(context, PreprocessorTokenType::OpeningCurlyBrace)
            },
            | Some('=') => {
                self.contents.push('=');
                self.generate_token(context, PreprocessorTokenType::LessThanEquals)
            },
            | Some('<') => {
                self.contents.push('<');
                self.tokenize_left_shift(context)
            },
            | None | Some(Ok(_)) => {
                context.source.index = last_index;
                context.source.column = last_column;
                context.source.line = last_line;
                self.generate_token(context, PreprocessorTokenType::LessThan)
            },
        }
    }

    fn tokenize_left_shift(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_index = self.initial_processor.position().index;
        let last_column = self.initial_processor.position().column;
        let last_line = self.initial_processor.position().line;

        match self.initial_processor.next_item(context) {
            | Some(Ok('=')) => {
                self.contents.push('=');
                self.generate_token(context, PreprocessorTokenType::LessThanLessThanEquals)
            },
            | None | Some(Ok(_)) => {
                context.source.index = last_index;
                context.source.column = last_column;
                context.source.line = last_line;
                self.generate_token(context, PreprocessorTokenType::LessThanLessThan)
            },
        }
    }

    fn tokenize_right_angle_bracket(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_index = self.initial_processor.position().index;
        let last_column = self.initial_processor.position().column;
        let last_line = self.initial_processor.position().line;

        match self.initial_processor.next_item(context) {
            | Some('>') => {
                self.contents.push('>');
                self.tokenize_right_shift(context)
            },
            | Some('=') => {
                self.contents.push('=');
                self.generate_token(context, PreprocessorTokenType::GreaterThanEquals)
            },
            | None | Some(_) => {
                context.source.index = last_index;
                context.source.column = last_column;
                context.source.line = last_line;
                self.generate_token(context, PreprocessorTokenType::GreaterThan)
            },
        }
    }

    fn tokenize_right_shift(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_index = self.initial_processor.position().index;
        let last_column = self.initial_processor.position().column;
        let last_line = self.initial_processor.position().line;

        match self.initial_processor.next_item(context) {
            | Some(Ok('=')) => {
                self.contents.push('=');
                self.generate_token(context, PreprocessorTokenType::GreaterThanGreaterThanEquals)
            },
            | None | Some(_) => {
                context.source.index = last_index;
                context.source.column = last_column;
                context.source.line = last_line;
                self.generate_token(context, PreprocessorTokenType::GreaterThanGreaterThan)
            },
        }
    }

    fn tokenize_colon(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_index = self.initial_processor.position().index;
        let last_column = self.initial_processor.position().column;
        let last_line = self.initial_processor.position().line;

        match self.initial_processor.next_item(context) {
            | Some('>') => {
                self.contents.push('>');
                self.generate_token(context, PreprocessorTokenType::ClosingSquareBracket)
            },
            | None | Some(_) => {
                context.source.index = last_index;
                context.source.column = last_column;
                context.source.line = last_line;
                self.generate_token(context, PreprocessorTokenType::Colon)
            },
        }
    }

    fn tokenize_plus(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_index = self.initial_processor.position().index;
        let last_column = self.initial_processor.position().column;
        let last_line = self.initial_processor.position().line;

        match self.initial_processor.next_item(context) {
            | Some('+') => {
                self.contents.push('+');
                self.generate_token(context, PreprocessorTokenType::PlusPlus)
            },
            | Some('=') => {
                self.contents.push('=');
                self.generate_token(context, PreprocessorTokenType::PlusEquals)
            },
            | None | Some(_) => {
                context.source.index = last_index;
                context.source.column = last_column;
                context.source.line = last_line;
                self.generate_token(context, PreprocessorTokenType::Plus)
            },
        }
    }

    fn tokenize_minus(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_index = self.initial_processor.position().index;
        let last_column = self.initial_processor.position().column;
        let last_line = self.initial_processor.position().line;

        match self.initial_processor.next_item(context) {
            | Some('-') => {
                self.contents.push('-');
                self.generate_token(context, PreprocessorTokenType::MinusMinus)
            },
            | Some('=') => {
                self.contents.push('=');
                self.generate_token(context, PreprocessorTokenType::MinusEquals)
            },
            | Some('>') => {
                self.contents.push('>');
                self.generate_token(context, PreprocessorTokenType::Arrow)
            },
            | None | Some(_) => {
                context.source.index = last_index;
                context.source.column = last_column;
                context.source.line = last_line;
                self.generate_token(context, PreprocessorTokenType::Minus)
            },
        }
    }

    fn tokenize_asterisk(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_index = self.initial_processor.position().index;
        let last_column = self.initial_processor.position().column;
        let last_line = self.initial_processor.position().line;

        match self.initial_processor.next_item(context) {
            | Some('=') => {
                self.contents.push('=');
                self.generate_token(context, PreprocessorTokenType::AsteriskEquals)
            },
            | None | Some(_) => {
                context.source.index = last_index;
                context.source.column = last_column;
                context.source.line = last_line;

                self.generate_token(context, PreprocessorTokenType::Asterisk)
            },
        }
    }

    fn tokenize_caret(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_index = self.initial_processor.position().index;
        let last_column = self.initial_processor.position().column;
        let last_line = self.initial_processor.position().line;

        match self.initial_processor.next_item(context) {
            | Some('=') => {
                self.contents.push('=');
                self.generate_token(context, PreprocessorTokenType::CaretEquals)
            },
            | None | Some(_) => {
                context.source.index = last_index;
                context.source.column = last_column;
                context.source.line = last_line;

                self.generate_token(context, PreprocessorTokenType::Caret)
            },
        }
    }

    fn tokenize_ampersand(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_index = self.initial_processor.position().index;
        let last_column = self.initial_processor.position().column;
        let last_line = self.initial_processor.position().line;

        match self.initial_processor.next_item(context) {
            | Some('&') => {
                self.contents.push('&');
                self.generate_token(context, PreprocessorTokenType::AmpersandAmpersand)
            },
            | Some('=') => {
                self.contents.push('=');
                self.generate_token(context, PreprocessorTokenType::AmpersandEquals)
            },
            | None | Some(_) => {
                context.source.index = last_index;
                context.source.column = last_column;
                context.source.line = last_line;

                self.generate_token(context, PreprocessorTokenType::Ampersand)
            },
        }
    }

    fn tokenize_pipe(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_index = self.initial_processor.position().index;
        let last_column = self.initial_processor.position().column;
        let last_line = self.initial_processor.position().line;

        match self.initial_processor.next_item(context) {
            | Some('|') => {
                self.contents.push('|');
                self.generate_token(context, PreprocessorTokenType::PipePipe)
            },
            | Some('=') => {
                self.contents.push('=');
                self.generate_token(context, PreprocessorTokenType::PipeEquals)
            },
            | None | Some(_) => {
                context.source.index = last_index;
                context.source.column = last_column;
                context.source.line = last_line;

                self.generate_token(context, PreprocessorTokenType::Pipe)
            },
        }
    }

    fn tokenize_exclamation_mark(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_index = self.initial_processor.position().index;
        let last_column = self.initial_processor.position().column;
        let last_line = self.initial_processor.position().line;

        match self.initial_processor.next_item(context) {
            | Some('=') => {
                self.contents.push('=');
                self.generate_token(context, PreprocessorTokenType::ExclamationMarkEquals)
            },
            | None | Some(_) => {
                context.source.index = last_index;
                context.source.column = last_column;
                context.source.line = last_line;
                self.generate_token(context, PreprocessorTokenType::ExclamationMark)
            },
        }
    }

    fn tokenize_equals(&mut self, context: &mut Context) -> PreprocessorToken {
        let last_index = self.initial_processor.position().index;
        let last_column = self.initial_processor.position().column;
        let last_line = self.initial_processor.position().line;

        match self.initial_processor.next_item(context) {
            | Some('=') => {
                self.contents.push('=');
                self.generate_token(context, PreprocessorTokenType::EqualsEquals)
            },
            | None | Some(_) => {
                context.source.index = last_index;
                context.source.column = last_column;
                context.source.line = last_line;

                self.generate_token(context, PreprocessorTokenType::Equals)
            },
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) enum PreprocessorTokenType {
    // Literals
    Identifier,
    Number,
    String,
    Character,

    // Expanded from hash operator
    GeneratedString,
    WideGeneratedString,

    // Generated when a macro argument generated no tokens
    Placeholder,

    // Include tokens.
    AngleBracketString,
    IncludeString,

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

impl Default for PreprocessorToken {
    fn default() -> Self {
        Self {
            kind:           PreprocessorTokenType::WideGeneratedString,
            source_vectors: SourceVectors {
                start_index: usize::MAX,
                length:      usize::MAX,
            },
            contents:       StringCacheId::from_usize(usize::MAX),
        }
    }
}
