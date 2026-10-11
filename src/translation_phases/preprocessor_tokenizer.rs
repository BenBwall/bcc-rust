//! Each source buffer is processed once through translation phases 1-3.
//! Trigraph replacement, newline mapping, and line splicing run first. The
//! lexer then reads the resulting text from left to right, records token
//! boundaries and original source positions, and saves diagnostics for replay
//! when phase 4 reads each token. Comments and horizontal whitespace become one
//! space; newlines stay.
//!
//! For example, `ab` followed by a backslash, a physical newline, and `cd + 1`
//! becomes `abcd + 1` after splicing. The lexing loop records an identifier, a
//! space, `+`, a space, and a preprocessing number. Their positions still refer
//! to the physical source. [`TokenSource`] reads these saved entries without
//! lexing again.
//!
//! Read [`LexedFile::lex`] first, then [`Lexer::run`], [`Lexer`], and
//! [`LexingFile`]. Follow [`splicing::splice`] for phases 1-2,
//! [`Lexer::lex_token`] for token selection, and [`LexingFile::finish`] for
//! packed storage. [`TokenSource`] and [`LexedFiles`] connect the saved file to
//! phase 4.
//!
//! Files by role:
//! - Source mapping: `splicing.rs` replaces trigraphs and deletes splices;
//!   `positions.rs` maps offsets back to physical source positions.
//! - Scanning: `scanning.rs` reads bytes and handles the end of input;
//!   `scanning/tokens.rs` selects tokens, identifiers, and numbers;
//!   `scanning/literals.rs` reads quoted tokens and comments;
//!   `scanning/spelling.rs` records entries, spellings, and diagnostics;
//!   `ucn.rs` canonicalizes identifier universal character names.
//! - Packed storage: `storage.rs` packs entries and side tables.
//! - Token sources: `token_source.rs` reads lexed files; `replay.rs` reads
//!   tokens made in phase 4.
//! - Token and diagnostic types: `token.rs` defines preprocessing tokens;
//!   `errors.rs` defines lexical and final-newline diagnostics.
//! - Tests: `tests.rs` checks lexer snapshots, replay, and preprocessing;
//!   `storage.rs` also holds the packed-entry layout test.
//!
//! C99: §5.1.1.2 paragraph 1 (phases 1-3), pp. 9-10; PDF pp. 21-22;
//! trigraph replacement §5.2.1.1 paragraph 1, p. 18; PDF p. 30;
//! lexical categories and maximal munch §6.4 paragraphs 1-4, pp. 49-50;
//! PDF pp. 61-62. Collapsing horizontal whitespace is the
//! implementation-defined choice in phase 3. Header names are interpreted by
//! phase-4 `#include` handling (§6.4 paragraph 4, p. 50; PDF p. 62; §6.4.7, pp.
//! 64-65; PDF pp. 76-77). Escape values and conversion to C tokens belong to
//! later phases.

// Source mapping.
mod positions;
mod splicing;

// Token scanning.
mod scanning;
pub(crate) mod ucn;

// Packed storage.
mod storage;

// Token sources.
mod replay;
mod token_source;

// Token and diagnostic types.
pub(super) mod errors;
mod token;

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

// Tests.
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod tests;
