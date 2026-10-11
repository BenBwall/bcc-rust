//! Saved phase-3 tokens and the side tables needed to read them in phase 4.
//! [`LexingFile::finish`] keeps exactly the entries written and copies the side
//! tables beside them. [`LexedFile`] stores packed positions, spellings, and
//! deferred diagnostics. Cursor movement belongs to [`super::token_source`];
//! this module does not expand macros or convert preprocessing tokens to C
//! tokens.
//!
//! C99: §5.1.1.2 paragraph 1 (phase 3), p. 10; PDF p. 22;
//! preprocessing-token categories §6.4 paragraphs 1-3, p. 49; PDF p. 61.

use std::cell::Cell;

use super::{
    Lexer,
    LexingFile,
    PreprocessorTokenType,
    PreprocessorTokenizerError,
    PreprocessorTokenizerErrorType,
};
use crate::{
    configuration::Feature,
    translation_phases::{
        Context,
        SourcePosition,
        SourceVector,
        provenance::source_offset,
    },
    util::{
        bump::Bump,
        string_cache::StringCacheId,
    },
};

impl<'arena> LexingFile<'arena, '_> {
    /// Keeps exactly the entries written, returning the rest of the
    /// reservation to `arena`, and stores the side tables after them.
    pub(super) fn finish(
        self,
        arena: &'arena Bump,
        source_file_index: u32,
        escaped_final_newline: Option<SourceVector>,
    ) -> LexedFile<'arena> {
        let Self {
            entries,
            line_high_starts,
            end_of_tokens,
            eof,
            diagnostics,
            other_locations,
            final_newline_entry,
            final_newline_readers,
            lacks_final_newline,
            end_readers,
        } = self;
        LexedFile {
            source_file_index,
            registration: None,
            entries: entries.into_slice(),
            line_high_starts: arena.alloc_slice_copy(&line_high_starts),
            end_of_tokens,
            eof,
            diagnostics: arena.alloc_slice_copy(&diagnostics),
            extensions_reported: arena
                .alloc_slice_fill_iter(diagnostics.iter().map(|_| Cell::new(false))),
            other_locations: arena.alloc_slice_fill_iter(other_locations),
            final_newline_entry,
            final_newline_readers: arena.alloc_slice_copy(&final_newline_readers),
            lacks_final_newline,
            escaped_final_newline,
            end_readers: arena.alloc_slice_copy(&end_readers),
        }
    }
}

/// Every preprocessing token of one source buffer, in order. Entry `i` ends
/// where entry `i + 1` starts. Other-token provenance can exclude deleted
/// splices without changing these shared boundaries.
///
/// Lexing appends entries to one array at the end of the caller's arena,
/// whose capacity is the rest of the arena's reservation. Nothing else is
/// allocated there while a file is lexed, so the array never moves, and
/// pages are committed as entries are written. The finished file keeps its
/// length and gives the rest back: a file costs one exact-size array,
/// written once, and holds no storage of its own.
/// C99: preprocessing-token formation §5.1.1.2p3, p. 10; PDF p. 22; lexical
/// categories §6.4p1-3, p. 49; PDF p. 61.
pub(super) struct LexedFile<'a> {
    pub(super) source_file_index:     u32,
    /// The file's index in the preprocessor's registry of opened files, if
    /// it was opened there rather than for temporary use.
    pub(super) registration:          Option<u32>,
    pub(super) entries:               &'a [Entry],
    /// Entry indices where the high byte of the source line changes.
    pub(super) line_high_starts:      &'a [(u32, u8)],
    /// Where the last entry ends.
    pub(super) end_of_tokens:         SourcePosition,
    /// Where reading past the last entry stands: past any trailing splices.
    pub(super) eof:                   SourcePosition,
    /// Sorted by entry.
    pub(super) diagnostics:           &'a [(u32, LexDiagnostic)],
    /// Whether each extension diagnostic, by its index in `diagnostics`, was
    /// reported. Its spelling is written once, so however often phase 4
    /// reads the token (in a macro body, an argument prescan, or after a
    /// lookahead rewinds), the diagnostic is reported once; reading it in a
    /// skipped group, with tokenizer diagnostics ignored, does not count.
    pub(super) extensions_reported:   &'a [Cell<bool>],
    /// Exact character spans for Other tokens, sorted by entry. Normal token
    /// spans still use the adjacent entry boundaries above.
    pub(super) other_locations:       &'a [(u32, SourceVector)],
    /// The entry for a missing final newline read at the start of a token.
    pub(super) final_newline_entry:   Option<usize>,
    /// Entries whose lexing read the supplied final newline while looking
    /// ahead. Sorted, and empty unless the final newline is missing.
    pub(super) final_newline_readers: &'a [u32],
    pub(super) lacks_final_newline:   bool,
    /// The splice that escapes the source's final newline, with its line
    /// given before any `#line` renumbering.
    pub(super) escaped_final_newline: Option<SourceVector>,
    /// Entries whose lexing read the end of input. Sorted, and empty unless
    /// the final newline is escaped.
    pub(super) end_readers:           &'a [u32],
}

/// One lexed entry: a preprocessing token, or whitespace or a comment
/// (`kind` is `None`), and where it starts.
/// C99: phase-3 decomposition §5.1.1.2p3, p. 10; PDF p. 22.
///
/// The low three bytes of the source line leave a byte for the kind in a
/// 16-byte, naturally aligned entry. The containing file records where the
/// high byte changes, preserving the full `u32` source-position range.
#[derive(Clone, Copy)]
#[repr(C)]
pub(super) struct Entry {
    pub(super) contents: StringCacheId,
    /// Byte offset of the entry's start in the source.
    pub(super) index:    u32,
    pub(super) column:   u32,
    pub(super) line:     [u8; 3],
    pub(super) kind:     Option<PreprocessorTokenType>,
}

/// A diagnostic raised while lexing, replayed whenever its token is read.
#[derive(Clone, Copy, Debug)]
pub(super) enum LexDiagnostic {
    Extension {
        feature:  Feature,
        spelling: &'static str,
        start:    SourcePosition,
        length:   usize,
    },
    Tokenizer {
        error_type: PreprocessorTokenizerErrorType,
        start:      SourcePosition,
        length:     usize,
        character:  Option<char>,
    },
    MissingFinalNewline,
    EscapedFinalNewline,
}

impl<'a> LexedFile<'a> {
    /// Reports the diagnostics recorded while lexing `entry`.
    #[inline(always)]
    pub(super) fn replay_diagnostics(
        &self,
        context: &mut Context<'_>,
        entry: usize,
        source_file_index: u32,
        line_delta: u32,
    ) {
        if self.diagnostics.is_empty() {
            return;
        }
        self.replay_diagnostics_slow(context, entry, source_file_index, line_delta);
    }

    #[cold]
    #[inline(never)]
    fn replay_diagnostics_slow(
        &self,
        context: &mut Context<'_>,
        entry: usize,
        source_file_index: u32,
        line_delta: u32,
    ) {
        let entry = u32::try_from(entry).expect("lexed token index exceeds u32::MAX");
        let first = self
            .diagnostics
            .partition_point(|(owner, _)| *owner < entry);
        for ((_, diagnostic), reported) in self.diagnostics[first..]
            .iter()
            .zip(&self.extensions_reported[first..])
            .take_while(|((owner, _), _)| *owner == entry)
        {
            match *diagnostic {
                | LexDiagnostic::Extension {
                    feature,
                    spelling,
                    mut start,
                    length,
                } =>
                    if !context.ignore_tokenizer_errors() && !reported.replace(true) {
                        start.line = start.line.wrapping_add(line_delta);
                        let vectors =
                            context.create_source_vectors(start, source_file_index, length);
                        context.report_extension(feature, spelling, vectors);
                    },
                | LexDiagnostic::Tokenizer {
                    error_type,
                    start,
                    length,
                    character,
                } => context.preprocessor_tokenizer_error(PreprocessorTokenizerError {
                    source_vector: SourceVector {
                        index: source_offset(start.index),
                        column: start.column,
                        line: start.line.wrapping_add(line_delta),
                        source_file_index,
                        length: source_offset(length),
                    },
                    error_type,
                    character,
                }),
                | LexDiagnostic::MissingFinalNewline =>
                    self.report_missing_final_newline(context, source_file_index, line_delta),
                | LexDiagnostic::EscapedFinalNewline =>
                    self.report_escaped_final_newline(context, source_file_index, line_delta),
            }
        }
    }

    /// GCC `-D` definitions enter at translation phase 3: backslashes and
    /// trigraph spellings on argv are already characters, not physical source
    /// to splice or translate. Keep the ordinary lexer and provenance format.
    /// C99: command-line extension to §5.1.1.2p3, p. 10; PDF p. 22.
    pub(super) fn lex_command_line(
        context: &mut Context<'_>,
        arena: &'a Bump,
        source_file_index: u32,
        source: &str,
    ) -> Self {
        let scratch = Bump::new();
        Lexer::new(context, arena, &scratch, source, &[], source.is_empty())
            .run()
            .finish(arena, source_file_index, None)
    }

    /// A copy of this file in `arena`, outside any registry.
    pub(super) fn copy_into<'b>(&self, arena: &'b Bump) -> LexedFile<'b> {
        LexedFile {
            source_file_index:     self.source_file_index,
            registration:          None,
            entries:               arena.alloc_slice_copy(self.entries),
            line_high_starts:      arena.alloc_slice_copy(self.line_high_starts),
            end_of_tokens:         self.end_of_tokens,
            eof:                   self.eof,
            diagnostics:           arena.alloc_slice_copy(self.diagnostics),
            extensions_reported:   arena.alloc_slice_fill_iter(
                self.extensions_reported
                    .iter()
                    .map(|reported| Cell::new(reported.get())),
            ),
            other_locations:       arena
                .alloc_slice_fill_iter(self.other_locations.iter().cloned()),
            final_newline_entry:   self.final_newline_entry,
            final_newline_readers: arena.alloc_slice_copy(self.final_newline_readers),
            lacks_final_newline:   self.lacks_final_newline,
            escaped_final_newline: self.escaped_final_newline.clone(),
            end_readers:           arena.alloc_slice_copy(self.end_readers),
        }
    }

    /// Reports the escaped final newline, as reading the end of input does
    /// once after each real character.
    pub(super) fn report_escaped_final_newline(
        &self,
        context: &mut Context<'_>,
        source_file_index: u32,
        line_delta: u32,
    ) {
        if let Some(vector) = &self.escaped_final_newline {
            context.escaped_final_newline(SourceVector {
                line: vector.line.wrapping_add(line_delta),
                source_file_index,
                ..vector.clone()
            });
        }
    }

    /// Whether the source's final newline is escaped by a line splice.
    pub(super) fn has_escaped_final_newline(&self) -> bool {
        self.escaped_final_newline.is_some()
    }

    /// Whether lexing `entry` read the end of input, which reports an
    /// escaped final newline until a real character is read again.
    pub(super) fn reads_end(&self, entry: usize) -> bool {
        self.end_readers
            .binary_search(&u32::try_from(entry).expect("lexed token index exceeds u32::MAX"))
            .is_ok()
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    #[inline(always)]
    pub(super) fn kind(&self, entry: usize) -> Option<PreprocessorTokenType> {
        self.entries[entry].kind()
    }

    #[inline(always)]
    pub(super) fn contents(&self, entry: usize) -> StringCacheId {
        self.entries[entry].contents()
    }

    /// Where `entry` starts, or where the last entry ends for `len()`.
    #[inline(always)]
    pub(super) fn start(&self, entry: usize) -> SourcePosition {
        let Some(value) = self.entries.get(entry) else {
            return self.end_of_tokens;
        };
        let mut position = value.start();
        if self
            .line_high_starts
            .first()
            .is_some_and(|&(first, _)| entry >= first as usize)
        {
            position.line |= self.line_high_bits(entry);
        }
        position
    }

    #[cold]
    #[inline(never)]
    fn line_high_bits(&self, entry: usize) -> u32 {
        let after = self
            .line_high_starts
            .partition_point(|&(index, _)| (index as usize) <= entry);
        u32::from(self.line_high_starts[after - 1].1) << 24
    }

    #[inline(always)]
    pub(super) fn end_index(&self, entry: usize) -> usize {
        self.entries
            .get(entry + 1)
            .map_or(self.end_of_tokens.index, |next| next.index() as usize)
    }

    /// The actual character span, excluding any splice before an Other token.
    pub(super) fn other_location(&self, entry: usize) -> &SourceVector {
        let entry = u32::try_from(entry).expect("lexed token index exceeds u32::MAX");
        let found = self
            .other_locations
            .binary_search_by_key(&entry, |&(index, _)| index)
            .expect("Other tokens have exact character spans");
        &self.other_locations[found].1
    }

    /// Whether reading `entry` withholds the supplied final newline, because
    /// the last character it read was a newline.
    #[inline(always)]
    pub(super) fn withholds_final_newline_after(&self, entry: usize) -> bool {
        self.kind(entry) == Some(PreprocessorTokenType::Newline)
            || (!self.final_newline_readers.is_empty()
                && self
                    .final_newline_readers
                    .binary_search(
                        &u32::try_from(entry).expect("lexed token index exceeds u32::MAX"),
                    )
                    .is_ok())
    }

    /// Whether the source does not end in a newline, so reading at its end
    /// supplies one.
    pub(super) fn lacks_final_newline(&self) -> bool {
        self.lacks_final_newline
    }

    /// Reports the missing final newline at the end of input.
    pub(super) fn report_missing_final_newline(
        &self,
        context: &mut Context<'_>,
        source_file_index: u32,
        line_delta: u32,
    ) {
        context.missing_final_newline(SourceVector {
            index: source_offset(self.eof.index),
            column: self.eof.column,
            line: self.eof.line.wrapping_add(line_delta),
            source_file_index,
            length: 0,
        });
    }

    /// Whether `entry` is the supplied final newline itself.
    #[inline(always)]
    pub(super) fn is_final_newline(&self, entry: usize) -> bool {
        self.final_newline_entry == Some(entry)
    }

    pub(super) fn eof(&self) -> SourcePosition {
        self.eof
    }

    /// The entry starting at `position`, `len()` for the end of the last
    /// entry, or `None` when `position` is inside an entry.
    pub(super) fn boundary(&self, position: SourcePosition) -> Option<usize> {
        let target = u32::try_from(position.index).ok()?;
        let entry = self.entries.partition_point(|entry| entry.index() < target);
        (self.start(entry).index == position.index && self.start(entry).column == position.column)
            .then_some(entry)
    }

    /// The first entry starting at or after `position`.
    pub(super) fn entry_after(&self, position: SourcePosition) -> usize {
        self.entries
            .partition_point(|entry| (entry.index() as usize) < position.index)
    }
}

impl Entry {
    #[inline(always)]
    fn kind(self) -> Option<PreprocessorTokenType> {
        self.kind
    }

    #[inline(always)]
    fn contents(self) -> StringCacheId {
        self.contents
    }

    #[inline(always)]
    fn index(self) -> u32 {
        self.index
    }

    #[inline(always)]
    fn start(self) -> SourcePosition {
        SourcePosition {
            index:  self.index as usize,
            line:   u32::from_le_bytes([self.line[0], self.line[1], self.line[2], 0]),
            column: self.column,
        }
    }
}

#[cfg(test)]
mod entry_layout_tests {
    use super::*;

    #[test]
    fn entries_keep_full_source_lines_across_the_inline_limit() {
        const INLINE_LINE_MAX: u32 = 0x00FF_FFFF;
        assert_eq!(size_of::<Entry>(), 16);
        assert_eq!(align_of::<Entry>(), 4);

        let tu = Bump::new();
        let mut context = Context::new(&tu);
        let arena = Bump::new();
        let scratch = Bump::new();
        let mut lexer = Lexer::new(&mut context, &arena, &scratch, "a\nb\nc\n", &[], false);
        lexer.tracker.position.line = INLINE_LINE_MAX - 1;
        let file = lexer.run().finish(&arena, 0, None);
        assert_eq!(file.start(0).line, INLINE_LINE_MAX - 1);
        assert_eq!(file.start(2).line, INLINE_LINE_MAX);
        assert_eq!(file.start(4).line, INLINE_LINE_MAX + 1);
        assert_eq!(file.start(file.len()).line, INLINE_LINE_MAX + 2);
        assert_eq!(file.line_high_starts.len(), 1);

        let copy_arena = Bump::new();
        let copy = file.copy_into(&copy_arena);
        assert_eq!(copy.start(2).line, INLINE_LINE_MAX);
        assert_eq!(copy.start(4).line, INLINE_LINE_MAX + 1);
    }
}
