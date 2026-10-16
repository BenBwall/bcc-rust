//! Effects-only evaluation interprets the recipes semantic analysis retained.
//! Overflow predicates ignore their third operand's value but keep its
//! effects, including those of statement expressions in it. This file never
//! reclassifies operators, volatility or selections: each task reads one
//! retained recipe and schedules its uses, branches or block on the explicit
//! task stack. GNU extension: GCC "Integer Overflow Builtins".
//! <https://gcc.gnu.org/onlinedocs/gcc/Integer-Overflow-Builtins.html>
//! C99: §5.1.2.3p2, p. 13; PDF p. 25 (side effects).

use super::{
    FunctionLowerer,
    LoweringError,
    Task,
};
use crate::translation_phases::{
    parsing::syntax::Expression,
    semantic_analysis::{
        EffectContext,
        EffectRecipe,
        ExprId,
    },
};

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    /// Schedules one expression's retained effects-only recipe. Each task
    /// leaves no item, so nested syntax still uses one explicit stack. A
    /// missing recipe is a provenance-bearing `MissingFact`.
    pub(super) fn builtin_effects(
        &mut self,
        expression: &'tu Expression<'tu>,
        context: EffectContext,
    ) -> Result<(), LoweringError> {
        #[cfg(test)]
        crate::test_support::EFFECT_RECIPE_STEPS.with(|steps| steps.set(steps.get() + 1));
        let sema = self.unit.sema;
        let recipe = sema.effect_recipe(expression, context).ok_or_else(|| {
            LoweringError::missing(
                "expression effect recipe is not retained",
                expression.source_vectors,
            )
        })?;
        let syntax = |id: ExprId| {
            sema.expressions
                .get(id.0)
                .map(|info| info.expression)
                .ok_or_else(|| {
                    LoweringError::missing(
                        "an effect recipe names an expression that is not retained",
                        expression.source_vectors,
                    )
                })
        };
        match recipe {
            | EffectRecipe::None => return Ok(()),
            | EffectRecipe::EvaluateValue(id) => {
                self.tasks.push(Task::Discard);
                self.tasks.push(Task::Evaluate(syntax(id)?));
                return Ok(());
            },
            | EffectRecipe::StatementBlock(_) => {
                self.statement_expression(expression, super::StatementMode::Effects)?;
            },
            | EffectRecipe::Sequence(uses) =>
                for effect in uses.iter().rev() {
                    self.tasks.push(Task::BuiltinEffects(
                        syntax(effect.expression)?,
                        effect.context,
                    ));
                },
            | EffectRecipe::Conditional {
                condition,
                then,
                otherwise,
            } => {
                let condition = syntax(condition)?;
                let then = then
                    .map(|effect| syntax(effect.expression).map(|e| (e, effect.context)))
                    .transpose()?;
                let otherwise = otherwise
                    .map(|effect| syntax(effect.expression).map(|e| (e, effect.context)))
                    .transpose()?;
                self.effect_branches(condition, then, otherwise);
            },
        }
        // The expression's own type-site actions run before its uses.
        self.prepare_expression_actions(expression)?;
        Ok(())
    }

    /// Branches once on `condition`; an arm without effects goes straight
    /// to the merge block. C99: §6.5.13p4, p. 89; PDF p. 101; §6.5.14p4,
    /// p. 89; PDF p. 101; §6.5.15p4, p. 90; PDF p. 102.
    fn effect_branches(
        &mut self,
        condition: &'tu Expression<'tu>,
        then: Option<(&'tu Expression<'tu>, EffectContext)>,
        otherwise: Option<(&'tu Expression<'tu>, EffectContext)>,
    ) {
        if let (Some(then_effects), Some(otherwise_effects)) = (then, otherwise) {
            let then = self.draft.create_block(&[]);
            let otherwise = self.draft.create_block(&[]);
            let merge = self.draft.create_block(&[]);
            self.push_all(&[
                Task::Condition {
                    expression: condition,
                    then,
                    otherwise,
                },
                Task::Enter {
                    block: then,
                    seal:  true,
                },
                Task::BuiltinEffects(then_effects.0, then_effects.1),
                Task::JumpTo {
                    block: merge,
                    seal:  false,
                },
                Task::Enter {
                    block: otherwise,
                    seal:  true,
                },
                Task::BuiltinEffects(otherwise_effects.0, otherwise_effects.1),
                Task::JumpTo {
                    block: merge,
                    seal:  false,
                },
                Task::Enter {
                    block: merge,
                    seal:  true,
                },
            ]);
            return;
        }
        let Some((arm, context)) = then.or(otherwise) else {
            return;
        };
        let effects = self.draft.create_block(&[]);
        let merge = self.draft.create_block(&[]);
        let (then, otherwise) = if then.is_some() {
            (effects, merge)
        } else {
            (merge, effects)
        };
        self.push_all(&[
            Task::Condition {
                expression: condition,
                then,
                otherwise,
            },
            Task::Enter {
                block: effects,
                seal:  true,
            },
            Task::BuiltinEffects(arm, context),
            Task::JumpTo {
                block: merge,
                seal:  false,
            },
            Task::Enter {
                block: merge,
                seal:  true,
            },
        ]);
    }
}
