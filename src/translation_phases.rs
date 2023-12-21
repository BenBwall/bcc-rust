use std::{
    convert::Infallible,
    fmt::Debug,
};

use crate::util::string_cache::Id as StringCacheId;

pub(crate) mod phase_0_newline_tracking;
pub(crate) mod phase_1_map_character_sets;
pub(crate) mod phase_2_remove_escaped_newlines;
pub(crate) mod phase_3_preprocessor_tokenizer;
pub(crate) mod phase_4_preprocessing;

#[doc(hidden)]
#[macro_export]
macro_rules! bail {
    ($e:expr $(,)?) => {
        match $e {
            | Ok(v) => v,
            | Err(e) => return Some(Err(e.into())),
        }
    };
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum ErrorSeverity {
    Warning,
    Error,
    Note,
}

#[derive(PartialEq, Eq, Debug, Clone, Copy, Hash)]
pub(crate) struct Position {
    pub(crate) index:       usize,
    pub(crate) line:        usize,
    pub(crate) column:      usize,
    pub(crate) source_file: StringCacheId,
}

pub(crate) trait SavePoint: Clone + Debug {
    fn current_position(&self) -> Position;
}

impl SavePoint for Position {
    fn current_position(&self) -> Position {
        *self
    }
}

pub(crate) trait GetSeverity {
    fn severity(&self) -> ErrorSeverity;
}

impl GetSeverity for ErrorSeverity {
    fn severity(&self) -> ErrorSeverity {
        *self
    }
}

impl GetSeverity for Infallible {
    fn severity(&self) -> ErrorSeverity {
        match *self {}
    }
}

pub(crate) trait TranslationPhase:
    Iterator<Item = Result<Self::Yield, Self::Error>>
{
    type Yield;
    type SavePoint: SavePoint;
    type Error: std::error::Error + GetSeverity;
    fn save(&self) -> Self::SavePoint;
    fn restore(&mut self, save_point: Self::SavePoint);
    fn current_position(&self) -> Position;
}
