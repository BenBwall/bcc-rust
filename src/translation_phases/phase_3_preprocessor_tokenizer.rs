use std::{
    fmt::Display,
    mem::replace,
};

use arrayvec::ArrayVec;
use smallstr::SmallString;
use thiserror::Error;

use super::{
    ErrorSeverity,
    GetPosition,
    GetSeverity,
    SavePoint as ISavePoint,
    SourcePosition,
    SourceVector,
    SourceVectors,
    TranslationPhase,
};
use crate::util::string_cache::{
    Id as StringCacheId,
    Interner,
};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PreprocessorTokenizer {
    state: State,
    current_token_start: SourcePosition,
    last_char: Option<char>,
    pending_tokens: StackStack<PreprocessorToken, 2>,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum State {
    Default,
    MiddleOfHashHash,
    MiddleOfIdentifier,
    MiddleOfWideStringOrIdentifier,
    MiddleOfNumberOrPeriod,
    MiddleOfNumber,
    MiddleOfExponent,
    MiddleOfCharacter,
    MiddleOfString,
    MiddleOfIncludeString,
    MiddleOfAngleBracketString,
    MiddleOfForwardSlash,
    MiddleOfPercent,
    MiddleOfHashDigraph,
    MiddleOfDoubleHashDigraph,
    MiddleOfEllipsis,
    MiddleOfColon,
    MiddleOfLeftAngleBracket,
    MiddleOfLeftShift,
    MiddleOfRightAngleBracket,
    MiddleOfRightShift,
    MiddleOfPlus,
    MiddleOfMinus,
    MiddleOfAsterisk,
    MiddleOfCaret,
    MiddleOfAmpersand,
    MiddleOfPipe,
    MiddleOfExclamationMark,
    MiddleOfEquals,
    MiddleOfLineComment,
    MiddleOfBlockComment,
    BeforeBlockCommentEnd,
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
            | Self::MissingSignInExponent => write!(
                f,
                "Exponent in floating point number is missing a sign"
            ),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PreprocessorTokenizerError {
    source_vector: SourceVector,
    error_type:     PreprocessorTokenizerErrorType,
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
    type Error = PreprocessorTokenizerError;
    type Input = char;
    type Yield = PreprocessorToken;

    fn next_item(
        &mut self,
        input: Self::Input,
        context: &mut super::Context,
    ) -> Result<Option<Self::Yield>, Self::Error> {
        if let Some(c) = self.last_char {
            match self.next(c, context) {
                | Ok(Some(c)) => {
                    self.last = Some(input);
                    return Ok(Some(c))
                },
                | Ok(None) => self.last = None,
                | Err(e) => {
                    self.last = Some(input);
                    return Err(e);
                },
            }
        }
        self.next(input, context)
    }

    fn eoi(&mut self, context: &mut super::Context) -> Result<Option<Self::Yield>, Self::Error> {
        if let Some(c) = self.last_char.take() {
            match self.next(c, context) {
                | Ok(Some(c)) => return Ok(Some(c)),
                | Ok(None) => (),
                | Err(e) => return Err(e),
            }
        }
        self.handle_eoi(context)
    }
}

impl<Prev> PreprocessorTokenizer<Prev, Prev::SavePoint>
where
    Prev: TranslationPhase<Yield = char>,
{
    fn handle_eoi(&mut self, context: &mut Context) -> Result<Option<Self::Yield>, Self::Error> {
        match self.state {
            State::Default => Ok(None),
            State::MiddleOfHashHash => self.generate_token(PreprocessorTokenType::Hash),
            State::MiddleOfIdentifier => self.generate_token(PreprocessorTokenType::Identifier),
            State::MiddleOfWideStringOrIdentifier => self.generate_token(PreprocessorTokenType::Identifier),
            State::MiddleOfNumberOrPeriod => self.generate_token(PreprocessorTokenType::Period),
            State::MiddleOfNumber => self.generate_token(PreprocessorTokenType::Number),
            State::MiddleOfExponent => self.generate_error(PreprocessorTokenizerErrorType::MissingSignInExponent),
            State::MiddleOfCharacter => self.generate_error(PreprocessorTokenizerErrorType::UnterminatedCharacter),
            State::MiddleOfString => self.generate_error(PreprocessorTokenizerErrorType::UnterminatedString),
            State::MiddleOfIncludeString => self.generate_error(PreprocessorTokenizerErrorType::UnterminatedIncludeString),
            State::MiddleOfAngleBracketString => self.generate_error(PreprocessorTokenizerErrorType::UnterminatedIncludeString),
            State::MiddleOfForwardSlash => self.generate_token(PreprocessorTokenType::ForwardSlash),
            State::MiddleOfPercent => self.generate_token(PreprocessorTokenType::Percent),
            State::MiddleOfHashDigraph => self.generate_token(PreprocessorTokenType::Hash),
            State::MiddleOfDoubleHashDigraph => {self.pending_tokens.self.}
        }
    }
    
    fn next(
        &mut self,
        input: char,
        context: &mut Context,
    ) -> Result<Option<Self::Yield>, Self::Error> {
        match self.state {
            | State::MiddleOfHashHash => self.tokenize_hash(input),
            | State::MiddleOfIdentifier => self.tokenize_identifier(input),
            | State::MiddleOfWideStringOrIdentifier => self.tokenize_wide_string_or_identifier(input),
            | State::MiddleOfNumberOrPeriod => self.tokenize_period_or_number(input),
            | State::MiddleOfNumber => self.tokenize_number(input),
            | State::MiddleOfExponent => self.tokenize_exponent_sign(input),
            | State::MiddleOfCharacter => self.tokenize_char(input),
            | State::MiddleOfString => self.tokenize_string(input),
            | State::MiddleOfIncludeString => self.tokenize_include_string(input),
            | State::MiddleOfAngleBracketString => return self.tokenize_angle_bracket_string(input),
            | State::MiddleOfForwardSlash => self.tokenize_or_step_over_forward_slash(input),
            | State::MiddleOfPercent => self.tokenize_percent(input),
            | State::MiddleOfHashDigraph => self.tokenize_hash_digraph(input),
            | State::MiddleOfLeftAngleBracket => self.tokenize_left_angle_bracket(input),
            | State::MiddleOfLeftShift => self.tokenize_left_shift(input),
            | State::MiddleOfRightAngleBracket => self.tokenize_right_angle_bracket(input),
            | State::MiddleOfRightShift => self.tokenize_right_shift(input),
            | State::MiddleOfDoubleHashDigraph =>
                self.tokenize_double_hash_digraph(input),
            | State::MiddleOfEllipsis => self.tokenize_ellipsis(input),
            | State::MiddleOfColon => self.tokenize_colon(input),
            | State::MiddleOfPlus => self.tokenize_plus(input),
            | State::MiddleOfMinus => self.tokenize_minus(input),
            | State::MiddleOfAsterisk => self.tokenize_asterisk(input),
            | State::MiddleOfCaret => self.tokenize_caret(input),
            | State::MiddleOfAmpersand => self.tokenize_ampersand(input),
            | State::MiddleOfPipe => self.tokenize_pipe(input),
            | State::MiddleOfExclamationMark => self.tokenize_exclamation_mark(input),
            | State::MiddleOfEquals => self.tokenize_equals(input),
            | State::MiddleOfLineComment => self.step_over_line_comment(input),
            | State::MiddleOfBlockComment => self.step_over_block_comment(input),
            | State::BeforeBlockCommentEnd => self.step_over_block_comment_end(input),
            | State::Default => self.tokenize(input),
        }
    }
    fn error_from_prev(
        &self,
        error: Prev::Error,
    ) -> Result<<Self as TranslationPhase>::Yield, <Self as TranslationPhase>::Error> {
        let _ = self;
        Err(PreprocessorTokenizerError::ErrorFromPrev(error))
    }

    fn generate_error(
        &mut self,
        error_type: PreprocessorTokenizerErrorType,
    ) -> Result<Option<PreprocessorToken>, PreprocessorTokenizerError> {
        let current_save = self.previous_phase.save();
        self.previous_phase
            .restore(self.current_token_start.clone());
        let mut contents: SmallString<[u8; 1024]> = SmallString::new();
        while self.previous_phase.current_position().index < current_save.current_position().index {
            match self.previous_phase.next() {
                | Some(Ok(c)) => contents.push(c),
                | Some(Err(_)) => continue,
                | None => break,
            }
        }
        self.previous_phase.restore(current_save);
        let start_position = self.current_token_start.current_position();

        Err(PreprocessorTokenizerError::TokenizerError(
            InnerPreprocessorTokenizerError {
                start_position,
                length: self.current_position().index - start_position.index,
                error_type,
                contents: self.string_cache.intern(contents.as_str()),
            },
        ))
    }

    #[allow(clippy::unnecessary_wraps)]
    fn generate_token(
        &mut self,
        token_type: PreprocessorTokenType,
    ) -> Result<Option<PreprocessorToken>, PreprocessorTokenizerError> {
        let current_save = self.previous_phase.save();
        self.previous_phase
            .restore(self.current_token_start.clone());
        let mut contents: SmallString<[u8; 1024]> = SmallString::new();
        while self.previous_phase.current_position().index < current_save.current_position().index {
            match self.previous_phase.next() {
                | Some(Ok(c)) => contents.push(c),
                | Some(Err(_)) => continue,
                | None => break,
            }
        }
        self.previous_phase.restore(current_save);
        let start_position = self.current_token_start.current_position();

        Ok(Some(PreprocessorToken {
            source_vectors: SourceVector {
                position: start_position,
                length:   self.current_position().index - start_position.index,
            }
            .into(),
            kind:           token_type,
            contents:       self.string_cache.intern(contents.as_str()),
        }))
    }

    fn step_over_whitespace(&mut self) -> ParserResult<Prev::Error> {
        let mut save_point = self.save();
        while self
            .previous_phase
            .next()
            .is_some_and(|r| r.is_ok_and(|c| c.is_whitespace() && c != '\n'))
        {
            save_point = self.save();
        }
        self.restore(save_point);
        let mut res = self.generate_token(PreprocessorTokenType::Whitespace);
        let Ok(ref mut token) = res else {
            unreachable!()
        };
        token.contents = self.string_cache.intern(" ");
        res
    }

    fn tokenize_or_step_over_forward_slash(&mut self) -> ParserResult<Prev::Error> {
        let current = self.previous_phase.next();
        match current {
            | Some(Err(e)) => {
                self.state = State::MiddleOfForwardSlash;
                self.error_from_prev(e)
            },
            | Some(Ok('/')) => self.step_over_line_comment(),
            | Some(Ok('*')) => self.step_over_block_comment(),
            | Some(Ok('=')) => self.generate_token(PreprocessorTokenType::ForwardSlashEquals),
            | Some(Ok(_)) | None => {
                self.state = State::Default;
                self.generate_token(PreprocessorTokenType::ForwardSlash)
            },
        }
    }

    fn step_over_line_comment(&mut self) -> ParserResult<Prev::Error> {
        let mut save_point = self.save();

        loop {
            match self.previous_phase.next() {
                | None => {
                    save_point = self.save();
                    break;
                },
                | Some(Err(e)) => {
                    self.state = State::MiddleOfLineComment;
                    return self.error_from_prev(e);
                },
                | Some(Ok('\n')) => break,
                | Some(Ok(_)) => {
                    save_point = self.save();
                    continue;
                },
            }
        }

        self.restore(save_point);
        let mut res = self.generate_token(PreprocessorTokenType::Whitespace);
        let Ok(ref mut token) = res else {
            unreachable!()
        };
        token.contents = self.string_cache.intern(" ");
        res
    }

    fn step_over_block_comment(&mut self) -> ParserResult<Prev::Error> {
        let mut save_point = self.save();

        loop {
            match self.previous_phase.next() {
                | None => break,
                | Some(Err(e)) => {
                    self.state = State::MiddleOfBlockComment;
                    return self.error_from_prev(e);
                },
                | Some(Ok('*')) => return self.step_over_block_comment_end(),
                | Some(Ok(_)) => {
                    save_point = self.save();
                    continue;
                },
            }
        }
        self.restore(save_point);
        let mut res = self.generate_token(PreprocessorTokenType::Whitespace);
        let Ok(ref mut token) = res else {
            unreachable!()
        };
        token.contents = self.string_cache.intern(" ");
        res
    }

    fn step_over_block_comment_end(&mut self) -> ParserResult<Prev::Error> {
        let mut save_point = self.save();

        loop {
            match self.previous_phase.next() {
                | None => break,
                | Some(Err(e)) => {
                    self.state = State::BeforeBlockCommentEnd;
                    return self.error_from_prev(e);
                },
                | Some(Ok('/')) => {
                    save_point = self.save();
                    break;
                },
                | Some(Ok(_)) => {
                    save_point = self.save();
                    continue;
                },
            }
        }
        self.restore(save_point);
        let mut res = self.generate_token(PreprocessorTokenType::Whitespace);
        let Ok(ref mut token) = res else {
            unreachable!()
        };
        token.contents = self.string_cache.intern(" ");
        res
    }

    pub(crate) fn new(previous_phase: Prev, string_cache: StringCache) -> Self {
        Self {
            current_token_start: previous_phase.save(),
            previous_phase,
            state: State::Default,
            string_cache,
            is_tokenizing_include_string: false,
        }
    }

    fn tokenize(&mut self) -> ParserReturn<Prev::Error> {
        self.current_token_start = self.previous_phase.save();
        let result = self.previous_phase.next()?;

        let result = match result {
            | Ok(c) => c,
            | Err(e) => return Some(self.error_from_prev(e)),
        };
        Some(match result {
            | '\n' => self.generate_token(PreprocessorTokenType::Newline),
            | '0'..='9' => self.tokenize_number(),
            | '{' => self.generate_token(PreprocessorTokenType::OpeningCurlyBrace),
            | '}' => self.generate_token(PreprocessorTokenType::ClosingCurlyBrace),
            | '(' => self.generate_token(PreprocessorTokenType::OpeningParenthesis),
            | ')' => self.generate_token(PreprocessorTokenType::ClosingParenthesis),
            | '[' => self.generate_token(PreprocessorTokenType::OpeningSquareBracket),
            | ']' => self.generate_token(PreprocessorTokenType::ClosingSquareBracket),
            | 'L' => self.tokenize_wide_string_or_identifier(),
            | c if c.is_alphabetic() || c == '_' => self.tokenize_keyword_or_identifier(),
            | '.' => self.tokenize_period_or_number(),
            | '"' =>
                if self.is_tokenizing_include_string {
                    self.tokenize_include_string()
                } else {
                    self.tokenize_string()
                },
            | '\'' => self.tokenize_char(),
            | '#' => self.tokenize_hash(),
            | ' ' => self.step_over_whitespace(),
            | '/' => self.tokenize_or_step_over_forward_slash(),
            | '%' => self.tokenize_percent(),
            | '<' =>
                if self.is_tokenizing_include_string {
                    return self.tokenize_angle_bracket_string();
                } else {
                    self.tokenize_left_angle_bracket()
                },
            | '>' => self.tokenize_right_angle_bracket(),
            | ',' => self.generate_token(PreprocessorTokenType::Comma),
            | ';' => self.generate_token(PreprocessorTokenType::SemiColon),
            | '?' => self.generate_token(PreprocessorTokenType::QuestionMark),
            | '~' => self.generate_token(PreprocessorTokenType::Tilde),
            | ':' => self.tokenize_colon(),
            | '+' => self.tokenize_plus(),
            | '-' => self.tokenize_minus(),
            | '*' => self.tokenize_asterisk(),
            | '^' => self.tokenize_caret(),
            | '&' => self.tokenize_ampersand(),
            | '|' => self.tokenize_pipe(),
            | '!' => self.tokenize_exclamation_mark(),
            | '=' => self.tokenize_equals(),
            | _ => self.generate_error(PreprocessorTokenizerErrorType::UnknownToken),
        })
    }

    fn tokenize_hash(&mut self) -> ParserResult<Prev::Error> {
        let save_point = self.save();
        match self.previous_phase.next() {
            | Some(Err(e)) => {
                self.state = State::MiddleOfHashHash;
                self.error_from_prev(e)Interner
            },
            | Some(Ok('#')) => self.generate_token(PreprocessorTokenType::HashHash),
            | None | Some(Ok(_)) => {
                self.restore(save_point);
                self.state = State::Default;
                self.generate_token(PreprocessorTokenType::Hash)
            },
        }
    }

    fn tokenize_identifier(&mut self) -> ParserResult<Prev::Error> {
        let mut save_point = self.save();

        loop {
            break match self.previous_phase.next() {
                | Some(Err(e)) => {
                    self.state = State::MiddleOfIdentifier;
                    self.error_from_prev(e)
                },
                | Some(Ok(c)) if c.is_alphanumeric() || c == '_' => {
                    save_point = self.save();
                    continue;
                },
                | None | Some(Ok(_)) => {
                    self.restore(save_point);
                    self.state = State::Default;
                    self.generate_token(PreprocessorTokenType::Identifier)
                },
            };
        }
    }

    fn tokenize_keyword_or_identifier(&mut self) -> ParserResult<Prev::Error> {
        let mut res = self.tokenize_identifier()?;
        if self.string_cache.get(res.contents) == Some("defined") {
            res.kind = PreprocessorTokenType::Defined;
        }
        Ok(res)
    }

    fn tokenize_wide_string_or_identifier(&mut self) -> ParserResult<Prev::Error> {
        let save_point = self.save();
        match self.previous_phase.next() {
            | Some(Err(e)) => {
                self.state = State::MiddleOfWideStringOrIdentifier;
                self.error_from_prev(e)
            },
            | Some(Ok('"')) => self.tokenize_string(),
            | Some(Ok('\'')) => self.tokenize_char(),
            | None | Some(Ok(_)) => {
                self.restore(save_point);
                self.state = State::Default;
                self.tokenize_keyword_or_identifier()
            },
        }
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn tokenize_string_like(
        &mut self,
        end_char: char,
        ignore_escapes: bool,
        return_token_type: PreprocessorTokenType,
        unterminated_error_type: PreprocessorTokenizerErrorType,
        newline_error_type: PreprocessorTokenizerErrorType,
        middle_of_state: State<Prev::SavePoint>,
    ) -> ParserResult<Prev::Error> {
        loop {
            let current = match self.previous_phase.next() {
                | None => return self.generate_error(unterminated_error_type),
                | Some(Err(e)) => {
                    self.state = middle_of_state;
                    return self.error_from_prev(e);
                },
                | Some(Ok(current)) => current,
            };
            if !ignore_escapes {
                let save_point = self.save();
                let next = self.previous_phase.next();
                if current == '\\' && matches!(next, Some(Ok(v)) if v == end_char) {
                    continue;
                }
                self.restore(save_point.clone());
            }
            if current == end_char {
                break;
            }
            if current == '\n' {
                self.state = middle_of_state;
                return self.generate_error(newline_error_type);
            }
        }

        self.generate_token(return_token_type)
    }

    fn tokenize_string(&mut self) -> ParserResult<Prev::Error> {
        self.tokenize_string_like(
            '"',
            false,
            PreprocessorTokenType::String,
            PreprocessorTokenizerErrorType::UnterminatedString,
            PreprocessorTokenizerErrorType::NewlineInString,
            State::MiddleOfString,
        )
    }

    fn tokenize_char(&mut self) -> ParserResult<Prev::Error> {
        self.tokenize_string_like(
            '\'',
            false,
            PreprocessorTokenType::Character,
            PreprocessorTokenizerErrorType::UnterminatedCharacter,
            PreprocessorTokenizerErrorType::NewlineInCharacter,
            State::MiddleOfCharacter,
        )
    }

    fn tokenize_angle_bracket_string(&mut self) -> Option<ParserResult<Prev::Error>> {
        loop {
            let current = match self.previous_phase.next() {
                | None => {
                    self.is_tokenizing_include_string = false;
                    return self.tokenize();
                },
                | Some(Err(e)) => {
                    self.state = State::MiddleOfAngleBracketString;
                    return Some(self.error_from_prev(e));
                },
                | Some(Ok(current)) => current,
            };
            if current == '>' {
                break;
            }
            if current == '\n' {
                self.is_tokenizing_include_string = false;
                return self.tokenize();
            }
        }
        Some(self.generate_token(PreprocessorTokenType::AngleBracketString))
    }

    fn tokenize_include_string(&mut self) -> ParserResult<Prev::Error> {
        self.tokenize_string_like(
            '"',
            true,
            PreprocessorTokenType::IncludeString,
            PreprocessorTokenizerErrorType::UnterminatedIncludeString,
            PreprocessorTokenizerErrorType::NewlineInIncludeString,
            State::MiddleOfIncludeString,
        )
    }

    fn tokenize_period_or_number(&mut self) -> ParserResult<Prev::Error> {
        let save_point = self.previous_phase.save();
        match self.previous_phase.next() {
            | Some(Err(e)) => {
                self.state = State::MiddleOfNumberOrPeriod;
                self.error_from_prev(e)
            },
            | Some(Ok('.')) => self.tokenize_ellipsis(save_point),
            | Some(Ok(v)) if v.is_ascii_digit() => self.tokenize_number(),
            | None | Some(Ok(_)) => {
                self.previous_phase.restore(save_point);
                self.state = State::Default;
                self.generate_token(PreprocessorTokenType::Period)
            },
        }
    }

    fn tokenize_ellipsis(
        &mut self,
        end_of_first_period: Prev::SavePoint,
    ) -> ParserResult<Prev::Error> {
        match self.previous_phase.next() {
            | Some(Err(e)) => {
                self.state = State::MiddleOfEllipsis {
                    end_of_first_period: Box::new(end_of_first_period),
                };
                self.error_from_prev(e)
            },
            | Some(Ok('.')) => self.generate_token(PreprocessorTokenType::Ellipsis),
            | None | Some(Ok(_)) => {
                self.previous_phase.restore(end_of_first_period);
                self.state = State::Default;
                self.generate_token(PreprocessorTokenType::Period)
            },
        }
    }

    fn tokenize_exponent_sign(&mut self) -> ParserResult<Prev::Error> {
        let save_point = self.save();
        match self.previous_phase.next() {
            | Some(Err(e)) => {
                self.state = State::MiddleOfExponent;
                self.error_from_prev(e)
            },
            | Some(Ok(v)) if v == '+' || v == '-' => self.tokenize_number(),
            | None | Some(Ok(_)) => {
                self.state = State::Default;
                self.restore(save_point);
                self.tokenize_number()
            },
        }
    }

    fn tokenize_number(&mut self) -> ParserResult<Prev::Error> {
        let mut save_point = self.save();

        loop {
            let current = self.previous_phase.next();
            let current = match current {
                | Some(Err(e)) => {
                    self.state = State::MiddleOfNumber;
                    return self.error_from_prev(e);
                },
                | None => {
                    self.restore(save_point);
                    self.state = State::Default;
                    return self.generate_token(PreprocessorTokenType::Number);
                },
                | Some(Ok(v)) => v,
            };
            let save_point2 = self.save();

            if matches!(current, 'e' | 'E' | 'p' | 'P') {
                match self.previous_phase.next() {
                    | None => return self.generate_token(PreprocessorTokenType::Number),
                    | Some(Err(e)) => {
                        self.state = State::MiddleOfExponent;
                        return self.error_from_prev(e);
                    },
                    | Some(Ok(next)) =>
                        if matches!(next, '+' | '-') {
                            save_point = self.save();
                            continue;
                        },
                }
            }

            self.restore(save_point2);

            if current.is_alphanumeric() || current == '.' {
                save_point = self.save();
                continue;
            }
            self.restore(save_point);

            break;
        }
        self.generate_token(PreprocessorTokenType::Number)
    }

    fn tokenize_percent(&mut self) -> ParserResult<Prev::Error> {
        let save_point = self.save();
        match self.previous_phase.next() {
            | Some(Err(e)) => {
                self.state = State::MiddleOfPercent;
                self.error_from_prev(e)
            },
            | Some(Ok('=')) => self.generate_token(PreprocessorTokenType::PercentEquals),
            | Some(Ok('>')) => self.generate_token(PreprocessorTokenType::ClosingCurlyBrace),
            | Some(Ok(':')) => self.tokenize_hash_digraph(),
            | Some(Ok(_)) | None => {
                self.restore(save_point);
                self.state = State::Default;
                self.generate_token(PreprocessorTokenType::Percent)
            },
        }
    }

    fn tokenize_hash_digraph(&mut self) -> ParserResult<Prev::Error> {
        let save_point = self.previous_phase.save();
        match self.previous_phase.next() {
            | Some(Err(e)) => {
                self.state = State::MiddleOfHashDigraph;
                self.error_from_prev(e)
            },
            | Some(Ok('%')) => self.tokenize_double_hash_digraph(save_point),
            | None | Some(Ok(_)) => {
                self.previous_phase.restore(save_point);
                self.state = State::Default;
                self.generate_token(PreprocessorTokenType::Hash)
            },
        }
    }

    fn tokenize_double_hash_digraph(
        &mut self,
        end_of_first_half: Prev::SavePoint,
    ) -> ParserResult<Prev::Error> {
        match self.previous_phase.next() {
            | Some(Err(e)) => {
                self.state = State::MiddleOfDoubleHashDigraph;
                self.error_from_prev(e)
            },
            | Some(Ok(':')) => self.generate_token(PreprocessorTokenType::HashHash),
            | None | Some(Ok(_)) => {
                self.previous_phase.restore(end_of_first_half);
                self.state = State::Default;
                self.generate_token(PreprocessorTokenType::Hash)
            },
        }
    }

    fn tokenize_left_angle_bracket(&mut self) -> ParserResult<Prev::Error> {
        let save_point = self.save();
        match self.previous_phase.next() {
            | Some(Err(e)) => {
                self.state = State::MiddleOfLeftAngleBracket;
                self.error_from_prev(e)
            },
            | Some(Ok(':')) => self.generate_token(PreprocessorTokenType::OpeningSquareBracket),
            | Some(Ok('%')) => self.generate_token(PreprocessorTokenType::OpeningCurlyBrace),
            | Some(Ok('=')) => self.generate_token(PreprocessorTokenType::LessThanEquals),
            | Some(Ok('<')) => self.tokenize_left_shift(),
            | None | Some(Ok(_)) => {
                self.restore(save_point);
                self.state = State::Default;
                self.generate_token(PreprocessorTokenType::LessThan)
            },
        }
    }

    fn tokenize_left_shift(&mut self) -> ParserResult<Prev::Error> {
        let save_point = self.save();
        match self.previous_phase.next() {
            | Some(Err(e)) => {
                self.state = State::MiddleOfLeftShift;
                self.error_from_prev(e)
            },
            | Some(Ok('=')) => self.generate_token(PreprocessorTokenType::LessThanLessThanEquals),
            | None | Some(Ok(_)) => {
                self.restore(save_point);
                self.state = State::Default;
                self.generate_token(PreprocessorTokenType::LessThanLessThan)
            },
        }
    }

    fn tokenize_right_angle_bracket(&mut self) -> ParserResult<Prev::Error> {
        let save_point = self.save();
        match self.previous_phase.next() {
            | Some(Err(e)) => {
                self.state = State::MiddleOfRightAngleBracket;
                self.error_from_prev(e)
            },
            | Some(Ok('>')) => self.tokenize_right_shift(),
            | Some(Ok('=')) => self.generate_token(PreprocessorTokenType::GreaterThanEquals),
            | None | Some(Ok(_)) => {
                self.restore(save_point);
                self.state = State::Default;
                self.generate_token(PreprocessorTokenType::GreaterThan)
            },
        }
    }

    fn tokenize_right_shift(&mut self) -> ParserResult<Prev::Error> {
        let save_point = self.save();
        match self.previous_phase.next() {
            | Some(Err(e)) => {
                self.state = State::MiddleOfRightShift;
                self.error_from_prev(e)
            },
            | Some(Ok('=')) =>
                self.generate_token(PreprocessorTokenType::GreaterThanGreaterThanEquals),
            | None | Some(Ok(_)) => {
                self.restore(save_point);
                self.state = State::Default;
                self.generate_token(PreprocessorTokenType::GreaterThanGreaterThan)
            },
        }
    }

    fn tokenize_colon(&mut self) -> ParserResult<Prev::Error> {
        let save_point = self.save();
        match self.previous_phase.next() {
            | Some(Err(e)) => {
                self.state = State::MiddleOfColon;
                self.error_from_prev(e)
            },
            | Some(Ok('>')) => self.generate_token(PreprocessorTokenType::ClosingSquareBracket),
            | None | Some(Ok(_)) => {
                self.restore(save_point);
                self.state = State::Default;
                self.generate_token(PreprocessorTokenType::Colon)
            },
        }
    }

    fn tokenize_plus(&mut self) -> ParserResult<Prev::Error> {
        let save_point = self.save();
        match self.previous_phase.next() {
            | Some(Err(e)) => {
                self.state = State::MiddleOfPlus;
                self.error_from_prev(e)
            },
            | Some(Ok('+')) => self.generate_token(PreprocessorTokenType::PlusPlus),
            | Some(Ok('=')) => self.generate_token(PreprocessorTokenType::PlusEquals),
            | None | Some(Ok(_)) => {
                self.restore(save_point);
                self.state = State::Default;
                self.generate_token(PreprocessorTokenType::Plus)
            },
        }
    }

    fn tokenize_minus(&mut self) -> ParserResult<Prev::Error> {
        let save_point = self.save();
        match self.previous_phase.next() {
            | Some(Err(e)) => {
                self.state = State::MiddleOfMinus;
                self.error_from_prev(e)
            },
            | Some(Ok('-')) => self.generate_token(PreprocessorTokenType::MinusMinus),
            | Some(Ok('=')) => self.generate_token(PreprocessorTokenType::MinusEquals),
            | Some(Ok('>')) => self.generate_token(PreprocessorTokenType::Arrow),
            | None | Some(Ok(_)) => {
                self.restore(save_point);
                self.state = State::Default;
                self.generate_token(PreprocessorTokenType::Minus)
            },
        }
    }

    fn tokenize_asterisk(&mut self) -> ParserResult<Prev::Error> {
        let save_point = self.save();
        match self.previous_phase.next() {
            | Some(Err(e)) => {
                self.state = State::MiddleOfAsterisk;
                self.error_from_prev(e)
            },
            | Some(Ok('=')) => self.generate_token(PreprocessorTokenType::AsteriskEquals),
            | None | Some(Ok(_)) => {
                self.restore(save_point);
                self.state = State::Default;
                self.generate_token(PreprocessorTokenType::Asterisk)
            },
        }
    }

    fn tokenize_caret(&mut self) -> ParserResult<Prev::Error> {
        let save_point = self.save();
        match self.previous_phase.next() {
            | Some(Err(e)) => {
                self.state = State::MiddleOfCaret;
                self.error_from_prev(e)
            },
            | Some(Ok('=')) => self.generate_token(PreprocessorTokenType::CaretEquals),
            | None | Some(Ok(_)) => {
                self.restore(save_point);
                self.state = State::Default;
                self.generate_token(PreprocessorTokenType::Caret)
            },
        }
    }

    fn tokenize_ampersand(&mut self) -> ParserResult<Prev::Error> {
        let save_point = self.save();
        match self.previous_phase.next() {
            | Some(Err(e)) => {
                self.state = State::MiddleOfAmpersand;
                self.error_from_prev(e)
            },
            | Some(Ok('&')) => self.generate_token(PreprocessorTokenType::AmpersandAmpersand),
            | Some(Ok('=')) => self.generate_token(PreprocessorTokenType::AmpersandEquals),
            | None | Some(Ok(_)) => {
                self.restore(save_point);
                self.state = State::Default;
                self.generate_token(PreprocessorTokenType::Ampersand)
            },
        }
    }

    fn tokenize_pipe(&mut self) -> ParserResult<Prev::Error> {
        let save_point = self.save();
        match self.previous_phase.next() {
            | Some(Err(e)) => {
                self.state = State::MiddleOfPipe;
                self.error_from_prev(e)
            },
            | Some(Ok('|')) => self.generate_token(PreprocessorTokenType::PipePipe),
            | Some(Ok('=')) => self.generate_token(PreprocessorTokenType::PipeEquals),
            | None | Some(Ok(_)) => {
                self.restore(save_point);
                self.state = State::Default;
                self.generate_token(PreprocessorTokenType::Pipe)
            },
        }
    }

    fn tokenize_exclamation_mark(&mut self) -> ParserResult<Prev::Error> {
        let save_point = self.save();
        match self.previous_phase.next() {
            | Some(Err(e)) => {
                self.state = State::MiddleOfExclamationMark;
                self.error_from_prev(e)
            },
            | Some(Ok('=')) => self.generate_token(PreprocessorTokenType::ExclamationMarkEquals),
            | None | Some(Ok(_)) => {
                self.restore(save_point);
                self.state = State::Default;
                self.generate_token(PreprocessorTokenType::ExclamationMark)
            },
        }
    }

    fn tokenize_equals(&mut self) -> ParserResult<Prev::Error> {
        let save_point = self.save();
        match self.previous_phase.next() {
            | Some(Err(e)) => {
                self.state = State::MiddleOfEquals;
                self.error_from_prev(e)
            },
            | Some(Ok('=')) => self.generate_token(PreprocessorTokenType::EqualsEquals),
            | None | Some(Ok(_)) => {
                self.restore(save_point);
                self.state = State::Default;
                self.generate_token(PreprocessorTokenType::Equals)
            },
        }
    }
}

type ParserReturn<PrevError> = Option<ParserResult<PrevError>>;
type ParserResult<PrevError> = Result<PreprocessorToken, PreprocessorTokenizerError<PrevError>>;

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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
                length: usize::MAX,
            },
            contents:       StringCacheId::from_usize(usize::MAX),
        }
    }

}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;
    use crate::{
        translation_phases::{
            phase_0_newline_tracking::NewlineTracking,
            phase_1_map_character_sets::{
                MapCharacterSets,
                MapCharacterSetsError,
            },
            phase_2_remove_escaped_newlines::{
                MissingNewlineError,
                RemoveEscapedNewlines,
                RemoveEscapedNewlinesError,
            },
        },
        util::string_cache::{
            Id as StringCacheId,
            StringCache,
        },
    };

    #[rstest]
    #[case("", vec![
        Err(PreprocessorTokenizerError::ErrorFromPrev(RemoveEscapedNewlinesError::MissingFinalNewLine(MissingNewlineError(SourcePosition {
            index:       0,
            line:        1,
            column:      1,
            source_file: StringCacheId::from(0),
        })))),
        Ok(PreprocessorToken {
            kind: PreprocessorTokenType::Newline,
            source_vectors: SourceVectors::from(SourceVector {
                position: SourcePosition {
                    index: 0,
                    line: 1,
                    column: 1,
                    source_file: StringCacheId::from(0),
                },
                length: 0,
            }),
            contents: StringCacheId::from(1),

        })
    ])]
    #[case("1", vec![
        Err(PreprocessorTokenizerError::ErrorFromPrev(RemoveEscapedNewlinesError::MissingFinalNewLine(MissingNewlineError(SourcePosition {
            index:       1,
            line:        1,
            column:      2,
            source_file: StringCacheId::from(0),
        })))),
        Ok(PreprocessorToken {
            kind: PreprocessorTokenType::Number,
            source_vectors: SourceVectors::from(SourceVector {
            position: SourcePosition {
                index: 0,
                line: 1,
                column: 1,
                source_file: StringCacheId::from(0),
            },
            length: 1,
        }),
            contents: StringCacheId::from(1),
        }),
        Ok(PreprocessorToken {
            kind: PreprocessorTokenType::Newline,
            source_vectors: SourceVectors::from(SourceVector {
            position: SourcePosition {
                index: 1,
                line: 1,
                column: 2,
                source_file: StringCacheId::from(0),
            },
            length: 0,
        }),
            contents: StringCacheId::from(2),
        }),
    ])]
    #[case("int x = @1;\n", vec![
        Ok(PreprocessorToken {
            kind: PreprocessorTokenType::Identifier,
            source_vectors: SourceVectors::from(SourceVector {
            position: SourcePosition {
                index: 0,
                line: 1,
                column: 1,
                source_file: StringCacheId::from(0),
            Interner
            length: 3,
        }),
            contents: StringCacheId::from(1),
        }),
        Ok(PreprocessorToken {
            kind: PreprocessorTokenType::Identifier,
            source_vectors: SourceVectors::from(SourceVector {
            position: SourcePosition {
                index: 4,
                line: 1,
                column: 5,
                source_file: StringCacheId::from(0),
            },
            length: 1,
        }),
            contents: StringCacheId::from(2),
        }),
        Ok(PreprocessorToken {
            kind: PreprocessorTokenType::Equals,
            source_vectors: SourceVectors::from(SourceVector {
            position: SourcePosition {
                index: 6,
                line: 1,
                column: 7,
                source_file: StringCacheId::from(0),
            },

            length: 1,}),
            contents: StringCacheId::from(3),
        }),
        Err(PreprocessorTokenizerError::TokenizerError(InnerPreprocessorTokenizerError {
            start_position: SourcePosition {
                index: 8,
                line: 1,
                column: 9,
                source_file: StringCacheId::from(0),
            },
            length: 1,
            error_type: PreprocessorTokenizerErrorType::UnknownToken,
            contents: StringCacheId::from(4),
        })),
        Ok(PreprocessorToken {
            kind: PreprocessorTokenType::Number,
            source_vectors: SourceVectors::from(SourceVector {
            position: SourcePosition {
                index: 9,
                line: 1,
                column: 10,
                source_file: StringCacheId::from(0),
            },
            length: 1,
        }),
            contents: StringCacheId::from(5),
        }),
        Ok(PreprocessorToken {
            kind: PreprocessorTokenType::SemiColon,
            source_vectors: SourceVectors::from(SourceVector {
            position: SourcePosition {
                index: 10,
                line: 1,
                column: 11,
                source_file: StringCacheId::from(0),
            },
            length: 1,
        }),
            contents: StringCacheId::from(6),
        }),
        Ok(PreprocessorToken {
            kind: PreprocessorTokenType::Newline,
            source_vectors: SourceVectors::from(SourceVector {
            position: SourcePosition {
                index: 11,
                line: 1,
                column: 12,
                source_file: StringCacheId::from(0),
            },

            length: 1,}),
            contents: StringCacheId::from(7),
        }),
    ])]
    #[case("#define MAX(A, B) A > B\\\n    ? A\\\n    : B",
        vec![
            Ok(PreprocessorToken {
                kind: PreprocessorTokenType::Hash,
                source_vectors: SourceVectors::from(SourceVector {
                position: SourcePosition {
                    index: 0,
                    line: 1,
                    column: 1,
                    source_file: StringCacheId::from(0),
                },
                length: 1,
            }),
                contents: StringCacheId::from(1),
            }),
            Ok(PreprocessorToken {
                kind: PreprocessorTokenType::Identifier,
                source_vectors: SourceVectors::from(SourceVector {
                position: SourcePosition {
                    index: 1,
                    line: 1,
                    column: 2,
                    source_file: StringCacheId::from(0),
                },
                length: 6,
            }),
                contents: StringCacheId::from(2),
            }),
            Ok(PreprocessorToken {
                kind: PreprocessorTokenType::Identifier,
                source_vectors: SourceVectors::from(SourceVector {
                position: SourcePosition {
                    index: 8,
                    line: 1,
                    column: 9,
                    source_file: StringCacheId::from(0),
                },
                length: 3,
            }),
                contents: StringCacheId::from(3),
            }),
            Ok(PreprocessorToken {
                kind: PreprocessorTokenType::OpeningParenthesis,
                source_vectors: SourceVectors::from(SourceVector {
                position: SourcePosition {
                    index: 11,
                    line: 1,
                    column: 12,
                    source_file: StringCacheId::from(0),
                },
                length: 1,
            }),
                contents: StringCacheId::from(4),
            }),
            Ok(PreprocessorToken {
                kind: PreprocessorTokenType::Identifier,
                source_vectors: SourceVectors::from(SourceVector {
                position: SourcePosition {
                    index: 12,
                    line: 1,
                    column: 13,
                    source_file: StringCacheId::from(0),
                },
                length: 1,
            }),
                contents: StringCacheId::from(5),
            }),
            Ok(PreprocessorToken {
                kind: PreprocessorTokenType::Comma,
                source_vectors: SourceVectors::from(SourceVector {
                position: SourcePosition {
                    index: 13,
                    line: 1,
                    column: 14,
                    source_file: StringCacheId::from(0),
                },
                length: 1,
            }),
                contents: StringCacheId::from(6),
            }),
            Ok(PreprocessorToken {
                kind: PreprocessorTokenType::Identifier,
                source_vectors: SourceVectors::from(SourceVector {
                position: SourcePosition {
                    index: 15,
                    line: 1,
                    column: 16,
                    source_file: StringCacheId::from(0),
                },
                length: 1,
            }),
                contents: StringCacheId::from(7),
            }),
            Ok(PreprocessorToken {
                kind: PreprocessorTokenType::ClosingParenthesis,
                source_vectors: SourceVectors::from(SourceVector {
                position: SourcePosition {
                    index: 16,
                    line: 1,
                    column: 17,
                    source_file: StringCacheId::from(0),
                },
                length: 1,
            }),
                contents: StringCacheId::from(8),
            }),
            Ok(PreprocessorToken {
                kind: PreprocessorTokenType::Identifier,
                source_vectors: SourceVectors::from(SourceVector {
                position: SourcePosition {
                    index: 18,
                    line: 1,
                    column: 19,
                    source_file: StringCacheId::from(0),
                },
                length: 1,
            }),
                contents: StringCacheId::from(5),
            }),
            Ok(PreprocessorToken {
                kind: PreprocessorTokenType::GreaterThan,
                source_vectors: SourceVectors::from(SourceVector {
                position: SourcePosition {
                    index: 20,
                    line: 1,
                    column: 21,
                    source_file: StringCacheId::from(0),
                },
                length: 1,
            }),
                contents: StringCacheId::from(9),
            }),
            Ok(PreprocessorToken {
                kind: PreprocessorTokenType::Identifier,
                source_vectors: SourceVectors::from(SourceVector {
                position: SourcePosition {
                    index: 22,
                    line: 1,
                    column: 23,
                    source_file: StringCacheId::from(0),
                },
                length: 1,
            }),
                contents: StringCacheId::from(7),
            }),
            Ok(PreprocessorToken {
                kind: PreprocessorTokenType::QuestionMark,
                source_vectors: SourceVectors::from(SourceVector {
                position: SourcePosition {
                    index: 29,
                    line: 2,
                    column: 5,
                    source_file: StringCacheId::from(0),
                },
                length: 1,
            }),
                contents: StringCacheId::from(10),
            }),
            Ok(PreprocessorToken {
                kind: PreprocessorTokenType::Identifier,
                source_vectors: SourceVectors::from(SourceVector {
                position: SourcePosition {
                    index: 31,
                    line: 2,
                    column: 7,
                    source_file: StringCacheId::from(0),
                },
                length: 1,
            }),
                contents: StringCacheId::from(5),
            }),
            Ok(PreprocessorToken {
                kind: PreprocessorTokenType::Colon,
                source_vectors: SourceVectors::from(SourceVector {
                position: SourcePosition {
                    index: 38,
                    line: 3,
                    column: 5,
                    source_file: StringCacheId::from(0),
                },
                length: 1,
            }),
                contents: StringCacheId::from(11),
            }),
            Err(PreprocessorTokenizerError::ErrorFromPrev(RemoveEscapedNewlinesError::MissingFinalNewLine(MissingNewlineError(SourcePosition {
                index:       41,
                line:        3,
                column:      8,
                source_file: StringCacheId::from(0),
            })))),
            Ok(PreprocessorToken {
                kind: PreprocessorTokenType::Identifier,
                source_vectors: SourceVectors::from(SourceVector {
                position: SourcePosition {
                    index: 40,
                    line: 3,
                    column: 7,
                    source_file: StringCacheId::from(0),
                },
                length: 1,
            }),
                contents: StringCacheId::from(7),
            }),
            Ok(PreprocessorToken {
                kind: PreprocessorTokenType::Newline,
                source_vectors: SourceVectors::from(SourceVector {
                position: SourcePosition {
                    index: 41,
                    line: 3,
                    column: 8,
                    source_file: StringCacheId::from(0),
                },
                length: 0,
            }),
                contents: StringCacheId::from(12),
            }),
        ]
    )]
    #[case("const char *str = \"Hello, World!\\n\";\n", vec![
        Ok(PreprocessorToken {
            kind: PreprocessorTokenType::Identifier,
            source_vectors: SourceVectors::from(SourceVector {
            position: SourcePosition {
                index: 0,
                line: 1,
                column: 1,
                source_file: StringCacheId::from(0),
            },
            length: 5,
        }),
            contents: StringCacheId::from(1),
        }),
        Ok(PreprocessorToken {
            kind: PreprocessorTokenType::Identifier,
            source_vectors: SourceVectors::from(SourceVector {
            position: SourcePosition {
                index: 6,
                line: 1,
                column: 7,
                source_file: StringCacheId::from(0),
            },
            length: 4,
        }),
            contents: StringCacheId::from(2),
        }),
        Ok(PreprocessorToken {
            kind: PreprocessorTokenType::Asterisk,
            source_vectors: SourceVectors::from(SourceVector {
            position: SourcePosition {
                index: 11,
                line: 1,
                column: 12,
                source_file: StringCacheId::from(0),
            },
            length: 1,
        }),
            contents: StringCacheId::from(3),
        }),
        Ok(PreprocessorToken {
            kind: PreprocessorTokenType::Identifier,
            source_vectors: SourceVectors::from(SourceVector {
            position: SourcePosition {
                index: 12,
                line: 1,
                column: 13,
                source_file: StringCacheId::from(0),
            },
            length: 3,
        }),
            contents: StringCacheId::from(4),
        }),
        Ok(PreprocessorToken {
            kind: PreprocessorTokenType::Equals,
            source_vectors: SourceVectors::from(SourceVector {
            position: SourcePosition {
                index: 16,
                line: 1,
                column: 17,
                source_file: StringCacheId::from(0),
            },
            length: 1,
        }),
            contents: StringCacheId::from(5),
        }),
        Ok(PreprocessorToken {
            kind: PreprocessorTokenType::String,
            source_vectors: SourceVectors::from(SourceVector {
            position: SourcePosition {
                index: 18,
                line: 1,
                column: 19,
                source_file: StringCacheId::from(0),
            },
            length: 17,
        }),
            contents: StringCacheId::from(6),
        }),
        Ok(PreprocessorToken {
            kind: PreprocessorTokenType::SemiColon,
            source_vectors: SourceVectors::from(SourceVector {
            position: SourcePosition {
                index: 35,
                line: 1,
                column: 36,
                source_file: StringCacheId::from(0),
            },
            length: 1,
        }),
            contents: StringCacheId::from(7),
        }),
        Ok(PreprocessorToken {
            kind: PreprocessorTokenType::Newline,
            source_vectors: SourceVectors::from(SourceVector {
            position: SourcePosition {
                index: 36,
                line: 1,
                column: 37,
                source_file: StringCacheId::from(0),
            },
            length: 1,
        }),
            contents: StringCacheId::from(8),
        }),
    ])]
    #[case("\"\\\"\"", vec![
        Ok(PreprocessorToken {
            kind: PreprocessorTokenType::String,
            source_vectors: SourceVectors::from(SourceVector {
            position: SourcePosition {
                index: 0,
                line: 1,
                column: 1,
                source_file: StringCacheId::from(0),
            },
            length: 4,
        }),
            contents: StringCacheId::from(1),
        }),
        Err(PreprocessorTokenizerError::ErrorFromPrev(RemoveEscapedNewlinesError::MissingFinalNewLine(MissingNewlineError(SourcePosition {
            index:       4,
            line:        1,
            column:      5,
            source_file: StringCacheId::from(0),
        })))),
        Ok(PreprocessorToken {
            kind: PreprocessorTokenType::Newline,
            source_vectors: SourceVectors::from(SourceVector {
            position: SourcePosition {
                index: 4,
                line: 1,
                column: 5,
                source_file: StringCacheId::from(0),
            },
            length: 0,
        }),
            contents: StringCacheId::from(2),
        }),
    ])]
    fn test_phase_3_preprocessor_tokenizer(
        #[case] input: &str,
        #[case] expected: Vec<
            Result<
                PreprocessorToken,
                PreprocessorTokenizerError<
                    RemoveEscapedNewlinesError<MapCharacterSetsError<Infallible>>,
                >,
            >,
        >,
    ) {
        let mut string_cache = StringCache::new();
        let mut tokenizer = PreprocessorTokenizer::new(
            RemoveEscapedNewlines::new(MapCharacterSets::new(NewlineTracking::new(
                input.to_owned().into(),
                string_cache.intern("<input>"),
            ))),
            string_cache,
        );
        let actual = tokenizer.by_ref().collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }
}
Interner