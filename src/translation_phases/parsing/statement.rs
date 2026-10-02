//! Statement frame covering every C99 statement family.

use std::fmt::Debug;

use super::{
    Parser,
    compound_statement::CompoundStatementFrame,
    declaration::{
        DeclarationContext,
        DeclarationFrame,
    },
    errors::ParserErrorType,
    expression::{
        ExpressionBoundary,
        ExpressionFrame,
        ExpressionMode,
    },
    expression_operators::{
        is_operator,
        is_postfix_starter,
    },
    machine::{
        ConstantExpressionResult,
        ExpressionResult,
        ParseAction,
        ParseFrame,
        ParseFrameKind,
        ParseValue,
    },
    recovery::{
        ExpressionTerminator,
        SynchronizationKind,
        SynchronizationSet,
    },
    scope::{
        ScopeKind,
        SwitchScope,
    },
    syntax::{
        ConstantExpressionSlot,
        ExpressionSlot,
        ForInitializer,
        Identifier,
        Statement,
        StatementIndex,
        StatementType,
    },
};
use crate::{
    translation_phases::{
        Context,
        GetPosition,
        GetSourceFileIndex,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HeaderKind {
    If,
    Switch,
    While,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum LabelPrefix {
    Identifier(Identifier),
    Case(ConstantExpressionSlot),
    Default,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum SimpleJump {
    Break,
    Continue,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum StatementPhase {
    Start,
    AwaitCompound,
    AwaitExpression,
    ExpressionSemicolon(ExpressionSlot),
    ReturnStart,
    AwaitReturnExpression,
    ReturnSemicolon(Option<ExpressionSlot>),
    SimpleJumpSemicolon(SimpleJump),
    GotoIdentifier,
    GotoSemicolon(Identifier),
    IdentifierLabelColon(Identifier),
    CaseExpression,
    AwaitCaseExpression,
    CaseColon(ConstantExpressionSlot),
    DefaultColon,
    PushLabeled(LabelPrefix),
    AwaitLabeled(LabelPrefix),
    HeaderOpening(HeaderKind),
    HeaderExpression(HeaderKind),
    AwaitHeaderExpression(HeaderKind),
    HeaderClosing(HeaderKind, ExpressionSlot),
    PushHeaderBody(HeaderKind, ExpressionSlot),
    AwaitHeaderBody(HeaderKind, ExpressionSlot),
    IfAfterThen(ExpressionSlot, StatementIndex),
    PushElse(ExpressionSlot, StatementIndex),
    AwaitElse(ExpressionSlot, StatementIndex),
    DoPushBody,
    DoAwaitBody,
    DoWhileKeyword(StatementIndex),
    DoOpening(StatementIndex),
    DoExpression(StatementIndex),
    DoAwaitExpression(StatementIndex),
    DoClosing(StatementIndex, ExpressionSlot),
    DoSemicolon(StatementIndex, ExpressionSlot),
    ForOpening,
    ForInitializer,
    AwaitForInitializerExpression,
    AwaitForInitializerDeclaration,
    ForInitializerSemicolon(ExpressionSlot),
    ForCondition(Option<ForInitializer>),
    AwaitForCondition(Option<ForInitializer>),
    ForConditionSemicolon(Option<ForInitializer>, ExpressionSlot),
    ForIteration(Option<ForInitializer>, Option<ExpressionSlot>),
    AwaitForIteration(Option<ForInitializer>, Option<ExpressionSlot>),
    ForClosing(
        Option<ForInitializer>,
        Option<ExpressionSlot>,
        Option<ExpressionSlot>,
    ),
    ForPushBody(
        Option<ForInitializer>,
        Option<ExpressionSlot>,
        Option<ExpressionSlot>,
    ),
    ForAwaitBody(
        Option<ForInitializer>,
        Option<ExpressionSlot>,
        Option<ExpressionSlot>,
    ),
    Recovered,
    Finish(StatementType),
}

#[derive(Debug)]
pub(super) struct StatementFrame {
    phase:                     StatementPhase,
    pub(super) source_vectors: Option<SourceVectors>,
    starting_error_count:      usize,
    entry_scope_depth:         Option<usize>,
    implicit_scope:            Option<ScopeKind>,
    owns_switch_scope:         bool,
    leave_else_unconsumed:     bool,
}

impl HeaderKind {
    fn name(self) -> &'static str {
        match self {
            | Self::If => "if statement",
            | Self::Switch => "switch statement",
            | Self::While => "while statement",
        }
    }

    fn scope_kind(self) -> ScopeKind {
        match self {
            | Self::If | Self::Switch => ScopeKind::ImplicitSelection,
            | Self::While => ScopeKind::ImplicitIteration,
        }
    }
}

#[expect(
    clippy::missing_assert_message,
    clippy::too_many_lines,
    reason = "The single iterative statement grammar dispatcher keeps all phase transitions and \
              delimiter ownership visible in one frame implementation."
)]
impl StatementFrame {
    pub(super) fn new(starting_error_count: usize, implicit_scope: Option<ScopeKind>) -> Self {
        Self {
            phase: StatementPhase::Start,
            source_vectors: None,
            starting_error_count,
            entry_scope_depth: None,
            implicit_scope,
            owns_switch_scope: false,
            leave_else_unconsumed: false,
        }
    }

    fn with_else_ownership(
        starting_error_count: usize,
        implicit_scope: Option<ScopeKind>,
        leave_else_unconsumed: bool,
    ) -> Self {
        Self {
            leave_else_unconsumed,
            ..Self::new(starting_error_count, implicit_scope)
        }
    }

    fn for_if_then(starting_error_count: usize, implicit_scope: Option<ScopeKind>) -> Self {
        Self::with_else_ownership(starting_error_count, implicit_scope, true)
    }

    pub(super) fn step(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        token: Option<Token>,
        returned: Option<ParseValue>,
    ) -> ParseAction {
        if self.entry_scope_depth.is_none() {
            self.entry_scope_depth = Some(parser.scopes.depth());
            if let Some(kind) = self.implicit_scope {
                parser.scopes.enter_scope(kind);
            }
        }

        match self.phase {
            | StatementPhase::Start => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::OpeningCurlyBrace) {
                    self.phase = StatementPhase::AwaitCompound;
                    return ParseAction::Push(ParseFrame::CompoundStatement(
                        CompoundStatementFrame::new(parser.hard_error_count, false),
                    ));
                }
                if is_operator(token, OperatorTokenType::Semicolon) {
                    self.merge_token(parser, context, token.expect("semicolon exists"));
                    self.phase = StatementPhase::Finish(StatementType::Null);
                    return ParseAction::Consume;
                }
                if let Some(token) = token
                    && token.kind == TokenType::Identifier
                    && is_operator(parser.cursor.following(context), OperatorTokenType::Colon)
                {
                    let identifier = Identifier::from_token(token);
                    if let Some(labels) = parser.label_scopes.last_mut() {
                        _ = labels.definitions.insert(identifier.name);
                    }
                    self.merge_token(parser, context, token);
                    self.phase = StatementPhase::IdentifierLabelColon(identifier);
                    return ParseAction::Consume;
                }
                if let Some(token) = token
                    && let TokenType::Keyword(keyword) = token.kind
                {
                    match keyword {
                        | KeywordTokenType::Return => {
                            self.merge_token(parser, context, token);
                            self.phase = StatementPhase::ReturnStart;
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::Break => {
                            self.merge_token(parser, context, token);
                            self.phase = StatementPhase::SimpleJumpSemicolon(SimpleJump::Break);
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::Continue => {
                            self.merge_token(parser, context, token);
                            self.phase = StatementPhase::SimpleJumpSemicolon(SimpleJump::Continue);
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::Goto => {
                            self.merge_token(parser, context, token);
                            self.phase = StatementPhase::GotoIdentifier;
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::Case => {
                            self.merge_token(parser, context, token);
                            self.phase = StatementPhase::CaseExpression;
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::Default => {
                            self.merge_token(parser, context, token);
                            self.phase = StatementPhase::DefaultColon;
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::If => {
                            Self::enter_construct_scope(parser, ScopeKind::ImplicitSelection);
                            self.merge_token(parser, context, token);
                            self.phase = StatementPhase::HeaderOpening(HeaderKind::If);
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::Switch => {
                            Self::enter_construct_scope(parser, ScopeKind::ImplicitSelection);
                            parser.switch_scopes.push(SwitchScope::default());
                            self.owns_switch_scope = true;
                            self.merge_token(parser, context, token);
                            self.phase = StatementPhase::HeaderOpening(HeaderKind::Switch);
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::While => {
                            Self::enter_construct_scope(parser, ScopeKind::ImplicitIteration);
                            self.merge_token(parser, context, token);
                            self.phase = StatementPhase::HeaderOpening(HeaderKind::While);
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::Do => {
                            Self::enter_construct_scope(parser, ScopeKind::ImplicitIteration);
                            self.merge_token(parser, context, token);
                            self.phase = StatementPhase::DoPushBody;
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::For => {
                            Self::enter_construct_scope(parser, ScopeKind::ImplicitIteration);
                            self.merge_token(parser, context, token);
                            self.phase = StatementPhase::ForOpening;
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::Sizeof => {},
                        | _ if parser.declaration_starter(token) => {
                            parser.report(
                                context,
                                ParserErrorType::ExpectedStatement(Some(token.kind)),
                                Some(token),
                            );
                            self.phase = StatementPhase::Recovered;
                            return ParseAction::Recover(SynchronizationSet {
                                kind:   SynchronizationKind::Statement,
                                target: ParseFrameKind::Statement,
                            });
                        },
                        | _ => {},
                    }
                }
                let stray_else = token
                    .is_some_and(|token| token.kind == TokenType::Keyword(KeywordTokenType::Else));
                if token.is_none()
                    || is_operator(token, OperatorTokenType::ClosingCurlyBrace)
                    || stray_else
                {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedStatement(token.map(|token| token.kind)),
                        token,
                    );
                    if stray_else && !self.leave_else_unconsumed {
                        self.merge_token(parser, context, token.expect("else token exists"));
                        self.phase = StatementPhase::Finish(StatementType::Null);
                        return ParseAction::Consume;
                    }
                    return self.finish(parser, context, StatementType::Null);
                }
                self.phase = StatementPhase::AwaitExpression;
                ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                    ExpressionMode::Expression,
                    ExpressionBoundary::Statement(ExpressionTerminator::Semicolon),
                    parser.hard_error_count,
                )))
            },
            | StatementPhase::AwaitCompound => {
                let Some(ParseValue::CompoundStatement(statement)) = returned else {
                    panic!("compound statement returned an unexpected value: {returned:?}");
                };
                self.merge_statement(parser, context, statement);
                self.finish_existing(parser, statement)
            },
            | StatementPhase::AwaitExpression => {
                let slot = Self::parsed_slot(returned);
                self.merge_slot(parser, context, slot);
                self.phase = StatementPhase::ExpressionSemicolon(slot);
                ParseAction::Continue
            },
            | StatementPhase::ExpressionSemicolon(slot) => {
                debug_assert!(returned.is_none());
                self.own_semicolon_or_report(parser, context, token, "expression statement");
                self.phase = StatementPhase::Finish(StatementType::Expression(slot));
                if is_operator(token, OperatorTokenType::Semicolon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::ReturnStart => {
                debug_assert!(returned.is_none());
                let identifier_continues_expression = token
                    .is_some_and(|token| token.kind == TokenType::Identifier)
                    && parser.cursor.following(context).is_some_and(|following| {
                        following.kind == TokenType::Operator(OperatorTokenType::Asterisk)
                            || is_postfix_starter(following.kind)
                    });
                if is_operator(token, OperatorTokenType::Semicolon) {
                    self.merge_token(parser, context, token.expect("semicolon exists"));
                    self.phase = StatementPhase::Finish(StatementType::Return(None));
                    ParseAction::Consume
                } else if Self::at_expression_boundary(token, ExpressionTerminator::Semicolon)
                    || token.is_some_and(|token| {
                        !identifier_continues_expression
                            && parser.declaration_recovery_starts_here(context, token)
                    })
                    || token.is_some_and(|token| token.kind == TokenType::Identifier)
                        && is_operator(parser.cursor.following(context), OperatorTokenType::Colon)
                {
                    self.phase = StatementPhase::ReturnSemicolon(None);
                    ParseAction::Reprocess
                } else {
                    self.phase = StatementPhase::AwaitReturnExpression;
                    ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                        ExpressionMode::Expression,
                        ExpressionBoundary::Statement(ExpressionTerminator::Semicolon),
                        parser.hard_error_count,
                    )))
                }
            },
            | StatementPhase::AwaitReturnExpression => {
                let slot = Self::parsed_slot(returned);
                self.merge_slot(parser, context, slot);
                self.phase = StatementPhase::ReturnSemicolon(Some(slot));
                ParseAction::Reprocess
            },
            | StatementPhase::ReturnSemicolon(expression) => {
                debug_assert!(returned.is_none());
                self.own_semicolon_or_report(parser, context, token, "return statement");
                self.phase = StatementPhase::Finish(StatementType::Return(expression));
                if is_operator(token, OperatorTokenType::Semicolon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::SimpleJumpSemicolon(jump) => {
                debug_assert!(returned.is_none());
                self.own_semicolon_or_report(parser, context, token, "jump statement");
                self.phase = StatementPhase::Finish(match jump {
                    | SimpleJump::Break => StatementType::Break,
                    | SimpleJump::Continue => StatementType::Continue,
                });
                if is_operator(token, OperatorTokenType::Semicolon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::GotoIdentifier => {
                debug_assert!(returned.is_none());
                let identifier = if let Some(token) = token
                    && token.kind == TokenType::Identifier
                {
                    let identifier = Identifier::from_token(token);
                    if let Some(labels) = parser.label_scopes.last_mut() {
                        _ = labels.references.insert(identifier.name);
                    }
                    self.merge_token(parser, context, token);
                    self.phase = StatementPhase::GotoSemicolon(identifier);
                    return ParseAction::Consume;
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedGotoLabel(token.map(|token| token.kind)),
                        token,
                    );
                    Identifier::new(
                        context.string_cache.intern("<missing-label>"),
                        context.create_source_vectors(
                            parser.position(context),
                            parser.source_file_index(),
                            0,
                        ),
                    )
                };
                self.phase = StatementPhase::GotoSemicolon(identifier);
                ParseAction::Reprocess
            },
            | StatementPhase::GotoSemicolon(identifier) => {
                debug_assert!(returned.is_none());
                self.own_semicolon_or_report(parser, context, token, "goto statement");
                self.phase = StatementPhase::Finish(StatementType::Goto(identifier));
                if is_operator(token, OperatorTokenType::Semicolon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::IdentifierLabelColon(identifier) => {
                debug_assert!(returned.is_none());
                self.own_colon_or_report(parser, context, token, "identifier label");
                self.phase = StatementPhase::PushLabeled(LabelPrefix::Identifier(identifier));
                if is_operator(token, OperatorTokenType::Colon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::CaseExpression => {
                debug_assert!(returned.is_none());
                if Self::at_expression_boundary(token, ExpressionTerminator::Colon) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedStatementExpression(
                            "case label",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase =
                        StatementPhase::CaseColon(Self::missing_constant_slot(parser, context));
                    ParseAction::Reprocess
                } else {
                    self.phase = StatementPhase::AwaitCaseExpression;
                    ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                        ExpressionMode::ConstantExpression,
                        ExpressionBoundary::Statement(ExpressionTerminator::Colon),
                        parser.hard_error_count,
                    )))
                }
            },
            | StatementPhase::AwaitCaseExpression => {
                let slot = Self::parsed_constant_slot(returned);
                self.merge_constant_slot(parser, context, slot);
                self.phase = StatementPhase::CaseColon(slot);
                ParseAction::Reprocess
            },
            | StatementPhase::CaseColon(expression) => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::Semicolon)
                    && is_operator(parser.cursor.following(context), OperatorTokenType::Colon)
                {
                    self.own_colon_or_report(parser, context, token, "case label");
                    self.merge_token(parser, context, token.expect("semicolon exists"));
                    return ParseAction::Consume;
                }
                self.own_colon_or_report(parser, context, token, "case label");
                self.phase = StatementPhase::PushLabeled(LabelPrefix::Case(expression));
                if is_operator(token, OperatorTokenType::Colon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::DefaultColon => {
                debug_assert!(returned.is_none());
                self.own_colon_or_report(parser, context, token, "default label");
                if parser
                    .switch_scopes
                    .last()
                    .is_some_and(|switch| switch.has_default)
                {
                    parser.report(context, ParserErrorType::DuplicateDefaultLabel, token);
                } else if let Some(switch) = parser.switch_scopes.last_mut() {
                    switch.has_default = true;
                }
                self.phase = StatementPhase::PushLabeled(LabelPrefix::Default);
                if is_operator(token, OperatorTokenType::Colon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::PushLabeled(prefix) => {
                debug_assert!(returned.is_none());
                self.phase = StatementPhase::AwaitLabeled(prefix);
                ParseAction::Push(ParseFrame::Statement(StatementFrame::with_else_ownership(
                    parser.hard_error_count,
                    None,
                    self.leave_else_unconsumed,
                )))
            },
            | StatementPhase::AwaitLabeled(prefix) => {
                let Some(ParseValue::Statement(statement)) = returned else {
                    panic!("labeled child returned an unexpected value: {returned:?}");
                };
                self.merge_statement(parser, context, statement);
                self.finish(
                    parser,
                    context,
                    match prefix {
                        | LabelPrefix::Identifier(identifier) =>
                            StatementType::Label(identifier, statement),
                        | LabelPrefix::Case(expression) =>
                            StatementType::Case(expression, statement),
                        | LabelPrefix::Default => StatementType::Default(statement),
                    },
                )
            },
            | StatementPhase::HeaderOpening(kind) => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::OpeningParenthesis) {
                    self.merge_token(parser, context, token.expect("opening parenthesis exists"));
                    self.phase = StatementPhase::HeaderExpression(kind);
                    ParseAction::Consume
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedOpeningParenthesisInStatement(
                            kind.name(),
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = StatementPhase::HeaderExpression(kind);
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::HeaderExpression(kind) => {
                debug_assert!(returned.is_none());
                if Self::at_expression_boundary(token, ExpressionTerminator::ClosingParenthesis) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedStatementExpression(
                            kind.name(),
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase =
                        StatementPhase::HeaderClosing(kind, Self::missing_slot(parser, context));
                    ParseAction::Reprocess
                } else {
                    self.phase = StatementPhase::AwaitHeaderExpression(kind);
                    ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                        ExpressionMode::Expression,
                        ExpressionBoundary::ClosingParenthesis,
                        parser.hard_error_count,
                    )))
                }
            },
            | StatementPhase::AwaitHeaderExpression(kind) => {
                let slot = Self::parsed_slot(returned);
                self.merge_slot(parser, context, slot);
                self.phase = StatementPhase::HeaderClosing(kind, slot);
                ParseAction::Continue
            },
            | StatementPhase::HeaderClosing(kind, expression) => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    self.merge_token(parser, context, token.expect("closing parenthesis exists"));
                    self.phase = StatementPhase::PushHeaderBody(kind, expression);
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon)
                    && is_operator(
                        parser.cursor.following(context),
                        OperatorTokenType::ClosingParenthesis,
                    )
                {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            kind.name(),
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.merge_token(parser, context, token.expect("semicolon exists"));
                    ParseAction::Consume
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            kind.name(),
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = StatementPhase::PushHeaderBody(kind, expression);
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::PushHeaderBody(kind, expression) => {
                debug_assert!(returned.is_none());
                self.phase = StatementPhase::AwaitHeaderBody(kind, expression);
                let frame = if kind == HeaderKind::If {
                    StatementFrame::for_if_then(parser.hard_error_count, Some(kind.scope_kind()))
                } else {
                    StatementFrame::with_else_ownership(
                        parser.hard_error_count,
                        Some(kind.scope_kind()),
                        self.leave_else_unconsumed,
                    )
                };
                ParseAction::Push(ParseFrame::Statement(frame))
            },
            | StatementPhase::AwaitHeaderBody(kind, expression) => {
                let Some(ParseValue::Statement(body)) = returned else {
                    panic!("selection/iteration body returned an unexpected value: {returned:?}");
                };
                self.merge_statement(parser, context, body);
                match kind {
                    | HeaderKind::If => {
                        self.phase = StatementPhase::IfAfterThen(expression, body);
                        ParseAction::Reprocess
                    },
                    | HeaderKind::Switch => self.finish(
                        parser,
                        context,
                        StatementType::Switch {
                            condition_expression: expression,
                            body_statement:       body,
                        },
                    ),
                    | HeaderKind::While => self.finish(
                        parser,
                        context,
                        StatementType::While {
                            condition_expression: expression,
                            body_statement:       body,
                        },
                    ),
                }
            },
            | StatementPhase::IfAfterThen(expression, then_statement) => {
                debug_assert!(returned.is_none());
                if token
                    .is_some_and(|token| token.kind == TokenType::Keyword(KeywordTokenType::Else))
                {
                    self.merge_token(parser, context, token.expect("else token exists"));
                    self.phase = StatementPhase::PushElse(expression, then_statement);
                    ParseAction::Consume
                } else {
                    self.finish(
                        parser,
                        context,
                        StatementType::If {
                            condition_expression: expression,
                            then_statement,
                            else_statement: None,
                        },
                    )
                }
            },
            | StatementPhase::PushElse(expression, then_statement) => {
                debug_assert!(returned.is_none());
                self.phase = StatementPhase::AwaitElse(expression, then_statement);
                ParseAction::Push(ParseFrame::Statement(StatementFrame::with_else_ownership(
                    parser.hard_error_count,
                    Some(ScopeKind::ImplicitSelection),
                    self.leave_else_unconsumed,
                )))
            },
            | StatementPhase::AwaitElse(expression, then_statement) => {
                let Some(ParseValue::Statement(else_statement)) = returned else {
                    panic!("else child returned an unexpected value: {returned:?}");
                };
                self.merge_statement(parser, context, else_statement);
                self.finish(
                    parser,
                    context,
                    StatementType::If {
                        condition_expression: expression,
                        then_statement,
                        else_statement: Some(else_statement),
                    },
                )
            },
            | StatementPhase::DoPushBody => {
                debug_assert!(returned.is_none());
                self.phase = StatementPhase::DoAwaitBody;
                ParseAction::Push(ParseFrame::Statement(StatementFrame::with_else_ownership(
                    parser.hard_error_count,
                    Some(ScopeKind::ImplicitIteration),
                    self.leave_else_unconsumed,
                )))
            },
            | StatementPhase::DoAwaitBody => {
                let Some(ParseValue::Statement(body)) = returned else {
                    panic!("do body returned an unexpected value: {returned:?}");
                };
                self.merge_statement(parser, context, body);
                self.phase = StatementPhase::DoWhileKeyword(body);
                ParseAction::Reprocess
            },
            | StatementPhase::DoWhileKeyword(body) => {
                debug_assert!(returned.is_none());
                if token
                    .is_some_and(|token| token.kind == TokenType::Keyword(KeywordTokenType::While))
                {
                    self.merge_token(parser, context, token.expect("while token exists"));
                    self.phase = StatementPhase::DoOpening(body);
                    ParseAction::Consume
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedWhileAfterDoBody(token.map(|token| token.kind)),
                        token,
                    );
                    self.phase = StatementPhase::DoOpening(body);
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::DoOpening(body) => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::OpeningParenthesis) {
                    self.merge_token(parser, context, token.expect("opening parenthesis exists"));
                    self.phase = StatementPhase::DoExpression(body);
                    ParseAction::Consume
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedOpeningParenthesisInStatement(
                            "do-while statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = StatementPhase::DoExpression(body);
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::DoExpression(body) => {
                debug_assert!(returned.is_none());
                if Self::at_expression_boundary(token, ExpressionTerminator::ClosingParenthesis) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedStatementExpression(
                            "do-while statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase =
                        StatementPhase::DoClosing(body, Self::missing_slot(parser, context));
                    ParseAction::Reprocess
                } else {
                    self.phase = StatementPhase::DoAwaitExpression(body);
                    ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                        ExpressionMode::Expression,
                        ExpressionBoundary::ClosingParenthesis,
                        parser.hard_error_count,
                    )))
                }
            },
            | StatementPhase::DoAwaitExpression(body) => {
                let expression = Self::parsed_slot(returned);
                self.merge_slot(parser, context, expression);
                self.phase = StatementPhase::DoClosing(body, expression);
                ParseAction::Reprocess
            },
            | StatementPhase::DoClosing(body, expression) => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    self.merge_token(parser, context, token.expect("closing parenthesis exists"));
                    self.phase = StatementPhase::DoSemicolon(body, expression);
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon)
                    && is_operator(
                        parser.cursor.following(context),
                        OperatorTokenType::ClosingParenthesis,
                    )
                {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            "do-while statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.merge_token(parser, context, token.expect("semicolon exists"));
                    ParseAction::Consume
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            "do-while statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = StatementPhase::DoSemicolon(body, expression);
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::DoSemicolon(body, expression) => {
                debug_assert!(returned.is_none());
                self.own_semicolon_or_report(parser, context, token, "do-while statement");
                self.phase = StatementPhase::Finish(StatementType::DoWhile {
                    condition_expression: expression,
                    body_statement:       body,
                });
                if is_operator(token, OperatorTokenType::Semicolon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::ForOpening => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::OpeningParenthesis) {
                    self.merge_token(parser, context, token.expect("opening parenthesis exists"));
                    self.phase = StatementPhase::ForInitializer;
                    ParseAction::Consume
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedOpeningParenthesisInStatement(
                            "for statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = StatementPhase::ForInitializer;
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::ForInitializer => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::Semicolon) {
                    self.merge_token(parser, context, token.expect("semicolon exists"));
                    self.phase = StatementPhase::ForCondition(None);
                    ParseAction::Consume
                } else if token.is_none()
                    || is_operator(token, OperatorTokenType::ClosingParenthesis)
                {
                    self.own_semicolon_or_report(parser, context, token, "for initializer");
                    self.phase = StatementPhase::ForCondition(None);
                    ParseAction::Reprocess
                } else if token.is_some_and(|token| parser.declaration_starter(token)) {
                    self.phase = StatementPhase::AwaitForInitializerDeclaration;
                    ParseAction::Push(ParseFrame::Declaration(DeclarationFrame::new(
                        parser.syntax.init_declarators.len().to_u32(),
                        DeclarationContext::ForInitializer,
                        parser.hard_error_count,
                    )))
                } else {
                    self.phase = StatementPhase::AwaitForInitializerExpression;
                    ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                        ExpressionMode::Expression,
                        ExpressionBoundary::Statement(ExpressionTerminator::ForSemicolon),
                        parser.hard_error_count,
                    )))
                }
            },
            | StatementPhase::AwaitForInitializerExpression => {
                let expression = Self::parsed_slot(returned);
                self.merge_slot(parser, context, expression);
                self.phase = StatementPhase::ForInitializerSemicolon(expression);
                ParseAction::Reprocess
            },
            | StatementPhase::AwaitForInitializerDeclaration => {
                let Some(ParseValue::Declaration(declaration)) = returned else {
                    panic!("for declaration returned an unexpected value: {returned:?}");
                };
                let source = parser.syntax.declarations[declaration.0 as usize].source_vectors;
                self.source_vectors = Some(
                    self.source_vectors
                        .map_or(source, |existing| context.merge_vectors(existing, source)),
                );
                self.phase =
                    StatementPhase::ForCondition(Some(ForInitializer::Declaration(declaration)));
                ParseAction::Reprocess
            },
            | StatementPhase::ForInitializerSemicolon(expression) => {
                debug_assert!(returned.is_none());
                self.own_semicolon_or_report(parser, context, token, "for initializer");
                self.phase =
                    StatementPhase::ForCondition(Some(ForInitializer::Expression(expression)));
                if is_operator(token, OperatorTokenType::Semicolon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::ForCondition(initializer) => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::OpeningCurlyBrace) {
                    self.own_semicolon_or_report(parser, context, token, "for condition");
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            "for statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = StatementPhase::ForPushBody(initializer, None, None);
                    ParseAction::Reprocess
                } else if is_operator(token, OperatorTokenType::Semicolon) {
                    self.merge_token(parser, context, token.expect("semicolon exists"));
                    self.phase = StatementPhase::ForIteration(initializer, None);
                    ParseAction::Consume
                } else if token.is_none()
                    || is_operator(token, OperatorTokenType::ClosingParenthesis)
                {
                    self.own_semicolon_or_report(parser, context, token, "for condition");
                    self.phase = StatementPhase::ForIteration(initializer, None);
                    ParseAction::Reprocess
                } else {
                    self.phase = StatementPhase::AwaitForCondition(initializer);
                    ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                        ExpressionMode::Expression,
                        ExpressionBoundary::Statement(ExpressionTerminator::ForSemicolon),
                        parser.hard_error_count,
                    )))
                }
            },
            | StatementPhase::AwaitForCondition(initializer) => {
                let expression = Self::parsed_slot(returned);
                self.merge_slot(parser, context, expression);
                self.phase = StatementPhase::ForConditionSemicolon(initializer, expression);
                ParseAction::Reprocess
            },
            | StatementPhase::ForConditionSemicolon(initializer, condition) => {
                debug_assert!(returned.is_none());
                self.own_semicolon_or_report(parser, context, token, "for condition");
                self.phase = StatementPhase::ForIteration(initializer, Some(condition));
                if is_operator(token, OperatorTokenType::Semicolon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::ForIteration(initializer, condition) => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    self.merge_token(parser, context, token.expect("closing parenthesis exists"));
                    self.phase = StatementPhase::ForPushBody(initializer, condition, None);
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon)
                    && is_operator(
                        parser.cursor.following(context),
                        OperatorTokenType::ClosingParenthesis,
                    )
                {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            "for statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.merge_token(parser, context, token.expect("semicolon exists"));
                    ParseAction::Consume
                } else if token.is_none() {
                    self.phase = StatementPhase::ForClosing(initializer, condition, None);
                    ParseAction::Reprocess
                } else {
                    self.phase = StatementPhase::AwaitForIteration(initializer, condition);
                    ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                        ExpressionMode::Expression,
                        ExpressionBoundary::ClosingParenthesis,
                        parser.hard_error_count,
                    )))
                }
            },
            | StatementPhase::AwaitForIteration(initializer, condition) => {
                let expression = Self::parsed_slot(returned);
                self.merge_slot(parser, context, expression);
                self.phase = StatementPhase::ForClosing(initializer, condition, Some(expression));
                ParseAction::Reprocess
            },
            | StatementPhase::ForClosing(initializer, condition, iteration) => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    self.merge_token(parser, context, token.expect("closing parenthesis exists"));
                    self.phase = StatementPhase::ForPushBody(initializer, condition, iteration);
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon)
                    && is_operator(
                        parser.cursor.following(context),
                        OperatorTokenType::ClosingParenthesis,
                    )
                {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            "for statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.merge_token(parser, context, token.expect("semicolon exists"));
                    ParseAction::Consume
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            "for statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = StatementPhase::ForPushBody(initializer, condition, iteration);
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::ForPushBody(initializer, condition, iteration) => {
                debug_assert!(returned.is_none());
                self.phase = StatementPhase::ForAwaitBody(initializer, condition, iteration);
                ParseAction::Push(ParseFrame::Statement(StatementFrame::with_else_ownership(
                    parser.hard_error_count,
                    Some(ScopeKind::ImplicitIteration),
                    self.leave_else_unconsumed,
                )))
            },
            | StatementPhase::ForAwaitBody(initializer, condition, iteration) => {
                let Some(ParseValue::Statement(body)) = returned else {
                    panic!("for body returned an unexpected value: {returned:?}");
                };
                self.merge_statement(parser, context, body);
                self.finish(
                    parser,
                    context,
                    StatementType::For {
                        initializer,
                        condition_expression: condition,
                        iteration_expression: iteration,
                        body_statement: body,
                    },
                )
            },
            | StatementPhase::Recovered => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::Semicolon) {
                    self.merge_token(parser, context, token.expect("semicolon exists"));
                    self.phase = StatementPhase::Finish(StatementType::Null);
                    ParseAction::Consume
                } else {
                    self.finish(parser, context, StatementType::Null)
                }
            },
            | StatementPhase::Finish(kind) => {
                debug_assert!(returned.is_none());
                self.finish(parser, context, kind)
            },
        }
    }

    fn enter_construct_scope(parser: &mut Parser, kind: ScopeKind) {
        parser.scopes.enter_scope(kind);
    }

    fn at_expression_boundary(token: Option<Token>, terminator: ExpressionTerminator) -> bool {
        token.is_none_or(|token| {
            SynchronizationKind::StatementExpression(terminator).stops_before(token.kind)
        })
    }

    fn missing_slot(parser: &Parser, context: &mut Context) -> ExpressionSlot {
        let position = parser.position(context);
        let source_file_index = parser.source_file_index();
        ExpressionSlot::Missing(context.create_source_vectors(position, source_file_index, 0))
    }

    fn missing_constant_slot(parser: &Parser, context: &mut Context) -> ConstantExpressionSlot {
        let position = parser.position(context);
        let source_file_index = parser.source_file_index();
        ConstantExpressionSlot::Missing(context.create_source_vectors(
            position,
            source_file_index,
            0,
        ))
    }

    fn merge_token(&mut self, parser: &Parser, context: &mut Context, token: Token) {
        parser.merge_source(context, &mut self.source_vectors, token);
    }

    fn merge_statement(
        &mut self,
        parser: &Parser,
        context: &mut Context,
        statement: StatementIndex,
    ) {
        let source = parser.statement_source(statement);
        self.source_vectors = Some(
            self.source_vectors
                .map_or(source, |existing| context.merge_vectors(existing, source)),
        );
    }

    fn parsed_slot(returned: Option<ParseValue>) -> ExpressionSlot {
        let Some(ParseValue::Expression(ExpressionResult { index, recovered })) = returned else {
            panic!("expression frame returned an unexpected value: {returned:?}");
        };
        let _ = recovered;
        ExpressionSlot::Parsed(index)
    }

    fn parsed_constant_slot(returned: Option<ParseValue>) -> ConstantExpressionSlot {
        let Some(ParseValue::ConstantExpression(ConstantExpressionResult { index, recovered })) =
            returned
        else {
            panic!("constant-expression frame returned an unexpected value: {returned:?}");
        };
        let _ = recovered;
        ConstantExpressionSlot::Parsed(index)
    }

    fn merge_slot(&mut self, parser: &Parser, context: &mut Context, slot: ExpressionSlot) {
        let source = match slot {
            | ExpressionSlot::Parsed(index) =>
                parser.syntax.expressions[index.0 as usize].source_vectors,
            | ExpressionSlot::Missing(source) => source,
        };
        if source.length > 0 {
            self.source_vectors = Some(
                self.source_vectors
                    .map_or(source, |existing| context.merge_vectors(existing, source)),
            );
        }
    }

    fn merge_constant_slot(
        &mut self,
        parser: &Parser,
        context: &mut Context,
        slot: ConstantExpressionSlot,
    ) {
        let source = match slot {
            | ConstantExpressionSlot::Parsed(index) =>
                parser.syntax.expressions[index.0 as usize].source_vectors,
            | ConstantExpressionSlot::Missing(source) => source,
        };
        if source.length > 0 {
            self.source_vectors = Some(
                self.source_vectors
                    .map_or(source, |existing| context.merge_vectors(existing, source)),
            );
        }
    }

    fn own_semicolon_or_report(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        token: Option<Token>,
        position: &'static str,
    ) {
        if is_operator(token, OperatorTokenType::Semicolon) {
            self.merge_token(parser, context, token.expect("semicolon exists"));
        } else {
            parser.report(
                context,
                ParserErrorType::ExpectedSemicolonInStatement(
                    position,
                    token.map(|token| token.kind),
                ),
                token,
            );
        }
    }

    fn own_colon_or_report(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        token: Option<Token>,
        position: &'static str,
    ) {
        if is_operator(token, OperatorTokenType::Colon) {
            self.merge_token(parser, context, token.expect("colon exists"));
        } else {
            parser.report(
                context,
                ParserErrorType::ExpectedColonInLabel(position, token.map(|token| token.kind)),
                token,
            );
        }
    }

    fn finish_existing(&mut self, parser: &mut Parser, statement: StatementIndex) -> ParseAction {
        self.restore_scopes(parser);
        ParseAction::Reduce(ParseValue::Statement(statement))
    }

    fn finish(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        kind: StatementType,
    ) -> ParseAction {
        let index = parser.syntax.statements.len().to_u32();
        let source_vectors = self.source_vectors.unwrap_or_else(|| {
            context.create_source_vectors(parser.position(context), parser.source_file_index(), 0)
        });
        parser.push_syntax(
            |syntax| &mut syntax.statements,
            Statement {
                kind,
                source_vectors,
                recovered: parser.hard_error_count > self.starting_error_count,
            },
        );
        self.restore_scopes(parser);
        ParseAction::Reduce(ParseValue::Statement(StatementIndex(
            index,
            parser.syntax_id,
        )))
    }

    fn restore_scopes(&mut self, parser: &mut Parser) {
        if self.owns_switch_scope {
            let _switch_scope = parser
                .switch_scopes
                .pop()
                .expect("switch statement owns its switch scope");
            self.owns_switch_scope = false;
        }
        parser.scopes.restore_depth(
            self.entry_scope_depth
                .expect("statement frame recorded its entry depth"),
        );
    }
}

/// Returns whether a token begins a statement production rather than an
/// expression that the expression frame can consume.
pub(super) fn is_statement_keyword(token: TokenType) -> bool {
    matches!(
        token,
        TokenType::Keyword(
            KeywordTokenType::Break
                | KeywordTokenType::Case
                | KeywordTokenType::Continue
                | KeywordTokenType::Default
                | KeywordTokenType::Do
                | KeywordTokenType::Else
                | KeywordTokenType::For
                | KeywordTokenType::Goto
                | KeywordTokenType::If
                | KeywordTokenType::Return
                | KeywordTokenType::Switch
                | KeywordTokenType::While
        )
    )
}
