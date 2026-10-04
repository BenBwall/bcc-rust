//! Evaluation of `#if` and `#elif` controlling expressions.

use std::{
    fmt::Debug,
    mem::replace,
    num::NonZeroU32,
    ops::ControlFlow,
};

use super::{
    Preprocessor,
    errors::{
        PreprocessorError,
        PreprocessorErrorType,
    },
    token::{
        IntegerTokenType,
        TokenType,
    },
};
use crate::{
    configuration::{
        CStandard,
        ExtensionPolicy,
    },
    translation_phases::{
        Context,
        GetPosition,
        SetPosition,
        SourceVectors,
        preprocessor_tokenizer::{
            PreprocessorToken,
            PreprocessorTokenType,
        },
    },
    util::last_entry::last_entry,
};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(super) enum PreprocessorExpressionParserState {
    Unary,
    Binary,
}

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
pub(super) enum PreprocessorExpressionAssociativity {
    Left,
    Right,
}

impl PreprocessorExpressionOperator {
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
pub(crate) struct PreprocessorExpressionParser {
    operator_stack:   Vec<LocatedExpressionOperator>,
    operand_stack:    PreprocessorExpressionOperandStack,
    state:            PreprocessorExpressionParserState,
    open_parentheses: Vec<usize>,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum PreprocessorExpressionOperand {
    Signed(i64),
    Unsigned(u64),
}

impl PreprocessorExpressionOperand {
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
pub(super) struct EvaluatedPreprocessorExpressionOperand {
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

#[derive(Debug, PartialEq, Clone, Default)]
pub(super) struct PreprocessorExpressionOperandStack {
    values:                    Vec<EvaluatedPreprocessorExpressionOperand>,
    floor:                     usize,
    pending_evaluated_comma:   bool,
    pending_arithmetic_faults: Option<NonZeroU32>,
    arithmetic_faults:         Vec<ArithmeticFaultNode>,
}

impl PreprocessorExpressionOperandStack {
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
        let index = u32::try_from(self.arithmetic_faults.len())
            .expect("expression fault arena exceeds u32");
        self.arithmetic_faults.push(node);
        NonZeroU32::new(
            index
                .checked_add(1)
                .expect("expression fault arena exceeds u32"),
        )
        .unwrap()
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

    fn emit_faults(&self, context: &mut Context<'_>, root: Option<NonZeroU32>) {
        let Some(root) = root else {
            return;
        };
        let mut pending = vec![root];
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

impl PreprocessorExpressionParser {
    pub(super) fn new() -> Self {
        Self {
            operator_stack:   Vec::new(),
            operand_stack:    PreprocessorExpressionOperandStack::default(),
            state:            PreprocessorExpressionParserState::Unary,
            open_parentheses: Vec::new(),
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
impl Preprocessor<'_> {
    fn map_operator(
        &mut self,
        _context: &Context<'_>,
        operator: PreprocessorToken,
    ) -> PreprocessorExpressionOperator {
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

    #[expect(
        clippy::too_many_lines,
        reason = "This function is long because it contains the logic for evaluating an operator \
                  in a constant expression. I don't think splitting it up would anything clearer."
    )]
    fn handle_expression_operator(
        &mut self,
        context: &mut Context<'_>,
        operator: LocatedExpressionOperator,
    ) {
        match operator.kind {
            | PreprocessorExpressionOperator::UnaryPlus => {
                if self.expression_parser.operand_stack.is_empty() {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::UnaryPlusWithoutOperand,
                        source_vectors,
                    });
                }
            },
            | PreprocessorExpressionOperator::UnaryMinus => {
                let Some(operand) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::UnaryMinusWithoutOperand,
                        source_vectors,
                    });
                    return;
                };
                let (new, did_overflow) = operand.as_signed().overflowing_neg();
                if operand.is_signed() && did_overflow {
                    self.expression_parser.operand_stack.record_fault(
                        ArithmeticFaultKind::UnaryMinusOverflow,
                        operator.source_vectors,
                    );
                }
                self.expression_parser
                    .operand_stack
                    .push(operand.set_signed(new));
            },
            | PreprocessorExpressionOperator::BitwiseNot => {
                let Some(operand) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::BitwiseNotWithoutOperand,
                        source_vectors,
                    });
                    return;
                };
                self.expression_parser
                    .operand_stack
                    .push(operand.map_unsigned(|v| !v));
            },
            | PreprocessorExpressionOperator::LogicalNot => {
                let Some(operand) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::LogicalNotWithoutOperand,
                        source_vectors,
                    });
                    return;
                };
                self.expression_parser
                    .operand_stack
                    .push(PreprocessorExpressionOperand::Signed(i64::from(
                        operand.as_signed() == 0,
                    )));
            },
            | PreprocessorExpressionOperator::BinaryPlus => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .unwrap_or_else(|| PreprocessorExpressionOperand::Signed(0).into());
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::BinaryPlusWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let (new, did_overflow) = lhs.as_signed().overflowing_add(rhs.as_signed());
                if did_overflow && !is_unsigned {
                    self.expression_parser.operand_stack.record_fault(
                        ArithmeticFaultKind::BinaryPlusOverflow,
                        operator.source_vectors,
                    );
                }
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(new as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(new)
                });
            },
            | PreprocessorExpressionOperator::BinaryMinus => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .unwrap_or_else(|| PreprocessorExpressionOperand::Signed(0).into());
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::BinaryMinusWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let (new, did_overflow) = lhs.as_signed().overflowing_sub(rhs.as_signed());
                if did_overflow && !is_unsigned {
                    self.expression_parser.operand_stack.record_fault(
                        ArithmeticFaultKind::BinaryMinusOverflow,
                        operator.source_vectors,
                    );
                }
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(new as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(new)
                });
            },
            | PreprocessorExpressionOperator::Multiply => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .unwrap_or_else(|| PreprocessorExpressionOperand::Signed(0).into());
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::MultiplyWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let (new, did_overflow) = lhs.as_signed().overflowing_mul(rhs.as_signed());
                if did_overflow && !is_unsigned {
                    self.expression_parser.operand_stack.record_fault(
                        ArithmeticFaultKind::MultiplyOverflow,
                        operator.source_vectors,
                    );
                }
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(new as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(new)
                });
            },
            | PreprocessorExpressionOperator::Divide => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .unwrap_or_else(|| PreprocessorExpressionOperand::Signed(0).into());
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::DivideWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                if rhs.as_signed() == 0 {
                    self.expression_parser
                        .operand_stack
                        .record_fault(ArithmeticFaultKind::DivideByZero, operator.source_vectors);
                    self.expression_parser.operand_stack.push(if is_unsigned {
                        PreprocessorExpressionOperand::Unsigned(0)
                    } else {
                        PreprocessorExpressionOperand::Signed(0)
                    });
                    return;
                }
                let (new, did_overflow) = if is_unsigned {
                    ((lhs.as_unsigned() / rhs.as_unsigned()) as i64, false)
                } else {
                    lhs.as_signed().overflowing_div(rhs.as_signed())
                };
                if did_overflow && !is_unsigned {
                    self.expression_parser
                        .operand_stack
                        .record_fault(ArithmeticFaultKind::DivideOverflow, operator.source_vectors);
                }
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(new as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(new)
                });
            },
            | PreprocessorExpressionOperator::Modulo => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .unwrap_or_else(|| PreprocessorExpressionOperand::Signed(0).into());
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::ModuloWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                if rhs.as_signed() == 0 {
                    self.expression_parser
                        .operand_stack
                        .record_fault(ArithmeticFaultKind::ModuloByZero, operator.source_vectors);
                    self.expression_parser.operand_stack.push(if is_unsigned {
                        PreprocessorExpressionOperand::Unsigned(0)
                    } else {
                        PreprocessorExpressionOperand::Signed(0)
                    });
                    return;
                }
                let (new, did_overflow) = if is_unsigned {
                    ((lhs.as_unsigned() % rhs.as_unsigned()) as i64, false)
                } else {
                    lhs.as_signed().overflowing_rem(rhs.as_signed())
                };
                if did_overflow && !is_unsigned {
                    self.expression_parser
                        .operand_stack
                        .record_fault(ArithmeticFaultKind::ModuloOverflow, operator.source_vectors);
                }
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(new as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(new)
                });
            },
            | PreprocessorExpressionOperator::LeftShift => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .unwrap_or_else(|| PreprocessorExpressionOperand::Signed(0).into());
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::LeftShiftWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                // C99 §6.5.7: shifts promote each operand independently;
                // the right operand never changes the result's type.
                let is_unsigned = lhs.is_unsigned();
                let (new, did_overflow) = {
                    let did_overflow = u32::try_from(rhs.as_unsigned()).is_err();
                    let (new, overflow) = lhs.as_signed().overflowing_shl(rhs.as_unsigned() as u32);
                    (new, did_overflow || overflow)
                };
                if did_overflow {
                    self.expression_parser.operand_stack.record_fault(
                        ArithmeticFaultKind::LeftShiftOverflow,
                        operator.source_vectors,
                    );
                }
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(new as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(new)
                });
            },
            | PreprocessorExpressionOperator::RightShift => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .unwrap_or_else(|| PreprocessorExpressionOperand::Signed(0).into());
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::RightShiftWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned();
                let (new, did_overflow) = {
                    let did_overflow = u32::try_from(rhs.as_unsigned()).is_err();
                    let (new, overflow) = if is_unsigned {
                        let (new, overflow) =
                            lhs.as_unsigned().overflowing_shr(rhs.as_unsigned() as u32);
                        (new as i64, overflow)
                    } else {
                        lhs.as_signed().overflowing_shr(rhs.as_unsigned() as u32)
                    };
                    (new, did_overflow || overflow)
                };
                if did_overflow {
                    self.expression_parser.operand_stack.record_fault(
                        ArithmeticFaultKind::RightShiftOverflow,
                        operator.source_vectors,
                    );
                }
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(new as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(new)
                });
            },
            | PreprocessorExpressionOperator::LessThan => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .unwrap_or_else(|| PreprocessorExpressionOperand::Signed(0).into());
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::LessThanWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let result = if is_unsigned {
                    lhs.as_unsigned() < rhs.as_unsigned()
                } else {
                    lhs.as_signed() < rhs.as_signed()
                };
                self.expression_parser
                    .operand_stack
                    .push(PreprocessorExpressionOperand::Signed(i64::from(result)));
            },
            | PreprocessorExpressionOperator::LessThanEquals => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .unwrap_or_else(|| PreprocessorExpressionOperand::Signed(0).into());
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::LessThanEqualsWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let result = if is_unsigned {
                    lhs.as_unsigned() <= rhs.as_unsigned()
                } else {
                    lhs.as_signed() <= rhs.as_signed()
                };
                self.expression_parser
                    .operand_stack
                    .push(PreprocessorExpressionOperand::Signed(i64::from(result)));
            },
            | PreprocessorExpressionOperator::GreaterThan => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .unwrap_or_else(|| PreprocessorExpressionOperand::Signed(0).into());
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::GreaterThanWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let result = if is_unsigned {
                    lhs.as_unsigned() > rhs.as_unsigned()
                } else {
                    lhs.as_signed() > rhs.as_signed()
                };
                self.expression_parser
                    .operand_stack
                    .push(PreprocessorExpressionOperand::Signed(i64::from(result)));
            },
            | PreprocessorExpressionOperator::GreaterThanEquals => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .unwrap_or_else(|| PreprocessorExpressionOperand::Signed(0).into());
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::GreaterThanEqualsWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let result = if is_unsigned {
                    lhs.as_unsigned() >= rhs.as_unsigned()
                } else {
                    lhs.as_signed() >= rhs.as_signed()
                };
                self.expression_parser
                    .operand_stack
                    .push(PreprocessorExpressionOperand::Signed(i64::from(result)));
            },
            | PreprocessorExpressionOperator::Equals => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .unwrap_or_else(|| PreprocessorExpressionOperand::Signed(0).into());
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::EqualsWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                self.expression_parser
                    .operand_stack
                    .push(PreprocessorExpressionOperand::Signed(i64::from(
                        lhs.as_signed() == rhs.as_signed(),
                    )));
            },
            | PreprocessorExpressionOperator::NotEquals => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .unwrap_or_else(|| PreprocessorExpressionOperand::Signed(0).into());
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::NotEqualsWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                self.expression_parser
                    .operand_stack
                    .push(PreprocessorExpressionOperand::Signed(i64::from(
                        lhs.as_signed() != rhs.as_signed(),
                    )));
            },
            | PreprocessorExpressionOperator::BitwiseAnd => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .unwrap_or_else(|| PreprocessorExpressionOperand::Signed(0).into());
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::BitwiseAndWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let result = lhs.as_signed() & rhs.as_signed();
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(result as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(result)
                });
            },
            | PreprocessorExpressionOperator::BitwiseXor => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .unwrap_or_else(|| PreprocessorExpressionOperand::Signed(0).into());
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::BitwiseXorWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let result = lhs.as_signed() ^ rhs.as_signed();
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(result as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(result)
                });
            },
            | PreprocessorExpressionOperator::BitwiseOr => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .unwrap_or_else(|| PreprocessorExpressionOperand::Signed(0).into());
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::BitwiseOrWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let result = lhs.as_signed() | rhs.as_signed();
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(result as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(result)
                });
            },
            | PreprocessorExpressionOperator::LogicalAnd => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop_isolated()
                    .unwrap_or_else(|| PreprocessorExpressionOperand::Signed(0).into());
                let Some(lhs) = self.expression_parser.operand_stack.pop_isolated() else {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::LogicalAndWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let lhs_is_true = lhs.as_signed() != 0;
                self.expression_parser
                    .operand_stack
                    .retain_faults(lhs.arithmetic_faults);
                if lhs_is_true {
                    self.expression_parser
                        .operand_stack
                        .retain_faults(rhs.arithmetic_faults);
                }
                self.expression_parser.operand_stack.push(
                    EvaluatedPreprocessorExpressionOperand::with_comma_liveness(
                        PreprocessorExpressionOperand::Signed(i64::from(
                            lhs_is_true && rhs.as_signed() != 0,
                        )),
                        lhs.contains_evaluated_comma
                            || (lhs_is_true && rhs.contains_evaluated_comma),
                    ),
                );
            },
            | PreprocessorExpressionOperator::LogicalOr => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop_isolated()
                    .unwrap_or_else(|| PreprocessorExpressionOperand::Signed(0).into());
                let Some(lhs) = self.expression_parser.operand_stack.pop_isolated() else {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::LogicalOrWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let lhs_is_true = lhs.as_signed() != 0;
                self.expression_parser
                    .operand_stack
                    .retain_faults(lhs.arithmetic_faults);
                if !lhs_is_true {
                    self.expression_parser
                        .operand_stack
                        .retain_faults(rhs.arithmetic_faults);
                }
                self.expression_parser.operand_stack.push(
                    EvaluatedPreprocessorExpressionOperand::with_comma_liveness(
                        PreprocessorExpressionOperand::Signed(i64::from(
                            lhs_is_true || rhs.as_signed() != 0,
                        )),
                        lhs.contains_evaluated_comma
                            || (!lhs_is_true && rhs.contains_evaluated_comma),
                    ),
                );
            },
            | PreprocessorExpressionOperator::Comma => {
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
            | PreprocessorExpressionOperator::QuestionMark => {
                context.preprocessor_error(PreprocessorError {
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
            | PreprocessorExpressionOperator::Conditional => {
                let Some(final_operand) = self.expression_parser.operand_stack.pop_isolated()
                else {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::TernaryOperatorWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser
                        .operand_stack
                        .push(PreprocessorExpressionOperand::Signed(0));
                    return;
                };
                let Some(middle_operand) = self.expression_parser.operand_stack.pop_isolated()
                else {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::TernaryOperatorWithoutMhs,
                        source_vectors,
                    });
                    self.expression_parser
                        .operand_stack
                        .push(PreprocessorExpressionOperand::Signed(0));
                    return;
                };
                let Some(condition) = self.expression_parser.operand_stack.pop_isolated() else {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::TernaryOperatorWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser
                        .operand_stack
                        .push(PreprocessorExpressionOperand::Signed(0));
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
                        condition.contains_evaluated_comma
                            || selected_operand.contains_evaluated_comma,
                    ),
                );
            },
            | PreprocessorExpressionOperator::OpeningParenthesis => {
                let source_vectors = operator.source_vectors;
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::UnterminatedOpeningParenthesisInPreprocessorExpression,
                    source_vectors,
                });
                self.finish_expression_group();
            },
        }
    }

    fn parse_defined_operator(&mut self, context: &mut Context<'_>) {
        let Some(ident_or_opening_paren) = self.expect_token_from_previous_phase::<true>(context,
            |_, _, t| matches!(t.kind, PreprocessorTokenType::Identifier | PreprocessorTokenType::UniversalIdentifier | PreprocessorTokenType::OpeningParenthesis),
            |_, _, t|
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
            self.skip_token_unless_line_end(context);
            self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(0));
            self.expression_parser.state = PreprocessorExpressionParserState::Binary;
            return;
        };
        if ident_or_opening_paren.kind.is_identifier() {
            let is_defined = self
                .macro_definitions
                .contains_key(&ident_or_opening_paren.identifier_id(context));
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
            context,
            |_, _, t| t.kind.is_identifier(),
            |_, _, t| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::MissingIdentifierInDefinedDirective(
                        t.kind,
                    ),
                    source_vectors: t.source_vectors,
                })
            },
            "parsing defined operator",
        ) else {
            self.skip_token_unless_line_end(context);
            self.expression_parser
                .operand_stack
                .push(PreprocessorExpressionOperand::Signed(0));
            self.expression_parser.state = PreprocessorExpressionParserState::Binary;
            return;
        };

        _ = self.expect_token_from_previous_phase::<true>(
            context,
            |_, _, t| matches!(t.kind, PreprocessorTokenType::ClosingParenthesis),
            |_, _, t| {
                ControlFlow::Break(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::MissingClosingParenthesisInDefinedDirective(t.kind),
                    source_vectors: t.source_vectors,
                })
            },
            "parsing defined operator",
        );
        let is_defined = self
            .macro_definitions
            .contains_key(&ident.identifier_id(context));
        self.expression_parser
            .operand_stack
            .push(PreprocessorExpressionOperand::Signed(i64::from(is_defined)));
        self.expression_parser.state = PreprocessorExpressionParserState::Binary;
    }

    /// Consumes the next token of a directive after it was diagnosed, unless
    /// it ends the line.
    fn skip_token_unless_line_end(&mut self, context: &mut Context<'_>) {
        let position = self.position(context);
        match Self::next_ignore_whitespace(&mut self.tokenizer, context) {
            | Some(token) if token.kind != PreprocessorTokenType::Newline => {},
            | _ => self.set_position(context, position),
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

    pub(super) fn eval_preprocessor_expression<'tu>(
        &mut self,
        context: &mut Context<'tu>,
        on_no_expression_error: PreprocessorErrorType<'tu>,
    ) -> bool {
        const UNARY: PreprocessorExpressionParserState = PreprocessorExpressionParserState::Unary;
        const BINARY: PreprocessorExpressionParserState = PreprocessorExpressionParserState::Binary;
        self.expression_parser.reset();
        'main: loop {
            match self.next_preprocessor_token::<true>(context) {
                | None => {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(
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
                        context.preprocessor_error(PreprocessorError {
                                error_type: PreprocessorErrorType::TildeInsteadOfBinaryOperatorInPreprocessorExpression,
                                source_vectors: token.source_vectors,
                            },
                        )
                ,
                    (PreprocessorTokenType::ExclamationMark, UNARY) =>
                        self.expression_parser.operator_stack.push(LocatedExpressionOperator { kind: PreprocessorExpressionOperator::LogicalNot, source_vectors: token.source_vectors }),
                    (PreprocessorTokenType::ExclamationMark, BINARY) =>
                        context.preprocessor_error(PreprocessorError {
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
                        context.preprocessor_error(PreprocessorError {
                                error_type:     PreprocessorErrorType::FunctionCallOperatorNotSupportedInPreprocessorExpression,
                                source_vectors: token.source_vectors,
                            },
                        );
                        let mut paren_depth = 1;
                        // Step over function call.
                        while paren_depth > 0 {
                            match self.next_preprocessor_token::<true>(context) {
                                | None => {
                                    let source_vectors = self.current_location(context);
                                    context.preprocessor_error(PreprocessorError {
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
                            context.preprocessor_error(PreprocessorError {
                                    error_type:     PreprocessorErrorType::UnexpectedTokenInPreprocessorExpression(token.kind),
                                    source_vectors: token.source_vectors,
                                },
                            );
                            continue 'main;
                        }
                        if state == UNARY {
                            let error_type = match self.expression_parser.operator_stack.last().map(|operator| operator.kind) {
                                | Some(PreprocessorExpressionOperator::UnaryPlus) => PreprocessorErrorType::UnaryPlusWithoutOperand,
                                | Some(PreprocessorExpressionOperator::UnaryMinus) => PreprocessorErrorType::UnaryMinusWithoutOperand,
                                | Some(PreprocessorExpressionOperator::BitwiseNot) => PreprocessorErrorType::BitwiseNotWithoutOperand,
                                | Some(PreprocessorExpressionOperator::LogicalNot) => PreprocessorErrorType::LogicalNotWithoutOperand,
                                | Some(PreprocessorExpressionOperator::OpeningParenthesis) | None => PreprocessorErrorType::EmptyParenthesesInPreprocessorExpression,
                                | Some(operator) => PreprocessorErrorType::ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(operator),
                            };
                            context.preprocessor_error(PreprocessorError {
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
                            self.handle_expression_operator(context, op);
                        }
                    }
                    (PreprocessorTokenType::Defined, UNARY) =>
                        self.parse_defined_operator(context),
                    (PreprocessorTokenType::Defined, BINARY) => context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::DefinedOperatorInsteadOfBinaryOperatorInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    ),
                    (PreprocessorTokenType::Asterisk, UNARY) => context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::DereferenceOperatorNotSupportedInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    ),
                    (PreprocessorTokenType::Ampersand, UNARY) => context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::AddressOfOperatorNotSupportedInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    ),
                    (PreprocessorTokenType::Colon, state) => {
                        if state == UNARY {
                            context.preprocessor_error(PreprocessorError {
                                error_type: PreprocessorErrorType::TernaryOperatorWithoutMhs,
                                source_vectors: token.source_vectors,
                            });
                        }
                        let mut matched_question_mark = false;
                        while let Some(mut entry) =
                            last_entry(&mut self.expression_parser.operator_stack)
                        {
                            match entry.kind {
                                | PreprocessorExpressionOperator::QuestionMark => {
                                    _ = entry.insert(LocatedExpressionOperator {
                                        kind: PreprocessorExpressionOperator::Conditional,
                                        source_vectors: token.source_vectors,
                                    });
                                    matched_question_mark = true;
                                    break;
                                },
                                | PreprocessorExpressionOperator::OpeningParenthesis => break,
                                | _ => {
                                    let operator = entry.remove();
                                    self.handle_expression_operator(context, operator);
                                },
                            }
                        }
                        if matched_question_mark {
                            if state == UNARY {
                                self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(0));
                            }
                            self.expression_parser.state = UNARY;
                        } else {
                            context.preprocessor_error(PreprocessorError {
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
                        UNARY) => context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::BinaryOperatorInsteadOfUnaryExpressionInPreprocessorExpression(self.map_operator(context, token)),
                            source_vectors: token.source_vectors,
                        }),
                    | (PreprocessorTokenType::Plus | PreprocessorTokenType::Minus | PreprocessorTokenType::Asterisk
                    | PreprocessorTokenType::ForwardSlash | PreprocessorTokenType::Percent | PreprocessorTokenType::LessThanLessThan |
                    PreprocessorTokenType::GreaterThanGreaterThan | PreprocessorTokenType::LessThan | PreprocessorTokenType::LessThanEquals | PreprocessorTokenType::GreaterThan |
                    PreprocessorTokenType::GreaterThanEquals | PreprocessorTokenType::EqualsEquals | PreprocessorTokenType::ExclamationMarkEquals | PreprocessorTokenType::Ampersand |
                    PreprocessorTokenType::Caret | PreprocessorTokenType::Pipe | PreprocessorTokenType::AmpersandAmpersand | PreprocessorTokenType::PipePipe | PreprocessorTokenType::QuestionMark |
                    PreprocessorTokenType::Comma,
                    BINARY) => {
                        let token_op = self.map_operator(context, token);
                        while let Some(op) = self.expression_parser.operator_stack.pop() {
                            if op.kind == PreprocessorExpressionOperator::QuestionMark
                                && token_op == PreprocessorExpressionOperator::Comma
                            {
                                self.expression_parser.operator_stack.push(op);
                                break;
                            } else if op.kind.has_precedence_over(token_op) {
                                self.handle_expression_operator(context, op);
                            } else {
                                self.expression_parser.operator_stack.push(op);
                                break;
                            }
                        }
                        self.expression_parser.operator_stack.push(LocatedExpressionOperator { kind: token_op, source_vectors: token.source_vectors });
                        self.expression_parser.state = UNARY;
                    },
                    (PreprocessorTokenType::Number, UNARY) => {
                        match self.parse_number(context, token,).kind {
                            | TokenType::Float(_) => {
                                // C99 §6.10.1p1 admits only integer constant
                                // expressions. Recover with a zero operand
                                // so evaluation continues without cascading.
                                context.preprocessor_error(PreprocessorError {
                                    error_type: PreprocessorErrorType::FloatInsteadOfIntegerInPreprocessorExpression,
                                    source_vectors: token.source_vectors,
                                });
                                self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(0));
                            },
                            | TokenType::Integer(v) => match v {
                                | IntegerTokenType::Int(i) => self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(i64::from(i))),
                                | IntegerTokenType::Long(l) => self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(l.get())),
                                | IntegerTokenType::LongLong(ll) => self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(ll.get())),
                                | IntegerTokenType::UnsignedInt(ui) => self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Unsigned(u64::from(ui))),
                                | IntegerTokenType::UnsignedLong(ul) => self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Unsigned(ul.get())),
                                | IntegerTokenType::UnsignedLongLong(ull) => self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Unsigned(ull.get())),
                            },
                            _ => unreachable!("Compiler bug: parse_number should return a number token."),
                        }
                        self.expression_parser.state = BINARY;
                    },
                    (PreprocessorTokenType::Number, BINARY) => context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::NumberInsteadOfBinaryOperatorInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    ),
                    (PreprocessorTokenType::Identifier | PreprocessorTokenType::UniversalIdentifier | PreprocessorTokenType::UnavailableIdentifier | PreprocessorTokenType::UnavailableUniversalIdentifier, UNARY) => {
                        context.preprocessor_error(PreprocessorError {
                                    error_type: PreprocessorErrorType::UndefinedIdentifierInPreprocessorExpression(context.diagnostic_text(context.string_cache.at(token.contents))),
                                    source_vectors: token.source_vectors,
                                },
                        );
                        self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(0));
                        self.expression_parser.state = BINARY;
                    },
                    (PreprocessorTokenType::Identifier | PreprocessorTokenType::UniversalIdentifier | PreprocessorTokenType::UnavailableIdentifier | PreprocessorTokenType::UnavailableUniversalIdentifier, BINARY) => context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::IdentifierInsteadOfBinaryOperatorInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    ),
                    (PreprocessorTokenType::Character, UNARY) => {
                        let value = i64::from(Self::parse_character(context, token));
                        self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(value));
                        self.expression_parser.state = BINARY;
                    },
                    (PreprocessorTokenType::Character, BINARY) => context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::CharacterInsteadOfBinaryOperatorInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    ),
                    _ => context.preprocessor_error(PreprocessorError {
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
            let error_type = match operator.kind {
                | PreprocessorExpressionOperator::UnaryPlus => PreprocessorErrorType::UnaryPlusWithoutOperand,
                | PreprocessorExpressionOperator::UnaryMinus => PreprocessorErrorType::UnaryMinusWithoutOperand,
                | PreprocessorExpressionOperator::BitwiseNot => PreprocessorErrorType::BitwiseNotWithoutOperand,
                | PreprocessorExpressionOperator::LogicalNot => PreprocessorErrorType::LogicalNotWithoutOperand,
                | other => PreprocessorErrorType::ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(other),
            };
            context.preprocessor_error(PreprocessorError {
                error_type,
                source_vectors: operator.source_vectors,
            });
            self.expression_parser
                .operand_stack
                .push(PreprocessorExpressionOperand::Signed(0));
        }
        while let Some(op) = self.expression_parser.operator_stack.pop() {
            self.handle_expression_operator(context, op);
        }
        match self.expression_parser.operand_stack.len() {
            | 1 => {
                let operand = self.expression_parser.operand_stack.pop_isolated().unwrap();
                self.expression_parser
                    .operand_stack
                    .emit_faults(context, operand.arithmetic_faults);
                let extension_policy = match context.configuration.standard() {
                    | CStandard::C99 => context.configuration.extension_policy(),
                };
                if operand.contains_evaluated_comma && extension_policy != ExtensionPolicy::Allow {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::CommaOperatorInPreprocessorExpression(
                            extension_policy,
                        ),
                        source_vectors,
                    });
                }
                operand.as_signed() != 0
            },
            | 0 => {
                let source_vectors = self.current_location(context);
                context.preprocessor_error(PreprocessorError {
                    error_type: on_no_expression_error,
                    source_vectors,
                });
                true
            },
            | _ => {
                let source_vectors = self.current_location(context);
                context.preprocessor_error(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::ExpectedBinaryOperatorInPreprocessorExpression,
                    source_vectors,
                });
                true
            },
        }
    }
}
