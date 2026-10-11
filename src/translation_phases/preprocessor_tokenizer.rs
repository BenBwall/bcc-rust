//! Whole-file lexing: translation phases 1 through 3.
//!
//! C99: §5.1.1.2p1, p. 9; PDF p. 21; §5.1.1.2p2-3, p. 10; PDF p. 22. Trigraphs
//! follow §5.2.1.1p1, p. 18; PDF p. 30; preprocessing-token boundaries follow
//! §6.4p1-4, pp. 49-50; PDF pp. 61-62.
//!
//! Translation phases 1 and 2 run over the entire buffer first: one
//! vectorized scan finds every byte that could start a trigraph, a line
//! splice, or a carriage return, so a buffer without any is lexed in place
//! and only the rest is copied once. Phase 3 then lexes the spliced text to
//! completion into the struct-of-arrays [`LexedFile`], which
//! [`token_source`] replays to phase 4.
//!
//! Provenance and diagnostics follow reading order: positions refer to the
//! original source, a token after a deleted splice starts at the splice, the
//! newline supplied for a file without a final one is consumed by whichever
//! token first looks at the end of input, and a final newline escaped by a
//! line splice is reported once each time the end of input is read after a
//! real character. Non-newline whitespace is collapsed to one space, the
//! implementation-defined choice in §5.1.1.2p3, p. 10; PDF p. 22.

//! Preprocessing-token formation in translation phase 3.
//!
//! C99: §5.1.1.2p3, p. 10; PDF p. 22; lexical categories and maximal munch are
//! §6.4p1-4, pp. 49-50; PDF pp. 61-62. This lexer does not form header-name
//! tokens; `#include` handling interprets their source spelling in phase 4
//! (§6.4p4, p. 50; PDF p. 62; §6.4.7, pp. 64-65; PDF pp. 76-77).
pub(super) mod errors;
mod positions;
mod replay;
mod scanning;
mod splicing;
mod storage;
mod token;
mod token_source;
pub(crate) mod ucn;
pub(crate) use errors::{
    PreprocessorTokenizerError,
    PreprocessorTokenizerErrorType,
};
use positions::PositionTracker;
pub(crate) use positions::position_after;
use splicing::splice;
pub(crate) use splicing::{
    LogicalCharacter,
    logical_characters,
};
use storage::{
    Entry,
    LexDiagnostic,
    LexedFile,
};
pub(crate) use token::{
    PreprocessorToken,
    PreprocessorTokenType,
};
pub(crate) use token_source::{
    LexedFiles,
    TokenSource,
};

use super::{
    Context,
    SourcePosition,
    SourceVector,
    SourceVectors,
    initial_processing::terminal_splice_length,
    provenance::source_offset,
};
use crate::{
    configuration::Feature,
    util::{
        bump::{
            ArenaVec,
            Bump,
            TailVec,
        },
        string_cache::StringCacheId,
    },
};

impl<'a> LexedFile<'a> {
    /// Runs translation phases 1 through 3 over all of `source`, keeping the
    /// result in `arena`.
    /// C99: §5.1.1.2p1-3, pp. 9-10; PDF pp. 21-22.
    ///
    /// Lexing's temporary storage (text that phases 1 and 2 change, side
    /// tables until they are copied beside the entries, and canonical UCN
    /// spellings) comes from an arena of its own. It reserves a region only
    /// when the file needs one of them, and releases it when the file is
    /// lexed, so a large file's spliced copy does not stay committed for the
    /// rest of preprocessing.
    pub(super) fn lex(
        context: &mut Context<'_>,
        arena: &'a Bump,
        source_file_index: u32,
        source: &str,
    ) -> Self {
        assert!(
            u32::try_from(source.len()).is_ok(),
            "source file exceeds u32::MAX bytes"
        );
        let scratch = Bump::new();
        let trigraphs = context.configuration.accepts(Feature::Trigraphs);
        let (text, remaps) = splice(source, &scratch, trigraphs);
        let terminal_splice = terminal_splice_length(source, trigraphs);
        let file = Lexer::new(context, arena, &scratch, text, remaps, source.is_empty())
            .with_terminal_splice(terminal_splice.is_some())
            .run();
        let escaped_final_newline = terminal_splice.map(|length| {
            let index = source.len() - length;
            let line_start = source[..index].rfind(['\r', '\n']).map_or(0, |i| i + 1);
            SourceVector {
                index: source_offset(index),
                column: u32::try_from(source[line_start..index].chars().count() + 1)
                    .expect("source column exceeds u32::MAX"),
                line: file.eof.line.saturating_sub(1),
                source_file_index,
                length: source_offset(length),
            }
        });
        file.finish(arena, source_file_index, escaped_final_newline)
    }
}

impl<'arena, 's> Lexer<'_, '_, 'arena, 's> {
    fn run(mut self) -> LexingFile<'arena, 's> {
        loop {
            let start = self.pos;
            let position = self.tracker.advance(start);
            self.read_end = false;
            self.read_final_newline = false;
            let Some(byte) = self.peek(start) else {
                break;
            };
            self.reached_eof = false;
            let lexed = if start == self.bytes.len() {
                // The supplied final newline, read at the start of a token.
                self.reached_eof = true;
                self.file.final_newline_entry = Some(self.file.entries.len());
                self.respelled(start, PreprocessorTokenType::Newline, "\n")
            } else {
                self.lex_token(start, position, byte)
            };
            self.push(position, lexed);
        }
        // Reading past the last entry is the cursor's to report.
        self.pending.clear();
        let end = self.tracker.advance(self.bytes.len());
        self.file.eof = self.tracker.advance_past_deletions(self.bytes.len());
        self.file.end_of_tokens = if self.reached_eof { self.file.eof } else { end };
        self.file
    }
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "Each flag is one piece of the end-of-input reading state."
)]
struct Lexer<'a, 'tu, 'arena, 's> {
    /// One-byte spellings recur for punctuation and canonical whitespace.
    /// Cache their IDs lazily to preserve the interner's insertion order.
    ascii:               [Option<StringCacheId>; 128],
    context:             &'a mut Context<'tu>,
    text:                &'a str,
    bytes:               &'a [u8],
    tracker:             PositionTracker<'a>,
    /// Spliced offset of the next token.
    pos:                 usize,
    /// High byte of the line number stored for the previous entry.
    current_line_high:   u8,
    /// Whether the input lacks a final newline, so one is supplied.
    lacks_final_newline: bool,
    /// Whether reading the end of input now yields the supplied newline.
    /// Reading it withholds it until a real character is read again.
    virtual_newline:     bool,
    /// Whether the token being lexed read through the end of input.
    reached_eof:         bool,
    /// Whether the source's final newline is escaped by a line splice.
    terminal_splice:     bool,
    /// Whether reading the end of input now reports the escaped final
    /// newline: until a real character is read again, it is reported once.
    splice_armed:        bool,
    /// Whether the token being lexed looked at the end of input.
    read_end:            bool,
    /// Whether the token being lexed read the supplied final newline.
    read_final_newline:  bool,
    /// Lexing's temporary storage.
    scratch:             &'s Bump,
    /// Diagnostics raised while lexing the current token, in order.
    pending:             ArenaVec<'s, LexDiagnostic>,
    file:                LexingFile<'arena, 's>,
}

/// A [`LexedFile`] while its entries are being lexed. Its side tables grow
/// in the lexing scratch arena until they are copied after the entries.
struct LexingFile<'arena, 's> {
    /// The rest of the arena's reservation while lexing, committed as
    /// entries are written, so it never moves or overcommits.
    entries:               TailVec<'arena, Entry>,
    line_high_starts:      ArenaVec<'s, (u32, u8)>,
    end_of_tokens:         SourcePosition,
    eof:                   SourcePosition,
    diagnostics:           ArenaVec<'s, (u32, LexDiagnostic)>,
    other_locations:       ArenaVec<'s, (u32, SourceVector)>,
    final_newline_entry:   Option<usize>,
    final_newline_readers: ArenaVec<'s, u32>,
    lacks_final_newline:   bool,
    end_readers:           ArenaVec<'s, u32>,
}

#[cfg(test)]
mod batch {
    pub(super) use super::storage::LexedFile;
}

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod tests;
