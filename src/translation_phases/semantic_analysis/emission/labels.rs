//! What each statement holds that Clang's code generation asks about before
//! skipping or folding it: an ordinary label, a case label of an enclosing
//! switch, or a `break` that leaves it (`CodeGenFunction::ContainsLabel` and
//! `containsBreak`). Both walk `Stmt::children()`, so a statement expression
//! in an expression or an initializer counts as well. One walk per function
//! records the facts of every statement and each switch's own case labels in
//! source order, the first time the function needs them.
//! C99: §6.8.1 paragraphs 2-4, pp. 131-132; PDF pp. 143-144 (labels);
//! §6.8.4.2 paragraphs 2-4, p. 134; PDF p. 146 (case labels belong to the
//! closest enclosing switch); §6.8.6.3 paragraph 2, p. 138; PDF p. 150.
//! GNU extension: GCC manual, "Statement Exprs".

use super::super::{
    Analyzer,
    ExpressionType,
    SyntaxOperand,
};
use crate::{
    translation_phases::parsing::{
        OffsetMember,
        declaration_syntax::{
            Declaration,
            DesignatorType,
            Initializer,
            InitializerType,
        },
        syntax::{
            BlockItem,
            Expression,
            ExpressionSlot,
            ForInitializer,
            Statement,
            StatementType,
        },
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

impl<'tu, 's> Analyzer<'_, 'tu, 's> {
    /// What `statement` of the current function holds, walking the
    /// function first if no earlier question did.
    pub(in crate::translation_phases::semantic_analysis) fn holds(
        &mut self,
        statement: &'tu Statement<'tu>,
    ) -> Holds {
        self.walk_current_function();
        self.held(statement)
    }

    /// What `statement` holds, once its function is walked.
    pub(super) fn held(&self, statement: &'tu Statement<'tu>) -> Holds {
        self.functions
            .emission
            .holds
            .get(&std::ptr::from_ref(statement).addr())
            .copied()
            .unwrap_or_default()
    }

    /// Clang's `ContainsLabel`: whether `statement` holds an ordinary label,
    /// or, unless `ignore_cases`, a case label of an enclosing switch.
    pub(in crate::translation_phases::semantic_analysis) fn statement_contains_label(
        &mut self,
        statement: &'tu Statement<'tu>,
        ignore_cases: bool,
    ) -> bool {
        let holds = self.holds(statement);
        holds.label || (!ignore_cases && holds.case)
    }

    /// Clang's dead automatic initializer check, including labels in nested
    /// statement expressions. C99: §6.7.8, pp. 125-130; PDF pp. 137-142.
    pub(in crate::translation_phases::semantic_analysis) fn initializer_contains_label(
        &mut self,
        initializer: &'tu Initializer<'tu>,
    ) -> bool {
        let mut expressions = ArenaVec::new_in(self.scratch);
        initializer_expressions(self.scratch, initializer, &mut |e| expressions.push(e));
        expressions
            .into_iter()
            .any(|e| self.expression_contains_label(e))
    }

    /// What a declaration's initializers hold, in their statement
    /// expressions, once its function is walked.
    pub(super) fn declaration_holds(&self, declaration: &'tu Declaration<'tu>) -> Holds {
        let mut expressions = ArenaVec::new_in(self.scratch);
        declaration_children(self.scratch, declaration, &mut |step| {
            if let Step::Expression(expression) = step {
                expressions.push(expression);
            }
        });
        let mut holds = Holds::default();
        while let Some(expression) = expressions.pop() {
            if let ExpressionType::StatementExpression(statement) = expression.kind {
                let held = self.held(statement);
                holds.label |= held.label;
                holds.case |= held.case;
                holds.escaping_break |= held.escaping_break;
            } else {
                expression_children(self.scratch, expression, |child| expressions.push(child));
            }
        }
        holds
    }

    /// Whether `expression` holds an ordinary label, in a statement
    /// expression, as Clang's `ContainsLabel` asks of a condition before
    /// folding it; memoized, since conditions nest.
    pub(in crate::translation_phases::semantic_analysis) fn expression_contains_label(
        &mut self,
        expression: &'tu Expression<'tu>,
    ) -> bool {
        let mut work = ArenaVec::new_in(self.scratch);
        work.push((expression, false));
        while let Some((current, children_done)) = work.pop() {
            let key = std::ptr::from_ref(current).addr();
            if self
                .functions
                .emission
                .labeled_expressions
                .contains_key(&key)
            {
                continue;
            }
            if let ExpressionType::StatementExpression(statement) = current.kind {
                let holds = self.holds(statement);
                let labeled = holds.label || holds.case;
                _ = self
                    .functions
                    .emission
                    .labeled_expressions
                    .insert(key, labeled);
            } else if children_done {
                let mut labeled = false;
                expression_children(self.scratch, current, |child| {
                    labeled |= self
                        .functions
                        .emission
                        .labeled_expressions
                        .get(&std::ptr::from_ref(child).addr())
                        .copied()
                        .unwrap_or(false);
                });
                _ = self
                    .functions
                    .emission
                    .labeled_expressions
                    .insert(key, labeled);
            } else {
                work.push((current, true));
                expression_children(self.scratch, current, |child| work.push((child, false)));
            }
        }
        self.functions
            .emission
            .labeled_expressions
            .get(&std::ptr::from_ref(expression).addr())
            .copied()
            .unwrap_or(false)
    }

    /// The case and default labels of `switch` in source order, and the
    /// position of the last case range among them, once its function is
    /// walked. `None` for a switch this model does not fold.
    pub(super) fn switch_labels(
        &mut self,
        switch: &'tu Statement<'tu>,
    ) -> Option<SwitchLabels<'tu, 's>> {
        self.walk_current_function();
        self.functions
            .emission
            .switches
            .get(&std::ptr::from_ref(switch).addr())
            .copied()
    }

    /// The position of case or default `label` among its switch's labels.
    pub(super) fn label_position(&self, label: &'tu Statement<'tu>) -> Option<usize> {
        self.functions
            .emission
            .label_positions
            .get(&std::ptr::from_ref(label).addr())
            .copied()
    }

    /// Records [`Holds`] for every statement of the current function and
    /// the labels of its switches, in one iterative walk over statements,
    /// the expressions in them and their initializers. A nested function
    /// is walked as a function of its own.
    fn walk_current_function(&mut self) {
        let Some(function) = self.functions.current else {
            return;
        };
        if self
            .functions
            .emission
            .walked
            .insert(std::ptr::from_ref(function.syntax).addr(), ())
            .is_some()
        {
            return;
        }
        let scratch = self.scratch;
        let mut work: ArenaVec<'_, Step<'tu>> = ArenaVec::new_in(scratch);
        // What each statement being walked has collected from its children.
        let mut open: ArenaVec<'_, Holds> = ArenaVec::new_in(scratch);
        // The switches being walked, with their labels so far.
        let mut switches: ArenaVec<'_, OpenSwitch<'tu, '_>> = ArenaVec::new_in(scratch);
        work.push(Step::Enter(function.syntax.body));
        while let Some(step) = work.pop() {
            match step {
                | Step::Enter(statement) => {
                    open.push(Holds::default());
                    work.push(Step::Exit(statement));
                    let start = work.len();
                    statement_children(scratch, statement, |child| work.push(child));
                    work[start..].reverse();
                    match statement.kind {
                        | StatementType::Case(..)
                        | StatementType::Default(_)
                        | StatementType::CaseRange(_) =>
                            if let Some(switch) = switches.last_mut() {
                                if matches!(statement.kind, StatementType::CaseRange(_)) {
                                    switch.last_range = Some(switch.labels.len());
                                }
                                switch.labels.push(statement);
                            },
                        | StatementType::Seh(_)
                        | StatementType::SehLeave
                        | StatementType::NamedBreak(_)
                        | StatementType::NamedContinue(_) =>
                            if let Some(switch) = switches.last_mut() {
                                switch.supported = false;
                            },
                        | _ => {},
                    }
                },
                | Step::EnterSwitch(statement) => switches.push(OpenSwitch {
                    statement,
                    labels: ArenaVec::new_in(scratch),
                    last_range: None,
                    supported: true,
                }),
                | Step::Exit(statement) => {
                    let mut holds = open.pop().expect("a statement is open");
                    match statement.kind {
                        | StatementType::Label(..) => holds.label = true,
                        | StatementType::Case(..)
                        | StatementType::Default(_)
                        | StatementType::CaseRange(_) => holds.case = true,
                        | StatementType::Break => holds.escaping_break = true,
                        | _ => {},
                    }
                    match statement.kind {
                        // A switch owns its case labels and its breaks;
                        // a loop owns its breaks.
                        | StatementType::Switch { .. } => {
                            holds.case = false;
                            holds.escaping_break = false;
                            if let Some(switch) = switches.pop()
                                && switch.supported
                            {
                                let labels = scratch.alloc_slice_copy(&switch.labels);
                                for (position, &label) in labels.iter().enumerate() {
                                    _ = self
                                        .functions
                                        .emission
                                        .label_positions
                                        .insert(std::ptr::from_ref(label).addr(), position);
                                }
                                _ = self.functions.emission.switches.insert(
                                    std::ptr::from_ref(switch.statement).addr(),
                                    SwitchLabels {
                                        labels,
                                        last_range: switch.last_range,
                                    },
                                );
                            }
                        },
                        | StatementType::While { .. }
                        | StatementType::DoWhile { .. }
                        | StatementType::For(_) => holds.escaping_break = false,
                        | _ => {},
                    }
                    _ = self
                        .functions
                        .emission
                        .holds
                        .insert(std::ptr::from_ref(statement).addr(), holds);
                    if let Some(parent) = open.last_mut() {
                        parent.label |= holds.label;
                        parent.case |= holds.case;
                        parent.escaping_break |= holds.escaping_break;
                    }
                },
                | Step::Expression(expression) =>
                    if let ExpressionType::StatementExpression(statement) = expression.kind {
                        work.push(Step::Enter(statement));
                    } else {
                        let start = work.len();
                        expression_children(scratch, expression, |child| {
                            work.push(Step::Expression(child));
                        });
                        work[start..].reverse();
                    },
            }
        }
    }
}

/// What a statement holds, as Clang's code generation asks before skipping
/// or folding it.
#[derive(Clone, Copy, Default)]
pub(in crate::translation_phases::semantic_analysis) struct Holds {
    /// An ordinary label.
    pub(in crate::translation_phases::semantic_analysis) label: bool,
    /// A case or default label of an enclosing switch.
    pub(in crate::translation_phases::semantic_analysis) case: bool,
    /// A `break` that leaves the statement.
    pub(super) escaping_break: bool,
}

/// One switch's case and default labels.
#[derive(Clone, Copy)]
pub(in crate::translation_phases::semantic_analysis) struct SwitchLabels<'tu, 's> {
    /// The labels in source order.
    pub(super) labels:     &'s [&'tu Statement<'tu>],
    /// The position of the last case range among them.
    pub(super) last_range: Option<usize>,
}

/// A switch whose body the function walk is in.
struct OpenSwitch<'tu, 's> {
    statement:  &'tu Statement<'tu>,
    labels:     ArenaVec<'s, &'tu Statement<'tu>>,
    last_range: Option<usize>,
    /// The body holds no statement this model leaves out of folding.
    supported:  bool,
}

/// One step of the function walk.
#[derive(Clone, Copy)]
enum Step<'tu> {
    Enter(&'tu Statement<'tu>),
    /// The body of this switch begins: case labels belong to it.
    EnterSwitch(&'tu Statement<'tu>),
    Exit(&'tu Statement<'tu>),
    Expression(&'tu Expression<'tu>),
}

/// Clang's `Stmt::children()` of `statement` in source order, as walk steps:
/// substatements, expressions and the initializers of declarations. A
/// nested function definition is not a child; a switch marks where its body
/// begins.
fn statement_children<'tu>(
    scratch: &Bump,
    statement: &'tu Statement<'tu>,
    mut push: impl FnMut(Step<'tu>),
) {
    let slot = |slot: ExpressionSlot<'tu>, push: &mut dyn FnMut(Step<'tu>)| match slot {
        | ExpressionSlot::Parsed(expression) => push(Step::Expression(expression)),
        | ExpressionSlot::Selection(header) => {
            declaration_children(scratch, header.declaration, push);
            if let Some(ExpressionSlot::Parsed(expression)) = header.expression {
                push(Step::Expression(expression));
            }
        },
        | ExpressionSlot::Missing(_) => {},
    };
    match statement.kind {
        | StatementType::Compound { items } =>
            for item in items {
                match *item {
                    | BlockItem::Statement(child) => push(Step::Enter(child)),
                    | BlockItem::Declaration(declaration) =>
                        declaration_children(scratch, declaration, &mut push),
                    | BlockItem::FunctionDefinition(_) => {},
                }
            },
        | StatementType::Declaration(declaration) =>
            declaration_children(scratch, declaration, &mut push),
        | StatementType::Attributed(attributed) => push(Step::Enter(attributed.statement)),
        | StatementType::Label(_, child)
        | StatementType::Default(child)
        | StatementType::Case(_, child) => push(Step::Enter(child)),
        | StatementType::CaseRange(range) => push(Step::Enter(range.statement)),
        | StatementType::Expression(expression)
        | StatementType::ComputedGoto(expression)
        | StatementType::Return(Some(expression)) => slot(expression, &mut push),
        | StatementType::If {
            condition_expression,
            then_statement,
            else_statement,
        } => {
            slot(condition_expression, &mut push);
            push(Step::Enter(then_statement));
            if let Some(child) = else_statement {
                push(Step::Enter(child));
            }
        },
        | StatementType::Switch {
            condition_expression,
            body_statement,
        } => {
            slot(condition_expression, &mut push);
            push(Step::EnterSwitch(statement));
            push(Step::Enter(body_statement));
        },
        | StatementType::While {
            condition_expression,
            body_statement,
        } => {
            slot(condition_expression, &mut push);
            push(Step::Enter(body_statement));
        },
        | StatementType::DoWhile {
            condition_expression,
            body_statement,
        } => {
            push(Step::Enter(body_statement));
            slot(condition_expression, &mut push);
        },
        | StatementType::For(for_loop) => {
            match for_loop.initializer {
                | Some(ForInitializer::Declaration(declaration)) =>
                    declaration_children(scratch, declaration, &mut push),
                | Some(ForInitializer::Expression(expression)) => slot(expression, &mut push),
                | None => {},
            }
            if let Some(condition) = for_loop.condition_expression {
                slot(condition, &mut push);
            }
            if let Some(iteration) = for_loop.iteration_expression {
                slot(iteration, &mut push);
            }
            push(Step::Enter(for_loop.body_statement));
        },
        | _ => {},
    }
}

/// The expressions in the initializers of `declaration`'s declarators, as
/// walk steps.
fn declaration_children<'tu>(
    scratch: &Bump,
    declaration: &'tu Declaration<'tu>,
    push: &mut dyn FnMut(Step<'tu>),
) {
    for init in declaration.init_declarators {
        if let Some(initializer) = init.initializer {
            initializer_expressions(scratch, initializer, &mut |expression| {
                push(Step::Expression(expression));
            });
        }
    }
}

/// The expressions of `initializer` in source order, nested lists and the
/// expressions of array designators included.
fn initializer_expressions<'tu>(
    scratch: &Bump,
    initializer: &'tu Initializer<'tu>,
    push: &mut dyn FnMut(&'tu Expression<'tu>),
) {
    let mut work = ArenaVec::new_in(scratch);
    work.push(initializer);
    while let Some(initializer) = work.pop() {
        match initializer.kind {
            | InitializerType::AssignmentExpression(expression) => push(expression),
            | InitializerType::InitializerList(list) => {
                // Later elements wait below earlier ones; an element's
                // designators come before its value.
                for element in list.elements.iter().rev() {
                    work.push(element.initializer);
                }
                let mut designators = ArenaVec::new_in(scratch);
                for element in list.elements {
                    for designator in element.designation.iter().flat_map(|d| d.designators) {
                        match designator.kind {
                            | DesignatorType::Array(index) => designators.push(index.expression()),
                            | DesignatorType::Range(range) => {
                                designators.push(range.lower.expression());
                                designators.push(range.upper.expression());
                            },
                            | _ => {},
                        }
                    }
                }
                for expression in designators {
                    push(expression);
                }
            },
        }
    }
}

/// The operands of an expression, in source order: the syntax children
/// Clang's `Stmt::children()` visits, without type names.
pub(super) fn expression_children<'tu>(
    scratch: &Bump,
    expression: &'tu Expression<'tu>,
    mut push: impl FnMut(&'tu Expression<'tu>),
) {
    match expression.kind {
        | ExpressionType::Parenthesized { expression }
        | ExpressionType::Unary {
            operand_expression: expression,
            ..
        }
        | ExpressionType::SizeofExpr(expression)
        | ExpressionType::AlignofExpr(expression)
        | ExpressionType::DirectMember {
            base_expression: expression,
            ..
        }
        | ExpressionType::IndirectMember {
            base_expression: expression,
            ..
        }
        | ExpressionType::Cast {
            operand_expression: expression,
            ..
        } => push(expression),
        | ExpressionType::Binary {
            left_expression,
            right_expression,
            ..
        } => {
            push(left_expression);
            push(right_expression);
        },
        | ExpressionType::Conditional(conditional) => {
            push(conditional.condition_expression);
            push(conditional.then_expression);
            push(conditional.else_expression);
        },
        | ExpressionType::OmittedConditional(conditional) => {
            push(conditional.condition_expression);
            push(conditional.else_expression);
        },
        | ExpressionType::Call {
            function_expression,
            arguments,
        } => {
            push(function_expression);
            for &argument in arguments {
                push(argument);
            }
        },
        | ExpressionType::CompoundLiteral { initializer, .. } =>
            initializer_expressions(scratch, initializer, &mut push),
        | ExpressionType::Countof(SyntaxOperand::Expression(operand)) => push(operand),
        | ExpressionType::Generic(generic) => {
            if let SyntaxOperand::Expression(operand) = generic.controlling {
                push(operand);
            }
            for association in generic.associations {
                push(association.expression);
            }
        },
        | ExpressionType::Builtin(builtin) => {
            for operand in builtin.operands {
                if let SyntaxOperand::Expression(operand) = *operand {
                    push(operand);
                }
            }
            for member in builtin.members {
                if let OffsetMember::Index(index) = *member {
                    push(index);
                }
            }
        },
        | _ => {},
    }
}
