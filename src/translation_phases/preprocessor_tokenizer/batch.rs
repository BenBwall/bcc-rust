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
//! [`super::token_source`] replays to phase 4.
//!
//! Provenance and diagnostics follow reading order: positions refer to the
//! original source, a token after a deleted splice starts at the splice, the
//! newline supplied for a file without a final one is consumed by whichever
//! token first looks at the end of input, and a final newline escaped by a
//! line splice is reported once each time the end of input is read after a
//! real character. Non-newline whitespace is collapsed to one space, the
//! implementation-defined choice in §5.1.1.2p3, p. 10; PDF p. 22.

use std::ops::Range;

use super::{
    super::initial_processing::terminal_splice_length,
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
        bump::{
            ArenaString,
            ArenaVec,
            Bump,
            TailVec,
        },
        byte_scan,
        string_cache::StringCacheId,
    },
};

/// How one byte of spliced text, or the gap before it, maps to the original
/// source when the mapping is not one byte to one byte.
#[derive(Clone, Copy, Debug)]
enum RemapKind {
    /// A line splice of this many original bytes was deleted before the byte.
    Deleted(usize),
    /// The byte replaced a three-character trigraph.
    Trigraph,
    /// The line feed replaced a carriage return and line feed.
    CarriageReturnLineFeed,
}

#[derive(Clone, Copy, Debug)]
struct Remap {
    /// Offset in the spliced text.
    clean: usize,
    kind:  RemapKind,
}

/// Accepts LF, CRLF, and CR as physical end-of-line indicators.
/// C99: phase-1 mapping is implementation-defined, §5.1.1.2p1, p. 9; PDF p. 21;
/// source new-lines §5.2.1p3, p. 17; PDF p. 29.
fn line_ending_length(bytes: &[u8]) -> Option<usize> {
    match bytes {
        | [b'\r', b'\n', ..] => Some(2),
        | [b'\n' | b'\r', ..] => Some(1),
        | _ => None,
    }
}

/// Maps the nine trigraph suffixes to their source characters.
/// C99: §5.2.1.1p1, p. 18; PDF p. 30.
fn trigraph_replacement(byte: u8) -> Option<char> {
    Some(match byte {
        | b'=' => '#',
        | b')' => ']',
        | b'!' => '|',
        | b'(' => '[',
        | b'\'' => '^',
        | b'>' => '}',
        | b'/' => '\\',
        | b'<' => '{',
        | b'-' => '~',
        | _ => return None,
    })
}

/// One character after translation phases 1 and 2, with the source bytes
/// that spell it.
/// C99: §5.1.1.2p1-2, pp. 9-10; PDF pp. 21-22.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LogicalCharacter {
    pub(crate) character: char,
    /// Offset of its spelling in the source.
    pub(crate) index:     usize,
    /// Length of its spelling in bytes: 3 for a trigraph.
    pub(crate) length:    usize,
}

/// The characters that `source[range]` spells once trigraphs are replaced
/// and line splices deleted. The range must not end inside a trigraph or
/// line splice; token spans never do. The characters are collected in `arena`.
/// C99: §5.1.1.2p1, p. 9; PDF p. 21; §5.1.1.2p2, p. 10; PDF p. 22.
pub(crate) fn logical_characters<'a>(
    arena: &'a Bump,
    source: &str,
    range: Range<usize>,
    trigraphs: bool,
) -> ArenaVec<'a, LogicalCharacter> {
    let bytes = &source.as_bytes()[..range.end];
    let mut characters = ArenaVec::new_in(arena);
    let mut index = range.start;
    while index < range.end {
        let rest = &bytes[index..];
        let splice = match rest {
            | [b'\\', after @ ..] => line_ending_length(after).map(|ending| 1 + ending),
            | [b'?', b'?', b'/', after @ ..] if trigraphs =>
                line_ending_length(after).map(|ending| 3 + ending),
            | _ => None,
        };
        if let Some(length) = splice {
            index += length;
            continue;
        }
        let (character, length) = match rest {
            | [b'?', b'?', third, ..]
                if trigraphs && let Some(replacement) = trigraph_replacement(*third) =>
                (replacement, 3),
            | _ => {
                let character = source[index..]
                    .chars()
                    .next()
                    .expect("ranges start at character boundaries");
                (character, character.len_utf8())
            },
        };
        characters.push(LogicalCharacter {
            character,
            index,
            length,
        });
        index += length;
    }
    characters
}

/// The position of source offset `to`, found by reading forward from `from`
/// in `source`.
pub(crate) fn position_after(source: &str, from: SourcePosition, to: usize) -> SourcePosition {
    let mut position = from;
    let mut characters = source[from.index..to].chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            | '\r' | '\n' => {
                if character == '\r' && characters.peek() == Some(&'\n') {
                    _ = characters.next();
                }
                position.line += 1;
                position.column = 1;
            },
            | _ => position.column += 1,
        }
    }
    position.index = to;
    position
}

/// Applies translation phases 1 and 2 to a whole buffer: trigraphs are
/// replaced, line endings become `'\n'`, and line splices are deleted. The
/// returned remaps record every place where spliced and original offsets stop
/// advancing together. A buffer that needs changes is copied into `scratch`.
/// C99: §5.1.1.2p1-2, pp. 9-10; PDF pp. 21-22; trigraph mapping §5.2.1.1p1,
/// p. 18; PDF p. 30.
fn splice<'a>(source: &'a str, scratch: &'a Bump, trigraphs: bool) -> (&'a str, &'a [Remap]) {
    let bytes = source.as_bytes();
    let mut special = byte_scan::find_phase2_special(bytes);
    if special == bytes.len() {
        return (source, &[]);
    }
    // Phases 1 and 2 only shorten the text, so its buffer never grows; the
    // remaps after it are the arena's latest block and grow in place.
    let mut text = ArenaString::with_capacity_in(bytes.len(), scratch);
    let mut remaps = ArenaVec::new_in(scratch);
    let mut copied = 0;
    while special < bytes.len() {
        // Every special byte is ASCII, so these are character boundaries.
        text.push_str(&source[copied..special]);
        let rest = &bytes[special..];
        special += match rest {
            | [b'\r', b'\n', ..] => {
                remaps.push(Remap {
                    clean: text.len(),
                    kind:  RemapKind::CarriageReturnLineFeed,
                });
                text.push('\n');
                2
            },
            | [b'\r', ..] => {
                text.push('\n');
                1
            },
            | [b'\\', after @ ..] => match line_ending_length(after) {
                | Some(ending) => {
                    remaps.push(Remap {
                        clean: text.len(),
                        kind:  RemapKind::Deleted(1 + ending),
                    });
                    1 + ending
                },
                | None => {
                    text.push('\\');
                    1
                },
            },
            | [b'?', b'?', third, after @ ..]
                if trigraphs && let Some(replacement) = trigraph_replacement(*third) =>
                match line_ending_length(after) {
                    // `??/` splices like a backslash.
                    | Some(ending) if replacement == '\\' => {
                        remaps.push(Remap {
                            clean: text.len(),
                            kind:  RemapKind::Deleted(3 + ending),
                        });
                        3 + ending
                    },
                    | _ => {
                        remaps.push(Remap {
                            clean: text.len(),
                            kind:  RemapKind::Trigraph,
                        });
                        text.push(replacement);
                        3
                    },
                },
            | _ => {
                text.push('?');
                1
            },
        };
        copied = special;
        special += byte_scan::find_phase2_special(&bytes[special..]);
    }
    text.push_str(&source[copied..]);
    (text.into_str(), remaps.leak())
}

/// Maps monotonically increasing offsets in spliced text back to original
/// source positions.
#[derive(Clone, Debug)]
struct PositionTracker<'a> {
    text:       &'a [u8],
    remaps:     &'a [Remap],
    next_remap: usize,
    clean:      usize,
    position:   SourcePosition,
}

impl<'a> PositionTracker<'a> {
    fn new(text: &'a [u8], remaps: &'a [Remap]) -> Self {
        Self {
            text,
            remaps,
            next_remap: 0,
            clean: 0,
            position: SourcePosition::default(),
        }
    }

    /// Advances to spliced offset `to` and returns the position there. A splice
    /// deleted immediately before `to` is not yet skipped, because the
    /// token starts before the splice it reads through.
    fn advance(&mut self, to: usize) -> SourcePosition {
        debug_assert!(to >= self.clean, "positions are queried in order");
        while let Some(&Remap { clean, kind }) = self.remaps.get(self.next_remap)
            && clean < to
        {
            self.advance_plain(clean);
            match kind {
                | RemapKind::Deleted(length) => self.skip_deleted(length),
                | RemapKind::Trigraph => {
                    self.clean += 1;
                    self.position.index += 3;
                    self.add_columns(3);
                },
                | RemapKind::CarriageReturnLineFeed => {
                    self.clean += 1;
                    self.position.index += 2;
                    self.add_lines(1);
                    self.position.column = 1;
                },
            }
            self.next_remap += 1;
        }
        self.advance_plain(to);
        self.position
    }

    /// Like [`Self::advance`], but also skips splices deleted immediately
    /// before `to`, giving where the character at `to` is spelled.
    fn advance_past_deletions(&mut self, to: usize) -> SourcePosition {
        _ = self.advance(to);
        while let Some(&Remap {
            clean,
            kind: RemapKind::Deleted(length),
        }) = self.remaps.get(self.next_remap)
            && clean == to
        {
            self.skip_deleted(length);
            self.next_remap += 1;
        }
        self.position
    }

    /// Whether the byte at the tracker's offset replaced a trigraph.
    fn at_trigraph(&self) -> bool {
        matches!(
            self.remaps.get(self.next_remap),
            Some(&Remap {
                clean,
                kind: RemapKind::Trigraph,
            }) if clean == self.clean
        )
    }

    fn skip_deleted(&mut self, length: usize) {
        self.position.index += length;
        self.add_lines(1);
        self.position.column = 1;
    }

    fn add_lines(&mut self, count: u32) {
        self.position.line = self
            .position
            .line
            .checked_add(count)
            .expect("source line exceeds u32::MAX");
    }

    fn add_columns(&mut self, count: u32) {
        self.position.column = self
            .position
            .column
            .checked_add(count)
            .expect("source column exceeds u32::MAX");
    }

    fn advance_plain(&mut self, to: usize) {
        let bytes = &self.text[self.clean..to];
        if bytes.is_empty() {
            return;
        }
        if byte_scan::find_line_feed(bytes) == bytes.len() {
            let count = u32::try_from(byte_scan::count_chars(bytes))
                .expect("source column exceeds u32::MAX");
            self.add_columns(count);
        } else {
            let last = bytes
                .iter()
                .rposition(|&byte| byte == b'\n')
                .expect("a line feed was found");
            let lines = u32::try_from(byte_scan::count_line_feeds(bytes))
                .expect("source line exceeds u32::MAX");
            self.add_lines(lines);
            let columns = u32::try_from(byte_scan::count_chars(&bytes[last + 1..]))
                .expect("source column exceeds u32::MAX");
            self.position.column = 1_u32
                .checked_add(columns)
                .expect("source column exceeds u32::MAX");
        }
        self.position.index += bytes.len();
        self.clean = to;
    }
}

/// A diagnostic raised while lexing, replayed whenever its token is read.
#[derive(Clone, Copy, Debug)]
enum LexDiagnostic {
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

/// One lexed entry: a preprocessing token, or whitespace or a comment
/// (`kind` is `None`), and where it starts.
/// C99: phase-3 decomposition §5.1.1.2p3, p. 10; PDF p. 22.
///
/// Packed, so an entry costs the 17 bytes its fields need. Fields are read
/// by value only, through the accessors.
#[derive(Clone, Copy)]
#[repr(C, packed)]
struct Entry {
    contents: StringCacheId,
    /// Byte offset of the entry's start in the source.
    index:    u32,
    line:     u32,
    column:   u32,
    kind:     Option<PreprocessorTokenType>,
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
            line:   self.line,
            column: self.column,
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
    pub(super) source_file_index: u32,
    /// The file's index in the preprocessor's registry of opened files, if
    /// it was opened there rather than for temporary use.
    pub(super) registration: Option<u32>,
    entries: &'a [Entry],
    /// Where the last entry ends.
    end_of_tokens: SourcePosition,
    /// Where reading past the last entry stands: past any trailing splices.
    eof: SourcePosition,
    /// Sorted by entry.
    diagnostics: &'a [(u32, LexDiagnostic)],
    /// Exact character spans for Other tokens, sorted by entry. Normal token
    /// spans still use the adjacent entry boundaries above.
    other_locations: &'a [(u32, SourceVector)],
    /// The entry for a missing final newline read at the start of a token.
    final_newline_entry: Option<usize>,
    /// Entries whose lexing read the supplied final newline while looking
    /// ahead. Sorted, and empty unless the final newline is missing.
    final_newline_readers: &'a [u32],
    lacks_final_newline: bool,
    /// The splice that escapes the source's final newline, with its line
    /// given before any `#line` renumbering.
    escaped_final_newline: Option<SourceVector>,
    /// Entries whose lexing read the end of input. Sorted, and empty unless
    /// the final newline is escaped.
    end_readers: &'a [u32],
}

/// A [`LexedFile`] while its entries are being lexed. Its side tables grow
/// in the lexing scratch arena until they are copied after the entries.
struct LexingFile<'arena, 's> {
    /// The rest of the arena's reservation while lexing, committed as
    /// entries are written, so it never moves or overcommits.
    entries:               TailVec<'arena, Entry>,
    end_of_tokens:         SourcePosition,
    eof:                   SourcePosition,
    diagnostics:           ArenaVec<'s, (u32, LexDiagnostic)>,
    other_locations:       ArenaVec<'s, (u32, SourceVector)>,
    final_newline_entry:   Option<usize>,
    final_newline_readers: ArenaVec<'s, u32>,
    lacks_final_newline:   bool,
    end_readers:           ArenaVec<'s, u32>,
}

impl<'arena> LexingFile<'arena, '_> {
    /// Keeps exactly the entries written, returning the rest of the
    /// reservation to `arena`, and stores the side tables after them.
    fn finish(
        self,
        arena: &'arena Bump,
        source_file_index: u32,
        escaped_final_newline: Option<SourceVector>,
    ) -> LexedFile<'arena> {
        let Self {
            entries,
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
            end_of_tokens,
            eof,
            diagnostics: arena.alloc_slice_copy(&diagnostics),
            other_locations: arena.alloc_slice_fill_iter(other_locations),
            final_newline_entry,
            final_newline_readers: arena.alloc_slice_copy(&final_newline_readers),
            lacks_final_newline,
            escaped_final_newline,
            end_readers: arena.alloc_slice_copy(&end_readers),
        }
    }
}

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

    /// A copy of this file in `arena`, outside any registry.
    pub(super) fn copy_into<'b>(&self, arena: &'b Bump) -> LexedFile<'b> {
        LexedFile {
            source_file_index:     self.source_file_index,
            registration:          None,
            entries:               arena.alloc_slice_copy(self.entries),
            end_of_tokens:         self.end_of_tokens,
            eof:                   self.eof,
            diagnostics:           arena.alloc_slice_copy(self.diagnostics),
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
        self.entries
            .get(entry)
            .map_or(self.end_of_tokens, |entry| entry.start())
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
        for (_, diagnostic) in self.diagnostics[first..]
            .iter()
            .take_while(|(owner, _)| *owner == entry)
        {
            match *diagnostic {
                | LexDiagnostic::Extension {
                    feature,
                    spelling,
                    mut start,
                    length,
                } =>
                    if !context.ignore_tokenizer_errors() {
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
}

/// A token's kind and spelling once its end is known.
#[derive(Clone, Copy)]
struct Lexed {
    kind:     Option<PreprocessorTokenType>,
    contents: StringCacheId,
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "Each flag is one piece of the end-of-input reading state."
)]
struct Lexer<'a, 'tu, 'arena, 's> {
    context:             &'a mut Context<'tu>,
    text:                &'a str,
    bytes:               &'a [u8],
    tracker:             PositionTracker<'a>,
    /// Spliced offset of the next token.
    pos:                 usize,
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

impl<'a, 'tu, 'arena, 's> Lexer<'a, 'tu, 'arena, 's> {
    fn new(
        context: &'a mut Context<'tu>,
        arena: &'arena Bump,
        scratch: &'s Bump,
        text: &'a str,
        remaps: &'a [Remap],
        physically_empty: bool,
    ) -> Self {
        let bytes = text.as_bytes();
        let lacks_final_newline = !physically_empty && bytes.last() != Some(&b'\n');
        Self {
            context,
            text,
            bytes,
            tracker: PositionTracker::new(bytes, remaps),
            pos: 0,
            lacks_final_newline,
            virtual_newline: lacks_final_newline,
            reached_eof: false,
            terminal_splice: false,
            splice_armed: true,
            read_end: false,
            read_final_newline: false,
            scratch,
            pending: ArenaVec::new_in(scratch),
            file: LexingFile {
                entries: arena.tail_vec(),
                end_of_tokens: SourcePosition::default(),
                eof: SourcePosition::default(),
                diagnostics: ArenaVec::new_in(scratch),
                other_locations: ArenaVec::new_in(scratch),
                final_newline_entry: None,
                final_newline_readers: ArenaVec::new_in(scratch),
                lacks_final_newline,
                end_readers: ArenaVec::new_in(scratch),
            },
        }
    }

    fn with_terminal_splice(mut self, terminal_splice: bool) -> Self {
        self.terminal_splice = terminal_splice;
        self
    }

    /// The spliced byte at `offset`. Looking at the end of input reads the
    /// supplied final newline, reporting its absence, or reports a final
    /// newline that a line splice escapes.
    #[inline(always)]
    fn peek(&mut self, offset: usize) -> Option<u8> {
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

    fn char_at(&self, offset: usize) -> char {
        self.text[offset..]
            .chars()
            .next()
            .expect("offsets are character boundaries")
    }

    fn intern(&mut self, start: usize, end: usize) -> StringCacheId {
        self.context.string_cache.intern(&self.text[start..end])
    }

    /// Finishes a token spelled exactly by `start..end`.
    fn spelled(&mut self, start: usize, end: usize, kind: PreprocessorTokenType) -> Lexed {
        self.pos = end;
        Lexed {
            kind:     Some(kind),
            contents: self.intern(start, end),
        }
    }

    /// Digraph-only diagnostics stay outside the common token completion path.
    /// C95 amendment 1 introduced the alternative token spellings.
    fn digraph(&mut self, start: usize, end: usize, kind: PreprocessorTokenType) -> Lexed {
        let position = self.tracker.advance(start);
        self.record_extension(Feature::Digraphs, "digraph", position, end - start);
        self.spelled(start, end, kind)
    }

    /// Keeps extension diagnostics beside their entry, so skipped groups stay
    /// silent. C99: §5.1.1.3p1, p. 11; PDF p. 23; GNU lexical extensions.
    fn record_extension(
        &mut self,
        feature: Feature,
        spelling: &'static str,
        start: SourcePosition,
        length: usize,
    ) {
        if !self.context.configuration.is_native(feature)
            || matches!(feature, Feature::DollarIdentifiers)
        {
            self.file.diagnostics.push((
                Self::checked_entry_index(self.file.entries.len()),
                LexDiagnostic::Extension {
                    feature,
                    spelling,
                    start,
                    length,
                },
            ));
        }
    }

    /// Finishes a token whose spelling differs from its source text.
    fn respelled(&mut self, end: usize, kind: PreprocessorTokenType, spelling: &str) -> Lexed {
        self.pos = end;
        Lexed {
            kind:     Some(kind),
            contents: self.context.string_cache.intern(spelling),
        }
    }

    /// Where reading past the end stands:
    /// beyond every trailing splice.
    fn eof_position(&self) -> SourcePosition {
        let mut tracker = self.tracker.clone();
        tracker.advance_past_deletions(self.bytes.len())
    }

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

    /// Leaves room for the one-past-last entry used when reading EOF.
    fn checked_entry_index(length: usize) -> u32 {
        let next = length
            .checked_add(1)
            .and_then(|count| u32::try_from(count).ok())
            .expect("lexed file exceeds u32::MAX entries");
        next - 1
    }

    fn push(&mut self, position: SourcePosition, lexed: Lexed) {
        let entry = Self::checked_entry_index(self.file.entries.len());
        if self.read_final_newline {
            self.file.final_newline_readers.push(entry);
        }
        if self.read_end && self.terminal_splice {
            self.file.end_readers.push(entry);
        }
        self.file
            .diagnostics
            .extend(self.pending.drain(..).map(|diagnostic| (entry, diagnostic)));
        self.file.entries.push(Entry {
            contents: lexed.contents,
            index:    u32::try_from(position.index).expect("source files are smaller than 4 GiB"),
            line:     position.line,
            column:   position.column,
            kind:     lexed.kind,
        });
    }

    /// Lexes the token starting with `byte` at `start`.
    /// C99: maximal munch §6.4p4, p. 50; PDF p. 62; `preprocessing-token`
    /// §6.4p1, p. 49; PDF p. 61. Header names are deferred to `#include`
    /// handling (§6.4.7, pp. 64-65; PDF pp. 76-77).
    #[inline(always)]
    fn lex_token(&mut self, start: usize, position: SourcePosition, byte: u8) -> Lexed {
        use PreprocessorTokenType as T;
        match byte {
            | b'\n' => self.spelled(start, start + 1, T::Newline),
            | b'0'..=b'9' => self.lex_number(start, start + 1),
            | b'{' => self.spelled(start, start + 1, T::OpeningCurlyBrace),
            | b'}' => self.spelled(start, start + 1, T::ClosingCurlyBrace),
            | b'(' => self.spelled(start, start + 1, T::OpeningParenthesis),
            | b')' => self.spelled(start, start + 1, T::ClosingParenthesis),
            | b'[' => self.spelled(start, start + 1, T::OpeningSquareBracket),
            | b']' => self.spelled(start, start + 1, T::ClosingSquareBracket),
            | b'L' => match self.peek(start + 1) {
                | Some(quote @ (b'"' | b'\'')) =>
                    self.lex_quoted(start, position, start + 2, quote),
                | _ => self.lex_identifier(start, start + 1),
            },
            | b'u' | b'U'
                if self
                    .context
                    .configuration
                    .accepts(Feature::UnicodeLiteralPrefixes) =>
            {
                let mut end = start + 1;
                if byte == b'u' && self.peek(end) == Some(b'8') {
                    end += 1;
                }
                match self.peek(end) {
                    | Some(quote @ (b'"' | b'\''))
                        if end == start + 1
                            || quote == b'"'
                            || self
                                .context
                                .configuration
                                .accepts(Feature::Utf8CharacterConstants) =>
                        self.lex_quoted(start, position, end + 1, quote),
                    | _ => self.lex_identifier(start, end),
                }
            },
            | b'$' if self
                .context
                .configuration
                .accepts(Feature::DollarIdentifiers) =>
                self.lex_extended_identifier(start, start),
            | b'\\' if super::ucn::decode(&self.text[start..], true).is_some() =>
                self.lex_identifier(start, start),
            | b'A'..=b'Z' | b'a'..=b'z' | b'_' => self.lex_identifier(start, start + 1),
            | b'.' => match self.peek(start + 1) {
                | Some(b'.') => match self.peek(start + 2) {
                    | Some(b'.') => self.spelled(start, start + 3, T::Ellipsis),
                    | _ => self.spelled(start, start + 1, T::Period),
                },
                | Some(b'0'..=b'9') => self.lex_number(start, start + 2),
                | _ => self.spelled(start, start + 1, T::Period),
            },
            | b'"' | b'\'' => self.lex_quoted(start, position, start + 1, byte),
            | b'#' => self.one_of(start, T::Hash, &[(b'#', T::HashHash)]),
            | b' ' | b'\t' | b'\x0b' | b'\x0c' => self.lex_whitespace(start + 1),
            | b'/' => match self.peek(start + 1) {
                | Some(b'=') => self.spelled(start, start + 2, T::ForwardSlashEquals),
                | Some(b'/') if self.context.configuration.accepts(Feature::LineComments) => {
                    self.record_extension(Feature::LineComments, "//", position, 2);
                    let end = self.skip_line_comment(start + 2);
                    self.lex_whitespace(end)
                },
                | Some(b'*') => {
                    let end = self.skip_block_comment(start + 2);
                    self.lex_whitespace(end)
                },
                | _ => self.spelled(start, start + 1, T::ForwardSlash),
            },
            | b'%' => match self.peek(start + 1) {
                | Some(b'=') => self.spelled(start, start + 2, T::PercentEquals),
                | Some(b'>') if self.context.configuration.accepts(Feature::Digraphs) =>
                    self.digraph(start, start + 2, T::ClosingCurlyBrace),
                | Some(b':') if self.context.configuration.accepts(Feature::Digraphs) =>
                    match self.peek(start + 2) {
                        | Some(b'%') => match self.peek(start + 3) {
                            | Some(b':') => self.digraph(start, start + 4, T::HashHash),
                            | _ => self.digraph(start, start + 2, T::Hash),
                        },
                        | _ => self.digraph(start, start + 2, T::Hash),
                    },
                | _ => self.spelled(start, start + 1, T::Percent),
            },
            | b'<' => match self.peek(start + 1) {
                | Some(b':') if self.context.configuration.accepts(Feature::Digraphs) =>
                    self.digraph(start, start + 2, T::OpeningSquareBracket),
                | Some(b'%') if self.context.configuration.accepts(Feature::Digraphs) =>
                    self.digraph(start, start + 2, T::OpeningCurlyBrace),
                | Some(b'=') => self.spelled(start, start + 2, T::LessThanEquals),
                | Some(b'<') => match self.peek(start + 2) {
                    | Some(b'=') => self.spelled(start, start + 3, T::LessThanLessThanEquals),
                    | _ => self.spelled(start, start + 2, T::LessThanLessThan),
                },
                | _ => self.spelled(start, start + 1, T::LessThan),
            },
            | b'>' => match self.peek(start + 1) {
                | Some(b'>') => match self.peek(start + 2) {
                    | Some(b'=') => self.spelled(start, start + 3, T::GreaterThanGreaterThanEquals),
                    | _ => self.spelled(start, start + 2, T::GreaterThanGreaterThan),
                },
                | Some(b'=') => self.spelled(start, start + 2, T::GreaterThanEquals),
                | _ => self.spelled(start, start + 1, T::GreaterThan),
            },
            | b',' => self.spelled(start, start + 1, T::Comma),
            | b';' => self.spelled(start, start + 1, T::SemiColon),
            | b'?' => self.spelled(start, start + 1, T::QuestionMark),
            | b'~' => self.spelled(start, start + 1, T::Tilde),
            | b':' if self.peek(start + 1) == Some(b'>')
                && self.context.configuration.accepts(Feature::Digraphs) =>
                self.digraph(start, start + 2, T::ClosingSquareBracket),
            | b':' => self.spelled(start, start + 1, T::Colon),
            | b'+' => self.one_of(
                start,
                T::Plus,
                &[(b'+', T::PlusPlus), (b'=', T::PlusEquals)],
            ),
            | b'-' => self.one_of(
                start,
                T::Minus,
                &[
                    (b'-', T::MinusMinus),
                    (b'=', T::MinusEquals),
                    (b'>', T::Arrow),
                ],
            ),
            | b'*' => self.one_of(start, T::Asterisk, &[(b'=', T::AsteriskEquals)]),
            | b'^' => self.one_of(start, T::Caret, &[(b'=', T::CaretEquals)]),
            | b'&' => self.one_of(
                start,
                T::Ampersand,
                &[(b'&', T::AmpersandAmpersand), (b'=', T::AmpersandEquals)],
            ),
            | b'|' => self.one_of(
                start,
                T::Pipe,
                &[(b'|', T::PipePipe), (b'=', T::PipeEquals)],
            ),
            | b'!' => self.one_of(
                start,
                T::ExclamationMark,
                &[(b'=', T::ExclamationMarkEquals)],
            ),
            | b'=' => self.one_of(start, T::Equals, &[(b'=', T::EqualsEquals)]),
            | 0x80.. if self.char_at(start).is_alphabetic() =>
                self.lex_identifier(start, start + self.char_at(start).len_utf8()),
            | _ => self.lex_other(start),
        }
    }

    /// A one-character punctuator, or a two-character one when the next
    /// character is listed.
    /// C99: `punctuator` §6.4.6p1, p. 63; PDF p. 75.
    fn one_of(
        &mut self,
        start: usize,
        single: PreprocessorTokenType,
        doubles: &[(u8, PreprocessorTokenType)],
    ) -> Lexed {
        let next = self.peek(start + 1);
        match doubles.iter().find(|(second, _)| Some(*second) == next) {
            | Some(&(_, kind)) => self.spelled(start, start + 2, kind),
            | None => self.spelled(start, start + 1, single),
        }
    }

    fn lex_other(&mut self, start: usize) -> Lexed {
        let character = self.char_at(start);
        let position = self.tracker.advance_past_deletions(start);
        let length = if self.tracker.at_trigraph() {
            3
        } else {
            character.len_utf8()
        };
        let entry = Self::checked_entry_index(self.file.entries.len());
        self.file
            .other_locations
            .push((entry, SourceVector::new(position, 0, length)));
        self.spelled(
            start,
            start + character.len_utf8(),
            PreprocessorTokenType::Other,
        )
    }

    /// Recognizes `identifier` spellings and the phase-4 `defined` operator.
    /// C99: §6.4.2.1p1, p. 51; PDF p. 63; §6.10.1p1, pp. 147-148;
    /// PDF pp. 159-160.
    fn lex_identifier(&mut self, start: usize, mut end: usize) -> Lexed {
        end += byte_scan::identifier_run(&self.bytes[end..]);
        let next = self.peek(end);
        if matches!(next, Some(b'\\' | 0x80..))
            || (next == Some(b'$')
                && self
                    .context
                    .configuration
                    .accepts(Feature::DollarIdentifiers))
        {
            return self.lex_extended_identifier(start, end);
        }
        let kind = if &self.bytes[start..end] == b"defined" {
            PreprocessorTokenType::Defined
        } else {
            PreprocessorTokenType::Identifier
        };
        self.spelled(start, end, kind)
    }

    /// Extends `identifier` across UCNs and accepted multibyte characters.
    /// C99: §6.4.2.1p1-3, p. 51; PDF p. 63; UCN form §6.4.3p1, p. 53;
    /// PDF p. 65.
    #[cold]
    fn lex_extended_identifier(&mut self, start: usize, mut end: usize) -> Lexed {
        let mut universal = false;
        loop {
            end += byte_scan::identifier_run(&self.bytes[end..]);
            match self.peek(end) {
                | Some(b'$')
                    if self
                        .context
                        .configuration
                        .accepts(Feature::DollarIdentifiers) =>
                {
                    let position = self.tracker.advance_past_deletions(end);
                    self.record_extension(Feature::DollarIdentifiers, "$", position, 1);
                    end += 1;
                },
                | Some(b'\\') if super::ucn::decode(&self.text[end..], end == start).is_some() => {
                    end += super::ucn::decode(&self.text[end..], end == start)
                        .unwrap()
                        .1;
                    universal = true;
                },
                | Some(0x80..) if self.char_at(end).is_alphanumeric() =>
                    end += self.char_at(end).len_utf8(),
                | _ => break,
            }
        }
        let kind = if &self.bytes[start..end] == b"defined" {
            PreprocessorTokenType::Defined
        } else {
            PreprocessorTokenType::Identifier
        };
        let mut token = self.spelled(start, end, kind);
        if universal {
            let (kind, contents) =
                super::ucn::identifier(self.context, self.scratch, token.contents);
            token.kind = Some(kind);
            token.contents = contents;
        }
        token
    }

    /// Extends a `pp-number`, including exponent sign and identifier suffix.
    /// C99: §6.4.8p1-4, p. 65; PDF p. 77; maximal munch §6.4p4, p. 50;
    /// PDF p. 62.
    #[inline(always)]
    fn lex_number(&mut self, start: usize, mut end: usize) -> Lexed {
        loop {
            let run = byte_scan::number_run(&self.bytes[end..]);
            end += run;
            if run != 0
                && matches!(self.bytes[end - 1], b'e' | b'E' | b'p' | b'P')
                && matches!(self.peek(end), Some(b'+' | b'-'))
            {
                end += 1;
                continue;
            }
            match self.peek(end) {
                | Some(b'\'')
                    if self.context.configuration.accepts(Feature::DigitSeparators)
                        && let Some(after) = self.number_separator_end(end) =>
                {
                    let ascii_nondigit = after == end + 2;
                    end = after;
                    if ascii_nondigit
                        && matches!(self.bytes[end - 1], b'e' | b'E' | b'p' | b'P')
                        && matches!(self.peek(end), Some(b'+' | b'-'))
                    {
                        end += 1;
                    }
                },
                | Some(b'$')
                    if self
                        .context
                        .configuration
                        .accepts(Feature::DollarIdentifiers) =>
                    end += 1,
                | Some(b'\\') if super::ucn::decode(&self.text[end..], false).is_some() =>
                    end += super::ucn::decode(&self.text[end..], false).unwrap().1,
                | Some(0x80..) if self.char_at(end).is_alphanumeric() =>
                    end += self.char_at(end).len_utf8(),
                | Some(_) => break,
                // Reading the end of input leaves the position past any
                // trailing splice.
                | None => {
                    self.reached_eof = true;
                    break;
                },
            }
        }
        // Number spellings carry the trailing NUL that numeric conversion
        // expects.
        self.pos = end;
        Lexed {
            kind:     Some(PreprocessorTokenType::Number),
            contents: self
                .context
                .string_cache
                .intern_concat(&[&self.text[start..end], "\0"]),
        }
    }

    /// C23 pp-number separator grammar admits digits and identifier
    /// nondigits; conversion subsequently requires digits of the radix.
    fn number_separator_end(&mut self, apostrophe: usize) -> Option<usize> {
        let index = apostrophe + 1;
        match self.peek(index)? {
            | byte if byte.is_ascii_alphanumeric() || byte == b'_' => Some(index + 1),
            | b'$' if self
                .context
                .configuration
                .accepts(Feature::DollarIdentifiers) =>
                Some(index + 1),
            | b'\\' =>
                super::ucn::decode(&self.text[index..], false).map(|(_, length)| index + length),
            | 0x80.. if self.char_at(index).is_alphanumeric() =>
                Some(index + self.char_at(index).len_utf8()),
            | _ => None,
        }
    }

    /// Continues a whitespace token from `end`; comments join it.
    /// C99: comment replacement and whitespace choice §5.1.1.2p3, p. 10;
    /// PDF p. 22; comments §6.4.9p1-2, p. 66; PDF p. 78.
    fn lex_whitespace(&mut self, mut end: usize) -> Lexed {
        loop {
            end += byte_scan::horizontal_space_run(&self.bytes[end..]);
            match self.peek(end) {
                | Some(b'/') => match self.peek(end + 1) {
                    | Some(b'/') if self.context.configuration.accepts(Feature::LineComments) => {
                        let position = self.tracker.advance_past_deletions(end);
                        self.record_extension(Feature::LineComments, "//", position, 2);
                        end = self.skip_line_comment(end + 2);
                    },
                    | Some(b'*') => end = self.skip_block_comment(end + 2),
                    | _ => break,
                },
                | Some(0x80..) if self.char_at(end).is_whitespace() =>
                    end += self.char_at(end).len_utf8(),
                | _ => break,
            }
        }
        self.respelled(end, PreprocessorTokenType::Whitespace, " ")
    }

    /// Returns the offset of the line ending that ends a `//` comment.
    /// C99: §6.4.9p2, p. 66; PDF p. 78.
    fn skip_line_comment(&mut self, body: usize) -> usize {
        let end = body + byte_scan::find_line_feed(&self.bytes[body..]);
        _ = self.peek(end);
        end
    }

    /// Returns the offset after the `*/` that ends a block comment, or the
    /// end of input.
    /// C99: §6.4.9p1, p. 66; PDF p. 78; partial-comment constraint §5.1.1.2p3,
    /// p. 10; PDF p. 22.
    fn skip_block_comment(&mut self, mut end: usize) -> usize {
        let body = end;
        loop {
            end += byte_scan::block_comment_run(&self.bytes[end..]);
            match self.peek(end) {
                // Unterminated: the token ends past any trailing splice.
                | None => {
                    let start = self.tracker.advance_past_deletions(body - 2);
                    let eof = self.eof_position();
                    self.pending.push(LexDiagnostic::Tokenizer {
                        error_type: PreprocessorTokenizerErrorType::UnterminatedBlockComment,
                        start,
                        length: eof.index - start.index,
                        character: None,
                    });
                    self.reached_eof = true;
                    return end;
                },
                | Some(b'*') => {
                    end += 1;
                    if self.peek(end) == Some(b'/') {
                        return end + 1;
                    }
                },
                | Some(_) if end < self.bytes.len() => end += 1,
                // The supplied final newline.
                | Some(_) => {},
            }
        }
    }

    /// Lexes a string or character literal whose body starts at `body`.
    /// C99: `character-constant` and `escape-sequence` §6.4.4.4p1, p. 59;
    /// PDF p. 71; `string-literal` §6.4.5p1, p. 62; PDF p. 74. Escape validity
    /// and value are checked after lexing.
    fn lex_quoted(
        &mut self,
        start: usize,
        position: SourcePosition,
        body: usize,
        quote: u8,
    ) -> Lexed {
        use PreprocessorTokenType as T;
        use PreprocessorTokenizerErrorType as E;
        let (kind, unterminated, newline) = match quote {
            | b'"' => (T::String, E::UnterminatedString, E::NewlineInString),
            | _ => (
                T::Character,
                E::UnterminatedCharacter,
                E::NewlineInCharacter,
            ),
        };
        let mut end = body;
        loop {
            end += byte_scan::literal_run(&self.bytes[end..]);
            match self.peek(end) {
                | None => {
                    let eof = self.eof_position();
                    self.pending.push(LexDiagnostic::Tokenizer {
                        error_type: unterminated,
                        start:      position,
                        length:     eof.index - position.index,
                        character:  None,
                    });
                    self.reached_eof = true;
                    return self.spelled(start, end, kind);
                },
                | Some(b'\\') => {
                    end += 1;
                    match self.peek(end) {
                        | Some(b'\n') | None => {},
                        | Some(_) => end += self.char_at(end).len_utf8(),
                    }
                },
                | Some(b'\n') => {
                    let at = self.tracker.advance(end);
                    self.pending.push(LexDiagnostic::Tokenizer {
                        error_type: newline,
                        start:      position,
                        length:     at.index - position.index,
                        character:  None,
                    });
                    // Close the literal for recovery.
                    let quote = [quote];
                    let quote = std::str::from_utf8(&quote).expect("quotes are ASCII");
                    self.pos = end;
                    return Lexed {
                        kind:     Some(kind),
                        contents: self
                            .context
                            .string_cache
                            .intern_concat(&[&self.text[start..end], quote]),
                    };
                },
                | Some(byte) if byte == quote => return self.spelled(start, end + 1, kind),
                | Some(_) => end += 1,
            }
        }
    }
}
