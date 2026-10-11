use super::{
    Context,
    SourcePosition,
    SourceVectors,
};

/// Reads one stage of the conceptual translation sequence.
/// C99: §5.1.1.2p1-7, pp. 9-10; PDF pp. 21-22.
pub(crate) trait TranslationPhase<'tu>:
    GetPosition + SetPosition + GetSourceFileIndex + SetSourceFileIndex
{
    type Item;
    fn next_item(&mut self, context: &mut Context<'tu>) -> Option<Self::Item>;
}

pub(crate) trait GetPosition {
    fn position(&self, context: &Context<'_>) -> SourcePosition;
    #[inline(always)]
    fn index(&self, context: &Context<'_>) -> usize {
        self.position(context).index
    }
    #[inline(always)]
    fn column(&self, context: &Context<'_>) -> u32 {
        self.position(context).column
    }
}

/// Moves a reader to a source position. The derived setters read the rest
/// of the current position through [`GetPosition`].
pub(crate) trait SetPosition: GetPosition {
    fn set_position(&mut self, position: SourcePosition);
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Only the tokenizer tests move a reader to another line."
        )
    )]
    #[inline(always)]
    fn set_line(&mut self, context: &Context<'_>, line: u32) {
        self.set_position(SourcePosition {
            index: self.index(context),
            line,
            column: self.column(context),
        });
    }
}

pub(crate) trait GetSourceFileIndex {
    fn source_file_index(&self) -> u32;
}

pub(crate) trait SetSourceFileIndex {
    fn set_source_file_index(&mut self, source_file_index: u32);
}

pub(crate) trait GetSourceVectors {
    fn source_vectors(&self, context: &mut Context<'_>) -> SourceVectors;
}

pub(super) trait StrExt {
    /// Returns the character at the given index,
    /// or `None` if the index is out of bounds or is in the middle of a
    /// character.
    fn char_at(&self, index: usize) -> Option<char>;
}

impl StrExt for str {
    fn char_at(&self, index: usize) -> Option<char> {
        self.get(index..)?.chars().next()
    }
}
