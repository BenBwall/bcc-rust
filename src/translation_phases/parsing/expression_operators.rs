//! Operator classification and precedence tables for expressions.
//!
//! Translation phase 7 syntax analysis (§5.1.1.2, p. 10; PDF p. 22). C99
//! states operator precedence only through the grammar of §6.5: precedence
//! follows the order of its major subclauses, highest first, and each
//! subclause's syntax gives its associativity (§6.5 paragraph 3 and
//! footnote 74, p. 67; PDF p. 79). The tables below flatten that grammar
//! (§6.5.1-§6.5.17, pp. 69-94; PDF pp. 81-106; §A.2.1, pp. 409-411;
//! PDF pp. 421-423) for the expression frame's Double-E reducer. Levels 1
//! and 2 (postfix, unary, and cast) are recognized by the frame itself.

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
    SourceVectors,
    preprocessing::{
        KeywordTokenType,
        OperatorTokenType,
        Token,
        TokenType,
    },
};

/// A pending operator on the expression frame's operator stack.
///
/// `Question` marks a `?` whose middle operand is still being parsed;
/// `Conditional` holds the finished middle operand of
/// `logical-OR-expression ? expression : conditional-expression`.
/// C99: §6.5.15 paragraph 1, p. 90; PDF p. 102.
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
    /// Binding level, lower binding tighter. The conditional operator is
    /// level 13, between `logical-OR-expression` and
    /// `assignment-expression`.
    /// C99: §6.5.15, p. 90; PDF p. 102.
    pub(super) fn precedence(self) -> u32 {
        match self {
            | Self::Binary { operator, .. } => binary_operator_precedence(operator),
            | Self::Question { .. } | Self::Conditional { .. } => 13,
        }
    }

    /// Assignment and the conditional operator group right to left: their
    /// right operands are `assignment-expression` and
    /// `conditional-expression` again.
    /// C99: §6.5.15 paragraph 1, p. 90; PDF p. 102; §6.5.16 paragraph 1,
    /// p. 91; PDF p. 103.
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

/// Binding level of each binary operator, lower binding tighter, numbered
/// in the order of §6.5's subclauses (footnote 74, p. 67; PDF p. 79).
///
/// C99: level 3 `multiplicative-expression` §6.5.5, p. 82; PDF p. 94;
/// 4 `additive-expression` §6.5.6, p. 82; PDF p. 94; 5 `shift-expression`
/// §6.5.7, p. 84; PDF p. 96; 6 `relational-expression` §6.5.8, p. 85;
/// PDF p. 97; 7 `equality-expression` §6.5.9, p. 86; PDF p. 98;
/// 8 `AND-expression` §6.5.10, p. 87; PDF p. 99; 9 `exclusive-OR-expression`
/// §6.5.11, p. 88; PDF p. 100; 10 `inclusive-OR-expression` §6.5.12, p. 88;
/// PDF p. 100; 11 `logical-AND-expression` §6.5.13, p. 89; PDF p. 101;
/// 12 `logical-OR-expression` §6.5.14, p. 89; PDF p. 101;
/// 14 `assignment-expression` §6.5.16, p. 91; PDF p. 103; 15 `expression`
/// (comma) §6.5.17, p. 94; PDF p. 106. Subscripting is a postfix operator
/// (level 1, §6.5.2, p. 69; PDF p. 81) that the frame builds directly.
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

/// Maps a punctuator to the binary, assignment, or comma operator it spells.
///
/// C99: operators of §6.5.5-§6.5.14, pp. 82-89; PDF pp. 94-101;
/// `assignment-operator` §6.5.16 paragraph 1, p. 91; PDF p. 103; comma
/// §6.5.17 paragraph 1, p. 94; PDF p. 106.
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

/// Maps a prefix punctuator to its unary operator and the operand mode the
/// grammar requires: `++` and `--` take a `unary-expression`, while each
/// `unary-operator` takes a `cast-expression`.
///
/// C99: §6.5.3 paragraph 1, p. 78; PDF p. 90.
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

/// Returns whether a token can begin a `unary-expression`: a
/// `primary-expression` token, `sizeof`, `(`, or a prefix operator.
///
/// C99: §6.5.1 paragraph 1, p. 69; PDF p. 81; §6.5.3 paragraph 1, p. 78;
/// PDF p. 90.
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

/// Returns whether a token begins a `postfix-expression` suffix: `[`, `(`,
/// `.`, `->`, `++`, or `--`.
///
/// C99: §6.5.2 paragraph 1, p. 69; PDF p. 81.
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

/// Returns whether `*` is directly followed by `]`, the variable-length
/// array marker `[ * ]` of a `direct-declarator` or
/// `direct-abstract-declarator`, rather than a multiplication or
/// indirection.
///
/// C99: §6.7.5 paragraph 1, p. 114; PDF p. 126; §6.7.6 paragraph 1,
/// p. 122; PDF p. 134.
pub(super) fn is_array_pointer_marker(
    parser: &mut Parser<'_, '_, '_>,
    token: Option<Token>,
) -> bool {
    is_operator(token, OperatorTokenType::Asterisk)
        && is_operator(
            parser.cursor.following(),
            OperatorTokenType::ClosingSquareBracket,
        )
}

/// Returns whether an operator is an `assignment-operator`.
///
/// C99: §6.5.16 paragraph 1, p. 91; PDF p. 103.
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
