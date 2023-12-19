use std::{convert::Infallible, sync::Arc};

use crate::{
    util::string_cache::{Id, StringCache},
    HashMap,
};

use super::{
    phase_3_preprocessor_tokenizer::{PreprocessorToken, PreprocessorTokenizerError, PreprocessorTokenType},
    Position, TranslationPhase,
};

const PREDEFINED_MACRO_NAMES: [&str; 4] = ["__LINE__", "__FILE__", "__DATE__", "__TIME__"];

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) enum TokenizerFrameType {
    IncludedHeader,
    ObjectLikeMacroInvocation,
    FunctionLikeMacroInvocation { argument_names: Arc<Vec<Id>> },
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) enum MacroDefinition<SavePoint> {
    ObjectLike {
        start_save_point: SavePoint,
    },
    FunctionLike {
        argument_names: Arc<Vec<Id>>,
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
    pub(crate) inner: InnerSavePoint,
    pub(crate) tokenizer_stack: Vec<TokenizerFrame<InnerSavePoint>>,
}

impl<Inner> super::SavePoint for SavePoint<Inner>
where
    Inner: super::SavePoint,
{
    fn current_position(&self) -> Position {
        self.inner.current_position()
    }
}

pub(crate) struct Token<InnerSavePoint> {
    pub(crate) token_type: TokenType,
    pub(crate) save_point: InnerSavePoint,
}

pub(crate) enum TokenType {}

pub(crate) struct PreprocessorError<InnerSavePoint> {
    pub(crate) error_type: PreprocessorErrorType,
    pub(crate) save_point: InnerSavePoint,
}

pub(crate) enum PreprocessorErrorType {}

#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) struct Preprocessor<Prev, PrevSavePoint> {
    previous_phase: Prev,
    tokenizer_stack: Vec<TokenizerFrame<PrevSavePoint>>,
    object_like_macro_definitions: HashMap<Id, MacroDefinition<SavePoint<PrevSavePoint>>>,
}

impl<Prev, PrevSavePoint> Preprocessor<Prev, PrevSavePoint> {
    pub(crate) fn new(previous_phase: Prev, string_cache: &mut StringCache) -> Self {
        Self {
            previous_phase,
            tokenizer_stack: Vec::new(),
            object_like_macro_definitions: PREDEFINED_MACRO_NAMES
                .into_iter()
                .map(|s| -> (Id, MacroDefinition<SavePoint<PrevSavePoint>>) {
                    (string_cache.intern(s), MacroDefinition::BuiltIn)
                })
                .collect(),
        }
    }
}

impl<Prev> Preprocessor<Prev, Prev::SavePoint> where Prev : TranslationPhase + Iterator<Item = Result<PreprocessorToken<Prev::SavePoint>, PreprocessorTokenizerError<Prev::SavePoint>>> {
    fn map_preprocessor_token(&mut self, token: PreprocessorToken<Prev::SavePoint>) -> Result<Token<SavePoint<Prev::SavePoint>>, PreprocessorError<SavePoint<Prev::SavePoint>>> {
        match token.token_type {
            PreprocessorTokenType::Number => {
            
            }
        }
    }
    fn validate_hexadecimal_float(&mut self, token: PreprocessorToken<Prev::SavePoint>) -> Result<Token<SavePoint<Prev::SavePoint>>, PreprocessorError<SavePoint<Prev::SavePoint>>> {
        todo!()
    }
}

impl<Prev> Iterator for Preprocessor<Prev, Prev::SavePoint>
where
    Prev: TranslationPhase
        + Iterator<
            Item = Result<
                PreprocessorToken<Prev::SavePoint>,
                PreprocessorTokenizerError<Prev::SavePoint>,
            >,
        >,
{
    type Item =
        Result<Token<SavePoint<Prev::SavePoint>>, PreprocessorError<SavePoint<Prev::SavePoint>>>;

    fn next(&mut self) -> Option<Self::Item> {


        todo!()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.previous_phase.size_hint()
    }
}

impl<Prev> TranslationPhase for Preprocessor<Prev, Prev::SavePoint>
where
    Prev: TranslationPhase
        + Iterator<
            Item = Result<
                PreprocessorToken<Prev::SavePoint>,
                PreprocessorTokenizerError<Prev::SavePoint>,
            >,
        >,
{
    type SavePoint = SavePoint<Prev::SavePoint>;
    fn save(&self) -> Self::SavePoint {
        SavePoint {
            inner: self.previous_phase.save(),
            tokenizer_stack: self.tokenizer_stack.clone(),
        }
    }
    fn restore(&mut self, save_point: Self::SavePoint) {
        self.previous_phase.restore(save_point.inner);
        self.tokenizer_stack = save_point.tokenizer_stack;
    }
    fn current_position(&self) -> Position {
        self.previous_phase.current_position()
    }
}
