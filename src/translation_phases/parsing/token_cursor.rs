//! The parser's view of the preprocessed token array.
//!
//! Infrastructure for translation phase 7 (§5.1.1.2 paragraph 1, p. 10;
//! PDF p. 22): the array holds the phase-6 output of the whole translation
//! unit, and the parser reads it as tokens (§6.4 paragraphs 1-3, p. 49;
//! PDF p. 61).

use crate::{
    translation_phases::{
        Context,
        GetPosition,
        GetSourceFileIndex,
        SourcePosition,
        SourceVectors,
        preprocessing::{
            OperatorTokenType,
            Preprocessor,
            Token,
            TokenType,
        },
        provenance::SourceArena,
    },
    util::{
        bump::{
            ArenaVec,
            Bump,
        },
        region_vec::RegionVec,
    },
};

/// The parser's current-token and arbitrary-lookahead view of the
/// preprocessed token array.
///
/// The whole translation unit is preprocessed before parsing starts, so the
/// cursor is an index into that array: lookahead reads ahead of it and
/// buffers nothing. `consume` advances exactly one token, and lookahead never
/// changes `current`.
///
/// C99: this cursor consumes the phase-7 token stream described by §5.1.1.2,
/// phases 6-7, p. 10; PDF p. 22. Token categories are specified by
/// §6.4, pp. 49-50; PDF pp. 61-62.
pub(super) struct TokenCursor<'tu, 'p> {
    /// The preprocessed token array.
    pub(super) upstream: Upstream,
    /// The index of the current token in the array.
    position: usize,
    /// Tokens at or past this index read as the end of input: the array's
    /// length, or the current position once input is abandoned.
    end: usize,
    /// The most recently consumed token, used to suggest insertions after it.
    pub(super) previous: Option<Token>,
    /// Number of tokens consumed so far.
    pub(super) consumed: usize,
    /// One shared retained copy, allocated only when an attribute needs it.
    retained: Option<&'tu [Token]>,
    /// Whether each opening delimiter has a complete alignment operand.
    alignment_closures: Option<&'p [bool]>,
    #[cfg(test)]
    pub(super) alignment_work: usize,
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

impl TokenCursor<'_, '_> {
    /// Advances by one token.
    pub(super) fn consume(&mut self) {
        debug_assert!(self.at(0).is_some(), "cannot consume parser EOF");
        self.previous = self.at(0);
        self.position += 1;
        self.consumed += 1;
    }
}

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

impl<'tu, 'p> TokenCursor<'tu, 'p> {
    /// Reports EOF from now on without reading the rest of the input.
    pub(super) fn abandon(&mut self) {
        self.end = self.position.min(self.end);
    }

    /// Creates a cursor at the first token of `upstream`.
    pub(super) fn new(upstream: Upstream) -> Self {
        let end = upstream.tokens.len();
        Self {
            upstream,
            position: 0,
            end,
            previous: None,
            consumed: 0,
            retained: None,
            alignment_closures: None,
            #[cfg(test)]
            alignment_work: 0,
        }
    }

    /// The token `offset` places past the current one, or `None` at the end
    /// of input.
    fn at(&self, offset: usize) -> Option<Token> {
        let index = self.position.checked_add(offset)?;
        (index < self.end).then(|| self.upstream.tokens[index])
    }

    /// Returns the current token.
    pub(super) fn current(&self) -> Option<Token> {
        self.at(0)
    }

    /// Returns the token immediately following `current` without consuming.
    pub(super) fn following(&self) -> Option<Token> {
        self.lookahead(0)
    }

    /// Returns zero-based lookahead beyond `current` without consuming.
    pub(super) fn lookahead(&self, index: usize) -> Option<Token> {
        self.at(0).and_then(|_| self.at(index.checked_add(1)?))
    }

    /// Tokens consumed by a child production, retained by an enclosing
    /// attribute without reparsing or advancing the cursor. `start` is a saved
    /// consumed count from this cursor.
    pub(super) fn consumed_since(&self, start: usize) -> &[Token] {
        &self.upstream.tokens[start..self.position]
    }

    /// Share ranges of the phase-6 array rather than copying nested operands.
    /// C99: translation phases 6-7, §5.1.1.2p1, p. 10; PDF p. 22.
    pub(super) fn retain_since(&mut self, start: usize, arena: &'tu Bump) -> &'tu [Token] {
        let tokens = *self.retained.get_or_insert_with(|| {
            #[cfg(test)]
            {
                self.alignment_work += self.upstream.tokens.len();
            }
            arena.alloc_slice_copy(&self.upstream.tokens)
        });
        &tokens[start..self.position]
    }

    /// Phase-6 collection retains consecutive token provenance adjacently.
    /// A consumed token range therefore also shares its exact provenance,
    /// including macro/include vectors and discarded malformed tokens.
    /// C99: §5.1.1.3p1, p. 11; PDF p. 23.
    pub(super) fn source_since(&self, start: usize) -> SourceVectors {
        let tokens = self.consumed_since(start);
        let (Some(first), Some(last)) = (tokens.first(), tokens.last()) else {
            return SourceVectors::default();
        };
        let (arena, begin) = SourceArena::decode(first.source_vectors);
        let (last_arena, end) = SourceArena::decode(last.source_vectors);
        debug_assert_eq!(
            arena,
            SourceArena::ParserTokens,
            "phase-6 token source was retained"
        );
        debug_assert_eq!(
            last_arena, arena,
            "consumed token sources share retained storage"
        );
        arena.encode(begin, end + last.source_vectors.length())
    }

    /// Index all delimiter matches once, using an explicit stack. Semicolons
    /// invalidate operands except within a descendant brace pair, exactly as
    /// the attribute recovery lookahead requires. Mismatches invalidate every
    /// pending opener; EOF leaves unmatched openers false.
    /// C99: §6.7, p. 97; PDF p. 109 (GNU aligned operand extension).
    pub(super) fn alignment_argument_closes(&mut self, arena: &'p Bump) -> bool {
        if self.alignment_closures.is_none() {
            let mut matches = ArenaVec::with_capacity_in(self.upstream.tokens.len(), arena);
            matches.resize(self.upstream.tokens.len(), false);
            let mut stack = ArenaVec::new_in(arena);
            let mut semicolons = 0usize;
            for (index, token) in self.upstream.tokens.iter().enumerate() {
                #[cfg(test)]
                {
                    self.alignment_work += 1;
                }
                let TokenType::Operator(operator) = token.kind else {
                    continue;
                };
                let closing = match operator {
                    | OperatorTokenType::OpeningParenthesis =>
                        Some(OperatorTokenType::ClosingParenthesis),
                    | OperatorTokenType::OpeningSquareBracket =>
                        Some(OperatorTokenType::ClosingSquareBracket),
                    | OperatorTokenType::OpeningCurlyBrace =>
                        Some(OperatorTokenType::ClosingCurlyBrace),
                    | _ => None,
                };
                if let Some(closing) = closing {
                    stack.push((index, closing, semicolons));
                } else if operator == OperatorTokenType::Semicolon {
                    semicolons += 1;
                } else if matches!(
                    operator,
                    OperatorTokenType::ClosingParenthesis
                        | OperatorTokenType::ClosingSquareBracket
                        | OperatorTokenType::ClosingCurlyBrace
                ) {
                    if let Some((opening, expected, before)) = stack.pop()
                        && expected == operator
                    {
                        let brace = operator == OperatorTokenType::ClosingCurlyBrace;
                        matches[opening] = brace || before == semicolons;
                        if brace {
                            semicolons = before;
                        }
                    } else {
                        stack.clear();
                    }
                }
            }
            // C99 §6.7, p. 97; PDF p. 109 (GNU aligned operand extension):
            // both study paths retain the same delimiter matches for parsing.
            #[cfg(feature = "parser-study-leak-alignment")]
            {
                self.alignment_closures = Some(matches.leak());
            }
            #[cfg(not(feature = "parser-study-leak-alignment"))]
            {
                self.alignment_closures = Some(arena.alloc_slice_copy(&matches));
            }
        }
        self.alignment_closures
            .is_some_and(|matches| matches.get(self.position) == Some(&true))
    }
}

impl GetPosition for Upstream {
    fn position(&self, _context: &Context<'_>) -> SourcePosition {
        self.end
    }
}

impl GetSourceFileIndex for Upstream {
    fn source_file_index(&self) -> u32 {
        self.source_file_index
    }
}
