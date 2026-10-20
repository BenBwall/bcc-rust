//! Where Clang's code generation emits an expression statement's code: a
//! call that does not return leaves a new block without predecessors, the
//! rest of the expression is emitted there, and `CodeGenFunction::EmitStmt`
//! erases such a block when the statement created it, so the statements
//! that follow are skipped until a label. An expression that branches and
//! joins its paths in a block with predecessors ends that state. Clang's
//! expression emitters that do, with default options and no sanitizers:
//! - `&&` and `||` whose left operand does not fold (`land.end`, `lor.end`);
//! - a scalar `?:` whose condition does not fold, unless both operands are
//!   evaluatable constants, which it selects (`cond.end`); a complex or
//!   aggregate `?:` always (`ComplexExprEmitter` and `AggExprEmitter` do not
//!   fold their condition);
//! - a multiplication of two floating complex operands, which checks the result
//!   for NaNs (`complex_mul_cont`);
//! - an atomic compound assignment or increment that `atomicrmw` cannot
//!   perform, which loops on `cmpxchg` (`atomic_cont`);
//! - an atomic compare-and-exchange builtin (`cmpxchg.continue`);
//! - `va_arg` of a scalar on the System V ABI, which reads registers or memory
//!   (`vaarg.end`).
//!
//! Only an evaluated, emitted expression counts: neither a `sizeof`,
//! `alignof` or `typeof` operand nor an operand a folded condition skips.
//! A statement expression's value statement is emitted even after a jump
//! when the expression has a value, and keeps its block.
//! C99: §6.8.3 paragraph 2, p. 132; PDF p. 144; §6.5.13-§6.5.15,
//! pp. 89-91; PDF pp. 101-103; §6.5.5 paragraph 3, p. 82; PDF p. 94.
//! C11: §6.5.16.2 paragraph 3, pp. 103-104; PDF pp. 121-122 (atomic
//! compound assignment); §7.17.7.4 paragraphs 1-5, pp. 283-284;
//! PDF pp. 301-302.
//! GNU extension: GCC manual, "Statement Exprs".

use super::super::{
    Analyzer,
    BindingOrigin,
    TypeKind,
    Work,
    functions::FunctionWork,
    types::{
        Scalar,
        TypeId,
    },
};
use crate::{
    builtins::BuiltinId,
    target::Target,
    translation_phases::{
        parsing::syntax::{
            BinaryOperator,
            BlockItem,
            ConditionalExpression,
            Expression,
            ExpressionSlot,
            ExpressionType,
            Statement,
            StatementType,
            UnaryOperator,
        },
        preprocessing::KeywordTokenType,
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Follows the blocks code generation creates for expression `e`, once
    /// it is analyzed: an evaluated call that does not return leaves a block
    /// without predecessors, and an expression that joins its paths leaves
    /// one with predecessors.
    /// C99: §6.5.2.2 paragraph 10, p. 72; PDF p. 84.
    pub(in crate::translation_phases::semantic_analysis) fn expression_emitted(
        &mut self,
        e: &'tu Expression<'tu>,
    ) {
        if self.functions.dead
            || self.functions.sizeof.is_some()
            || self.functions.reference_context.is_some()
        {
            return;
        }
        if let ExpressionType::Call {
            function_expression,
            ..
        } = e.kind
            && self.call_does_not_return(function_expression)
        {
            self.functions.emission.orphan = Some(self.functions.emission.serial);
        } else if self.joins_paths(e) {
            self.functions.emission.orphan = None;
        }
    }

    /// Whether Clang emits `e` with a branch whose paths join in a block
    /// with predecessors (see the module map).
    fn joins_paths(&mut self, e: &'tu Expression<'tu>) -> bool {
        match e.kind {
            | ExpressionType::Binary {
                operator: BinaryOperator::LogicalAnd | BinaryOperator::LogicalOr,
                left_expression,
                ..
            } => self.folded_condition(left_expression).is_none(),
            | ExpressionType::Conditional(conditional) =>
                self.conditional_joins(e, conditional, true),
            | ExpressionType::OmittedConditional(conditional) =>
                self.conditional_joins(e, conditional, false),
            | ExpressionType::Binary {
                operator:
                    operator @ (BinaryOperator::Multiplication
                    | BinaryOperator::MultiplicationAssignment),
                left_expression,
                right_expression,
            } if self.complex_floating(left_expression)
                && self.complex_floating(right_expression) =>
            {
                let _ = operator;
                true
            },
            | ExpressionType::Binary {
                operator,
                left_expression,
                ..
            } => self.atomic_update_loops(left_expression, Some(operator)),
            | ExpressionType::Unary {
                operator:
                    UnaryOperator::PreIncrement
                    | UnaryOperator::PreDecrement
                    | UnaryOperator::PostIncrement
                    | UnaryOperator::PostDecrement,
                operand_expression,
            } => self.atomic_update_loops(operand_expression, None),
            | ExpressionType::Call {
                function_expression,
                ..
            } => self
                .expression_info(function_expression)
                .binding
                .is_some_and(|binding| {
                    matches!(
                        self.bindings[binding].origin,
                        BindingOrigin::Intrinsic(
                            BuiltinId::AtomicC11CompareExchangeStrong
                                | BuiltinId::AtomicC11CompareExchangeWeak
                                | BuiltinId::AtomicGnuCompareExchangeN
                                | BuiltinId::AtomicGnuCompareExchange
                        )
                    )
                }),
            | ExpressionType::Builtin(builtin)
                if builtin.keyword == KeywordTokenType::BuiltinVaArg =>
                !matches!(
                    self.context.configuration.target(),
                    Target::WindowsGnu | Target::WindowsMsvc
                ) && self.register_class(self.expression_info(e).ty),
            | _ => false,
        }
    }

    /// Whether a conditional joins its paths: a scalar or void one does when
    /// its condition does not fold and an operand is not an evaluatable
    /// constant (`omitted` operands never are); a complex or aggregate one
    /// always does.
    fn conditional_joins(
        &mut self,
        e: &'tu Expression<'tu>,
        conditional: &'tu ConditionalExpression<'tu>,
        middle: bool,
    ) -> bool {
        let ty = self.types.non_atomic(self.expression_info(e).ty);
        let aggregate = matches!(
            self.types.nodes[ty.index],
            TypeKind::Tag(_)
                | TypeKind::Scalar(
                    Scalar::ComplexFloat
                        | Scalar::ComplexDouble
                        | Scalar::ComplexLongDouble
                        | Scalar::ComplexFloat128
                )
        );
        if aggregate {
            return true;
        }
        if self
            .folded_condition(conditional.condition_expression)
            .is_some()
        {
            return false;
        }
        !(middle
            && self.evaluatable(conditional.then_expression)
            && self.evaluatable(conditional.else_expression))
    }

    /// Whether `operand` has a floating complex type, so that a product of
    /// two of them checks its result for NaNs (C99 Annex G.5.1 is not
    /// required, but Clang's default complex range is `full`).
    fn complex_floating(&self, operand: &'tu Expression<'tu>) -> bool {
        let ty = self.types.non_atomic(self.expression_info(operand).ty);
        matches!(
            self.types.nodes[ty.index],
            TypeKind::Scalar(
                Scalar::ComplexFloat
                    | Scalar::ComplexDouble
                    | Scalar::ComplexLongDouble
                    | Scalar::ComplexFloat128
            )
        )
    }

    /// Whether an update of atomic `target` loops on `cmpxchg`: a compound
    /// assignment by `operator` (an increment or decrement without one)
    /// that `atomicrmw` cannot perform (`ScalarExprEmitter`).
    fn atomic_update_loops(
        &self,
        target: &'tu Expression<'tu>,
        operator: Option<BinaryOperator>,
    ) -> bool {
        use BinaryOperator as B;
        let ty = self.expression_info(target).ty;
        let TypeKind::Atomic(value) = self.types.nodes[ty.index] else {
            return false;
        };
        let value = &self.types.nodes[value.index];
        let integer = matches!(value, TypeKind::Scalar(scalar) if scalar.integer());
        let boolean = matches!(value, TypeKind::Scalar(Scalar::Bool));
        match operator {
            | None =>
                !(boolean
                    || integer
                    || matches!(
                        value,
                        TypeKind::Scalar(Scalar::Float | Scalar::Double | Scalar::Float128)
                    )),
            | Some(
                B::AdditionAssignment
                | B::SubtractionAssignment
                | B::BitwiseAndAssignment
                | B::BitwiseXorAssignment
                | B::BitwiseOrAssignment,
            ) => boolean || !integer,
            | Some(
                B::MultiplicationAssignment
                | B::DivisionAssignment
                | B::ModuloAssignment
                | B::LeftShiftAssignment
                | B::RightShiftAssignment,
            ) => true,
            | Some(_) => false,
        }
    }

    /// Whether the System V ABI passes a `va_arg` of type `ty` in registers,
    /// which Clang reads with a branch: an integer, pointer, `float` or
    /// `double`, but not `long double`.
    fn register_class(&self, ty: TypeId) -> bool {
        let ty = self.types.non_atomic(ty);
        match self.types.nodes[ty.index] {
            | TypeKind::Pointer(_) => true,
            | TypeKind::Scalar(scalar) =>
                scalar.integer() || matches!(scalar, Scalar::Float | Scalar::Double),
            | _ => false,
        }
    }

    /// Begins expression statement `statement`. A statement expression's
    /// value statement is emitted even when the code before it is skipped,
    /// as `CodeGenFunction::EmitCompoundStmtWithoutScope` ensures an
    /// insertion point for it when the statement expression has a value.
    /// C99: §6.8.3 paragraph 2, p. 132; PDF p. 144.
    pub(in crate::translation_phases::semantic_analysis) fn begin_expression_statement(
        &mut self,
        statement: &'tu Statement<'tu>,
        expression: &'tu Expression<'tu>,
    ) {
        let emission = &mut self.functions.emission;
        emission.serial += 1;
        let serial = emission.serial;
        let value = emission
            .value_statements
            .remove(&std::ptr::from_ref(statement).addr())
            .map(|()| (self.functions.dead, self.reference_count()));
        if let Some((dead, _)) = value {
            self.functions.dead = false;
            if dead {
                self.functions.emission.orphan = Some(serial);
            }
        }
        self.work
            .push(Work::FunctionWork(FunctionWork::ExpressionStatementEnd {
                expression,
                serial,
                value,
            }));
    }

    /// Ends expression statement number `serial`. Clang's `EmitStmt` erases
    /// a block without predecessors that the statement created and skips
    /// what follows. A statement expression's value statement with a value
    /// keeps its block for the enclosing expression; one without a value is
    /// an ordinary statement, skipped after a jump (`value`: the skipped
    /// state and reference count before it).
    pub(in crate::translation_phases::semantic_analysis) fn end_expression_statement(
        &mut self,
        expression: &'tu Expression<'tu>,
        serial: usize,
        value: Option<(bool, usize)>,
    ) {
        let void = matches!(
            self.types.nodes[self.expression_info(expression).ty.index],
            TypeKind::Scalar(Scalar::Void)
        );
        match value {
            | Some(_) if !void => {},
            | Some((true, start)) => {
                self.skip_references(start..self.reference_count());
                self.functions.dead = true;
            },
            | _ =>
                if self
                    .functions
                    .emission
                    .orphan
                    .is_some_and(|since| since >= serial)
                {
                    self.functions.dead = true;
                    self.functions.emission.orphan = None;
                },
        }
    }

    /// Begins statement expression `block`, emitted in the code around it,
    /// and records its value statement: the last item, after its labels.
    pub(in crate::translation_phases::semantic_analysis) fn begin_statement_expression(
        &mut self,
        block: &'tu Statement<'tu>,
    ) {
        self.work
            .push(Work::FunctionWork(FunctionWork::StatementExpressionEnd(
                self.functions.dead,
            )));
        if self.functions.dead {
            return;
        }
        let StatementType::Compound { items } = block.kind else {
            return;
        };
        let Some(&BlockItem::Statement(mut last)) = items.last() else {
            return;
        };
        loop {
            match last.kind {
                | StatementType::Label(_, child) => last = child,
                | StatementType::Attributed(attributed) => last = attributed.statement,
                | _ => break,
            }
        }
        if matches!(
            last.kind,
            StatementType::Expression(ExpressionSlot::Parsed(_))
        ) {
            _ = self
                .functions
                .emission
                .value_statements
                .insert(std::ptr::from_ref(last).addr(), ());
        }
    }
}
