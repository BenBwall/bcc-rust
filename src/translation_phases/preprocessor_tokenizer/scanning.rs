//! The lexer reads one spliced byte at a time through [`Lexer::peek`]. Reading
//! past the buffer records final-newline diagnostics and can supply a missing
//! newline. The token scanners share this reading state and return a [`Lexed`]
//! kind and spelling for the entry loop to store.
//!
//! For example, reading the end of a buffer containing `x` supplies one
//! newline. A second read at that position returns no byte until a real
//! character is read again. This preserves the order of token formation and
//! diagnostic replay.
//!
//! Read [`Lexer::peek`], then [`Lexed`], [`Lexer::lex_token`], and
//! [`Lexer::push`]. The whole-file loop is [`Lexer::run`].
//!
//! Files by role:
//! - Token boundaries: `scanning/tokens.rs` selects punctuation, identifiers,
//!   numbers, and other characters.
//! - Quoted tokens and whitespace: `scanning/literals.rs` reads literals,
//!   comments, and whitespace.
//! - Output and state setup: `scanning/spelling.rs` interns spellings, records
//!   entries and diagnostics, and constructs the lexer.
//!
//! C99: §5.1.1.2 paragraph 1 (phase 3), p. 10; PDF p. 22;
//! preprocessing-token categories and maximal munch §6.4 paragraphs 1-4,
//! pp. 49-50; PDF pp. 61-62. This module records token spellings;
//! literal values and phase-7 conversion are checked later.

// Token boundaries and quoted tokens.
mod literals;
mod tokens;

// Entry recording and lexer setup.
mod spelling;

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
