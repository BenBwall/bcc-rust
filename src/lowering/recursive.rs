//! Experimental recursive traversal using the existing IR builders. Common
//! expressions return their `Item` through Rust calls. Compound statements
//! visit siblings in a loop. Remaining constructs reuse the existing tasks,
//! draining only the work scheduled by that construct before returning.
//!
//! The task seam retains extensions, VLA entry actions, cleanup and builtin
//! recipes without rewriting their semantics. It is a mixed prototype, not
//! a replacement for every task or operand buffer. The caller must supply a
//! sufficiently large native stack; the command-line study feature does so.
//!
//! C99: §6.5.16.2 paragraph 3, p. 93; PDF p. 105 (compound assignment);
//! §6.8.2, p. 132; PDF p. 144 (block order).

use super::{
    FunctionLowerer,
    LoweringError,
    StatementMode,
    Task,
    expressions::compound_operator,
    work::Item,
};
use crate::translation_phases::parsing::syntax::{
    BinaryOperator,
    BlockItem,
    Expression,
    ExpressionType,
    Statement,
    StatementType,
    UnaryOperator,
};

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    /// Starts recursive traversal from the function's prepared entry tasks.
    pub(super) fn run_recursive(&mut self) -> Result<(), LoweringError> {
        self.drain_recursive(0)?;
        debug_assert!(self.operands.is_empty(), "every operand is consumed");
        Ok(())
    }

    /// Executes one construct without consuming a caller's pending tasks.
    fn recursive_task(&mut self, task: Task<'tu>) -> Result<(), LoweringError> {
        self.recursion.enter();
        let floor = self.tasks.len();
        let result = match task {
            | Task::Evaluate(expression) => self
                .recursive_expression(expression, true)
                .map(|item| self.operands.push(item)),
            | Task::EvaluateRaw(expression) => self
                .recursive_expression(expression, false)
                .map(|item| self.operands.push(item)),
            | Task::Statement(statement) =>
                self.recursive_statement(statement, StatementMode::Discard),
            | Task::StatementValue(statement) =>
                self.recursive_statement(statement, StatementMode::Value),
            | Task::StatementEffects(statement) =>
                self.recursive_statement(statement, StatementMode::Effects),
            | task => self.step(task).and_then(|()| self.drain_recursive(floor)),
        };
        self.recursion.leave();
        result
    }

    /// A loop over siblings keeps width from becoming native recursion depth.
    fn drain_recursive(&mut self, floor: usize) -> Result<(), LoweringError> {
        while self.tasks.len() > floor {
            let task = self
                .tasks
                .pop()
                .expect("scheduled child above caller floor");
            self.recursive_task(task)?;
        }
        Ok(())
    }

    /// Returns a child result directly; shared builders still use their small
    /// operand seam when completing an operator or applying conversions.
    fn recursive_expression(
        &mut self,
        expression: &'tu Expression<'tu>,
        converted: bool,
    ) -> Result<Item, LoweringError> {
        self.recursion.enter();
        let result = self.recursive_expression_inner(expression, converted);
        self.recursion.leave();
        result
    }

    fn recursive_expression_inner(
        &mut self,
        expression: &'tu Expression<'tu>,
        converted: bool,
    ) -> Result<Item, LoweringError> {
        use ExpressionType as E;
        let floor = self.tasks.len();
        let info = self.info(expression)?;
        // Preserve the machine's early folding and refusal behavior exactly.
        let common = match expression.kind {
            | E::Parenthesized { .. }
            | E::Generic(_)
            | E::Unary { .. }
            | E::Cast { .. }
            | E::DirectMember { .. }
            | E::IndirectMember { .. } => true,
            | E::Binary { operator, .. } => !matches!(
                operator,
                BinaryOperator::LogicalAnd | BinaryOperator::LogicalOr
            ),
            | _ => false,
        };
        let item = if common
            && !info.ice
            && info.floating().is_none()
            && !self.unit.sema.types.unanalyzed(info.ty)
        {
            // Entry actions were pushed last by evaluate_raw, so they precede
            // child evaluation. This includes bounds with observable effects.
            self.prepare_expression_actions(expression)?;
            self.drain_recursive(floor)?;
            match expression.kind {
                | E::Parenthesized { expression: inner }
                | E::Unary {
                    operator: UnaryOperator::Extension,
                    operand_expression: inner,
                } => self.recursive_expression(inner, true)?,
                | E::Generic(_) => {
                    let selected = info.selected_expression.ok_or_else(|| {
                        LoweringError::missing(
                            "a selection has no operand",
                            expression.source_vectors,
                        )
                    })?;
                    self.recursive_expression(selected, true)?
                },
                | E::Binary {
                    operator,
                    left_expression,
                    right_expression,
                } => {
                    match operator {
                        | BinaryOperator::Comma => {
                            let _ = self.recursive_expression(left_expression, true)?;
                            self.recursive_expression(right_expression, true)?
                        },
                        | operator if compound_operator(operator).is_some() => {
                            // RHS first, then evaluate/read the lvalue once.
                            let right = self.recursive_expression(right_expression, true)?;
                            let place = self.recursive_expression(left_expression, false)?;
                            self.operands.push(place);
                            self.read_for_update(left_expression)?;
                            let old = self.pop_excess();
                            self.operands.extend_from_slice(&[right, place, old]);
                            self.finish(expression)?;
                            self.pop()
                        },
                        | _ => {
                            let left = self.recursive_expression(
                                left_expression,
                                operator != BinaryOperator::Assignment,
                            )?;
                            let right = self.recursive_expression(right_expression, true)?;
                            self.operands.extend_from_slice(&[left, right]);
                            self.finish(expression)?;
                            self.pop()
                        },
                    }
                },
                | E::Unary {
                    operator,
                    operand_expression,
                } => {
                    let raw = matches!(
                        operator,
                        UnaryOperator::AddressOf
                            | UnaryOperator::Real
                            | UnaryOperator::Imag
                            | UnaryOperator::PreIncrement
                            | UnaryOperator::PreDecrement
                            | UnaryOperator::PostIncrement
                            | UnaryOperator::PostDecrement
                    );
                    let operand = self.recursive_expression(operand_expression, !raw)?;
                    self.operands.push(operand);
                    if matches!(
                        operator,
                        UnaryOperator::PreIncrement
                            | UnaryOperator::PreDecrement
                            | UnaryOperator::PostIncrement
                            | UnaryOperator::PostDecrement
                    ) {
                        self.operands.push(operand);
                        self.read_for_update(operand_expression)?;
                    }
                    self.finish(expression)?;
                    self.pop()
                },
                | E::DirectMember {
                    base_expression, ..
                } => {
                    let base = self.recursive_expression(base_expression, false)?;
                    self.operands.push(base);
                    self.finish(expression)?;
                    self.pop()
                },
                | E::IndirectMember {
                    base_expression, ..
                }
                | E::Cast {
                    operand_expression: base_expression,
                    ..
                } => {
                    let base = self.recursive_expression(base_expression, true)?;
                    self.operands.push(base);
                    self.finish(expression)?;
                    self.pop()
                },
                | _ => unreachable!("direct expression forms checked above"),
            }
        } else {
            self.evaluate_raw(expression)?;
            self.drain_recursive(floor)?;
            self.pop()
        };
        if !converted {
            return Ok(item);
        }
        self.operands.push(item);
        let skip = usize::from(
            matches!(expression.kind, E::Binary { operator, .. } if compound_operator(operator).is_some()),
        );
        self.convert_recorded(expression, skip)?;
        Ok(self.pop())
    }

    fn recursive_statement(
        &mut self,
        statement: &'tu Statement<'tu>,
        mode: StatementMode,
    ) -> Result<(), LoweringError> {
        let floor = self.tasks.len();
        if let StatementType::Compound { items } = statement.kind {
            let control = self.unit.sema.control.statement(statement).ok_or_else(|| {
                LoweringError::missing(
                    "statement control facts are not retained",
                    statement.source_vectors,
                )
            })?;
            debug_assert!(
                std::ptr::eq(control.syntax, statement),
                "statement syntax identity"
            );
            for item in items {
                let task = match *item {
                    | BlockItem::Statement(child) if mode == StatementMode::Effects =>
                        Task::StatementEffects(child),
                    | BlockItem::Statement(child) => Task::Statement(child),
                    | BlockItem::Declaration(declaration) => Task::Declaration(declaration),
                    // A nested function is lowered as a function of its own.
                    | BlockItem::FunctionDefinition(definition) => {
                        self.nested_definition(definition)?;
                        continue;
                    },
                };
                self.recursive_task(task)?;
            }
            self.recursive_task(Task::CleanupNormal(statement))
        } else {
            self.statement(statement, mode)?;
            self.drain_recursive(floor)
        }
    }
}

/// Untimed optional address sampling; no counter is retained in the IR.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Stats {
    pub(crate) entries:   usize,
    pub(crate) max_depth: usize,
    pub(crate) low:       usize,
    pub(crate) high:      usize,
}

impl Stats {
    pub(crate) fn merge(&mut self, other: Self) {
        self.entries += other.entries;
        self.max_depth = self.max_depth.max(other.max_depth);
        if other.low != 0 {
            self.low = if self.low == 0 {
                other.low
            } else {
                self.low.min(other.low)
            };
        }
        self.high = self.high.max(other.high);
    }
}

pub(super) struct State {
    pub(super) enabled: bool,
    sample:             bool,
    depth:              usize,
    pub(super) stats:   Stats,
}

impl State {
    pub(super) fn new(enabled: bool, sample: bool) -> Self {
        Self {
            enabled,
            sample,
            depth: 0,
            stats: Stats::default(),
        }
    }

    #[inline(always)]
    fn enter(&mut self) {
        if !self.sample {
            return;
        }
        let marker = 0_u8;
        let address = (&raw const marker).addr();
        self.depth += 1;
        self.stats.entries += 1;
        self.stats.max_depth = self.stats.max_depth.max(self.depth);
        self.stats.low = if self.stats.low == 0 {
            address
        } else {
            self.stats.low.min(address)
        };
        self.stats.high = self.stats.high.max(address);
    }

    #[inline(always)]
    fn leave(&mut self) {
        if self.sample {
            self.depth -= 1;
        }
    }
}
