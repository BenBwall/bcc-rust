//! How Clang reports what the analysis found (`UninitValsDiagReporter` and
//! `DiagnoseUninitializedUse` in `AnalysisBasedWarnings.cpp`): each variable
//! once, at the first use it can report, with the most certain uses first.
//! A use that is always uninitialized is `-Wuninitialized`; one that is
//! uninitialized after a branch names each branch under
//! `-Wsometimes-uninitialized`, with a note where the use is and a note
//! suggesting the branch's removal; one Clang cannot tie to a branch is
//! `-Wconditional-uninitialized`, which `-Wall` leaves off, so the next use
//! is tried. `int x = x;` leaves `x` uninitialized on purpose, which
//! makes its uses only possibly uninitialized, unless one always is.
//! C99: §6.2.4p5, p. 32; PDF p. 44; §5.1.1.3p1, p. 11; PDF p. 23.

use super::{
    super::{
        Analyzer,
        ExpressionType as E,
        RelatedNotes,
        SemanticErrorKind,
        types::TypeKind,
    },
    cfg::{
        Cfg,
        Declared,
        Label,
        Terminator,
    },
    values::{
        Branch,
        Kind,
        Use,
        Uses,
    },
};
use crate::{
    configuration::DiagnosticGroup as G,
    diagnostics::format_in,
    translation_phases::{
        Context,
        SourceVectors,
        parsing::{
            SyntaxOperand,
            declaration_syntax::{
                Declaration,
                DirectDeclarator,
                Initializer,
                InitializerType,
            },
            syntax::{
                BinaryOperator as B,
                BlockItem,
                Expression,
                ExpressionSlot,
                ForInitializer,
                Statement,
                StatementType as S,
                UnaryOperator as U,
            },
        },
        preprocessing::KeywordTokenType,
    },
    util::{
        bump::ArenaVec,
        string_cache::StringCacheId,
    },
};

/// One warning, ready to report, and the notes that follow it.
#[derive(Clone, Copy)]
struct Warning {
    group:     G,
    /// The range reported, whose start is the caret.
    site:      SourceVectors,
    /// A range Clang underlines beside it.
    highlight: SourceVectors,
    text:      StringCacheId,
    notes:     RelatedNotes,
}

impl Warning {
    fn note(&mut self, site: SourceVectors, text: StringCacheId) {
        if let Some(slot) = self.notes.0.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some((site, text));
        }
    }
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Reports each variable in the order the final pass first met it
    /// (`flushDiagnostics`, `diagnoseUnitializedVar`).
    pub(super) fn report_uninitialized(&mut self, cfg: &Cfg<'_, 'tu>, uses: &Uses<'_, 'tu>) {
        let order: ArenaVec<'_, u32> = {
            let mut order = ArenaVec::with_capacity_in(uses.order.len(), self.scratch);
            order.extend(uses.order.iter().copied());
            order
        };
        for &var in &order {
            let Some(declared) = cfg.declared[var as usize] else {
                continue;
            };
            let binding = cfg.vars[var as usize];
            let self_init = uses.self_init[var as usize];
            // Each use with its place in the order the pass found them.
            let mut sorted: ArenaVec<'_, (usize, Use<'tu>)> = ArenaVec::new_in(self.scratch);
            sorted.extend(
                uses.found
                    .iter()
                    .filter(|&&(v, _)| v == var)
                    .map(|&(_, u)| u)
                    .enumerate(),
            );
            if self_init
                && sorted.iter().any(|(_, u)| u.kind >= Kind::AfterDecl)
                && let Some(user) = declared
                    .initializer
                    .and_then(|initializer| self.self_initializer(initializer, binding))
            {
                let always = Use {
                    user,
                    kind: Kind::Always,
                    const_ptr: false,
                    branches: (0, 0),
                };
                if self.diagnose_uninitialized_use(
                    cfg,
                    &uses.branches,
                    binding,
                    declared,
                    always,
                    true,
                ) {
                    continue;
                }
            }
            // Direct uses first, the most certain first, then by place, and
            // uses at one place in the order found. The standard library's
            // stable sort would take its scratch space from the global heap.
            sorted.sort_unstable_by_key(|&(found, u)| {
                let at = self.context.expansion_location(u.user.source_vectors);
                (
                    u.const_ptr,
                    std::cmp::Reverse(u.kind),
                    at.source_file_index,
                    at.index,
                    found,
                )
            });
            for (_, u) in &sorted {
                let reported = if u.const_ptr {
                    let text = self.variable_words(
                        binding,
                        "is uninitialized when passed as a const pointer argument here",
                    );
                    self.emit_uninitialized(Warning {
                        group: G::UninitializedConstPointer,
                        site: u.user.source_vectors,
                        highlight: SourceVectors::default(),
                        text,
                        notes: RelatedNotes::default(),
                    })
                } else {
                    // With `int x = x;`, every use is only possibly
                    // uninitialized.
                    let u = if self_init {
                        Use {
                            kind: Kind::Maybe,
                            branches: (0, 0),
                            ..*u
                        }
                    } else {
                        *u
                    };
                    self.diagnose_uninitialized_use(
                        cfg,
                        &uses.branches,
                        binding,
                        declared,
                        u,
                        false,
                    )
                };
                if reported {
                    break;
                }
            }
        }
    }

    /// `DiagnoseUninitializedUse`: whether the last warning it gave was
    /// reported.
    fn diagnose_uninitialized_use(
        &mut self,
        cfg: &Cfg<'_, 'tu>,
        branches: &[Branch],
        binding: usize,
        declared: Declared<'tu>,
        u: Use<'tu>,
        always_report_self_init: bool,
    ) -> bool {
        let name = self.bindings[binding].name;
        if let Some(initializer) = declared.initializer {
            if !always_report_self_init
                && let InitializerType::AssignmentExpression(value) = initializer.kind
                && std::ptr::eq(self.without_implicit(value), u.user)
            {
                return false;
            }
            if contains(initializer, u.user) {
                let text = self.variable_words(
                    binding,
                    "is uninitialized when used within its own initialization",
                );
                return self.emit_uninitialized(Warning {
                    group: G::UninitializedSelfReference,
                    site: u.user.source_vectors,
                    highlight: name.source_vectors,
                    text,
                    notes: RelatedNotes::default(),
                });
            }
        }
        let use_note = self
            .context
            .string_cache
            .intern("uninitialized use occurs here");
        let mut warnings: ArenaVec<'_, Warning> = ArenaVec::new_in(self.scratch);
        match u.kind {
            | Kind::Always => warnings.push(Warning {
                group:     G::Uninitialized,
                site:      u.user.source_vectors,
                highlight: SourceVectors::default(),
                text:      self.variable_words(binding, "is uninitialized when used here"),
                notes:     RelatedNotes::default(),
            }),
            | Kind::AfterDecl | Kind::AfterCall => {
                let words = if u.kind == Kind::AfterDecl {
                    "is used uninitialized whenever its declaration is reached"
                } else {
                    let function = self
                        .function_name
                        .map_or("", |name| self.context.string_cache.at(name.name));
                    &*format_in!(
                        self.scratch,
                        "is used uninitialized whenever function '{function}' is called"
                    )
                };
                let mut warning = Warning {
                    group:     G::SometimesUninitialized,
                    site:      name.source_vectors,
                    highlight: Self::declared_range(declared),
                    text:      self.variable_words(binding, words),
                    notes:     RelatedNotes::default(),
                };
                warning.note(u.user.source_vectors, use_note);
                warnings.push(warning);
            },
            | Kind::Maybe | Kind::Sometimes => {
                let (start, count) = u.branches;
                for branch in &branches[start as usize..(start + count) as usize] {
                    if let Some(mut warning) = self.branch_warning(cfg, binding, *branch) {
                        let remove = warning.notes.0[0].take();
                        warning.note(u.user.source_vectors, use_note);
                        if let Some((site, text)) = remove {
                            warning.note(site, text);
                        }
                        warnings.push(warning);
                    }
                }
                if warnings.is_empty() {
                    warnings.push(Warning {
                        group:     G::ConditionalUninitialized,
                        site:      u.user.source_vectors,
                        highlight: SourceVectors::default(),
                        text:      self
                            .variable_words(binding, "may be uninitialized when used here"),
                        notes:     RelatedNotes::default(),
                    });
                }
            },
        }
        let (site, text) = self.variable_note(binding, declared);
        if let Some(last) = warnings.last_mut() {
            last.note(site, text);
        }
        let mut reported = false;
        for warning in &warnings {
            reported = self.emit_uninitialized(*warning);
        }
        reported
    }

    /// The warning for one branch after which a use is uninitialized, with
    /// the note that suggests removing it as its first note, or `None` for a
    /// terminator Clang does not name.
    fn branch_warning(
        &mut self,
        cfg: &Cfg<'_, 'tu>,
        binding: usize,
        branch: Branch,
    ) -> Option<Warning> {
        let output = branch.output;
        let condition_words = |true_branch: bool| if true_branch { "true" } else { "false" };
        let (words, range, remove): (&str, SourceVectors, Option<(SourceVectors, &str)>) = match cfg
            .blocks[branch.block as usize]
            .terminator?
        {
            | Terminator::If(statement) => {
                let S::If {
                    condition_expression: ExpressionSlot::Parsed(condition),
                    then_statement,
                    else_statement,
                } = statement.kind
                else {
                    return None;
                };
                let removed = if output == 1 {
                    Context::span_before(statement.source_vectors, then_statement.source_vectors)
                } else {
                    match else_statement {
                        | Some(else_statement) => Context::span_before(
                            statement.source_vectors,
                            else_statement.source_vectors,
                        ),
                        | None => Some(clang_statement_range(statement)),
                    }
                };
                (
                    &*format_in!(
                        self.scratch,
                        "'if' condition is {}",
                        condition_words(output == 0)
                    ),
                    condition.source_vectors,
                    Some((removed?, "remove the 'if' if its condition")),
                )
            },
            | Terminator::Conditional(e) => {
                let (E::Conditional(c) | E::OmittedConditional(c)) = e.kind else {
                    return None;
                };
                let kept = if output == 1 {
                    c.then_expression
                } else {
                    c.else_expression
                };
                (
                    &*format_in!(
                        self.scratch,
                        "'?:' condition is {}",
                        condition_words(output == 0)
                    ),
                    c.condition_expression.source_vectors,
                    Some((
                        Context::span_before(e.source_vectors, kept.source_vectors)?,
                        "remove the '?:' if its condition",
                    )),
                )
            },
            | Terminator::Logical(e) => {
                let E::Binary {
                    operator,
                    left_expression,
                    right_expression,
                } = e.kind
                else {
                    return None;
                };
                let spelling = if operator == B::LogicalAnd {
                    "&&"
                } else {
                    "||"
                };
                // `true && y` and `false || y` remove the left operand
                // and the operator, otherwise the operator becomes
                // its value.
                let removed = if (operator == B::LogicalAnd) == (output == 1) {
                    Context::span_before(e.source_vectors, right_expression.source_vectors)?
                } else {
                    e.source_vectors
                };
                (
                    &*format_in!(
                        self.scratch,
                        "'{spelling}' condition is {}",
                        condition_words(output == 0)
                    ),
                    left_expression.source_vectors,
                    Some((
                        removed,
                        if operator == B::LogicalAnd {
                            "remove the '&&' if its condition"
                        } else {
                            "remove the '||' if its condition"
                        },
                    )),
                )
            },
            | Terminator::While(statement) | Terminator::For(statement) => {
                let (keyword, condition) = match statement.kind {
                    | S::While {
                        condition_expression: ExpressionSlot::Parsed(condition),
                        ..
                    } => ("while", condition),
                    | S::For(for_loop) => match for_loop.condition_expression {
                        | Some(ExpressionSlot::Parsed(condition)) => ("for", condition),
                        | _ => return None,
                    },
                    | _ => return None,
                };
                (
                    &*format_in!(
                        self.scratch,
                        "'{keyword}' loop {}",
                        if output == 0 {
                            "is entered"
                        } else {
                            "exits because its condition is false"
                        }
                    ),
                    condition.source_vectors,
                    Some((condition.source_vectors, "remove the condition if it")),
                )
            },
            | Terminator::Do(statement) => {
                let S::DoWhile {
                    condition_expression: ExpressionSlot::Parsed(condition),
                    ..
                } = statement.kind
                else {
                    return None;
                };
                (
                    if output == 0 {
                        "'do' loop condition is true"
                    } else {
                        "'do' loop exits because its condition is false"
                    },
                    condition.source_vectors,
                    Some((condition.source_vectors, "remove the condition if it")),
                )
            },
            | Terminator::Switch => match cfg.blocks[branch.successor as usize].label {
                | Some(Label::Case(bound)) => ("switch case is taken", bound.source_vectors, None),
                | Some(Label::Default(statement)) => (
                    "switch default is taken",
                    Context::first_token_vector(statement.source_vectors),
                    None,
                ),
                | _ => return None,
            },
            | Terminator::Other => return None,
        };
        let text = self.variable_words(
            binding,
            format_in!(self.scratch, "is used uninitialized whenever {words}"),
        );
        let mut warning = Warning {
            group: G::SometimesUninitialized,
            site: range,
            highlight: SourceVectors::default(),
            text,
            notes: RelatedNotes::default(),
        };
        if let Some((removed, words)) = remove {
            // The condition the branch needs, which the other branch's
            // removal assumes.
            let assumed = if output == 0 { "false" } else { "true" };
            let words = format_in!(self.scratch, "{words} is always {assumed}");
            let words = self.context.string_cache.intern(words);
            warning.note(removed, words);
        }
        Some(warning)
    }

    /// "variable 'x' " followed by `words`, interned.
    fn variable_words(&mut self, binding: usize, words: &str) -> StringCacheId {
        let name = self
            .context
            .string_cache
            .at(self.bindings[binding].name.name);
        let text = format_in!(self.scratch, "variable '{name}' {words}");
        self.context.string_cache.intern(text)
    }

    /// `SuggestInitializationFixit`, or the note naming the declaration:
    /// Clang suggests `= 0` and its kin for a scalar declared without an
    /// initializer outside a macro, but not for an enumeration, a record or
    /// a vector, which have no zero it spells.
    fn variable_note(
        &mut self,
        binding: usize,
        declared: Declared<'tu>,
    ) -> (SourceVectors, StringCacheId) {
        let ty = self.bindings[binding].ty;
        let zero = !matches!(
            self.types.nodes[ty.index],
            TypeKind::Tag(_) | TypeKind::Vector { .. }
        );
        let end = Self::declarator_end(declared);
        if zero
            && declared.initializer.is_none()
            && let Some(end) = end
            && !self.context.produced_by_macro_expansion(end)
        {
            let location = self.context.file_location(end);
            let vector = self.context.file_vector(location).clone();
            let site = self.context.retain_source_end(&vector);
            let name = self
                .context
                .string_cache
                .at(self.bindings[binding].name.name);
            let text = format_in!(
                self.scratch,
                "initialize the variable '{name}' to silence this warning"
            );
            return (site, self.context.string_cache.intern(text));
        }
        let start = Context::first_token_vector(declared.declaration.source_vectors);
        let site = self.context.retain_source_start(start);
        (site, self.variable_words(binding, "is declared here"))
    }

    /// The last token of the declarator, without its attributes and `asm`
    /// label (`VarDecl::getEndLoc` without an initializer).
    fn declarator_end(declared: Declared<'tu>) -> Option<SourceVectors> {
        let declarator = declared.declarator;
        let trailing = declarator.kind.iter().find_map(|direct| match *direct {
            | DirectDeclarator::Attributes(attributes) => Some(attributes.source_vectors),
            | DirectDeclarator::AsmLabel(asm) => Some(asm.source_vectors),
            | _ => None,
        });
        match trailing {
            | Some(trailing) => Context::token_before(declarator.source_vectors, trailing),
            | None => Some(Context::last_source_vector(declarator.source_vectors)),
        }
    }

    /// The declaration from its first token through the declarator
    /// (`VarDecl::getSourceRange` without an initializer).
    fn declared_range(declared: Declared<'tu>) -> SourceVectors {
        Self::declarator_end(declared)
            .and_then(|end| Context::span_through(declared.declaration.source_vectors, end))
            .unwrap_or(declared.declarator.source_vectors)
    }

    /// Reports `warning` with its notes, returning whether it was reported:
    /// Clang's `isLastDiagnosticIgnored` after it.
    fn emit_uninitialized(&mut self, warning: Warning) -> bool {
        if self.tainted
            || self
                .context
                .group_severity(warning.group, warning.site)
                .is_none()
        {
            return false;
        }
        self.with_highlights([warning.highlight, SourceVectors::default()], |analyzer| {
            analyzer.with_related_notes(warning.notes, |analyzer| {
                analyzer.group_diagnostic(
                    warning.group,
                    SemanticErrorKind::Uninitialized(warning.group),
                    warning.site,
                    Some(warning.text),
                    None,
                );
            });
        });
        true
    }

    /// `Expr::IgnoreParenImpCasts`: through parentheses and what Clang's
    /// `IgnoreParens` steps through.
    fn without_implicit(&self, mut e: &'tu Expression<'tu>) -> &'tu Expression<'tu> {
        loop {
            e = match e.kind {
                | E::Parenthesized { expression }
                | E::Unary {
                    operator: U::Extension,
                    operand_expression: expression,
                } => expression,
                | E::Builtin(builtin) if builtin.keyword == KeywordTokenType::BuiltinChooseExpr => {
                    let [
                        SyntaxOperand::Expression(condition),
                        SyntaxOperand::Expression(yes),
                        SyntaxOperand::Expression(no),
                    ] = *builtin.operands.as_slice()
                    else {
                        return e;
                    };
                    match Self::constant_truth(self.expression_info(condition)) {
                        | Some(true) => yes,
                        | Some(false) => no,
                        | None => return e,
                    }
                },
                | _ => return e,
            };
        }
    }
}

/// A statement's range as Clang's `Stmt::getSourceRange` gives it, which
/// ends before the `;` of a statement that ends with an expression or a
/// jump.
fn clang_statement_range(statement: &Statement<'_>) -> SourceVectors {
    let mut last = statement;
    loop {
        last = match last.kind {
            | S::If {
                then_statement,
                else_statement,
                ..
            } => else_statement.unwrap_or(then_statement),
            | S::While { body_statement, .. } | S::Switch { body_statement, .. } => body_statement,
            | S::For(for_loop) => for_loop.body_statement,
            | S::Label(_, child) | S::Case(_, child) | S::Default(child) => child,
            | S::CaseRange(range) => range.statement,
            | S::Attributed(attributed) => attributed.statement,
            | _ => break,
        };
    }
    let semicolon = matches!(
        last.kind,
        S::Expression(_)
            | S::Return(_)
            | S::Goto(_)
            | S::ComputedGoto(_)
            | S::Break
            | S::Continue
            | S::DoWhile { .. }
            | S::Asm(_)
    );
    let end = Context::last_source_vector(statement.source_vectors);
    if semicolon && let Some(trimmed) = Context::span_before(statement.source_vectors, end) {
        trimmed
    } else {
        statement.source_vectors
    }
}

/// Whether `user` is part of `initializer` (Clang's `ContainsReference`,
/// which the evaluated parts are enough for: the analysis met `user` where
/// it is evaluated).
fn contains(initializer: &Initializer<'_>, user: &Expression<'_>) -> bool {
    match initializer.kind {
        | InitializerType::AssignmentExpression(e) => expression_contains(e, user),
        | InitializerType::InitializerList(list) => list
            .elements
            .iter()
            .any(|element| contains(element.initializer, user)),
    }
}

fn expression_contains(e: &Expression<'_>, user: &Expression<'_>) -> bool {
    if std::ptr::eq(e, user) {
        return true;
    }
    match e.kind {
        | E::Parenthesized { expression }
        | E::Unary {
            operand_expression: expression,
            ..
        }
        | E::Cast {
            operand_expression: expression,
            ..
        }
        | E::DirectMember {
            base_expression: expression,
            ..
        }
        | E::IndirectMember {
            base_expression: expression,
            ..
        } => expression_contains(expression, user),
        | E::Binary {
            left_expression,
            right_expression,
            ..
        } =>
            expression_contains(left_expression, user)
                || expression_contains(right_expression, user),
        | E::Conditional(c) | E::OmittedConditional(c) =>
            expression_contains(c.condition_expression, user)
                || expression_contains(c.then_expression, user)
                || expression_contains(c.else_expression, user),
        | E::Call {
            function_expression,
            arguments,
        } =>
            expression_contains(function_expression, user)
                || arguments
                    .iter()
                    .any(|argument| expression_contains(argument, user)),
        | E::CompoundLiteral { initializer, .. } => contains(initializer, user),
        | E::Generic(generic) => generic
            .associations
            .iter()
            .any(|association| expression_contains(association.expression, user)),
        | E::Builtin(builtin) => builtin.operands.iter().any(|operand| match *operand {
            | SyntaxOperand::Expression(operand) => expression_contains(operand, user),
            | SyntaxOperand::Type(_) => false,
        }),
        | E::StatementExpression(statement) => statement_contains(statement, user),
        | _ => false,
    }
}

/// Whether a statement expression's statements hold `user`.
fn statement_contains(statement: &Statement<'_>, user: &Expression<'_>) -> bool {
    let slot = |slot: ExpressionSlot<'_>| match slot {
        | ExpressionSlot::Parsed(e) => expression_contains(e, user),
        | _ => false,
    };
    let declaration = |declaration: &Declaration<'_>| {
        declaration.init_declarators.iter().any(|init| {
            init.initializer
                .is_some_and(|initializer| contains(initializer, user))
        })
    };
    match statement.kind {
        | S::Compound { items } => items.iter().any(|item| match *item {
            | BlockItem::Statement(statement) => statement_contains(statement, user),
            | BlockItem::Declaration(d) => declaration(d),
            | BlockItem::FunctionDefinition(_) => false,
        }),
        | S::Declaration(d) => declaration(d),
        | S::Expression(e) | S::ComputedGoto(e) | S::Return(Some(e)) => slot(e),
        | S::If {
            condition_expression,
            then_statement,
            else_statement,
        } =>
            slot(condition_expression)
                || statement_contains(then_statement, user)
                || else_statement.is_some_and(|e| statement_contains(e, user)),
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
        } => slot(condition_expression) || statement_contains(body_statement, user),
        | S::For(for_loop) => {
            let initializer = match for_loop.initializer {
                | Some(ForInitializer::Declaration(d)) => declaration(d),
                | Some(ForInitializer::Expression(e)) => slot(e),
                | None => false,
            };
            initializer
                || for_loop.condition_expression.is_some_and(slot)
                || for_loop.iteration_expression.is_some_and(slot)
                || statement_contains(for_loop.body_statement, user)
        },
        | S::Case(_, child) | S::Default(child) | S::Label(_, child) =>
            statement_contains(child, user),
        | S::CaseRange(range) => statement_contains(range.statement, user),
        | S::Attributed(attributed) => statement_contains(attributed.statement, user),
        | _ => false,
    }
}
