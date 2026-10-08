//! Iterative discovery of declarations, nested scopes and expression type
//! names. C99: scopes §6.2.1, pp. 29-30; PDF pp. 41-42; statements §6.8,
//! pp. 131-139; PDF pp. 143-151. No statement/expression typing is done here.

use super::{
    Analyzer,
    Expression,
    ExpressionSlot,
    ExpressionType,
    ForInitializer,
    ScopeKind,
    Statement,
    StatementType,
    Work,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    pub(super) fn expression_slot(&mut self, slot: ExpressionSlot<'tu>) {
        match slot {
            | ExpressionSlot::Parsed(e) => self.work.push(Work::Expression(e)),
            | ExpressionSlot::Selection(header) => {
                if let Some(e) = header.expression {
                    self.work.push(Work::Slot(e));
                }
                self.work.push(Work::Declaration(header.declaration));
            },
            | ExpressionSlot::Missing(_) => {},
        }
    }

    /// Structural walking gives selection/iteration statements their C99
    /// scopes. C99: §6.8.4p3, p. 133; PDF p. 145; §6.8.5p5, p. 135; PDF p.
    /// 147.
    pub(super) fn statement(&mut self, s: &'tu Statement<'tu>, new_scope: bool) {
        use StatementType as S;
        match s.kind {
            | S::Compound { items } => {
                if new_scope {
                    self.enter(ScopeKind::Block);
                    self.work.push(Work::PopScope);
                }
                for &item in items.iter().rev() {
                    self.work.push(Work::BlockItem(item));
                }
            },
            | S::Declaration(d) => self.work.push(Work::Declaration(d)),
            | S::Attributed(s) => self.work.push(Work::Statement(s.statement, new_scope)),
            | S::Label(_, s) | S::Default(s) => self.work.push(Work::Statement(s, new_scope)),
            | S::Case(expression, s) => {
                self.work.push(Work::Statement(s, new_scope));
                if let super::super::parsing::syntax::ConstantExpressionSlot::Parsed(e) = expression
                {
                    self.work.push(Work::Expression(e.expression()));
                }
            },
            | S::CaseRange(c) => {
                self.work.push(Work::Statement(c.statement, new_scope));
                for expression in [c.upper, c.lower] {
                    if let super::super::parsing::syntax::ConstantExpressionSlot::Parsed(e) =
                        expression
                    {
                        self.work.push(Work::Expression(e.expression()));
                    }
                }
            },
            | S::If {
                condition_expression,
                then_statement,
                else_statement,
            } => {
                self.enter(ScopeKind::Block);
                self.work.push(Work::PopScope);
                if let Some(s) = else_statement {
                    self.work.push(Work::Statement(s, true));
                }
                self.work.push(Work::Statement(then_statement, true));
                self.expression_slot(condition_expression);
            },
            | S::Switch {
                condition_expression,
                body_statement,
            }
            | S::While {
                condition_expression,
                body_statement,
            }
            | S::DoWhile {
                condition_expression,
                body_statement,
            } => {
                self.enter(ScopeKind::Block);
                self.work.push(Work::PopScope);
                self.work.push(Work::Statement(body_statement, true));
                self.expression_slot(condition_expression);
            },
            | S::For(f) => {
                self.enter(ScopeKind::Block);
                self.work.push(Work::PopScope);
                self.work.push(Work::Statement(f.body_statement, true));
                if let Some(e) = f.iteration_expression {
                    self.expression_slot(e);
                }
                if let Some(e) = f.condition_expression {
                    self.expression_slot(e);
                }
                if let Some(initializer) = f.initializer {
                    match initializer {
                        | ForInitializer::Declaration(d) => self.work.push(Work::Declaration(d)),
                        | ForInitializer::Expression(e) => self.expression_slot(e),
                    }
                }
            },
            | S::Expression(e) | S::ComputedGoto(e) | S::Return(Some(e)) => self.expression_slot(e),
            | _ => {},
        }
    }

    /// Type names in casts/sizeof/compound literals are resolved without typing
    /// operands. C99: §6.7.6, p. 122; PDF p. 134.
    pub(super) fn expression(&mut self, e: &'tu Expression<'tu>) {
        use ExpressionType as E;
        match e.kind {
            | E::Parenthesized { expression }
            | E::Unary {
                operand_expression: expression,
                ..
            }
            | E::SizeofExpr(expression)
            | E::AlignofExpr(expression) => self.work.push(Work::Expression(expression)),
            | E::Binary {
                left_expression,
                right_expression,
                ..
            } => {
                self.work.push(Work::Expression(right_expression));
                self.work.push(Work::Expression(left_expression));
            },
            | E::Conditional(c) | E::OmittedConditional(c) => {
                self.work.push(Work::Expression(c.else_expression));
                self.work.push(Work::Expression(c.then_expression));
                self.work.push(Work::Expression(c.condition_expression));
            },
            | E::Cast {
                target_type,
                operand_expression,
            } => {
                self.work.push(Work::Expression(operand_expression));
                self.work.push(Work::DiscardType);
                self.work.push(Work::TypeName(target_type));
            },
            | E::SizeofType(name) | E::AlignofType(name) => {
                self.work.push(Work::DiscardType);
                self.work.push(Work::TypeName(name));
            },
            | E::CompoundLiteral {
                type_name,
                initializer,
            } => {
                self.work.push(Work::Initializer(initializer));
                self.work.push(Work::DiscardType);
                self.work.push(Work::TypeName(type_name));
            },
            | E::Call {
                function_expression,
                arguments,
            } => {
                for &argument in arguments.iter().rev() {
                    self.work.push(Work::Expression(argument));
                }
                self.work.push(Work::Expression(function_expression));
            },
            | E::DirectMember {
                base_expression, ..
            }
            | E::IndirectMember {
                base_expression, ..
            } => self.work.push(Work::Expression(base_expression)),
            | E::StatementExpression(s) => self.work.push(Work::Statement(s, true)),
            | E::Generic(g) => {
                for association in g.associations.iter().rev() {
                    self.work.push(Work::Expression(association.expression));
                    if let Some(name) = association.type_name {
                        self.syntax_operand(super::SyntaxOperand::Type(name));
                    }
                }
                self.syntax_operand(g.controlling);
            },
            | E::Countof(operand) => self.syntax_operand(operand),
            | E::Builtin(b) =>
                for &operand in b.operands.iter().rev() {
                    self.syntax_operand(operand);
                },
            | _ => {},
        }
    }
}
