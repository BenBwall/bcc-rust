//! `-Wbool-operation`: `~` applied to a truth value, whose result is never
//! zero for a `_Bool` and is the truth value's complement nowhere, and GCC's
//! `++` and `--` of a `_Bool`, which only set or flip it. C99 §6.5.3.3p4
//! defines `~`, and §6.5.2.4p2 and §6.5.3.1p2 the increments, so each is a
//! warning §5.1.1.3p1 permits.
//!
//! GCC reports both as it builds the operator (`build_unary_op` in
//! `c-typeck.cc`), and notes `did you mean to use logical not?` after a `~`.
//! Clang reports only the `~`, as it walks the finished full expression
//! (`AnalyzeImplicitConversions` in `SemaChecking.cpp`): each `~` it meets
//! as an operand, with that operand's range, and in the words that its
//! result is always true when the operand converts to `_Bool`. A `~` waits
//! as a [`PendingNegation`] until whatever takes its value is known.
//! C99: §6.5.3.3p4, p. 79; PDF p. 91; §6.5.2.4p2, p. 75; PDF p. 87;
//! §6.5.3.1p2, p. 78; PDF p. 90; §5.1.1.3p1, p. 11; PDF p. 23.

use super::{
    Analyzer,
    ExpressionInfo,
    SemanticErrorKind,
    types::{
        Scalar,
        TypeKind,
    },
};
use crate::{
    configuration::{
        DiagnosticGroup as G,
        RuleSet,
    },
    translation_phases::{
        SourceVectors,
        parsing::syntax::{
            BinaryOperator as B,
            Expression,
            ExpressionType as E,
            UnaryOperator as U,
        },
    },
    util::bump::ArenaVec,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Checks the unary operator `e`, applied to `operand`, as it is built.
    /// GCC reports `++`, `--` and `~` now; Clang's `~` waits for what takes
    /// its value.
    pub(super) fn check_bool_operation(
        &mut self,
        e: &'tu Expression<'tu>,
        operator: U,
        operand: ExpressionInfo<'tu>,
    ) {
        if !self.unused.bool_operation || self.tainted {
            return;
        }
        let site = e.operator_source_vectors.unwrap_or(e.source_vectors);
        let gcc = self.context.configuration.compiler_identity().rules() == RuleSet::Gcc;
        match operator {
            | U::PreIncrement | U::PostIncrement | U::PreDecrement | U::PostDecrement
                if gcc && self.bool_typed(operand.ty) =>
            {
                let rule = if matches!(operator, U::PreIncrement | U::PostIncrement) {
                    BoolOperationRule::GccIncrement
                } else {
                    BoolOperationRule::GccDecrement
                };
                self.bool_operation_warning(rule, site, None);
            },
            | U::BitwiseNot if gcc && self.gcc_complements_truth(operand.expression) =>
                self.bool_operation_warning(BoolOperationRule::GccBitwiseNot, site, Some(site)),
            // Clang's walk stays out of `sizeof` and its kin.
            | U::BitwiseNot
                if !gcc
                    && !self.unevaluated_operand()
                    && self.clang_known_boolean(operand.expression) =>
                self.bool_negations.push(PendingNegation {
                    negation: e,
                    holder:   e,
                }),
            | _ => {},
        }
    }

    /// Clang's `Expr::isKnownToHaveBooleanValue`: of type `_Bool`, or an
    /// integer whose value is 0 or 1 by its form: a comparison, `!`, `&&`
    /// or `||`, a bitwise operator or a conditional whose operands all are,
    /// or an assignment or comma whose right operand is. An explicit cast
    /// to another type hides what its operand is.
    fn clang_known_boolean(&self, e: &'tu Expression<'tu>) -> bool {
        // Every operand still to be found boolean.
        let mut work = ArenaVec::new_in(self.scratch);
        work.push(e);
        while let Some(e) = work.pop() {
            let e = Self::without_parentheses(e);
            let ty = self.expression_info(e).ty;
            if self.bool_typed(ty) {
                continue;
            }
            if self.integer_type(ty).is_none() {
                return false;
            }
            match e.kind {
                | E::Unary {
                    operator: U::LogicalNot,
                    ..
                }
                | E::Binary {
                    operator:
                        B::Equal
                        | B::NotEqual
                        | B::LessThan
                        | B::GreaterThan
                        | B::LessThanOrEqual
                        | B::GreaterThanOrEqual
                        | B::LogicalAnd
                        | B::LogicalOr,
                    ..
                } => {},
                | E::Unary {
                    operator: U::Plus,
                    operand_expression,
                } => work.push(operand_expression),
                | E::Binary {
                    operator: B::BitwiseAnd | B::BitwiseXor | B::BitwiseOr,
                    left_expression,
                    right_expression,
                } => {
                    work.push(left_expression);
                    work.push(right_expression);
                },
                | E::Binary {
                    operator: B::Comma | B::Assignment,
                    right_expression,
                    ..
                } => work.push(right_expression),
                | E::Conditional(c) => {
                    work.push(c.then_expression);
                    work.push(c.else_expression);
                },
                | _ => return false,
            }
        }
        true
    }

    /// Gives the waiting `~` its parentheses when `e` is them: Clang's range
    /// is the operand its walk meets, which they are.
    pub(super) fn note_negation_parentheses(&mut self, e: &'tu Expression<'tu>) {
        if let E::Parenthesized { expression } = e.kind
            && let Some(pending) = self
                .bool_negations
                .iter_mut()
                .find(|pending| std::ptr::eq(pending.holder, expression))
        {
            pending.holder = e;
        }
    }

    /// Reports the waiting `~` that `value` is, which converts to `target`:
    /// always true when `target` is `_Bool`.
    pub(super) fn convert_negation(&mut self, value: &'tu Expression<'tu>, bool_target: bool) {
        if let Some(index) = self
            .bool_negations
            .iter()
            .position(|pending| std::ptr::eq(pending.holder, value))
        {
            let pending = self.bool_negations.swap_remove(index);
            self.report_negation(pending, bool_target);
        }
    }

    /// Reports each waiting `~` but `keep`, whose value nothing converts to
    /// `_Bool`: the expression that took it is done.
    pub(super) fn flush_negations(&mut self, keep: Option<&'tu Expression<'tu>>) {
        let mut index = 0;
        while index < self.bool_negations.len() {
            let pending = self.bool_negations[index];
            if keep.is_some_and(|keep| std::ptr::eq(pending.holder, keep)) {
                index += 1;
            } else {
                _ = self.bool_negations.remove(index);
                self.report_negation(pending, false);
            }
        }
    }

    fn report_negation(&mut self, pending: PendingNegation<'tu>, bool_target: bool) {
        let rule = if bool_target {
            BoolOperationRule::ClangBitwiseNotTrue
        } else {
            BoolOperationRule::ClangBitwiseNot
        };
        let site = pending
            .negation
            .operator_source_vectors
            .unwrap_or(pending.negation.source_vectors);
        self.with_highlights(
            [pending.holder.source_vectors, SourceVectors::default()],
            |analyzer| analyzer.bool_operation_warning(rule, site, None),
        );
    }

    fn bool_operation_warning(
        &mut self,
        rule: BoolOperationRule,
        site: SourceVectors,
        note: Option<SourceVectors>,
    ) {
        self.group_diagnostic(
            G::BoolOperation,
            SemanticErrorKind::BoolOperation(rule),
            site,
            None,
            note,
        );
    }

    fn bool_typed(&self, ty: super::types::TypeId) -> bool {
        self.types.nodes[self.types.non_atomic(ty).index] == TypeKind::Scalar(Scalar::Bool)
    }
}

/// A `~` Clang reports once what takes its value is known, and the operand
/// that holds it: the `~` itself or the parentheses around it.
#[derive(Clone, Copy)]
pub(super) struct PendingNegation<'tu> {
    negation: &'tu Expression<'tu>,
    holder:   &'tu Expression<'tu>,
}

/// One `-Wbool-operation` warning, in its compiler's words.
/// C99: §5.1.1.3p1, p. 11; PDF p. 23.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum BoolOperationRule {
    GccIncrement,
    GccDecrement,
    GccBitwiseNot,
    ClangBitwiseNot,
    /// Of an operand converted to `_Bool`.
    ClangBitwiseNotTrue,
}

impl BoolOperationRule {
    pub(super) const fn words(self) -> &'static str {
        match self {
            | Self::GccIncrement => "increment of a boolean expression",
            | Self::GccDecrement => "decrement of a boolean expression",
            | Self::GccBitwiseNot => "'~' on a boolean expression",
            | Self::ClangBitwiseNot =>
                "bitwise negation of a boolean expression; did you mean logical negation?",
            | Self::ClangBitwiseNotTrue =>
                "bitwise negation of a boolean expression always evaluates to 'true'; did you mean \
                 logical negation?",
        }
    }
}
