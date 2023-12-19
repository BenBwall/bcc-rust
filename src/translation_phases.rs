use std::{fmt::Debug, iter::Peekable};

use crate::util::string_cache::Id;

pub(crate) mod phase_0_newline_tracking;
pub(crate) mod phase_1_map_character_sets;
pub(crate) mod phase_2_remove_escaped_newlines;
pub(crate) mod phase_3_preprocessor_tokenizer;
pub(crate) mod phase_4_preprocessing;

#[derive(PartialEq, Eq, Debug, Clone, Copy, Hash)]
pub(crate) struct Position {
    pub(crate) index: usize,
    pub(crate) line: usize,
    pub(crate) column: usize,
    pub(crate) source_file: Id,
}

pub(crate) trait SavePoint: Clone + Debug {
    fn current_position(&self) -> Position;
}

impl SavePoint for Position {
    fn current_position(&self) -> Position {
        *self
    }
}

pub(crate) trait TranslationPhase: Iterator {
    type SavePoint: SavePoint;
    fn save(&self) -> Self::SavePoint;
    fn restore(&mut self, save_point: Self::SavePoint);
    fn current_position(&self) -> Position;
}
