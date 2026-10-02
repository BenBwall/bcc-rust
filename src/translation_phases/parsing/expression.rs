//! Expression frame and its Double-E precedence reducer.

use std::fmt::Debug;

use super::{
    Parser,
    errors::ParserErrorType,
    expression_operators::{
        LanguageExpressionOperator,
        binary_operator,
        is_assignment_operator,
        is_expression_operand_starter,
        is_postfix_starter,
        prefix_operator,
    },
    initializer::InitializerFrame,
    machine::{
        ConstantExpressionResult,
        ExpressionResult,
        InitializerResult,
        ParseAction,
        ParseFrame,
        ParseValue,
        any_expression_value,
        expression_value,
    },
    recovery::ExpressionTerminator,
    statement::is_statement_keyword,
    syntax::{
        BinaryOperator,
        Constant,
        ConstantExpressionIndex,
        ExpressionIndex,
        ExpressionType,
        Identifier,
        SyntaxList,
        TypeNameIndex,
        UnaryOperator,
    },
    type_name::TypeNameFrame,
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
pub(super) enum ExpressionMode {
    Expression,
    AssignmentExpression,
    ConstantExpression,
    CastExpression,
    UnaryExpression,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExpressionParserState {
    Operand,
    Operator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExpressionBoundary {
    Statement(ExpressionTerminator),
    ClosingParenthesis,
    ClosingSquareBracket,
    Argument,
    Initializer,
    ArrayBound,
    StructMember,
    Enumerator,
    Designator,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ExpressionOperand {
    index:              ExpressionIndex,
    unary_expression:   bool,
    postfix_expression: bool,
}

#[derive(Debug)]
pub(super) struct ExpressionFrame {
    mode:                  ExpressionMode,
    boundary:              ExpressionBoundary,
    recovery_boundary:     ExpressionBoundary,
    phase:                 ExpressionPhase,
    operators:             Vec<LanguageExpressionOperator>,
    operands:              Vec<ExpressionOperand>,
    state:                 ExpressionParserState,
    starting_error_count:  usize,
    /// Postfix call under construction, boxed because most expression frames
    /// never parse a call.
    pub(super) call:       Option<Box<CallState>>,
    pending_sizeof_prefix: Option<SourceVectors>,
}

/// Arguments and provenance of the postfix call an expression frame is
/// building.
#[derive(Debug, Default)]
pub(super) struct CallState {
    pub(super) arguments: Vec<ExpressionIndex>,
    source_vectors:       Vec<SourceVectors>,
    operator_sources:     Vec<SourceVectors>,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum ExpressionPhase {
    Parse,
    Finish,
    RecoverUnexpectedBrace(u32, SourceVectors),
    PushGrouped(SourceVectors),
    AwaitGrouped(SourceVectors),
    CloseGrouped(SourceVectors, ExpressionIndex),
    PushSubscript(ExpressionIndex, SourceVectors),
    AwaitSubscript(ExpressionIndex, SourceVectors),
    CloseSubscript(ExpressionIndex, SourceVectors, ExpressionIndex),
    CallStart(ExpressionIndex, SourceVectors),
    PushCallArgument(ExpressionIndex),
    AwaitCallArgument(ExpressionIndex),
    CallSeparator(ExpressionIndex),
    ExpectMember(ExpressionIndex, bool, SourceVectors),
    PushPrefix(UnaryOperator, SourceVectors, ExpressionMode),
    AwaitPrefix(UnaryOperator, SourceVectors),
    SizeofStart(SourceVectors),
    PushSizeofExpression(SourceVectors),
    AwaitSizeofExpression(SourceVectors),
    PushTypeName(SourceVectors, TypeNameUse),
    AwaitTypeName(SourceVectors, TypeNameUse),
    CloseTypeName(SourceVectors, TypeNameUse, TypeNameIndex),
    PushCompoundLiteral(TypeNameIndex, SourceVectors, TypeNameUse),
    AwaitCompoundLiteral(TypeNameIndex, SourceVectors, TypeNameUse),
    PushCastOperand(TypeNameIndex, SourceVectors),
    AwaitCastOperand(TypeNameIndex, SourceVectors),
    PushConditionalMiddle,
    AwaitConditionalMiddle,
    ExpectConditionalColon(ExpressionIndex),
    PushConditionalElse,
    AwaitConditionalElse,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum TypeNameUse {
    Cast,
    Sizeof(SourceVectors),
    UnaryCompoundLiteral,
}

impl ExpressionFrame {
    pub(super) fn new(
        mode: ExpressionMode,
        boundary: ExpressionBoundary,
        starting_error_count: usize,
    ) -> Self {
        Self::with_recovery_boundary(mode, boundary, boundary, starting_error_count)
    }

    fn nested(
        &self,
        mode: ExpressionMode,
        boundary: ExpressionBoundary,
        starting_error_count: usize,
    ) -> Self {
        Self::with_recovery_boundary(mode, boundary, self.recovery_boundary, starting_error_count)
    }

    pub(super) fn with_recovery_boundary(
        mode: ExpressionMode,
        boundary: ExpressionBoundary,
        recovery_boundary: ExpressionBoundary,
        starting_error_count: usize,
    ) -> Self {
        Self {
            mode,
            boundary,
            recovery_boundary,
            phase: ExpressionPhase::Parse,
            operators: Vec::new(),
            operands: Vec::new(),
            state: ExpressionParserState::Operand,
            starting_error_count,
            call: None,
            pending_sizeof_prefix: None,
        }
    }

    fn closing_parenthesis_is_boundary(&self) -> bool {
        [self.boundary, self.recovery_boundary]
            .into_iter()
            .any(|boundary| {
                matches!(
                    boundary,
                    ExpressionBoundary::ClosingParenthesis
                        | ExpressionBoundary::Argument
                        | ExpressionBoundary::Statement(ExpressionTerminator::ClosingParenthesis)
                )
            })
    }

    fn closing_square_bracket_is_boundary(&self) -> bool {
        [self.boundary, self.recovery_boundary]
            .into_iter()
            .any(|boundary| {
                matches!(
                    boundary,
                    ExpressionBoundary::ClosingSquareBracket
                        | ExpressionBoundary::ArrayBound
                        | ExpressionBoundary::Designator
                )
            })
    }

    #[expect(
        clippy::missing_assert_message,
        clippy::too_many_lines,
        reason = "The explicit expression frame keeps the C precedence grammar and transition \
                  invariants in one non-recursive state machine."
    )]
    pub(super) fn step(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        token: Option<Token>,
        returned: Option<ParseValue>,
    ) -> ParseAction {
        match self.phase {
            | ExpressionPhase::RecoverUnexpectedBrace(depth, source_vectors) => {
                debug_assert!(returned.is_none());
                let Some(token) = token else {
                    self.push_error_with_source(parser, source_vectors, None);
                    self.phase = ExpressionPhase::Parse;
                    return self.finish(parser, context);
                };
                let source_vectors = context.merge_vectors(source_vectors, token.source_vectors);
                let depth = match token.kind {
                    | TokenType::Operator(OperatorTokenType::OpeningCurlyBrace) => depth + 1,
                    | TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) => depth - 1,
                    | _ => depth,
                };
                if depth == 0 {
                    self.push_error_with_source(parser, source_vectors, None);
                    self.phase = ExpressionPhase::Parse;
                } else {
                    self.phase = ExpressionPhase::RecoverUnexpectedBrace(depth, source_vectors);
                }
                return ParseAction::Consume;
            },
            | ExpressionPhase::PushGrouped(opening) => {
                debug_assert!(returned.is_none());
                self.phase = ExpressionPhase::AwaitGrouped(opening);
                return ParseAction::Push(ParseFrame::Expression(self.nested(
                    ExpressionMode::Expression,
                    ExpressionBoundary::ClosingParenthesis,
                    parser.hard_error_count,
                )));
            },
            | ExpressionPhase::AwaitGrouped(opening) => {
                let child = expression_value(returned);
                self.phase = ExpressionPhase::CloseGrouped(opening, child);
                return ParseAction::Continue;
            },
            | ExpressionPhase::CloseGrouped(opening, child) => {
                debug_assert!(returned.is_none());
                let mut operator_source = opening;
                let mut source = context.merge_vectors(
                    opening,
                    parser.syntax.expressions[child.0 as usize].source_vectors,
                );
                let consume = if let Some(close) = token
                    && close.kind == TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                {
                    source = context.merge_vectors(source, close.source_vectors);
                    operator_source = context.merge_vectors(operator_source, close.source_vectors);
                    true
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            "grouped expression",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    false
                };
                let index = parser.store_expression(
                    ExpressionType::Parenthesized { expression: child },
                    source,
                    Some(operator_source),
                    parser.hard_error_count > self.starting_error_count,
                );
                self.push_operand(index, true, true);
                self.phase = ExpressionPhase::Parse;
                return if consume {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                };
            },
            | ExpressionPhase::PushSubscript(base, opening) => {
                debug_assert!(returned.is_none());
                self.phase = ExpressionPhase::AwaitSubscript(base, opening);
                return ParseAction::Push(ParseFrame::Expression(self.nested(
                    ExpressionMode::Expression,
                    ExpressionBoundary::ClosingSquareBracket,
                    parser.hard_error_count,
                )));
            },
            | ExpressionPhase::AwaitSubscript(base, opening) => {
                let child = expression_value(returned);
                self.phase = ExpressionPhase::CloseSubscript(base, opening, child);
                return ParseAction::Continue;
            },
            | ExpressionPhase::CloseSubscript(base, opening, child) => {
                debug_assert!(returned.is_none());
                let mut operator_source = opening;
                let mut source = context.merge_vectors(
                    parser.syntax.expressions[base.0 as usize].source_vectors,
                    opening,
                );
                source = context.merge_vectors(
                    source,
                    parser.syntax.expressions[child.0 as usize].source_vectors,
                );
                let consume = if let Some(close) = token
                    && close.kind == TokenType::Operator(OperatorTokenType::ClosingSquareBracket)
                {
                    source = context.merge_vectors(source, close.source_vectors);
                    operator_source = context.merge_vectors(operator_source, close.source_vectors);
                    true
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingSquareBracketInSubscript(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    false
                };
                let index = parser.store_expression(
                    ExpressionType::Binary {
                        operator:         BinaryOperator::Subscript,
                        left_expression:  base,
                        right_expression: child,
                    },
                    source,
                    Some(operator_source),
                    parser.hard_error_count > self.starting_error_count,
                );
                self.push_operand(index, true, true);
                self.phase = ExpressionPhase::Parse;
                return if consume {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                };
            },
            | ExpressionPhase::CallStart(base, opening) => {
                debug_assert!(returned.is_none());
                let base_source = parser.syntax.expressions[base.0 as usize].source_vectors;
                let call = self.call_state();
                call.source_vectors.clear();
                call.source_vectors.extend([base_source, opening]);
                call.operator_sources.clear();
                call.operator_sources.push(opening);
                if let Some(close) = token
                    && close.kind == TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                {
                    self.merge_call_source(close.source_vectors);
                    self.merge_call_operator_source(close.source_vectors);
                    self.finish_call(parser, context, base);
                    self.phase = ExpressionPhase::Parse;
                    return ParseAction::Consume;
                }
                self.phase = ExpressionPhase::PushCallArgument(base);
                return ParseAction::Reprocess;
            },
            | ExpressionPhase::PushCallArgument(base) => {
                debug_assert!(returned.is_none());
                self.phase = ExpressionPhase::AwaitCallArgument(base);
                return ParseAction::Push(ParseFrame::Expression(self.nested(
                    ExpressionMode::AssignmentExpression,
                    ExpressionBoundary::Argument,
                    parser.hard_error_count,
                )));
            },
            | ExpressionPhase::AwaitCallArgument(base) => {
                let argument = expression_value(returned);
                self.call_state().arguments.push(argument);
                self.merge_call_source(
                    parser.syntax.expressions[argument.0 as usize].source_vectors,
                );
                self.phase = ExpressionPhase::CallSeparator(base);
                return ParseAction::Continue;
            },
            | ExpressionPhase::CallSeparator(base) => {
                debug_assert!(returned.is_none());
                if let Some(separator) = token
                    && separator.kind == TokenType::Operator(OperatorTokenType::Comma)
                {
                    self.merge_call_source(separator.source_vectors);
                    self.merge_call_operator_source(separator.source_vectors);
                    self.phase = ExpressionPhase::PushCallArgument(base);
                    return ParseAction::Consume;
                }
                // A child may return without consuming an enclosing
                // grammar boundary (for example
                // a label after a malformed initializer).
                // Avoid retrying the boundary as another call argument.
                if token.is_some_and(|token| {
                    is_expression_operand_starter(token.kind)
                        && !self.is_strong_grammar_boundary(parser, context, token)
                }) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionCall(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = ExpressionPhase::PushCallArgument(base);
                    return ParseAction::Reprocess;
                }
                let consume = if let Some(close) = token
                    && close.kind == TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                {
                    self.merge_call_source(close.source_vectors);
                    self.merge_call_operator_source(close.source_vectors);
                    true
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            "function call",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    false
                };
                self.finish_call(parser, context, base);
                self.phase = ExpressionPhase::Parse;
                return if consume {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                };
            },
            | ExpressionPhase::ExpectMember(base, indirect, operator_source) => {
                debug_assert!(returned.is_none());
                let Some(member_token) = token.filter(|token| token.kind == TokenType::Identifier)
                else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedMemberIdentifier(token.map(|token| token.kind)),
                        token,
                    );
                    let source = context.merge_vectors(
                        parser.syntax.expressions[base.0 as usize].source_vectors,
                        operator_source,
                    );
                    self.push_error_with_source(parser, source, Some(operator_source));
                    self.phase = ExpressionPhase::Parse;
                    return ParseAction::Reprocess;
                };
                let source = context.merge_vectors(
                    parser.syntax.expressions[base.0 as usize].source_vectors,
                    operator_source,
                );
                let source = context.merge_vectors(source, member_token.source_vectors);
                let kind = if indirect {
                    ExpressionType::IndirectMember {
                        base_expression: base,
                        member:          Identifier::from_token(member_token),
                    }
                } else {
                    ExpressionType::DirectMember {
                        base_expression: base,
                        member:          Identifier::from_token(member_token),
                    }
                };
                let index = parser.store_expression(
                    kind,
                    source,
                    Some(operator_source),
                    parser.hard_error_count > self.starting_error_count,
                );
                self.push_operand(index, true, true);
                self.phase = ExpressionPhase::Parse;
                return ParseAction::Consume;
            },
            | ExpressionPhase::PushPrefix(operator, operator_source, child_mode) => {
                debug_assert!(returned.is_none());
                self.phase = ExpressionPhase::AwaitPrefix(operator, operator_source);
                return ParseAction::Push(ParseFrame::Expression(self.nested(
                    child_mode,
                    self.boundary,
                    parser.hard_error_count,
                )));
            },
            | ExpressionPhase::AwaitPrefix(operator, operator_source) => {
                let operand = expression_value(returned);
                let source = context.merge_vectors(
                    operator_source,
                    parser.syntax.expressions[operand.0 as usize].source_vectors,
                );
                let index = parser.store_expression(
                    ExpressionType::Unary {
                        operator,
                        operand_expression: operand,
                    },
                    source,
                    Some(operator_source),
                    parser.hard_error_count > self.starting_error_count,
                );
                self.push_operand(index, true, false);
                self.phase = ExpressionPhase::Parse;
                return ParseAction::Reprocess;
            },
            | ExpressionPhase::SizeofStart(sizeof_source) => {
                debug_assert!(returned.is_none());
                if let Some(opening) = token
                    && opening.kind == TokenType::Operator(OperatorTokenType::OpeningParenthesis)
                    && parser
                        .cursor
                        .following(context)
                        .is_some_and(|following| parser.type_name_starter(following))
                {
                    self.phase = ExpressionPhase::PushTypeName(
                        opening.source_vectors,
                        TypeNameUse::Sizeof(sizeof_source),
                    );
                    return ParseAction::Consume;
                }
                self.phase = ExpressionPhase::PushSizeofExpression(sizeof_source);
                return ParseAction::Reprocess;
            },
            | ExpressionPhase::PushSizeofExpression(sizeof_source) => {
                debug_assert!(returned.is_none());
                self.phase = ExpressionPhase::AwaitSizeofExpression(sizeof_source);
                return ParseAction::Push(ParseFrame::Expression(self.nested(
                    ExpressionMode::UnaryExpression,
                    self.boundary,
                    parser.hard_error_count,
                )));
            },
            | ExpressionPhase::AwaitSizeofExpression(sizeof_source) => {
                let operand = expression_value(returned);
                let source = context.merge_vectors(
                    sizeof_source,
                    parser.syntax.expressions[operand.0 as usize].source_vectors,
                );
                let index = parser.store_expression(
                    ExpressionType::SizeofExpr(operand),
                    source,
                    Some(sizeof_source),
                    parser.hard_error_count > self.starting_error_count,
                );
                self.push_operand(index, true, false);
                self.phase = ExpressionPhase::Parse;
                return ParseAction::Reprocess;
            },
            | ExpressionPhase::PushTypeName(opening_source, use_kind) => {
                debug_assert!(returned.is_none());
                self.phase = ExpressionPhase::AwaitTypeName(opening_source, use_kind);
                return ParseAction::Push(ParseFrame::TypeName(TypeNameFrame::new(
                    parser.hard_error_count,
                )));
            },
            | ExpressionPhase::AwaitTypeName(opening_source, use_kind) => {
                let Some(ParseValue::TypeName(type_name)) = returned else {
                    panic!("type-name frame returned an unexpected value: {returned:?}");
                };
                self.phase = ExpressionPhase::CloseTypeName(opening_source, use_kind, type_name);
                return ParseAction::Continue;
            },
            | ExpressionPhase::CloseTypeName(opening_source, use_kind, type_name) => {
                debug_assert!(returned.is_none());
                let mut source = context.merge_vectors(
                    opening_source,
                    parser.syntax.type_names[type_name.0 as usize].source_vectors,
                );
                let consume = if let Some(close) = token
                    && close.kind == TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                {
                    source = context.merge_vectors(source, close.source_vectors);
                    true
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            "type name",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    false
                };
                let starts_compound_literal = consume
                    && parser.cursor.following(context).is_some_and(|following| {
                        following.kind == TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
                    })
                    && !parser.cursor.lookahead(context, 1).is_some_and(|first| {
                        first.kind != TokenType::Identifier
                            && (parser.declaration_starter(first)
                                || is_statement_keyword(first.kind))
                    });
                if starts_compound_literal {
                    self.phase = ExpressionPhase::PushCompoundLiteral(type_name, source, use_kind);
                } else {
                    match use_kind {
                        | TypeNameUse::Sizeof(sizeof_source) => {
                            let source = context.merge_vectors(sizeof_source, source);
                            let index = parser.store_expression(
                                ExpressionType::SizeofType(type_name),
                                source,
                                Some(sizeof_source),
                                parser.hard_error_count > self.starting_error_count,
                            );
                            self.push_operand(index, true, false);
                            self.phase = ExpressionPhase::Parse;
                        },
                        | TypeNameUse::Cast => {
                            self.phase = ExpressionPhase::PushCastOperand(type_name, source);
                        },
                        | TypeNameUse::UnaryCompoundLiteral => {
                            parser.report(
                                context,
                                ParserErrorType::ExpectedStatementExpression(
                                    "compound literal initializer",
                                    token.map(|token| token.kind),
                                ),
                                token,
                            );
                            self.push_error(parser, context, token);
                            self.phase = ExpressionPhase::Parse;
                        },
                    }
                }
                return if consume {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                };
            },
            | ExpressionPhase::PushCompoundLiteral(type_name, type_source, use_kind) => {
                debug_assert!(returned.is_none());
                self.phase =
                    ExpressionPhase::AwaitCompoundLiteral(type_name, type_source, use_kind);
                return ParseAction::Push(ParseFrame::Initializer(InitializerFrame::new(
                    parser.hard_error_count,
                    self.closing_parenthesis_is_boundary(),
                    self.closing_square_bracket_is_boundary(),
                )));
            },
            | ExpressionPhase::AwaitCompoundLiteral(type_name, type_source, use_kind) => {
                let Some(ParseValue::Initializer(InitializerResult {
                    index: initializer, ..
                })) = returned
                else {
                    panic!(
                        "compound-literal initializer returned an unexpected value: {returned:?}"
                    );
                };
                let source = context.merge_vectors(
                    type_source,
                    parser.syntax.initializers[initializer.0 as usize].source_vectors,
                );
                let compound = parser.store_expression(
                    ExpressionType::CompoundLiteral {
                        type_name,
                        initializer,
                    },
                    source,
                    Some(type_source),
                    parser.hard_error_count > self.starting_error_count,
                );
                if let TypeNameUse::Sizeof(sizeof_source) = use_kind {
                    self.pending_sizeof_prefix = Some(sizeof_source);
                }
                self.push_operand(compound, true, true);
                self.phase = ExpressionPhase::Parse;
                return ParseAction::Reprocess;
            },
            | ExpressionPhase::PushCastOperand(type_name, type_source) => {
                debug_assert!(returned.is_none());
                self.phase = ExpressionPhase::AwaitCastOperand(type_name, type_source);
                return ParseAction::Push(ParseFrame::Expression(self.nested(
                    ExpressionMode::CastExpression,
                    self.boundary,
                    parser.hard_error_count,
                )));
            },
            | ExpressionPhase::AwaitCastOperand(type_name, type_source) => {
                let operand = expression_value(returned);
                let source = context.merge_vectors(
                    type_source,
                    parser.syntax.expressions[operand.0 as usize].source_vectors,
                );
                let index = parser.store_expression(
                    ExpressionType::Cast {
                        target_type:        type_name,
                        operand_expression: operand,
                    },
                    source,
                    Some(type_source),
                    parser.hard_error_count > self.starting_error_count,
                );
                self.push_operand(index, false, false);
                self.phase = ExpressionPhase::Parse;
                return ParseAction::Reprocess;
            },
            | ExpressionPhase::PushConditionalMiddle => {
                debug_assert!(returned.is_none());
                self.phase = ExpressionPhase::AwaitConditionalMiddle;
                return ParseAction::Push(ParseFrame::Expression(self.nested(
                    ExpressionMode::Expression,
                    ExpressionBoundary::Statement(ExpressionTerminator::Colon),
                    parser.hard_error_count,
                )));
            },
            | ExpressionPhase::AwaitConditionalMiddle => {
                let middle = expression_value(returned);
                self.phase = ExpressionPhase::ExpectConditionalColon(middle);
                return ParseAction::Reprocess;
            },
            | ExpressionPhase::ExpectConditionalColon(middle) => {
                debug_assert!(returned.is_none());
                if let Some(colon) = token
                    && colon.kind == TokenType::Operator(OperatorTokenType::Colon)
                {
                    let Some(LanguageExpressionOperator::Question { source_vectors }) =
                        self.operators.pop()
                    else {
                        panic!("conditional middle must retain its question marker");
                    };
                    self.operators
                        .push(LanguageExpressionOperator::Conditional {
                            middle,
                            question_source: source_vectors,
                            colon_source: Some(colon.source_vectors),
                        });
                    self.phase = ExpressionPhase::PushConditionalElse;
                    return ParseAction::Consume;
                }
                parser.report(
                    context,
                    ParserErrorType::ExpectedColonInLabel(
                        "conditional expression",
                        token.map(|token| token.kind),
                    ),
                    token,
                );
                let Some(LanguageExpressionOperator::Question { source_vectors }) =
                    self.operators.pop()
                else {
                    panic!("conditional middle must retain its question marker");
                };
                self.operators
                    .push(LanguageExpressionOperator::Conditional {
                        middle,
                        question_source: source_vectors,
                        colon_source: None,
                    });
                self.phase = ExpressionPhase::PushConditionalElse;
                return ParseAction::Reprocess;
            },
            | ExpressionPhase::PushConditionalElse => {
                debug_assert!(returned.is_none());
                self.phase = ExpressionPhase::AwaitConditionalElse;
                return ParseAction::Push(ParseFrame::Expression(self.nested(
                    ExpressionMode::ConstantExpression,
                    self.boundary,
                    parser.hard_error_count,
                )));
            },
            | ExpressionPhase::AwaitConditionalElse => {
                let final_expression = any_expression_value(returned);
                let Some(LanguageExpressionOperator::Conditional {
                    middle,
                    question_source,
                    colon_source,
                }) = self.operators.pop()
                else {
                    panic!("conditional final operand must retain its completed marker");
                };
                let condition = self.pop_operand().index;
                let source = context.merge_vectors(
                    parser.syntax.expressions[condition.0 as usize].source_vectors,
                    question_source,
                );
                let source = context.merge_vectors(
                    source,
                    parser.syntax.expressions[middle.0 as usize].source_vectors,
                );
                let source =
                    colon_source.map_or(source, |colon| context.merge_vectors(source, colon));
                let source = context.merge_vectors(
                    source,
                    parser.syntax.expressions[final_expression.0 as usize].source_vectors,
                );
                let index = parser.store_expression(
                    ExpressionType::Conditional {
                        condition_expression: condition,
                        then_expression:      middle,
                        else_expression:      final_expression,
                    },
                    source,
                    Some(colon_source.map_or(question_source, |colon| {
                        context.merge_vectors(question_source, colon)
                    })),
                    parser.hard_error_count > self.starting_error_count,
                );
                self.push_operand(index, false, false);
                self.phase = ExpressionPhase::Parse;
                return ParseAction::Reprocess;
            },
            | ExpressionPhase::Finish => return self.finish(parser, context),
            | ExpressionPhase::Parse => {
                debug_assert!(returned.is_none());
            },
        }

        if self.state == ExpressionParserState::Operator
            && self.pending_sizeof_prefix.is_some()
            && !token.is_some_and(|token| is_postfix_starter(token.kind))
        {
            let operand = self.pop_operand().index;
            let sizeof_source = self
                .pending_sizeof_prefix
                .take()
                .expect("pending sizeof operand has an operator source");
            let source = context.merge_vectors(
                sizeof_source,
                parser.syntax.expressions[operand.0 as usize].source_vectors,
            );
            let index = parser.store_expression(
                ExpressionType::SizeofExpr(operand),
                source,
                Some(sizeof_source),
                parser.hard_error_count > self.starting_error_count,
            );
            self.push_operand(index, true, false);
            return ParseAction::Reprocess;
        }

        if let Some(token) = token
            && Self::is_closing_delimiter(token.kind)
            && self.should_consume_unowned_closer(token.kind)
        {
            let position = if self.state == ExpressionParserState::Operand {
                "expression operand"
            } else {
                "operator in expression"
            };
            parser.report(
                context,
                ParserErrorType::ExpectedStatementExpression(position, Some(token.kind)),
                Some(token),
            );
            self.phase = ExpressionPhase::Finish;
            return ParseAction::Consume;
        }

        if self.state == ExpressionParserState::Operator
            && self.is_boundary(token.map(|token| token.kind))
        {
            return self.finish(parser, context);
        }

        if self.state == ExpressionParserState::Operand {
            let Some(token) = token else {
                parser.report(
                    context,
                    ParserErrorType::ExpectedStatementExpression("expression", None),
                    None,
                );
                self.push_error(parser, context, None);
                return self.finish(parser, context);
            };
            // A visible typedef spelling is still a syntactically valid
            // primary expression. At the first initializer operand, do
            // not reinterpret declaration-shaped
            // lookahead as recovery evidence.
            let following_kind = parser
                .cursor
                .following(context)
                .map(|following| following.kind);
            let identifier_is_unambiguous_recovery_boundary = token.kind == TokenType::Identifier
                && (following_kind == Some(TokenType::Operator(OperatorTokenType::Colon))
                    || self.recovery_boundary == ExpressionBoundary::Initializer
                        && following_kind == Some(TokenType::Identifier));
            let identifier_is_operand =
                token.kind == TokenType::Identifier && !identifier_is_unambiguous_recovery_boundary;
            if self.is_owning_boundary(token.kind)
                || self.recovery_boundary != self.boundary
                    && Self::is_owning_boundary_for(self.recovery_boundary, token.kind)
                || !identifier_is_operand && self.is_strong_grammar_boundary(parser, context, token)
            {
                parser.report(
                    context,
                    ParserErrorType::ExpectedStatementExpression(
                        "expression operand",
                        Some(token.kind),
                    ),
                    Some(token),
                );
                self.push_error(parser, context, Some(token));
                return self.finish(parser, context);
            }
            if token.kind == TokenType::Keyword(KeywordTokenType::Sizeof) {
                self.phase = ExpressionPhase::SizeofStart(token.source_vectors);
                return ParseAction::Consume;
            }
            if let Some((operator, child_mode)) = prefix_operator(token.kind) {
                self.phase =
                    ExpressionPhase::PushPrefix(operator, token.source_vectors, child_mode);
                return ParseAction::Consume;
            }
            if token.kind == TokenType::Operator(OperatorTokenType::OpeningParenthesis) {
                let type_name_use = if self.mode == ExpressionMode::UnaryExpression {
                    parser
                        .cursor
                        .following(context)
                        .is_some_and(|following| parser.type_name_starter(following))
                        .then_some(TypeNameUse::UnaryCompoundLiteral)
                } else {
                    parser
                        .cursor
                        .following(context)
                        .is_some_and(|following| parser.type_name_starter(following))
                        .then_some(TypeNameUse::Cast)
                };
                if let Some(type_name_use) = type_name_use {
                    self.phase = ExpressionPhase::PushTypeName(token.source_vectors, type_name_use);
                    return ParseAction::Consume;
                }
                self.phase = ExpressionPhase::PushGrouped(token.source_vectors);
                return ParseAction::Consume;
            }
            let kind = match token.kind {
                | TokenType::Identifier =>
                    ExpressionType::Identifier(Identifier::from_token(token)),
                | TokenType::Integer(value) => ExpressionType::Constant(Constant::Integer(value)),
                | TokenType::Float(value) => ExpressionType::Constant(Constant::Float(value)),
                | TokenType::Character(value) => ExpressionType::Constant(Constant::Char(value)),
                | TokenType::String(value) => ExpressionType::StringLiteral(value),
                | _ => {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedStatementExpression(
                            "expression",
                            Some(token.kind),
                        ),
                        Some(token),
                    );
                    if token.kind == TokenType::Operator(OperatorTokenType::OpeningCurlyBrace) {
                        self.phase =
                            ExpressionPhase::RecoverUnexpectedBrace(1, token.source_vectors);
                        return ParseAction::Consume;
                    }
                    self.push_error(parser, context, Some(token));
                    return ParseAction::Consume;
                },
            };
            let index = parser.store_expression(
                kind,
                token.source_vectors,
                None,
                parser.hard_error_count > self.starting_error_count,
            );
            self.push_operand(index, true, true);
            ParseAction::Consume
        } else {
            let Some(token) = token else {
                return self.finish(parser, context);
            };
            if is_postfix_starter(token.kind)
                && !self
                    .operands
                    .last()
                    .is_some_and(|operand| operand.postfix_expression)
            {
                parser.report(
                    context,
                    ParserErrorType::ExpectedStatementExpression(
                        "operator after a non-postfix expression",
                        Some(token.kind),
                    ),
                    Some(token),
                );
                return self.finish(parser, context);
            }
            match token.kind {
                | TokenType::Operator(OperatorTokenType::OpeningSquareBracket) => {
                    let base = self.pop_operand().index;
                    self.phase = ExpressionPhase::PushSubscript(base, token.source_vectors);
                    return ParseAction::Consume;
                },
                | TokenType::Operator(OperatorTokenType::OpeningParenthesis) => {
                    let base = self.pop_operand().index;
                    if let Some(call) = &mut self.call {
                        call.arguments.clear();
                        call.source_vectors.clear();
                    }
                    self.phase = ExpressionPhase::CallStart(base, token.source_vectors);
                    return ParseAction::Consume;
                },
                | TokenType::Operator(OperatorTokenType::Period | OperatorTokenType::Arrow) => {
                    let base = self.pop_operand().index;
                    self.phase = ExpressionPhase::ExpectMember(
                        base,
                        token.kind == TokenType::Operator(OperatorTokenType::Arrow),
                        token.source_vectors,
                    );
                    return ParseAction::Consume;
                },
                | TokenType::Operator(
                    OperatorTokenType::PlusPlus | OperatorTokenType::MinusMinus,
                ) => {
                    let base = self.pop_operand().index;
                    let source = context.merge_vectors(
                        parser.syntax.expressions[base.0 as usize].source_vectors,
                        token.source_vectors,
                    );
                    let operator = if token.kind == TokenType::Operator(OperatorTokenType::PlusPlus)
                    {
                        UnaryOperator::PostIncrement
                    } else {
                        UnaryOperator::PostDecrement
                    };
                    let index = parser.store_expression(
                        ExpressionType::Unary {
                            operator,
                            operand_expression: base,
                        },
                        source,
                        Some(token.source_vectors),
                        parser.hard_error_count > self.starting_error_count,
                    );
                    self.push_operand(index, true, true);
                    return ParseAction::Consume;
                },
                | _ => {},
            }
            if token.kind == TokenType::Operator(OperatorTokenType::QuestionMark)
                && !matches!(
                    self.mode,
                    ExpressionMode::CastExpression | ExpressionMode::UnaryExpression
                )
            {
                while self.operators.last().is_some_and(|operator| {
                    !matches!(operator, LanguageExpressionOperator::Question { .. })
                        && operator.precedence() < 13
                }) {
                    self.reduce_one(parser, context);
                }
                self.operators.push(LanguageExpressionOperator::Question {
                    source_vectors: token.source_vectors,
                });
                self.phase = ExpressionPhase::PushConditionalMiddle;
                return ParseAction::Consume;
            }
            if let Some(operator) = binary_operator(token.kind)
                && !(operator == BinaryOperator::Comma && self.mode != ExpressionMode::Expression)
                && !(is_assignment_operator(operator)
                    && self.mode == ExpressionMode::ConstantExpression)
                && !matches!(
                    self.mode,
                    ExpressionMode::CastExpression | ExpressionMode::UnaryExpression
                )
            {
                let incoming = LanguageExpressionOperator::Binary {
                    operator,
                    source_vectors: token.source_vectors,
                };
                while self.operators.last().is_some_and(|stacked| {
                    !matches!(stacked, LanguageExpressionOperator::Question { .. })
                        && stacked.has_precedence_over(incoming)
                }) {
                    self.reduce_one(parser, context);
                }
                if is_assignment_operator(operator)
                    && !self
                        .operands
                        .last()
                        .is_some_and(|operand| operand.unary_expression)
                {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedStatementExpression(
                            "unary-expression left operand of assignment",
                            Some(token.kind),
                        ),
                        Some(token),
                    );
                }
                self.operators.push(incoming);
                self.state = ExpressionParserState::Operand;
                return ParseAction::Consume;
            }
            parser.report(
                context,
                ParserErrorType::ExpectedStatementExpression(
                    "operator in expression",
                    Some(token.kind),
                ),
                Some(token),
            );
            self.finish(parser, context)
        }
    }

    fn push_operand(
        &mut self,
        index: ExpressionIndex,
        unary_expression: bool,
        postfix_expression: bool,
    ) {
        self.operands.push(ExpressionOperand {
            index,
            unary_expression,
            postfix_expression,
        });
        self.state = ExpressionParserState::Operator;
    }

    fn pop_operand(&mut self) -> ExpressionOperand {
        self.operands
            .pop()
            .expect("operator state has an expression operand")
    }

    fn pop_operand_or_error(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
    ) -> ExpressionOperand {
        if let Some(operand) = self.operands.pop() {
            return operand;
        }
        self.push_error(parser, context, None);
        self.operands
            .pop()
            .expect("error expression supplies one operand")
    }

    fn call_state(&mut self) -> &mut CallState {
        self.call.get_or_insert_with(Box::default)
    }

    fn merge_call_source(&mut self, source: SourceVectors) {
        self.call_state().source_vectors.push(source);
    }

    fn merge_call_operator_source(&mut self, source: SourceVectors) {
        self.call_state().operator_sources.push(source);
    }

    fn finish_call(&mut self, parser: &mut Parser, context: &mut Context, base: ExpressionIndex) {
        let call = self.call.as_mut().expect("a call is being finished");
        let start = parser.syntax.expression_indices.len().to_u32();
        parser.append_syntax(|syntax| &mut syntax.expression_indices, &mut call.arguments);
        let arguments = SyntaxList::new(
            parser.syntax_id,
            start,
            parser.syntax.expression_indices.len().to_u32(),
        );
        let index = parser.store_expression(
            ExpressionType::Call {
                function_expression: base,
                arguments,
            },
            context.merge_vector_list(&call.source_vectors),
            Some(context.merge_vector_list(&call.operator_sources)),
            parser.hard_error_count > self.starting_error_count,
        );
        call.source_vectors.clear();
        call.operator_sources.clear();
        self.push_operand(index, true, true);
    }

    fn reduce_one(&mut self, parser: &mut Parser, context: &mut Context) {
        let operator = self
            .operators
            .pop()
            .expect("a pending expression operator exists");
        match operator {
            | LanguageExpressionOperator::Binary {
                operator,
                source_vectors,
            } => {
                let right = self.pop_operand_or_error(parser, context).index;
                let left = self.pop_operand_or_error(parser, context).index;
                let source = context.merge_vectors(
                    parser.syntax.expressions[left.0 as usize].source_vectors,
                    source_vectors,
                );
                let source = context.merge_vectors(
                    source,
                    parser.syntax.expressions[right.0 as usize].source_vectors,
                );
                let index = parser.store_expression(
                    ExpressionType::Binary {
                        operator,
                        left_expression: left,
                        right_expression: right,
                    },
                    source,
                    Some(source_vectors),
                    parser.hard_error_count > self.starting_error_count,
                );
                self.push_operand(index, false, false);
            },
            | LanguageExpressionOperator::Question { .. }
            | LanguageExpressionOperator::Conditional { .. } =>
                panic!("conditional markers reduce only after their child frames return"),
        }
    }

    fn is_boundary(&self, token: Option<TokenType>) -> bool {
        let Some(token) = token else {
            return true;
        };
        if matches!(
            self.mode,
            ExpressionMode::CastExpression | ExpressionMode::UnaryExpression
        ) && !is_postfix_starter(token)
        {
            return true;
        }
        if self.mode == ExpressionMode::AssignmentExpression
            && token == TokenType::Operator(OperatorTokenType::Comma)
        {
            return true;
        }
        if self.mode == ExpressionMode::ConstantExpression
            && matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma
                        | OperatorTokenType::Equals
                        | OperatorTokenType::PlusEquals
                        | OperatorTokenType::MinusEquals
                        | OperatorTokenType::AsteriskEquals
                        | OperatorTokenType::ForwardSlashEquals
                        | OperatorTokenType::PercentEquals
                        | OperatorTokenType::LessThanLessThanEquals
                        | OperatorTokenType::GreaterThanGreaterThanEquals
                        | OperatorTokenType::AmpersandEquals
                        | OperatorTokenType::CaretEquals
                        | OperatorTokenType::PipeEquals
                )
            )
        {
            return true;
        }
        if self.boundary == ExpressionBoundary::Argument
            && is_expression_operand_starter(token)
            && !is_postfix_starter(token)
            && binary_operator(token).is_none()
        {
            return true;
        }
        self.is_owning_boundary(token)
    }

    fn is_owning_boundary(&self, token: TokenType) -> bool {
        Self::is_owning_boundary_for(self.boundary, token)
    }

    fn is_owning_boundary_for(boundary: ExpressionBoundary, token: TokenType) -> bool {
        if Self::is_closing_delimiter(token) {
            return true;
        }
        match boundary {
            | ExpressionBoundary::Statement(
                ExpressionTerminator::Semicolon | ExpressionTerminator::ForSemicolon,
            ) => token == TokenType::Operator(OperatorTokenType::Semicolon),
            | ExpressionBoundary::Statement(ExpressionTerminator::ClosingParenthesis)
            | ExpressionBoundary::ClosingParenthesis =>
                token == TokenType::Operator(OperatorTokenType::ClosingParenthesis),
            | ExpressionBoundary::Statement(ExpressionTerminator::Colon) =>
                token == TokenType::Operator(OperatorTokenType::Colon),
            | ExpressionBoundary::ClosingSquareBracket
            | ExpressionBoundary::ArrayBound
            | ExpressionBoundary::Designator =>
                token == TokenType::Operator(OperatorTokenType::ClosingSquareBracket),
            | ExpressionBoundary::Argument => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma | OperatorTokenType::ClosingParenthesis
                )
            ),
            | ExpressionBoundary::Initializer => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma
                        | OperatorTokenType::ClosingCurlyBrace
                        | OperatorTokenType::Semicolon
                )
            ),
            | ExpressionBoundary::StructMember => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma
                        | OperatorTokenType::Semicolon
                        | OperatorTokenType::ClosingCurlyBrace
                )
            ),
            | ExpressionBoundary::Enumerator => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma | OperatorTokenType::ClosingCurlyBrace
                )
            ),
        }
    }

    fn is_closing_delimiter(token: TokenType) -> bool {
        matches!(
            token,
            TokenType::Operator(
                OperatorTokenType::ClosingParenthesis
                    | OperatorTokenType::ClosingSquareBracket
                    | OperatorTokenType::ClosingCurlyBrace
            )
        )
    }

    fn should_consume_unowned_closer(&self, token: TokenType) -> bool {
        match self.boundary {
            | ExpressionBoundary::Statement(ExpressionTerminator::Semicolon) => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::ClosingParenthesis | OperatorTokenType::ClosingSquareBracket
                )
            ),
            | ExpressionBoundary::Statement(
                ExpressionTerminator::ForSemicolon | ExpressionTerminator::ClosingParenthesis,
            ) => token == TokenType::Operator(OperatorTokenType::ClosingSquareBracket),
            | ExpressionBoundary::Statement(ExpressionTerminator::Colon)
            | ExpressionBoundary::ClosingParenthesis
            | ExpressionBoundary::ClosingSquareBracket
            | ExpressionBoundary::Argument
            | ExpressionBoundary::Initializer
            | ExpressionBoundary::ArrayBound
            | ExpressionBoundary::StructMember
            | ExpressionBoundary::Enumerator
            | ExpressionBoundary::Designator => false,
        }
    }

    fn is_strong_grammar_boundary(
        &self,
        parser: &mut Parser,
        context: &mut Context,
        token: Token,
    ) -> bool {
        let identifier_precedes_conditional_colon = self.boundary
            == ExpressionBoundary::Statement(ExpressionTerminator::Colon)
            && token.kind == TokenType::Identifier
            && parser.cursor.following(context).is_some_and(|following| {
                following.kind == TokenType::Operator(OperatorTokenType::Colon)
            });
        token.kind == TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
            && matches!(self.recovery_boundary, ExpressionBoundary::Statement(_))
            || Self::is_strong_grammar_boundary_for(parser, context, token, self.boundary)
            || self.recovery_boundary != self.boundary
                && !identifier_precedes_conditional_colon
                && Self::is_strong_grammar_boundary_for(
                    parser,
                    context,
                    token,
                    self.recovery_boundary,
                )
    }

    pub(super) fn is_strong_grammar_boundary_for(
        parser: &mut Parser,
        context: &mut Context,
        token: Token,
        boundary: ExpressionBoundary,
    ) -> bool {
        let identifier_continues_as_postfix = token.kind == TokenType::Identifier
            && parser
                .cursor
                .following(context)
                .is_some_and(|following| is_postfix_starter(following.kind));
        let declaration_starts_here = parser.declaration_recovery_starts_here(context, token);
        match boundary {
            | ExpressionBoundary::Initializer =>
                is_statement_keyword(token.kind)
                    || declaration_starts_here
                        && !matches!(
                            parser
                                .cursor
                                .following(context)
                                .map(|following| following.kind),
                            Some(TokenType::Operator(
                                OperatorTokenType::Comma
                                    | OperatorTokenType::Semicolon
                                    | OperatorTokenType::ClosingCurlyBrace
                            ))
                        ),
            | ExpressionBoundary::ArrayBound =>
                token.kind != TokenType::Identifier
                    && parser.declaration_starter(token)
                    && !matches!(
                        parser
                            .cursor
                            .following(context)
                            .map(|following| following.kind),
                        Some(TokenType::Operator(OperatorTokenType::ClosingSquareBracket))
                    ),
            | ExpressionBoundary::StructMember =>
                token.kind != TokenType::Identifier
                    && parser.declaration_starter(token)
                    && !matches!(
                        parser
                            .cursor
                            .following(context)
                            .map(|following| following.kind),
                        Some(TokenType::Operator(
                            OperatorTokenType::Comma
                                | OperatorTokenType::Semicolon
                                | OperatorTokenType::ClosingCurlyBrace
                        ))
                    ),
            | ExpressionBoundary::Enumerator =>
                token.kind != TokenType::Identifier
                    && parser.declaration_starter(token)
                    && !matches!(
                        parser
                            .cursor
                            .following(context)
                            .map(|following| following.kind),
                        Some(TokenType::Operator(
                            OperatorTokenType::Comma | OperatorTokenType::ClosingCurlyBrace
                        ))
                    ),
            | ExpressionBoundary::Statement(_) | ExpressionBoundary::ClosingParenthesis =>
                declaration_starts_here && !identifier_continues_as_postfix
                    || is_statement_keyword(token.kind)
                    || matches!(
                        boundary,
                        ExpressionBoundary::Statement(ExpressionTerminator::Semicolon)
                    ) && token.kind == TokenType::Identifier
                        && parser.cursor.following(context).is_some_and(|following| {
                            following.kind == TokenType::Operator(OperatorTokenType::Colon)
                        }),
            | ExpressionBoundary::ClosingSquareBracket
            | ExpressionBoundary::Argument
            | ExpressionBoundary::Designator => false,
        }
    }

    fn push_error(&mut self, parser: &mut Parser, context: &mut Context, anchor: Option<Token>) {
        let anchor_vectors = anchor.map(|token| {
            context
                .get_source_vectors(token.source_vectors)
                .iter()
                .map(|source| (source.position(context), source.source_file_index))
                .collect::<Vec<_>>()
        });
        let source_vectors = match anchor_vectors.filter(|vectors| !vectors.is_empty()) {
            | Some(vectors) => vectors.into_iter().fold(
                SourceVectors::default(),
                |combined, (position, source_file_index)| {
                    let anchor = context.create_source_vectors(position, source_file_index, 0);
                    context.merge_vectors(combined, anchor)
                },
            ),
            | None => context.create_source_vectors(
                parser.position(context),
                parser.source_file_index(),
                0,
            ),
        };
        self.push_error_with_source(parser, source_vectors, None);
    }

    fn push_error_with_source(
        &mut self,
        parser: &mut Parser,
        source_vectors: SourceVectors,
        operator_source_vectors: Option<SourceVectors>,
    ) {
        let index = parser.store_expression(
            ExpressionType::Error,
            source_vectors,
            operator_source_vectors,
            true,
        );
        self.operands.push(ExpressionOperand {
            index,
            unary_expression: false,
            postfix_expression: false,
        });
        self.state = ExpressionParserState::Operator;
    }

    fn finish(&mut self, parser: &mut Parser, context: &mut Context) -> ParseAction {
        while !self.operators.is_empty() {
            self.reduce_one(parser, context);
        }
        let operand = self.operands.pop().unwrap_or_else(|| {
            self.push_error(parser, context, None);
            self.operands
                .pop()
                .expect("error expression supplies one operand")
        });
        let recovered = parser.hard_error_count > self.starting_error_count;
        parser.syntax.expressions[operand.index.0 as usize].recovered |= recovered;
        if self.mode == ExpressionMode::ConstantExpression {
            ParseAction::Reduce(ParseValue::ConstantExpression(ConstantExpressionResult {
                index: ConstantExpressionIndex(operand.index.0, operand.index.1),
                recovered,
            }))
        } else {
            ParseAction::Reduce(ParseValue::Expression(ExpressionResult {
                index: operand.index,
                recovered,
            }))
        }
    }
}
