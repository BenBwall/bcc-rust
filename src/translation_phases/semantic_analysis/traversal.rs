//! Iterative traversal of declarations, nested scopes and expression type
//! names. C99: scopes §6.2.1, pp. 29-30; PDF pp. 41-42; statements §6.8,
//! pp. 131-139; PDF pp. 143-151. Expression constraints run after their
//! children.

use super::{
    Analyzer,
    ScopeKind,
    Work,
    functions::FunctionWork,
};
use crate::translation_phases::parsing::syntax::{
    BinaryOperator,
    Expression,
    ExpressionSlot,
    ExpressionType,
    ForInitializer,
    Statement,
    StatementType,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
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
        self.control.expression(e);
        self.discover_implicit_callee(e);
        if !e.recovered
            && let E::LabelAddress(name) = e.kind
        {
            self.label_address(e, name);
        }
        // Reserved builtin designators need their binding even before a
        // compile-time selection becomes a callee, or an address is checked.
        // GNU extension: GCC "Other Builtins"; C99 §7.1.3p1, p. 166;
        // PDF p. 178 (implementation-reserved identifiers).
        if !e.recovered
            && let E::Identifier(name) = e.kind
            && self.builtin_name(name)
        {
            // Only an inlined call can forward the caller's variadic
            // arguments; a body that does keeps calling the external
            // definition instead (`FunctionRecord::forwards_variadic`).
            // GNU extension: GCC manual, "Constructing Calls".
            if matches!(
                self.context.string_cache.at(name.name),
                "__builtin_va_arg_pack" | "__builtin_va_arg_pack_len"
            ) && let Some(function) = self.functions.current
            {
                self.function_records[function.id].forwards_variadic = true;
                if let Some(binding) = function.binding
                    && self.bindings[binding].linkage == super::Linkage::External
                {
                    _ = self
                        .unchecked_wrappers
                        .insert(self.bindings[binding].name.name, ());
                }
            }
            self.implicit_function(e);
        }
        if matches!(e.kind, E::SizeofExpr(_) | E::SizeofType(_)) {
            self.enter_sizeof(e);
        }
        if matches!(e.kind, E::AlignofExpr(_) | E::AlignofType(_)) {
            self.enter_reference_context(None);
        }
        self.taint(
            e.recovered
                || matches!(e.kind, E::Builtin(b) if !super::builtins::modeled(b.keyword))
                || matches!(e.kind, E::Countof(_)),
        );
        let branch = self.begin_branch_operand(e);
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
                operator,
            } => {
                if operator == BinaryOperator::Comma {
                    self.discard_expression_result(left_expression);
                }
                let live = match operator {
                    | BinaryOperator::LogicalAnd => Some(true),
                    | BinaryOperator::LogicalOr => Some(false),
                    | _ => None,
                };
                if live.is_some() {
                    if !branch {
                        self.work.push(Work::FunctionWork(FunctionWork::ScalarLogicalEnd(e)));
                    }
                    // The scalar logical visitor emits its nonconstant left
                    // operand through EmitBranchOnBoolExpr too. Its nested
                    // logical operands therefore share the branch policy.
                    self.branch_condition(left_expression);
                    self.work
                        .push(Work::FunctionWork(FunctionWork::RestoreElided(
                            self.functions.elided,
                        )));
                    self.work
                        .push(Work::FunctionWork(FunctionWork::RestoreUnvisited(
                            self.functions.unvisited,
                        )));
                    self.work.push(Work::FunctionWork(FunctionWork::RestoreDead(
                        self.functions.dead,
                    )));
                }
                self.work.push(Work::Expression(right_expression));
                if let Some(live) = live.filter(|_| !branch) {
                    self.dead_arm(left_expression, live, Some(right_expression));
                    self.begin_condition_references(left_expression);
                }
                self.work.push(Work::Expression(left_expression));
            },
            | E::Conditional(c) | E::OmittedConditional(c) => {
                // The result type is known only after both arms. Clang's
                // complex and aggregate emitters preserve both arms even
                // for a folded condition; scalar emission skips one.
                let outer = self.skipped_state();
                self.work
                    .push(Work::FunctionWork(FunctionWork::ConditionalMark(
                        e, 2, outer, self.functions.elided,
                    )));
                for (arm, live, mark) in [(c.else_expression, false, 1), (c.then_expression, true, 0)] {
                    if branch {
                        self.work.push(Work::Expression(arm));
                    } else {
                        self.work.push(Work::FunctionWork(FunctionWork::ConditionalArm {
                            owner: e,
                            arm,
                            live,
                            outer,
                            outer_elided: self.functions.elided,
                        }));
                    }
                    self.work.push(Work::FunctionWork(FunctionWork::ConditionalMark(
                        e, mark, outer, self.functions.elided,
                    )));
                }
                // GNU `x ?: y` binds its common operand before folding the
                // condition (Clang `OpaqueValueMapping`), so it is always
                // emitted. GNU extension: GCC manual, "Conditionals".
                if matches!(e.kind, E::Conditional(_)) {
                    self.begin_condition_references(c.condition_expression);
                }
                self.work.push(Work::Expression(c.condition_expression));
            },
            | E::Cast {
                target_type,
                operand_expression,
            } => {
                self.work.push(Work::FunctionWork(FunctionWork::CastOperand(operand_expression)));
                self.work.push(Work::TypeName(target_type, false));
            },
            | E::SizeofType(name) | E::AlignofType(name) => {
                self.work.push(Work::DiscardType);
                self.work.push(Work::TypeName(name, false));
            },
            | E::CompoundLiteral {
                type_name,
                initializer,
            } => {
                _ = self.work.pop();
                self.work.push(Work::CompoundInitializer(e, initializer));
                self.work.push(Work::DiscardType);
                // Clang recovers folded bounds here, where C99 §6.5.2.5p1
                // requires a non-VLA type (p. 75; PDF p. 87).
                self.work.push(Work::TypeName(type_name, true));
            },
            | E::Call {
                function_expression,
                arguments,
            } => {
                self.work.push(Work::FunctionWork(FunctionWork::CallArguments(
                    function_expression,
                    arguments,
                )));
                self.mark_implicit_callee(function_expression);
                self.work.push(Work::Expression(function_expression));
            },
            | E::DirectMember {
                base_expression, ..
            }
            | E::IndirectMember {
                base_expression, ..
            } => self.work.push(Work::Expression(base_expression)),
            | E::StatementExpression(s) => {
                // Code generation resumes after the statement expression
                // even when its body jumps away.
                self.begin_statement_expression(s);
                self.enter_statement_expression(e.source_vectors);
                self.work.push(Work::Statement(s, true));
            },
            | E::Generic(g) => {
                // Code generation continues after the selection in the
                // block its selected association ends in, which the
                // selection restores (`skip_unselected_generic`).
                self.work
                    .push(Work::FunctionWork(FunctionWork::RestoreOrphan(
                        self.functions.emission.orphan,
                    )));
                // Marks around each operand let the selection skip the
                // references code generation does not emit.
                for association in g.associations.iter().rev() {
                    self.work
                        .push(Work::FunctionWork(FunctionWork::GenericMark(g)));
                    self.work.push(Work::Expression(association.expression));
                    self.work
                        .push(Work::FunctionWork(FunctionWork::GenericMark(g)));
                    if let Some(name) = association.type_name {
                        self.syntax_operand(super::SyntaxOperand::Type(name));
                    }
                }
                self.work
                    .push(Work::FunctionWork(FunctionWork::GenericMark(g)));
                self.syntax_operand(g.controlling);
                self.work
                    .push(Work::FunctionWork(FunctionWork::GenericMark(g)));
            },
            | E::Countof(operand) => self.syntax_operand(operand),
            | E::Builtin(b)
                if b.keyword == crate::translation_phases::preprocessing::KeywordTokenType::BuiltinChooseExpr =>
            {
                // The condition is analyzed first, so the operand Clang does
                // not select is skipped as it is analyzed (`DeadArm`): only
                // the selected one's calls and joins move the insertion
                // state, and the restore below in the general arm is not
                // needed.
                if let [super::SyntaxOperand::Expression(condition), super::SyntaxOperand::Expression(yes), super::SyntaxOperand::Expression(no)] = &*b.operands {
                    self.work.push(Work::FunctionWork(FunctionWork::RestoreElided(self.functions.elided)));
                    self.work.push(Work::FunctionWork(FunctionWork::RestoreDead(self.functions.dead)));
                    self.work.push(Work::Expression(no));
                    self.dead_arm(condition, false, None);
                    self.work.push(Work::Expression(yes));
                    self.dead_arm(condition, true, None);
                    self.begin_condition_references(condition);
                    self.work.push(Work::Expression(condition));
                }
            },
            | E::Builtin(b) => {
                // Code generation emits only the selected operand of
                // `__builtin_choose_expr`, which is known once both are
                // analyzed; a call in either that does not return leaves
                // the code after it emitted, as without the model, and a
                // join in either is kept.
                if b.keyword
                    == crate::translation_phases::preprocessing::KeywordTokenType::BuiltinChooseExpr
                {
                    self.work
                        .push(Work::FunctionWork(FunctionWork::RestoreUnjoinedOrphan(
                            self.functions.emission.orphan,
                        )));
                }
                for &member in b.members.iter().rev() {
                    if let super::super::parsing::OffsetMember::Index(index) = member {
                        self.work.push(Work::Expression(index));
                    }
                }
                for &operand in b.operands.iter().rev() {
                    self.syntax_operand(operand);
                }
            },
            | _ => {},
        }
    }

    /// C99: §6.8.1-§6.8.6.4, pp. 131-139; PDF pp. 143-151.
    pub(super) fn statement(&mut self, s: &'tu Statement<'tu>, new_scope: bool) {
        use StatementType as S;

        use super::statements::StatementWork as W;
        self.taint(s.recovered);
        let value_statement = self
            .functions
            .emission
            .value_statements
            .contains_key(&std::ptr::from_ref(s).addr());
        let simple = matches!(
            s.kind,
            S::Null
                | S::Compound { .. }
                | S::Declaration(_)
                | S::Attributed(_)
                | S::Label(..)
                | S::Case(..)
                | S::CaseRange(_)
                | S::Default(_)
                | S::Goto(_)
                | S::Break
                | S::Continue
                | S::LocalLabels(_)
                | S::SehLeave
        );
        let omitted = if self.functions.dead && !simple && !value_statement {
            if self.statement_contains_label(s, false) {
                // Clang EmitStmt ensures an insertion point for a label.
                self.functions.dead = false;
                false
            } else {
                true
            }
        } else {
            false
        };
        if (omitted || self.folded_statement_elided(s)) && !self.functions.elided {
            // Clang EmitSimpleStmt handles declarations and compounds even
            // without an insertion point; EmitStmt skips other label-free
            // statements, including all declarations nested inside them.
            self.work
                .push(Work::FunctionWork(FunctionWork::RestoreElided(false)));
            self.work.push(Work::FunctionWork(FunctionWork::RestoreDead(
                self.functions.dead,
            )));
            self.work
                .push(Work::FunctionWork(FunctionWork::RestoreOrphan(
                    self.functions.emission.orphan,
                )));
            self.functions.elided = true;
        }
        // Code generation skips such a statement whole, unlike a block or a
        // declaration, whose static objects it still emits. An expression
        // statement decides this once a statement expression's value
        // statement has been resumed ([`Self::begin_expression_statement`]).
        if self.functions.dead
            && !self.functions.unvisited
            && matches!(
                s.kind,
                S::If { .. }
                    | S::Switch { .. }
                    | S::While { .. }
                    | S::DoWhile { .. }
                    | S::For(_)
                    | S::Return(_)
                    | S::ComputedGoto(_)
                    | S::Asm(_)
                    | S::MsAsm(_)
                    | S::Seh(_)
            )
        {
            self.work
                .push(Work::FunctionWork(FunctionWork::RestoreUnvisited(false)));
            self.functions.unvisited = true;
        }
        let outside = self.control.current;
        if let Some(id) = self.control.statement(s) {
            self.control.statements[id.0].elided = self.functions.elided;
        }
        self.work.push(Work::StatementWork(W::RetainTail(s)));
        match s.kind {
            | S::Compound { items } => {
                if !new_scope {
                    self.work.push(Work::StatementWork(W::FinishStatement(s)));
                }
                if new_scope {
                    self.enter(ScopeKind::Block);
                    self.work.push(Work::PopScope);
                    if let Some(outside) = outside {
                        self.work
                            .push(Work::StatementWork(W::NormalExit(s, outside)));
                    }
                }
                for &item in items.iter().rev() {
                    self.work.push(Work::BlockItem(item));
                }
            },
            | S::Declaration(d) => self.work.push(Work::Declaration(d)),
            | S::Attributed(s) => {
                self.check_attribute_list(s.attributes);
                self.work.push(Work::Statement(s.statement, new_scope));
            },
            | S::Label(name, child) => {
                _ = self.label(s, name, true);
                // A label is a jump target: code generation emits from it,
                // in a block that the code before it can branch to.
                self.functions.dead = false;
                self.functions.emission.orphan = None;
                self.work.push(Work::Statement(child, new_scope));
            },
            | S::Goto(name) => {
                _ = self.label(s, name, false);
                self.functions.dead = true;
            },
            | S::LocalLabels(names) => self.local_labels(names),
            | S::Default(child) => {
                self.functions.dead = self.switch_label_dead(s);
                // The parser already diagnoses duplicate defaults, preserving
                // its established diagnostic and recovery contract.
                self.switch_label(s, s.source_vectors);
                self.work.push(Work::Statement(child, new_scope));
            },
            | S::Case(expression, child) => {
                self.functions.dead = self.switch_label_dead(s);
                self.work.push(Work::Statement(child, new_scope));
                self.case_label(s, expression, None, s.source_vectors);
            },
            | S::CaseRange(c) => {
                self.functions.dead = self.switch_label_dead(s);
                self.work.push(Work::Statement(c.statement, new_scope));
                self.case_label(s, c.lower, Some(c.upper), s.source_vectors);
            },
            | S::If {
                condition_expression,
                then_statement,
                else_statement,
            } => {
                self.enter(ScopeKind::Block);
                self.work.push(Work::PopScope);
                if let Some(outside) = outside {
                    self.work
                        .push(Work::StatementWork(W::NormalExit(s, outside)));
                }
                // Code after the statement is skipped when both arms end
                // skipped, a missing `else` being an empty arm.
                self.work
                    .push(Work::FunctionWork(FunctionWork::RestoreUnvisited(
                        self.functions.unvisited,
                    )));
                self.work.push(Work::FunctionWork(FunctionWork::JoinArms(
                    self.functions.elided,
                )));
                let condition = super::statements::slot_expression(condition_expression);
                if let Some(child) = else_statement {
                    self.work.push(Work::StatementWork(W::Substatement(child)));
                }
                if let Some(condition) = condition {
                    self.dead_statement_arm(s, condition, false, else_statement);
                }
                self.work.push(Work::FunctionWork(FunctionWork::ThenEnd));
                self.work
                    .push(Work::StatementWork(W::Substatement(then_statement)));
                if let Some(condition) = condition {
                    self.dead_statement_arm(s, condition, true, Some(then_statement));
                }
                self.work.push(Work::Condition(condition_expression, false));
                if let Some(condition) = condition {
                    self.begin_condition_references(condition);
                    self.branch_condition(condition);
                }
                self.expression_slot(condition_expression);
            },
            | S::Switch {
                condition_expression,
                body_statement,
            } => {
                self.enter(ScopeKind::Block);
                self.work.push(Work::PopScope);
                if let Some(outside) = outside {
                    self.work
                        .push(Work::StatementWork(W::NormalExit(s, outside)));
                }
                // The body is skipped up to its first case label, which
                // resumes the code around the switch, as does its end
                // unless a folded switch's selected statements end skipped.
                self.work
                    .push(Work::FunctionWork(FunctionWork::RestoreSwitchDead(
                        self.functions.switch_dead,
                    )));
                self.work
                    .push(Work::FunctionWork(FunctionWork::RestoreUnvisited(
                        self.functions.unvisited,
                    )));
                self.work.push(Work::FunctionWork(FunctionWork::SwitchEnd(
                    self.functions.dead,
                )));
                self.functions.switch_dead = self.functions.dead;
                self.work.push(Work::StatementWork(W::SwitchReady(
                    s,
                    condition_expression,
                    body_statement,
                )));
                self.work
                    .push(Work::FunctionWork(FunctionWork::RestoreDead(true)));
                self.work.push(Work::Condition(condition_expression, true));
                if let Some(condition) = super::statements::slot_expression(condition_expression) {
                    self.begin_condition_references(condition);
                }
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
                if let Some(outside) = outside {
                    self.work
                        .push(Work::StatementWork(W::NormalExit(s, outside)));
                }
                self.work.push(Work::FunctionWork(FunctionWork::LoopEnd(
                    s,
                    self.functions.dead,
                )));
                if matches!(s.kind, S::DoWhile { .. }) {
                    self.work.push(Work::Condition(condition_expression, false));
                    self.expression_slot(condition_expression);
                    self.work
                        .push(Work::StatementWork(W::LoopBody(s, body_statement)));
                } else {
                    self.work
                        .push(Work::StatementWork(W::LoopBody(s, body_statement)));
                    self.work.push(Work::Condition(condition_expression, false));
                    self.expression_slot(condition_expression);
                }
            },
            | S::For(f) => {
                self.enter(ScopeKind::Block);
                self.work.push(Work::PopScope);
                if let Some(outside) = outside {
                    self.work
                        .push(Work::StatementWork(W::NormalExit(s, outside)));
                }
                self.work.push(Work::FunctionWork(FunctionWork::LoopEnd(
                    s,
                    self.functions.dead,
                )));
                self.work
                    .push(Work::StatementWork(W::LoopBody(s, f.body_statement)));
                if let Some(e) = f.iteration_expression {
                    if let ExpressionSlot::Parsed(expression) = e {
                        self.discard_expression_result(expression);
                    }
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
                        | ForInitializer::Expression(e) => {
                            if let ExpressionSlot::Parsed(expression) = e {
                                self.discard_expression_result(expression);
                            }
                            self.expression_slot(e);
                        },
                    }
                }
            },
            | S::Return(slot) => {
                if let Some(function) = self.functions.current {
                    let control = self.function_records[function.id].control;
                    _ = self.control.jump(
                        s,
                        super::ControlTarget::FunctionExit(control.id),
                        super::ControlPoint {
                            function: control.id,
                            region:   control.root,
                        },
                    );
                }
                self.work
                    .push(Work::FunctionWork(FunctionWork::RestoreDead(true)));
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
            | S::Break => {
                self.generated_break();
                self.switch_break(s);
                self.control_jump(s, false);
                self.functions.dead = true;
            },
            | S::Continue => {
                self.control_jump(s, true);
                self.functions.dead = true;
            },
            | S::Expression(e) => {
                if let ExpressionSlot::Parsed(expression) = e {
                    if !value_statement {
                        self.discard_expression_result(expression);
                    }
                    self.begin_expression_statement(s, expression);
                }
                self.expression_slot(e);
            },
            | S::ComputedGoto(e) => {
                self.work
                    .push(Work::FunctionWork(FunctionWork::RestoreDead(true)));
                self.computed_goto(s, e);
            },
            | S::Asm(asm) => {
                if let Some(function) = self.functions.current {
                    self.function_records[function.id].assembly = true;
                }
                self.assembly_statement(s, asm);
            },
            | S::MsAsm(_) =>
                if let Some(function) = self.functions.current {
                    self.function_records[function.id].assembly = true;
                },
            | _ => {},
        }
    }

    /// A label in the skipped arm prevents Clang from eliding that whole
    /// arm, including statements before the label. An enclosing switch's
    /// case labels count, but a nested switch owns and hides its cases.
    /// C99: §6.8.4.1p2, pp. 133-134; PDF pp. 145-146.
    fn dead_statement_arm(
        &mut self,
        owner: &'tu Statement<'tu>,
        condition: &'tu Expression<'tu>,
        live: bool,
        statement: Option<&'tu Statement<'tu>>,
    ) {
        self.work
            .push(Work::FunctionWork(FunctionWork::DeadStatementArm {
                owner,
                condition,
                live,
                outer: self.skipped_state(),
                outer_elided: self.functions.elided,
                statement,
            }));
    }

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
}
