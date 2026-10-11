//! Recording phase-3 token entries. [`super::super::Lexer::push`] writes a
//! packed entry and attaches its diagnostics and end-of-input reading state.
//! The remaining methods intern spellings, record extension locations, and set
//! up the lexer. Diagnostics are replayed by the token source; this module does
//! not interpret token values or expand macros.
//!
//! C99: §5.1.1.2 paragraph 1 (phase 3), p. 10; PDF p. 22;
//! preprocessing-token categories §6.4 paragraphs 1-3, p. 49; PDF p. 61.
//! Non-newline whitespace is replaced by one space, the implementation-defined
//! choice in phase 3.

use super::{
    super::{
        Lexer,
        LexingFile,
        PreprocessorTokenType,
        PreprocessorTokenizerErrorType,
        positions::PositionTracker,
        splicing::Remap,
        storage::{
            Entry,
            LexDiagnostic,
        },
    },
    Lexed,
};
use crate::{
    configuration::Feature,
    translation_phases::{
        Context,
        SourcePosition,
    },
    util::{
        bump::{
            ArenaVec,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

impl Lexer<'_, '_, '_, '_> {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "LexedFile checked the original source length before lexing"
    )]
    pub(in crate::translation_phases::preprocessor_tokenizer) fn push(
        &mut self,
        position: SourcePosition,
        lexed: Lexed,
    ) {
        let entry = Self::checked_entry_index(self.file.entries.len());
        let line_high = (position.line >> 24) as u8;
        if line_high != self.current_line_high {
            self.file.line_high_starts.push((entry, line_high));
            self.current_line_high = line_high;
        }
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
            // `LexedFile::lex` checked the original source length before
            // phases 1 and 2, which can only shorten it.
            index:    position.index as u32,
            column:   position.column,
            line:     {
                let bytes = position.line.to_le_bytes();
                [bytes[0], bytes[1], bytes[2]]
            },
            kind:     lexed.kind,
        });
    }

    /// Leaves room for the one-past-last entry used when reading EOF.
    pub(in crate::translation_phases::preprocessor_tokenizer) fn checked_entry_index(
        length: usize,
    ) -> u32 {
        let next = length
            .checked_add(1)
            .and_then(|count| u32::try_from(count).ok())
            .expect("lexed file exceeds u32::MAX entries");
        next - 1
    }

    /// Finishes a token spelled exactly by `start..end`.
    pub(in crate::translation_phases::preprocessor_tokenizer) fn spelled(
        &mut self,
        start: usize,
        end: usize,
        kind: PreprocessorTokenType,
    ) -> Lexed {
        self.pos = end;
        Lexed {
            kind:     Some(kind),
            contents: self.intern(start, end),
        }
    }

    /// Digraph-only diagnostics stay outside the common token completion path.
    /// C95 amendment 1 introduced the alternative token spellings.
    /// C99: §6.4.6p3, p. 64; PDF p. 76.
    pub(in crate::translation_phases::preprocessor_tokenizer) fn digraph(
        &mut self,
        start: usize,
        end: usize,
        kind: PreprocessorTokenType,
    ) -> Lexed {
        self.record_spliced_extension(Feature::Digraphs, "digraph", start, end);
        self.spelled(start, end, kind)
    }

    /// Maps a spelling's endpoints to original source bytes. Adjacent splices
    /// stay outside the diagnostic, while splices within the spelling are kept.
    /// C99: phase-2 deletion §5.1.1.2p1, pp. 9-10; PDF pp. 21-22.
    pub(in crate::translation_phases::preprocessor_tokenizer) fn record_spliced_extension(
        &mut self,
        feature: Feature,
        spelling: &'static str,
        start: usize,
        end: usize,
    ) {
        let position = self.tracker.advance_past_deletions(start);
        let end = self.tracker.advance(end);
        self.record_extension(feature, spelling, position, end.index - position.index);
    }

    /// Keeps extension diagnostics beside their entry, so skipped groups stay
    /// silent. C99: §5.1.1.3p1, p. 11; PDF p. 23; GNU lexical extensions.
    pub(in crate::translation_phases::preprocessor_tokenizer) fn record_extension(
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
    pub(in crate::translation_phases::preprocessor_tokenizer) fn respelled(
        &mut self,
        end: usize,
        kind: PreprocessorTokenType,
        spelling: &str,
    ) -> Lexed {
        self.pos = end;
        Lexed {
            kind:     Some(kind),
            contents: self.intern_spelling(spelling),
        }
    }

    /// Queues a diagnostic from `start` to the end of input, which a scan
    /// reached without closing its token. Scan loops keep only the call.
    #[cold]
    #[inline(never)]
    pub(in crate::translation_phases::preprocessor_tokenizer) fn diagnose_to_eof(
        &mut self,
        error_type: PreprocessorTokenizerErrorType,
        start: SourcePosition,
    ) {
        let eof = self.eof_position();
        self.pending.push(LexDiagnostic::Tokenizer {
            error_type,
            start,
            length: eof.index - start.index,
            character: None,
        });
        self.reached_eof = true;
    }

    /// Queues a diagnostic from `start` to the byte offset `end`.
    #[cold]
    #[inline(never)]
    pub(in crate::translation_phases::preprocessor_tokenizer) fn diagnose_to(
        &mut self,
        error_type: PreprocessorTokenizerErrorType,
        start: SourcePosition,
        end: usize,
    ) {
        let at = self.tracker.advance(end);
        self.pending.push(LexDiagnostic::Tokenizer {
            error_type,
            start,
            length: at.index - start.index,
            character: None,
        });
    }

    pub(in crate::translation_phases::preprocessor_tokenizer) fn intern(
        &mut self,
        start: usize,
        end: usize,
    ) -> StringCacheId {
        let text = self.text;
        self.intern_spelling(&text[start..end])
    }

    pub(in crate::translation_phases::preprocessor_tokenizer) fn intern_spelling(
        &mut self,
        spelling: &str,
    ) -> StringCacheId {
        if let [byte @ 0..=127] = spelling.as_bytes() {
            let index = usize::from(*byte);
            if let Some(id) = self.ascii[index] {
                return id;
            }
            let id = self.context.string_cache.intern(spelling);
            self.ascii[index] = Some(id);
            id
        } else {
            self.context.string_cache.intern(spelling)
        }
    }
}

impl<'a, 'tu, 'arena, 's> Lexer<'a, 'tu, 'arena, 's> {
    pub(in crate::translation_phases::preprocessor_tokenizer) fn char_at(
        &self,
        offset: usize,
    ) -> char {
        self.text[offset..]
            .chars()
            .next()
            .expect("offsets are character boundaries")
    }

    /// Where reading past the end stands:
    /// beyond every trailing splice.
    pub(in crate::translation_phases::preprocessor_tokenizer) fn eof_position(
        &self,
    ) -> SourcePosition {
        let mut tracker = self.tracker.clone();
        tracker.advance_past_deletions(self.bytes.len())
    }

    pub(in crate::translation_phases::preprocessor_tokenizer) fn new(
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
            ascii: [None; 128],
            context,
            text,
            bytes,
            tracker: PositionTracker::new(bytes, remaps),
            pos: 0,
            current_line_high: 0,
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
                line_high_starts: ArenaVec::new_in(scratch),
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

    pub(in crate::translation_phases::preprocessor_tokenizer) fn with_terminal_splice(
        mut self,
        terminal_splice: bool,
    ) -> Self {
        self.terminal_splice = terminal_splice;
        self
    }
}
