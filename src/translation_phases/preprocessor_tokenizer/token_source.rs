//! The preprocessing-token source that phase 4 reads: a lexed source file
//! or replayed tokens.
//!
//! C99: phase-3 token formation §5.1.1.2p3 and phase-4 macro expansion
//! §5.1.1.2p4, p. 10; PDF p. 22.

use std::fmt::{
    Debug,
    Formatter,
    Result as FmtResult,
};

use super::{
    PreprocessorToken,
    batch::LexedFile,
    replay::ReplayCursor,
};
use crate::{
    translation_phases::{
        Context,
        GetPosition,
        GetSourceFileIndex,
        SetPosition,
        SetSourceFileIndex,
        SourcePosition,
        SourceVector,
        SourceVectors,
        TranslationPhase,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// A cloneable, rewindable stream of preprocessing tokens over one source
/// buffer. Phase 4 keeps one per source file, macro replacement list, and
/// macro argument.
/// C99: §5.1.1.2p3-4, p. 10; PDF p. 22.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TokenSource<'a> {
    /// A source buffer, lexed completely when it was opened.
    File(LexedCursor<'a>),
    /// Tokens phase 4 already produced.
    Replay(ReplayCursor<'a>),
}

impl Default for TokenSource<'_> {
    fn default() -> Self {
        Self::Replay(ReplayCursor::default())
    }
}

impl<'a> TokenSource<'a> {
    /// Whether all tokens of a replay have been consumed. File sources retain
    /// their own EOF and missing-final-newline processing.
    pub(crate) fn is_exhausted_replay(&self) -> bool {
        matches!(self, Self::Replay(cursor) if cursor.is_exhausted())
    }

    /// Lexes all of `source` (translation phases 1 through 3) into `arena`
    /// and opens it.
    /// C99: §5.1.1.2p1-3, pp. 9-10; PDF pp. 21-22.
    pub(crate) fn new(
        context: &mut Context<'_>,
        arena: &'a Bump,
        source_file_index: u32,
        source: &str,
    ) -> Self {
        Self::File(LexedCursor::new(arena.alloc(LexedFile::lex(
            context,
            arena,
            source_file_index,
            source,
        ))))
    }

    /// Replays the tokens of `parts` in order, keeping them in `arena`;
    /// `empty_location` locates an empty replay.
    pub(crate) fn replay(
        context: &Context<'_>,
        arena: &'a Bump,
        parts: &[&[PreprocessorToken]],
        empty_location: SourceVector,
    ) -> Self {
        Self::Replay(ReplayCursor::new(context, arena, parts, empty_location))
    }

    /// A zero-length diagnostic location at `position`, which this source
    /// reported.
    pub(crate) fn location_at(
        &self,
        context: &mut Context<'_>,
        position: SourcePosition,
    ) -> SourceVectors {
        match self {
            | Self::Replay(cursor) => cursor.location_at(context, position),
            | Self::File(_) => context.create_source_vectors(position, self.source_file_index(), 0),
        }
    }
}

impl GetPosition for TokenSource<'_> {
    #[inline(always)]
    fn position(&self, _context: &Context<'_>) -> SourcePosition {
        match self {
            | Self::File(cursor) => cursor.position(),
            | Self::Replay(cursor) => cursor.position(),
        }
    }
}

impl SetPosition for TokenSource<'_> {
    #[inline(always)]
    fn set_position(&mut self, position: SourcePosition) {
        match self {
            | Self::File(cursor) => cursor.set_position(position),
            | Self::Replay(cursor) => cursor.set_position(position),
        }
    }
}

impl GetSourceFileIndex for TokenSource<'_> {
    #[inline(always)]
    fn source_file_index(&self) -> u32 {
        match self {
            | Self::File(cursor) => cursor.source_file_index,
            | Self::Replay(cursor) => cursor.source_file_index(),
        }
    }
}

impl SetSourceFileIndex for TokenSource<'_> {
    fn set_source_file_index(&mut self, source_file_index: u32) {
        match self {
            | Self::File(cursor) => cursor.source_file_index = source_file_index,
            // Replayed tokens keep the files they came from.
            | Self::Replay(_) => (),
        }
    }
}

impl TranslationPhase<'_> for TokenSource<'_> {
    type Item = PreprocessorToken;

    #[inline(always)]
    fn next_item(&mut self, context: &mut Context<'_>) -> Option<PreprocessorToken> {
        match self {
            | Self::File(cursor) => cursor.next_item(context),
            | Self::Replay(cursor) => cursor.next_item(context),
        }
    }
}

/// A position in a [`LexedFile`].
///
/// Reading an entry reports its diagnostics and records its provenance, as
/// lexing it would. Phase 4 only ever rewinds to positions it read, which are
/// entry boundaries.
/// C99: phase-3 token formation and phase-4 consumption §5.1.1.2p3-4, p. 10;
/// PDF p. 22.
#[derive(Clone)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Each flag is one piece of the end-of-input reading state."
)]
pub(crate) struct LexedCursor<'a> {
    file:                   &'a LexedFile<'a>,
    /// The next entry to read.
    next:                   usize,
    /// Whether reading past the last entry has already returned `None`.
    finished:               bool,
    /// Added to every line, so `#line` renumbering survives replay.
    line_delta:             u32,
    source_file_index:      u32,
    /// Whether reading at the end of the entries returns `None` rather than
    /// supplying a missing final newline: true when the last character read,
    /// in reading order, was a newline.
    final_newline_withheld: bool,
    /// Whether the escaped final newline was reported since the last real
    /// character was read, so reading the end of input stays silent.
    splice_reported:        bool,
    /// Whether a rewind placed the cursor at the end of input, past any
    /// trailing splices, rather than at the end of the last entry.
    at_eof:                 bool,
}

impl PartialEq for LexedCursor<'_> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.file, other.file)
            && self.next == other.next
            && self.finished == other.finished
            && self.line_delta == other.line_delta
            && self.source_file_index == other.source_file_index
            && self.final_newline_withheld == other.final_newline_withheld
            && self.splice_reported == other.splice_reported
            && self.at_eof == other.at_eof
    }
}

impl Eq for LexedCursor<'_> {}

impl Debug for LexedCursor<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("LexedCursor")
            .field("source_file_index", &self.source_file_index)
            .field("entries", &self.file.len())
            .field("next", &self.next)
            .field("finished", &self.finished)
            .field("line_delta", &self.line_delta)
            .field("final_newline_withheld", &self.final_newline_withheld)
            .field("splice_reported", &self.splice_reported)
            .field("at_eof", &self.at_eof)
            .finish()
    }
}

impl<'a> LexedCursor<'a> {
    fn new(file: &'a LexedFile<'a>) -> Self {
        Self {
            source_file_index: file.source_file_index,
            file,
            next: 0,
            finished: false,
            line_delta: 0,
            final_newline_withheld: false,
            splice_reported: false,
            at_eof: false,
        }
    }

    fn position(&self) -> SourcePosition {
        let mut position = if self.finished || self.at_eof {
            self.file.eof()
        } else {
            self.file.start(self.next)
        };
        position.line = position.line.wrapping_add(self.line_delta);
        position
    }

    /// The same reading state over `file`, a copy of this cursor's file.
    fn with_file<'b>(&self, file: &'b LexedFile<'b>) -> LexedCursor<'b> {
        LexedCursor {
            file,
            next: self.next,
            finished: self.finished,
            line_delta: self.line_delta,
            source_file_index: self.source_file_index,
            final_newline_withheld: self.final_newline_withheld,
            splice_reported: self.splice_reported,
            at_eof: self.at_eof,
        }
    }

    /// Resumes reading at `position`, which must start an entry or be the
    /// end of the entries.
    fn set_position(&mut self, position: SourcePosition) {
        let eof = self.file.eof();
        self.finished = false;
        // Trailing splices put the end of input past the last entry's end.
        if (position.index, position.column) == (eof.index, eof.column)
            && self.file.start(self.file.len()).index != eof.index
        {
            self.next = self.file.len();
            self.at_eof = true;
            self.line_delta = position.line.wrapping_sub(eof.line);
            return;
        }
        let boundary = self.file.boundary(position);
        debug_assert!(
            boundary.is_some(),
            "token sources rewind only to entry boundaries, not {position:?}"
        );
        let entry = boundary.unwrap_or_else(|| self.file.entry_after(position));
        self.next = entry;
        self.at_eof = false;
        self.line_delta = position.line.wrapping_sub(self.file.start(entry).line);
    }

    /// Reading the end of input reports an escaped final newline once after
    /// each real character.
    fn read_end(&mut self, context: &mut Context<'_>) {
        if !self.splice_reported && self.file.has_escaped_final_newline() {
            self.file.report_escaped_final_newline(
                context,
                self.source_file_index,
                self.line_delta,
            );
        }
        self.splice_reported = true;
    }

    #[inline(always)]
    fn next_item(&mut self, context: &mut Context<'_>) -> Option<PreprocessorToken> {
        loop {
            let entry = self.next;
            if entry >= self.file.len() {
                self.read_end(context);
                if !self.finished && !self.final_newline_withheld && self.file.lacks_final_newline()
                {
                    return Some(self.supply_final_newline(context));
                }
                self.finished = true;
                return None;
            }
            if self.final_newline_withheld && self.file.is_final_newline(entry) {
                self.read_end(context);
                self.next = self.file.len();
                self.finished = true;
                return None;
            }
            self.final_newline_withheld = self.file.withholds_final_newline_after(entry);
            if self.file.has_escaped_final_newline() {
                self.splice_reported = self.file.reads_end(entry);
            }
            self.next += 1;
            self.file
                .replay_diagnostics(context, entry, self.source_file_index, self.line_delta);
            let Some(kind) = self.file.kind(entry) else {
                continue;
            };
            let (mut start, length) = if kind == super::PreprocessorTokenType::Other {
                let source = self.file.other_location(entry);
                (source.position(context), source.length as usize)
            } else {
                let start = self.file.start(entry);
                (start, self.file.end_index(entry) - start.index)
            };
            start.line = start.line.wrapping_add(self.line_delta);
            let vector = context.push_lexed_source_vector(start, self.source_file_index, length);
            return Some(PreprocessorToken {
                kind,
                source_vectors: SourceVectors::new(vector, vector + 1),
                contents: self.file.contents(entry),
            });
        }
    }

    /// Reads the missing final newline at the end of the entries, which
    /// only happens when the entries were not read in lexing order.
    #[cold]
    #[inline(never)]
    fn supply_final_newline(&mut self, context: &mut Context<'_>) -> PreprocessorToken {
        let mut start = self.file.start(self.file.len());
        start.line = start.line.wrapping_add(self.line_delta);
        if !self.file.has_escaped_final_newline() {
            self.file.report_missing_final_newline(
                context,
                self.source_file_index,
                self.line_delta,
            );
        }
        self.final_newline_withheld = true;
        self.finished = true;
        let length = self.file.eof().index - start.index;
        let vector = context.push_lexed_source_vector(start, self.source_file_index, length);
        PreprocessorToken {
            kind:           super::PreprocessorTokenType::Newline,
            source_vectors: SourceVectors::new(vector, vector + 1),
            contents:       context.string_cache.intern("\n"),
        }
    }
}

/// The source files opened during one preprocessing run, kept in its arena.
///
/// Phase 4 reads files through cursors whose lifetime may be shorter than
/// the run, such as expansion state. [`Self::persist`] returns such a
/// source to the run's lifetime, so a macro definition or a resting include
/// frame can keep reading it.
pub(crate) struct LexedFiles<'pp> {
    arena: &'pp Bump,
    files: ArenaVec<'pp, &'pp LexedFile<'pp>>,
}

impl Debug for LexedFiles<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("LexedFiles")
            .field("files", &self.files.len())
            .finish()
    }
}

impl<'pp> LexedFiles<'pp> {
    pub(crate) fn new_in(arena: &'pp Bump) -> Self {
        Self {
            arena,
            files: ArenaVec::new_in(arena),
        }
    }

    /// Lexes all of `source` (translation phases 1 through 3) into the
    /// run's arena and opens it.
    pub(crate) fn open(
        &mut self,
        context: &mut Context<'_>,
        source_file_index: u32,
        source: &str,
    ) -> TokenSource<'pp> {
        let file = LexedFile::lex(context, self.arena, source_file_index, source);
        TokenSource::File(LexedCursor::new(self.register(file)))
    }

    fn register(&mut self, mut file: LexedFile<'pp>) -> &'pp LexedFile<'pp> {
        file.registration =
            Some(u32::try_from(self.files.len()).expect("opened file index exceeds u32::MAX"));
        let file = &*self.arena.alloc(file);
        self.files.push(file);
        file
    }

    /// Opens GCC command-line directives directly at translation phase 3.
    /// C99: command-line extension to §5.1.1.2p3, p. 10; PDF p. 22.
    pub(crate) fn open_command_line(
        &mut self,
        context: &mut Context<'_>,
        source_file_index: u32,
        source: &str,
    ) -> TokenSource<'pp> {
        let file = LexedFile::lex_command_line(context, self.arena, source_file_index, source);
        TokenSource::File(LexedCursor::new(self.register(file)))
    }

    /// `source` in the run's lifetime. A cursor over a file opened here is
    /// re-pointed at that file; any other file, and any replay, is copied
    /// into the arena.
    pub(crate) fn persist(&mut self, source: &TokenSource<'_>) -> TokenSource<'pp> {
        match source {
            | TokenSource::File(cursor) => {
                let registered = cursor
                    .file
                    .registration
                    .and_then(|index| self.files.get(index as usize).copied())
                    .filter(|file| std::ptr::addr_eq(*file, cursor.file));
                let file = match registered {
                    | Some(file) => file,
                    | None => {
                        let copy = cursor.file.copy_into(self.arena);
                        self.register(copy)
                    },
                };
                TokenSource::File(cursor.with_file(file))
            },
            | TokenSource::Replay(cursor) => TokenSource::Replay(cursor.copy_into(self.arena)),
        }
    }
}
