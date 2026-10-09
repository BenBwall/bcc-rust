//! Struct and union specifier frame.
//!
//! Translation phase 7 syntax analysis (§5.1.1.2, p. 10; PDF p. 22) of
//! `struct-or-union-specifier`, `struct-declaration-list`,
//! `struct-declaration`, `struct-declarator-list`, and `struct-declarator`
//! (C99: §6.7.2.1 paragraph 1, p. 101; PDF p. 113; §A.2.2, p. 412;
//! PDF p. 424), with the tag forms of §6.7.2.3, pp. 106-107;
//! PDF pp. 118-119.
//!
//! Tags and members have their own name spaces (§6.2.3 paragraph 1, p. 31;
//! PDF p. 43), so neither touches the scope stack. Diagnosed here: syntax
//! the grammar rejects, including an empty member list and a member
//! declaration without a declarator, which C99 does not allow. Left to
//! semantic analysis: the member constraints of §6.7.2.1 paragraphs 2-4,
//! p. 101; PDF p. 113, the named-member and layout rules of paragraphs 7-16,
//! pp. 102-103; PDF pp. 114-115, and the tag constraints of §6.7.2.3
//! paragraphs 1-2, p. 106; PDF p. 118. Members and nested bodies accumulate
//! through arena storage and the frame stack, so the minimums of 1023
//! members and 63 nested definitions (§5.2.4.1, p. 21; PDF p. 33) impose no
//! fixed ceiling.

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
        TypeQualifiers,
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
    statement::is_statement_keyword,
    syntax::Identifier,
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

/// Parses a struct-or-union specifier, including its optional tag and member
/// declaration list.
///
/// C99: structure and union specifiers, member declarations, and bit-fields
/// are §6.7.2.1, pp. 101-104; PDF pp. 113-116.
#[derive(Debug)]
pub(super) struct StructOrUnionSpecifierFrame<'tu, 'p> {
    /// Current tag/member transition.
    phase: StructOrUnionPhase,
    attribute_resume: StructOrUnionPhase,
    attributes: Option<&'tu SpecifierExtension<'tu>>,
    /// Keyword-selected aggregate kind.
    kind: Option<StructOrUnion>,
    /// Optional tag identifier.
    identifier: Option<Identifier>,
    /// Completed member declarations before arena insertion.
    pub(super) declarations: ArenaVec<'p, StructDeclaration<'tu>>,
    /// Declarators belonging to the member declaration in progress.
    pub(super) member_declarators: ArenaVec<'p, StructDeclarator<'tu>>,
    /// Specifiers shared by the member declarators in progress.
    member_specifiers: Option<DeclarationSpecifiers<'tu>>,
    /// Named declarator waiting for an optional bit-field width.
    member_declarator: Option<Declarator<'tu>>,
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
    pub(super) source_vectors: ArenaVec<'p, SourceVectors>,
    /// Provenance accumulated for the member declaration in progress.
    member_source: Option<SourceVectors>,
    /// Provenance for the member declarator/bit-field currently being built.
    current_member_declarator_source: Option<SourceVectors>,
    /// `__extension__` suppression depth when the specifier began. Each
    /// member restores it, so a marker before one member covers only that
    /// member (GNU extension; C99 §5.1.1.3, p. 11; PDF p. 23).
    suppression_entry: usize,
}

/// State transitions for a struct/union tag and member body.
///
/// C99: §6.7.2.1, pp. 101-104; PDF pp. 113-116.
#[derive(Debug, Clone, Copy)]
enum StructOrUnionPhase {
    /// Consume and classify the `struct` or `union` keyword.
    Start,
    AwaitTagAttributes,
    AwaitMemberAttributes,
    AwaitAssertion,
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
    /// Receive the bit-field width child.
    AwaitBitFieldWidth,
    /// Require `,` or `;` after one struct declarator.
    AfterStructDeclarator,
    /// Store the completed body and return the tag specifier.
    FinishBody,
}

impl<'tu, 'p> StructOrUnionSpecifierFrame<'tu, 'p> {
    pub(super) fn new(arena: &'p Bump) -> Self {
        Self {
            phase: StructOrUnionPhase::Start,
            attribute_resume: StructOrUnionPhase::NameOrBody,
            attributes: None,
            kind: None,
            identifier: None,
            declarations: ArenaVec::new_in(arena),
            member_declarators: ArenaVec::new_in(arena),
            member_specifiers: None,
            member_declarator: None,
            body_started: false,
            resuming_after_member_recovery: false,
            width_recovered: false,
            source_vectors: ArenaVec::new_in(arena),
            member_source: None,
            current_member_declarator_source: None,
            suppression_entry: 0,
        }
    }

    pub(super) fn step(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
        returned: Option<ParseValue<'tu>>,
    ) -> ParseAction<'tu, 'p> {
        if matches!(self.phase, StructOrUnionPhase::FinishBody) && parser.attribute_starter(token) {
            self.attribute_resume = self.phase;
            self.phase = StructOrUnionPhase::AwaitTagAttributes;
            return ParseAction::Push(parser.pooled_modern_frame(ModernKind::Attributes));
        }
        match self.phase {
            | StructOrUnionPhase::AwaitMemberAttributes => {
                let Some(ParseValue::Modern(ModernValue::Attributes(x))) = returned else {
                    panic!("bit-field attribute child protocol");
                };
                if let Some(member) = self.member_declarators.last_mut() {
                    member.attributes = Some(parser.alloc_syntax(SpecifierExtension {
                        kind:           SpecifierExtensionKind::Attributes(x),
                        next:           member.attributes,
                        source_vectors: x.source_vectors,
                    }));
                    member.source_vectors = parser
                        .context
                        .merge_vectors(member.source_vectors, x.source_vectors);
                }
                parser
                    .context
                    .merge_into(&mut self.member_source, x.source_vectors);
                self.phase = StructOrUnionPhase::AfterStructDeclarator;
                ParseAction::Continue
            },
            | StructOrUnionPhase::AwaitTagAttributes => {
                let Some(ParseValue::Modern(ModernValue::Attributes(x))) = returned else {
                    panic!("aggregate attributes protocol: {returned:?}")
                };
                self.attributes = Some(parser.alloc_syntax(SpecifierExtension {
                    kind:           SpecifierExtensionKind::Attributes(x),
                    next:           self.attributes,
                    source_vectors: x.source_vectors,
                }));
                self.source_vectors.push(x.source_vectors);
                self.phase = self.attribute_resume;
                ParseAction::Continue
            },
            | StructOrUnionPhase::AwaitAssertion => {
                let Some(ParseValue::Modern(ModernValue::Assertion(assertion))) = returned else {
                    panic!("member assertion protocol: {returned:?}")
                };
                self.declarations.push(StructDeclaration {
                    assertion:              Some(assertion),
                    extensions:             None,
                    type_qualifiers:        TypeQualifiers::empty(),
                    type_specifiers:        super::declaration_syntax::TypeSpecifiers::Empty,
                    struct_declarator_list: crate::util::arena_list::ArenaList::empty(),
                    source_vectors:         assertion.source_vectors,
                });
                self.source_vectors.push(assertion.source_vectors);
                self.phase = StructOrUnionPhase::MemberStart;
                ParseAction::Continue
            },
            | StructOrUnionPhase::Start => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                self.suppression_entry = parser.pedantic_suppression;
                let Some(token) = token else {
                    parser.report(ParserErrorType::ExpectedStructOrUnionKeyword(None), None);
                    return self.finish(parser);
                };
                self.kind = match token.kind {
                    | TokenType::Keyword(KeywordTokenType::Struct) => Some(StructOrUnion::Struct),
                    | TokenType::Keyword(KeywordTokenType::Union) => Some(StructOrUnion::Union),
                    | _ => {
                        parser.report(
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
                if parser.attribute_starter(token) {
                    self.attribute_resume = StructOrUnionPhase::NameOrBody;
                    self.phase = StructOrUnionPhase::AwaitTagAttributes;
                    return ParseAction::Push(parser.pooled_modern_frame(ModernKind::Attributes));
                }
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // The grammar allows either a tagged reference, a tagged
                // definition, or an anonymous definition. Record the tag
                // first and decide whether a body follows in a separate phase.
                if let Some(token) = token
                    && matches!(token.kind, TokenType::Identifier)
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
                    // C99 §6.7.2.1p1: `struct` and `union` take a tag, a
                    // body, or both.
                    parser.report(
                        ParserErrorType::StructOrUnionSpecifierWithoutNameAndBody(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.finish(parser)
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
                    self.finish(parser)
                }
            },
            | StructOrUnionPhase::MemberStart => {
                if token.is_some_and(|x| {
                    matches!(x.kind, TokenType::Keyword(KeywordTokenType::StaticAssert))
                }) {
                    self.phase = StructOrUnionPhase::AwaitAssertion;
                    return ParseAction::Push(parser.pooled_modern_frame(ModernKind::Assertion));
                }
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // This frame owns the body's `}`. Any other token begins a
                // specifier-qualifier-list child for the next member.
                if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
                    let token = token.expect("closing-curly-brace token exists");
                    // C99 §6.7.2.1p1: a struct-declaration-list has at least
                    // one struct-declaration.
                    if self.declarations.is_empty() {
                        parser.extension(
                            crate::configuration::Feature::EmptyStructs,
                            "empty struct or union",
                            token,
                        );
                    }
                    self.source_vectors.push(token.source_vectors);
                    self.phase = StructOrUnionPhase::FinishBody;
                    ParseAction::Consume
                } else if token.is_none() {
                    parser.report(
                        ParserErrorType::ExpectedClosingCurlyBraceInStructDeclarationList(None),
                        None,
                    );
                    self.phase = StructOrUnionPhase::FinishBody;
                    ParseAction::Reprocess
                } else if is_operator(token, OperatorTokenType::Semicolon) {
                    // C99 §6.7.2.1p1: a struct-declaration begins with a
                    // specifier-qualifier-list. GNU extension: an extra `;` in
                    // the list declares nothing, so it does not make the list
                    // nonempty.
                    let token = token.expect("semicolon token exists");
                    parser.extension(
                        crate::configuration::Feature::ExtraSemicolons,
                        "extra semicolon in a struct or union",
                        token,
                    );
                    self.source_vectors.push(token.source_vectors);
                    ParseAction::Consume
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
                // C99 §6.7.2.1p1 and p11.
                if is_operator(token, OperatorTokenType::Colon) {
                    let token = token.expect("colon token exists");
                    parser.merge_source(&mut self.member_source, token);
                    parser.merge_source(&mut self.current_member_declarator_source, token);
                    self.member_declarator = None;
                    self.phase = StructOrUnionPhase::PushBitFieldWidth;
                    ParseAction::Consume
                } else if let Some(token) = token
                    && matches!(
                        token.kind,
                        TokenType::Operator(OperatorTokenType::Semicolon)
                    )
                {
                    // C99 §6.7.2.1p1: a struct-declaration needs a
                    // struct-declarator-list; C99 has no anonymous members.
                    let anonymous = matches!(
                        specifiers.type_specifiers,
                        super::declaration_syntax::TypeSpecifiers::StructOrUnion(x)
                            if x.identifier.is_none() && x.struct_declaration_list.is_some()
                    );
                    // MSVC also accepts a typedef name of a structure or
                    // union; whether the typedef names one is semantic.
                    let ms_anonymous = matches!(
                        specifiers.type_specifiers,
                        super::declaration_syntax::TypeSpecifiers::TypedefName(_)
                    ) || matches!(
                        specifiers.type_specifiers,
                        super::declaration_syntax::TypeSpecifiers::StructOrUnion(x)
                            if x.identifier.is_some()
                    );
                    let ms_anonymous = ms_anonymous
                        && parser
                            .context
                            .configuration
                            .accepts(crate::configuration::Feature::MsAnonymousStructs);
                    if ms_anonymous {
                        parser.extension(
                            crate::configuration::Feature::MsAnonymousStructs,
                            "anonymous tagged struct or union member",
                            token,
                        );
                    } else if anonymous {
                        parser.extension(
                            crate::configuration::Feature::AnonymousAggregates,
                            "anonymous struct or union member",
                            token,
                        );
                    } else {
                        parser.report(ParserErrorType::EmptyStructDeclarator, Some(token));
                    }
                    parser.merge_source(&mut self.member_source, token);
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
                    parser.merge_source(&mut self.member_source, token);
                    parser.merge_source(&mut self.current_member_declarator_source, token);
                    self.member_declarator = None;
                    self.phase = StructOrUnionPhase::PushBitFieldWidth;
                    ParseAction::Consume
                } else {
                    self.phase = StructOrUnionPhase::AwaitMemberDeclarator;
                    ParseAction::Push(ParseFrame::Declarator(DeclaratorFrame::new(
                        parser.arena,
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
                    parser
                        .context
                        .merge_into(&mut self.member_source, source_vectors);
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
                    parser.merge_source(&mut self.member_source, token);
                    parser.merge_source(&mut self.current_member_declarator_source, token);
                    self.phase = StructOrUnionPhase::PushBitFieldWidth;
                    ParseAction::Consume
                } else {
                    let source_vectors = self
                        .current_member_declarator_source
                        .take()
                        .unwrap_or_default();
                    self.member_declarators.push(StructDeclarator {
                        attributes: None,
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
                    parser.arena,
                    ExpressionMode::ConstantExpression,
                    ExpressionBoundary::StructMember,
                    parser.hard_error_count,
                )))
            },
            | StructOrUnionPhase::AwaitBitFieldWidth => {
                let Some(ParseValue::ConstantExpression(ConstantExpressionResult {
                    expression: index,
                    recovered,
                })) = returned
                else {
                    panic!("bit-field frame returned an unexpected value: {returned:?}");
                };
                self.width_recovered = recovered;
                let source_vectors = index.expression().source_vectors;
                parser
                    .context
                    .merge_into(&mut self.member_source, source_vectors);
                parser
                    .context
                    .merge_into(&mut self.current_member_declarator_source, source_vectors);
                let source_vectors = self
                    .current_member_declarator_source
                    .take()
                    .unwrap_or_default();
                self.member_declarators.push(StructDeclarator {
                    attributes: None,
                    declarator: self.member_declarator.take(),
                    bitfield_width: Some(index),
                    source_vectors,
                });
                self.phase = StructOrUnionPhase::AfterStructDeclarator;
                ParseAction::Reprocess
            },
            | StructOrUnionPhase::AfterStructDeclarator => {
                if parser.attribute_starter(token) {
                    self.phase = StructOrUnionPhase::AwaitMemberAttributes;
                    return ParseAction::Push(parser.pooled_modern_frame(ModernKind::Attributes));
                }
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
                    parser.merge_source(&mut self.member_source, token);
                    self.phase = StructOrUnionPhase::PushMemberDeclarator;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon) {
                    let token = token.expect("semicolon token exists");
                    parser.merge_source(&mut self.member_source, token);
                    self.finish_member(parser);
                    self.phase = StructOrUnionPhase::MemberStart;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
                    if !resuming_after_recovery {
                        parser.report(
                            ParserErrorType::ExpectedSemicolonBeforeClosingCurlyBraceInStructDeclaratorList,
                            token,
                        );
                    }
                    self.finish_member(parser);
                    self.phase = StructOrUnionPhase::MemberStart;
                    ParseAction::Reprocess
                } else if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    if Self::member_list_continues(parser) {
                        // A stray `)` cannot end the member list; its `}`
                        // follows, so the member continues after it.
                        if !(resuming_after_recovery || width_recovered) {
                            parser.report(
                                ParserErrorType::ExpectedCommaOrSemicolonInStructDeclaratorList(
                                    token.map(|token| token.kind),
                                ),
                                token,
                            );
                        }
                        let token = token.expect("closing-parenthesis token exists");
                        parser.merge_source(&mut self.member_source, token);
                        // The malformed terminator is diagnosed, so a `}` or
                        // following member that ends this one is not.
                        self.resuming_after_member_recovery = true;
                        return ParseAction::Consume;
                    }
                    parser.report(
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
                        ParserErrorType::ExpectedCommaOrSemicolonInStructDeclaratorList(None),
                        None,
                    );
                    self.finish_member(parser);
                    self.phase = StructOrUnionPhase::FinishBody;
                    ParseAction::Reprocess
                } else {
                    parser.report(
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
                self.finish(parser)
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
    fn member_list_continues(parser: &mut Parser<'_, 'tu, 'p>) -> bool {
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
            let Some(token) = parser.cursor.lookahead(index) else {
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
                            || matches!(previous, Some(TokenType::Identifier))
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

    fn finish_member(&mut self, parser: &mut Parser<'_, 'tu, 'p>) {
        parser.pedantic_suppression = self.suppression_entry;
        // Commit all declarators for this shared specifier-qualifier-list as a
        // single member declaration with one stable arena slice.
        for member in &self.member_declarators {
            if let Some(declarator) = member.declarator
                && declarator.is_unsized_array()
            {
                parser.extension_source(
                    crate::configuration::Feature::FlexibleArrayMembers,
                    "flexible array member",
                    declarator.source_vectors,
                );
            }
        }
        let start = parser.alloc_syntax_list(&mut self.member_declarators);
        let specifiers = self
            .member_specifiers
            .take()
            .expect("a struct member has specifiers");
        let source_vectors = self.member_source.take().unwrap_or_default();
        self.declarations.push(StructDeclaration {
            assertion: None,
            extensions: specifiers.extensions,
            type_qualifiers: specifiers.type_qualifiers,
            type_specifiers: specifiers.type_specifiers,
            struct_declarator_list: start,
            source_vectors,
        });
        self.source_vectors.push(source_vectors);
    }

    fn finish(&mut self, parser: &mut Parser<'_, 'tu, 'p>) -> ParseAction<'tu, 'p> {
        parser.pedantic_suppression = self.suppression_entry;
        let declaration_list = self
            .body_started
            .then(|| parser.alloc_syntax_list(&mut self.declarations));
        let source_vectors = parser.context.merge_vector_list(&self.source_vectors);
        let index = parser.alloc_syntax(StructOrUnionSpecifier {
            attributes: self.attributes,
            struct_or_union: self.kind.unwrap_or(StructOrUnion::Struct),
            identifier: self.identifier,
            struct_declaration_list: declaration_list,
            source_vectors,
        });
        ParseAction::Reduce(ParseValue::StructOrUnionSpecifier(index))
    }
}
