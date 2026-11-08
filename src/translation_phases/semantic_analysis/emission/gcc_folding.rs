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
//! C99: §6.5.13 paragraphs 3-4, p. 89; PDF p. 101; §6.5.14 paragraphs 3-4,
//! pp. 89-90; PDF pp. 101-102; §6.5.17 paragraph 2, p. 94; PDF p. 106.

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
