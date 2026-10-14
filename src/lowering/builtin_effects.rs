//! Overflow predicates ignore their third operand's value, retaining effects.
//! GNU extension: GCC "Integer Overflow Builtins".
//! <https://gcc.gnu.org/onlinedocs/gcc/Integer-Overflow-Builtins.html>

use super::{
    FunctionLowerer,
    LoweringError,
    Task,
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

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    /// Schedules only observable effects. Nonvolatile unused reads and value
    /// arithmetic are absent, even alongside an increment or a call. A branch
    /// condition is evaluated only when it selects different effects. Each
    /// task leaves no item, so nested syntax still uses one explicit stack.
    pub(super) fn builtin_effects(
        &mut self,
        expression: &'tu Expression<'tu>,
        read: bool,
    ) -> Result<(), LoweringError> {
        if self.info(expression)?.ice {
            return Ok(());
        }
        if read
            && self
                .info(expression)?
                .ty
                .qualifiers
                .contains(TypeQualifiers::VOLATILE)
        {
            self.tasks.push(Task::Discard);
            self.tasks.push(Task::Evaluate(expression));
            return Ok(());
        }
        let sema = self.unit.sema;
        let pure = |e: &'tu Expression<'tu>| {
            sema.effect_pure(e, true).ok_or_else(|| {
                LoweringError::missing(
                    "expression effect summary is not retained",
                    e.source_vectors,
                )
            })
        };
        match expression.kind {
            | E::Parenthesized { expression }
            | E::Cast {
                operand_expression: expression,
                ..
            } => self.tasks.push(Task::BuiltinEffects(expression, read)),
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
            }
            | E::Call { .. } => {
                self.tasks.push(Task::Discard);
                self.tasks.push(Task::Evaluate(expression));
            },
            | E::Unary {
                operator,
                operand_expression,
            } => self.tasks.push(Task::BuiltinEffects(
                operand_expression,
                operator != U::AddressOf,
            )),
            | E::Binary {
                operator: operator @ (B::LogicalAnd | B::LogicalOr),
                left_expression,
                right_expression,
            } =>
                if pure(right_expression)? {
                    self.tasks.push(Task::BuiltinEffects(left_expression, true));
                } else {
                    let right = self.draft.create_block(&[]);
                    let merge = self.draft.create_block(&[]);
                    self.tasks.push(Task::Enter {
                        block: merge,
                        seal:  true,
                    });
                    self.tasks.push(Task::JumpTo {
                        block: merge,
                        seal:  false,
                    });
                    self.tasks
                        .push(Task::BuiltinEffects(right_expression, true));
                    self.tasks.push(Task::Enter {
                        block: right,
                        seal:  true,
                    });
                    let (then, otherwise) = if operator == B::LogicalAnd {
                        (right, merge)
                    } else {
                        (merge, right)
                    };
                    self.tasks.push(Task::Condition {
                        expression: left_expression,
                        then,
                        otherwise,
                    });
                },
            | E::Binary {
                left_expression,
                right_expression,
                ..
            } => {
                self.tasks
                    .push(Task::BuiltinEffects(right_expression, true));
                self.tasks.push(Task::BuiltinEffects(left_expression, true));
            },
            | E::Conditional(c) | E::OmittedConditional(c) => {
                let omitted = matches!(expression.kind, E::OmittedConditional(_));
                if (omitted || pure(c.then_expression)?) && pure(c.else_expression)? {
                    self.tasks
                        .push(Task::BuiltinEffects(c.condition_expression, true));
                } else {
                    let then = self.draft.create_block(&[]);
                    let otherwise = self.draft.create_block(&[]);
                    let merge = self.draft.create_block(&[]);
                    self.tasks.push(Task::Enter {
                        block: merge,
                        seal:  true,
                    });
                    self.tasks.push(Task::JumpTo {
                        block: merge,
                        seal:  false,
                    });
                    self.tasks
                        .push(Task::BuiltinEffects(c.else_expression, true));
                    self.tasks.push(Task::Enter {
                        block: otherwise,
                        seal:  true,
                    });
                    self.tasks.push(Task::JumpTo {
                        block: merge,
                        seal:  false,
                    });
                    if !omitted {
                        self.tasks
                            .push(Task::BuiltinEffects(c.then_expression, true));
                    }
                    self.tasks.push(Task::Enter {
                        block: then,
                        seal:  true,
                    });
                    self.tasks.push(Task::Condition {
                        expression: c.condition_expression,
                        then,
                        otherwise,
                    });
                }
            },
            | E::DirectMember {
                base_expression, ..
            } => self
                .tasks
                .push(Task::BuiltinEffects(base_expression, false)),
            | E::IndirectMember {
                base_expression, ..
            } => self.tasks.push(Task::BuiltinEffects(base_expression, true)),
            | E::StatementExpression(_) => {
                self.statement_expression(expression, super::StatementMode::Effects)?;
            },
            | E::Builtin(_) | E::Generic(_) => {
                if let Some(selected) = self.info(expression)?.selected_expression {
                    self.tasks.push(Task::BuiltinEffects(selected, read));
                } else {
                    self.tasks.push(Task::Discard);
                    self.tasks.push(Task::Evaluate(expression));
                }
            },
            | E::Identifier(_) | E::Constant(_) | E::StringLiteral(_) => {},
            | _ => {
                self.tasks.push(Task::Discard);
                self.tasks.push(Task::Evaluate(expression));
            },
        }
        self.prepare_expression_actions(expression)?;
        Ok(())
    }
}
