//! Buffered parser-facing view of the preprocessed token stream.

use std::collections::VecDeque;

use crate::translation_phases::{
    Context,
    TranslationPhase,
    preprocessing::{
        Preprocessor,
        Token,
    },
};

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
    pub(super) preprocessor: Preprocessor,
    /// Token currently owned by the active parser frame.
    current:                 Option<Token>,
    /// Tokens fetched beyond `current`, ordered nearest first.
    lookahead:               VecDeque<Token>,
    /// The most recently consumed token, used to suggest insertions after it.
    pub(super) previous:     Option<Token>,
    /// Whether the upstream preprocessor has returned EOF.
    reached_eof:             bool,
}

impl TokenCursor {
    /// Creates an empty cursor over `preprocessor`; no token is fetched
    /// eagerly.
    pub(super) fn new(preprocessor: Preprocessor) -> Self {
        Self {
            preprocessor,
            current: None,
            lookahead: VecDeque::new(),
            previous: None,
            reached_eof: false,
        }
    }

    /// Fetches the next upstream token and records its provenance in the
    /// parser token arena, so consecutive tokens have adjacent provenance.
    fn fetch(&mut self, context: &mut Context) -> Option<Token> {
        let mut token = self.preprocessor.next_item(context)?;
        token.source_vectors = context.record_parser_token_source(token.source_vectors);
        Some(token)
    }

    /// Returns the current token, fetching it once if necessary.
    pub(super) fn current(&mut self, context: &mut Context) -> Option<Token> {
        if self.current.is_none() && !self.reached_eof {
            self.current = self.fetch(context);
            self.reached_eof = self.current.is_none();
        }
        self.current
    }

    /// Returns the token immediately following `current` without consuming.
    pub(super) fn following(&mut self, context: &mut Context) -> Option<Token> {
        self.lookahead(context, 0)
    }

    /// Returns zero-based lookahead beyond `current` without consuming.
    pub(super) fn lookahead(&mut self, context: &mut Context, index: usize) -> Option<Token> {
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
    }
}
