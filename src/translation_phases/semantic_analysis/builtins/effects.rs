//! Retained dual-context expression effect summaries, produced bottom-up.
//! ICE eligibility remains independent from absence of observable effects.
//! GNU extension: GCC Integer Overflow Builtins and Typeof.
//! <https://gcc.gnu.org/onlinedocs/gcc/Integer-Overflow-Builtins.html>
//! <https://gcc.gnu.org/onlinedocs/gcc/Typeof.html>
//! C99: §6.7.3p6, p. 109; PDF p. 121 (volatile accesses).

use super::super::{
    Analyzer,
    ExpressionInfo,
};
use crate::translation_phases::parsing::{
    declaration_syntax::TypeQualifiers,
    syntax::{
        BinaryOperator as B,
        Expression,
        ExpressionType as E,
        UnaryOperator as U,
    },
};

/// One summary for both value reads and designation/effects-only evaluation.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ExpressionEffects {
    pub(crate) value_pure:       bool,
    pub(crate) designation_pure: bool,
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    pub(in crate::translation_phases::semantic_analysis) fn summarize_effects(
        &self,
        result: ExpressionInfo<'tu>,
    ) -> ExpressionEffects {
        let summary = |read| {
            #[cfg(test)]
            crate::test_support::OVERFLOW_EFFECT_VISITS.with(|visits| visits.set(visits.get() + 1));
            let children = match dependencies(result.expression, read, &|e| {
                Some(if std::ptr::eq(e, result.expression) {
                    result
                } else {
                    self.expression_info(e)
                })
            }) {
                | Rule::Known(pure) => pure,
                | Rule::Children(children) => children
                    .into_iter()
                    .flatten()
                    .all(|(e, read)| self.effect_pure(e, read)),
            };
            let actions = self
                .expression_actions
                .get(&std::ptr::from_ref(result.expression).addr());
            children
                && actions.is_none_or(|site| {
                    site.operand_evaluations
                        .iter()
                        .all(|&e| self.effect_pure(e, true))
                        && site
                            .bounds
                            .iter()
                            .all(|&index| self.effect_pure(self.types.runtime_bounds[index], true))
                })
        };
        ExpressionEffects {
            value_pure:       summary(true),
            designation_pure: summary(false),
        }
    }

    pub(in crate::translation_phases::semantic_analysis) fn effect_pure(
        &self,
        e: &Expression<'_>,
        read: bool,
    ) -> bool {
        self.expression_indices
            .get(&std::ptr::from_ref(e).addr())
            .and_then(|&i| self.expression_effects.get(i))
            .is_some_and(|effects| {
                if read {
                    effects.value_pure
                } else {
                    effects.designation_pure
                }
            })
    }
}

enum Rule<'tu> {
    Known(bool),
    Children([Option<(&'tu Expression<'tu>, bool)>; 3]),
}

/// Inspect only the current node; child effects are combined by the explicit
/// postorder driver. Address-of and member-base reads retain their C meaning.
/// C99: §6.5.3.2p3, pp. 78-79; PDF pp. 90-91; §6.7.3p6, p. 109; PDF p. 121.
fn dependencies<'tu>(
    e: &'tu Expression<'tu>,
    read: bool,
    info: &impl Fn(&'tu Expression<'tu>) -> Option<ExpressionInfo<'tu>>,
) -> Rule<'tu> {
    let Some(result) = info(e) else {
        return Rule::Known(false);
    };
    if read && result.ty.qualifiers.contains(TypeQualifiers::VOLATILE) {
        return Rule::Known(false);
    }
    if result.ice && result.integer.is_some() {
        return Rule::Known(true);
    }
    let one = |e, read| Rule::Children([Some((e, read)), None, None]);
    match e.kind {
        | E::Identifier(_) | E::Constant(_) | E::StringLiteral(_) => Rule::Known(true),
        | E::SizeofExpr(_) | E::SizeofType(_) | E::AlignofExpr(_) | E::AlignofType(_)
            if result.ice =>
            Rule::Known(true),
        | E::Parenthesized { expression }
        | E::Cast {
            operand_expression: expression,
            ..
        } => one(expression, read),
        | E::Unary {
            operator: U::PreIncrement | U::PreDecrement | U::PostIncrement | U::PostDecrement,
            ..
        }
        | E::Binary {
            operator:
                B::Assignment
                | B::MultiplicationAssignment
                | B::DivisionAssignment
                | B::ModuloAssignment
                | B::AdditionAssignment
                | B::SubtractionAssignment
                | B::LeftShiftAssignment
                | B::RightShiftAssignment
                | B::BitwiseAndAssignment
                | B::BitwiseXorAssignment
                | B::BitwiseOrAssignment,
            ..
        } => Rule::Known(false),
        | E::Unary {
            operator,
            operand_expression,
        } => one(operand_expression, operator != U::AddressOf),
        | E::Binary {
            left_expression,
            right_expression,
            ..
        } => Rule::Children([
            Some((left_expression, true)),
            Some((right_expression, true)),
            None,
        ]),
        | E::Conditional(c) | E::OmittedConditional(c) => Rule::Children([
            Some((c.condition_expression, true)),
            Some((c.then_expression, true)),
            Some((c.else_expression, true)),
        ]),
        | E::DirectMember {
            base_expression, ..
        } => one(base_expression, false),
        | E::IndirectMember {
            base_expression, ..
        } => one(base_expression, true),
        | _ => Rule::Known(false),
    }
}
