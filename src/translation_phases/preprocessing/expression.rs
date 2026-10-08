//! Evaluation of `#if` and `#elif` controlling expressions.
//!
//! C99: conditional inclusion, §6.10.1 paragraphs 1-4, pp. 147-148; PDF
//! pp. 159-160, with the constant-expression rules of §6.6 paragraphs 3-6,
//! p. 95; PDF p. 107. Macros are replaced first, except the operand of
//! `defined`; remaining identifiers, keywords included, become 0; and the
//! arithmetic is that of `intmax_t` and `uintmax_t` (§6.10.1 paragraph 4,
//! p. 148; PDF p. 160), here `i64` and `u64`.
//!
//! The operators and their grouping are those of §6.5, pp. 67-94; PDF
//! pp. 79-106, which a Double-E reducer evaluates directly. Operands are
//! converted as by the usual arithmetic conversions (§6.3.1.8 paragraph 1,
//! pp. 44-45; PDF pp. 56-57): one unsigned operand makes the operation
//! unsigned.
//!
//! Behavior C99 leaves open: a `defined` produced by macro replacement is
//! undefined (§6.10.1 paragraph 4) and is evaluated like a written one, as
//! GCC does. An evaluated comma operator (§6.6 paragraph 3) is an extension
//! under the extension policy.

use std::{
    fmt::Debug,
    mem::replace,
    num::NonZeroU32,
    ops::ControlFlow,
};

use super::{
    Expander,
    errors::{
        PreprocessorError,
        PreprocessorErrorType,
    },
    token::{
        IntegerTokenType,
        TokenType,
    },
    token_conversion::IntegerRepresentation,
};
use crate::{
    configuration::{
        CStandard,
        ExtensionPolicy,
    },
    translation_phases::{
        Context,
        SourceVectors,
        preprocessor_tokenizer::{
            PreprocessorToken,
            PreprocessorTokenType,
        },
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum PreprocessorExpressionParserState {
    Unary,
    Binary,
}

/// The operators an `#if` expression can evaluate.
///
/// C99: §6.10.1 paragraph 1, p. 147; PDF p. 159, and §6.6 paragraphs 3 and
/// 6, p. 95; PDF p. 107. Casts, `sizeof`, assignment, increment and
/// decrement, function calls, and the address and member operators have no
/// place in it; `defined` is read separately.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum PreprocessorExpressionOperator {
    // Unary
    UnaryPlus,
    UnaryMinus,
    BitwiseNot,
    LogicalNot,

    // Binary
    BinaryPlus,
    BinaryMinus,
    Multiply,
    Divide,
    Modulo,
    LessThan,
    LessThanEquals,
    GreaterThan,
    GreaterThanEquals,
    Equals,
    NotEquals,
    LeftShift,
    RightShift,
    BitwiseAnd,
    BitwiseXor,
    BitwiseOr,
    LogicalAnd,
    LogicalOr,
    QuestionMark,
    Conditional,
    Comma,

    // Grouping
    OpeningParenthesis,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum PreprocessorExpressionAssociativity {
    Left,
    Right,
}

impl PreprocessorExpressionOperator {
    /// The diagnostic for a unary operator with no operand.
    fn unary_operand_missing<'tu>(self) -> Option<PreprocessorErrorType<'tu>> {
        match self {
            | Self::UnaryPlus => Some(PreprocessorErrorType::UnaryPlusWithoutOperand),
            | Self::UnaryMinus => Some(PreprocessorErrorType::UnaryMinusWithoutOperand),
            | Self::BitwiseNot => Some(PreprocessorErrorType::BitwiseNotWithoutOperand),
            | Self::LogicalNot => Some(PreprocessorErrorType::LogicalNotWithoutOperand),
            | _ => None,
        }
    }

    /// The diagnostic for an operand that is missing after this operator
    /// where the expression or a group ends.
    fn operand_missing_while_reading<'tu>(self) -> PreprocessorErrorType<'tu> {
        self.unary_operand_missing().unwrap_or(
            PreprocessorErrorType::ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(
                self,
            ),
        )
    }

    /// The diagnostic for an operand found missing when this operator is
    /// reduced.
    fn operand_missing_at_reduction<'tu>(self) -> PreprocessorErrorType<'tu> {
        if let Some(error) = self.unary_operand_missing() {
            return error;
        }
        match self {
            | Self::BinaryPlus => PreprocessorErrorType::BinaryPlusWithoutRhs,
            | Self::BinaryMinus => PreprocessorErrorType::BinaryMinusWithoutRhs,
            | Self::Multiply => PreprocessorErrorType::MultiplyWithoutRhs,
            | Self::Divide => PreprocessorErrorType::DivideWithoutRhs,
            | Self::Modulo => PreprocessorErrorType::ModuloWithoutRhs,
            | Self::LessThan => PreprocessorErrorType::LessThanWithoutRhs,
            | Self::LessThanEquals => PreprocessorErrorType::LessThanEqualsWithoutRhs,
            | Self::GreaterThan => PreprocessorErrorType::GreaterThanWithoutRhs,
            | Self::GreaterThanEquals => PreprocessorErrorType::GreaterThanEqualsWithoutRhs,
            | Self::Equals => PreprocessorErrorType::EqualsWithoutRhs,
            | Self::NotEquals => PreprocessorErrorType::NotEqualsWithoutRhs,
            | Self::LeftShift => PreprocessorErrorType::LeftShiftWithoutRhs,
            | Self::RightShift => PreprocessorErrorType::RightShiftWithoutRhs,
            | Self::BitwiseAnd => PreprocessorErrorType::BitwiseAndWithoutRhs,
            | Self::BitwiseXor => PreprocessorErrorType::BitwiseXorWithoutRhs,
            | Self::BitwiseOr => PreprocessorErrorType::BitwiseOrWithoutRhs,
            | Self::LogicalAnd => PreprocessorErrorType::LogicalAndWithoutRhs,
            | Self::LogicalOr => PreprocessorErrorType::LogicalOrWithoutRhs,
            | other => unreachable!("{other:?} takes no operands from the stack"),
        }
    }

    /// The operator as written in C source.
    pub(super) fn spelling(self) -> &'static str {
        match self {
            | Self::UnaryPlus | Self::BinaryPlus => "+",
            | Self::UnaryMinus | Self::BinaryMinus => "-",
            | Self::BitwiseNot => "~",
            | Self::LogicalNot => "!",
            | Self::Multiply => "*",
            | Self::Divide => "/",
            | Self::Modulo => "%",
            | Self::LessThan => "<",
            | Self::LessThanEquals => "<=",
            | Self::GreaterThan => ">",
            | Self::GreaterThanEquals => ">=",
            | Self::Equals => "==",
            | Self::NotEquals => "!=",
            | Self::LeftShift => "<<",
            | Self::RightShift => ">>",
            | Self::BitwiseAnd => "&",
            | Self::BitwiseXor => "^",
            | Self::BitwiseOr => "|",
            | Self::LogicalAnd => "&&",
            | Self::LogicalOr => "||",
            | Self::QuestionMark => "?",
            | Self::Conditional => ":",
            | Self::Comma => ",",
            | Self::OpeningParenthesis => "(",
        }
    }

    /// Binding strength, lower binding tighter.
    ///
    /// C99: the order of the subclauses of §6.5 gives operator precedence
    /// (§6.5 paragraph 3 and footnote 74, p. 67; PDF p. 79).
    fn precedence(self) -> u32 {
        match self {
            // Based on https://en.cppreference.com/w/c/language/operator_precedence.
            | Self::UnaryPlus | Self::UnaryMinus | Self::BitwiseNot | Self::LogicalNot => 2,
            | Self::Multiply | Self::Divide | Self::Modulo => 3,
            | Self::BinaryPlus | Self::BinaryMinus => 4,
            | Self::LeftShift | Self::RightShift => 5,
            | Self::LessThan
            | Self::LessThanEquals
            | Self::GreaterThan
            | Self::GreaterThanEquals => 6,
            | Self::Equals | Self::NotEquals => 7,
            | Self::BitwiseAnd => 8,
            | Self::BitwiseXor => 9,
            | Self::BitwiseOr => 10,
            | Self::LogicalAnd => 11,
            | Self::LogicalOr => 12,
            | Self::QuestionMark | Self::Conditional => 13,
            | Self::Comma => 14,
            // OpeningParenthesis is not a normal operator. We only pop it off the stack when we
            // encounter a closing parenthesis.
            | Self::OpeningParenthesis => u32::MAX,
        }
    }

    fn associativity(self) -> PreprocessorExpressionAssociativity {
        match self {
            | Self::QuestionMark
            | Self::Conditional
            | Self::UnaryPlus
            | Self::UnaryMinus
            | Self::BitwiseNot
            | Self::LogicalNot => PreprocessorExpressionAssociativity::Right,
            | Self::BinaryMinus
            | Self::BinaryPlus
            | Self::Multiply
            | Self::Divide
            | Self::Modulo
            | Self::LeftShift
            | Self::RightShift
            | Self::LessThan
            | Self::LessThanEquals
            | Self::GreaterThan
            | Self::GreaterThanEquals
            | Self::Equals
            | Self::NotEquals
            | Self::BitwiseAnd
            | Self::BitwiseOr
            | Self::BitwiseXor
            | Self::LogicalAnd
            | Self::LogicalOr
            | Self::Comma
            | Self::OpeningParenthesis => PreprocessorExpressionAssociativity::Left,
        }
    }

    fn has_precedence_over(self, other: Self) -> bool {
        match self.associativity() {
            | PreprocessorExpressionAssociativity::Left => self.precedence() <= other.precedence(),
            | PreprocessorExpressionAssociativity::Right => self.precedence() < other.precedence(),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
struct LocatedExpressionOperator {
    kind:           PreprocessorExpressionOperator,
    source_vectors: SourceVectors,
}

/// Arithmetic with no defined result: signed overflow, which leaves a
/// constant expression out of its type's range (C99: §6.6 paragraph 4,
/// p. 95; PDF p. 107); a zero divisor (§6.5.5 paragraph 5, p. 82; PDF
/// p. 94); and a shift count that is negative or at least the width
/// (§6.5.7 paragraph 3, p. 84; PDF p. 96).
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum ArithmeticFaultKind {
    UnaryMinusOverflow,
    BinaryPlusOverflow,
    BinaryMinusOverflow,
    MultiplyOverflow,
    DivideByZero,
    DivideOverflow,
    ModuloByZero,
    ModuloOverflow,
    LeftShiftOverflow,
    RightShiftOverflow,
}

impl ArithmeticFaultKind {
    fn error_type(self) -> PreprocessorErrorType<'static> {
        match self {
            | Self::UnaryMinusOverflow => PreprocessorErrorType::UnaryMinusOverflow,
            | Self::BinaryPlusOverflow => PreprocessorErrorType::BinaryPlusOverflow,
            | Self::BinaryMinusOverflow => PreprocessorErrorType::BinaryMinusOverflow,
            | Self::MultiplyOverflow => PreprocessorErrorType::MultiplyOverflow,
            | Self::DivideByZero => PreprocessorErrorType::DivideByZero,
            | Self::DivideOverflow => PreprocessorErrorType::DivideOverflow,
            | Self::ModuloByZero => PreprocessorErrorType::ModuloByZero,
            | Self::ModuloOverflow => PreprocessorErrorType::ModuloOverflow,
            | Self::LeftShiftOverflow => PreprocessorErrorType::LeftShiftOverflow,
            | Self::RightShiftOverflow => PreprocessorErrorType::RightShiftOverflow,
        }
    }
}

/// Only arithmetic that would diagnose allocates nodes. Operand roots let
/// short-circuit operators discard a whole unevaluated subtree in constant
/// time.
///
/// C99: an unevaluated operand of `&&`, `||`, or `?:` raises no fault
/// (§6.5.13 paragraph 4 and §6.5.14 paragraph 4, p. 89; PDF p. 101;
/// §6.5.15 paragraph 4, p. 90; PDF p. 102), and only an evaluated
/// subexpression is held to §6.6 paragraph 3, p. 95; PDF p. 107.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum ArithmeticFaultNode {
    Fault {
        kind:           ArithmeticFaultKind,
        source_vectors: SourceVectors,
    },
    Join {
        left:  NonZeroU32,
        right: NonZeroU32,
    },
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct PreprocessorExpressionParser<'pp> {
    operator_stack:   ArenaVec<'pp, LocatedExpressionOperator>,
    operand_stack:    PreprocessorExpressionOperandStack<'pp>,
    state:            PreprocessorExpressionParserState,
    open_parentheses: ArenaVec<'pp, usize>,
}

/// A value of an `#if` expression: every signed type acts as `intmax_t`
/// and every unsigned type as `uintmax_t`, both 64 bits here.
///
/// C99: §6.10.1 paragraph 4, p. 148; PDF p. 160.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum PreprocessorExpressionOperand {
    Signed(i64),
    Unsigned(u64),
}

impl PreprocessorExpressionOperand {
    /// The value whose bits are `bits`, unsigned or signed.
    fn of_type(is_unsigned: bool, bits: i64) -> Self {
        if is_unsigned {
            Self::Unsigned(bits as u64)
        } else {
            Self::Signed(bits)
        }
    }

    fn as_signed(self) -> i64 {
        match self {
            | Self::Signed(v) => v,
            | Self::Unsigned(v) => v as i64,
        }
    }

    fn as_unsigned(self) -> u64 {
        match self {
            | Self::Signed(v) => v as u64,
            | Self::Unsigned(v) => v,
        }
    }

    fn is_signed(self) -> bool {
        match self {
            | Self::Signed(_) => true,
            | Self::Unsigned(_) => false,
        }
    }

    fn is_unsigned(self) -> bool {
        match self {
            | Self::Signed(_) => false,
            | Self::Unsigned(_) => true,
        }
    }

    fn set_signed(self, value: i64) -> Self {
        match self {
            | Self::Signed(_) => Self::Signed(value),
            | Self::Unsigned(_) => Self::Unsigned(value as u64),
        }
    }

    fn map_signed(self, f: impl FnOnce(i64) -> i64) -> Self {
        match self {
            | Self::Signed(v) => Self::Signed(f(v)),
            | Self::Unsigned(v) => Self::Unsigned(f(v as i64) as u64),
        }
    }

    fn map_unsigned(self, f: impl FnOnce(u64) -> u64) -> Self {
        self.map_signed(|v| f(v as u64) as i64)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
struct EvaluatedPreprocessorExpressionOperand {
    value:                    PreprocessorExpressionOperand,
    contains_evaluated_comma: bool,
    arithmetic_faults:        Option<NonZeroU32>,
}

impl From<PreprocessorExpressionOperand> for EvaluatedPreprocessorExpressionOperand {
    fn from(value: PreprocessorExpressionOperand) -> Self {
        Self {
            value,
            contains_evaluated_comma: false,
            arithmetic_faults: None,
        }
    }
}

impl EvaluatedPreprocessorExpressionOperand {
    fn with_comma_liveness(
        value: PreprocessorExpressionOperand,
        contains_evaluated_comma: bool,
    ) -> Self {
        Self {
            value,
            contains_evaluated_comma,
            arithmetic_faults: None,
        }
    }

    fn as_signed(self) -> i64 {
        self.value.as_signed()
    }

    fn as_unsigned(self) -> u64 {
        self.value.as_unsigned()
    }

    fn is_signed(self) -> bool {
        self.value.is_signed()
    }

    fn is_unsigned(self) -> bool {
        self.value.is_unsigned()
    }

    fn set_signed(self, value: i64) -> Self {
        Self {
            value: self.value.set_signed(value),
            ..self
        }
    }

    fn map_unsigned(self, f: impl FnOnce(u64) -> u64) -> Self {
        Self {
            value: self.value.map_unsigned(f),
            ..self
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
struct PreprocessorExpressionOperandStack<'pp> {
    values:                    ArenaVec<'pp, EvaluatedPreprocessorExpressionOperand>,
    floor:                     usize,
    pending_evaluated_comma:   bool,
    pending_arithmetic_faults: Option<NonZeroU32>,
    arithmetic_faults:         ArenaVec<'pp, ArithmeticFaultNode>,
    /// Fault nodes waiting to be reported, kept for the next report.
    fault_scan:                ArenaVec<'pp, NonZeroU32>,
}

impl<'pp> PreprocessorExpressionOperandStack<'pp> {
    fn new(pp: &'pp Bump) -> Self {
        Self {
            values:                    ArenaVec::new_in(pp),
            floor:                     0,
            pending_evaluated_comma:   false,
            pending_arithmetic_faults: None,
            arithmetic_faults:         ArenaVec::new_in(pp),
            fault_scan:                ArenaVec::new_in(pp),
        }
    }

    fn clear(&mut self) {
        self.values.clear();
        self.floor = 0;
        self.pending_evaluated_comma = false;
        self.pending_arithmetic_faults = None;
        self.arithmetic_faults.clear();
    }

    fn is_empty(&self) -> bool {
        self.values.len() == self.floor
    }

    fn len(&self) -> usize {
        self.values.len() - self.floor
    }

    fn push(&mut self, operand: impl Into<EvaluatedPreprocessorExpressionOperand>) {
        let mut operand = operand.into();
        operand.contains_evaluated_comma |= self.pending_evaluated_comma;
        self.pending_evaluated_comma = false;
        let pending_faults = self.pending_arithmetic_faults.take();
        operand.arithmetic_faults = self.join_faults(pending_faults, operand.arithmetic_faults);
        self.values.push(operand);
    }

    fn pop(&mut self) -> Option<EvaluatedPreprocessorExpressionOperand> {
        if self.is_empty() {
            return None;
        }
        let mut operand = self.values.pop()?;
        self.pending_evaluated_comma |= operand.contains_evaluated_comma;
        // Ordinary reductions pop RHS before LHS. Prepend each transferred
        // root so diagnostics retain source evaluation order without
        // duplicates.
        self.pending_arithmetic_faults = self.join_faults(
            operand.arithmetic_faults.take(),
            self.pending_arithmetic_faults,
        );
        Some(operand)
    }

    fn pop_isolated(&mut self) -> Option<EvaluatedPreprocessorExpressionOperand> {
        if self.is_empty() {
            None
        } else {
            self.values.pop()
        }
    }

    fn add_fault_node(&mut self, node: ArithmeticFaultNode) -> NonZeroU32 {
        let id = self
            .arithmetic_faults
            .len()
            .checked_add(1)
            .and_then(|length| u32::try_from(length).ok())
            .and_then(NonZeroU32::new)
            .expect("expression fault arena exceeds u32::MAX nodes");
        self.arithmetic_faults.push(node);
        id
    }

    fn join_faults(
        &mut self,
        left: Option<NonZeroU32>,
        right: Option<NonZeroU32>,
    ) -> Option<NonZeroU32> {
        match (left, right) {
            | (Some(left), Some(right)) =>
                Some(self.add_fault_node(ArithmeticFaultNode::Join { left, right })),
            | (root, None) | (None, root) => root,
        }
    }

    fn retain_faults(&mut self, root: Option<NonZeroU32>) {
        self.pending_arithmetic_faults = self.join_faults(self.pending_arithmetic_faults, root);
    }

    fn record_fault(&mut self, kind: ArithmeticFaultKind, source_vectors: SourceVectors) {
        let root = self.add_fault_node(ArithmeticFaultNode::Fault {
            kind,
            source_vectors,
        });
        self.retain_faults(Some(root));
    }

    fn emit_faults(&mut self, context: &mut Context<'_>, root: Option<NonZeroU32>) {
        let Some(root) = root else {
            return;
        };
        let pending = &mut self.fault_scan;
        pending.push(root);
        while let Some(index) = pending.pop() {
            match self.arithmetic_faults[index.get() as usize - 1] {
                | ArithmeticFaultNode::Fault {
                    kind,
                    source_vectors,
                } => context.preprocessor_error(PreprocessorError {
                    error_type: kind.error_type(),
                    source_vectors,
                }),
                | ArithmeticFaultNode::Join { left, right } => {
                    pending.push(right);
                    pending.push(left);
                },
            }
        }
    }
}

impl<'pp> PreprocessorExpressionParser<'pp> {
    pub(super) fn new(pp: &'pp Bump) -> Self {
        Self {
            operator_stack:   ArenaVec::new_in(pp),
            operand_stack:    PreprocessorExpressionOperandStack::new(pp),
            state:            PreprocessorExpressionParserState::Unary,
            open_parentheses: ArenaVec::new_in(pp),
        }
    }

    fn reset(&mut self) {
        self.operator_stack.clear();
        self.operand_stack.clear();
        self.state = PreprocessorExpressionParserState::Unary;
        self.open_parentheses.clear();
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "Integer literal values are range-checked before narrowing."
)]
impl<'tu> Expander<'_, 'tu, '_, '_> {
    fn map_operator(&mut self, operator: PreprocessorToken) -> PreprocessorExpressionOperator {
        let state = replace(
            &mut self.expression_parser.state,
            PreprocessorExpressionParserState::Unary,
        );
        match operator.kind {
            | PreprocessorTokenType::Plus =>
                if state == PreprocessorExpressionParserState::Unary {
                    PreprocessorExpressionOperator::UnaryPlus
                } else {
                    PreprocessorExpressionOperator::BinaryPlus
                },
            | PreprocessorTokenType::Minus =>
                if state == PreprocessorExpressionParserState::Unary {
                    PreprocessorExpressionOperator::UnaryMinus
                } else {
                    PreprocessorExpressionOperator::BinaryMinus
                },
            | PreprocessorTokenType::Asterisk => PreprocessorExpressionOperator::Multiply,
            | PreprocessorTokenType::ForwardSlash => PreprocessorExpressionOperator::Divide,
            | PreprocessorTokenType::Percent => PreprocessorExpressionOperator::Modulo,
            | PreprocessorTokenType::LessThanLessThan => PreprocessorExpressionOperator::LeftShift,
            | PreprocessorTokenType::GreaterThanGreaterThan =>
                PreprocessorExpressionOperator::RightShift,
            | PreprocessorTokenType::LessThan => PreprocessorExpressionOperator::LessThan,
            | PreprocessorTokenType::LessThanEquals =>
                PreprocessorExpressionOperator::LessThanEquals,
            | PreprocessorTokenType::GreaterThan => PreprocessorExpressionOperator::GreaterThan,
            | PreprocessorTokenType::GreaterThanEquals =>
                PreprocessorExpressionOperator::GreaterThanEquals,
            | PreprocessorTokenType::EqualsEquals => PreprocessorExpressionOperator::Equals,
            | PreprocessorTokenType::ExclamationMarkEquals =>
                PreprocessorExpressionOperator::NotEquals,
            | PreprocessorTokenType::Ampersand => PreprocessorExpressionOperator::BitwiseAnd,
            | PreprocessorTokenType::Caret => PreprocessorExpressionOperator::BitwiseXor,
            | PreprocessorTokenType::Pipe => PreprocessorExpressionOperator::BitwiseOr,
            | PreprocessorTokenType::AmpersandAmpersand =>
                PreprocessorExpressionOperator::LogicalAnd,
            | PreprocessorTokenType::PipePipe => PreprocessorExpressionOperator::LogicalOr,
            | PreprocessorTokenType::QuestionMark => PreprocessorExpressionOperator::QuestionMark,
            | PreprocessorTokenType::Comma => PreprocessorExpressionOperator::Comma,
            | PreprocessorTokenType::Tilde => PreprocessorExpressionOperator::BitwiseNot,
            | PreprocessorTokenType::ExclamationMark => PreprocessorExpressionOperator::LogicalNot,
            | _ => unreachable!(),
        }
    }

    /// Applies `operator` to the operands on the stack.
    ///
    /// C99: the semantics of §6.5.3.3 through §6.5.17, pp. 79-94; PDF
    /// pp. 91-106. Unsigned arithmetic wraps and never overflows (§6.2.5
    /// paragraph 9, p. 34; PDF p. 46); comparisons and logical operators
    /// yield a signed 0 or 1.
    fn handle_expression_operator(&mut self, operator: LocatedExpressionOperator) {
        use PreprocessorExpressionOperator as Op;
        match operator.kind {
            | Op::UnaryPlus | Op::UnaryMinus | Op::BitwiseNot | Op::LogicalNot =>
                self.apply_unary_operator(operator),
            | Op::BinaryPlus
            | Op::BinaryMinus
            | Op::Multiply
            | Op::Divide
            | Op::Modulo
            | Op::LeftShift
            | Op::RightShift
            | Op::LessThan
            | Op::LessThanEquals
            | Op::GreaterThan
            | Op::GreaterThanEquals
            | Op::Equals
            | Op::NotEquals
            | Op::BitwiseAnd
            | Op::BitwiseXor
            | Op::BitwiseOr => self.apply_binary_operator(operator),
            | Op::LogicalAnd | Op::LogicalOr => self.apply_logical_operator(operator.kind),
            // C99 §6.6p3 forbids an evaluated comma operator; it is reported
            // once the whole expression is known (an extension under the
            // extension policy).
            | Op::Comma => {
                let Some(rhs) = self.expression_parser.operand_stack.pop_isolated() else {
                    return;
                };
                let Some(lhs) = self.expression_parser.operand_stack.pop_isolated() else {
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                self.expression_parser
                    .operand_stack
                    .retain_faults(lhs.arithmetic_faults);
                self.expression_parser
                    .operand_stack
                    .retain_faults(rhs.arithmetic_faults);
                self.expression_parser.operand_stack.push(
                    EvaluatedPreprocessorExpressionOperand::with_comma_liveness(rhs.value, true),
                );
            },
            | Op::QuestionMark => {
                self.context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::TernaryOperatorWithoutColon,
                    source_vectors: operator.source_vectors,
                });
                // Collapse this incomplete conditional within its group. Its
                // parent operands cannot participate in recovery.
                _ = self.expression_parser.operand_stack.pop();
                _ = self.expression_parser.operand_stack.pop();
                self.expression_parser
                    .operand_stack
                    .push(PreprocessorExpressionOperand::Signed(0));
            },
            | Op::Conditional => self.apply_conditional_operator(),
            | Op::OpeningParenthesis => {
                let source_vectors = operator.source_vectors;
                self.context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::UnterminatedOpeningParenthesisInPreprocessorExpression,
                    source_vectors,
                });
                self.finish_expression_group();
            },
        }
    }

    /// Reports that `operator`'s operand is missing as it is reduced.
    fn report_missing_operand(&mut self, operator: PreprocessorExpressionOperator) {
        let source_vectors = self.current_location();
        self.context.preprocessor_error(PreprocessorError {
            error_type: operator.operand_missing_at_reduction(),
            source_vectors,
        });
    }

    /// Applies a unary arithmetic operator.
    ///
    /// C99: §6.5.3.3 paragraphs 2-5, p. 79; PDF p. 91.
    fn apply_unary_operator(&mut self, operator: LocatedExpressionOperator) {
        use PreprocessorExpressionOperator as Op;
        // Unary `+` leaves its operand in place.
        if operator.kind == Op::UnaryPlus {
            if self.expression_parser.operand_stack.is_empty() {
                self.report_missing_operand(operator.kind);
            }
            return;
        }
        let Some(operand) = self.expression_parser.operand_stack.pop() else {
            self.report_missing_operand(operator.kind);
            return;
        };
        let result = match operator.kind {
            | Op::UnaryMinus => {
                let (new, did_overflow) = operand.as_signed().overflowing_neg();
                if operand.is_signed() && did_overflow {
                    self.expression_parser.operand_stack.record_fault(
                        ArithmeticFaultKind::UnaryMinusOverflow,
                        operator.source_vectors,
                    );
                }
                operand.set_signed(new)
            },
            | Op::BitwiseNot => operand.map_unsigned(|v| !v),
            | Op::LogicalNot =>
                PreprocessorExpressionOperand::Signed(i64::from(operand.as_signed() == 0)).into(),
            | other => unreachable!("{other:?} is not a unary operator"),
        };
        self.expression_parser.operand_stack.push(result);
    }

    /// Pops the two operands of a binary `operator`. A missing right operand
    /// reads as 0; a missing left one is reported and leaves the right one
    /// on the stack as the result. `isolated` pops keep each operand's
    /// faults and comma liveness with it rather than with the result.
    fn pop_binary_operands(
        &mut self,
        operator: PreprocessorExpressionOperator,
        isolated: bool,
    ) -> Option<(
        EvaluatedPreprocessorExpressionOperand,
        EvaluatedPreprocessorExpressionOperand,
    )> {
        let stack = &mut self.expression_parser.operand_stack;
        let mut pop = || {
            if isolated {
                stack.pop_isolated()
            } else {
                stack.pop()
            }
        };
        let rhs = pop().unwrap_or_else(|| PreprocessorExpressionOperand::Signed(0).into());
        let Some(lhs) = pop() else {
            self.report_missing_operand(operator);
            self.expression_parser.operand_stack.push(rhs);
            return None;
        };
        Some((lhs, rhs))
    }

    /// Applies a binary arithmetic, shift, relational, equality, or bitwise
    /// operator after the usual arithmetic conversions.
    ///
    /// C99: §6.5.5-§6.5.12, pp. 82-89; PDF pp. 94-101.
    fn apply_binary_operator(&mut self, operator: LocatedExpressionOperator) {
        use ArithmeticFaultKind as Fault;
        use PreprocessorExpressionOperand as Operand;
        use PreprocessorExpressionOperator as Op;
        let Some((lhs, rhs)) = self.pop_binary_operands(operator.kind, false) else {
            return;
        };
        let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
        let truth = |value: bool| (Operand::Signed(i64::from(value)), None);
        // Unsigned results never overflow (C99 §6.2.5p9).
        let arithmetic = |(new, did_overflow): (i64, bool), fault: Fault| {
            (
                Operand::of_type(is_unsigned, new),
                (did_overflow && !is_unsigned).then_some(fault),
            )
        };
        let (result, fault) = match operator.kind {
            | Op::BinaryPlus => arithmetic(
                lhs.as_signed().overflowing_add(rhs.as_signed()),
                Fault::BinaryPlusOverflow,
            ),
            | Op::BinaryMinus => arithmetic(
                lhs.as_signed().overflowing_sub(rhs.as_signed()),
                Fault::BinaryMinusOverflow,
            ),
            | Op::Multiply => arithmetic(
                lhs.as_signed().overflowing_mul(rhs.as_signed()),
                Fault::MultiplyOverflow,
            ),
            // C99 §6.5.5p5-6: a zero divisor is undefined, and integer
            // division truncates toward zero.
            | Op::Divide | Op::Modulo if rhs.as_signed() == 0 => (
                Operand::of_type(is_unsigned, 0),
                Some(if operator.kind == Op::Divide {
                    Fault::DivideByZero
                } else {
                    Fault::ModuloByZero
                }),
            ),
            | Op::Divide if is_unsigned => (
                Operand::Unsigned(lhs.as_unsigned() / rhs.as_unsigned()),
                None,
            ),
            | Op::Divide => arithmetic(
                lhs.as_signed().overflowing_div(rhs.as_signed()),
                Fault::DivideOverflow,
            ),
            | Op::Modulo if is_unsigned => (
                Operand::Unsigned(lhs.as_unsigned() % rhs.as_unsigned()),
                None,
            ),
            | Op::Modulo => arithmetic(
                lhs.as_signed().overflowing_rem(rhs.as_signed()),
                Fault::ModuloOverflow,
            ),
            // C99 §6.5.7: shifts promote each operand independently; the
            // right operand never changes the result's type. Only the shift
            // count is checked; a signed left shift that loses bits (§6.5.7p4)
            // is not diagnosed.
            | Op::LeftShift => {
                let count_overflows = u32::try_from(rhs.as_unsigned()).is_err();
                let (new, overflow) = lhs.as_signed().overflowing_shl(rhs.as_unsigned() as u32);
                (
                    Operand::of_type(lhs.is_unsigned(), new),
                    (count_overflows || overflow).then_some(Fault::LeftShiftOverflow),
                )
            },
            // C99 §6.5.7p5: shifting a negative signed value right is
            // implementation-defined; here it is arithmetic.
            | Op::RightShift => {
                let count_overflows = u32::try_from(rhs.as_unsigned()).is_err();
                let (new, overflow) = if lhs.is_unsigned() {
                    let (new, overflow) =
                        lhs.as_unsigned().overflowing_shr(rhs.as_unsigned() as u32);
                    (new as i64, overflow)
                } else {
                    lhs.as_signed().overflowing_shr(rhs.as_unsigned() as u32)
                };
                (
                    Operand::of_type(lhs.is_unsigned(), new),
                    (count_overflows || overflow).then_some(Fault::RightShiftOverflow),
                )
            },
            | Op::LessThan if is_unsigned => truth(lhs.as_unsigned() < rhs.as_unsigned()),
            | Op::LessThan => truth(lhs.as_signed() < rhs.as_signed()),
            | Op::LessThanEquals if is_unsigned => truth(lhs.as_unsigned() <= rhs.as_unsigned()),
            | Op::LessThanEquals => truth(lhs.as_signed() <= rhs.as_signed()),
            | Op::GreaterThan if is_unsigned => truth(lhs.as_unsigned() > rhs.as_unsigned()),
            | Op::GreaterThan => truth(lhs.as_signed() > rhs.as_signed()),
            | Op::GreaterThanEquals if is_unsigned => truth(lhs.as_unsigned() >= rhs.as_unsigned()),
            | Op::GreaterThanEquals => truth(lhs.as_signed() >= rhs.as_signed()),
            | Op::Equals => truth(lhs.as_signed() == rhs.as_signed()),
            | Op::NotEquals => truth(lhs.as_signed() != rhs.as_signed()),
            | Op::BitwiseAnd => (
                Operand::of_type(is_unsigned, lhs.as_signed() & rhs.as_signed()),
                None,
            ),
            | Op::BitwiseXor => (
                Operand::of_type(is_unsigned, lhs.as_signed() ^ rhs.as_signed()),
                None,
            ),
            | Op::BitwiseOr => (
                Operand::of_type(is_unsigned, lhs.as_signed() | rhs.as_signed()),
                None,
            ),
            | other => unreachable!("{other:?} is not a binary arithmetic operator"),
        };
        if let Some(fault) = fault {
            self.expression_parser
                .operand_stack
                .record_fault(fault, operator.source_vectors);
        }
        self.expression_parser.operand_stack.push(result);
    }

    /// Applies `&&` or `||`.
    ///
    /// C99 §6.5.13p4 and §6.5.14p4: the right operand is evaluated only when
    /// the left one does not decide the result, so only then do its faults
    /// and evaluated commas count.
    fn apply_logical_operator(&mut self, operator: PreprocessorExpressionOperator) {
        let Some((lhs, rhs)) = self.pop_binary_operands(operator, true) else {
            return;
        };
        let is_and = operator == PreprocessorExpressionOperator::LogicalAnd;
        let lhs_is_true = lhs.as_signed() != 0;
        let rhs_is_evaluated = lhs_is_true == is_and;
        self.expression_parser
            .operand_stack
            .retain_faults(lhs.arithmetic_faults);
        if rhs_is_evaluated {
            self.expression_parser
                .operand_stack
                .retain_faults(rhs.arithmetic_faults);
        }
        let rhs_is_true = rhs.as_signed() != 0;
        let value = if is_and {
            lhs_is_true && rhs_is_true
        } else {
            lhs_is_true || rhs_is_true
        };
        self.expression_parser.operand_stack.push(
            EvaluatedPreprocessorExpressionOperand::with_comma_liveness(
                PreprocessorExpressionOperand::Signed(i64::from(value)),
                lhs.contains_evaluated_comma || (rhs_is_evaluated && rhs.contains_evaluated_comma),
            ),
        );
    }

    /// Pops one operand of a conditional operator, or reports `missing` and
    /// leaves a 0 in its place.
    fn pop_conditional_operand(
        &mut self,
        missing: PreprocessorErrorType<'tu>,
    ) -> Option<EvaluatedPreprocessorExpressionOperand> {
        let operand = self.expression_parser.operand_stack.pop_isolated();
        if operand.is_none() {
            let source_vectors = self.current_location();
            self.context.preprocessor_error(PreprocessorError {
                error_type: missing,
                source_vectors,
            });
            self.expression_parser
                .operand_stack
                .push(PreprocessorExpressionOperand::Signed(0));
        }
        operand
    }

    /// Applies `? :` once its third operand is known.
    ///
    /// C99: §6.5.15 paragraphs 4-5, p. 90; PDF p. 102.
    fn apply_conditional_operator(&mut self) {
        let Some(final_operand) =
            self.pop_conditional_operand(PreprocessorErrorType::TernaryOperatorWithoutRhs)
        else {
            return;
        };
        let Some(middle_operand) =
            self.pop_conditional_operand(PreprocessorErrorType::TernaryOperatorWithoutMhs)
        else {
            return;
        };
        let Some(condition) =
            self.pop_conditional_operand(PreprocessorErrorType::TernaryOperatorWithoutRhs)
        else {
            return;
        };
        // The unselected arm still participates in the usual
        // arithmetic conversions (C99 §6.5.15p5).
        let is_unsigned = middle_operand.is_unsigned() || final_operand.is_unsigned();
        let selected_operand = if condition.as_signed() != 0 {
            middle_operand
        } else {
            final_operand
        };
        self.expression_parser
            .operand_stack
            .retain_faults(condition.arithmetic_faults);
        self.expression_parser
            .operand_stack
            .retain_faults(selected_operand.arithmetic_faults);
        self.expression_parser.operand_stack.push(
            EvaluatedPreprocessorExpressionOperand::with_comma_liveness(
                if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(selected_operand.as_unsigned())
                } else {
                    selected_operand.value
                },
                condition.contains_evaluated_comma || selected_operand.contains_evaluated_comma,
            ),
        );
    }

    /// Evaluates `defined identifier` or `defined ( identifier )` to 1 when
    /// the identifier is a macro name and 0 otherwise. The identifier is read
    /// before macro replacement.
    ///
    /// C99: §6.10.1 paragraph 1, p. 148; PDF p. 160, and paragraph 4.
    fn parse_defined_operator(&mut self) {
        let Some(ident_or_opening_paren) = self.expect_token_from_previous_phase::<true>(|_, t| matches!(t.kind, PreprocessorTokenType::Identifier | PreprocessorTokenType::UniversalIdentifier | PreprocessorTokenType::OpeningParenthesis),
            |_, t|
                ControlFlow::Break(PreprocessorError {
                        error_type:     PreprocessorErrorType::MissingOpeningParenthesisOrIdentifierInDefinedDirective(t.kind),
                        source_vectors: t.source_vectors,
                    },
                )
            ,
            "parsing defined operator",
        ) else {
            // The malformed operator still stands for one operand, so the
            // expression continues in the binary state without cascading.
            self.skip_token_unless_line_end();
            self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(0));
            self.expression_parser.state = PreprocessorExpressionParserState::Binary;
            return;
        };
        if ident_or_opening_paren.kind.is_identifier() {
            let is_defined = self
                .state
                .macro_definitions
                .contains_key(&ident_or_opening_paren.identifier_id(self.context));
            self.expression_parser
                .operand_stack
                .push(PreprocessorExpressionOperand::Signed(i64::from(is_defined)));
            self.expression_parser.state = PreprocessorExpressionParserState::Binary;
            return;
        }
        assert_eq!(
            ident_or_opening_paren.kind,
            PreprocessorTokenType::OpeningParenthesis,
            "Compiler bug: ident_or_opening_paren should be an opening parenthesis or identifier."
        );
        let Some(ident) = self.expect_token_from_previous_phase::<true>(
            |_, t| t.kind.is_identifier(),
            |_, t| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::MissingIdentifierInDefinedDirective(
                        t.kind,
                    ),
                    source_vectors: t.source_vectors,
                })
            },
            "parsing defined operator",
        ) else {
            self.skip_token_unless_line_end();
            self.expression_parser
                .operand_stack
                .push(PreprocessorExpressionOperand::Signed(0));
            self.expression_parser.state = PreprocessorExpressionParserState::Binary;
            return;
        };

        _ = self.expect_token_from_previous_phase::<true>(
            |_, t| matches!(t.kind, PreprocessorTokenType::ClosingParenthesis),
            |_, t| {
                ControlFlow::Break(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::MissingClosingParenthesisInDefinedDirective(t.kind),
                    source_vectors: t.source_vectors,
                })
            },
            "parsing defined operator",
        );
        let is_defined = self
            .state
            .macro_definitions
            .contains_key(&ident.identifier_id(self.context));
        self.expression_parser
            .operand_stack
            .push(PreprocessorExpressionOperand::Signed(i64::from(is_defined)));
        self.expression_parser.state = PreprocessorExpressionParserState::Binary;
    }

    /// Consumes the next token of a directive after it was diagnosed, unless
    /// it ends the line.
    fn skip_token_unless_line_end(&mut self) {
        let position = self.position();
        match Self::next_ignore_whitespace(&mut self.tokenizer, self.context) {
            | Some(token) if token.kind != PreprocessorTokenType::Newline => {},
            | _ => self.set_position(position),
        }
    }

    /// Restore a parent floor only after this group's values have been
    /// normalized. Pop transfers evaluated fault roots exactly once.
    fn finish_expression_group(&mut self) {
        let stack = &mut self.expression_parser.operand_stack;
        if stack.len() != 1 {
            while stack.pop().is_some() {}
            stack.push(PreprocessorExpressionOperand::Signed(0));
        }
        stack.floor = self.expression_parser.open_parentheses.pop().unwrap_or(0);
    }

    /// Reads the rest of an `#if` or `#elif` line, macro-replacing it, and
    /// returns whether the controlling expression is nonzero.
    ///
    /// C99: §6.10.1 paragraphs 3-4, p. 148; PDF p. 160. Integer constants
    /// are typed as though every signed type were `intmax_t` and every
    /// unsigned type `uintmax_t`, so `0xFFFFFFFF` is signed here though
    /// `unsigned int` in phase 7 (footnote 145); only a `u` suffix or a value
    /// above `INTMAX_MAX` makes one unsigned. A character constant's value is
    /// implementation-defined (paragraph 4): a single narrow character is
    /// its nonnegative byte.
    pub(super) fn eval_preprocessor_expression(
        &mut self,
        on_no_expression_error: PreprocessorErrorType<'tu>,
    ) -> bool {
        let conditional_queries = replace(&mut self.state.conditional_queries, true);
        let result = self
            .eval_preprocessor_value(on_no_expression_error)
            .is_none_or(|value| value.as_signed() != 0);
        self.state.conditional_queries = conditional_queries;
        result
    }

    /// C23: #embed limit uses an integer constant expression (§6.10.4.2p1).
    pub(super) fn eval_resource_limit(&mut self) -> Option<u64> {
        let saved_parser = replace(
            &mut self.expression_parser,
            PreprocessorExpressionParser::new(self.state.arena),
        );
        let newlines = (self.last_was_newline, self.current_is_newline);
        let before = self.context.pending_error_count();
        let result = self.eval_preprocessor_value(PreprocessorErrorType::LanguageConstraint(
            "expected embed limit expression",
        ));
        self.expression_parser = saved_parser;
        (self.last_was_newline, self.current_is_newline) = newlines;
        let mut diagnostics = ArenaVec::new_in(self.scratch);
        diagnostics.extend(self.context.split_off_pending_errors(before));
        let failed = diagnostics.iter().any(|error| {
            crate::translation_phases::GetSeverity::severity(error)
                == crate::translation_phases::ErrorSeverity::Error
        });
        self.context.append_pending_errors(diagnostics);
        if failed {
            return None;
        }
        match result? {
            | PreprocessorExpressionOperand::Signed(value) if value < 0 => {
                let source = self.current_location();
                self.language_error("embed limit must be nonnegative", source);
                None
            },
            | value => Some(value.as_unsigned()),
        }
    }

    fn eval_preprocessor_value(
        &mut self,
        on_no_expression_error: PreprocessorErrorType<'tu>,
    ) -> Option<PreprocessorExpressionOperand> {
        const UNARY: PreprocessorExpressionParserState = PreprocessorExpressionParserState::Unary;
        const BINARY: PreprocessorExpressionParserState = PreprocessorExpressionParserState::Binary;
        self.expression_parser.reset();
        'main: loop {
            match self.next_preprocessor_token::<true>() {
                | None => {
                    let source_vectors = self.current_location();
                    self.context.preprocessor_error(
                        PreprocessorError {
                            error_type: PreprocessorErrorType::UnexpectedEndOfInput("parsing preprocessor expression"),
                            source_vectors,
                        });
                    break 'main;
                },
                | Some(token) => match (token.kind, self.expression_parser.state) {
                    | (PreprocessorTokenType::Newline, _) =>
                        break 'main,
                    (PreprocessorTokenType::Plus, UNARY) =>
                        self.expression_parser.operator_stack.push(LocatedExpressionOperator { kind: PreprocessorExpressionOperator::UnaryPlus, source_vectors: token.source_vectors }),
                    (PreprocessorTokenType::Minus, UNARY) =>
                        self.expression_parser.operator_stack.push(LocatedExpressionOperator { kind: PreprocessorExpressionOperator::UnaryMinus, source_vectors: token.source_vectors }),
                    (PreprocessorTokenType::Tilde, UNARY) =>
                        self.expression_parser.operator_stack.push(LocatedExpressionOperator { kind: PreprocessorExpressionOperator::BitwiseNot, source_vectors: token.source_vectors }),
                    (PreprocessorTokenType::Tilde, BINARY) =>
                        self.context.preprocessor_error(PreprocessorError {
                                error_type: PreprocessorErrorType::TildeInsteadOfBinaryOperatorInPreprocessorExpression,
                                source_vectors: token.source_vectors,
                            },
                        )
                ,
                    (PreprocessorTokenType::ExclamationMark, UNARY) =>
                        self.expression_parser.operator_stack.push(LocatedExpressionOperator { kind: PreprocessorExpressionOperator::LogicalNot, source_vectors: token.source_vectors }),
                    (PreprocessorTokenType::ExclamationMark, BINARY) =>
                        self.context.preprocessor_error(PreprocessorError {
                                error_type:     PreprocessorErrorType::ExclamationMarkInsteadOfBinaryOperatorInPreprocessorExpression,
                                source_vectors: token.source_vectors,
                            },
                        )
                    ,
                    (PreprocessorTokenType::OpeningParenthesis, UNARY) => {
                        self.expression_parser.operator_stack.push(LocatedExpressionOperator { kind: PreprocessorExpressionOperator::OpeningParenthesis, source_vectors: token.source_vectors });
                        let stack = &mut self.expression_parser.operand_stack;
                        self.expression_parser.open_parentheses.push(stack.floor);
                        stack.floor = stack.values.len();
                    }
                    (PreprocessorTokenType::OpeningParenthesis, BINARY) => {
                        self.context.preprocessor_error(PreprocessorError {
                                error_type:     PreprocessorErrorType::FunctionCallOperatorNotSupportedInPreprocessorExpression,
                                source_vectors: token.source_vectors,
                            },
                        );
                        let mut paren_depth = 1;
                        // Step over function call, which C99 §6.6p3 excludes.
                        while paren_depth > 0 {
                            match self.next_preprocessor_token::<true>() {
                                | None => {
                                    let source_vectors = self.current_location();
                                    self.context.preprocessor_error(PreprocessorError {
                                            error_type: PreprocessorErrorType::UnexpectedEndOfInput("parsing preprocessor expression"),
                                            source_vectors,
                                        },
                                    );
                                    break 'main;
                                },
                                | Some(token) => match token.kind {
                                    | PreprocessorTokenType::Newline => break 'main,
                                    | PreprocessorTokenType::OpeningParenthesis => paren_depth += 1,
                                    | PreprocessorTokenType::ClosingParenthesis => paren_depth -= 1,
                                    | _ => (),
                                },
                            }
                        }
                    }
                    (PreprocessorTokenType::ClosingParenthesis, state) => {
                        if self.expression_parser.open_parentheses.is_empty() {
                            self.context.preprocessor_error(PreprocessorError {
                                    error_type:     PreprocessorErrorType::UnexpectedTokenInPreprocessorExpression(token.kind),
                                    source_vectors: token.source_vectors,
                                },
                            );
                            continue 'main;
                        }
                        if state == UNARY {
                            let error_type = match self.expression_parser.operator_stack.last().map(|operator| operator.kind) {
                                | Some(PreprocessorExpressionOperator::OpeningParenthesis) | None => PreprocessorErrorType::EmptyParenthesesInPreprocessorExpression,
                                | Some(operator) => operator.operand_missing_while_reading(),
                            };
                            self.context.preprocessor_error(PreprocessorError {
                                error_type,
                                source_vectors: token.source_vectors,
                            });
                            // An invalid group still contributes one recovered
                            // operand, so following operators cannot underflow.
                            self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(0));
                        }
                        self.expression_parser.state = BINARY;
                        while let Some(op) = self.expression_parser.operator_stack.pop() {
                            if op.kind == PreprocessorExpressionOperator::OpeningParenthesis {
                                self.finish_expression_group();
                                break;
                            }
                            self.handle_expression_operator(op);
                        }
                    }
                    (PreprocessorTokenType::Defined, UNARY) =>
                        self.parse_defined_operator(),
                    (PreprocessorTokenType::Defined, BINARY) => self.context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::DefinedOperatorInsteadOfBinaryOperatorInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    ),
                    (PreprocessorTokenType::Asterisk, UNARY) => self.context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::DereferenceOperatorNotSupportedInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    ),
                    (PreprocessorTokenType::Ampersand, UNARY) => self.context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::AddressOfOperatorNotSupportedInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    ),
                    (PreprocessorTokenType::Colon, state) => {
                        if state == UNARY {
                            self.context.preprocessor_error(PreprocessorError {
                                error_type: PreprocessorErrorType::TernaryOperatorWithoutMhs,
                                source_vectors: token.source_vectors,
                            });
                        }
                        let mut matched_question_mark = false;
                        while let Some(last) = self.expression_parser.operator_stack.last() {
                            match last.kind {
                                | PreprocessorExpressionOperator::QuestionMark => {
                                    *self.expression_parser.operator_stack.last_mut().unwrap() = LocatedExpressionOperator {
                                        kind: PreprocessorExpressionOperator::Conditional,
                                        source_vectors: token.source_vectors,
                                    };
                                    matched_question_mark = true;
                                    break;
                                },
                                | PreprocessorExpressionOperator::OpeningParenthesis => break,
                                | _ => {
                                    let operator = self.expression_parser.operator_stack.pop().unwrap();
                                    self.handle_expression_operator(operator);
                                },
                            }
                        }
                        if matched_question_mark {
                            if state == UNARY {
                                self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(0));
                            }
                            self.expression_parser.state = UNARY;
                        } else {
                            self.context.preprocessor_error(PreprocessorError {
                                error_type: PreprocessorErrorType::ColonWithoutMatchingQuestionMark,
                                source_vectors: token.source_vectors,
                            });
                        }
                    },
                    (
                        | PreprocessorTokenType::ForwardSlash | PreprocessorTokenType::Percent | PreprocessorTokenType::LessThanLessThan |
                        PreprocessorTokenType::GreaterThanGreaterThan | PreprocessorTokenType::LessThan | PreprocessorTokenType::LessThanEquals | PreprocessorTokenType::GreaterThan |
                        PreprocessorTokenType::GreaterThanEquals | PreprocessorTokenType::EqualsEquals | PreprocessorTokenType::ExclamationMarkEquals |
                        PreprocessorTokenType::Caret | PreprocessorTokenType::Pipe | PreprocessorTokenType::AmpersandAmpersand | PreprocessorTokenType::PipePipe | PreprocessorTokenType::QuestionMark |
                        PreprocessorTokenType::Comma,
                        UNARY) => {
                            let operator = self.map_operator(token);
                            self.context.preprocessor_error(PreprocessorError {
                                error_type: PreprocessorErrorType::BinaryOperatorInsteadOfUnaryExpressionInPreprocessorExpression(operator),
                                source_vectors: token.source_vectors,
                            });
                        },
                    | (PreprocessorTokenType::Plus | PreprocessorTokenType::Minus | PreprocessorTokenType::Asterisk
                    | PreprocessorTokenType::ForwardSlash | PreprocessorTokenType::Percent | PreprocessorTokenType::LessThanLessThan |
                    PreprocessorTokenType::GreaterThanGreaterThan | PreprocessorTokenType::LessThan | PreprocessorTokenType::LessThanEquals | PreprocessorTokenType::GreaterThan |
                    PreprocessorTokenType::GreaterThanEquals | PreprocessorTokenType::EqualsEquals | PreprocessorTokenType::ExclamationMarkEquals | PreprocessorTokenType::Ampersand |
                    PreprocessorTokenType::Caret | PreprocessorTokenType::Pipe | PreprocessorTokenType::AmpersandAmpersand | PreprocessorTokenType::PipePipe | PreprocessorTokenType::QuestionMark |
                    PreprocessorTokenType::Comma,
                    BINARY) => {
                        let token_op = self.map_operator(token);
                        while let Some(op) = self.expression_parser.operator_stack.pop() {
                            if op.kind == PreprocessorExpressionOperator::QuestionMark
                                && token_op == PreprocessorExpressionOperator::Comma
                            {
                                self.expression_parser.operator_stack.push(op);
                                break;
                            } else if op.kind.has_precedence_over(token_op) {
                                self.handle_expression_operator(op);
                            } else {
                                self.expression_parser.operator_stack.push(op);
                                break;
                            }
                        }
                        self.expression_parser.operator_stack.push(LocatedExpressionOperator { kind: token_op, source_vectors: token.source_vectors });
                        self.expression_parser.state = UNARY;
                    },
                    (PreprocessorTokenType::Number, UNARY) => {
                        match self.parse_number(token, IntegerRepresentation::IntMax).kind {
                            | TokenType::Float(_) => {
                                // C99 §6.10.1p1 admits only integer constant
                                // expressions. Recover with a zero operand
                                // so evaluation continues without cascading.
                                self.context.preprocessor_error(PreprocessorError {
                                    error_type: PreprocessorErrorType::FloatInsteadOfIntegerInPreprocessorExpression,
                                    source_vectors: token.source_vectors,
                                });
                                self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(0));
                            },
                            | TokenType::Integer(v) => match v {
                                | IntegerTokenType::Int(i) => self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(i64::from(i))),
                                | IntegerTokenType::Long(l) => self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(l.get())),
                                | IntegerTokenType::Imaginary(_, _) => { self.language_error("imaginary constants are not permitted in preprocessing expressions", token.source_vectors); self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(0)); },
                                | IntegerTokenType::BitInt(ll, _, false) => self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(ll.get() as i64)),

                                | IntegerTokenType::LongLong(ll) => self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(ll.get())),
                                | IntegerTokenType::UnsignedInt(ui) => self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Unsigned(u64::from(ui))),
                                | IntegerTokenType::UnsignedLong(ul) => self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Unsigned(ul.get())),
                                | IntegerTokenType::UnsignedLongLong(ull) | IntegerTokenType::BitInt(ull, _, true) => self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Unsigned(ull.get())),
                            },
                            _ => unreachable!("Compiler bug: parse_number should return a number token."),
                        }
                        self.expression_parser.state = BINARY;
                    },
                    (PreprocessorTokenType::Number, BINARY) => self.context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::NumberInsteadOfBinaryOperatorInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    ),
                    (PreprocessorTokenType::Identifier | PreprocessorTokenType::UniversalIdentifier | PreprocessorTokenType::UnavailableIdentifier | PreprocessorTokenType::UnavailableUniversalIdentifier, UNARY) => {
                        // C23: §6.10.2p13, p. 167; PDF p. 180 replaces
                        // remaining `true` with 1 after macro expansion.
                        // C99 §6.10.1p4 replaces all identifiers with 0.
                        let spelling = self.context.string_cache.at(token.contents);
                        let boolean = self.context.configuration.standard() >= CStandard::C23
                            && matches!(spelling, "true" | "false");
                        let value = i64::from(boolean && spelling == "true");
                        if !boolean {
                            self.context.preprocessor_error(PreprocessorError {
                                    error_type: PreprocessorErrorType::UndefinedIdentifierInPreprocessorExpression(self.context.diagnostic_text(self.context.string_cache.at(token.contents))),
                                    source_vectors: token.source_vectors,
                                },
                            );
                        }
                        self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(value));
                        self.expression_parser.state = BINARY;
                    },
                    (PreprocessorTokenType::Identifier | PreprocessorTokenType::UniversalIdentifier | PreprocessorTokenType::UnavailableIdentifier | PreprocessorTokenType::UnavailableUniversalIdentifier, BINARY) => self.context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::IdentifierInsteadOfBinaryOperatorInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    ),
                    (PreprocessorTokenType::Character, UNARY) => {
                        let value = i64::from(self.parse_character(token));
                        self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(value));
                        self.expression_parser.state = BINARY;
                    },
                    (PreprocessorTokenType::Character, BINARY) => self.context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::CharacterInsteadOfBinaryOperatorInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    ),
                    _ => self.context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::UnexpectedTokenInPreprocessorExpression(token.kind),
                            source_vectors: token.source_vectors,
                        },
                    ),
                },
            }
        }
        if self.expression_parser.state == UNARY
            && let Some(operator) = self.expression_parser.operator_stack.last().copied()
            && operator.kind != PreprocessorExpressionOperator::OpeningParenthesis
        {
            self.context.preprocessor_error(PreprocessorError {
                error_type:     operator.kind.operand_missing_while_reading(),
                source_vectors: operator.source_vectors,
            });
            self.expression_parser
                .operand_stack
                .push(PreprocessorExpressionOperand::Signed(0));
        }
        while let Some(op) = self.expression_parser.operator_stack.pop() {
            self.handle_expression_operator(op);
        }
        match self.expression_parser.operand_stack.len() {
            | 1 => {
                let operand = self.expression_parser.operand_stack.pop_isolated().unwrap();
                self.expression_parser
                    .operand_stack
                    .emit_faults(self.context, operand.arithmetic_faults);
                let extension_policy = self.context.configuration.extension_policy();
                if operand.contains_evaluated_comma && extension_policy != ExtensionPolicy::Allow {
                    let source_vectors = self.current_location();
                    self.context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::CommaOperatorInPreprocessorExpression(
                            extension_policy,
                        ),
                        source_vectors,
                    });
                }
                Some(operand.value)
            },
            | 0 => {
                let source_vectors = self.current_location();
                self.context.preprocessor_error(PreprocessorError {
                    error_type: on_no_expression_error,
                    source_vectors,
                });
                None
            },
            | _ => {
                let source_vectors = self.current_location();
                self.context.preprocessor_error(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::ExpectedBinaryOperatorInPreprocessorExpression,
                    source_vectors,
                });
                None
            },
        }
    }
}
