//! The parser's view of the preprocessed token array.

use crate::{
    translation_phases::{
        Context,
        GetPosition,
        GetSourceFileIndex,
        SetPosition,
        SetSourceFileIndex,
        SourcePosition,
        preprocessing::{
            Preprocessor,
            Token,
        },
    },
    util::region_vec::RegionVec,
};

/// The preprocessed translation unit that the parser reads.
pub(super) struct Upstream {
    /// Phase-6 output remains available until parsing ends.
    tokens: RegionVec<Token>,
    /// Where the preprocessor stopped, used to locate end-of-input
    /// diagnostics.
    end: SourcePosition,
    source_file_index: u32,
    /// The first output token that exceeded the source-provenance budget.
    pub(super) preprocessing_limit_token: Option<Token>,
}

impl Upstream {
    /// Runs the whole of `preprocessor`, so parsing never interleaves with
    /// preprocessing.
    pub(super) fn preprocess_all<'tu>(
        mut preprocessor: Preprocessor<'tu, '_>,
        context: &mut Context<'tu>,
        source_segment_limit: usize,
    ) -> Self {
        let mut tokens = RegionVec::new();
        let preprocessing_limit_token =
            preprocessor.preprocess_into_arena(context, source_segment_limit, &mut tokens);
        Self {
            tokens,
            end: preprocessor.end_position(),
            source_file_index: preprocessor.end_source_file_index(),
            preprocessing_limit_token,
        }
    }
}

impl GetPosition for Upstream {
    fn position(&self, _context: &Context<'_>) -> SourcePosition {
        self.end
    }
}

impl SetPosition for Upstream {
    fn set_position(&mut self, _context: &mut Context<'_>, position: SourcePosition) {
        self.end = position;
    }
}

impl GetSourceFileIndex for Upstream {
    fn source_file_index(&self) -> u32 {
        self.source_file_index
    }
}

impl SetSourceFileIndex for Upstream {
    fn set_source_file_index(&mut self, _context: &mut Context<'_>, source_file_index: u32) {
        self.source_file_index = source_file_index;
    }
}

/// The parser's current-token and arbitrary-lookahead view of the
/// preprocessed token array.
///
/// The whole translation unit is preprocessed before parsing starts, so the
/// cursor is an index into that array: lookahead reads ahead of it and
/// buffers nothing. `consume` advances exactly one token, and lookahead never
/// changes `current`.
///
/// C99: this cursor consumes the phase-7 token stream described by §5.1.1.2,
/// phases 6-7, pp. 9-10; PDF pp. 21-22. Token categories are specified by
/// §6.4, pp. 49-50; PDF pp. 61-62.
pub(super) struct TokenCursor {
    /// The preprocessed token array.
    pub(super) upstream: Upstream,
    /// The index of the current token in the array.
    position:            usize,
    /// Tokens at or past this index read as the end of input: the array's
    /// length, or the current position once input is abandoned.
    end:                 usize,
    /// The most recently consumed token, used to suggest insertions after it.
    pub(super) previous: Option<Token>,
    /// Number of tokens consumed so far.
    pub(super) consumed: usize,
}

impl TokenCursor {
    /// Creates a cursor at the first token of `upstream`.
    pub(super) fn new(upstream: Upstream) -> Self {
        let end = upstream.tokens.len();
        Self {
            upstream,
            position: 0,
            end,
            previous: None,
            consumed: 0,
        }
    }

    /// The token `offset` places past the current one, or `None` at the end
    /// of input.
    fn at(&self, offset: usize) -> Option<Token> {
        let index = self.position.checked_add(offset)?;
        (index < self.end).then(|| self.upstream.tokens[index])
    }

    /// Returns the current token.
    pub(super) fn current(&mut self, _context: &mut Context<'_>) -> Option<Token> {
        self.at(0)
    }

    /// Returns the token immediately following `current` without consuming.
    pub(super) fn following(&mut self, context: &mut Context<'_>) -> Option<Token> {
        self.lookahead(context, 0)
    }

    /// Returns zero-based lookahead beyond `current` without consuming.
    pub(super) fn lookahead(&mut self, _context: &mut Context<'_>, index: usize) -> Option<Token> {
        self.at(0).and_then(|_| self.at(index.checked_add(1)?))
    }

    /// Reports EOF from now on without reading the rest of the input.
    pub(super) fn abandon(&mut self) {
        self.end = self.position.min(self.end);
    }

    /// Advances by one token.
    pub(super) fn consume(&mut self) {
        debug_assert!(self.at(0).is_some(), "cannot consume parser EOF");
        self.previous = self.at(0);
        self.position += 1;
        self.consumed += 1;
    }
}
