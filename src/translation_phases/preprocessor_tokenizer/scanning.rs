mod literals;
mod spelling;
mod tokens;
use super::{
    Lexer,
    PreprocessorTokenType,
    storage::LexDiagnostic,
    ucn,
};
use crate::util::string_cache::StringCacheId;

impl Lexer<'_, '_, '_, '_> {
    /// The spliced byte at `offset`. Looking at the end of input reads the
    /// supplied final newline, reporting its absence, or reports a final
    /// newline that a line splice escapes.
    #[inline(always)]
    pub(super) fn peek(&mut self, offset: usize) -> Option<u8> {
        if let Some(&byte) = self.bytes.get(offset) {
            self.virtual_newline = self.lacks_final_newline;
            self.splice_armed = true;
            return Some(byte);
        }
        self.read_end = true;
        if self.terminal_splice && self.splice_armed {
            self.splice_armed = false;
            self.pending.push(LexDiagnostic::EscapedFinalNewline);
        }
        if offset == self.bytes.len() && self.virtual_newline {
            self.virtual_newline = false;
            self.read_final_newline = true;
            if !self.terminal_splice {
                self.pending.push(LexDiagnostic::MissingFinalNewline);
            }
            return Some(b'\n');
        }
        None
    }
}

/// A token's kind and spelling once its end is known.
#[derive(Clone, Copy)]
pub(super) struct Lexed {
    pub(super) kind:     Option<PreprocessorTokenType>,
    pub(super) contents: StringCacheId,
}
