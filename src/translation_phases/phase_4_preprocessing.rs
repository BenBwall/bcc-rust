use std::sync::Arc;

use bigdecimal::{
    num_bigint::BigInt,
    num_traits::Pow,
    BigDecimal,
};

use crate::{
    util::string_cache::{
        Id as StringCacheId,
        StringCache,
    },
    HashMap,
};

trait StrExt {
    /// Returns the character at the given index,
    /// or `None` if the index is out of bounds or index is in the middle of a
    /// character.
    fn char_at(&self, index: usize) -> Option<char>;
}

impl StrExt for str {
    fn char_at(&self, index: usize) -> Option<char> {
        self.get(index..)?.chars().next()
    }
}

use super::{
    phase_3_preprocessor_tokenizer::{
        PreprocessorToken,
        PreprocessorTokenType,
        PreprocessorTokenizerError,
    },
    ErrorSeverity,
    GetSeverity,
    Position,
    TranslationPhase,
};

const PREDEFINED_MACRO_NAMES: [&str; 4] = ["__LINE__", "__FILE__", "__DATE__", "__TIME__"];

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) enum TokenizerFrameType {
    IncludedHeader,
    ObjectLikeMacroInvocation,
    FunctionLikeMacroInvocation {
        argument_names: Arc<Vec<StringCacheId>>,
    },
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) enum MacroDefinition<SavePoint> {
    ObjectLike {
        start_save_point: SavePoint,
    },
    FunctionLike {
        argument_names:   Arc<Vec<StringCacheId>>,
        start_save_point: SavePoint,
    },
    BuiltIn,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct TokenizerFrame<InnerSavePoint> {
    frame_type: TokenizerFrameType,
    save_point: InnerSavePoint,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct SavePoint<InnerSavePoint> {
    pub(crate) inner:           InnerSavePoint,
    pub(crate) tokenizer_stack: Vec<TokenizerFrame<InnerSavePoint>>,
    pub(crate) state:           State,
}

impl<Inner> super::SavePoint for SavePoint<Inner>
where
    Inner: super::SavePoint,
{
    fn current_position(&self) -> Position {
        self.inner.current_position()
    }
}

pub(crate) struct Token {
    pub(crate) token_type:     TokenType,
    pub(crate) start_position: Position,
    pub(crate) contents:       StringCacheId,
}

pub(crate) enum IntegerTokenType {
    Short(i16),
    Int(i32),
    Long(i64),
    LongLong(i64),
    UnsignedShort(u16),
    UnsignedInt(u32),
    UnsignedLong(u64),
    UnsignedLongLong(u64),
    Int128(i128),
    UInt128(u128),
}

pub(crate) enum FloatTokenType {
    Float(f32),
    Double(f64),
    LongDouble(BigDecimal),
}

pub(crate) enum TokenType {
    Integer(IntegerTokenType),
    Float(FloatTokenType),
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum State {
    Normal,
    FloatingPointLiteralExponentOverflow,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct InnerPreprocessorError {
    pub(crate) error_type:     PreprocessorErrorType,
    pub(crate) start_position: Position,
    pub(crate) contents:       StringCacheId,
}

impl GetSeverity for InnerPreprocessorError {
    fn severity(&self) -> ErrorSeverity {
        match self.error_type {
            | PreprocessorErrorType::InvalidHexadecimalFloatLiteral => ErrorSeverity::Error,
            | PreprocessorErrorType::InvalidDecimalFloatLiteral => ErrorSeverity::Error,
            | PreprocessorErrorType::FloatingPointLiteralExponentOverflow => ErrorSeverity::Error,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) enum PreprocessorError<PrevError> {
    PreviousPhaseError(PrevError),
    InnerPreprocessorError(InnerPreprocessorError),
}

impl<PrevError> GetSeverity for PreprocessorError<PrevError>
where
    PrevError: GetSeverity,
{
    fn severity(&self) -> ErrorSeverity {
        match self {
            | Self::PreviousPhaseError(e) => e.severity(),
            | Self::InnerPreprocessorError(e) => e.severity(),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum PreprocessorErrorType {
    InvalidHexadecimalFloatLiteral,
    InvalidDecimalFloatLiteral,
    FloatingPointLiteralExponentOverflow,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) struct Preprocessor<Prev, PrevSavePoint> {
    previous_phase: Prev,
    tokenizer_stack: Vec<TokenizerFrame<PrevSavePoint>>,
    object_like_macro_definitions:
        HashMap<StringCacheId, MacroDefinition<SavePoint<PrevSavePoint>>>,
    state: State,
}

impl<Prev, PrevSavePoint> Preprocessor<Prev, PrevSavePoint> {
    pub(crate) fn new(previous_phase: Prev, string_cache: &mut StringCache) -> Self {
        Self {
            previous_phase,
            tokenizer_stack: Vec::new(),
            object_like_macro_definitions: PREDEFINED_MACRO_NAMES
                .into_iter()
                .map(
                    |s| -> (StringCacheId, MacroDefinition<SavePoint<PrevSavePoint>>) {
                        (string_cache.intern(s), MacroDefinition::BuiltIn)
                    },
                )
                .collect(),
            state: State::Normal,
        }
    }
}

impl<PrevPrevError, Prev> Preprocessor<Prev, Prev::SavePoint>
where
    Prev: TranslationPhase<
            Yield = PreprocessorToken,
            Error = PreprocessorTokenizerError<PrevPrevError>,
        > + AsMut<StringCache>
        + AsRef<StringCache>,
{
    fn map_preprocessor_token(
        &mut self,
        token: PreprocessorToken,
    ) -> Result<Token, PreprocessorError<Prev::Error>> {
        let save_point = self.previous_phase.save();
        let contents = self
            .previous_phase
            .as_ref()
            .get(token.contents)
            .unwrap_or_else(|| {
                panic!(
                    "Compiler bug: PreprocessorToken::contents is out of bounds: {} > {}",
                    token.contents.to_usize(),
                    self.previous_phase.as_ref().len()
                )
            });
        match token.token_type {
            | PreprocessorTokenType::Number => {
                let is_hex = contents.starts_with("0x") || contents.starts_with("0X");
                let is_binary = contents.starts_with("0b") || contents.starts_with("0B");
                let is_octal = contents.starts_with('0') && !is_hex && !is_binary;
                if is_hex {
                    if contents.contains(|c| c == '.' || c == 'p' || c == 'P') {
                        self.parse_hexadecimal_float(token, contents)
                    } else {
                        self.parse_hexadecimal_integer(token, contents)
                    }
                } else if is_binary {
                    self.parse_binary_integer(token, contents)
                } else if contents.contains(|c| c == '.' || c == 'e' || c == 'E') {
                    self.parse_float(token, contents)
                } else if is_octal {
                    self.parse_octal_integer(token, contents)
                } else {
                    self.parse_decimal_integer(token, contents)
                }
            },
            | _ => todo!(),
        }
        todo!()
    }

    fn parse_hexadecimal_float(
        &mut self,
        token: PreprocessorToken,
        contents: &str,
    ) -> Result<Token, PreprocessorError<Prev::Error>> {
        let mut result = BigDecimal::from(0);
        let mut index = 2;
        while contents.char_at(index).map_or(false, |c| c.is_digit(16)) {
            let digit = contents.char_at(index).unwrap().to_digit(16).unwrap();
            result *= 16;
            result += digit;
            index += 1;
        }
        if contents.char_at(index) == Some('.') {
            index += 1;
        }
        if matches!(contents.char_at(index), Some('p' | 'P')) {
            let exponent = (|| {
                index += 1;
                let sign = match contents.char_at(index) {
                    | Some('+') => {
                        index += 1;
                        1
                    },
                    | Some('-') => {
                        index += 1;
                        -1
                    },
                    | _ => 1,
                };

                let mut exponent = 0isize;
                while contents.char_at(index).map_or(false, |c| c.is_digit(10)) {
                    let digit = contents.char_at(index).unwrap().to_digit(10).unwrap();
                    exponent = exponent.checked_mul(10)?;
                    exponent = exponent.checked_add(digit as _)?;
                    index += 1;
                }
                exponent *= sign;
                Some(exponent)
            })();
            if let Some(exponent) = exponent {
                if exponent < 0 {
                    let exponent: BigInt = if exponent == isize::MIN {
                        BigInt::from(16).pow((isize::MAX + 1) as usize)
                    } else {
                        BigInt::from(16).pow((-exponent) as usize)
                    };
                    match <u128 as TryFrom<BigInt>>::try_from(exponent) {
                        | Ok(exponent) => {
                            result = result / exponent;
                        },
                        | Err(_) => {
                            result *= 0;
                        },
                    }
                    let exponent = match TryInto::<u128>::try_into(exponent) {
                        | Ok(exponent) => exponent,
                        | Err(_) => {
                            return Err(PreprocessorError::InnerPreprocessorError(
                                InnerPreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::FloatingPointLiteralExponentOverflow,
                                    start_position: token.start_position,
                                    contents:       token.contents,
                                },
                            ));
                        },
                    };
                    result = result / exponent;
                } else {
                    let exponent: BigInt = 16.pow(exponent as usize);
                    result *= exponent;
                }
            } else {
                return Err(PreprocessorError::InnerPreprocessorError(
                    InnerPreprocessorError {
                        error_type:     PreprocessorErrorType::InvalidHexadecimalFloatLiteral,
                        start_position: token.start_position,
                        contents:       token.contents,
                    },
                ));
            }
        }
        if index != contents.len() {
            todo!();
        }
        todo!()
    }
}

impl<PrevPrevError, Prev> Iterator for Preprocessor<Prev, Prev::SavePoint>
where
    Prev: TranslationPhase<
            Yield = PreprocessorToken,
            Error = PreprocessorTokenizerError<PrevPrevError>,
        > + AsMut<StringCache>
        + AsRef<StringCache>,
{
    type Item = Result<Token, PreprocessorError<Prev::Error>>;

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.previous_phase.size_hint()
    }
}

impl<PrevPrevError, Prev> TranslationPhase for Preprocessor<Prev, Prev::SavePoint>
where
    Prev: TranslationPhase<
            Yield = PreprocessorToken,
            Error = PreprocessorTokenizerError<PrevPrevError>,
        > + AsMut<StringCache>
        + AsRef<StringCache>,
{
    type Error = PreprocessorError<Prev::Error>;
    type SavePoint = SavePoint<Prev::SavePoint>;
    type Yield = Token;

    fn save(&self) -> Self::SavePoint {
        SavePoint {
            inner:           self.previous_phase.save(),
            tokenizer_stack: self.tokenizer_stack.clone(),
            state:           self.state,
        }
    }

    fn restore(&mut self, save_point: Self::SavePoint) {
        self.previous_phase.restore(save_point.inner);
        self.tokenizer_stack = save_point.tokenizer_stack;
        self.state = save_point.state;
    }

    fn current_position(&self) -> Position {
        self.previous_phase.current_position()
    }
}
