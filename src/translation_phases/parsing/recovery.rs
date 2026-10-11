//! Synchronization sets and delimiter tracking for syntax recovery.
//!
//! Serves translation phase 7 (§5.1.1.2 paragraph 1, p. 10; PDF p. 22).
//! C99 requires at least one diagnostic for each syntax-rule or constraint
//! violation (§5.1.1.3 paragraph 1, p. 11; PDF p. 23) and lets translation
//! continue afterwards (footnote 8, p. 11; PDF p. 23), but does not specify
//! how a parser resynchronizes. The stop tokens here come from the
//! delimiters and terminators of the productions they recover.

use std::fmt::Debug;

#[cfg(test)]
use super::FrameTraceEvent;
use super::{
    Parser,
    errors::{
        RecoverySummary,
        RelatedParserDiagnostic,
    },
    expression_operators::is_operator,
    machine::ParseFrameKind,
    statement::is_statement_keyword,
};
use crate::{
    translation_phases::{
        SourceVectors,
        TranslationError,
        preprocessing::{
            KeywordTokenType,
            OperatorTokenType,
            TokenType,
        },
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

impl Parser<'_, '_, '_> {
    /// Consumes malformed input until the active synchronization policy says
    /// its owning frame can safely resume.
    ///
    /// C99: continued translation after a required diagnostic is permitted by
    /// §5.1.1.3 paragraph 1 and footnote 8, p. 11; PDF p. 23. C99 does not
    /// specify how an implementation resynchronizes.
    #[cold]
    #[inline(never)]
    pub(super) fn recover(
        &mut self,
        set: SynchronizationSet,
        #[cfg(test)] depth: usize,
    ) -> Option<SourceVectors> {
        let mut source_vectors = None;
        let mut consumed_tokens = 0_usize;
        self.recovery.begin(set);

        while let Some(token) = self.cursor.current() {
            let state = self.recovery.active();
            let recovery_set = state.set;
            let at_top_level = state.parentheses == 0 && state.brackets == 0 && state.braces == 0;
            let delimiter_depth = DelimiterDepth {
                parentheses: state.parentheses,
                brackets:    state.brackets,
                braces:      state.braces,
            };
            let colon_matches_conditional =
                matches!(token.kind, TokenType::Operator(OperatorTokenType::Colon))
                    && state
                        .questions
                        .iter()
                        .rev()
                        .any(|question| *question == delimiter_depth);
            let at_unambiguous_owning_delimiter = !colon_matches_conditional
                && recovery_set.kind.stops_before_despite_unbalanced_child(
                    token.kind,
                    state.parentheses,
                    state.brackets,
                    state.braces,
                );
            let stops_at_initial_declaration = matches!(
                (recovery_set.kind, recovery_set.target),
                |(
                    SynchronizationKind::Declaration
                    | SynchronizationKind::BlockDeclaration
                    | SynchronizationKind::OldStyleParameter
                    | SynchronizationKind::Parameter,
                    _,
                )| (
                    SynchronizationKind::StructMember,
                    ParseFrameKind::StructOrUnionSpecifier
                ) | (
                    SynchronizationKind::EnumeratorValue,
                    ParseFrameKind::EnumSpecifier
                )
            );
            let stops_at_declaration_after_malformed_prefix = matches!(
                recovery_set.kind,
                SynchronizationKind::ArrayBound
                    | SynchronizationKind::VariadicParameterList
                    | SynchronizationKind::StructMember
                    | SynchronizationKind::EnumeratorValue
                    | SynchronizationKind::StatementExpression(_)
            );
            let at_next_declaration = (stops_at_initial_declaration
                || consumed_tokens > 0 && stops_at_declaration_after_malformed_prefix)
                && at_top_level
                && self.declaration_starter(token)
                && !(matches!(token.kind, TokenType::Identifier)
                    && matches!(
                        state.last_token,
                        Some(TokenType::Operator(
                            OperatorTokenType::Period | OperatorTokenType::Arrow
                        ))
                    ));
            let at_next_k_and_r_identifier =
                matches!(recovery_set.kind, SynchronizationKind::KAndRParameter)
                    && at_top_level
                    && matches!(token.kind, TokenType::Identifier)
                    && !self.scopes.is_typedef(token.contents);
            let at_next_enumerator =
                matches!(recovery_set.kind, SynchronizationKind::EnumeratorValue)
                    && at_top_level
                    && recovery_set.target == ParseFrameKind::EnumSpecifier
                    && matches!(token.kind, TokenType::Identifier);
            let has_pending_conditional_at_depth = state.questions.contains(&delimiter_depth);
            let at_next_identifier_label =
                (matches!(
                    recovery_set.kind,
                    SynchronizationKind::StatementExpression(ExpressionTerminator::Semicolon)
                ) || matches!(recovery_set.kind, SynchronizationKind::BlockDeclaration)
                    || matches!(recovery_set.kind, SynchronizationKind::Statement)
                        && consumed_tokens > 0)
                    && at_top_level
                    && !has_pending_conditional_at_depth
                    && matches!(token.kind, TokenType::Identifier)
                    && is_operator(self.cursor.following(), OperatorTokenType::Colon);
            let at_statement_body_brace = (matches!(
                recovery_set.kind,
                SynchronizationKind::StatementExpression(
                    ExpressionTerminator::ClosingParenthesis | ExpressionTerminator::Semicolon
                )
            ) || matches!(
                recovery_set.kind,
                SynchronizationKind::BlockDeclaration | SynchronizationKind::ForInitializer
            )) && at_top_level
                && matches!(
                    token.kind,
                    TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
                )
                && (!matches!(
                    state.last_token,
                    Some(TokenType::Operator(OperatorTokenType::ClosingParenthesis))
                ) || !state.last_closed_parenthesis_was_type_name
                    || state.last_closed_parenthesis_was_sizeof_type_name
                        && self.cursor.following().is_some_and(|following| {
                            is_statement_keyword(following.kind)
                                || !matches!(following.kind, TokenType::Identifier)
                                    && self.declaration_starter(following)
                                || matches!(
                                    following.kind,
                                    TokenType::Operator(
                                        OperatorTokenType::Semicolon
                                            | OperatorTokenType::ClosingCurlyBrace
                                    )
                                )
                        }));
            let at_next_statement_keyword = matches!(
                recovery_set.kind,
                SynchronizationKind::Statement | SynchronizationKind::ForInitializer
            ) && consumed_tokens > 0
                && at_top_level
                && is_statement_keyword(token.kind);
            if at_unambiguous_owning_delimiter
                || at_next_declaration
                || at_next_k_and_r_identifier
                || at_next_enumerator
                || at_next_identifier_label
                || at_statement_body_brace
                || at_next_statement_keyword
                || at_top_level
                    && !colon_matches_conditional
                    && recovery_set.kind.stops_before(token.kind)
            {
                break;
            }

            let opens_type_name = matches!(
                token.kind,
                TokenType::Operator(OperatorTokenType::OpeningParenthesis)
            ) && self
                .cursor
                .following()
                .is_some_and(|following| self.declaration_starter(following));
            self.recovery.consume(token.kind, opens_type_name);

            #[cfg(test)]
            self.trace.push(FrameTraceEvent {
                frame: set.target.label(),
                action: "recover-consume",
                token: Some(token.kind),
                depth,
            });
            self.merge_source(&mut source_vectors, token);
            self.suppress_token_diagnostic(token);
            self.cursor.consume();
            consumed_tokens += 1;
        }
        let stopped_token = self.cursor.current();
        let stopped_at = stopped_token.map(|token| token.kind);
        let ranges = source_vectors.map(|discarded| self.context.diagnostic_slice(&[discarded]));
        let related = stopped_token.map(|token| {
            self.context.diagnostic_slice(&[RelatedParserDiagnostic {
                message:        "parsing resumes here",
                source_vectors: token.source_vectors,
            }])
        });
        if let Some(TranslationError::Parsing(error)) = self.context
            .pending_errors
            .iter_mut()
            .rev()
            .find(|error| matches!(error, TranslationError::Parsing(error) if error.recovery.is_none()))
        {
            if let Some(ranges) = ranges {
                error.ranges = ranges;
            }
            if let Some(related) = related {
                error.related = related;
            }
            error.recovery = Some(RecoverySummary {
                owner: set.target,
                discarded: source_vectors,
                discarded_tokens: consumed_tokens,
                stopped_at,
            });
        }
        self.recovery.finish();
        source_vectors
    }
}

/// A production-specific synchronization policy and the frame allowed to
/// resume after the scan.
///
/// C99: recovery serves the diagnostic requirement in §5.1.1.3, p. 11; PDF
/// p. 23. The standard does not prescribe synchronization sets.
#[derive(Debug, Clone, Copy)]
pub(super) struct SynchronizationSet {
    pub(super) kind:   SynchronizationKind,
    pub(super) target: ParseFrameKind,
}

/// The token a statement-owned expression ends at.
///
/// C99: `Semicolon` ends an `expression-statement` or `return` operand
/// (§6.8.3 paragraph 1, p. 132; PDF p. 144; §6.8.6 paragraph 1, p. 136;
/// PDF p. 148); `ForSemicolon` ends the first two `for` clauses (§6.8.5
/// paragraph 1, p. 135; PDF p. 147); `ClosingParenthesis` ends a
/// controlling expression (§6.8.4 paragraph 1, p. 133; PDF p. 145; §6.8.5
/// paragraph 1, p. 135; PDF p. 147); `Colon` ends a `case` expression
/// (§6.8.1 paragraph 1, p. 131; PDF p. 143) or a conditional's middle
/// operand (§6.5.15 paragraph 1, p. 90; PDF p. 102).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExpressionTerminator {
    Semicolon,
    ForSemicolon,
    ClosingParenthesis,
    Colon,
}

/// Selects the grammar-specific boundary rules used during recovery.
///
/// C99: boundaries are derived from the productions in §6.5-§6.9,
/// pp. 67-144; PDF pp. 79-156; C99 does not specify recovery itself.
#[derive(Debug, Clone, Copy)]
pub(super) enum SynchronizationKind {
    /// Stop before a declarator separator, terminator, or enclosing brace.
    Declaration,
    /// Stop before a block statement keyword as well as declaration boundaries.
    BlockDeclaration,
    /// Stop before the function body of an old-style definition.
    OldStyleParameter,
    /// Stop before a `for` header's closing parenthesis as well as declaration
    /// separators.
    ForInitializer,
    /// Stop before the owning `]` or an enclosing declaration boundary.
    ArrayBound,
    /// Stop before a prototype parameter separator or enclosing boundary.
    Parameter,
    /// Stop before an identifier-list separator or enclosing boundary.
    KAndRParameter,
    /// Stop before the `)` that must follow `...` or an enclosing boundary.
    VariadicParameterList,
    /// Consume a comma-introduced parameter that illegally follows `...`.
    VariadicTrailingParameter,
    /// Stop before a member separator or enclosing struct boundary.
    StructMember,
    /// Stop before an enumerator separator or enclosing enum boundary.
    EnumeratorValue,
    /// Stop before a caller-owned statement-expression delimiter.
    StatementExpression(ExpressionTerminator),
    /// Stop before a statement boundary while retaining the enclosing `}`.
    Statement,
}

/// Parser-owned state for the currently active synchronization scan.
///
/// Keeping delimiter depth here makes recovery ownership inspectable and keeps
/// the state resumable if the token source becomes asynchronous in a later
/// phase. `active` is `None` whenever normal frame execution is in progress.
/// A finished scan keeps its emptied stacks in `spare` for the next one, so
/// recovery reuses the same parse-arena storage however often it runs.
///
/// C99: §5.1.1.3, p. 11; PDF p. 23 requires diagnostics but leaves recovery
/// strategy to the implementation.
pub(super) struct RecoveryState<'p> {
    pub(super) active: Option<ActiveRecovery<'p>>,
    spare:             Option<ActiveRecovery<'p>>,
    arena:             &'p Bump,
}

/// Delimiter depth at which a conditional question mark was consumed, so a
/// scan does not mistake the `:` of `? :` (§6.5.15 paragraph 1, p. 90;
/// PDF p. 102) for a label or `case` colon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct DelimiterDepth {
    pub(super) parentheses: usize,
    pub(super) brackets:    usize,
    pub(super) braces:      usize,
}

/// Delimiter depth and policy for one active recovery scan.
///
/// C99: delimiter ownership follows the productions of §6.5-§6.9,
/// pp. 67-144; PDF pp. 79-156. Depth tracking is an implementation mechanism.
#[derive(Debug)]
pub(super) struct ActiveRecovery<'p> {
    pub(super) set: SynchronizationSet,
    pub(super) parentheses: usize,
    pub(super) brackets: usize,
    pub(super) braces: usize,
    pub(super) questions: ArenaVec<'p, DelimiterDepth>,
    pub(super) last_token: Option<TokenType>,
    parenthesized_type_names: ArenaVec<'p, bool>,
    parenthesized_sizeof_type_names: ArenaVec<'p, bool>,
    pub(super) last_closed_parenthesis_was_type_name: bool,
    pub(super) last_closed_parenthesis_was_sizeof_type_name: bool,
}

impl DelimiterDepth {
    /// Whether no delimiter is open.
    pub(super) fn is_top_level(self) -> bool {
        self.parentheses == 0 && self.brackets == 0 && self.braces == 0
    }
}

impl<'p> RecoveryState<'p> {
    /// Starts a scan owned by `set.target` with balanced delimiter depth.
    pub(super) fn begin(&mut self, set: SynchronizationSet) {
        debug_assert!(self.active.is_none(), "recovery scans cannot nest");
        let (mut questions, mut parenthesized_type_names, mut parenthesized_sizeof_type_names) =
            self.spare.take().map_or_else(
                || {
                    (
                        ArenaVec::new_in(self.arena),
                        ArenaVec::new_in(self.arena),
                        ArenaVec::new_in(self.arena),
                    )
                },
                |spare| {
                    (
                        spare.questions,
                        spare.parenthesized_type_names,
                        spare.parenthesized_sizeof_type_names,
                    )
                },
            );
        questions.clear();
        parenthesized_type_names.clear();
        parenthesized_sizeof_type_names.clear();
        self.active = Some(ActiveRecovery {
            set,
            parentheses: 0,
            brackets: 0,
            braces: 0,
            questions,
            last_token: None,
            parenthesized_type_names,
            parenthesized_sizeof_type_names,
            last_closed_parenthesis_was_type_name: false,
            last_closed_parenthesis_was_sizeof_type_name: false,
        });
    }

    /// Returns the active scan; callers use this to decide whether to stop.
    pub(super) fn active(&self) -> &ActiveRecovery<'p> {
        self.active.as_ref().expect("a recovery scan is active")
    }

    /// Records one consumed token for balanced recovery.
    pub(super) fn consume(&mut self, token: TokenType, opens_type_name: bool) {
        let state = self.active.as_mut().expect("a recovery scan is active");
        state.last_closed_parenthesis_was_type_name = false;
        state.last_closed_parenthesis_was_sizeof_type_name = false;
        match token {
            | TokenType::Operator(OperatorTokenType::OpeningParenthesis) => {
                state.parentheses += 1;
                state.parenthesized_type_names.push(opens_type_name);
                state.parenthesized_sizeof_type_names.push(
                    opens_type_name
                        && matches!(
                            state.last_token,
                            Some(TokenType::Keyword(KeywordTokenType::Sizeof))
                        ),
                );
            },
            | TokenType::Operator(OperatorTokenType::ClosingParenthesis) if state.parentheses > 0 =>
            {
                state.parentheses -= 1;
                let closed_type_name = state.parenthesized_type_names.pop().unwrap_or(false);
                let closed_sizeof_type_name =
                    state.parenthesized_sizeof_type_names.pop().unwrap_or(false);
                state.last_closed_parenthesis_was_type_name = closed_type_name;
                state.last_closed_parenthesis_was_sizeof_type_name = closed_sizeof_type_name;
                Self::discard_closed_questions(state);
            },
            | TokenType::Operator(OperatorTokenType::OpeningSquareBracket) => {
                state.brackets += 1;
            },
            | TokenType::Operator(OperatorTokenType::ClosingSquareBracket) if state.brackets > 0 =>
            {
                state.brackets -= 1;
                Self::discard_closed_questions(state);
            },
            | TokenType::Operator(OperatorTokenType::OpeningCurlyBrace) => {
                state.braces += 1;
            },
            | TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) if state.braces > 0 => {
                state.braces -= 1;
                Self::discard_closed_questions(state);
            },
            | TokenType::Operator(OperatorTokenType::QuestionMark) => {
                state.questions.push(DelimiterDepth {
                    parentheses: state.parentheses,
                    brackets:    state.brackets,
                    braces:      state.braces,
                });
            },
            | TokenType::Operator(OperatorTokenType::Colon) => {
                let depth = DelimiterDepth {
                    parentheses: state.parentheses,
                    brackets:    state.brackets,
                    braces:      state.braces,
                };
                if let Some(index) = state
                    .questions
                    .iter()
                    .rposition(|question| *question == depth)
                {
                    _ = state.questions.remove(index);
                }
            },
            | _ => {},
        }
        state.last_token = Some(token);
    }

    fn discard_closed_questions(state: &mut ActiveRecovery<'_>) {
        let depth = DelimiterDepth {
            parentheses: state.parentheses,
            brackets:    state.brackets,
            braces:      state.braces,
        };
        state.questions.retain(|question| {
            question.parentheses <= depth.parentheses
                && question.brackets <= depth.brackets
                && question.braces <= depth.braces
        });
    }

    /// Completes the active scan and restores normal parser execution.
    pub(super) fn finish(&mut self) {
        self.spare = Some(self.active.take().expect("a recovery scan is active"));
    }

    /// Abandons any active scan, as when parsing stops at a resource limit.
    pub(super) fn abandon(&mut self) {
        if let Some(scan) = self.active.take() {
            self.spare = Some(scan);
        }
    }

    /// No active scan; scan stacks will come from `arena`.
    pub(super) fn new_in(arena: &'p Bump) -> Self {
        Self {
            active: None,
            spare: None,
            arena,
        }
    }
}

impl SynchronizationKind {
    /// Returns whether a balanced recovery scan must stop before `token`.
    pub(super) fn stops_before(self, token: TokenType) -> bool {
        match self {
            | Self::Declaration => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma
                        | OperatorTokenType::Semicolon
                        | OperatorTokenType::ClosingCurlyBrace
                )
            ),
            | Self::BlockDeclaration =>
                is_statement_keyword(token)
                    || matches!(
                        token,
                        TokenType::Operator(
                            OperatorTokenType::Comma
                                | OperatorTokenType::Semicolon
                                | OperatorTokenType::ClosingCurlyBrace
                        )
                    ),
            | Self::OldStyleParameter => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma
                        | OperatorTokenType::Semicolon
                        | OperatorTokenType::OpeningCurlyBrace
                        | OperatorTokenType::ClosingCurlyBrace
                )
            ),
            | Self::ForInitializer => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma
                        | OperatorTokenType::Semicolon
                        | OperatorTokenType::ClosingParenthesis
                        | OperatorTokenType::ClosingCurlyBrace
                )
            ),
            | Self::ArrayBound => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma
                        | OperatorTokenType::ClosingSquareBracket
                        | OperatorTokenType::Semicolon
                )
            ),
            | Self::Parameter | Self::KAndRParameter => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma
                        | OperatorTokenType::ClosingParenthesis
                        | OperatorTokenType::ClosingCurlyBrace
                        | OperatorTokenType::Semicolon
                )
            ),
            | Self::VariadicParameterList | Self::VariadicTrailingParameter => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::ClosingParenthesis
                        | OperatorTokenType::ClosingCurlyBrace
                        | OperatorTokenType::Semicolon
                )
            ),
            | Self::StructMember => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma
                        | OperatorTokenType::Semicolon
                        | OperatorTokenType::ClosingParenthesis
                        | OperatorTokenType::ClosingCurlyBrace
                )
            ),
            | Self::EnumeratorValue => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma
                        | OperatorTokenType::Semicolon
                        | OperatorTokenType::ClosingParenthesis
                        | OperatorTokenType::ClosingCurlyBrace
                )
            ),
            | Self::StatementExpression(terminator) =>
                is_statement_keyword(token)
                    || match terminator {
                        | ExpressionTerminator::Semicolon => matches!(
                            token,
                            TokenType::Operator(
                                OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace
                            )
                        ),
                        | ExpressionTerminator::ForSemicolon => matches!(
                            token,
                            TokenType::Operator(
                                OperatorTokenType::Semicolon
                                    | OperatorTokenType::ClosingParenthesis
                                    | OperatorTokenType::ClosingCurlyBrace
                            )
                        ),
                        | ExpressionTerminator::ClosingParenthesis => matches!(
                            token,
                            TokenType::Operator(
                                OperatorTokenType::ClosingParenthesis
                                    | OperatorTokenType::Semicolon
                                    | OperatorTokenType::ClosingCurlyBrace
                            )
                        ),
                        | ExpressionTerminator::Colon => matches!(
                            token,
                            TokenType::Operator(
                                OperatorTokenType::Colon
                                    | OperatorTokenType::Semicolon
                                    | OperatorTokenType::ClosingCurlyBrace
                            )
                        ),
                    },
            | Self::Statement => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace
                )
            ),
        }
    }

    /// Returns whether an enclosing delimiter is unambiguously owned by the
    /// caller even when malformed child delimiters remain unbalanced.
    pub(super) fn stops_before_despite_unbalanced_child(
        self,
        token: TokenType,
        parentheses: usize,
        brackets: usize,
        braces: usize,
    ) -> bool {
        match self {
            | Self::Declaration =>
                braces == 0
                    && matches!(
                        token,
                        TokenType::Operator(
                            OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace
                        )
                    ),
            | Self::BlockDeclaration =>
                braces == 0
                    && matches!(
                        token,
                        TokenType::Operator(
                            OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace
                        )
                    ),
            | Self::OldStyleParameter =>
                braces == 0
                    && matches!(
                        token,
                        TokenType::Operator(
                            OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace
                        )
                    )
                    || parentheses == 0
                        && brackets == 0
                        && braces == 0
                        && matches!(
                            token,
                            TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
                        ),
            | Self::ForInitializer =>
                braces == 0
                    && matches!(
                        token,
                        TokenType::Operator(
                            OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace
                        )
                    )
                    || parentheses == 0
                        && matches!(
                            token,
                            TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                        ),
            | Self::EnumeratorValue =>
                braces == 0
                    && matches!(
                        token,
                        TokenType::Operator(
                            OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace
                        )
                    )
                    || parentheses == 0
                        && matches!(
                            token,
                            TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                        ),
            | Self::StructMember =>
                braces == 0
                    && matches!(
                        token,
                        TokenType::Operator(
                            OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace
                        )
                    )
                    || parentheses == 0
                        && matches!(
                            token,
                            TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                        ),
            | Self::ArrayBound =>
                braces == 0 && matches!(token, TokenType::Operator(OperatorTokenType::Semicolon))
                    || brackets == 0
                        && matches!(
                            token,
                            TokenType::Operator(OperatorTokenType::ClosingSquareBracket)
                        )
                    || parentheses == 0
                        && matches!(
                            token,
                            TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                        )
                    || braces == 0
                        && matches!(
                            token,
                            TokenType::Operator(OperatorTokenType::ClosingCurlyBrace)
                        ),
            | Self::Parameter
            | Self::KAndRParameter
            | Self::VariadicParameterList
            | Self::VariadicTrailingParameter =>
                braces == 0 && matches!(token, TokenType::Operator(OperatorTokenType::Semicolon))
                    || parentheses == 0
                        && matches!(
                            token,
                            TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                        )
                    || braces == 0
                        && matches!(
                            token,
                            TokenType::Operator(OperatorTokenType::ClosingCurlyBrace)
                        ),
            | Self::StatementExpression(terminator) => match terminator {
                | ExpressionTerminator::Semicolon =>
                    braces == 0
                        && matches!(
                            token,
                            TokenType::Operator(
                                OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace
                            )
                        ),
                | ExpressionTerminator::ForSemicolon =>
                    braces == 0
                        && matches!(
                            token,
                            TokenType::Operator(
                                OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace
                            )
                        )
                        || parentheses == 0
                            && matches!(
                                token,
                                TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                            ),
                | ExpressionTerminator::ClosingParenthesis =>
                    parentheses == 0
                        && braces == 0
                        && matches!(
                            token,
                            TokenType::Operator(
                                OperatorTokenType::ClosingParenthesis
                                    | OperatorTokenType::Semicolon
                                    | OperatorTokenType::ClosingCurlyBrace
                            )
                        ),
                | ExpressionTerminator::Colon =>
                    parentheses == 0
                        && brackets == 0
                        && braces == 0
                        && matches!(
                            token,
                            TokenType::Operator(
                                OperatorTokenType::Colon
                                    | OperatorTokenType::Semicolon
                                    | OperatorTokenType::ClosingCurlyBrace
                            )
                        ),
            },
            | Self::Statement =>
                braces == 0
                    && matches!(
                        token,
                        TokenType::Operator(
                            OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace
                        )
                    ),
        }
    }
}
