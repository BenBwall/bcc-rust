//! Buffered parser-facing view of the preprocessed token stream.

use std::collections::VecDeque;

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
    util::bump::{
        Bump,
        RegionVec,
    },
};

/// The preprocessed translation unit that the parser reads.
pub(super) struct Upstream {
    /// Phase-6 output remains available until parsing ends.
    tokens: RegionVec<Token>,
    next_token: usize,
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
        let mut tokens = RegionVec::new_in(Bump::new());
        let preprocessing_limit_token =
            preprocessor.preprocess_into_arena(context, source_segment_limit, &mut tokens);
        Self {
            tokens,
            next_token: 0,
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

/// Buffered adapter from the preprocessor's iterator interface to parser
/// current-token and arbitrary-lookahead operations.
///
/// `consume` advances exactly one token. Lookahead never changes `current`, and
/// EOF is memoized so the preprocessor is not polled after completion.
///
/// C99: this cursor consumes the phase-7 token stream described by §5.1.1.2,
/// phases 6-7, pp. 9-10; PDF pp. 21-22. Token categories are specified by
/// §6.4, pp. 49-50; PDF pp. 61-62.
pub(super) struct TokenCursor {
    /// Upstream producer of parser-facing tokens.
    pub(super) upstream: Upstream,
    /// Token currently owned by the active parser frame.
    current:             Option<Token>,
    /// Tokens fetched beyond `current`, ordered nearest first.
    lookahead:           VecDeque<Token>,
    /// The most recently consumed token, used to suggest insertions after it.
    pub(super) previous: Option<Token>,
    /// Whether the upstream preprocessor has returned EOF.
    reached_eof:         bool,
    /// Number of tokens consumed so far.
    pub(super) consumed: usize,
}

impl TokenCursor {
    /// Creates an empty cursor over `upstream`; no token is fetched eagerly.
    pub(super) fn new(upstream: Upstream) -> Self {
        Self {
            upstream,
            current: None,
            lookahead: VecDeque::new(),
            previous: None,
            reached_eof: false,
            consumed: 0,
        }
    }

    /// Fetches the next upstream token. Its provenance was copied to the
    /// parser-token provenance when it was preprocessed, so consecutive
    /// tokens have adjacent provenance.
    fn fetch(&mut self, _context: &mut Context<'_>) -> Option<Token> {
        let token = self.upstream.tokens.get(self.upstream.next_token).copied();
        if token.is_some() {
            self.upstream.next_token += 1;
        }
        token
    }

    /// Returns the current token, fetching it once if necessary.
    pub(super) fn current(&mut self, context: &mut Context<'_>) -> Option<Token> {
        if self.current.is_none() && !self.reached_eof {
            self.current = self.fetch(context);
            self.reached_eof = self.current.is_none();
        }
        self.current
    }

    /// Returns the token immediately following `current` without consuming.
    pub(super) fn following(&mut self, context: &mut Context<'_>) -> Option<Token> {
        self.lookahead(context, 0)
    }

    /// Returns zero-based lookahead beyond `current` without consuming.
    pub(super) fn lookahead(&mut self, context: &mut Context<'_>, index: usize) -> Option<Token> {
        let _ = self.current(context)?;
        while self.lookahead.len() <= index && !self.reached_eof {
            let next = self.fetch(context);
            self.reached_eof = next.is_none();
            if let Some(next) = next {
                self.lookahead.push_back(next);
            }
        }
        self.lookahead.get(index).copied()
    }

    /// Drops buffered tokens and reports EOF without fetching the rest of
    /// the input.
    pub(super) fn abandon(&mut self) {
        self.current = None;
        self.lookahead.clear();
        self.reached_eof = true;
    }

    /// Advances by one token while preserving any buffered lookahead.
    pub(super) fn consume(&mut self) {
        debug_assert!(self.current.is_some(), "cannot consume parser EOF");
        self.previous = self.current;
        self.current = self.lookahead.pop_front();
        self.consumed += 1;
    }
}
