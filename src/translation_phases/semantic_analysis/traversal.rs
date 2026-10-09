//! Iterative traversal of declarations, nested scopes and expression type
//! names. C99: scopes §6.2.1, pp. 29-30; PDF pp. 41-42; statements §6.8,
//! pp. 131-139; PDF pp. 143-151. Expression constraints run after their
//! children.

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
            | ExpressionSlot::Parsed(e) => {
                self.work.push(Work::ValueExpression(e));
                self.work.push(Work::Expression(e));
            },
            | ExpressionSlot::Selection(header) => {
                if let Some(e) = header.expression {
                    self.work.push(Work::Slot(e));
                }
                self.work.push(Work::Declaration(header.declaration));
            },
            | ExpressionSlot::Missing(_) => {},
        }
    }

    /// C99: §6.8.1-§6.8.6.4, pp. 131-139; PDF pp. 143-151.
    pub(super) fn statement(&mut self, s: &'tu Statement<'tu>, new_scope: bool) {
        use StatementType as S;

        use super::statements::StatementWork as W;
        self.taint(s.recovered);
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
            | S::Label(name, child) => {
                self.label(name, true);
                self.work.push(Work::Statement(child, new_scope));
            },
            | S::Goto(name) => self.label(name, false),
            | S::LocalLabels(names) => self.local_labels(names),
            | S::Default(child) => {
                // The parser already diagnoses duplicate defaults, preserving
                // its established diagnostic and recovery contract.
                self.switch_label(s.source_vectors);
                self.work.push(Work::Statement(child, new_scope));
            },
            | S::Case(expression, child) => {
                self.work.push(Work::Statement(child, new_scope));
                self.case_label(expression, None, s.source_vectors);
            },
            | S::CaseRange(c) => {
                self.work.push(Work::Statement(c.statement, new_scope));
                self.case_label(c.lower, Some(c.upper), s.source_vectors);
            },
            | S::If {
                condition_expression,
                then_statement,
                else_statement,
            } => {
                self.enter(ScopeKind::Block);
                self.work.push(Work::PopScope);
                if let Some(child) = else_statement {
                    self.work.push(Work::StatementWork(W::Substatement(child)));
                }
                self.work
                    .push(Work::StatementWork(W::Substatement(then_statement)));
                self.work.push(Work::Condition(condition_expression, false));
                self.expression_slot(condition_expression);
            },
            | S::Switch {
                condition_expression,
                body_statement,
            } => {
                self.enter(ScopeKind::Block);
                self.work.push(Work::PopScope);
                self.work.push(Work::StatementWork(W::SwitchReady(
                    condition_expression,
                    body_statement,
                )));
                self.work.push(Work::Condition(condition_expression, true));
                self.expression_slot(condition_expression);
            },
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
                self.work.push(Work::StatementWork(W::LeaveLoop));
                self.statements.loops += 1;
                if matches!(s.kind, S::DoWhile { .. }) {
                    self.work.push(Work::Condition(condition_expression, false));
                    self.expression_slot(condition_expression);
                    self.work
                        .push(Work::StatementWork(W::Substatement(body_statement)));
                } else {
                    self.work
                        .push(Work::StatementWork(W::Substatement(body_statement)));
                    self.work.push(Work::Condition(condition_expression, false));
                    self.expression_slot(condition_expression);
                }
            },
            | S::For(f) => {
                self.enter(ScopeKind::Block);
                self.work.push(Work::PopScope);
                self.work.push(Work::StatementWork(W::LeaveLoop));
                self.statements.loops += 1;
                self.work
                    .push(Work::StatementWork(W::Substatement(f.body_statement)));
                if let Some(e) = f.iteration_expression {
                    self.expression_slot(e);
                }
                if let Some(e) = f.condition_expression {
                    self.work.push(Work::Condition(e, false));
                    self.expression_slot(e);
                }
                if let Some(initializer) = f.initializer {
                    match initializer {
                        | ForInitializer::Declaration(d) => {
                            self.for_declaration(d);
                            self.work.push(Work::Declaration(d));
                        },
                        | ForInitializer::Expression(e) => self.expression_slot(e),
                    }
                }
            },
            | S::Return(slot) => {
                self.work
                    .push(Work::StatementWork(W::ReturnDone(slot, s.source_vectors)));
                if let Some(ExpressionSlot::Parsed(e)) = slot {
                    // Assignment compatibility performs the value conversion.
                    self.work.push(Work::Expression(e));
                } else if let Some(slot) = slot {
                    self.expression_slot(slot);
                }
            },
            | S::Break if self.statements.loops == 0 && self.statements.switch.is_none() => self
                .error(
                    super::SemanticErrorKind::BreakOutsideLoopOrSwitch,
                    s.source_vectors,
                    None,
                    None,
                ),
            | S::Continue if self.statements.loops == 0 => self.error(
                super::SemanticErrorKind::ContinueOutsideLoop,
                s.source_vectors,
                None,
                None,
            ),
            | S::Expression(e) | S::ComputedGoto(e) => self.expression_slot(e),
            | _ => {},
        }
    }

    /// Resolve type names and type children before the enclosing expression.
    /// C99: §6.7.6, p. 122; PDF p. 134.
    pub(super) fn expression(&mut self, e: &'tu Expression<'tu>) {
        use ExpressionType as E;
        if self
            .expression_indices
            .contains_key(&std::ptr::from_ref(e).addr())
        {
            return;
        }
        if !e.recovered
            && let E::LabelAddress(name) = e.kind
        {
            self.label_address(name);
        }
        if matches!(e.kind, E::SizeofExpr(_) | E::SizeofType(_)) {
            self.enter_sizeof(e);
        }
        self.taint(
            e.recovered
                || matches!(
                    e.kind,
                    E::Builtin(_) | E::Generic(_) | E::Countof(_) | E::Nullptr
                ),
        );
        self.work.push(Work::ExpressionDone(e));
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
                _ = self.work.pop();
                self.work.push(Work::CompoundInitializer(e, initializer));
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
                self.implicit_function(function_expression);
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
