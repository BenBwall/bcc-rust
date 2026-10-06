//! Initializer and designation frame.
//!
//! Translation phase 7 syntax analysis (§5.1.1.2, p. 10; PDF p. 22) of
//! `initializer`, `initializer-list`, `designation`, `designator-list`, and
//! `designator` (C99: §6.7.8 paragraph 1, p. 125; PDF p. 137; §A.2.2,
//! pp. 414-415; PDF pp. 426-427), for init-declarators and for the braced
//! list of a compound literal (§6.5.2.5 paragraph 4, p. 75; PDF p. 87).
//!
//! Diagnosed here: syntax the grammar rejects, including an empty `{}`,
//! which C99 does not allow. Left to semantic analysis: the constraints of
//! §6.7.8 paragraphs 2-7, p. 125; PDF p. 137, and the current-object,
//! ordering, and implicit-initialization semantics of paragraphs 8-23,
//! pp. 126-128; PDF pp. 138-140. Elements and nested lists accumulate in
//! arena storage and the frame stack, so no fixed nesting ceiling applies.

use std::fmt::Debug;

use super::{
    Parser,
    declaration_syntax::{
        BracedInitializerList,
        Designation,
        Designator,
        DesignatorType,
        Initializer,
        InitializerElement,
        InitializerType,
    },
    errors::ParserErrorType,
    expression::{
        ExpressionBoundary,
        ExpressionFrame,
        ExpressionMode,
    },
    expression_operators::{
        binary_operator,
        is_postfix_starter,
    },
    frame_pool::{
        FramePools,
        PoolBox,
    },
    machine::{
        ConstantExpressionResult,
        InitializerResult,
        ParseAction,
        ParseFrame,
        ParseValue,
        expression_value,
    },
    recovery::DelimiterDepth,
    statement::is_statement_keyword,
    syntax::{
        ConstantExpression,
        ExpressionType,
        Identifier,
    },
};
use crate::{
    translation_phases::{
        Context,
        SourceVectors,
        preprocessing::{
            OperatorTokenType,
            Token,
            TokenType,
        },
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// Parses one `initializer`: an `assignment-expression` or a braced
/// `initializer-list` with an optional trailing comma.
///
/// C99: §6.7.8 paragraph 1, p. 125; PDF p. 137.
#[derive(Debug)]
pub(super) struct InitializerFrame<'tu, 'p> {
    phase: InitializerPhase<'tu>,
    pub(super) elements: ArenaVec<'p, InitializerElement<'tu>>,
    pub(super) source_vectors: ArenaVec<'p, SourceVectors>,
    opening_brace_source_vectors: Option<SourceVectors>,
    closing_brace_source_vectors: Option<SourceVectors>,
    /// Designation under construction, boxed because scalar initializers and
    /// undesignated elements never use it. The pools lend a box when the
    /// frame is pushed and take it back, reset, when the frame pops.
    pub(super) designation: Option<PoolBox<'p, DesignationState<'tu, 'p>>>,
    starting_error_count: usize,
    /// Whether `)` belongs to an enclosing expression or `for` header.
    closing_parenthesis_is_caller_boundary: bool,
    /// Whether `]` belongs to an enclosing expression.
    closing_square_bracket_is_caller_boundary: bool,
}

/// Designators and provenance of the designation preceding one initializer
/// element.
///
/// C99: `designation` and `designator-list`, §6.7.8 paragraph 1, p. 125;
/// PDF p. 137.
#[derive(Debug)]
pub(super) struct DesignationState<'tu, 'p> {
    pub(super) current_designators:    ArenaVec<'p, Designator<'tu>>,
    current_designation:               Option<&'tu Designation<'tu>>,
    designation_source_vectors:        Option<SourceVectors>,
    designation_equals_source_vectors: Option<SourceVectors>,
    current_designator_source:         Option<SourceVectors>,
    current_designation_recovered:     bool,
    synchronized_designator:           Option<SynchronizedDesignator<'tu>>,
}

/// Recovery state for an array designator whose `]` was not where expected.
///
/// C99: the designator form is `[ constant-expression ]`, §6.7.8
/// paragraph 1, p. 125; PDF p. 137.
#[derive(Debug, Clone, Copy)]
pub(super) struct SynchronizedDesignator<'tu> {
    expression:              ConstantExpression<'tu>,
    source_vectors:          SourceVectors,
    depth:                   DelimiterDepth,
    /// Whether lookahead found this designator's `]` later, so a top-level
    /// `,` or `=` before it is an invalid operator inside the brackets
    /// rather than the point where the `]` went missing.
    closing_bracket_follows: bool,
}

impl<'p> DesignationState<'_, 'p> {
    pub(super) fn new_in(arena: &'p Bump) -> Self {
        Self {
            current_designators:               ArenaVec::new_in(arena),
            current_designation:               None,
            designation_source_vectors:        None,
            designation_equals_source_vectors: None,
            current_designator_source:         None,
            current_designation_recovered:     false,
            synchronized_designator:           None,
        }
    }

    /// Clears the state for the next element while keeping list capacity.
    fn reset(&mut self) {
        self.current_designators.clear();
        self.current_designation = None;
        self.designation_source_vectors = None;
        self.designation_equals_source_vectors = None;
        self.current_designator_source = None;
        self.current_designation_recovered = false;
    }
}

/// State transitions for [`InitializerFrame`].
///
/// C99: §6.7.8 paragraph 1, p. 125; PDF p. 137.
#[derive(Debug, Clone, Copy)]
pub(super) enum InitializerPhase<'tu> {
    Start,
    PushScalar,
    AwaitScalar,
    ElementOrClose,
    Designation,
    FieldDesignator,
    PushArrayDesignator,
    AwaitArrayDesignator,
    CloseArrayDesignator(ConstantExpression<'tu>, bool),
    SynchronizeArrayDesignator,
    DesignationEquals,
    PushElement,
    AwaitElement,
    /// Skip a malformed element that a declaration or statement keyword
    /// starts while this list's own `}` still follows: the delimiter nesting
    /// inside the skipped tokens and their provenance.
    SkipMalformedElement(u32, Option<SourceVectors>),
    Separator,
    FinishList,
}

/// How a token that cannot continue an initializer list relates to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListBoundary {
    /// The token belongs to the list.
    None,
    /// The token belongs to the caller or to a following declaration or
    /// statement, so the list's `}` is missing.
    MissingClose,
    /// The token looks like a declaration or statement, but this list's
    /// `}` follows before any `;`, so it starts one malformed element.
    MalformedElement,
}

impl<'tu, 'p> InitializerFrame<'tu, 'p> {
    pub(super) fn new(
        arena: &'p Bump,
        starting_error_count: usize,
        closing_parenthesis_is_caller_boundary: bool,
        closing_square_bracket_is_caller_boundary: bool,
    ) -> Self {
        Self {
            phase: InitializerPhase::Start,
            elements: ArenaVec::new_in(arena),
            source_vectors: ArenaVec::new_in(arena),
            opening_brace_source_vectors: None,
            closing_brace_source_vectors: None,
            designation: None,
            starting_error_count,
            closing_parenthesis_is_caller_boundary,
            closing_square_bracket_is_caller_boundary,
        }
    }

    #[expect(
        clippy::missing_assert_message,
        reason = "Frame-state debug assertions are local transition invariants."
    )]
    pub(super) fn step(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
        returned: Option<ParseValue<'tu>>,
    ) -> ParseAction<'tu, 'p> {
        match self.phase {
            | InitializerPhase::Start => {
                debug_assert!(returned.is_none());
                if let Some(opening) = token
                    && opening.kind == TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
                {
                    self.opening_brace_source_vectors = Some(opening.source_vectors);
                    self.source_vectors.push((opening).source_vectors);
                    self.phase = InitializerPhase::ElementOrClose;
                    ParseAction::Consume
                } else {
                    self.phase = InitializerPhase::PushScalar;
                    ParseAction::Continue
                }
            },
            | InitializerPhase::PushScalar => {
                debug_assert!(returned.is_none());
                self.phase = InitializerPhase::AwaitScalar;
                ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                    parser.arena,
                    ExpressionMode::AssignmentExpression,
                    ExpressionBoundary::Initializer,
                    parser.hard_error_count,
                )))
            },
            | InitializerPhase::AwaitScalar => {
                let expression = expression_value(returned);
                let source_vectors = expression.source_vectors;
                let index = self.store_initializer(
                    parser,
                    InitializerType::AssignmentExpression(expression),
                    source_vectors,
                );
                ParseAction::Reduce(ParseValue::Initializer(InitializerResult {
                    initializer: index,
                    recovered:   parser.hard_error_count > self.starting_error_count,
                }))
            },
            | InitializerPhase::ElementOrClose => {
                debug_assert!(returned.is_none());
                if let Some(close) = token
                    && close.kind == TokenType::Operator(OperatorTokenType::ClosingCurlyBrace)
                {
                    // C99 §6.7.8p1: an initializer-list has at least one
                    // initializer; `{}` is not C99.
                    if self.elements.is_empty() {
                        parser.report(
                            ParserErrorType::ExpectedStatementExpression(
                                "nonempty initializer list",
                                Some(close.kind),
                            ),
                            Some(close),
                        );
                    }
                    self.closing_brace_source_vectors = Some(close.source_vectors);
                    self.source_vectors.push((close).source_vectors);
                    self.phase = InitializerPhase::FinishList;
                    return ParseAction::Consume;
                }
                if self.is_unowned_closing_delimiter(token) {
                    parser.report(
                        ParserErrorType::ExpectedStatementExpression(
                            "initializer element",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.source_vectors
                        .push((token.expect("an unowned closing delimiter exists")).source_vectors);
                    return ParseAction::Consume;
                }
                match self.list_boundary(parser, token) {
                    | ListBoundary::None => {},
                    | ListBoundary::MissingClose => {
                        parser.report(
                            ParserErrorType::ExpectedClosingCurlyBraceInInitializerList(
                                token.map(|token| token.kind),
                            ),
                            token,
                        );
                        self.phase = InitializerPhase::FinishList;
                        return ParseAction::Reprocess;
                    },
                    | ListBoundary::MalformedElement => {
                        // C99 §6.7.8p1: an initializer is an
                        // assignment-expression or a braced list.
                        parser.report(
                            ParserErrorType::ExpectedStatementExpression(
                                "initializer element",
                                token.map(|token| token.kind),
                            ),
                            token,
                        );
                        self.phase = InitializerPhase::SkipMalformedElement(0, None);
                        return ParseAction::Reprocess;
                    },
                }
                if let Some(designation) = &mut self.designation {
                    designation.reset();
                }
                self.phase = InitializerPhase::Designation;
                ParseAction::Continue
            },
            | InitializerPhase::Designation => {
                debug_assert!(returned.is_none());
                if let Some(designator) = token
                    && designator.kind == TokenType::Operator(OperatorTokenType::Period)
                {
                    self.merge_designation_source(parser.context, designator.source_vectors);
                    self.designation_state().current_designator_source =
                        Some(designator.source_vectors);
                    self.phase = InitializerPhase::FieldDesignator;
                    return ParseAction::Consume;
                }
                if let Some(designator) = token
                    && designator.kind
                        == TokenType::Operator(OperatorTokenType::OpeningSquareBracket)
                {
                    self.merge_designation_source(parser.context, designator.source_vectors);
                    self.designation_state().current_designator_source =
                        Some(designator.source_vectors);
                    self.phase = InitializerPhase::PushArrayDesignator;
                    return ParseAction::Consume;
                }
                self.phase = if self
                    .designation
                    .as_ref()
                    .is_none_or(|designation| designation.current_designators.is_empty())
                {
                    InitializerPhase::PushElement
                } else {
                    InitializerPhase::DesignationEquals
                };
                ParseAction::Reprocess
            },
            | InitializerPhase::FieldDesignator => {
                debug_assert!(returned.is_none());
                let Some(identifier) = token.filter(|token| token.kind == TokenType::Identifier)
                else {
                    // C99 §6.7.8p1: designator `. identifier`.
                    parser.report(
                        ParserErrorType::ExpectedMemberIdentifier(token.map(|token| token.kind)),
                        token,
                    );
                    let operator_source_vectors = self
                        .designation_state()
                        .current_designator_source
                        .take()
                        .unwrap_or_default();
                    self.designation_state()
                        .current_designators
                        .push(Designator {
                            kind: DesignatorType::Error,
                            operator_source_vectors,
                            closing_bracket_source_vectors: None,
                            source_vectors: operator_source_vectors,
                            recovered: true,
                        });
                    self.designation_state().current_designation_recovered = true;
                    self.phase = InitializerPhase::Designation;
                    return ParseAction::Reprocess;
                };
                self.merge_designation_source(parser.context, identifier.source_vectors);
                let operator_source_vectors = self
                    .designation_state()
                    .current_designator_source
                    .take()
                    .unwrap_or_default();
                let source_vectors = parser
                    .context
                    .merge_vectors(operator_source_vectors, identifier.source_vectors);
                self.designation_state()
                    .current_designators
                    .push(Designator {
                        kind: DesignatorType::Field(Identifier::from_token(identifier)),
                        operator_source_vectors,
                        closing_bracket_source_vectors: None,
                        source_vectors,
                        recovered: false,
                    });
                self.phase = InitializerPhase::Designation;
                ParseAction::Consume
            },
            | InitializerPhase::PushArrayDesignator => {
                debug_assert!(returned.is_none());
                self.phase = InitializerPhase::AwaitArrayDesignator;
                ParseAction::Push(ParseFrame::Expression(
                    ExpressionFrame::with_recovery_boundary(
                        parser.arena,
                        ExpressionMode::ConstantExpression,
                        ExpressionBoundary::Designator,
                        ExpressionBoundary::Initializer,
                        parser.hard_error_count,
                    ),
                ))
            },
            | InitializerPhase::AwaitArrayDesignator => {
                let Some(ParseValue::ConstantExpression(ConstantExpressionResult {
                    expression: index,
                    recovered,
                })) = returned
                else {
                    panic!("array designator returned an unexpected value: {returned:?}");
                };
                self.phase = InitializerPhase::CloseArrayDesignator(index, recovered);
                ParseAction::Reprocess
            },
            | InitializerPhase::CloseArrayDesignator(expression, expression_recovered) => {
                debug_assert!(returned.is_none());
                let expression_source = expression.expression().source_vectors;
                let operator_source_vectors = self
                    .designation_state()
                    .current_designator_source
                    .unwrap_or_default();
                let mut source_vectors = parser
                    .context
                    .merge_vectors(operator_source_vectors, expression_source);
                self.merge_designation_source(parser.context, expression_source);
                if let Some(close) = token
                    && close.kind == TokenType::Operator(OperatorTokenType::ClosingSquareBracket)
                {
                    source_vectors = parser
                        .context
                        .merge_vectors(source_vectors, close.source_vectors);
                    self.merge_designation_source(parser.context, close.source_vectors);
                    self.push_array_designator(
                        expression,
                        source_vectors,
                        Some(close.source_vectors),
                        expression_recovered,
                    );
                    self.phase = InitializerPhase::Designation;
                    ParseAction::Consume
                } else {
                    parser.report(
                        ParserErrorType::ExpectedClosingSquareBracketInArrayDesignator(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    let closing_bracket_follows = Self::closing_bracket_follows(parser, token);
                    self.designation_state().synchronized_designator =
                        Some(SynchronizedDesignator {
                            expression,
                            source_vectors,
                            depth: DelimiterDepth {
                                parentheses: 0,
                                brackets:    0,
                                braces:      0,
                            },
                            closing_bracket_follows,
                        });
                    self.phase = InitializerPhase::SynchronizeArrayDesignator;
                    ParseAction::Reprocess
                }
            },
            | InitializerPhase::SynchronizeArrayDesignator => {
                debug_assert!(returned.is_none());
                let SynchronizedDesignator {
                    expression,
                    mut source_vectors,
                    mut depth,
                    closing_bracket_follows,
                } = self
                    .designation_state()
                    .synchronized_designator
                    .take()
                    .expect("array-designator synchronization retains its state");
                if let Some(token) = token
                    && token.kind == TokenType::Operator(OperatorTokenType::ClosingSquareBracket)
                {
                    source_vectors = parser
                        .context
                        .merge_vectors(source_vectors, token.source_vectors);
                    self.merge_designation_source(parser.context, token.source_vectors);
                    if depth.brackets == 0 {
                        self.push_array_designator(
                            expression,
                            source_vectors,
                            Some(token.source_vectors),
                            true,
                        );
                        self.phase = InitializerPhase::Designation;
                    } else {
                        depth.brackets -= 1;
                        self.designation_state().synchronized_designator =
                            Some(SynchronizedDesignator {
                                expression,
                                source_vectors,
                                depth,
                                closing_bracket_follows,
                            });
                        self.phase = InitializerPhase::SynchronizeArrayDesignator;
                    }
                    return ParseAction::Consume;
                }
                // Nested delimiters cannot transfer a comma, `=`, or
                // weak grammar boundary to the
                // surrounding initializer. Owning
                // outer boundaries still stop the scan despite an
                // unmatched sibling delimiter.
                if self.at_array_designator_sync_boundary(
                    parser,
                    token,
                    depth,
                    closing_bracket_follows,
                ) {
                    self.push_array_designator(expression, source_vectors, None, true);
                    self.phase = InitializerPhase::Designation;
                    return ParseAction::Reprocess;
                }
                let Some(token) = token else {
                    self.push_array_designator(expression, source_vectors, None, true);
                    self.phase = InitializerPhase::Designation;
                    return ParseAction::Reprocess;
                };
                match token.kind {
                    | TokenType::Operator(OperatorTokenType::OpeningParenthesis) => {
                        depth.parentheses += 1;
                    },
                    | TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                        if depth.parentheses > 0 =>
                    {
                        depth.parentheses -= 1;
                    },
                    | TokenType::Operator(OperatorTokenType::OpeningSquareBracket) => {
                        depth.brackets += 1;
                    },
                    | TokenType::Operator(OperatorTokenType::OpeningCurlyBrace) => {
                        depth.braces += 1;
                    },
                    | TokenType::Operator(OperatorTokenType::ClosingCurlyBrace)
                        if depth.braces > 0 =>
                    {
                        depth.braces -= 1;
                    },
                    | _ => {},
                }
                source_vectors = parser
                    .context
                    .merge_vectors(source_vectors, token.source_vectors);
                self.merge_designation_source(parser.context, token.source_vectors);
                self.designation_state().synchronized_designator = Some(SynchronizedDesignator {
                    expression,
                    source_vectors,
                    depth,
                    closing_bracket_follows,
                });
                self.phase = InitializerPhase::SynchronizeArrayDesignator;
                ParseAction::Consume
            },
            | InitializerPhase::DesignationEquals => {
                debug_assert!(returned.is_none());
                let consume = if let Some(equals) = token
                    && equals.kind == TokenType::Operator(OperatorTokenType::Equals)
                {
                    // C99 §6.7.8p1: designation is `designator-list =`.
                    self.designation_state().designation_equals_source_vectors =
                        Some(equals.source_vectors);
                    self.merge_designation_source(parser.context, equals.source_vectors);
                    true
                } else {
                    parser.report(
                        ParserErrorType::ExpectedEqualsAfterInitializerDesignation(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.designation_state().current_designation_recovered = true;
                    false
                };
                self.finish_designation(parser);
                self.phase = InitializerPhase::PushElement;
                if consume {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | InitializerPhase::PushElement => {
                debug_assert!(returned.is_none());
                self.phase = InitializerPhase::AwaitElement;
                ParseAction::Push(self.element_frame(parser))
            },
            | InitializerPhase::AwaitElement => {
                let Some(ParseValue::Initializer(InitializerResult {
                    initializer: index, ..
                })) = returned
                else {
                    panic!("initializer element returned an unexpected value: {returned:?}");
                };
                self.push_element(parser.context, index);
                self.phase = InitializerPhase::Separator;
                ParseAction::Continue
            },
            | InitializerPhase::SkipMalformedElement(mut nesting, mut source) => {
                debug_assert!(returned.is_none());
                let stops = token.is_none_or(|token| match token.kind {
                    | TokenType::Operator(OperatorTokenType::Semicolon) => true,
                    | TokenType::Operator(
                        OperatorTokenType::Comma | OperatorTokenType::ClosingCurlyBrace,
                    ) => nesting == 0,
                    | TokenType::Operator(
                        OperatorTokenType::ClosingParenthesis
                        | OperatorTokenType::ClosingSquareBracket,
                    ) => nesting == 0 && self.at_caller_boundary(Some(token)),
                    | _ => false,
                });
                if stops {
                    let source_vectors = source.unwrap_or_else(|| parser.missing_syntax_source());
                    let expression =
                        parser.store_expression(ExpressionType::Error, source_vectors, None, true);
                    let index = parser.alloc_syntax(Initializer {
                        kind: InitializerType::AssignmentExpression(expression),
                        source_vectors,
                        recovered: true,
                    });
                    self.push_element(parser.context, index);
                    self.phase = InitializerPhase::Separator;
                    return ParseAction::Reprocess;
                }
                let token = token.expect("a skipped token exists");
                match token.kind {
                    | TokenType::Operator(
                        OperatorTokenType::OpeningParenthesis
                        | OperatorTokenType::OpeningSquareBracket
                        | OperatorTokenType::OpeningCurlyBrace,
                    ) => nesting = nesting.saturating_add(1),
                    | TokenType::Operator(
                        OperatorTokenType::ClosingParenthesis
                        | OperatorTokenType::ClosingSquareBracket
                        | OperatorTokenType::ClosingCurlyBrace,
                    ) => nesting = nesting.saturating_sub(1),
                    | _ => {},
                }
                source = Some(source.map_or(token.source_vectors, |existing| {
                    parser.context.merge_vectors(existing, token.source_vectors)
                }));
                self.phase = InitializerPhase::SkipMalformedElement(nesting, source);
                ParseAction::Consume
            },
            | InitializerPhase::Separator => {
                debug_assert!(returned.is_none());
                // C99 §6.7.8p1: `,` continues the list, and a `,` before `}`
                // is the trailing-comma form.
                if let Some(comma) = token
                    && comma.kind == TokenType::Operator(OperatorTokenType::Comma)
                {
                    self.elements
                        .last_mut()
                        .expect("a separator follows one initializer element")
                        .comma_source_vectors = Some(comma.source_vectors);
                    self.source_vectors.push((comma).source_vectors);
                    self.phase = InitializerPhase::ElementOrClose;
                    return ParseAction::Consume;
                }
                if let Some(close) = token
                    && close.kind == TokenType::Operator(OperatorTokenType::ClosingCurlyBrace)
                {
                    self.closing_brace_source_vectors = Some(close.source_vectors);
                    self.source_vectors.push((close).source_vectors);
                    self.phase = InitializerPhase::FinishList;
                    return ParseAction::Consume;
                }
                if self.is_unowned_closing_delimiter(token) {
                    parser.report(
                        ParserErrorType::ExpectedClosingCurlyBraceInInitializerList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.source_vectors
                        .push((token.expect("an unowned closing delimiter exists")).source_vectors);
                    // The element before the stray closer is complete, so a
                    // following `,` or `}` is still this list's separator.
                    return ParseAction::Consume;
                }
                let boundary = self.list_boundary(parser, token);
                parser.report(
                    ParserErrorType::ExpectedClosingCurlyBraceInInitializerList(
                        token.map(|token| token.kind),
                    ),
                    token,
                );
                self.phase = match boundary {
                    | ListBoundary::MissingClose => InitializerPhase::FinishList,
                    // The `,` is missing before a malformed element; skip it
                    // as one element so it is not diagnosed again.
                    | ListBoundary::MalformedElement =>
                        InitializerPhase::SkipMalformedElement(0, None),
                    | ListBoundary::None => InitializerPhase::ElementOrClose,
                };
                ParseAction::Reprocess
            },
            | InitializerPhase::FinishList => {
                debug_assert!(returned.is_none());
                let list = self.store_braced_list(parser);
                let source_vectors = parser.context.merge_vector_list(&self.source_vectors);
                let index = self.store_initializer(
                    parser,
                    InitializerType::InitializerList(list),
                    source_vectors,
                );
                ParseAction::Reduce(ParseValue::Initializer(InitializerResult {
                    initializer: index,
                    recovered:   parser.hard_error_count > self.starting_error_count,
                }))
            },
        }
    }

    /// Appends one element, with any designation that preceded it.
    fn push_element(&mut self, context: &mut Context<'_>, index: &'tu Initializer<'tu>) {
        let initializer_source = index.source_vectors;
        let current_designation = self
            .designation
            .as_mut()
            .and_then(|designation| designation.current_designation.take());
        let source_vectors = current_designation.map_or(initializer_source, |designation| {
            context.merge_vectors(designation.source_vectors, initializer_source)
        });
        self.elements.push(InitializerElement {
            designation: current_designation,
            initializer: index,
            comma_source_vectors: None,
            source_vectors,
        });
        self.source_vectors.push(source_vectors);
    }

    /// An unstarted child frame for one element of this list.
    fn element_frame(&self, parser: &Parser<'_, 'tu, 'p>) -> ParseFrame<'tu, 'p> {
        ParseFrame::Initializer(InitializerFrame::new(
            parser.arena,
            parser.hard_error_count,
            self.closing_parenthesis_is_caller_boundary,
            self.closing_square_bracket_is_caller_boundary,
        ))
    }

    fn designation_state(&mut self) -> &mut DesignationState<'tu, 'p> {
        self.designation
            .as_mut()
            .expect("a pushed initializer frame holds a designation state")
    }

    /// Returns this popped frame's lists and designation state to the pools.
    pub(super) fn reclaim_pooled(&mut self, pools: &mut FramePools<'tu, 'p>) {
        pools.initializers.reclaim(&mut self.elements);
        pools.source_vectors.reclaim(&mut self.source_vectors);
        if let Some(mut designation) = self.designation.take() {
            designation.reset();
            designation.synchronized_designator = None;
            pools.designations.reclaim(designation);
        }
    }

    fn merge_designation_source(&mut self, context: &mut Context<'_>, source: SourceVectors) {
        let designation = self.designation_state();
        designation.designation_source_vectors = Some(
            designation
                .designation_source_vectors
                .map_or(source, |existing| context.merge_vectors(existing, source)),
        );
    }

    fn push_array_designator(
        &mut self,
        expression: ConstantExpression<'tu>,
        source_vectors: SourceVectors,
        closing_bracket_source_vectors: Option<SourceVectors>,
        recovered: bool,
    ) {
        let designation = self.designation_state();
        designation.current_designators.push(Designator {
            kind: DesignatorType::Array(expression),
            operator_source_vectors: designation
                .current_designator_source
                .take()
                .unwrap_or_default(),
            closing_bracket_source_vectors,
            source_vectors,
            recovered,
        });
        designation.current_designation_recovered |= recovered;
    }

    fn at_array_designator_sync_boundary(
        &self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
        depth: DelimiterDepth,
        closing_bracket_follows: bool,
    ) -> bool {
        let Some(token) = token else {
            return true;
        };
        let at_top_level = depth.parentheses == 0 && depth.brackets == 0 && depth.braces == 0;
        at_top_level
            && !closing_bracket_follows
            && matches!(
                token.kind,
                TokenType::Operator(OperatorTokenType::Equals | OperatorTokenType::Comma)
            )
            || matches!(
                token.kind,
                TokenType::Operator(
                    OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace
                )
            )
            || self.closing_parenthesis_is_caller_boundary
                && depth.parentheses == 0
                && token.kind == TokenType::Operator(OperatorTokenType::ClosingParenthesis)
            || at_top_level
                && (is_statement_keyword(token.kind)
                    || ExpressionFrame::is_strong_grammar_boundary_for(
                        parser,
                        token,
                        ExpressionBoundary::Initializer,
                    ))
    }

    /// Whether this array designator's `]` follows `token` before the list,
    /// declaration, or an enclosing delimiter ends.
    ///
    /// C99 §6.7.8p1: a designator is `[ constant-expression ]`, so a
    /// top-level `,` or `=` inside one is a single syntax error when a `]`
    /// still closes it, e.g. `[1, 2] = 3`. Without that `]`, as in
    /// `{ [1 = 2 }`, the `,` or `=` is where the bracket went missing. The
    /// scan is bounded so repeated errors in one long list stay linear.
    fn closing_bracket_follows(parser: &mut Parser<'_, 'tu, 'p>, token: Option<Token>) -> bool {
        const SCAN_LIMIT: usize = 32;
        let mut depth = DelimiterDepth {
            parentheses: 0,
            brackets:    0,
            braces:      0,
        };
        let mut next = token;
        for index in 0..SCAN_LIMIT {
            let Some(token) = next else {
                return false;
            };
            let at_top_level = depth.parentheses == 0 && depth.brackets == 0 && depth.braces == 0;
            match token.kind {
                | TokenType::Operator(OperatorTokenType::ClosingSquareBracket) => {
                    if depth.brackets == 0 {
                        return at_top_level;
                    }
                    depth.brackets -= 1;
                },
                | TokenType::Operator(OperatorTokenType::OpeningSquareBracket) => {
                    depth.brackets += 1;
                },
                | TokenType::Operator(OperatorTokenType::OpeningParenthesis) => {
                    depth.parentheses += 1;
                },
                | TokenType::Operator(OperatorTokenType::ClosingParenthesis) => {
                    if depth.parentheses == 0 {
                        return false;
                    }
                    depth.parentheses -= 1;
                },
                | TokenType::Operator(OperatorTokenType::OpeningCurlyBrace) => {
                    depth.braces += 1;
                },
                | TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) => {
                    if depth.braces == 0 {
                        return false;
                    }
                    depth.braces -= 1;
                },
                | TokenType::Operator(OperatorTokenType::Semicolon) => return false,
                | kind if at_top_level
                    && (is_statement_keyword(kind)
                        || kind != TokenType::Identifier && parser.declaration_starter(token)) =>
                {
                    return false;
                },
                | _ => {},
            }
            next = parser.cursor.lookahead(index);
        }
        false
    }

    fn finish_designation(&mut self, parser: &mut Parser<'_, 'tu, 'p>) {
        let designation = self.designation_state();
        let recovered = designation.current_designation_recovered
            || designation
                .current_designators
                .iter()
                .any(|designator| designator.recovered);
        let designators = parser.alloc_syntax_list(&mut designation.current_designators);
        let index = parser.alloc_syntax(Designation {
            designators,
            equals_source_vectors: designation.designation_equals_source_vectors.take(),
            source_vectors: designation
                .designation_source_vectors
                .take()
                .unwrap_or_default(),
            recovered,
        });
        designation.current_designation = Some(index);
    }

    fn at_caller_boundary(&self, token: Option<Token>) -> bool {
        token.is_none_or(|token| {
            token.kind == TokenType::Operator(OperatorTokenType::Semicolon)
                || self.closing_parenthesis_is_caller_boundary
                    && token.kind == TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                || self.closing_square_bracket_is_caller_boundary
                    && token.kind == TokenType::Operator(OperatorTokenType::ClosingSquareBracket)
        })
    }

    fn is_unowned_closing_delimiter(&self, token: Option<Token>) -> bool {
        token.is_some_and(|token| {
            !self.closing_parenthesis_is_caller_boundary
                && token.kind == TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                || !self.closing_square_bracket_is_caller_boundary
                    && token.kind == TokenType::Operator(OperatorTokenType::ClosingSquareBracket)
        })
    }

    fn list_boundary(
        &self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
    ) -> ListBoundary {
        if self.at_caller_boundary(token) {
            return ListBoundary::MissingClose;
        }
        let Some(token) = token else {
            return ListBoundary::MissingClose;
        };
        if token.kind == TokenType::Identifier
            && parser.cursor.following().is_some_and(|following| {
                following.kind == TokenType::Operator(OperatorTokenType::Colon)
            })
        {
            return ListBoundary::MissingClose;
        }
        let identifier_continues_initializer = token.kind == TokenType::Identifier
            && parser.cursor.following().is_some_and(|following| {
                binary_operator(following.kind).is_some() || is_postfix_starter(following.kind)
            });
        let starts_declaration_or_statement = is_statement_keyword(token.kind)
            || parser.declaration_recovery_starts_here(token)
                && !identifier_continues_initializer
                && !matches!(
                    parser.cursor.following().map(|following| following.kind),
                    Some(TokenType::Operator(
                        OperatorTokenType::Comma
                            | OperatorTokenType::Semicolon
                            | OperatorTokenType::ClosingCurlyBrace
                    ))
                );
        if !starts_declaration_or_statement {
            ListBoundary::None
        } else if Self::closing_brace_follows(parser) {
            ListBoundary::MalformedElement
        } else {
            ListBoundary::MissingClose
        }
    }

    /// Whether this list's `}` follows the current token before a `;`, a
    /// statement keyword, or an unmatched closer.
    ///
    /// C99 §6.7.8p1: no initializer contains a `;`, so a keyword that would
    /// otherwise start a following declaration or statement is a malformed
    /// element when the list's own `}` comes first, as in `{ 1, int 0 }`.
    /// The scan is bounded so repeated errors in one long list stay linear.
    fn closing_brace_follows(parser: &mut Parser<'_, 'tu, 'p>) -> bool {
        const SCAN_LIMIT: usize = 64;
        let mut nesting = 0_u32;
        for index in 0..SCAN_LIMIT {
            let Some(token) = parser.cursor.lookahead(index) else {
                return false;
            };
            match token.kind {
                | TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) => {
                    if nesting == 0 {
                        return true;
                    }
                    nesting -= 1;
                },
                | TokenType::Operator(
                    OperatorTokenType::OpeningParenthesis
                    | OperatorTokenType::OpeningSquareBracket
                    | OperatorTokenType::OpeningCurlyBrace,
                ) => nesting += 1,
                | TokenType::Operator(
                    OperatorTokenType::ClosingParenthesis | OperatorTokenType::ClosingSquareBracket,
                ) => {
                    if nesting == 0 {
                        return false;
                    }
                    nesting -= 1;
                },
                | TokenType::Operator(OperatorTokenType::Semicolon) => return false,
                | kind if is_statement_keyword(kind) => return false,
                | _ => {},
            }
        }
        false
    }

    /// Stores the finished list's elements and braces, emptying the element
    /// vector for reuse.
    fn store_braced_list(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
    ) -> &'tu BracedInitializerList<'tu> {
        let elements = parser.alloc_syntax_list(&mut self.elements);
        parser.alloc_syntax_part(BracedInitializerList {
            elements,
            opening_brace_source_vectors: self.opening_brace_source_vectors,
            closing_brace_source_vectors: self.closing_brace_source_vectors,
        })
    }

    fn store_initializer(
        &self,
        parser: &mut Parser<'_, 'tu, 'p>,
        kind: InitializerType<'tu>,
        source_vectors: SourceVectors,
    ) -> &'tu Initializer<'tu> {
        parser.alloc_syntax(Initializer {
            kind,
            source_vectors,
            recovered: parser.hard_error_count > self.starting_error_count,
        })
    }
}
