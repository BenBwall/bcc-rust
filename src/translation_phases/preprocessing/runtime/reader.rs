//! Parser-token production, provenance compaction guards, and token
//! expectations.
//!
//! C99: translation phases 4-7, §5.1.1.2 paragraph 1 items 4-7, p. 10; PDF p.
//! 22; directive grammar, §6.10 paragraph 1, p. 145; PDF p. 157;
//! macro-invocation whitespace, §6.10.3 paragraph 10, p. 152; PDF p. 164.
//! Token conversion and string concatenation are delegated to
//! `token_conversion`.

use std::ops::ControlFlow;

use super::{
    super::{
        Expander,
        HashHash,
        PreprocessorError,
        PreprocessorErrorType,
        Token,
    },
    OutputPurpose,
    TokenizerFrame,
    TokenizerFrameType,
};
use crate::translation_phases::{
    Context,
    TranslationPhase,
    preprocessor_tokenizer::{
        PreprocessorToken,
        PreprocessorTokenType,
        TokenSource,
    },
};

impl Expander<'_, '_, '_, '_> {
    /// Returns the next phase-7 token, executing directives on the way.
    /// Conditionals still open at the end of input lack the `endif-line`
    /// that ends each `if-section`.
    ///
    /// C99: §6.10 paragraph 1, p. 145; PDF p. 157.
    pub(in crate::translation_phases::preprocessing) fn next_parser_token(
        &mut self,
    ) -> Option<Token> {
        self.context
            .append_pending_errors(self.pending_parser_errors.drain(..));
        if let Some(token) = self.pending_parser_token.take() {
            return Some(token);
        }
        loop {
            let Some(token) = self.next_preprocessor_token::<true>() else {
                for vectors in self.state.open_conditionals.drain(..) {
                    let source_vectors = self.context.push_source_vectors(vectors.source);
                    self.context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::MoreIfDirectivesThanEndifDirectives,
                        source_vectors,
                    });
                }
                return None;
            };

            if let Some(result) = self.map_preprocessor_token(token) {
                if matches!(self.output_purpose, OutputPurpose::Parsing)
                    && let Some(site) = self.expansion_end()
                {
                    self.context
                        .record_expansion_end(result.source_vectors, site);
                }
                return Some(result);
            }
        }
    }

    /// Whether the next [`Self::next_iterator_item`] call discards the
    /// preprocessor provenance arena. Consumers retaining provenance across
    /// calls, such as a deferred diagnostic, must resolve it first.
    ///
    /// A `##` operand held while the other operand's argument expands still
    /// refers to the arena, so compaction waits until every paste completes.
    pub(in crate::translation_phases::preprocessing) fn next_iterator_item_compacts(&self) -> bool {
        self.pending_parser_token.is_none()
            && self.pending_parser_errors.is_empty()
            && self
                .hash_hash_stack
                .iter()
                .all(|operand| matches!(operand, HashHash::Empty))
    }
}

impl Expander<'_, '_, '_, '_> {
    /// Returns the next phase-6 token, with adjacent string literals
    /// concatenated.
    ///
    /// C99: §5.1.1.2 paragraph 1 item 6, p. 10; PDF p. 22.
    pub(in crate::translation_phases::preprocessing) fn next_item(&mut self) -> Option<Token> {
        let token = self.next_parser_token()?;
        Some(self.concatenate_adjacent_strings(token))
    }
}

#[expect(
    clippy::needless_continue,
    reason = "Explicit continues make this tokenizer's nested control flow easier to audit."
)]
impl<'tu> Expander<'_, 'tu, '_, '_> {
    /// Marks the reader as at the start of a line, where a `#` begins a
    /// directive: a directive's line has been read through its new-line, or
    /// a group was skipped up to the next line.
    pub(in crate::translation_phases::preprocessing) fn resume_at_line_start(&mut self) {
        self.last_was_newline = true;
        self.current_is_newline = true;
    }

    pub(in crate::translation_phases::preprocessing) fn skip_until_newline(&mut self) {
        loop {
            if matches!(
                self.tokenizer.next_item(self.context),
                Some(PreprocessorToken {
                    kind: PreprocessorTokenType::Newline,
                    ..
                }) | None
            ) {
                self.resume_at_line_start();
                return;
            }
        }
    }

    pub(in crate::translation_phases::preprocessing) fn skip_and_expand_until_newline(&mut self) {
        loop {
            if matches!(
                self.next_preprocessor_token::<true>(),
                Some(PreprocessorToken {
                    kind: PreprocessorTokenType::Newline,
                    ..
                }) | None
            ) {
                self.resume_at_line_start();
                return;
            }
        }
    }

    pub(in crate::translation_phases::preprocessing) fn expect_token_preserving_rejected<
        const SHOULD_IGNORE_WHITESPACE: bool,
    >(
        &mut self,
        is_correct_token: impl FnMut(&mut Self, PreprocessorToken) -> bool,
        on_wrong_token_type: impl FnMut(
            &mut Self,
            PreprocessorToken,
        ) -> ControlFlow<PreprocessorError<'tu>>,
        eof_message: &'static str,
    ) -> Option<PreprocessorToken> {
        self.expect_expanded_token::<SHOULD_IGNORE_WHITESPACE>(
            is_correct_token,
            on_wrong_token_type,
            eof_message,
            true,
        )
    }

    pub(in crate::translation_phases::preprocessing) fn expect_token_without_rewind<
        const SHOULD_IGNORE_WHITESPACE: bool,
    >(
        &mut self,
        is_correct_token: impl FnMut(&mut Self, PreprocessorToken) -> bool,
        on_wrong_token_type: impl FnMut(
            &mut Self,
            PreprocessorToken,
        ) -> ControlFlow<PreprocessorError<'tu>>,
        eof_message: &'static str,
    ) -> Option<PreprocessorToken> {
        self.expect_expanded_token::<SHOULD_IGNORE_WHITESPACE>(
            is_correct_token,
            on_wrong_token_type,
            eof_message,
            false,
        )
    }

    fn expect_expanded_token<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
        mut is_correct_token: impl FnMut(&mut Self, PreprocessorToken) -> bool,
        mut on_wrong_token_type: impl FnMut(
            &mut Self,
            PreprocessorToken,
        ) -> ControlFlow<PreprocessorError<'tu>>,
        eof_message: &'static str,
        preserve_rejected: bool,
    ) -> Option<PreprocessorToken> {
        loop {
            match self.next_preprocessor_token::<SHOULD_IGNORE_WHITESPACE>() {
                | Some(token) => {
                    if SHOULD_IGNORE_WHITESPACE && token.kind == PreprocessorTokenType::Whitespace {
                        continue;
                    }
                    if is_correct_token(self, token) {
                        return Some(token);
                    }
                    match on_wrong_token_type(self, token) {
                        | ControlFlow::Continue(()) => continue,
                        | ControlFlow::Break(e) => {
                            if preserve_rejected {
                                // Expansion can change the active source.
                                // Replay the token and its provenance.
                                // A saved position may belong elsewhere.
                                let location = self
                                    .context
                                    .first_source_vector(token.source_vectors)
                                    .clone();
                                let tokenizer = TokenSource::replay(
                                    self.context,
                                    self.scratch,
                                    &[std::slice::from_ref(&token)],
                                    location,
                                );
                                self.push_tokenizer_frame(TokenizerFrame {
                                    frame_type: TokenizerFrameType::Rescan { argument: false },
                                    tokenizer,
                                });
                            }
                            self.context.preprocessor_error(e);
                            return None;
                        },
                    }
                },
                | None => {
                    let source_vectors = self.current_location();
                    self.context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(eof_message),
                        source_vectors,
                    });
                    return None;
                },
            }
        }
    }

    pub(in crate::translation_phases::preprocessing) fn expect_token_from_previous_phase<
        const SHOULD_IGNORE_WHITESPACE: bool,
    >(
        &mut self,
        mut is_correct_token: impl FnMut(&mut Self, PreprocessorToken) -> bool,
        mut on_wrong_token_type: impl FnMut(
            &mut Self,
            PreprocessorToken,
        ) -> ControlFlow<PreprocessorError<'tu>>,
        eof_message: &'static str,
    ) -> Option<PreprocessorToken> {
        loop {
            let start = self.position();
            match self.tokenizer.next_item(self.context) {
                | Some(token) => {
                    if SHOULD_IGNORE_WHITESPACE && token.kind == PreprocessorTokenType::Whitespace {
                        continue;
                    }
                    if is_correct_token(self, token) {
                        return Some(token);
                    }
                    match on_wrong_token_type(self, token) {
                        | ControlFlow::Continue(()) => continue,
                        | ControlFlow::Break(e) => {
                            self.set_position(start);
                            self.context.preprocessor_error(e);
                            return None;
                        },
                    }
                },
                | None => {
                    self.set_position(start);
                    let source_vectors = self.location_at(start);
                    self.context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(eof_message),
                        source_vectors,
                    });
                    return None;
                },
            }
        }
    }

    pub(in crate::translation_phases::preprocessing) fn next_ignore_whitespace(
        tokenizer: &mut TokenSource<'_>,
        context: &mut Context<'_>,
    ) -> Option<PreprocessorToken> {
        loop {
            match tokenizer.next_item(context) {
                | Some(t) if t.kind == PreprocessorTokenType::Whitespace => continue,
                | Some(t) => return Some(t),
                | None => return None,
            }
        }
    }

    /// Reads the next token of a macro invocation, where a new-line is an
    /// ordinary white-space character; runs of whitespace collapse to one.
    ///
    /// C99: §6.10.3 paragraph 10, p. 152; PDF p. 164.
    pub(in crate::translation_phases::preprocessing) fn next_treat_newlines_as_whitespace(
        tokenizer: &mut TokenSource<'_>,
        context: &mut Context<'_>,
        last_was_whitespace: &mut bool,
    ) -> Option<PreprocessorToken> {
        loop {
            match tokenizer.next_item(context) {
                | Some(mut t) => {
                    match t.kind {
                        | PreprocessorTokenType::Whitespace => {
                            if *last_was_whitespace {
                                continue;
                            }
                            *last_was_whitespace = true;
                        },
                        | PreprocessorTokenType::Newline => {
                            if *last_was_whitespace {
                                continue;
                            }
                            *last_was_whitespace = true;
                            t.contents = context.string_cache.intern(" ");
                            t.kind = PreprocessorTokenType::Whitespace;
                        },
                        | _ => *last_was_whitespace = false,
                    }
                    return Some(t);
                },
                | None => return None,
            }
        }
    }
}
