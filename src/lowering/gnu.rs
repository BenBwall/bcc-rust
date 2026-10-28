//! Cheap GNU control flow extensions over the existing explicit task stack.
//! GNU extension: GCC "Conditionals", "Statement Exprs", and "Case Ranges".
//! <https://gcc.gnu.org/onlinedocs/gcc/Conditionals.html>
//! <https://gcc.gnu.org/onlinedocs/gcc/Statement-Exprs.html>
//! <https://gcc.gnu.org/onlinedocs/gcc/Case-Ranges.html>
//! C99: §6.5.15 paragraph 4, p. 90; PDF p. 102 (conditional sequencing).

use super::{
    FunctionLowerer,
    Item,
    LoweringError,
    Operand,
    StatementMode,
    Task,
    ssa::Terminator,
};
use crate::{
    ir::Block,
    translation_phases::{
        parsing::syntax::{
            BlockItem,
            ConditionalExpression,
            Expression,
            StatementType,
        },
        semantic_analysis::ConversionKind,
    },
};

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    /// Tests the saved first value before converting it to the result type.
    /// Thus a float/pointer condition is not narrowed before its truth test.
    /// GNU extension: GCC "Conditionals"; C99: §6.5.15p4, p. 90; PDF p. 102.
    pub(super) fn omitted_conditional(
        &mut self,
        e: &'tu Expression<'tu>,
        c: &'tu ConditionalExpression<'tu>,
        otherwise: Block,
        merge: Block,
    ) -> Result<(), LoweringError> {
        let source = e.source_vectors;
        let mut item = self.pop_truth();
        // Only lvalue conversion and decay precede the truth test. Sema's
        // arithmetic conversions belong to the saved true result afterward.
        for conversion in self.conversions(c.condition_expression) {
            match (conversion.kind, item.operand) {
                | (ConversionKind::Lvalue, Operand::Place(place)) =>
                    item = self.read(place, item.ty, conversion.ty, source)?,
                | (
                    ConversionKind::ArrayDecay | ConversionKind::FunctionDecay,
                    Operand::Place(place),
                ) => {
                    let value = match place {
                        | super::Place::Memory(value) => value,
                        | super::Place::Function(func) =>
                            self.inst(crate::ir::InstData::FuncAddr { func }, crate::ir::Type::Ptr),
                        | _ =>
                            return Err(LoweringError::missing(
                                "a decayed condition has no address",
                                source,
                            )),
                    };
                    item = Item {
                        operand: Operand::Value(value),
                        ty:      conversion.ty,
                    };
                },
                | _ => {},
            }
        }
        let truth = self.truth(item, source)?;
        let ty = self.info(e)?.ty;
        // A void alternative makes the whole expression void (GNU), so the
        // tested value is discarded rather than passed to the merge.
        let true_edge = if self.repr(ty, source)? == super::types::Repr::Void {
            self.draft.edge(merge, &[])
        } else {
            let result = self.convert_value(item, ty, source)?;
            // GCC's `_Float16` result keeps its excess in an `f32` parameter.
            let value = if self.half(ty).is_some() && self.half_in_float() {
                self.excess(result, source)?
            } else {
                let result = self.settle(result);
                self.value(result, source)?
            };
            self.draft.edge(merge, &[value])
        };
        let edges = [true_edge, self.draft.edge(otherwise, &[])];
        self.draft.terminate(Terminator::Brif(truth, edges));
        self.draft.seal(otherwise);
        self.draft.switch_to(otherwise);
        self.tasks.push(Task::Merge {
            merge,
            ty,
            truth: false,
        });
        self.tasks.push(Task::JumpWithValue {
            merge,
            truth: false,
        });
        self.tasks.push(Task::Evaluate(c.else_expression));
        Ok(())
    }

    /// Executes a block for its final value or only for observable effects.
    /// Non-expression endings have void type. No recursive syntax walk.
    /// GNU extension: GCC "Statement Exprs".
    /// C99: §6.3.2.1p2-4, p. 46; PDF p. 58 (tail value conversions).
    pub(super) fn statement_expression(
        &mut self,
        expression: &'tu Expression<'tu>,
        mode: StatementMode,
    ) -> Result<(), LoweringError> {
        let resolved = self
            .unit
            .sema
            .statement_expression_facts(expression)
            .ok_or_else(|| {
                LoweringError::missing(
                    "statement-expression facts are not retained",
                    expression.source_vectors,
                )
            })?;
        let statement = resolved.block;
        let ty = resolved.result_type;
        let StatementType::Compound { items } = statement.kind else {
            return Err(LoweringError::missing(
                "a statement expression has no block",
                expression.source_vectors,
            ));
        };
        let facts = self.unit.sema.control.statement(statement).ok_or_else(|| {
            LoweringError::missing(
                "statement-expression control facts are not retained",
                statement.source_vectors,
            )
        })?;
        self.tasks.push(Task::CleanupNormal(statement));
        if let (Some(tail), Some(expression)) = (resolved.outer_tail, resolved.value_tail)
            && !self
                .unit
                .sema
                .control
                .statement(tail)
                .is_some_and(|facts| facts.elided)
        {
            if mode == StatementMode::Value && self.transition_has_cleanup(facts.exit) {
                self.tasks.push(Task::SnapshotArgument(expression));
            }
            // GCC and Clang both round a `_Float16` tail to its type.
            if mode == StatementMode::Value && self.excess_type(ty) {
                self.tasks.push(Task::Settle);
            }
            self.tasks.push(if mode == StatementMode::Effects {
                Task::StatementEffects(tail)
            } else {
                Task::StatementValue(tail)
            });
        } else if mode == StatementMode::Value {
            self.operands.push(Item {
                operand: Operand::Void,
                ty,
            });
        }
        let prefix = items.as_slice().get(..resolved.prefix).ok_or_else(|| {
            LoweringError::missing(
                "a statement expression has no retained prefix",
                expression.source_vectors,
            )
        })?;
        for item in prefix.iter().rev() {
            match *item {
                | BlockItem::Statement(s) => self.tasks.push(if mode == StatementMode::Effects {
                    Task::StatementEffects(s)
                } else {
                    Task::Statement(s)
                }),
                | BlockItem::Declaration(d) => self.tasks.push(Task::Declaration(d)),
                | BlockItem::FunctionDefinition(_) => {
                    self.tasks.push(Task::Statement(statement));
                    break;
                },
            }
        }
        Ok(())
    }
}
