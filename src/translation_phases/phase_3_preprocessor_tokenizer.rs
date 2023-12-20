use std::fmt::Display;

use smallstr::SmallString;
use thiserror::Error;

use super::{ErrorSeverity, GetSeverity, Position, SavePoint as ISavePoint, TranslationPhase};
use crate::util::string_cache::{Id as StringCacheId, StringCache};
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PreprocessorTokenizer<Prev> {
    previous_phase: Prev,
    is_lexing_include_directive: bool,
    string_cache: StringCache,
}

impl<Prev> AsRef<StringCache> for PreprocessorTokenizer<Prev> {
    fn as_ref(&self) -> &StringCache {
        &self.string_cache
    }
}

impl<Prev> AsMut<StringCache> for PreprocessorTokenizer<Prev> {
    fn as_mut(&mut self) -> &mut StringCache {
        &mut self.string_cache
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) struct SavePoint<Inner> {
    inner: Inner,
    is_lexing_include_directive: bool,
}

impl<Inner> super::SavePoint for SavePoint<Inner>
where
    Inner: super::SavePoint,
{
    fn current_position(&self) -> Position {
        self.inner.current_position()
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) enum PreprocessorErrorType {
    UnknownToken,
    UnterminatedCharacter,
    UnterminatedString,
    UnterminatedAngleBracketString,
    UnterminatedIncludeString,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) struct InnerPreprocessorTokenizerError<SavePoint> {
    start_save: SavePoint,
    length: usize,
    error_type: PreprocessorErrorType,
    contents: StringCacheId,
}

impl<SavePoint: ISavePoint> std::error::Error for InnerPreprocessorTokenizerError<SavePoint> {}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Error)]
pub(crate) enum PreprocessorTokenizerError<PrevSavePoint, PrevError> {
    #[error(transparent)]
    Inner(PrevError),
    #[error(transparent)]
    TokenizerError(InnerPreprocessorTokenizerError<SavePoint<PrevSavePoint>>),
}

impl<PrevSavePoint, PrevError> GetSeverity for PreprocessorTokenizerError<PrevSavePoint, PrevError>
where
    PrevError: GetSeverity,
{
    fn severity(&self) -> ErrorSeverity {
        match self {
            | Self::Inner(e) => e.severity(),
            | Self::TokenizerError(e) => e.severity(),
        }
    }
}

impl<PrevSavePoint> GetSeverity for InnerPreprocessorTokenizerError<SavePoint<PrevSavePoint>> {
    fn severity(&self) -> ErrorSeverity {
        match self.error_type {
            | PreprocessorErrorType::UnknownToken => ErrorSeverity::Error,
            | PreprocessorErrorType::UnterminatedCharacter => ErrorSeverity::Error,
            | PreprocessorErrorType::UnterminatedString => ErrorSeverity::Error,
            | PreprocessorErrorType::UnterminatedAngleBracketString => ErrorSeverity::Error,
            | PreprocessorErrorType::UnterminatedIncludeString => ErrorSeverity::Error,
        }
    }
}

impl<SavePoint> Display for InnerPreprocessorTokenizerError<SavePoint>
where
    SavePoint: ISavePoint,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let current_position = self.start_save.current_position();
        write!(
            f,
            "{:?} at {}:{}",
            self.error_type, current_position.line, current_position.column
        )
    }
}

impl<Prev> Iterator for PreprocessorTokenizer<Prev>
where
    Prev: TranslationPhase<Yield = char>,
{
    type Item = Result<
        PreprocessorToken<SavePoint<Prev::SavePoint>>,
        PreprocessorTokenizerError<SavePoint<Prev::SavePoint>, Prev::Error>,
    >;

    fn next(&mut self) -> Option<Self::Item> {
        fn inner<Prev: TranslationPhase<Yield = char>, const NUM_PARSER_FUNCTIONS: usize>(
            tokenizer: &mut PreprocessorTokenizer<Prev>,
            parser_functions: [ParserFunction<Prev>; NUM_PARSER_FUNCTIONS],
        ) -> Option<<PreprocessorTokenizer<Prev> as Iterator>::Item> {
            let save_point: SavePoint<<Prev as TranslationPhase>::SavePoint> = tokenizer.save();
            for (i, parser_function) in parser_functions.iter().enumerate() {
                let index_before = tokenizer.current_position().index;
                if let Some(result) = parser_function(tokenizer) {
                    Some(result.map_err(Into::into))
                }
                let index_after = tokenizer.current_position().index;
                assert_eq!(
                    index_after,
                    index_before,
                    "Compiler bug: Tokenize function number {i} Swallowed {} characters.",
                    index_after - index_before,
                );
            }
            if tokenizer.previous_phase.next().is_some() {
                let result = Some(Err(
                    tokenizer.generate_error(PreprocessorErrorType::UnknownToken, save_point)
                ));
                return result;
            }
            None
        }

        let step_over_functions: [StepOverFunction<Prev>; 3] = [
            PreprocessorTokenizer::step_over_whitespace,
            PreprocessorTokenizer::step_over_line_comment,
            PreprocessorTokenizer::step_over_block_comment,
        ];
        for step_over_function in step_over_functions {
            step_over_function(self);
        }

        if self.is_lexing_include_directive {
            inner(
                self,
                [
                    PreprocessorTokenizer::tokenize_angle_bracket_string,
                    PreprocessorTokenizer::tokenize_include_string,
                ],
            )
        } else {
            inner(
                self,
                [
                    PreprocessorTokenizer::tokenize_tag,
                    PreprocessorTokenizer::tokenize_newline,
                    PreprocessorTokenizer::tokenize_number,
                    PreprocessorTokenizer::tokenize_keyword,
                    PreprocessorTokenizer::tokenize_identifier,
                    PreprocessorTokenizer::tokenize_string,
                    PreprocessorTokenizer::tokenize_char,
                ],
            )
        }
    }
}

impl<Prev> TranslationPhase for PreprocessorTokenizer<Prev>
where
    Prev: TranslationPhase<Yield = char>,
{
    type Error = PreprocessorTokenizerError<SavePoint<Prev::SavePoint>, Prev::Error>;
    type SavePoint = SavePoint<Prev::SavePoint>;
    type Yield = PreprocessorToken<SavePoint<Prev::SavePoint>>;

    fn save(&self) -> Self::SavePoint {
        SavePoint {
            inner: self.previous_phase.save(),
            is_lexing_include_directive: self.is_lexing_include_directive,
        }
    }

    fn restore(&mut self, save_point: Self::SavePoint) {
        self.previous_phase.restore(save_point.inner);
        self.is_lexing_include_directive = save_point.is_lexing_include_directive;
    }

    fn current_position(&self) -> Position {
        self.previous_phase.current_position()
    }
}

impl<Prev> PreprocessorTokenizer<Prev>
where
    Prev: TranslationPhase<Yield = char>,
{
    fn generate_error(
        &mut self,
        error_type: PreprocessorErrorType,
        start_save: SavePoint<Prev::SavePoint>,
    ) -> InnerPreprocessorTokenizerError<SavePoint<Prev::SavePoint>> {
        let current_save = self.save();
        self.restore(start_save);
        let contents: SmallString<[u8; 1024]> = self
            .previous_phase
            .take_while(|_| self.current_position().index < current_save.current_position().index)
            .collect();
        self.restore(current_save);
        let start_position = start_save.current_position();

        InnerPreprocessorTokenizerError {
            start_save,
            length: self.current_position().index - start_position.index,
            error_type,
            contents: self.string_cache.intern(contents.as_str()),
        }
    }

    fn generate_token(
        &mut self,
        token_type: PreprocessorTokenType,
        start_save: SavePoint<Prev::SavePoint>,
    ) -> PreprocessorToken<SavePoint<Prev::SavePoint>> {
        let current_save = self.save();
        self.restore(start_save);
        let contents: SmallString<[u8; 1024]> = self
            .previous_phase
            .take_while(|_| self.current_position().index < current_save.current_position().index)
            .collect();
        self.restore(current_save);
        let start_position = start_save.current_position();

        PreprocessorToken {
            start_save,
            length: self.current_position().index - start_position.index,
            token_type,
            contents: self.string_cache.intern(contents.as_str()),
        }
    }

    fn step_over_whitespace(&mut self) {
        let mut save_point = self.save();
        while self
            .previous_phase
            .next()
            .is_some_and(|c| c.is_whitespace() && c != '\n')
        {
            save_point = self.save();
        }
        self.restore(save_point);
    }

    fn step_over_line_comment(&mut self) {
        let mut save_point = self.save();
        let current = self.previous_phase.next();
        let next = self.previous_phase.next();
        if current != Some('/') || next != Some('/') {
            self.restore(save_point);
            return;
        }
        while self.previous_phase.next().is_some_and(|c| c != '\n') {
            save_point = self.save();
        }
        self.restore(save_point);
    }

    fn step_over_block_comment(&mut self) {
        let mut save_point = self.save();
        let current = self.previous_phase.next();
        let next = self.previous_phase.next();
        if current != Some('/') || next != Some('*') {
            self.restore(save_point);
            return;
        }
        while self.previous_phase.next().is_some_and(|c| c != '*')
            && self.previous_phase.next().is_some_and(|c| c != '/')
        {
            save_point = self.save();
        }
        self.restore(save_point);
    }

    fn new(previous_phase: Prev, string_cache: StringCache) -> Self {
        Self {
            previous_phase,
            is_lexing_include_directive: false,
            string_cache,
        }
    }

    fn tokenize_tag(&mut self) -> ParserReturn<SavePoint<Prev::SavePoint>> {
        let save_point = self.save();
        'outer: for tag in &TAGS {
            for c in tag.tag.chars() {
                if self.previous_phase.next() != Some(c) {
                    self.restore(save_point.clone());
                    continue 'outer;
                }
            }
            return Some(Ok(self.generate_token(tag.token_type, save_point)));
        }
        None
    }

    fn tokenize_newline(&mut self) -> ParserReturn<SavePoint<Prev::SavePoint>> {
        let save_point = self.save();
        if self.previous_phase.next() == Some('\n') {
            Some(Ok(
                self.generate_token(PreprocessorTokenType::Newline, save_point)
            ))
        } else {
            self.restore(save_point);
            None
        }
    }

    fn tokenize_identifier(&mut self) -> ParserReturn<SavePoint<Prev::SavePoint>> {
        let save_point = self.save();
        if !self
            .previous_phase
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
        {
            self.restore(save_point);
            return None;
        }
        let mut save_point2 = self.save();
        while self
            .previous_phase
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
        {
            save_point2 = self.save();
        }
        self.restore(save_point2);
        Some(Ok(self.generate_token(
            PreprocessorTokenType::Identifier,
            save_point,
        )))
    }

    fn tokenize_keyword(&mut self) -> ParserReturn<SavePoint<Prev::SavePoint>> {
        let save_point = self.save();
        'outer: for keyword in &KEYWORDS {
            for c in keyword.keyword.chars() {
                if self.previous_phase.next() != Some(c) {
                    self.restore(save_point.clone());
                    continue 'outer;
                }
            }
            return Some(Ok(self.generate_token(keyword.token_type, save_point)));
        }
        None
    }

    fn tokenize_string_like(
        &mut self,
        start_char: char,
        end_char: char,
        ignore_escapes: bool,
        return_token_type: PreprocessorTokenType,
        error_type: PreprocessorErrorType,
    ) -> ParserReturn<SavePoint<Prev::SavePoint>> {
        let save_point = self.save();

        if self.previous_phase.next() != Some('L') {
            self.restore(save_point.clone());
        }

        if self.previous_phase.next() != Some(start_char) {
            self.restore(save_point);
            return None;
        }

        while let Some(current) = self.previous_phase.next() {
            if !ignore_escapes {
                let save_point = self.save();
                let next = self.previous_phase.next();
                if current == '\\' && next == Some(end_char) {
                    continue;
                }
                self.restore(save_point.clone());
            }
            if current == end_char {
                break;
            }
            if current == '\n' {
                return Some(Err(self.generate_error(error_type, save_point)));
            }
        }

        Some(Ok(self.generate_token(return_token_type, save_point)))
    }

    fn tokenize_string(&mut self) -> ParserReturn<SavePoint<Prev::SavePoint>> {
        self.tokenize_string_like(
            '"',
            '"',
            false,
            PreprocessorTokenType::String,
            PreprocessorErrorType::UnterminatedString,
        )
    }

    fn tokenize_char(&mut self) -> ParserReturn<SavePoint<Prev::SavePoint>> {
        self.tokenize_string_like(
            '\'',
            '\'',
            false,
            PreprocessorTokenType::Character,
            PreprocessorErrorType::UnterminatedCharacter,
        )
    }

    fn tokenize_angle_bracket_string(&mut self) -> ParserReturn<SavePoint<Prev::SavePoint>> {
        self.tokenize_string_like(
            '<',
            '>',
            true,
            PreprocessorTokenType::AngleBracketString,
            PreprocessorErrorType::UnterminatedAngleBracketString,
        )
    }

    fn tokenize_include_string(&mut self) -> ParserReturn<SavePoint<Prev::SavePoint>> {
        self.tokenize_string_like(
            '"',
            '"',
            true,
            PreprocessorTokenType::IncludeString,
            PreprocessorErrorType::UnterminatedIncludeString,
        )
    }

    fn tokenize_number(&mut self) -> ParserReturn<SavePoint<Prev::SavePoint>> {
        let save_point = self.save();

        if self.previous_phase.next() != Some('.') {
            self.restore(save_point.clone());
        }
        if !self
            .previous_phase
            .next()
            .is_some_and(|c| c.is_ascii_digit())
        {
            self.restore(save_point);
            return None;
        }
        let mut save_point2 = self.save();
        while let Some(current) = self.previous_phase.next() {
            const EXPONENTS: [&str; 8] = ["e-", "E-", "e+", "E+", "p-", "P-", "p+", "P+"];
            let save_point = self.save();
            let next = self.previous_phase.next();
            if EXPONENTS.iter().any(|exponent| {
                let &[b1, b2] = exponent.as_bytes() else {
                    unreachable!()
                };
                current == b1 as char && next == Some(b2 as char)
            }) {
                save_point2 = self.save();
                continue;
            }
            self.restore(save_point);

            if current.is_alphanumeric() || current == '.' {
                save_point2 = self.save();
                continue;
            }
            self.restore(save_point2);
            break;
        }
        Some(Ok(
            self.generate_token(PreprocessorTokenType::Number, save_point)
        ))
    }
}

type ParserReturn<Inner> = Option<ParserResult<Inner>>;
type ParserResult<Inner> = Result<PreprocessorToken<Inner>, InnerPreprocessorTokenizerError<Inner>>;
#[allow(type_alias_bounds)]
type ParserFunction<Prev>
where
    Prev: TranslationPhase,
= fn(&mut PreprocessorTokenizer<Prev>) -> ParserReturn<SavePoint<Prev::SavePoint>>;

#[allow(type_alias_bounds)]
type StepOverFunction<Prev>
where
    Prev: TranslationPhase,
= fn(&mut PreprocessorTokenizer<Prev>);

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) enum PreprocessorTokenType {
    // Literals
    Identifier,
    Number,
    String,
    Character,

    // Include tokens.
    AngleBracketString,
    IncludeString,

    // Whitespace
    Newline,

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

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PreprocessorToken<SavePoint> {
    pub(crate) token_type: PreprocessorTokenType,
    pub(crate) start_save: SavePoint,
    pub(crate) contents: StringCacheId,
    pub(crate) length: usize,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
struct Tag {
    tag: &'static str,
    token_type: PreprocessorTokenType,
}

const fn tag(tag: &'static str, token_type: PreprocessorTokenType) -> Tag {
    Tag { tag, token_type }
}

type Ptt = PreprocessorTokenType;

const TAGS: [Tag; 54] = [
    tag("[", Ptt::OpeningSquareBracket),
    tag("]", Ptt::ClosingSquareBracket),
    tag("(", Ptt::OpeningParenthesis),
    tag(")", Ptt::ClosingParenthesis),
    tag("{", Ptt::OpeningCurlyBrace),
    tag("}", Ptt::ClosingCurlyBrace),
    tag("...", Ptt::Ellipsis),
    tag(".", Ptt::Period),
    tag("->", Ptt::Arrow),
    tag("++", Ptt::PlusPlus),
    tag("--", Ptt::MinusMinus),
    tag("<<=", Ptt::LessThanLessThanEquals),
    tag(">>=", Ptt::GreaterThanGreaterThanEquals),
    tag("+=", Ptt::PlusEquals),
    tag("-=", Ptt::MinusEquals),
    tag("*=", Ptt::AsteriskEquals),
    tag("/=", Ptt::ForwardSlashEquals),
    tag("%=", Ptt::PercentEquals),
    tag("&=", Ptt::AmpersandEquals),
    tag("^=", Ptt::CaretEquals),
    tag("|=", Ptt::PipeEquals),
    tag("<:", Ptt::OpeningSquareBracket),
    tag(":>", Ptt::ClosingSquareBracket),
    tag("<%", Ptt::OpeningCurlyBrace),
    tag("%>", Ptt::ClosingCurlyBrace),
    tag("%:%:", Ptt::HashHash),
    tag("%:", Ptt::Hash),
    tag("<<", Ptt::LessThanLessThan),
    tag(">>", Ptt::GreaterThanGreaterThan),
    tag("<=", Ptt::LessThanEquals),
    tag(">=", Ptt::GreaterThanEquals),
    tag("==", Ptt::EqualsEquals),
    tag("!=", Ptt::ExclamationMarkEquals),
    tag("<", Ptt::LessThan),
    tag(">", Ptt::GreaterThan),
    tag("&&", Ptt::AmpersandAmpersand),
    tag("||", Ptt::PipePipe),
    tag("*", Ptt::Asterisk),
    tag("+", Ptt::Plus),
    tag("-", Ptt::Minus),
    tag("~", Ptt::Tilde),
    tag("!", Ptt::ExclamationMark),
    tag("/", Ptt::ForwardSlash),
    tag("%", Ptt::Percent),
    tag("&", Ptt::Ampersand),
    tag("^", Ptt::Caret),
    tag("|", Ptt::Pipe),
    tag("?", Ptt::QuestionMark),
    tag(":", Ptt::Colon),
    tag(";", Ptt::SemiColon),
    tag("=", Ptt::Equals),
    tag(",", Ptt::Comma),
    tag("##", Ptt::HashHash),
    tag("#", Ptt::Hash),
];

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
struct Keyword {
    keyword: &'static str,
    token_type: PreprocessorTokenType,
}

static KEYWORDS: [Keyword; 1] = [Keyword {
    keyword: "defined",
    token_type: PreprocessorTokenType::Defined,
}];

pub(crate) fn phase_3_preprocessor_tokenizer<Prev>(
    previous_phase: Prev,
    string_cache: StringCache,
) -> PreprocessorTokenizer<Prev>
where
    Prev: TranslationPhase<Yield = char>,
{
    PreprocessorTokenizer::new(previous_phase, string_cache)
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;
    use crate::{
        translation_phases::{
            phase_1_map_character_sets::SavePoint as MapCharacterSetsSavePoint,
            phase_2_remove_escaped_newlines::{
                RemoveEscapedNewlinesError, SavePoint as RemoveEscapedNewLinesSavePoint,
                State as RemoveEscapedNewLinesState,
            },
        },
        util::string_cache::{Id as StringCacheId, StringCache},
    };

    #[rstest]
    #[case("", vec![
        Err(PreprocessorTokenizerError::Inner(RemoveEscapedNewlinesError::MissingFinalNewLine)),
        Ok(PreprocessorToken {
            token_type: PreprocessorTokenType::Newline,
            start_save: SavePoint {
                inner: RemoveEscapedNewLinesSavePoint {
                    inner: MapCharacterSetsSavePoint {
                        inner: Position {
                            index: 0,
                            line: 1,
                            column: 1,
                            source_file: StringCacheId::from_usize(0),
                        },
                    },
                    last_was_newline: false,
                    state: RemoveEscapedNewLinesState::Done,
                },
                is_lexing_include_directive: false,
            },
            length: 0,
            contents: StringCacheId::from_usize(1),

        })
    ])]
    #[case("int x = @1;\n", vec![
        Ok(PreprocessorToken {
            token_type: PreprocessorTokenType::Identifier,
            start_save: SavePoint {
                inner: RemoveEscapedNewLinesSavePoint {
                    inner: MapCharacterSetsSavePoint {
                        inner: Position {
                            index: 0,
                            line: 1,
                            column: 1,
                            source_file: StringCacheId::from_usize(0),
                        },
                    },
                    last_was_newline: false,
                    state: RemoveEscapedNewLinesState::Normal,
                },
                is_lexing_include_directive: false,
            },
            length: 3,
            contents: StringCacheId::from_usize(1),
        }),
        Ok(PreprocessorToken {
            token_type: PreprocessorTokenType::Identifier,
            start_save: SavePoint {
                inner: RemoveEscapedNewLinesSavePoint {
                    inner: MapCharacterSetsSavePoint {
                        inner: Position {
                            index: 4,
                            line: 1,
                            column: 5,
                            source_file: StringCacheId::from_usize(0),
                        },
                    },
                    last_was_newline: false,
                    state: RemoveEscapedNewLinesState::Normal,
                },
                is_lexing_include_directive: false,
            },
            length: 1,
            contents: StringCacheId::from_usize(2),
        }),
        Ok(PreprocessorToken {
            token_type: PreprocessorTokenType::Equals,
            start_save: SavePoint {
                inner: RemoveEscapedNewLinesSavePoint {
                    inner: MapCharacterSetsSavePoint {
                        inner: Position {
                            index: 6,
                            line: 1,
                            column: 7,
                            source_file: StringCacheId::from_usize(0),
                        },
                    },
                    last_was_newline: false,
                    state: RemoveEscapedNewLinesState::Normal,
                },
                is_lexing_include_directive: false,
            },
            length: 1,
            contents: StringCacheId::from_usize(3),
        }),
        Err(PreprocessorTokenizerError::TokenizerError(InnerPreprocessorTokenizerError {
            start_save: SavePoint {
                inner: RemoveEscapedNewLinesSavePoint {
                    inner: MapCharacterSetsSavePoint {
                        inner: Position {
                            index: 8,
                            line: 1,
                            column: 9,
                            source_file: StringCacheId::from_usize(0),
                        },
                    },
                    last_was_newline: false,
                    state: RemoveEscapedNewLinesState::Normal,
                },
                is_lexing_include_directive: false,
            },
            length: 1,
            error_type: PreprocessorErrorType::UnknownToken,
            contents: StringCacheId::from_usize(4),
        })),
        Ok(PreprocessorToken {
            token_type: PreprocessorTokenType::Number,
            start_save: SavePoint {
                inner: RemoveEscapedNewLinesSavePoint {
                    inner: MapCharacterSetsSavePoint {
                        inner: Position {
                            index: 9,
                            line: 1,
                            column: 10,
                            source_file: StringCacheId::from_usize(0),
                        },
                    },
                    last_was_newline: false,
                    state: RemoveEscapedNewLinesState::Normal,
                },
                is_lexing_include_directive: false,
            },
            length: 1,
            contents: StringCacheId::from_usize(5),
        }),
        Ok(PreprocessorToken {
            token_type: PreprocessorTokenType::SemiColon,
            start_save: SavePoint {
                inner: RemoveEscapedNewLinesSavePoint {
                    inner: MapCharacterSetsSavePoint {
                        inner: Position {
                            index: 10,
                            line: 1,
                            column: 11,
                            source_file: StringCacheId::from_usize(0),
                        },
                    },
                    last_was_newline: false,
                    state: RemoveEscapedNewLinesState::Normal,
                },
                is_lexing_include_directive: false,
            },
            length: 1,
            contents: StringCacheId::from_usize(6),
        }),
        Ok(PreprocessorToken {
            token_type: PreprocessorTokenType::Newline,
            start_save: SavePoint {
                inner: RemoveEscapedNewLinesSavePoint {
                    inner: MapCharacterSetsSavePoint {
                        inner: Position {
                            index: 11,
                            line: 1,
                            column: 12,
                            source_file: StringCacheId::from_usize(0),
                        },
                    },
                    last_was_newline: false,
                    state: RemoveEscapedNewLinesState::Done,
                },
                is_lexing_include_directive: false,
            },
            length: 1,
            contents: StringCacheId::from_usize(7),
        }),
    ])]
    #[case("#define MAX(A, B) A > B\\\n    ? A\\\n    : B",
        vec![
            Ok(PreprocessorToken {
                token_type: PreprocessorTokenType::Hash,
                start_save: SavePoint {
                    inner: RemoveEscapedNewLinesSavePoint {
                        inner: MapCharacterSetsSavePoint {
                            inner: Position {
                                index: 0,
                                line: 1,
                                column: 1,
                                source_file: StringCacheId::from_usize(0),
                            },
                        },
                        last_was_newline: false,
                        state: RemoveEscapedNewLinesState::Normal,
                    },
                    is_lexing_include_directive: false,
                },
                length: 1,
                contents: StringCacheId::from_usize(1),
            }),
            Ok(PreprocessorToken {
                token_type: PreprocessorTokenType::Identifier,
                start_save: SavePoint {
                    inner: RemoveEscapedNewLinesSavePoint {
                        inner: MapCharacterSetsSavePoint {
                            inner: Position {
                                index: 1,
                                line: 1,
                                column: 2,
                                source_file: StringCacheId::from_usize(0),
                            },
                        },
                        last_was_newline: false,
                        state: RemoveEscapedNewLinesState::Normal,
                    },
                    is_lexing_include_directive: false,
                },
                length: 6,
                contents: StringCacheId::from_usize(2),
            }),
            Ok(PreprocessorToken {
                token_type: PreprocessorTokenType::Identifier,
                start_save: SavePoint {
                    inner: RemoveEscapedNewLinesSavePoint {
                        inner: MapCharacterSetsSavePoint {
                            inner: Position {
                                index: 8,
                                line: 1,
                                column: 9,
                                source_file: StringCacheId::from_usize(0),
                            },
                        },
                        last_was_newline: false,
                        state: RemoveEscapedNewLinesState::Normal,
                    },
                    is_lexing_include_directive: false,
                },
                length: 3,
                contents: StringCacheId::from_usize(3),
            }),
            Ok(PreprocessorToken {
                token_type: PreprocessorTokenType::OpeningParenthesis,
                start_save: SavePoint {
                    inner: RemoveEscapedNewLinesSavePoint {
                        inner: MapCharacterSetsSavePoint {
                            inner: Position {
                                index: 11,
                                line: 1,
                                column: 12,
                                source_file: StringCacheId::from_usize(0),
                            },
                        },
                        last_was_newline: false,
                        state: RemoveEscapedNewLinesState::Normal,
                    },
                    is_lexing_include_directive: false,
                },
                length: 1,
                contents: StringCacheId::from_usize(4),
            }),
            Ok(PreprocessorToken {
                token_type: PreprocessorTokenType::Identifier,
                start_save: SavePoint {
                    inner: RemoveEscapedNewLinesSavePoint {
                        inner: MapCharacterSetsSavePoint {
                            inner: Position {
                                index: 12,
                                line: 1,
                                column: 13,
                                source_file: StringCacheId::from_usize(0),
                            },
                        },
                        last_was_newline: false,
                        state: RemoveEscapedNewLinesState::Normal,
                    },
                    is_lexing_include_directive: false,
                },
                length: 1,
                contents: StringCacheId::from_usize(5),
            }),
            Ok(PreprocessorToken {
                token_type: PreprocessorTokenType::Comma,
                start_save: SavePoint {
                    inner: RemoveEscapedNewLinesSavePoint {
                        inner: MapCharacterSetsSavePoint {
                            inner: Position {
                                index: 13,
                                line: 1,
                                column: 14,
                                source_file: StringCacheId::from_usize(0),
                            },
                        },
                        last_was_newline: false,
                        state: RemoveEscapedNewLinesState::Normal,
                    },
                    is_lexing_include_directive: false,
                },
                length: 1,
                contents: StringCacheId::from_usize(6),
            }),
            Ok(PreprocessorToken {
                token_type: PreprocessorTokenType::Identifier,
                start_save: SavePoint {
                    inner: RemoveEscapedNewLinesSavePoint {
                        inner: MapCharacterSetsSavePoint {
                            inner: Position {
                                index: 15,
                                line: 1,
                                column: 16,
                                source_file: StringCacheId::from_usize(0),
                            },
                        },
                        last_was_newline: false,
                        state: RemoveEscapedNewLinesState::Normal,
                    },
                    is_lexing_include_directive: false,
                },
                length: 1,
                contents: StringCacheId::from_usize(7),
            }),
            Ok(PreprocessorToken {
                token_type: PreprocessorTokenType::ClosingParenthesis,
                start_save: SavePoint {
                    inner: RemoveEscapedNewLinesSavePoint {
                        inner: MapCharacterSetsSavePoint {
                            inner: Position {
                                index: 16,
                                line: 1,
                                column: 17,
                                source_file: StringCacheId::from_usize(0),
                            },
                        },
                        last_was_newline: false,
                        state: RemoveEscapedNewLinesState::Normal,
                    },
                    is_lexing_include_directive: false,
                },
                length: 1,
                contents: StringCacheId::from_usize(8),
            }),
            Ok(PreprocessorToken {
                token_type: PreprocessorTokenType::Identifier,
                start_save: SavePoint {
                    inner: RemoveEscapedNewLinesSavePoint {
                        inner: MapCharacterSetsSavePoint {
                            inner: Position {
                                index: 18,
                                line: 1,
                                column: 19,
                                source_file: StringCacheId::from_usize(0),
                            },
                        },
                        last_was_newline: false,
                        state: RemoveEscapedNewLinesState::Normal,
                    },
                    is_lexing_include_directive: false,
                },
                length: 1,
                contents: StringCacheId::from_usize(5),
            }),
            Ok(PreprocessorToken {
                token_type: PreprocessorTokenType::GreaterThan,
                start_save: SavePoint {
                    inner: RemoveEscapedNewLinesSavePoint {
                        inner: MapCharacterSetsSavePoint {
                            inner: Position {
                                index: 20,
                                line: 1,
                                column: 21,
                                source_file: StringCacheId::from_usize(0),
                            },
                        },
                        last_was_newline: false,
                        state: RemoveEscapedNewLinesState::Normal,
                    },
                    is_lexing_include_directive: false,
                },
                length: 1,
                contents: StringCacheId::from_usize(9),
            }),
            Ok(PreprocessorToken {
                token_type: PreprocessorTokenType::Identifier,
                start_save: SavePoint {
                    inner: RemoveEscapedNewLinesSavePoint {
                        inner: MapCharacterSetsSavePoint {
                            inner: Position {
                                index: 22,
                                line: 1,
                                column: 23,
                                source_file: StringCacheId::from_usize(0),
                            },
                        },
                        last_was_newline: false,
                        state: RemoveEscapedNewLinesState::Normal,
                    },
                    is_lexing_include_directive: false,
                },
                length: 1,
                contents: StringCacheId::from_usize(7),
            }),
            Ok(PreprocessorToken {
                token_type: PreprocessorTokenType::QuestionMark,
                start_save: SavePoint {
                    inner: RemoveEscapedNewLinesSavePoint {
                        inner: MapCharacterSetsSavePoint {
                            inner: Position {
                                index: 29,
                                line: 2,
                                column: 5,
                                source_file: StringCacheId::from_usize(0),
                            },
                        },
                        last_was_newline: false,
                        state: RemoveEscapedNewLinesState::Normal,
                    },
                    is_lexing_include_directive: false,
                },
                length: 1,
                contents: StringCacheId::from_usize(10),
            }),
            Ok(PreprocessorToken {
                token_type: PreprocessorTokenType::Identifier,
                start_save: SavePoint {
                    inner: RemoveEscapedNewLinesSavePoint {
                        inner: MapCharacterSetsSavePoint {
                            inner: Position {
                                index: 31,
                                line: 2,
                                column: 7,
                                source_file: StringCacheId::from_usize(0),
                            },
                        },
                        last_was_newline: false,
                        state: RemoveEscapedNewLinesState::Normal,
                    },
                    is_lexing_include_directive: false,
                },
                length: 1,
                contents: StringCacheId::from_usize(5),
            }),
            Ok(PreprocessorToken {
                token_type: PreprocessorTokenType::Colon,
                start_save: SavePoint {
                    inner: RemoveEscapedNewLinesSavePoint {
                        inner: MapCharacterSetsSavePoint {
                            inner: Position {
                                index: 38,
                                line: 3,
                                column: 5,
                                source_file: StringCacheId::from_usize(0),
                            },
                        },
                        last_was_newline: false,
                        state: RemoveEscapedNewLinesState::Normal,
                    },
                    is_lexing_include_directive: false,
                },
                length: 1,
                contents: StringCacheId::from_usize(11),
            }),
            Ok(PreprocessorToken {
                token_type: PreprocessorTokenType::Identifier,
                start_save: SavePoint {
                    inner: RemoveEscapedNewLinesSavePoint {
                        inner: MapCharacterSetsSavePoint {
                            inner: Position {
                                index: 40,
                                line: 3,
                                column: 7,
                                source_file: StringCacheId::from_usize(0),
                            },
                        },
                        last_was_newline: false,
                        state: RemoveEscapedNewLinesState::Normal,
                    },
                    is_lexing_include_directive: false,
                },
                length: 1,
                contents: StringCacheId::from_usize(7),
            }),
            Err(PreprocessorTokenizerError::Inner(RemoveEscapedNewlinesError::MissingFinalNewLine)),
            Ok(PreprocessorToken {
                token_type: PreprocessorTokenType::Newline,
                start_save: SavePoint {
                    inner: RemoveEscapedNewLinesSavePoint {
                        inner: MapCharacterSetsSavePoint {
                            inner: Position {
                                index: 41,
                                line: 3,
                                column: 8,
                                source_file: StringCacheId::from_usize(0),
                            },
                        },
                        last_was_newline: false,
                        state: RemoveEscapedNewLinesState::Done,
                    },
                    is_lexing_include_directive: false,
                },
                length: 0,
                contents: StringCacheId::from_usize(12),
            }),
        ]
    )]
    #[case("const char *str = \"Hello, World!\\n\";\n", vec![
        Ok(PreprocessorToken {
            token_type: PreprocessorTokenType::Identifier,
            start_save: SavePoint {
                inner: RemoveEscapedNewLinesSavePoint {
                    inner: MapCharacterSetsSavePoint {
                        inner: Position {
                            index: 0,
                            line: 1,
                            column: 1,
                            source_file: StringCacheId::from_usize(0),
                        },
                    },
                    last_was_newline: false,
                    state: RemoveEscapedNewLinesState::Normal,
                },
                is_lexing_include_directive: false,
            },
            length: 5,
            contents: StringCacheId::from_usize(1),
        }),
        Ok(PreprocessorToken {
            token_type: PreprocessorTokenType::Identifier,
            start_save: SavePoint {
                inner: RemoveEscapedNewLinesSavePoint {
                    inner: MapCharacterSetsSavePoint {
                        inner: Position {
                            index: 6,
                            line: 1,
                            column: 7,
                            source_file: StringCacheId::from_usize(0),
                        },
                    },
                    last_was_newline: false,
                    state: RemoveEscapedNewLinesState::Normal,
                },
                is_lexing_include_directive: false,
            },
            length: 4,
            contents: StringCacheId::from_usize(2),
        }),
        Ok(PreprocessorToken {
            token_type: PreprocessorTokenType::Asterisk,
            start_save: SavePoint {
                inner: RemoveEscapedNewLinesSavePoint {
                    inner: MapCharacterSetsSavePoint {
                        inner: Position {
                            index: 11,
                            line: 1,
                            column: 12,
                            source_file: StringCacheId::from_usize(0),
                        },
                    },
                    last_was_newline: false,
                    state: RemoveEscapedNewLinesState::Normal,
                },
                is_lexing_include_directive: false,
            },
            length: 1,
            contents: StringCacheId::from_usize(3),
        }),
        Ok(PreprocessorToken {
            token_type: PreprocessorTokenType::Identifier,
            start_save: SavePoint {
                inner: RemoveEscapedNewLinesSavePoint {
                    inner: MapCharacterSetsSavePoint {
                        inner: Position {
                            index: 12,
                            line: 1,
                            column: 13,
                            source_file: StringCacheId::from_usize(0),
                        },
                    },
                    last_was_newline: false,
                    state: RemoveEscapedNewLinesState::Normal,
                },
                is_lexing_include_directive: false,
            },
            length: 3,
            contents: StringCacheId::from_usize(4),
        }),
        Ok(PreprocessorToken {
            token_type: PreprocessorTokenType::Equals,
            start_save: SavePoint {
                inner: RemoveEscapedNewLinesSavePoint {
                    inner: MapCharacterSetsSavePoint {
                        inner: Position {
                            index: 16,
                            line: 1,
                            column: 17,
                            source_file: StringCacheId::from_usize(0),
                        },
                    },
                    last_was_newline: false,
                    state: RemoveEscapedNewLinesState::Normal,
                },
                is_lexing_include_directive: false,
            },
            length: 1,
            contents: StringCacheId::from_usize(5),
        }),
        Ok(PreprocessorToken {
            token_type: PreprocessorTokenType::String,
            start_save: SavePoint {
                inner: RemoveEscapedNewLinesSavePoint {
                    inner: MapCharacterSetsSavePoint {
                        inner: Position {
                            index: 18,
                            line: 1,
                            column: 19,
                            source_file: StringCacheId::from_usize(0),
                        },
                    },
                    last_was_newline: false,
                    state: RemoveEscapedNewLinesState::Normal,
                },
                is_lexing_include_directive: false,
            },
            length: 17,
            contents: StringCacheId::from_usize(6),
        }),
        Ok(PreprocessorToken {
            token_type: PreprocessorTokenType::SemiColon,
            start_save: SavePoint {
                inner: RemoveEscapedNewLinesSavePoint {
                    inner: MapCharacterSetsSavePoint {
                        inner: Position {
                            index: 35,
                            line: 1,
                            column: 36,
                            source_file: StringCacheId::from_usize(0),
                        },
                    },
                    last_was_newline: false,
                    state: RemoveEscapedNewLinesState::Normal,
                },
                is_lexing_include_directive: false,
            },
            length: 1,
            contents: StringCacheId::from_usize(7),
        }),
        Ok(PreprocessorToken {
            token_type: PreprocessorTokenType::Newline,
            start_save: SavePoint {
                inner: RemoveEscapedNewLinesSavePoint {
                    inner: MapCharacterSetsSavePoint {
                        inner: Position {
                            index: 36,
                            line: 1,
                            column: 37,
                            source_file: StringCacheId::from_usize(0),
                        },
                    },
                    last_was_newline: false,
                    state: RemoveEscapedNewLinesState::Done,
                },
                is_lexing_include_directive: false,
            },
            length: 1,
            contents: StringCacheId::from_usize(8),
        }),
    ])]
    #[case("\"\\\"\"", vec![
        Ok(PreprocessorToken {
            token_type: PreprocessorTokenType::String,
            start_save: SavePoint {
                inner: RemoveEscapedNewLinesSavePoint {
                    inner: MapCharacterSetsSavePoint {
                        inner: Position {
                            index: 0,
                            line: 1,
                            column: 1,
                            source_file: StringCacheId::from_usize(0),
                        },
                    },
                    last_was_newline: false,
                    state: RemoveEscapedNewLinesState::Normal,
                },
                is_lexing_include_directive: false,
            },
            length: 4,
            contents: StringCacheId::from_usize(1),
        }),
        Err(PreprocessorTokenizerError::Inner(RemoveEscapedNewlinesError::MissingFinalNewLine)),
        Ok(PreprocessorToken {
            token_type: PreprocessorTokenType::Newline,
            start_save: SavePoint {
                inner: RemoveEscapedNewLinesSavePoint {
                    inner: MapCharacterSetsSavePoint {
                        inner: Position {
                            index: 4,
                            line: 1,
                            column: 5,
                            source_file: StringCacheId::from_usize(0),
                        },
                    },
                    last_was_newline: false,
                    state: RemoveEscapedNewLinesState::Done,
                },
                is_lexing_include_directive: false,
            },
            length: 0,
            contents: StringCacheId::from_usize(2),
        }),
    ])]
    fn test_phase_3_preprocessor_tokenizer(
        #[case] input: &str,
        #[case] expected: Vec<
            Result<
                PreprocessorToken<
                    RemoveEscapedNewLinesSavePoint<MapCharacterSetsSavePoint<Position>>,
                >,
                PreprocessorTokenizerError<
                    RemoveEscapedNewLinesSavePoint<MapCharacterSetsSavePoint<Position>>,
                    RemoveEscapedNewlinesError<Infallible>,
                >,
            >,
        >,
    ) {
        use crate::translation_phases::{
            phase_0_newline_tracking::phase_0_newline_tracking,
            phase_1_map_character_sets::phase_1_map_character_sets,
            phase_2_remove_escaped_newlines::phase_2_remove_escaped_newlines,
            phase_3_preprocessor_tokenizer::phase_3_preprocessor_tokenizer,
        };

        let mut string_cache = StringCache::new();
        let mut tokenizer = phase_3_preprocessor_tokenizer(
            phase_2_remove_escaped_newlines(phase_1_map_character_sets(phase_0_newline_tracking(
                input,
                &mut string_cache,
            ))),
            string_cache,
        );
        let actual = tokenizer.by_ref().collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }
}
