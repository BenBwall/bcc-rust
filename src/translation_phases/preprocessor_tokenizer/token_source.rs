//! The preprocessing-token source that phase 4 reads, under either lexing
//! strategy.

use std::{
    fmt::{
        Debug,
        Formatter,
        Result as FmtResult,
    },
    rc::Rc,
};

use super::{
    PreprocessorToken,
    PreprocessorTokenizer,
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
    util::shared::SharedString,
};

/// When translation phases 1 through 3 run relative to phase 4.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum LexingStrategy {
    /// Lex each preprocessing token when phase 4 asks for it.
    #[default]
    Streaming,
    /// Lex each source buffer completely when it is opened, then replay its
    /// tokens.
    Batch,
}

/// A cloneable, rewindable stream of preprocessing tokens over one source
/// buffer. Phase 4 keeps one per source file, macro replacement list, and
/// macro argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TokenSource {
    Streaming(PreprocessorTokenizer),
    Batch(LexedCursor),
    /// Tokens phase 4 already produced, under either lexing strategy.
    Replay(ReplayCursor),
}

impl Default for TokenSource {
    fn default() -> Self {
        Self::Streaming(PreprocessorTokenizer::default())
    }
}

impl TokenSource {
    /// Opens `source` with the context's lexing strategy.
    pub(crate) fn new(context: &mut Context, source_file_index: u32, source: SharedString) -> Self {
        // Malformed terminal splices need reading-order EOF state, including
        // suppressed reads and cursor rewinds. Retain the streaming cursor for
        // this rare input; ordinary files keep the compact batch
        // representation.
        if crate::translation_phases::initial_processing::InitialProcessor::terminal_splice_length(
            &source,
        )
        .is_some()
        {
            return Self::Streaming(PreprocessorTokenizer::new(source_file_index, source));
        }
        match context.lexing_strategy() {
            | LexingStrategy::Streaming =>
                Self::Streaming(PreprocessorTokenizer::new(source_file_index, source)),
            | LexingStrategy::Batch => Self::Batch(LexedCursor::new(Rc::new(LexedFile::lex(
                context,
                source_file_index,
                source,
            )))),
        }
    }

    /// Replays `tokens`; `empty_location` locates an empty replay.
    pub(crate) fn replay(
        context: &Context,
        tokens: &[PreprocessorToken],
        empty_location: SourceVector,
    ) -> Self {
        Self::Replay(ReplayCursor::new(context, tokens, empty_location))
    }

    /// A zero-length diagnostic location at `position`, which this source
    /// reported.
    pub(crate) fn location_at(
        &self,
        context: &mut Context,
        position: SourcePosition,
    ) -> SourceVectors {
        match self {
            | Self::Replay(cursor) => cursor.location_at(context, position),
            | Self::Streaming(_) | Self::Batch(_) =>
                context.create_source_vectors(position, self.source_file_index(), 0),
        }
    }
}

impl GetPosition for TokenSource {
    #[inline(always)]
    fn position(&self, context: &Context) -> SourcePosition {
        match self {
            | Self::Streaming(tokenizer) => tokenizer.position(context),
            | Self::Batch(cursor) => cursor.position(context),
            | Self::Replay(cursor) => cursor.position(),
        }
    }
}

impl SetPosition for TokenSource {
    #[inline(always)]
    fn set_position(&mut self, context: &mut Context, position: SourcePosition) {
        match self {
            | Self::Streaming(tokenizer) => tokenizer.set_position(context, position),
            | Self::Batch(cursor) => cursor.set_position(context, position),
            | Self::Replay(cursor) => cursor.set_position(position),
        }
    }
}

impl GetSourceFileIndex for TokenSource {
    #[inline(always)]
    fn source_file_index(&self) -> u32 {
        match self {
            | Self::Streaming(tokenizer) => tokenizer.source_file_index(),
            | Self::Batch(cursor) => cursor.source_file_index,
            | Self::Replay(cursor) => cursor.source_file_index(),
        }
    }
}

impl SetSourceFileIndex for TokenSource {
    fn set_source_file_index(&mut self, context: &mut Context, source_file_index: u32) {
        match self {
            | Self::Streaming(tokenizer) =>
                tokenizer.set_source_file_index(context, source_file_index),
            | Self::Batch(cursor) => {
                cursor.source_file_index = source_file_index;
                if let Some(fallback) = &mut cursor.fallback {
                    fallback.set_source_file_index(context, source_file_index);
                }
            },
            // Replayed tokens keep the files they came from.
            | Self::Replay(_) => (),
        }
    }
}

impl TranslationPhase for TokenSource {
    type Item = PreprocessorToken;

    #[inline(always)]
    fn next_item(&mut self, context: &mut Context) -> Option<PreprocessorToken> {
        match self {
            | Self::Streaming(tokenizer) => tokenizer.next_item(context),
            | Self::Batch(cursor) => cursor.next_item(context),
            | Self::Replay(cursor) => cursor.next_item(context),
        }
    }
}

/// A position in a [`LexedFile`].
///
/// Reading an entry reports its diagnostics and records its provenance, as
/// lexing it would. Positions the entries cannot express—a rewind into the
/// middle of an entry, or a token lexed as a header name because phase 4 is
/// reading an `#include` operand—are served by a streaming tokenizer over the
/// same source until it reaches an entry boundary again.
#[derive(Clone)]
pub(crate) struct LexedCursor {
    file:                   Rc<LexedFile>,
    /// The next entry to read.
    next:                   usize,
    /// Whether reading past the last entry has already returned `None`.
    finished:               bool,
    /// Added to every line, so `#line` renumbering survives replay.
    line_delta:             u32,
    source_file_index:      u32,
    /// Mirrors [`PreprocessorTokenizer::final_newline_withheld`] for the
    /// entries read so far.
    final_newline_withheld: bool,
    fallback:               Option<Box<PreprocessorTokenizer>>,
}

impl PartialEq for LexedCursor {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.file, &other.file)
            && self.next == other.next
            && self.finished == other.finished
            && self.line_delta == other.line_delta
            && self.source_file_index == other.source_file_index
            && self.final_newline_withheld == other.final_newline_withheld
            && self.fallback == other.fallback
    }
}

impl Eq for LexedCursor {}

impl Debug for LexedCursor {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("LexedCursor")
            .field("source_file_index", &self.source_file_index)
            .field("entries", &self.file.len())
            .field("next", &self.next)
            .field("finished", &self.finished)
            .field("line_delta", &self.line_delta)
            .field("final_newline_withheld", &self.final_newline_withheld)
            .field("fallback", &self.fallback)
            .finish()
    }
}

impl LexedCursor {
    fn new(file: Rc<LexedFile>) -> Self {
        Self {
            source_file_index: file.source_file_index,
            file,
            next: 0,
            finished: false,
            line_delta: 0,
            final_newline_withheld: false,
            fallback: None,
        }
    }

    fn position(&self, context: &Context) -> SourcePosition {
        if let Some(fallback) = &self.fallback {
            return fallback.position(context);
        }
        let mut position = if self.finished {
            self.file.eof()
        } else {
            self.file.start(self.next)
        };
        position.line = position.line.wrapping_add(self.line_delta);
        position
    }

    fn set_position(&mut self, context: &mut Context, position: SourcePosition) {
        if !self.resynchronize(position) {
            self.start_fallback(context, position);
        }
    }

    #[cold]
    #[inline(never)]
    fn start_fallback(&mut self, context: &mut Context, position: SourcePosition) {
        let mut fallback =
            PreprocessorTokenizer::new(self.file.source_file_index, self.file.source.clone());
        fallback.set_source_file_index(context, self.source_file_index);
        fallback.set_position(context, position);
        fallback.set_final_newline_withheld(self.final_newline_withheld);
        self.fallback = Some(Box::new(fallback));
    }

    /// Resumes replay at `position` if an entry starts there.
    fn resynchronize(&mut self, position: SourcePosition) -> bool {
        let Some(entry) = self.file.boundary(position) else {
            return false;
        };
        self.next = entry;
        self.finished = false;
        self.line_delta = position.line.wrapping_sub(self.file.start(entry).line);
        self.fallback = None;
        true
    }

    /// Whether phase 4 wants `entry` lexed as a header name although it was
    /// lexed differently, or wants a header name lexed as ordinary tokens.
    #[inline(always)]
    fn needs_relexing(&self, context: &Context, entry: usize) -> bool {
        use super::PreprocessorTokenType as T;
        let kind = self.file.kind(entry);
        let header_name = matches!(kind, Some(T::AngleBracketString | T::IncludeString));
        if context.is_tokenizing_include_string() {
            !header_name
                && kind.is_some()
                && context
                    .string_cache
                    .at(self.file.contents(entry))
                    .starts_with(['<', '"'])
        } else {
            header_name
        }
    }

    #[inline(always)]
    fn next_item(&mut self, context: &mut Context) -> Option<PreprocessorToken> {
        loop {
            if self.fallback.is_some() {
                return self.next_fallback_item(context);
            }
            let entry = self.next;
            if entry >= self.file.len() {
                if !self.finished && !self.final_newline_withheld && self.file.lacks_final_newline()
                {
                    return Some(self.supply_final_newline(context));
                }
                self.finished = true;
                return None;
            }
            if self.needs_relexing(context, entry) {
                let position = self.position(context);
                self.start_fallback(context, position);
                continue;
            }
            if self.final_newline_withheld && self.file.is_final_newline(entry) {
                self.next = self.file.len();
                self.finished = true;
                return None;
            }
            self.final_newline_withheld = self.file.withholds_final_newline_after(entry);
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
            let vector = context.push_source_vector(start, self.source_file_index, length);
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
    fn supply_final_newline(&mut self, context: &mut Context) -> PreprocessorToken {
        let mut start = self.file.start(self.file.len());
        start.line = start.line.wrapping_add(self.line_delta);
        self.file
            .report_missing_final_newline(context, self.source_file_index, self.line_delta);
        self.final_newline_withheld = true;
        self.finished = true;
        let length = self.file.eof().index - start.index;
        let vector = context.push_source_vector(start, self.source_file_index, length);
        PreprocessorToken {
            kind:           super::PreprocessorTokenType::Newline,
            source_vectors: SourceVectors::new(vector, vector + 1),
            contents:       context.string_cache.intern("\n"),
        }
    }

    #[cold]
    #[inline(never)]
    fn next_fallback_item(&mut self, context: &mut Context) -> Option<PreprocessorToken> {
        let fallback = self
            .fallback
            .as_mut()
            .expect("fallback reading requires a fallback tokenizer");
        let token = fallback.next_item(context);
        let position = fallback.position(context);
        self.final_newline_withheld = fallback.final_newline_withheld();
        if token.is_none() {
            self.fallback = None;
            self.next = self.file.len();
            self.finished = true;
            self.line_delta = position.line.wrapping_sub(self.file.eof().line);
        } else if position.index < self.file.source.len() {
            // At the end of input only the streaming tokenizer knows whether
            // it already supplied the missing final newline, so it finishes.
            _ = self.resynchronize(position);
        }
        token
    }
}
