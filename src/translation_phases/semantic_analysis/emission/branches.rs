//! Clang's `EmitBranchOnBoolExpr` differs from scalar logical emission:
//! only 1&&X, X&&1, 0||X and X||0 simplify. In particular a nested 0&&X
//! or 1||X still creates X's code when the enclosing condition cannot fold.
//! C99: §6.5.13-§6.5.15, pp. 89-91; PDF pp. 101-103;
//! §6.8.4.1p2, p. 133; PDF p. 145. Emission policy follows Clang.

use super::super::{
    Analyzer,
    DefinitionKind,
    Scalar,
    TypeKind,
    Work,
    functions::FunctionWork,
};
use crate::{
    translation_phases::parsing::syntax::{
        BinaryOperator,
        Expression,
        ExpressionType,
        UnaryOperator,
    },
    util::bump::ArenaVec,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Marks precisely the syntax through which branch emission recurses.
    pub(in crate::translation_phases::semantic_analysis) fn branch_condition(
        &mut self,
        expression: &'tu Expression<'tu>,
    ) {
        let mut work = ArenaVec::new_in(self.scratch);
        work.push(expression);
        while let Some(e) = work.pop() {
            let key = std::ptr::from_ref(e).addr();
            if self.functions.emission.branches.insert(key, None).is_some() {
                continue;
            }
            match e.kind {
                | ExpressionType::Parenthesized { expression }
                | ExpressionType::Unary {
                    operator: UnaryOperator::LogicalNot,
                    operand_expression: expression,
                } => work.push(expression),
                | ExpressionType::Binary {
                    operator: BinaryOperator::LogicalAnd | BinaryOperator::LogicalOr,
                    left_expression,
                    right_expression,
                } => {
                    work.extend([left_expression, right_expression]);
                },
                | ExpressionType::Conditional(c) =>
                    work.extend([c.condition_expression, c.then_expression, c.else_expression]),
                | _ => {},
            }
        }
    }

    /// Clang's scalar visitor alone folds a conditional's unselected arm.
    /// C99: §6.5.15p3-6, pp. 90-91; PDF pp. 102-103.
    pub(in crate::translation_phases::semantic_analysis) fn scalar_conditional_emission(
        &self,
        expression: &'tu Expression<'tu>,
    ) -> bool {
        let ty = self.expression_info(expression).ty;
        // Keep main's conservative VM emission until composite pointer
        // bounds follow the selected arm; eliding one exposes a missing bound.
        !self.types.variably_modified(ty)
            && (self.real(ty)
                || matches!(
                    self.types.nodes[ty.index],
                    TypeKind::Pointer(_) | TypeKind::Nullptr | TypeKind::Scalar(Scalar::Void)
                ))
    }

    pub(in crate::translation_phases::semantic_analysis) fn complete_conditional_elision(
        &mut self,
        expression: &'tu Expression<'tu>,
    ) {
        let (ExpressionType::Conditional(c) | ExpressionType::OmittedConditional(c)) =
            expression.kind
        else {
            return;
        };
        if self
            .functions
            .emission
            .branches
            .contains_key(&std::ptr::from_ref(expression).addr())
            || !self.scalar_conditional_emission(expression)
        {
            return;
        }
        let info = self.expression_info(c.condition_expression);
        let constant_context = self
            .functions
            .initializer
            .is_some_and(|binding| self.bindings[binding].duration.is_static_or_thread());
        let value = if info.runtime_query && constant_context {
            Self::constant_truth(info)
        } else {
            self.folded_condition(c.condition_expression)
        };
        let omitted = match value {
            | Some(true) => c.else_expression,
            | Some(false) if matches!(expression.kind, ExpressionType::OmittedConditional(_)) =>
                return,
            | Some(false) => c.then_expression,
            | None => return,
        };
        if !self.expression_contains_label(omitted) {
            if let Some(&index) = self
                .expression_indices
                .get(&std::ptr::from_ref(omitted).addr())
            {
                self.expressions[index].elided = true;
            }
            if matches!(expression.kind, ExpressionType::Conditional(_)) {
                self.elide_condition_references(c.condition_expression);
            }
        }
    }

    pub(in crate::translation_phases::semantic_analysis) fn begin_branch_operand(
        &mut self,
        e: &'tu Expression<'tu>,
    ) -> bool {
        if !self
            .functions
            .emission
            .branches
            .contains_key(&std::ptr::from_ref(e).addr())
        {
            return false;
        }
        let start = self.emission_position();
        self.work
            .push(Work::FunctionWork(FunctionWork::BranchOperandEnd(e, start)));
        true
    }

    pub(in crate::translation_phases::semantic_analysis) fn end_branch_operand(
        &mut self,
        e: &'tu Expression<'tu>,
        start: Position,
    ) {
        let end = self.emission_position();
        _ = self
            .functions
            .emission
            .branches
            .insert(std::ptr::from_ref(e).addr(), Some((start, end)));
        if let ExpressionType::Binary {
            operator,
            left_expression,
            right_expression,
        } = e.kind
        {
            let live = match operator {
                | BinaryOperator::LogicalAnd => true,
                | BinaryOperator::LogicalOr => false,
                | _ => return,
            };
            let (omitted, retained) = if self.folded_condition(left_expression) == Some(live) {
                (left_expression, right_expression)
            } else if self.folded_condition(right_expression) == Some(live) {
                (right_expression, left_expression)
            } else {
                return;
            };
            self.elide_branch_operand(omitted);
            self.retain_branch_operand(e, retained);
        }
    }

    pub(in crate::translation_phases::semantic_analysis) fn emission_position(&self) -> Position {
        Position {
            references:  self.reference_count(),
            definitions: self.definitions.len(),
            statements:  self.control.statements.len(),
        }
    }

    fn elide_branch_operand(&mut self, e: &'tu Expression<'tu>) {
        let Some(Some((start, end))) = self
            .functions
            .emission
            .branches
            .get(&std::ptr::from_ref(e).addr())
            .copied()
        else {
            return;
        };
        self.elide_references(start.references..end.references);
        for definition in &mut self.definitions[start.definitions..end.definitions] {
            if definition.kind == DefinitionKind::Object {
                definition.elided = true;
            }
        }
        for statement in &mut self.control.statements[start.statements..end.statements] {
            statement.elided = true;
        }
        if let Some(&index) = self.expression_indices.get(&std::ptr::from_ref(e).addr()) {
            self.expressions[index].elided = true;
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::translation_phases::semantic_analysis) struct Position {
    pub(in crate::translation_phases::semantic_analysis) references:  usize,
    pub(in crate::translation_phases::semantic_analysis) definitions: usize,
    pub(in crate::translation_phases::semantic_analysis) statements:  usize,
}
