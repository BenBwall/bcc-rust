//! GCC's and Clang's warnings that a program's parentheses may not say what
//! it means: `-Wparentheses` and what it holds, `-Wlogical-not-parentheses`,
//! `-Wdangling-else`, and Clang's `-Wswitch-bool`. C99 §6.5 and §6.8.4.1p3
//! define what each of these programs means, and each warning is one
//! §5.1.1.3p1 permits. Parentheses are syntax nodes, so an operand that is a
//! binary expression here has none. The compilers ask different questions, so
//! each has its own reading, which follows `warn_about_parentheses` and
//! `warn_logical_not_parentheses` in GCC and `DiagnoseBinOpPrecedence` and
//! `diagnoseLogicalNotOnLHSofCheck` in Clang.
//! C99: §6.5p3, p. 67; PDF p. 79; §6.8.4.1p3, p. 133; PDF p. 145.

use super::{
    Analyzer,
    ExpressionInfo,
    SemanticErrorKind,
    expressions::ValueCategory,
    types::{
        Scalar,
        TypeKind,
    },
};
use crate::{
    configuration::{
        CompilerIdentity,
        DiagnosticGroup as G,
    },
    diagnostics::format_in,
    translation_phases::{
        SourceVectors,
        parsing::syntax::{
            BinaryOperator as B,
            ConditionalExpression,
            Expression,
            ExpressionSlot,
            ExpressionType as E,
            Statement,
            StatementType as S,
            UnaryOperator as U,
        },
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// The warnings a binary expression may draw from its operators.
    pub(super) fn check_operator_warnings(
        &mut self,
        e: &'tu Expression<'tu>,
        operator: B,
        left: &'tu Expression<'tu>,
        right: &'tu Expression<'tu>,
    ) {
        self.check_compared_signs(e, operator, left, right);
        self.check_operator_parentheses(e, operator, left, right);
    }

    /// The warnings a conditional expression may draw from its operands.
    pub(super) fn check_conditional_warnings(
        &mut self,
        e: &'tu Expression<'tu>,
        c: &'tu ConditionalExpression<'tu>,
    ) {
        self.check_conditional_precedence(e, c);
        if self.unused.sign_compare {
            self.check_conditional_sign(c.then_expression, c.else_expression);
        }
    }

    /// Checks the binary expression `e`, if a warning of this family may be
    /// on.
    /// C99: §6.5p3, p. 67; PDF p. 79.
    fn check_operator_parentheses(
        &mut self,
        e: &'tu Expression<'tu>,
        operator: B,
        left: &'tu Expression<'tu>,
        right: &'tu Expression<'tu>,
    ) {
        if !self.unused.parentheses {
            return;
        }
        let site = e.operator_source_vectors.unwrap_or(e.source_vectors);
        if self.is_gcc() {
            self.gcc_operator_parentheses(site, operator, left, right);
        } else {
            self.clang_operator_parentheses(site, operator, left, right);
        }
    }

    /// `warn_about_parentheses`, and the logical not `parser_build_binary_op`
    /// checks beside it. A warning goes at its operand's operator, or at this
    /// one when the operand folds to a constant and so has no location.
    fn gcc_operator_parentheses(
        &mut self,
        site: SourceVectors,
        operator: B,
        left: &'tu Expression<'tu>,
        right: &'tu Expression<'tu>,
    ) {
        let name = spelling(operator);
        // The first of these an operand meets is the only one reported.
        let pick = |wanted: &dyn Fn(B) -> bool| {
            [left, right]
                .into_iter()
                .find(|&operand| binary_operator(operand).is_some_and(wanted))
        };
        let additive = |inner| matches!(inner, B::Addition | B::Subtraction);
        let plus = |inner| inner == B::Addition;
        let found = match operator {
            | B::LeftShift | B::RightShift => pick(&plus)
                .or_else(|| pick(&additive))
                .map(|operand| (operand, Gcc::Inside)),
            | B::LogicalOr => pick(&|inner| inner == B::LogicalAnd).map(|e| (e, Gcc::Within)),
            | B::BitwiseOr =>
                pick(&|inner| additive(inner) || matches!(inner, B::BitwiseAnd | B::BitwiseXor))
                    .map(|e| (e, Gcc::Arithmetic))
                    .or_else(|| pick(&is_comparison).map(|e| (e, Gcc::Comparison))),
            | B::BitwiseXor => pick(&|inner| additive(inner) || inner == B::BitwiseAnd)
                .map(|e| (e, Gcc::Arithmetic))
                .or_else(|| pick(&is_comparison).map(|e| (e, Gcc::Comparison))),
            | B::BitwiseAnd => pick(&plus)
                .or_else(|| pick(&additive))
                .map(|e| (e, Gcc::InOperand))
                .or_else(|| pick(&is_comparison).map(|e| (e, Gcc::Comparison))),
            | B::Equal | B::NotEqual => pick(&is_comparison).map(|e| (e, Gcc::Comparison)),
            | _ if is_relational(operator) => pick(&is_relational).map(|e| (e, Gcc::Chain)),
            | _ => None,
        };
        if let Some((operand, kind)) = found {
            let inner = spelling(binary_operator(operand).unwrap_or(operator));
            let text = match kind {
                | Gcc::Inside => format_in!(
                    self.scratch,
                    "suggest parentheses around '{inner}' inside '{name}'"
                ),
                | Gcc::Within => "suggest parentheses around '&&' within '||'",
                | Gcc::Arithmetic => format_in!(
                    self.scratch,
                    "suggest parentheses around arithmetic in operand of '{name}'"
                ),
                | Gcc::InOperand => format_in!(
                    self.scratch,
                    "suggest parentheses around '{inner}' in operand of '{name}'"
                ),
                | Gcc::Comparison => format_in!(
                    self.scratch,
                    "suggest parentheses around comparison in operand of '{name}'"
                ),
                | Gcc::Chain => "comparisons like 'X<=Y<=Z' do not have their mathematical meaning",
            };
            let at = if self.expression_info(operand).integer.is_some() {
                site
            } else {
                operand
                    .operator_source_vectors
                    .unwrap_or(operand.source_vectors)
            };
            self.parentheses_warning(G::Parentheses, at, text);
        } else if logical_not(left).is_some()
            && matches!(operator, B::BitwiseAnd | B::BitwiseOr)
            && !self.gcc_truth_operand(right)
        {
            let text = format_in!(
                self.scratch,
                "suggest parentheses around operand of '!' or change '{name}' to '{name}{name}' \
                 or '!' to '~'"
            );
            self.parentheses_warning(G::Parentheses, left.source_vectors, text);
        }
        if is_comparison(operator) && self.gcc_logical_not_warns(operator, left, right) {
            self.parentheses_warning(
                G::LogicalNotParentheses,
                site,
                "logical not is only applied to the left hand side of comparison",
            );
        }
    }

    /// Whether `!x op right` is GCC's `warn_logical_not_parentheses`. What it
    /// does not ask of is what folding already made of `!x`: `!!x` and
    /// `!(x == 0)` are `x != 0`, and `!b` of a `_Bool` compares it with zero.
    fn gcc_logical_not_warns(
        &self,
        operator: B,
        left: &'tu Expression<'tu>,
        right: &'tu Expression<'tu>,
    ) -> bool {
        let Some(operand) = logical_not(left) else {
            return false;
        };
        if logical_not(right).is_some() || self.is_bool(self.expression_info(operand)) {
            return false;
        }
        let operand = strip_parentheses(operand);
        if logical_not(operand).is_some() {
            return false;
        }
        if let E::Binary {
            operator: B::Equal,
            right_expression,
            ..
        } = operand.kind
            && self.is_integer_zero(right_expression)
        {
            return false;
        }
        let right = strip_parentheses(right);
        if self.is_bool(self.expression_info(right))
            || binary_operator(right).is_some_and(|inner| {
                is_comparison(inner) || matches!(inner, B::LogicalAnd | B::LogicalOr)
            })
            || logical_not(right).is_some()
            || self.gcc_has_boolean_operands(right)
        {
            return false;
        }
        !(matches!(operator, B::Equal | B::NotEqual) && self.is_integer_zero(right))
    }

    /// GCC's `expr_has_boolean_operands_p`: `!x == ~(a | b)` of `_Bool`s.
    fn gcc_has_boolean_operands(&self, e: &'tu Expression<'tu>) -> bool {
        let e = strip_parentheses(e);
        if self.is_bool(self.expression_info(e)) {
            return true;
        }
        match e.kind {
            | E::Unary {
                operator: U::Plus | U::Minus | U::BitwiseNot,
                operand_expression,
            } => self.gcc_has_boolean_operands(operand_expression),
            | E::Binary {
                operator,
                left_expression,
                right_expression,
            } =>
                is_comparison(operator)
                    || (matches!(
                        operator,
                        B::Addition
                            | B::Subtraction
                            | B::Multiplication
                            | B::Division
                            | B::Modulo
                            | B::LeftShift
                            | B::RightShift
                            | B::BitwiseAnd
                            | B::BitwiseOr
                            | B::BitwiseXor
                    ) && self.gcc_has_boolean_operands(left_expression)
                        && self.gcc_has_boolean_operands(right_expression)),
            | _ => false,
        }
    }

    /// `DiagnoseBinOpPrecedence`, Clang's chained comparison, and its check of
    /// a logical not.
    fn clang_operator_parentheses(
        &mut self,
        site: SourceVectors,
        operator: B,
        left: &'tu Expression<'tu>,
        right: &'tu Expression<'tu>,
    ) {
        let name = spelling(operator);
        if matches!(operator, B::BitwiseAnd | B::BitwiseOr | B::BitwiseXor) {
            let (left_inner, right_inner) = (binary_operator(left), binary_operator(right));
            let compared = |inner: Option<B>| inner.is_some_and(is_comparison);
            let bitwise = |inner: Option<B>| {
                matches!(inner, Some(B::BitwiseAnd | B::BitwiseOr | B::BitwiseXor))
            };
            if compared(left_inner) != compared(right_inner)
                && !bitwise(left_inner)
                && !bitwise(right_inner)
            {
                let compared = spelling(
                    left_inner
                        .filter(|&inner| is_comparison(inner))
                        .or(right_inner)
                        .unwrap_or(operator),
                );
                let text = format_in!(
                    self.scratch,
                    "{name} has lower precedence than {compared}; {compared} will be evaluated \
                     first"
                );
                self.parentheses_warning(G::Parentheses, site, text);
            }
        }
        let in_macro = self.context.produced_by_macro_expansion(site);
        for operand in [left, right] {
            let Some(inner) = binary_operator(operand) else {
                continue;
            };
            let at = operand
                .operator_source_vectors
                .unwrap_or(operand.source_vectors);
            if !in_macro && precedes_within_bitwise(inner, operator) {
                let text = format_in!(self.scratch, "'{}' within '{name}'", spelling(inner));
                self.parentheses_warning(G::BitwiseOpParentheses, at, text);
            }
            if matches!(operator, B::LeftShift | B::RightShift)
                && matches!(inner, B::Addition | B::Subtraction)
            {
                let inner = spelling(inner);
                let text = format_in!(
                    self.scratch,
                    "operator '{name}' has lower precedence than '{inner}'; '{inner}' will be \
                     evaluated first"
                );
                self.parentheses_warning(G::ShiftOpParentheses, at, text);
            }
        }
        if operator == B::LogicalOr && !in_macro {
            self.clang_logical_and_in_or(left, right);
        }
        if is_relational(operator)
            && let Some(inner) = binary_operator(left).filter(|&inner| is_relational(inner))
        {
            let text = format_in!(
                self.scratch,
                "chained comparison 'X {} Y {name} Z' does not behave the same as a mathematical \
                 expression",
                spelling(inner)
            );
            // A program with this meaning is valid C, so only Clang's own
            // personality refuses it; plain bcc warns.
            let group = if self.context.configuration.compiler_extensions() {
                G::ChainedComparison
            } else {
                G::Parentheses
            };
            self.parentheses_warning(group, site, text);
        }
        if is_comparison(operator) || operator == B::BitwiseAnd {
            self.clang_logical_not(operator, left, right);
        }
    }

    /// `DiagnoseLogicalAndInLogicalOrLHS` and `...RHS`: a string literal says
    /// what `assert(a || b && "why")` is for, and `"why" && a || b`.
    fn clang_logical_and_in_or(&mut self, left: &'tu Expression<'tu>, right: &'tu Expression<'tu>) {
        let string_literal =
            |e: &'tu Expression<'tu>| matches!(strip_parentheses(e).kind, E::StringLiteral(..));
        let warned = match left.kind {
            | E::Binary {
                operator: B::LogicalAnd,
                left_expression,
                ..
            } if !string_literal(left_expression) => Some(left),
            | E::Binary {
                operator: B::LogicalOr,
                right_expression: inner,
                ..
            } => match inner.kind {
                | E::Binary {
                    operator: B::LogicalAnd,
                    right_expression,
                    ..
                } if string_literal(right_expression) => Some(inner),
                | _ => None,
            },
            | _ => None,
        };
        if let Some(operand) = warned {
            self.logical_and_warning(operand);
        }
        if let E::Binary {
            operator: B::LogicalAnd,
            right_expression,
            ..
        } = right.kind
            && !string_literal(right_expression)
        {
            self.logical_and_warning(right);
        }
    }

    fn logical_and_warning(&mut self, operand: &'tu Expression<'tu>) {
        self.parentheses_warning(
            G::LogicalOpParentheses,
            operand
                .operator_source_vectors
                .unwrap_or(operand.source_vectors),
            "'&&' within '||'",
        );
    }

    /// `diagnoseLogicalNotOnLHSofCheck`.
    fn clang_logical_not(
        &mut self,
        operator: B,
        left: &'tu Expression<'tu>,
        right: &'tu Expression<'tu>,
    ) {
        let Some(operand) = logical_not(left) else {
            return;
        };
        if self.clang_is_boolean(right) || self.clang_is_boolean(operand) {
            return;
        }
        let what = if operator == B::BitwiseAnd {
            "bitwise operator"
        } else {
            "comparison"
        };
        let text = format_in!(
            self.scratch,
            "logical not is only applied to the left hand side of this {what}"
        );
        self.parentheses_warning(G::LogicalNotParentheses, left.source_vectors, text);
    }

    /// Clang's `Expr::isKnownToHaveBooleanValue`. A cast the program wrote
    /// says what it means, as a literal does.
    fn clang_is_boolean(&self, e: &'tu Expression<'tu>) -> bool {
        let e = strip_parentheses(e);
        if self.is_bool(self.expression_info(e)) {
            return true;
        }
        match e.kind {
            | E::Unary {
                operator: U::LogicalNot,
                ..
            } => true,
            | E::Unary {
                operator: U::Plus,
                operand_expression,
            } => self.clang_is_boolean(operand_expression),
            | E::Binary {
                operator,
                left_expression,
                right_expression,
            } => match operator {
                | B::LogicalAnd | B::LogicalOr => true,
                | B::Comma | B::Assignment => self.clang_is_boolean(right_expression),
                | B::BitwiseAnd | B::BitwiseOr | B::BitwiseXor =>
                    self.clang_is_boolean(left_expression)
                        && self.clang_is_boolean(right_expression),
                | _ => is_comparison(operator),
            },
            | E::Conditional(c) =>
                self.clang_is_boolean(c.then_expression) && self.clang_is_boolean(c.else_expression),
            | _ => false,
        }
    }

    /// Checks the condition of an `if` or a loop: Clang's
    /// `CheckBooleanCondition`, and GCC's truth value conversion, which meets
    /// an unparenthesized assignment nowhere else.
    /// C99: §6.8.4.1p1, p. 133; PDF p. 145; §6.8.5p2, p. 135; PDF p. 147.
    pub(super) fn check_condition_parentheses(&mut self, slot: ExpressionSlot<'tu>) {
        if !self.unused.parentheses {
            return;
        }
        let Some(e) = super::statements::slot_expression(slot) else {
            return;
        };
        let operator = binary_operator(e);
        if self.is_gcc() {
            if operator == Some(B::Assignment) {
                self.parentheses_warning(
                    G::Parentheses,
                    e.source_vectors,
                    "suggest parentheses around assignment used as truth value",
                );
            }
            return;
        }
        if matches!(operator, Some(B::Assignment | B::BitwiseOrAssignment)) {
            self.parentheses_warning(
                G::Parentheses,
                e.operator_source_vectors.unwrap_or(e.source_vectors),
                "using the result of an assignment as a condition without parentheses",
            );
        }
        if matches!(e.kind, E::Parenthesized { .. })
            && !self.context.produced_by_macro_expansion(e.source_vectors)
            && let inner = strip_parentheses(e)
            && let E::Binary {
                operator: B::Equal,
                left_expression,
                ..
            } = inner.kind
            && self
                .expression_info(strip_parentheses(left_expression))
                .category
                == ValueCategory::ModifiableLvalue
        {
            self.parentheses_warning(
                G::ParenthesesEquality,
                inner
                    .operator_source_vectors
                    .unwrap_or(inner.source_vectors),
                "equality comparison with extraneous parentheses",
            );
        }
    }

    /// Clang's `DiagnoseConditionalPrecedence`: `a + (b < c) ? d : e`.
    /// C99: §6.5.15, p. 90; PDF p. 102.
    fn check_conditional_precedence(
        &mut self,
        e: &'tu Expression<'tu>,
        c: &'tu ConditionalExpression<'tu>,
    ) {
        if !self.unused.parentheses || self.is_gcc() {
            return;
        }
        let E::Binary {
            operator,
            right_expression,
            ..
        } = c.condition_expression.kind
        else {
            return;
        };
        let group = match operator {
            | B::BitwiseAnd | B::BitwiseOr => G::BitwiseConditionalParentheses,
            | B::Addition
            | B::Subtraction
            | B::Multiplication
            | B::Division
            | B::Modulo
            | B::LeftShift
            | B::RightShift => G::Parentheses,
            | _ => return,
        };
        let operand = strip_parentheses(right_expression);
        let looks_boolean = self.is_bool(self.expression_info(operand))
            || logical_not(operand).is_some()
            || binary_operator(operand).is_some_and(|inner| {
                is_comparison(inner) || matches!(inner, B::LogicalAnd | B::LogicalOr)
            });
        if looks_boolean {
            let name = spelling(operator);
            let text = format_in!(
                self.scratch,
                "operator '?:' has lower precedence than '{name}'; '{name}' will be evaluated \
                 first"
            );
            self.parentheses_warning(
                group,
                e.operator_source_vectors.unwrap_or(e.source_vectors),
                text,
            );
        }
    }

    /// Clang's `-Wswitch-bool`. GCC's asks about a `case` outside the range of
    /// `_Bool` instead.
    /// C99: §6.8.4.2p1, p. 134; PDF p. 146.
    pub(super) fn check_switch_condition(
        &mut self,
        site: SourceVectors,
        slot: ExpressionSlot<'tu>,
    ) {
        if !self.unused.parentheses || self.is_gcc() {
            return;
        }
        if let Some(e) = super::statements::slot_expression(slot)
            && self.clang_is_boolean(e)
        {
            self.parentheses_warning(G::SwitchBool, site, "switch condition has boolean value");
        }
    }

    /// `if (a) if (b) x; else y;`, whose `else` §6.8.4.1p3 gives to the nearest
    /// `if`. GCC looks through a label or `switch`, and Clang through a loop
    /// alone. GCC reports the outer `if`'s `(`, and Clang the `else`.
    /// C99: §6.8.4.1p3, p. 133; PDF p. 145.
    pub(super) fn check_dangling_else(
        &mut self,
        s: &'tu Statement<'tu>,
        then_statement: &'tu Statement<'tu>,
    ) {
        if !self.unused.parentheses {
            return;
        }
        let gcc = self.is_gcc();
        let mut statement = then_statement;
        let otherwise = loop {
            statement = match statement.kind {
                | S::If { else_statement, .. } => break else_statement,
                | S::While { body_statement, .. } => body_statement,
                | S::For(f) => f.body_statement,
                | S::Switch { body_statement, .. } if gcc => body_statement,
                | S::Label(_, child) | S::Case(_, child) | S::Default(child) if gcc => child,
                | _ => return,
            };
        };
        let Some(otherwise) = otherwise else {
            return;
        };
        if gcc {
            let site = self.context.dangling_else_site(then_statement);
            self.parentheses_warning(
                G::DanglingElse,
                site.unwrap_or(s.source_vectors),
                "suggest explicit braces to avoid ambiguous 'else'",
            );
        } else if let Some(keyword) = self.context.dangling_else_site(otherwise) {
            self.parentheses_warning(
                G::DanglingElse,
                keyword,
                "add explicit braces to avoid dangling else",
            );
        }
    }

    fn parentheses_warning(&mut self, group: G, site: SourceVectors, text: &str) {
        let name = self.context.string_cache.intern(text);
        self.spelled_group_diagnostic(
            group,
            SemanticErrorKind::Parentheses(group),
            site,
            Some(name),
            site,
        );
    }

    fn is_gcc(&self) -> bool {
        self.context.configuration.compiler_identity() == CompilerIdentity::Gcc
    }

    fn is_bool(&self, info: ExpressionInfo<'tu>) -> bool {
        matches!(
            self.types.nodes[self.types.non_atomic(info.ty).unqualified().index],
            TypeKind::Scalar(Scalar::Bool)
        )
    }

    /// Whether GCC takes `e`, the right operand of `!x & e` or `!x | e`, for
    /// a truth value and so leaves the `!` unquestioned (GCC manual, "Warning
    /// Options", `-Wparentheses`): a comparison, `&&`, `||` or `!` (with or
    /// without parentheses around it), an operand of type `_Bool`, or the
    /// constant 0 or 1. Measured against GCC 13 and 14: `!a & (b != 0)`,
    /// `!a | !b`, `!a & (_Bool)b` and `!a & 1` pass; `!a & b`, `!a & 2` and
    /// `!a & (b ? 1 : 0)` are reported.
    fn gcc_truth_operand(&self, e: &'tu Expression<'tu>) -> bool {
        let bare = strip_parentheses(e);
        logical_not(bare).is_some()
            || binary_operator(bare)
                .is_some_and(|op| is_comparison(op) || matches!(op, B::LogicalAnd | B::LogicalOr))
            || self.is_bool(self.expression_info(e))
            || self
                .expression_info(e)
                .integer
                .is_some_and(|value| value.value == 0 || value.value == 1)
    }

    fn is_integer_zero(&self, e: &'tu Expression<'tu>) -> bool {
        self.expression_info(e)
            .integer
            .is_some_and(|value| value.value == 0)
    }
}

/// What GCC says of an operand it would rather see parenthesized.
#[derive(Clone, Copy)]
enum Gcc {
    Inside,
    Within,
    Arithmetic,
    InOperand,
    Comparison,
    Chain,
}

fn binary_operator(e: &Expression<'_>) -> Option<B> {
    match e.kind {
        | E::Binary { operator, .. } => Some(operator),
        | _ => None,
    }
}

/// The operand of `e`, if `e` is a logical not with no parentheses around it.
fn logical_not<'tu>(e: &Expression<'tu>) -> Option<&'tu Expression<'tu>> {
    match e.kind {
        | E::Unary {
            operator: U::LogicalNot,
            operand_expression,
        } => Some(operand_expression),
        | _ => None,
    }
}

fn strip_parentheses<'tu>(mut e: &'tu Expression<'tu>) -> &'tu Expression<'tu> {
    while let E::Parenthesized { expression } = e.kind {
        e = expression;
    }
    e
}

const fn is_relational(operator: B) -> bool {
    matches!(
        operator,
        B::LessThan | B::GreaterThan | B::LessThanOrEqual | B::GreaterThanOrEqual
    )
}

const fn is_comparison(operator: B) -> bool {
    is_relational(operator) || matches!(operator, B::Equal | B::NotEqual)
}

/// Whether `inner` binds tighter than the bitwise `outer`, as `&` within `^`
/// and either of them within `|`.
const fn precedes_within_bitwise(inner: B, outer: B) -> bool {
    matches!(
        (inner, outer),
        (B::BitwiseAnd, B::BitwiseXor | B::BitwiseOr) | (B::BitwiseXor, B::BitwiseOr)
    )
}

const fn spelling(operator: B) -> &'static str {
    match operator {
        | B::Multiplication => "*",
        | B::Division => "/",
        | B::Modulo => "%",
        | B::Addition => "+",
        | B::Subtraction => "-",
        | B::LeftShift => "<<",
        | B::RightShift => ">>",
        | B::LessThan => "<",
        | B::GreaterThan => ">",
        | B::LessThanOrEqual => "<=",
        | B::GreaterThanOrEqual => ">=",
        | B::Equal => "==",
        | B::NotEqual => "!=",
        | B::BitwiseAnd => "&",
        | B::BitwiseXor => "^",
        | B::BitwiseOr => "|",
        | B::LogicalAnd => "&&",
        | B::LogicalOr => "||",
        | _ => "",
    }
}
