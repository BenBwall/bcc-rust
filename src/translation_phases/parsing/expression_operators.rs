//! Operator classification and precedence tables for expressions.

use std::fmt::Debug;

use super::{
    Parser,
    expression::ExpressionMode,
    syntax::{
        BinaryOperator,
        Expression,
        UnaryOperator,
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

#[derive(Debug, Clone, Copy)]
pub(super) enum LanguageExpressionOperator<'tu> {
    Binary {
        operator:       BinaryOperator,
        source_vectors: SourceVectors,
    },
    Question {
        source_vectors: SourceVectors,
    },
    Conditional {
        middle:          &'tu Expression<'tu>,
        question_source: SourceVectors,
        colon_source:    Option<SourceVectors>,
    },
}

impl LanguageExpressionOperator<'_> {
    pub(super) fn precedence(self) -> u32 {
        match self {
            | Self::Binary { operator, .. } => binary_operator_precedence(operator),
            | Self::Question { .. } | Self::Conditional { .. } => 13,
        }
    }

    fn is_right_associative(self) -> bool {
        matches!(
            self,
            Self::Binary { operator, .. } if is_assignment_operator(operator)
        ) || matches!(self, Self::Question { .. } | Self::Conditional { .. })
    }

    pub(super) fn has_precedence_over(self, incoming: Self) -> bool {
        if incoming.is_right_associative() {
            self.precedence() < incoming.precedence()
        } else {
            self.precedence() <= incoming.precedence()
        }
    }
}

/// Tests whether an optional token is a particular C punctuator.
///
/// C99: punctuators are §6.4.6, pp. 63-64; PDF pp. 75-76.
pub(super) fn is_operator(token: Option<Token>, operator: OperatorTokenType) -> bool {
    token.is_some_and(|token| token.kind == TokenType::Operator(operator))
}

fn binary_operator_precedence(operator: BinaryOperator) -> u32 {
    match operator {
        | BinaryOperator::Multiplication | BinaryOperator::Division | BinaryOperator::Modulo => 3,
        | BinaryOperator::Addition | BinaryOperator::Subtraction => 4,
        | BinaryOperator::LeftShift | BinaryOperator::RightShift => 5,
        | BinaryOperator::LessThan
        | BinaryOperator::GreaterThan
        | BinaryOperator::LessThanOrEqual
        | BinaryOperator::GreaterThanOrEqual => 6,
        | BinaryOperator::Equal | BinaryOperator::NotEqual => 7,
        | BinaryOperator::BitwiseAnd => 8,
        | BinaryOperator::BitwiseXor => 9,
        | BinaryOperator::BitwiseOr => 10,
        | BinaryOperator::LogicalAnd => 11,
        | BinaryOperator::LogicalOr => 12,
        | BinaryOperator::Assignment
        | BinaryOperator::MultiplicationAssignment
        | BinaryOperator::DivisionAssignment
        | BinaryOperator::ModuloAssignment
        | BinaryOperator::AdditionAssignment
        | BinaryOperator::SubtractionAssignment
        | BinaryOperator::LeftShiftAssignment
        | BinaryOperator::RightShiftAssignment
        | BinaryOperator::BitwiseAndAssignment
        | BinaryOperator::BitwiseXorAssignment
        | BinaryOperator::BitwiseOrAssignment => 14,
        | BinaryOperator::Comma => 15,
        | BinaryOperator::Subscript => 1,
    }
}

pub(super) fn binary_operator(token: TokenType) -> Option<BinaryOperator> {
    let operator = match token {
        | TokenType::Operator(OperatorTokenType::Asterisk) => BinaryOperator::Multiplication,
        | TokenType::Operator(OperatorTokenType::ForwardSlash) => BinaryOperator::Division,
        | TokenType::Operator(OperatorTokenType::Percent) => BinaryOperator::Modulo,
        | TokenType::Operator(OperatorTokenType::Plus) => BinaryOperator::Addition,
        | TokenType::Operator(OperatorTokenType::Minus) => BinaryOperator::Subtraction,
        | TokenType::Operator(OperatorTokenType::LessThanLessThan) => BinaryOperator::LeftShift,
        | TokenType::Operator(OperatorTokenType::GreaterThanGreaterThan) =>
            BinaryOperator::RightShift,
        | TokenType::Operator(OperatorTokenType::LessThan) => BinaryOperator::LessThan,
        | TokenType::Operator(OperatorTokenType::GreaterThan) => BinaryOperator::GreaterThan,
        | TokenType::Operator(OperatorTokenType::LessThanEquals) => BinaryOperator::LessThanOrEqual,
        | TokenType::Operator(OperatorTokenType::GreaterThanEquals) =>
            BinaryOperator::GreaterThanOrEqual,
        | TokenType::Operator(OperatorTokenType::EqualsEquals) => BinaryOperator::Equal,
        | TokenType::Operator(OperatorTokenType::ExclamationMarkEquals) => BinaryOperator::NotEqual,
        | TokenType::Operator(OperatorTokenType::Ampersand) => BinaryOperator::BitwiseAnd,
        | TokenType::Operator(OperatorTokenType::Caret) => BinaryOperator::BitwiseXor,
        | TokenType::Operator(OperatorTokenType::Pipe) => BinaryOperator::BitwiseOr,
        | TokenType::Operator(OperatorTokenType::AmpersandAmpersand) => BinaryOperator::LogicalAnd,
        | TokenType::Operator(OperatorTokenType::PipePipe) => BinaryOperator::LogicalOr,
        | TokenType::Operator(OperatorTokenType::Comma) => BinaryOperator::Comma,
        | TokenType::Operator(OperatorTokenType::Equals) => BinaryOperator::Assignment,
        | TokenType::Operator(OperatorTokenType::AsteriskEquals) =>
            BinaryOperator::MultiplicationAssignment,
        | TokenType::Operator(OperatorTokenType::ForwardSlashEquals) =>
            BinaryOperator::DivisionAssignment,
        | TokenType::Operator(OperatorTokenType::PercentEquals) => BinaryOperator::ModuloAssignment,
        | TokenType::Operator(OperatorTokenType::PlusEquals) => BinaryOperator::AdditionAssignment,
        | TokenType::Operator(OperatorTokenType::MinusEquals) =>
            BinaryOperator::SubtractionAssignment,
        | TokenType::Operator(OperatorTokenType::LessThanLessThanEquals) =>
            BinaryOperator::LeftShiftAssignment,
        | TokenType::Operator(OperatorTokenType::GreaterThanGreaterThanEquals) =>
            BinaryOperator::RightShiftAssignment,
        | TokenType::Operator(OperatorTokenType::AmpersandEquals) =>
            BinaryOperator::BitwiseAndAssignment,
        | TokenType::Operator(OperatorTokenType::CaretEquals) =>
            BinaryOperator::BitwiseXorAssignment,
        | TokenType::Operator(OperatorTokenType::PipeEquals) => BinaryOperator::BitwiseOrAssignment,
        | _ => return None,
    };
    Some(operator)
}

pub(super) fn prefix_operator(token: TokenType) -> Option<(UnaryOperator, ExpressionMode)> {
    let (operator, mode) = match token {
        | TokenType::Operator(OperatorTokenType::PlusPlus) =>
            (UnaryOperator::PreIncrement, ExpressionMode::UnaryExpression),
        | TokenType::Operator(OperatorTokenType::MinusMinus) =>
            (UnaryOperator::PreDecrement, ExpressionMode::UnaryExpression),
        | TokenType::Operator(OperatorTokenType::Ampersand) =>
            (UnaryOperator::AddressOf, ExpressionMode::CastExpression),
        | TokenType::Operator(OperatorTokenType::Asterisk) =>
            (UnaryOperator::Indirection, ExpressionMode::CastExpression),
        | TokenType::Operator(OperatorTokenType::Plus) =>
            (UnaryOperator::Plus, ExpressionMode::CastExpression),
        | TokenType::Operator(OperatorTokenType::Minus) =>
            (UnaryOperator::Minus, ExpressionMode::CastExpression),
        | TokenType::Operator(OperatorTokenType::Tilde) =>
            (UnaryOperator::BitwiseNot, ExpressionMode::CastExpression),
        | TokenType::Operator(OperatorTokenType::ExclamationMark) =>
            (UnaryOperator::LogicalNot, ExpressionMode::CastExpression),
        | _ => return None,
    };
    Some((operator, mode))
}

pub(super) fn is_expression_operand_starter(token: TokenType) -> bool {
    matches!(
        token,
        TokenType::Identifier
            | TokenType::Integer(_)
            | TokenType::Float(_)
            | TokenType::Character(_)
            | TokenType::String(_)
            | TokenType::Keyword(KeywordTokenType::Sizeof)
            | TokenType::Operator(OperatorTokenType::OpeningParenthesis)
    ) || prefix_operator(token).is_some()
}

pub(super) fn is_postfix_starter(token: TokenType) -> bool {
    matches!(
        token,
        TokenType::Operator(
            OperatorTokenType::OpeningSquareBracket
                | OperatorTokenType::OpeningParenthesis
                | OperatorTokenType::Period
                | OperatorTokenType::Arrow
                | OperatorTokenType::PlusPlus
                | OperatorTokenType::MinusMinus
        )
    )
}

pub(super) fn is_array_pointer_marker(
    parser: &mut Parser<'_, '_>,
    context: &mut Context<'_>,
    token: Option<Token>,
) -> bool {
    is_operator(token, OperatorTokenType::Asterisk)
        && is_operator(
            parser.cursor.following(context),
            OperatorTokenType::ClosingSquareBracket,
        )
}

pub(super) fn is_assignment_operator(operator: BinaryOperator) -> bool {
    matches!(
        operator,
        BinaryOperator::Assignment
            | BinaryOperator::MultiplicationAssignment
            | BinaryOperator::DivisionAssignment
            | BinaryOperator::ModuloAssignment
            | BinaryOperator::AdditionAssignment
            | BinaryOperator::SubtractionAssignment
            | BinaryOperator::LeftShiftAssignment
            | BinaryOperator::RightShiftAssignment
            | BinaryOperator::BitwiseAndAssignment
            | BinaryOperator::BitwiseXorAssignment
            | BinaryOperator::BitwiseOrAssignment
    )
}
