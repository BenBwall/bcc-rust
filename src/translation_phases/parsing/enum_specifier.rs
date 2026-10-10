//! Enum specifier frame.
//!
//! Translation phase 7 syntax analysis (§5.1.1.2, p. 10; PDF p. 22) of
//! `enum-specifier`, `enumerator-list`, and `enumerator` (C99: §6.7.2.2
//! paragraph 1, p. 105; PDF p. 117; §A.2.2, p. 413; PDF p. 425), with the
//! tag forms of §6.7.2.3, pp. 106-107; PDF pp. 118-119.
//!
//! Each enumeration constant enters the ordinary name space as soon as its
//! enumerator ends (§6.2.1 paragraph 7, p. 30; PDF p. 42; §6.2.3
//! paragraph 1, p. 31; PDF p. 43), so it can hide a typedef name in later
//! tokens. Diagnosed here: a malformed list, including an empty one, which
//! the grammar does not allow. Left to semantic analysis: the value
//! constraint and value assignment of §6.7.2.2 paragraphs 2-3, p. 105;
//! PDF p. 117, the compatible integer type of paragraph 4, p. 105;
//! PDF p. 117 (implementation-defined), and the tag constraints of §6.7.2.3
//! paragraphs 1-3, p. 106; PDF p. 118. Enumerators accumulate in arena
//! storage, so the 1023-constant minimum (§5.2.4.1, p. 21; PDF p. 33)
//! imposes no fixed ceiling.

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
        unexpected_return,
    },
    modern::{
        ModernKind,
        ModernValue,
        SpecifierExtension,
        SpecifierExtensionKind,
    },
    recovery::{
        SynchronizationKind,
        SynchronizationSet,
    },
    scope::NameClass,
    statement::is_statement_keyword,
    syntax::{
        ConstantExpression,
        Identifier,
    },
};
use crate::{
    translation_phases::{
        SourceVectors,
        preprocessing::{
            KeywordTokenType,
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

/// Parses an enum specifier, including its optional tag, enumerators, trailing
/// comma, and optional explicit values.
///
/// C99: enumeration specifiers and enumerators are §6.7.2.2,
/// pp. 105-106; PDF pp. 117-118; tags are §6.7.2.3, pp. 106-107;
/// PDF pp. 118-119.
#[derive(Debug)]
pub(super) struct EnumSpecifierFrame<'tu, 'p> {
    /// Current tag/enumerator transition.
    phase: EnumPhase,
    attribute_resume: EnumPhase,
    attributes: Option<&'tu SpecifierExtension<'tu>>,
    enumerator_attributes: Option<&'tu SpecifierExtension<'tu>>,
    underlying_type: Option<&'tu super::declaration_syntax::TypeName<'tu>>,
    /// Optional enum tag.
    name: Option<Identifier>,
    /// Completed enumerators before arena insertion.
    pub(super) enumerators: ArenaVec<'p, Enumerator<'tu>>,
    /// Enumerator name waiting for an optional explicit value.
    current_enumerator: Option<Identifier>,
    /// Whether `{` was consumed, distinguishing a reference from a definition.
    body_started: bool,
    /// Whether malformed-body recovery stopped before an outer declaration.
    stopped_before_declaration: bool,
    /// Hard-error count when `{` was consumed. A body that already reported
    /// a malformed enumerator does not also report that the list is empty.
    body_starting_error_count: usize,
    /// Provenance accumulated across the complete enum specifier.
    pub(super) source_vectors: ArenaVec<'p, SourceVectors>,
    /// Provenance for the enumerator currently being built.
    current_enumerator_source: Option<SourceVectors>,
    /// Whether the enumerator before the current separator position already
    /// reported an error, so a stray `)` there is not diagnosed again.
    resuming_after_error: bool,
}

/// State transitions for an enum tag and enumerator list.
///
/// C99: §6.7.2.2 paragraph 1, p. 105; PDF p. 117.
#[derive(Debug, Clone, Copy)]
enum EnumPhase {
    /// Consume the `enum` keyword.
    Start,
    AwaitTagAttributes,
    AwaitEnumeratorAttributes,
    PushUnderlyingType,
    AwaitUnderlyingType,
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
    /// Receive the enumerator-value child.
    AwaitEnumeratorValue,
    /// Require `,` or `}` after one enumerator.
    AfterEnumerator,
    /// Store the completed body and return the enum specifier.
    FinishBody,
}

impl<'tu, 'p> EnumSpecifierFrame<'tu, 'p> {
    /// Whether the current `:` opens an enum-type-specifier, whose
    /// specifier-qualifier-list must follow it. Any other colon belongs to
    /// the enclosing grammar: a generic association or an unnamed bit-field.
    /// C23: §6.7.3.3 paragraph 1, p. 109; PDF p. 122.
    fn underlying_type_follows(parser: &Parser<'_, 'tu, 'p>) -> bool {
        parser
            .cursor
            .following()
            .is_some_and(|token| parser.type_name_starter(token))
    }

    pub(super) fn new(arena: &'p Bump) -> Self {
        Self {
            phase: EnumPhase::Start,
            attribute_resume: EnumPhase::NameOrBody,
            attributes: None,
            enumerator_attributes: None,
            underlying_type: None,
            name: None,
            enumerators: ArenaVec::new_in(arena),
            current_enumerator: None,
            body_started: false,
            stopped_before_declaration: false,
            body_starting_error_count: 0,
            source_vectors: ArenaVec::new_in(arena),
            current_enumerator_source: None,
            resuming_after_error: false,
        }
    }

    pub(super) fn step(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
        returned: Option<ParseValue<'tu>>,
    ) -> ParseAction<'tu, 'p> {
        if matches!(self.phase, EnumPhase::FinishBody) && parser.attribute_starter(token) {
            self.attribute_resume = self.phase;
            self.phase = EnumPhase::AwaitTagAttributes;
            return ParseAction::Push(parser.pooled_modern_frame(ModernKind::Attributes));
        }
        match self.phase {
            | EnumPhase::PushUnderlyingType => {
                self.body_starting_error_count = parser.hard_error_count;
                self.phase = EnumPhase::AwaitUnderlyingType;
                ParseAction::Push(ParseFrame::DeclarationSpecifiers(
                    super::declaration_specifiers::DeclarationSpecifiersFrame::new(
                        super::declaration_specifiers::SpecifierMode::TypeName,
                    ),
                ))
            },
            | EnumPhase::AwaitUnderlyingType => {
                let Some(ParseValue::DeclarationSpecifiers(specifiers)) = returned else {
                    unexpected_return!("enum underlying type protocol: {returned:?}")
                };
                let x = parser.alloc_syntax(super::declaration_syntax::TypeName {
                    declaration_specifiers: specifiers,
                    declarator:             None,
                    source_vectors:         specifiers.source_vectors,
                    recovered:              parser.hard_error_count
                        > self.body_starting_error_count,
                });
                // Keep the first specifier when recovering an extra colon.
                // C23: §6.7.3.3 paragraph 1, p. 109; PDF p. 122 permits
                // only one optional enum-type-specifier.
                if self.underlying_type.is_none() {
                    self.underlying_type = Some(x);
                }
                self.source_vectors.push(x.source_vectors);
                self.phase = EnumPhase::AfterName;
                ParseAction::Continue
            },
            | EnumPhase::AwaitTagAttributes | EnumPhase::AwaitEnumeratorAttributes => {
                let Some(ParseValue::Modern(ModernValue::Attributes(x))) = returned else {
                    unexpected_return!("enum attributes protocol: {returned:?}")
                };
                let is_enumerator = matches!(self.phase, EnumPhase::AwaitEnumeratorAttributes);
                let next = if is_enumerator {
                    self.enumerator_attributes
                } else {
                    self.attributes
                };
                let attributes = Some(parser.alloc_syntax(SpecifierExtension {
                    kind: SpecifierExtensionKind::Attributes(x),
                    next,
                    source_vectors: x.source_vectors,
                }));
                self.source_vectors.push(x.source_vectors);
                if is_enumerator {
                    self.enumerator_attributes = attributes;
                    parser
                        .context
                        .merge_into(&mut self.current_enumerator_source, x.source_vectors);
                    self.phase = EnumPhase::AfterEnumeratorName;
                } else {
                    self.attributes = attributes;
                    self.phase = self.attribute_resume;
                }
                ParseAction::Continue
            },
            | EnumPhase::Start => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                let Some(token) = token else {
                    parser.report(ParserErrorType::ExpectedEnumKeyword(None), None);
                    return self.finish(parser);
                };
                if !matches!(token.kind, TokenType::Keyword(KeywordTokenType::Enum)) {
                    parser.report(
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
                if parser.attribute_starter(token) {
                    self.attribute_resume = EnumPhase::NameOrBody;
                    self.phase = EnumPhase::AwaitTagAttributes;
                    return ParseAction::Push(parser.pooled_modern_frame(ModernKind::Attributes));
                }
                if is_operator(token, OperatorTokenType::Colon)
                    && Self::underlying_type_follows(parser)
                {
                    let token = token.expect("colon exists");
                    parser.extension(
                        crate::configuration::Feature::EnumUnderlyingType,
                        "fixed enum underlying type",
                        token,
                    );
                    self.source_vectors.push(token.source_vectors);
                    self.phase = EnumPhase::PushUnderlyingType;
                    return ParseAction::Consume;
                }
                // Like tag specifiers for aggregates, an enum can be a tagged
                // reference, tagged definition, or anonymous definition.
                if let Some(token) = token
                    && matches!(token.kind, TokenType::Identifier)
                {
                    self.name = Some(Identifier::from_token(token));
                    self.source_vectors.push(token.source_vectors);
                    self.phase = EnumPhase::AfterName;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::OpeningCurlyBrace) {
                    let token = token.expect("opening-curly-brace token exists");
                    self.body_started = true;
                    self.body_starting_error_count = parser.hard_error_count;
                    self.source_vectors.push(token.source_vectors);
                    self.phase = EnumPhase::EnumeratorOrClose;
                    ParseAction::Consume
                } else {
                    // C99 §6.7.2.2p1: `enum` takes a tag, a list, or both.
                    parser.report(
                        ParserErrorType::EnumSpecifierWithoutNameAndBody(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.finish(parser)
                }
            },
            | EnumPhase::AfterName => {
                if is_operator(token, OperatorTokenType::Colon)
                    && Self::underlying_type_follows(parser)
                {
                    let token = token.expect("colon exists");
                    if self.underlying_type.is_some() {
                        parser.report(
                            ParserErrorType::ExpectedIsoSyntax(
                                "an enum body or declarator after its underlying type",
                                Some(token.kind),
                            ),
                            Some(token),
                        );
                    }
                    parser.extension(
                        crate::configuration::Feature::EnumUnderlyingType,
                        "fixed enum underlying type",
                        token,
                    );
                    self.source_vectors.push(token.source_vectors);
                    self.phase = EnumPhase::PushUnderlyingType;
                    return ParseAction::Consume;
                }
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                if is_operator(token, OperatorTokenType::OpeningCurlyBrace) {
                    let token = token.expect("opening-curly-brace token exists");
                    self.body_started = true;
                    self.body_starting_error_count = parser.hard_error_count;
                    self.source_vectors.push(token.source_vectors);
                    self.phase = EnumPhase::EnumeratorOrClose;
                    ParseAction::Consume
                } else {
                    self.finish(parser)
                }
            },
            | EnumPhase::EnumeratorOrClose => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // This state is also reached after a comma, which is why `}`
                // accepts the standard's optional trailing-comma form.
                // C99 §6.7.2.2p1: an enumerator-list has at least one
                // enumerator, so `{}` is diagnosed.
                if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
                    let token = token.expect("closing-curly-brace token exists");
                    if self.enumerators.is_empty()
                        && parser.hard_error_count == self.body_starting_error_count
                    {
                        parser.report(
                            ParserErrorType::ExpectedEnumeratorBeforeClosingCurlyBrace,
                            Some(token),
                        );
                    }
                    self.source_vectors.push(token.source_vectors);
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Consume
                } else if let Some(token) = token
                    && matches!(token.kind, TokenType::Identifier)
                {
                    self.current_enumerator = Some(Identifier::from_token(token));
                    self.current_enumerator_source = Some(token.source_vectors);
                    self.source_vectors.push(token.source_vectors);
                    self.phase = EnumPhase::AfterEnumeratorName;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon) {
                    parser.report(
                        ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Reprocess
                } else if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    parser.report(
                        ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    if Self::enumerator_list_continues(parser) {
                        // A stray `)` cannot end the list; its `}` follows.
                        let token = token.expect("closing-parenthesis token exists");
                        self.source_vectors.push(token.source_vectors);
                        return ParseAction::Consume;
                    }
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Reprocess
                } else if token.is_none() {
                    parser.report(
                        ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(
                            None,
                        ),
                        None,
                    );
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Reprocess
                } else if let Some(misplaced) = token
                    && Self::misplaced_enumerator(parser, misplaced)
                {
                    // C99 §6.7.2.2p1: an enumeration constant is an
                    // identifier. A keyword or constant written in its place
                    // is one bad enumerator, not the start of a declaration
                    // that ends this body.
                    parser.report(
                        ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(
                            Some(misplaced.kind),
                        ),
                        token,
                    );
                    self.source_vectors.push(misplaced.source_vectors);
                    self.phase = EnumPhase::AfterEnumeratorName;
                    ParseAction::Consume
                } else {
                    parser.report(
                        ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = EnumPhase::AfterEnumerator;
                    self.resuming_after_error = true;
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
                if parser.attribute_starter(token) {
                    self.phase = EnumPhase::AwaitEnumeratorAttributes;
                    return ParseAction::Push(parser.pooled_modern_frame(ModernKind::Attributes));
                }
                // The constant-expression is optional. Finalize immediately
                // unless `=` explicitly transfers ownership to the value child.
                if is_operator(token, OperatorTokenType::Equals) {
                    let token = token.expect("equals token exists");
                    self.source_vectors.push(token.source_vectors);
                    parser.merge_source(&mut self.current_enumerator_source, token);
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
                    parser.arena,
                    ExpressionMode::ConstantExpression,
                    ExpressionBoundary::Enumerator,
                    parser.hard_error_count,
                )))
            },
            | EnumPhase::AwaitEnumeratorValue => {
                let Some(ParseValue::ConstantExpression(ConstantExpressionResult {
                    expression: index,
                    recovered,
                })) = returned
                else {
                    unexpected_return!(
                        "enumerator-value frame returned an unexpected value: {returned:?}"
                    );
                };
                self.resuming_after_error = recovered;
                let source_vectors = index.expression().source_vectors;
                self.source_vectors.push(source_vectors);
                parser
                    .context
                    .merge_into(&mut self.current_enumerator_source, source_vectors);
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
                let resuming_after_error = std::mem::take(&mut self.resuming_after_error);
                if is_operator(token, OperatorTokenType::Comma) {
                    let token = token.expect("comma token exists");
                    if is_operator(
                        parser.cursor.following(),
                        OperatorTokenType::ClosingCurlyBrace,
                    ) {
                        parser.extension(
                            crate::configuration::Feature::TrailingEnumComma,
                            "trailing enum comma",
                            token,
                        );
                    }
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
                        ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Reprocess
                } else if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    let stray = Self::enumerator_list_continues(parser);
                    if !(stray && resuming_after_error) {
                        parser.report(
                            ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(
                                token.map(|token| token.kind),
                            ),
                            token,
                        );
                    }
                    if stray {
                        // A stray `)` cannot end the list; its `}` follows,
                        // so a following `,` or `}` is still the separator.
                        let token = token.expect("closing-parenthesis token exists");
                        self.source_vectors.push(token.source_vectors);
                        return ParseAction::Consume;
                    }
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Reprocess
                } else if token.is_some_and(|token| matches!(token.kind, TokenType::Identifier)) {
                    parser.report(
                        ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = EnumPhase::EnumeratorOrClose;
                    ParseAction::Reprocess
                } else if let Some(misplaced) = token
                    && Self::misplaced_enumerator(parser, misplaced)
                {
                    // The `,` is missing before a keyword or constant that
                    // stands in for the next enumerator; consume it so it is
                    // not diagnosed again as an enumerator.
                    parser.report(
                        ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(Some(
                            misplaced.kind,
                        )),
                        token,
                    );
                    self.source_vectors.push(misplaced.source_vectors);
                    self.phase = EnumPhase::AfterEnumeratorName;
                    ParseAction::Consume
                } else if token.is_some_and(|token| parser.declaration_starter(token)) {
                    parser.report(
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
                        ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(None),
                        None,
                    );
                    self.phase = EnumPhase::FinishBody;
                    ParseAction::Reprocess
                } else {
                    parser.report(
                        ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.resuming_after_error = true;
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
                self.finish(parser)
            },
        }
    }

    /// Whether `token`, which cannot start an enumerator, stands alone in
    /// enumerator position: a non-identifier, non-delimiter token followed by
    /// `,`, `=`, or `}`. Anything else keeps the malformed-body recovery that
    /// stops before a following declaration.
    fn misplaced_enumerator(parser: &mut Parser<'_, 'tu, 'p>, token: Token) -> bool {
        !matches!(token.kind, TokenType::Identifier)
            && !matches!(
                token.kind,
                TokenType::Operator(
                    OperatorTokenType::OpeningParenthesis
                        | OperatorTokenType::OpeningSquareBracket
                        | OperatorTokenType::OpeningCurlyBrace
                        | OperatorTokenType::ClosingParenthesis
                        | OperatorTokenType::ClosingSquareBracket
                        | OperatorTokenType::ClosingCurlyBrace
                        | OperatorTokenType::Comma
                        | OperatorTokenType::Semicolon
                        | OperatorTokenType::Equals
                )
            )
            && parser.cursor.following().is_some_and(|following| {
                matches!(
                    following.kind,
                    TokenType::Operator(
                        OperatorTokenType::Comma
                            | OperatorTokenType::Equals
                            | OperatorTokenType::ClosingCurlyBrace
                    )
                )
            })
    }

    /// Whether the body's `}` follows a stray `)` before anything that
    /// cannot appear in an enumerator list.
    ///
    /// C99 §6.7.2.2p1: an enumerator list holds only enumerators, `=`,
    /// constant expressions, and commas, so a `)` is stray when the list's
    /// `}` still follows. Inside an enclosing parenthesis, as in
    /// `int f(enum E { A ) int after;`, the `)` closes that parenthesis and
    /// the body's `}` is missing. The scan is bounded so repeated errors stay
    /// linear.
    fn enumerator_list_continues(parser: &mut Parser<'_, 'tu, 'p>) -> bool {
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
                | kind if is_statement_keyword(kind)
                    || nesting == 0
                        && !matches!(kind, TokenType::Identifier)
                        && parser.declaration_starter(token) =>
                {
                    return false;
                },
                | _ => {},
            }
        }
        false
    }

    fn finish_enumerator(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        expression: Option<ConstantExpression<'tu>>,
    ) {
        let source_vectors = self.current_enumerator_source.take().unwrap_or_default();
        if let Some(name) = self.current_enumerator.take() {
            self.enumerators.push(Enumerator {
                attributes: self.enumerator_attributes.take(),
                name,
                expression,
                source_vectors,
            });
            // Enumeration constants join the ordinary identifier namespace as
            // soon as their enumerator completes, affecting later values.
            // C99 §6.2.1p7: the scope begins after the defining enumerator,
            // so `A = A` reads an outer `A`.
            parser.scopes.publish(name.name, NameClass::Ordinary);
        }
    }

    fn finish(&mut self, parser: &mut Parser<'_, 'tu, 'p>) -> ParseAction<'tu, 'p> {
        let enumeration_list = self
            .body_started
            .then(|| parser.alloc_syntax_list(&mut self.enumerators));
        let source_vectors = parser.context.merge_vector_list(&self.source_vectors);
        let index = parser.alloc_syntax(EnumSpecifier {
            underlying_type: self.underlying_type,
            attributes: self.attributes,
            name: self.name,
            enumeration_list,
            source_vectors,
        });
        ParseAction::Reduce(ParseValue::EnumSpecifier(EnumSpecifierResult {
            index,
            stopped_before_declaration: self.stopped_before_declaration,
        }))
    }
}
