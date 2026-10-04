//! Struct and union specifier frame.

use std::fmt::Debug;

use super::{
    Parser,
    declaration_specifiers::{
        DeclarationSpecifiersFrame,
        SpecifierMode,
    },
    declaration_syntax::{
        DeclarationSpecifiers,
        Declarator,
        StructDeclaration,
        StructDeclarator,
        StructOrUnion,
        StructOrUnionSpecifier,
    },
    declarator::{
        DeclaratorFrame,
        DeclaratorMode,
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
        ParseAction,
        ParseFrame,
        ParseFrameKind,
        ParseValue,
    },
    recovery::{
        SynchronizationKind,
        SynchronizationSet,
    },
    statement::is_statement_keyword,
    syntax::{
        ExpressionIndex,
        Identifier,
        StructOrUnionSpecifierIndex,
    },
};
use crate::translation_phases::{
    Context,
    SourceVectors,
    preprocessing::{
        KeywordTokenType,
        OperatorTokenType,
        Token,
        TokenType,
    },
};

/// Parses a struct-or-union specifier, including its optional tag and member
/// declaration list.
///
/// C99: structure and union specifiers, member declarations, and bit-fields
/// are §6.7.2.1, pp. 101-104; PDF pp. 113-116.
#[derive(Debug)]
pub(super) struct StructOrUnionSpecifierFrame {
    /// Current tag/member transition.
    phase: StructOrUnionPhase,
    /// Keyword-selected aggregate kind.
    kind: Option<StructOrUnion>,
    /// Optional tag identifier.
    identifier: Option<Identifier>,
    /// Completed member declarations before arena insertion.
    pub(super) declarations: Vec<StructDeclaration>,
    /// Declarators belonging to the member declaration in progress.
    pub(super) member_declarators: Vec<StructDeclarator>,
    /// Specifiers shared by the member declarators in progress.
    member_specifiers: Option<DeclarationSpecifiers>,
    /// Named declarator waiting for an optional bit-field width.
    member_declarator: Option<Declarator>,
    /// Whether `{` was consumed, distinguishing a reference from a definition.
    body_started: bool,
    /// Whether member synchronization just ran after a reported malformed
    /// struct-declarator terminator, so stopping before `}` or a following
    /// declaration finishes the member without a second diagnostic.
    resuming_after_member_recovery: bool,
    /// Whether the bit-field width before the current separator position
    /// already reported an error, so a stray `)` there is not diagnosed
    /// again.
    width_recovered: bool,
    /// Provenance accumulated across the complete tag specifier.
    pub(super) source_vectors: Vec<SourceVectors>,
    /// Provenance accumulated for the member declaration in progress.
    member_source: Option<SourceVectors>,
    /// Provenance for the member declarator/bit-field currently being built.
    current_member_declarator_source: Option<SourceVectors>,
}

/// State transitions for a struct/union tag and member body.
///
/// C99: §6.7.2.1, pp. 101-104; PDF pp. 113-116.
#[derive(Debug, Clone, Copy)]
pub(super) enum StructOrUnionPhase {
    /// Consume and classify the `struct` or `union` keyword.
    Start,
    /// Parse an optional tag or anonymous opening brace.
    NameOrBody,
    /// Decide whether a named tag also has a body.
    AfterName,
    /// Parse `}` or begin another member declaration.
    MemberStart,
    /// Receive member specifiers and select named/unnamed declarator syntax.
    AwaitMemberSpecifiers,
    /// Push a member declarator unless an unnamed bit-field starts with `:`.
    PushMemberDeclarator,
    /// Receive the optional member declarator.
    AwaitMemberDeclarator,
    /// Decide whether a bit-field width follows the member declarator.
    AfterMemberDeclarator,
    /// Push the constant-expression bit-field width.
    PushBitFieldWidth,
    /// Receive the recovered bit-field width placeholder.
    AwaitBitFieldWidth,
    /// Require `,` or `;` after one struct declarator.
    AfterStructDeclarator,
    /// Store the completed body and return the tag-specifier handle.
    FinishBody,
}

impl StructOrUnionSpecifierFrame {
    pub(super) fn new() -> Self {
        Self {
            phase: StructOrUnionPhase::Start,
            kind: None,
            identifier: None,
            declarations: Vec::new(),
            member_declarators: Vec::new(),
            member_specifiers: None,
            member_declarator: None,
            body_started: false,
            resuming_after_member_recovery: false,
            width_recovered: false,
            source_vectors: Vec::new(),
            member_source: None,
            current_member_declarator_source: None,
        }
    }

    pub(super) fn step(
        &mut self,
        parser: &mut Parser,
        context: &mut Context<'_>,
        token: Option<Token>,
        returned: Option<ParseValue>,
    ) -> ParseAction {
        match self.phase {
            | StructOrUnionPhase::Start => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                let Some(token) = token else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedStructOrUnionKeyword(None),
                        None,
                    );
                    return self.finish(parser, context);
                };
                self.kind = match token.kind {
                    | TokenType::Keyword(KeywordTokenType::Struct) => Some(StructOrUnion::Struct),
                    | TokenType::Keyword(KeywordTokenType::Union) => Some(StructOrUnion::Union),
                    | _ => {
                        parser.report(
                            context,
                            ParserErrorType::ExpectedStructOrUnionKeyword(Some(token.kind)),
                            Some(token),
                        );
                        None
                    },
                };
                self.source_vectors.push(token.source_vectors);
                self.phase = StructOrUnionPhase::NameOrBody;
                ParseAction::Consume
            },
            | StructOrUnionPhase::NameOrBody => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // The grammar allows either a tagged reference, a tagged
                // definition, or an anonymous definition. Record the tag
                // first and decide whether a body follows in a separate phase.
                if let Some(token) = token
                    && token.kind == TokenType::Identifier
                {
                    self.identifier = Some(Identifier::from_token(token));
                    self.source_vectors.push(token.source_vectors);
                    self.phase = StructOrUnionPhase::AfterName;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::OpeningCurlyBrace) {
                    let token = token.expect("opening-curly-brace token exists");
                    self.body_started = true;
                    self.source_vectors.push(token.source_vectors);
                    self.phase = StructOrUnionPhase::MemberStart;
                    ParseAction::Consume
                } else {
                    parser.report(
                        context,
                        ParserErrorType::StructOrUnionSpecifierWithoutNameAndBody(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.finish(parser, context)
                }
            },
            | StructOrUnionPhase::AfterName => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                if is_operator(token, OperatorTokenType::OpeningCurlyBrace) {
                    let token = token.expect("opening-curly-brace token exists");
                    self.body_started = true;
                    self.source_vectors.push(token.source_vectors);
                    self.phase = StructOrUnionPhase::MemberStart;
                    ParseAction::Consume
                } else {
                    self.finish(parser, context)
                }
            },
            | StructOrUnionPhase::MemberStart => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // This frame owns the body's `}`. Any other token begins a
                // specifier-qualifier-list child for the next member.
                if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
                    let token = token.expect("closing-curly-brace token exists");
                    if self.declarations.is_empty() {
                        parser.report(
                            context,
                            ParserErrorType::ExpectedStructDeclarationBeforeClosingCurlyBrace,
                            Some(token),
                        );
                    }
                    self.source_vectors.push(token.source_vectors);
                    self.phase = StructOrUnionPhase::FinishBody;
                    ParseAction::Consume
                } else if token.is_none() {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingCurlyBraceInStructDeclarationList(None),
                        None,
                    );
                    self.phase = StructOrUnionPhase::FinishBody;
                    ParseAction::Reprocess
                } else {
                    self.member_source = None;
                    self.current_member_declarator_source = None;
                    self.phase = StructOrUnionPhase::AwaitMemberSpecifiers;
                    ParseAction::Push(ParseFrame::DeclarationSpecifiers(
                        DeclarationSpecifiersFrame::new(SpecifierMode::StructMember),
                    ))
                }
            },
            | StructOrUnionPhase::AwaitMemberSpecifiers => {
                let Some(ParseValue::DeclarationSpecifiers(specifiers)) = returned else {
                    panic!("member specifiers returned an unexpected value: {returned:?}");
                };
                self.member_specifiers = Some(specifiers);
                self.member_source = Some(specifiers.source_vectors);
                // A leading colon is the unnamed-bit-field alternative; it
                // deliberately bypasses the named declarator child.
                if is_operator(token, OperatorTokenType::Colon) {
                    let token = token.expect("colon token exists");
                    parser.merge_source(context, &mut self.member_source, token);
                    parser.merge_source(context, &mut self.current_member_declarator_source, token);
                    self.member_declarator = None;
                    self.phase = StructOrUnionPhase::PushBitFieldWidth;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon) {
                    parser.report(context, ParserErrorType::EmptyStructDeclarator, token);
                    let token = token.expect("semicolon token exists");
                    parser.merge_source(context, &mut self.member_source, token);
                    self.finish_member(parser);
                    self.phase = StructOrUnionPhase::MemberStart;
                    ParseAction::Consume
                } else {
                    self.phase = StructOrUnionPhase::PushMemberDeclarator;
                    ParseAction::Reprocess
                }
            },
            | StructOrUnionPhase::PushMemberDeclarator => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                if is_operator(token, OperatorTokenType::Colon) {
                    let token = token.expect("colon token exists");
                    parser.merge_source(context, &mut self.member_source, token);
                    parser.merge_source(context, &mut self.current_member_declarator_source, token);
                    self.member_declarator = None;
                    self.phase = StructOrUnionPhase::PushBitFieldWidth;
                    ParseAction::Consume
                } else {
                    self.phase = StructOrUnionPhase::AwaitMemberDeclarator;
                    ParseAction::Push(ParseFrame::Declarator(DeclaratorFrame::new(
                        DeclaratorMode::Named,
                    )))
                }
            },
            | StructOrUnionPhase::AwaitMemberDeclarator => {
                let Some(ParseValue::Declarator(declarator)) = returned else {
                    panic!("member declarator returned an unexpected value: {returned:?}");
                };
                self.member_declarator = declarator;
                if let Some(source_vectors) = declarator.map(|declarator| declarator.source_vectors)
                {
                    self.member_source =
                        Some(self.member_source.map_or(source_vectors, |existing| {
                            context.merge_vectors(existing, source_vectors)
                        }));
                    self.current_member_declarator_source = Some(source_vectors);
                }
                self.phase = StructOrUnionPhase::AfterMemberDeclarator;
                ParseAction::Reprocess
            },
            | StructOrUnionPhase::AfterMemberDeclarator => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // Colon upgrades the completed member into a named bit-field.
                // Otherwise commit it as an ordinary member without consuming
                // the separator that follows.
                if is_operator(token, OperatorTokenType::Colon) {
                    let token = token.expect("colon token exists");
                    parser.merge_source(context, &mut self.member_source, token);
                    parser.merge_source(context, &mut self.current_member_declarator_source, token);
                    self.phase = StructOrUnionPhase::PushBitFieldWidth;
                    ParseAction::Consume
                } else {
                    let source_vectors = self
                        .current_member_declarator_source
                        .take()
                        .unwrap_or_default();
                    self.member_declarators.push(StructDeclarator {
                        declarator: self.member_declarator.take(),
                        bitfield_width: None,
                        source_vectors,
                    });
                    self.phase = StructOrUnionPhase::AfterStructDeclarator;
                    ParseAction::Reprocess
                }
            },
            | StructOrUnionPhase::PushBitFieldWidth => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                self.phase = StructOrUnionPhase::AwaitBitFieldWidth;
                ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                    ExpressionMode::ConstantExpression,
                    ExpressionBoundary::StructMember,
                    parser.hard_error_count,
                )))
            },
            | StructOrUnionPhase::AwaitBitFieldWidth => {
                let Some(ParseValue::ConstantExpression(ConstantExpressionResult {
                    index,
                    recovered,
                })) = returned
                else {
                    panic!("bit-field frame returned an unexpected value: {returned:?}");
                };
                self.width_recovered = recovered;
                let source_vectors = parser.syntax[ExpressionIndex::from(index)].source_vectors;
                self.member_source = Some(self.member_source.map_or(source_vectors, |existing| {
                    context.merge_vectors(existing, source_vectors)
                }));
                self.current_member_declarator_source = Some(
                    self.current_member_declarator_source
                        .map_or(source_vectors, |existing| {
                            context.merge_vectors(existing, source_vectors)
                        }),
                );
                let source_vectors = self
                    .current_member_declarator_source
                    .take()
                    .unwrap_or_default();
                self.member_declarators.push(StructDeclarator {
                    declarator: self.member_declarator.take(),
                    bitfield_width: Some(index),
                    source_vectors,
                });
                self.phase = StructOrUnionPhase::AfterStructDeclarator;
                ParseAction::Reprocess
            },
            | StructOrUnionPhase::AfterStructDeclarator => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // `,` stays inside one struct-declarator-list; `;` commits the
                // whole struct-declaration. Enclosing delimiters are repaired
                // and reprocessed rather than swallowed by this frame.
                let resuming_after_recovery =
                    std::mem::take(&mut self.resuming_after_member_recovery);
                let width_recovered = std::mem::take(&mut self.width_recovered);
                if is_operator(token, OperatorTokenType::Comma) {
                    let token = token.expect("comma token exists");
                    parser.merge_source(context, &mut self.member_source, token);
                    self.phase = StructOrUnionPhase::PushMemberDeclarator;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon) {
                    let token = token.expect("semicolon token exists");
                    parser.merge_source(context, &mut self.member_source, token);
                    self.finish_member(parser);
                    self.phase = StructOrUnionPhase::MemberStart;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
                    if !resuming_after_recovery {
                        parser.report(
                            context,
                            ParserErrorType::ExpectedSemicolonBeforeClosingCurlyBraceInStructDeclaratorList,
                            token,
                        );
                    }
                    self.finish_member(parser);
                    self.phase = StructOrUnionPhase::MemberStart;
                    ParseAction::Reprocess
                } else if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    if Self::member_list_continues(parser, context) {
                        // A stray `)` cannot end the member list; its `}`
                        // follows, so the member continues after it.
                        if !(resuming_after_recovery || width_recovered) {
                            parser.report(
                                context,
                                ParserErrorType::ExpectedCommaOrSemicolonInStructDeclaratorList(
                                    token.map(|token| token.kind),
                                ),
                                token,
                            );
                        }
                        let token = token.expect("closing-parenthesis token exists");
                        parser.merge_source(context, &mut self.member_source, token);
                        // The malformed terminator is diagnosed, so a `}` or
                        // following member that ends this one is not.
                        self.resuming_after_member_recovery = true;
                        return ParseAction::Consume;
                    }
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingCurlyBraceInStructDeclarationList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.finish_member(parser);
                    self.phase = StructOrUnionPhase::FinishBody;
                    ParseAction::Reprocess
                } else if token.is_some_and(|token| parser.declaration_starter(token)) {
                    if !resuming_after_recovery {
                        parser.report(
                            context,
                            ParserErrorType::ExpectedCommaOrSemicolonInStructDeclaratorList(
                                token.map(|token| token.kind),
                            ),
                            token,
                        );
                    }
                    self.finish_member(parser);
                    self.phase = StructOrUnionPhase::MemberStart;
                    ParseAction::Reprocess
                } else if token.is_none() {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrSemicolonInStructDeclaratorList(None),
                        None,
                    );
                    self.finish_member(parser);
                    self.phase = StructOrUnionPhase::FinishBody;
                    ParseAction::Reprocess
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrSemicolonInStructDeclaratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.resuming_after_member_recovery = true;
                    ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::StructMember,
                        target: ParseFrameKind::StructOrUnionSpecifier,
                    })
                }
            },
            | StructOrUnionPhase::FinishBody => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                self.finish(parser, context)
            },
        }
    }

    /// Whether the body's `}` follows a stray `)` after a member declarator,
    /// with only member declarations in between.
    ///
    /// C99 §6.7.2.1p1: a struct-declaration-list holds member declarations,
    /// each starting with a specifier or qualifier, so a `)` is stray when
    /// the rest of the member and any member declarations lead to the list's
    /// `}`. Inside an enclosing parenthesis, as in
    /// `int f(struct S { int x ) int after;`, the `)` closes that
    /// parenthesis and the body's `}` is missing. The scan is bounded so
    /// repeated errors stay linear.
    fn member_list_continues(parser: &mut Parser, context: &mut Context<'_>) -> bool {
        const SCAN_LIMIT: usize = 64;
        // Parentheses and brackets, which never contain `;`.
        let mut groups = 0_u32;
        // Nested struct, union, or enum bodies.
        let mut braces = 0_u32;
        // The first tokens finish the current member; each later member
        // starts with a declaration specifier or qualifier.
        let mut member_start = false;
        let mut previous = None;
        let mut before_previous = None;
        for index in 0..SCAN_LIMIT {
            let Some(token) = parser.cursor.lookahead(context, index) else {
                return false;
            };
            let kind = token.kind;
            if is_statement_keyword(kind) {
                return false;
            }
            let top_level = groups == 0 && braces == 0;
            if top_level && member_start && !is_operator(Some(token), OperatorTokenType::Semicolon)
            {
                if !is_operator(Some(token), OperatorTokenType::ClosingCurlyBrace)
                    && !parser.declaration_starter(token)
                {
                    return false;
                }
                member_start = false;
            }
            match kind {
                | TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) => {
                    if groups > 0 {
                        return false;
                    }
                    if braces == 0 {
                        return true;
                    }
                    braces -= 1;
                },
                | TokenType::Operator(OperatorTokenType::Semicolon) => {
                    if groups > 0 {
                        return false;
                    }
                    member_start = braces == 0;
                },
                | TokenType::Operator(OperatorTokenType::Equals) if groups == 0 => return false,
                | TokenType::Operator(OperatorTokenType::OpeningCurlyBrace) => {
                    let opens_tag_body = |kind: Option<TokenType>| {
                        matches!(
                            kind,
                            Some(TokenType::Keyword(
                                KeywordTokenType::Struct
                                    | KeywordTokenType::Union
                                    | KeywordTokenType::Enum
                            ))
                        )
                    };
                    if groups == 0
                        && !(opens_tag_body(previous)
                            || previous == Some(TokenType::Identifier)
                                && opens_tag_body(before_previous))
                    {
                        return false;
                    }
                    braces += 1;
                },
                | TokenType::Operator(
                    OperatorTokenType::OpeningParenthesis | OperatorTokenType::OpeningSquareBracket,
                ) => groups += 1,
                | TokenType::Operator(
                    OperatorTokenType::ClosingParenthesis | OperatorTokenType::ClosingSquareBracket,
                ) => {
                    if groups == 0 {
                        return false;
                    }
                    groups -= 1;
                },
                | _ => {},
            }
            before_previous = previous;
            previous = Some(kind);
        }
        false
    }

    fn finish_member(&mut self, parser: &mut Parser) {
        // Commit all declarators for this shared specifier-qualifier-list as a
        // single member declaration with one stable arena slice.
        let start = parser.append_syntax(&mut self.member_declarators);
        let specifiers = self
            .member_specifiers
            .take()
            .expect("a struct member has specifiers");
        let source_vectors = self.member_source.take().unwrap_or_default();
        self.declarations.push(StructDeclaration {
            type_qualifiers: specifiers.type_qualifiers,
            type_specifiers: specifiers.type_specifiers,
            struct_declarator_list: start,
            source_vectors,
        });
        self.source_vectors.push(source_vectors);
    }

    fn finish(&mut self, parser: &mut Parser, context: &mut Context<'_>) -> ParseAction {
        let declaration_list = self
            .body_started
            .then(|| parser.append_syntax(&mut self.declarations));
        let index = parser.push_syntax(StructOrUnionSpecifier {
            struct_or_union:         self.kind.unwrap_or(StructOrUnion::Struct),
            identifier:              self.identifier,
            struct_declaration_list: declaration_list,
            source_vectors:          context.merge_vector_list(&self.source_vectors),
        });
        ParseAction::Reduce(ParseValue::StructOrUnionSpecifier(
            StructOrUnionSpecifierIndex(index),
        ))
    }
}
