//! The truth values GCC's front end folds a condition with side effects to,
//! which then decide the code around it at `-O0` as a constant does. GCC's
//! `fold` (`gcc/fold-const.cc`) rewrites `X && 0` to `(X, 0)` and `X || 1`
//! to `(X, 1)` when `X` has side effects, and folds an operator whose
//! operand is such a comma through to the comma's constant. The program
//! still evaluates `X`; only the code the constant decides against is
//! omitted, so a function called there alone is never referenced. Clang's
//! code generation folds only conditions without side effects
//! (`folding.rs`), so this applies under GCC's rules alone.
//!
//! The model covers parentheses, `__extension__`, `!`, the comma, `&&` and
//! `||`; GCC folds more (`X * 0`, `X & 0`, comparisons and casts of a folded
//! operand), which stays unfolded here and so retains more code than GCC.
//!
//! The same fold answers `__builtin_constant_p` under GCC's rules: an operand
//! without side effects that it folds to an integer, such as `i && 0`,
//! `i * 0` or `i && 0 ? i : 34`, is a constant, and one whose evaluated
//! part holds a comma, which GCC's C front end keeps as a `COMPOUND_EXPR`,
//! is not: `(1, 7)` and `1 + (1, 7)` are no constants to GCC, though Clang
//! evaluates them.
//! C99: §6.5.13 paragraphs 3-4, p. 89; PDF p. 101; §6.5.14 paragraphs 3-4,
//! pp. 89-90; PDF pp. 101-102; §6.5.15 paragraph 4, p. 90; PDF p. 102;
//! §6.5.17 paragraph 2, p. 94; PDF p. 106.
//! GNU extension: GCC manual, "Other Built-in Functions Provided by GCC",
//! `__builtin_constant_p`.

use super::super::Analyzer;
use crate::translation_phases::parsing::syntax::{
    BinaryOperator,
    Expression,
    ExpressionType,
    UnaryOperator,
};

/// How deep the fold follows nested operands; deeper conditions stay
/// unfolded, which only retains code.
const DEPTH: u32 = 64;

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// The truth value GCC's rules decide `condition` to, when code
    /// generation's own fold ([`Self::folded_condition`]) does not because
    /// the condition has side effects that still run: `d() && 0` is false
    /// and `d() || 1` true. A condition holding a label does not fold.
    /// C99: §6.5.13p3, p. 89; PDF p. 101; §6.5.14p3, p. 89; PDF p. 101.
    pub(in crate::translation_phases::semantic_analysis) fn gcc_decided_condition(
        &mut self,
        condition: &'tu Expression<'tu>,
    ) -> Option<bool> {
        if self.context.configuration.compiler_identity().rules()
            != crate::configuration::RuleSet::Gcc
            || self.folded_condition(condition).is_some()
            || self.expression_contains_label(condition)
        {
            return None;
        }
        self.gcc_truth(condition, 0)
    }

    /// What GCC answers for `__builtin_constant_p (operand)` where its fold
    /// decides it ([`GccConstant`]), or `None` to leave the answer to the
    /// evaluator Clang's rules share. GNU extension: GCC manual, "Other
    /// Built-in Functions Provided by GCC", `__builtin_constant_p`.
    pub(in crate::translation_phases::semantic_analysis) fn gcc_constant_query(
        &mut self,
        operand: &'tu Expression<'tu>,
    ) -> Option<bool> {
        if self.context.configuration.compiler_identity().rules()
            != crate::configuration::RuleSet::Gcc
            || self.has_side_effects(operand)
        {
            return None;
        }
        match self.gcc_constant(operand, 0) {
            | GccConstant::Folded => Some(true),
            | GccConstant::Comma => Some(false),
            | GccConstant::Unknown => None,
        }
    }

    /// Whether GCC's fold turns `e`, which has no side effects, into an
    /// integer constant, or keeps a comma it evaluates.
    fn gcc_constant(&mut self, e: &'tu Expression<'tu>, depth: u32) -> GccConstant {
        if depth == DEPTH {
            return GccConstant::Unknown;
        }
        let next = depth + 1;
        match e.kind {
            | ExpressionType::Parenthesized { expression }
            | ExpressionType::Unary {
                operator:
                    UnaryOperator::Extension
                    | UnaryOperator::LogicalNot
                    | UnaryOperator::Plus
                    | UnaryOperator::Minus
                    | UnaryOperator::BitwiseNot,
                operand_expression: expression,
            }
            | ExpressionType::Cast {
                operand_expression: expression,
                ..
            } => self
                .gcc_constant(expression, next)
                .or_truth(self.gcc_truth(e, next)),
            | ExpressionType::Binary {
                operator: BinaryOperator::Comma,
                ..
            } => GccConstant::Comma,
            | ExpressionType::Binary {
                operator: operator @ (BinaryOperator::LogicalAnd | BinaryOperator::LogicalOr),
                left_expression,
                right_expression,
            } => {
                let live = operator == BinaryOperator::LogicalAnd;
                let left = self.gcc_constant(left_expression, next);
                if left == GccConstant::Comma {
                    return left;
                }
                // The right operand matters only where the left one lets it
                // run, and is the value only after a constant left one.
                match self.gcc_truth(left_expression, next) {
                    | Some(truth) if truth != live => GccConstant::Folded,
                    | Some(_) => self
                        .gcc_constant(right_expression, next)
                        .or_truth(self.gcc_truth(e, next)),
                    | None => match self.gcc_constant(right_expression, next) {
                        | GccConstant::Comma => GccConstant::Comma,
                        | _ => GccConstant::Unknown.or_truth(self.gcc_truth(e, next)),
                    },
                }
            },
            | ExpressionType::Conditional(c) => {
                if self.gcc_constant(c.condition_expression, next) == GccConstant::Comma {
                    return GccConstant::Comma;
                }
                match self.gcc_truth(c.condition_expression, next) {
                    | Some(true) => self.gcc_constant(c.then_expression, next),
                    | Some(false) => self.gcc_constant(c.else_expression, next),
                    | None => GccConstant::Unknown,
                }
            },
            // `X * 0` and `X & 0` are zero, `X` having no side effects.
            | ExpressionType::Binary {
                operator: BinaryOperator::Multiplication | BinaryOperator::BitwiseAnd,
                left_expression,
                right_expression,
            } if [left_expression, right_expression].iter().any(|operand| {
                self.expression_info(operand)
                    .integer
                    .is_some_and(|value| value.is_zero())
            }) && self.integer_type(self.expression_info(e).ty).is_some() =>
            {
                let left = self.gcc_constant(left_expression, next);
                let right = self.gcc_constant(right_expression, next);
                if left == GccConstant::Comma || right == GccConstant::Comma {
                    GccConstant::Comma
                } else {
                    GccConstant::Folded
                }
            },
            | ExpressionType::Binary {
                left_expression,
                right_expression,
                ..
            } => {
                let left = self.gcc_constant(left_expression, next);
                let right = self.gcc_constant(right_expression, next);
                if left == GccConstant::Comma || right == GccConstant::Comma {
                    GccConstant::Comma
                } else {
                    GccConstant::Unknown
                }
            },
            | _ if self.expression_info(e).integer.is_some() => GccConstant::Folded,
            | _ => GccConstant::Unknown,
        }
    }

    /// GCC's fold of `e` to a truth value, with or without side effects.
    fn gcc_truth(&mut self, e: &'tu Expression<'tu>, depth: u32) -> Option<bool> {
        if let Some(truth) = self.folded_condition(e) {
            return Some(truth);
        }
        if depth == DEPTH {
            return None;
        }
        match e.kind {
            | ExpressionType::Parenthesized { expression }
            | ExpressionType::Unary {
                operator: UnaryOperator::Extension,
                operand_expression: expression,
            } => self.gcc_truth(expression, depth + 1),
            | ExpressionType::Unary {
                operator: UnaryOperator::LogicalNot,
                operand_expression,
            } => self
                .gcc_truth(operand_expression, depth + 1)
                .map(|truth| !truth),
            // The left operand runs; the right one is the value.
            | ExpressionType::Binary {
                operator: BinaryOperator::Comma,
                right_expression,
                ..
            } => self.gcc_truth(right_expression, depth + 1),
            | ExpressionType::Binary {
                operator: operator @ (BinaryOperator::LogicalAnd | BinaryOperator::LogicalOr),
                left_expression,
                right_expression,
            } => {
                // The value that lets `&&` (true) or `||` (false) go on to
                // its right operand.
                let live = operator == BinaryOperator::LogicalAnd;
                match self.gcc_truth(left_expression, depth + 1) {
                    | Some(truth) if truth != live => Some(truth),
                    | Some(_) => self.gcc_truth(right_expression, depth + 1),
                    // `X && 0` and `X || 1`: the right operand decides
                    // whenever `X` lets it run, and so does `X` otherwise.
                    | None => (self.gcc_truth(right_expression, depth + 1) == Some(!live))
                        .then_some(!live),
                }
            },
            | _ => None,
        }
    }
}

/// What GCC's fold makes of a `__builtin_constant_p` operand.
#[derive(Clone, Copy, PartialEq, Eq)]
enum GccConstant {
    /// An integer constant.
    Folded,
    /// A comma the operand evaluates, which GCC keeps.
    Comma,
    /// Anything else, left to the shared evaluator.
    Unknown,
}

impl GccConstant {
    /// `self`, or a constant where the enclosing truth value folds.
    fn or_truth(self, truth: Option<bool>) -> Self {
        match (self, truth) {
            | (Self::Comma, _) => Self::Comma,
            | (_, Some(_)) => Self::Folded,
            | (other, None) => other,
        }
    }
}
