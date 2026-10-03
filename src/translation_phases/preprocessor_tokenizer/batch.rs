//! Whole-file lexing for the batch strategy.
//!
//! Translation phases 1 and 2 run over the entire buffer first: one
//! vectorized scan finds every byte that could start a trigraph, a line
//! splice, or a carriage return, so a buffer without any is lexed in place
//! and only the rest is copied once. Phase 3 then lexes the spliced text to
//! completion into the struct-of-arrays [`LexedFile`], which
//! [`super::token_source`] replays to phase 4.
//!
//! The lexer reproduces the streaming tokenizer token for token, including
//! provenance and diagnostics: positions refer to the original source, a
//! token after a deleted splice starts at the splice, and the newline supplied
//! for a file without a final one is consumed by whichever token first looks
//! at the end of input.

use std::borrow::Cow;

use super::{
    PreprocessorTokenType,
    PreprocessorTokenizerError,
    PreprocessorTokenizerErrorType,
};
use crate::{
    translation_phases::{
        Context,
        SourcePosition,
        SourceVector,
        provenance::source_offset,
    },
    util::{
        byte_scan,
        shared::SharedString,
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

fn line_ending_length(bytes: &[u8]) -> Option<usize> {
    match bytes {
        | [b'\r', b'\n', ..] => Some(2),
        | [b'\n' | b'\r', ..] => Some(1),
        | _ => None,
    }
}

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

/// Applies translation phases 1 and 2 to a whole buffer: trigraphs are
/// replaced, line endings become `'\n'`, and line splices are deleted. The
/// returned remaps record every place where spliced and original offsets stop
/// advancing together.
fn splice(source: &str) -> (Cow<'_, str>, Vec<Remap>) {
    let bytes = source.as_bytes();
    let mut special = byte_scan::find_phase2_special(bytes);
    if special == bytes.len() {
        return (Cow::Borrowed(source), Vec::new());
    }
    let mut text = String::with_capacity(bytes.len());
    let mut remaps = Vec::new();
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
                if let Some(replacement) = trigraph_replacement(*third) =>
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
    (Cow::Owned(text), remaps)
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

#[expect(
    clippy::cast_possible_truncation,
    reason = "Lines and columns are 32-bit like the rest of source provenance."
)]
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
    /// streaming tokenizer starts a token before the splice it reads through.
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
                    self.position.column += 3;
                },
                | RemapKind::CarriageReturnLineFeed => {
                    self.clean += 1;
                    self.position.index += 2;
                    self.position.line += 1;
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
        self.position.line += 1;
        self.position.column = 1;
    }

    fn advance_plain(&mut self, to: usize) {
        let bytes = &self.text[self.clean..to];
        if bytes.is_empty() {
            return;
        }
        if byte_scan::find_line_feed(bytes) == bytes.len() {
            self.position.column += byte_scan::count_chars(bytes) as u32;
        } else {
            let last = bytes
                .iter()
                .rposition(|&byte| byte == b'\n')
                .expect("a line feed was found");
            self.position.line += byte_scan::count_line_feeds(bytes) as u32;
            self.position.column = 1 + byte_scan::count_chars(&bytes[last + 1..]) as u32;
        }
        self.position.index += bytes.len();
        self.clean = to;
    }
}

/// A diagnostic raised while lexing, replayed whenever its token is read.
#[derive(Clone, Debug)]
enum LexDiagnostic {
    Tokenizer {
        error_type: PreprocessorTokenizerErrorType,
        start:      SourcePosition,
        length:     usize,
        character:  Option<char>,
    },
    MissingFinalNewline,
}

/// Every preprocessing token of one source buffer, stored as parallel
/// arrays. Entry `i` ends where entry `i + 1` starts. Other-token provenance
/// can exclude deleted splices without changing these shared boundaries.
pub(super) struct LexedFile {
    pub(super) source_file_index: u32,
    pub(super) source: SharedString,
    kinds: Vec<Option<PreprocessorTokenType>>,
    contents: Vec<StringCacheId>,
    indices: Vec<u32>,
    lines: Vec<u32>,
    columns: Vec<u32>,
    /// Where the last entry ends.
    end_of_tokens: SourcePosition,
    /// Where reading past the last entry leaves the streaming tokenizer.
    eof: SourcePosition,
    /// Sorted by entry.
    diagnostics: Vec<(u32, LexDiagnostic)>,
    /// Exact character spans for Other tokens, sorted by entry. Normal token
    /// spans still use the adjacent entry boundaries above.
    other_locations: Vec<(u32, SourceVector)>,
    /// The entry for a missing final newline read at the start of a token.
    final_newline_entry: Option<usize>,
    /// Entries whose lexing read the supplied final newline while looking
    /// ahead. Sorted, and empty unless the final newline is missing.
    final_newline_readers: Vec<u32>,
    lacks_final_newline: bool,
}

impl LexedFile {
    /// Runs translation phases 1 through 3 over all of `source`.
    pub(super) fn lex(context: &mut Context, source_file_index: u32, source: SharedString) -> Self {
        let (text, remaps) = splice(&source);
        let file = Lexer::new(context, &text, &remaps, source.is_empty()).run();
        Self {
            source_file_index,
            source,
            ..file
        }
    }

    pub(super) fn len(&self) -> usize {
        self.kinds.len()
    }

    #[inline(always)]
    pub(super) fn kind(&self, entry: usize) -> Option<PreprocessorTokenType> {
        self.kinds[entry]
    }

    #[inline(always)]
    pub(super) fn contents(&self, entry: usize) -> StringCacheId {
        self.contents[entry]
    }

    /// Where `entry` starts, or where the last entry ends for `len()`.
    #[inline(always)]
    pub(super) fn start(&self, entry: usize) -> SourcePosition {
        match self.indices.get(entry) {
            | Some(&index) => SourcePosition {
                index:  index as usize,
                line:   self.lines[entry],
                column: self.columns[entry],
            },
            | None => self.end_of_tokens,
        }
    }

    #[inline(always)]
    pub(super) fn end_index(&self, entry: usize) -> usize {
        self.indices
            .get(entry + 1)
            .map_or(self.end_of_tokens.index, |&index| index as usize)
    }

    /// The actual character span, excluding any splice before an Other token.
    pub(super) fn other_location(&self, entry: usize) -> &SourceVector {
        let entry = u32::try_from(entry).expect("entry indices fit in u32");
        let found = self
            .other_locations
            .binary_search_by_key(&entry, |&(index, _)| index)
            .expect("Other tokens have exact character spans");
        &self.other_locations[found].1
    }

    /// Whether the streaming tokenizer withholds the supplied final newline
    /// after reading `entry`, because the last character it read was a
    /// newline.
    #[inline(always)]
    pub(super) fn withholds_final_newline_after(&self, entry: usize) -> bool {
        self.kinds[entry] == Some(PreprocessorTokenType::Newline)
            || (!self.final_newline_readers.is_empty()
                && self
                    .final_newline_readers
                    .binary_search(&u32::try_from(entry).expect("entry indices fit in u32"))
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
        context: &mut Context,
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
        let entry = self.indices.partition_point(|&index| index < target);
        (self.start(entry).index == position.index && self.start(entry).column == position.column)
            .then_some(entry)
    }

    /// Reports the diagnostics recorded while lexing `entry`, as reading it
    /// from the streaming tokenizer would.
    #[inline(always)]
    pub(super) fn replay_diagnostics(
        &self,
        context: &mut Context,
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
        context: &mut Context,
        entry: usize,
        source_file_index: u32,
        line_delta: u32,
    ) {
        let entry = u32::try_from(entry).expect("entry indices fit in u32");
        let first = self
            .diagnostics
            .partition_point(|(owner, _)| *owner < entry);
        for (_, diagnostic) in self.diagnostics[first..]
            .iter()
            .take_while(|(owner, _)| *owner == entry)
        {
            match *diagnostic {
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
            }
        }
    }
}

/// Where the lexer is within a possible `#include` line, which decides
/// whether `<` and `"` start header names.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DirectiveState {
    LineStart,
    AfterHash,
    AfterInclude,
    Other,
}

/// A token's kind and spelling once its end is known.
#[derive(Clone, Copy)]
struct Lexed {
    kind:     Option<PreprocessorTokenType>,
    contents: StringCacheId,
}

struct Lexer<'a> {
    context:             &'a mut Context,
    text:                &'a str,
    bytes:               &'a [u8],
    tracker:             PositionTracker<'a>,
    /// Spliced offset of the next token.
    pos:                 usize,
    /// Whether the input lacks a final newline, so one is supplied.
    lacks_final_newline: bool,
    /// Whether reading the end of input now yields the supplied newline. As
    /// in the streaming tokenizer, reading it withholds it until a real
    /// character is read again.
    virtual_newline:     bool,
    /// Whether the token being lexed read through the end of input.
    reached_eof:         bool,
    /// Diagnostics raised while lexing the current token, in order.
    pending:             Vec<LexDiagnostic>,
    directive:           DirectiveState,
    include:             StringCacheId,
    scratch:             String,
    file:                LexedFile,
}

impl<'a> Lexer<'a> {
    fn new(
        context: &'a mut Context,
        text: &'a str,
        remaps: &'a [Remap],
        physically_empty: bool,
    ) -> Self {
        let bytes = text.as_bytes();
        let lacks_final_newline = !physically_empty && bytes.last() != Some(&b'\n');
        // Entries average a few bytes each.
        let capacity = bytes.len() / 3;
        let include = context.string_cache.intern("include");
        Self {
            context,
            text,
            bytes,
            tracker: PositionTracker::new(bytes, remaps),
            pos: 0,
            lacks_final_newline,
            virtual_newline: lacks_final_newline,
            reached_eof: false,
            pending: Vec::new(),
            directive: DirectiveState::LineStart,
            include,
            scratch: String::new(),
            file: LexedFile {
                source_file_index: 0,
                source: SharedString::default(),
                kinds: Vec::with_capacity(capacity),
                contents: Vec::with_capacity(capacity),
                indices: Vec::with_capacity(capacity),
                lines: Vec::with_capacity(capacity),
                columns: Vec::with_capacity(capacity),
                end_of_tokens: SourcePosition::default(),
                eof: SourcePosition::default(),
                diagnostics: Vec::new(),
                other_locations: Vec::new(),
                final_newline_entry: None,
                final_newline_readers: Vec::new(),
                lacks_final_newline,
            },
        }
    }

    /// The spliced byte at `offset`. Looking at the end of input reads the
    /// supplied final newline, reporting its absence.
    #[inline(always)]
    fn peek(&mut self, offset: usize) -> Option<u8> {
        if let Some(&byte) = self.bytes.get(offset) {
            self.virtual_newline = self.lacks_final_newline;
            return Some(byte);
        }
        if offset == self.bytes.len() && self.virtual_newline {
            self.virtual_newline = false;
            self.pending.push(LexDiagnostic::MissingFinalNewline);
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

    /// Finishes a token whose spelling differs from its source text.
    fn respelled(&mut self, end: usize, kind: PreprocessorTokenType, spelling: &str) -> Lexed {
        self.pos = end;
        Lexed {
            kind:     Some(kind),
            contents: self.context.string_cache.intern(spelling),
        }
    }

    /// Where the streaming tokenizer stands after reading past the end:
    /// beyond every trailing splice.
    fn eof_position(&self) -> SourcePosition {
        let mut tracker = self.tracker.clone();
        tracker.advance_past_deletions(self.bytes.len())
    }

    fn run(mut self) -> LexedFile {
        loop {
            let start = self.pos;
            let position = self.tracker.advance(start);
            let Some(byte) = self.peek(start) else {
                break;
            };
            self.reached_eof = false;
            let lexed = if start == self.bytes.len() {
                // The supplied final newline, read at the start of a token.
                self.reached_eof = true;
                self.file.final_newline_entry = Some(self.file.kinds.len());
                self.respelled(start, PreprocessorTokenType::Newline, "\n")
            } else {
                self.lex_token(start, position, byte)
            };
            self.push(position, lexed);
        }
        let end = self.tracker.advance(self.bytes.len());
        self.file.eof = self.tracker.advance_past_deletions(self.bytes.len());
        self.file.end_of_tokens = if self.reached_eof { self.file.eof } else { end };
        self.file
    }

    fn push(&mut self, position: SourcePosition, lexed: Lexed) {
        let entry = u32::try_from(self.file.kinds.len()).expect("entry indices fit in u32");
        if self
            .pending
            .iter()
            .any(|diagnostic| matches!(diagnostic, LexDiagnostic::MissingFinalNewline))
        {
            self.file.final_newline_readers.push(entry);
        }
        self.file
            .diagnostics
            .extend(self.pending.drain(..).map(|diagnostic| (entry, diagnostic)));
        self.directive = match (self.directive, lexed.kind) {
            | (_, Some(PreprocessorTokenType::Newline)) => DirectiveState::LineStart,
            | (state, Some(PreprocessorTokenType::Whitespace)) => state,
            | (DirectiveState::LineStart, Some(PreprocessorTokenType::Hash)) =>
                DirectiveState::AfterHash,
            | (DirectiveState::AfterHash, Some(PreprocessorTokenType::Identifier))
                if lexed.contents == self.include =>
                DirectiveState::AfterInclude,
            | _ => DirectiveState::Other,
        };
        self.file.kinds.push(lexed.kind);
        self.file.contents.push(lexed.contents);
        self.file
            .indices
            .push(u32::try_from(position.index).expect("source files are smaller than 4 GiB"));
        self.file.lines.push(position.line);
        self.file.columns.push(position.column);
    }

    /// Lexes the token starting with `byte` at `start`, dispatching exactly
    /// as the streaming tokenizer does.
    fn lex_token(&mut self, start: usize, position: SourcePosition, byte: u8) -> Lexed {
        use PreprocessorTokenType as T;
        let include_line = self.directive == DirectiveState::AfterInclude;
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
                    self.lex_quoted(start, position, start + 2, quote, false),
                | _ => self.lex_identifier(start, start + 1),
            },
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
            | b'"' => self.lex_quoted(start, position, start + 1, b'"', include_line),
            | b'\'' => self.lex_quoted(start, position, start + 1, b'\'', false),
            | b'#' => self.one_of(start, T::Hash, &[(b'#', T::HashHash)]),
            | b' ' | b'\t' | b'\x0b' | b'\x0c' => self.lex_whitespace(start + 1),
            | b'/' => match self.peek(start + 1) {
                | Some(b'=') => self.spelled(start, start + 2, T::ForwardSlashEquals),
                | Some(b'/') => {
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
                | Some(b'>') => self.spelled(start, start + 2, T::ClosingCurlyBrace),
                | Some(b':') => match self.peek(start + 2) {
                    | Some(b'%') => match self.peek(start + 3) {
                        | Some(b':') => self.spelled(start, start + 4, T::HashHash),
                        | _ => self.spelled(start, start + 2, T::Hash),
                    },
                    | _ => self.spelled(start, start + 2, T::Hash),
                },
                | _ => self.spelled(start, start + 1, T::Percent),
            },
            | b'<' if include_line && let Some(end) = self.header_name_end(start) =>
                self.spelled(start, end, T::AngleBracketString),
            | b'<' => match self.peek(start + 1) {
                | Some(b':') => self.spelled(start, start + 2, T::OpeningSquareBracket),
                | Some(b'%') => self.spelled(start, start + 2, T::OpeningCurlyBrace),
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
            | b':' => self.one_of(start, T::Colon, &[(b'>', T::ClosingSquareBracket)]),
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
        let entry = u32::try_from(self.file.kinds.len()).expect("entry indices fit in u32");
        self.file
            .other_locations
            .push((entry, SourceVector::new(position, 0, length)));
        self.spelled(
            start,
            start + character.len_utf8(),
            PreprocessorTokenType::Other,
        )
    }

    fn lex_identifier(&mut self, start: usize, mut end: usize) -> Lexed {
        end += byte_scan::identifier_run(&self.bytes[end..]);
        if matches!(self.peek(end), Some(b'\\' | 0x80..)) {
            return self.lex_extended_identifier(start, end);
        }
        let kind = if &self.bytes[start..end] == b"defined" {
            PreprocessorTokenType::Defined
        } else {
            PreprocessorTokenType::Identifier
        };
        self.spelled(start, end, kind)
    }

    #[cold]
    fn lex_extended_identifier(&mut self, start: usize, mut end: usize) -> Lexed {
        let mut universal = false;
        loop {
            end += byte_scan::identifier_run(&self.bytes[end..]);
            match self.peek(end) {
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
            let (kind, contents) = super::ucn::identifier(self.context, token.contents);
            token.kind = Some(kind);
            token.contents = contents;
        }
        token
    }

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
                | Some(b'\\') if super::ucn::decode(&self.text[end..], false).is_some() =>
                    end += super::ucn::decode(&self.text[end..], false).unwrap().1,
                | Some(0x80..) if self.char_at(end).is_alphanumeric() =>
                    end += self.char_at(end).len_utf8(),
                | Some(_) => break,
                // The streaming tokenizer keeps its position after reading the
                // end of input, past any trailing splice.
                | None => {
                    self.reached_eof = true;
                    break;
                },
            }
        }
        // Number spellings carry the trailing NUL that numeric conversion
        // expects.
        let mut spelling = std::mem::take(&mut self.scratch);
        spelling.clear();
        spelling.push_str(&self.text[start..end]);
        spelling.push('\0');
        let lexed = self.respelled(end, PreprocessorTokenType::Number, &spelling);
        self.scratch = spelling;
        lexed
    }

    /// Continues a whitespace token from `end`; comments join it.
    fn lex_whitespace(&mut self, mut end: usize) -> Lexed {
        loop {
            end += byte_scan::horizontal_space_run(&self.bytes[end..]);
            match self.peek(end) {
                | Some(b'/') => match self.peek(end + 1) {
                    | Some(b'/') => end = self.skip_line_comment(end + 2),
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
    fn skip_line_comment(&mut self, body: usize) -> usize {
        let end = body + byte_scan::find_line_feed(&self.bytes[body..]);
        _ = self.peek(end);
        end
    }

    /// Returns the offset after the `*/` that ends a block comment, or the
    /// end of input.
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

    /// The end of a `<...>` header name at `start`, if the line has one.
    ///
    /// This does not read the supplied final newline: phase 4 reads a failed
    /// header name again as ordinary tokens, which the entries must match.
    fn header_name_end(&self, start: usize) -> Option<usize> {
        let mut end = start + 1;
        loop {
            match *self.bytes.get(end)? {
                | b'>' => return Some(end + 1),
                | b'\n' => return None,
                | _ => end += 1,
            }
        }
    }

    /// Lexes a string or character literal, or a quoted header name when
    /// `header` is set, whose body starts at `body`.
    fn lex_quoted(
        &mut self,
        start: usize,
        position: SourcePosition,
        body: usize,
        quote: u8,
        header: bool,
    ) -> Lexed {
        use PreprocessorTokenType as T;
        use PreprocessorTokenizerErrorType as E;
        let (kind, unterminated, newline) = match (quote, header) {
            | (b'"', true) => (
                T::IncludeString,
                E::UnterminatedIncludeString,
                E::NewlineInIncludeString,
            ),
            | (b'"', false) => (T::String, E::UnterminatedString, E::NewlineInString),
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
                | Some(b'\\') if !header => {
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
                    // The streaming tokenizer closes the literal for recovery.
                    let mut spelling = std::mem::take(&mut self.scratch);
                    spelling.clear();
                    spelling.push_str(&self.text[start..end]);
                    spelling.push(char::from(quote));
                    let lexed = self.respelled(end, kind, &spelling);
                    self.scratch = spelling;
                    return lexed;
                },
                | Some(byte) if byte == quote => return self.spelled(start, end + 1, kind),
                | Some(_) => end += 1,
            }
        }
    }
}
