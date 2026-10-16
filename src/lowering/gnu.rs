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
            Statement,
            StatementType,
        },
        semantic_analysis::{
            ConversionKind,
            TypeId,
        },
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
        let mut item = self.pop();
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
        let result = self.convert_value(item, ty, source)?;
        let value = self.value(result, source)?;
        let edges = [
            self.draft.edge(merge, &[value]),
            self.draft.edge(otherwise, &[]),
        ];
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
        statement: &'tu Statement<'tu>,
        ty: TypeId,
        mode: StatementMode,
    ) {
        let StatementType::Compound { items } = statement.kind else {
            self.tasks.push(Task::Statement(statement));
            if mode == StatementMode::Value {
                self.operands.push(Item {
                    operand: Operand::Void,
                    ty,
                });
            }
            return;
        };
        let tail =
            crate::translation_phases::semantic_analysis::statement_expression_tail(statement);
        if let Some((tail, _)) = tail {
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
        let prefix = if tail.is_some() {
            &items[..items.len() - 1]
        } else {
            items.as_slice()
        };
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
    }
}
