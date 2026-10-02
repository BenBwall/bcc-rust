//! Enum specifier frame.

use std::fmt::Debug;

use super::{
    Parser,
    declaration_syntax::{
        EnumSpecifier,
        Enumerator,
    },
    errors::ParserErrorType,
    expression::{
        ExpressionBoundary,
        ExpressionFrame,
        ExpressionMode,
    },
    expression_operators::is_operator,
    machine::{
        ConstantExpressionResult,
        EnumSpecifierResult,
        ParseAction,
        ParseFrame,
        ParseFrameKind,
        ParseValue,
    },
    recovery::{
        SynchronizationKind,
        SynchronizationSet,
    },
    scope::NameClass,
    syntax::{
        ConstantExpressionIndex,
        EnumSpecifierIndex,
        ExpressionIndex,
        Identifier,
        SyntaxList,
    },
};
use crate::{
    translation_phases::{
        Context,
        SourceVectors,
        preprocessing::{
            KeywordTokenType,
            OperatorTokenType,
            Token,
            TokenType,
        },
    },
    util::vector_slice::UsizeExt,
};

/// Parses an enum specifier, including its optional tag, enumerators, trailing
/// comma, and optional explicit values.
///
/// C99: enumeration specifiers and enumerators are §6.7.2.2,
/// pp. 105-107; PDF pp. 117-119.
#[derive(Debug)]
pub(super) struct EnumSpecifierFrame {
    /// Current tag/enumerator transition.
    phase: EnumPhase,
    /// Optional enum tag.
    name: Option<Identifier>,
    /// Completed enumerators before arena insertion.
    pub(super) enumerators: Vec<Enumerator>,
    /// Enumerator name waiting for an optional explicit value.
    current_enumerator: Option<Identifier>,
    /// Whether `{` was consumed, distinguishing a reference from a definition.
    body_started: bool,
    /// Whether malformed-body recovery stopped before an outer declaration.
    stopped_before_declaration: bool,
    /// Provenance accumulated across the complete enum specifier.
    pub(super) source_vectors: Vec<SourceVectors>,
    /// Provenance for the enumerator currently being built.
    current_enumerator_source: Option<SourceVectors>,
}

/// State transitions for an enum tag and enumerator list.
///
/// C99: §6.7.2.2, pp. 105-107; PDF pp. 117-119.
#[derive(Debug, Clone, Copy)]
pub(super) enum EnumPhase {
    /// Consume the `enum` keyword.
    Start,
    /// Parse an optional tag or anonymous opening brace.
    NameOrBody,
    /// Decide whether a named tag also has a body.
    AfterName,
    /// Parse an enumerator name or the body's closing brace.
    EnumeratorOrClose,
    /// Decide whether `=` introduces an explicit value.
    AfterEnumeratorName,
    /// Push the constant-expression value.
    PushEnumeratorValue,
    /// Receive the recovered enumerator-value placeholder.
    AwaitEnumeratorValue,
    /// Require `,` or `}` after one enumerator.
    AfterEnumerator,
    /// Store the completed body and return the enum-specifier handle.
    FinishBody,
}

impl EnumSpecifierFrame {
    pub(super) fn new() -> Self {
        Self {
            phase: EnumPhase::Start,
            name: None,
            enumerators: Vec::new(),
            current_enumerator: None,
            body_started: false,
            stopped_before_declaration: false,
            source_vectors: Vec::new(),
            current_enumerator_source: None,
        }
    }

    pub(super) fn step(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        token: Option<Token>,
        returned: Option<ParseValue>,
    ) -> ParseAction {
        match self.phase {
            | EnumPhase::Start => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                let Some(token) = token else {
                    parser.report(context, ParserErrorType::ExpectedEnumKeyword(None), None);
                    return self.finish(parser, context);
                };
                if token.kind != TokenType::Keyword(KeywordTokenType::Enum) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedEnumKeyword(Some(token.kind)),
                        Some(token),
                    );
                }
                self.source_vectors.push(token.source_vectors);
                self.phase = EnumPhase::NameOrBody;
                ParseAction::Consume
            },
            | EnumPhase::NameOrBody => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // Like tag specifiers for aggregates, an enum can be a tagged
                // reference, tagged definition, or anonymous definition.
                if let Some(token) = token
                    && token.kind == TokenType::Identifier
                {
                    self.name = Some(Identifier::from_token(token));
                    self.source_vectors.push(token.source_vectors);
                    self.phase = EnumPhase::AfterName;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::OpeningCurlyBrace) {
                    let token = token.expect("opening-curly-brace token exists");
                    self.body_started = true;
                    self.source_vectors.push(token.source_vectors);
                    self.phase = EnumPhase::EnumeratorOrClose;
                    ParseAction::Consume
                } else {
                    parser.report(
                        context,
                        ParserErrorType::EnumSpecifierWithoutNameAndBody(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.finish(parser, context)
                }
            },
            | EnumPhase::AfterName => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                if is_operator(token, OperatorTokenType::OpeningCurlyBrace) {
                    let token = token.expect("opening-curly-brace token exists");
                    self.body_started = true;
                    self.source_vectors.push(token.source_vectors);
                    self.phase = EnumPhase::EnumeratorOrClose;
                    ParseAction::Consume
                } else {
                    self.finish(parser, context)
                }
            },
            | EnumPhase::EnumeratorOrClose => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // This state is also reached after a comma, which is why `}`
                // accepts the standard's optional trailing-comma form.
                if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
                    let token = token.expect("closing-curly-brace token exists");
                    if self.enumerators.is_empty() {
                        parser.report(
                            context,
                            ParserErrorType::ExpectedEnumeratorBeforeClosingCurlyBrace,
                            Some(token),
                        );
                    }
                    self.source_vectors.push(token.source_vectors);
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Consume
                } else if let Some(token) = token
                    && token.kind == TokenType::Identifier
                {
                    self.current_enumerator = Some(Identifier::from_token(token));
                    self.current_enumerator_source = Some(token.source_vectors);
                    self.source_vectors.push(token.source_vectors);
                    self.phase = EnumPhase::AfterEnumeratorName;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Reprocess
                } else if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Reprocess
                } else if token.is_none() {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(
                            None,
                        ),
                        None,
                    );
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Reprocess
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = EnumPhase::AfterEnumerator;
                    ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::EnumeratorValue,
                        target: ParseFrameKind::EnumSpecifier,
                    })
                }
            },
            | EnumPhase::AfterEnumeratorName => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // The constant-expression is optional. Finalize immediately
                // unless `=` explicitly transfers ownership to the value child.
                if is_operator(token, OperatorTokenType::Equals) {
                    let token = token.expect("equals token exists");
                    self.source_vectors.push(token.source_vectors);
                    parser.merge_source(context, &mut self.current_enumerator_source, token);
                    self.phase = EnumPhase::PushEnumeratorValue;
                    ParseAction::Consume
                } else {
                    self.finish_enumerator(parser, None);
                    self.phase = EnumPhase::AfterEnumerator;
                    ParseAction::Reprocess
                }
            },
            | EnumPhase::PushEnumeratorValue => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                self.phase = EnumPhase::AwaitEnumeratorValue;
                ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                    ExpressionMode::ConstantExpression,
                    ExpressionBoundary::Enumerator,
                    parser.hard_error_count,
                )))
            },
            | EnumPhase::AwaitEnumeratorValue => {
                let Some(ParseValue::ConstantExpression(ConstantExpressionResult {
                    index, ..
                })) = returned
                else {
                    panic!("enumerator-value frame returned an unexpected value: {returned:?}");
                };
                let source_vectors = parser.syntax.expressions
                    [ExpressionIndex::from(index).0 as usize]
                    .source_vectors;
                self.source_vectors.push(source_vectors);
                self.current_enumerator_source = Some(
                    self.current_enumerator_source
                        .map_or(source_vectors, |existing| {
                            context.merge_vectors(existing, source_vectors)
                        }),
                );
                self.finish_enumerator(parser, Some(index));
                self.phase = EnumPhase::AfterEnumerator;
                ParseAction::Reprocess
            },
            | EnumPhase::AfterEnumerator => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // A following identifier is a strong omitted-comma recovery
                // point: preserve it and parse it as the next enumerator.
                if is_operator(token, OperatorTokenType::Comma) {
                    let token = token.expect("comma token exists");
                    self.source_vectors.push(token.source_vectors);
                    self.phase = EnumPhase::EnumeratorOrClose;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
                    let token = token.expect("closing-curly-brace token exists");
                    self.source_vectors.push(token.source_vectors);
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Reprocess
                } else if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Reprocess
                } else if token.is_some_and(|token| token.kind == TokenType::Identifier) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = EnumPhase::EnumeratorOrClose;
                    ParseAction::Reprocess
                } else if token.is_some_and(|token| parser.declaration_starter(token)) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.stopped_before_declaration = true;
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Reprocess
                } else if token.is_none() {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(None),
                        None,
                    );
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Reprocess
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::EnumeratorValue,
                        target: ParseFrameKind::EnumSpecifier,
                    })
                }
            },
            | EnumPhase::FinishBody => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                self.finish(parser, context)
            },
        }
    }

    fn finish_enumerator(
        &mut self,
        parser: &mut Parser,
        expression: Option<ConstantExpressionIndex>,
    ) {
        if let Some(name) = self.current_enumerator.take() {
            self.enumerators.push(Enumerator {
                name,
                expression,
                source_vectors: self.current_enumerator_source.take().unwrap_or_default(),
            });
            // Enumeration constants join the ordinary identifier namespace as
            // soon as their enumerator completes, affecting later values.
            parser.scopes.publish(name.name, NameClass::Ordinary);
        }
    }

    fn finish(&mut self, parser: &mut Parser, context: &mut Context) -> ParseAction {
        let enumeration_list = self.body_started.then(|| {
            let start = parser.syntax.enumerators.len().to_u32();
            parser.append_syntax(|syntax| &mut syntax.enumerators, &mut self.enumerators);
            SyntaxList::new(
                parser.syntax_id,
                start,
                parser.syntax.enumerators.len().to_u32(),
            )
        });
        let index = parser.syntax.enum_specifiers.len().to_u32();
        parser.push_syntax(
            |syntax| &mut syntax.enum_specifiers,
            EnumSpecifier {
                name: self.name,
                enumeration_list,
                source_vectors: context.merge_vector_list(&self.source_vectors),
            },
        );
        ParseAction::Reduce(ParseValue::EnumSpecifier(EnumSpecifierResult {
            index: EnumSpecifierIndex(index, parser.syntax_id),
            stopped_before_declaration: self.stopped_before_declaration,
        }))
    }
}
