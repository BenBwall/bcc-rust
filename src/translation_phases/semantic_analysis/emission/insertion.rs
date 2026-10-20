//! Where Clang's code generation emits an expression statement's code: a
//! call that does not return leaves a new block without predecessors, the
//! rest of the expression is emitted there, and `CodeGenFunction::EmitStmt`
//! erases such a block when the statement created it, so the statements
//! that follow are skipped until a label. An expression that branches and
//! joins its paths in a block with predecessors ends that state. Clang's
//! expression emitters that do, with default options and no sanitizers:
//! - `&&` and `||` whose left operand does not fold (`land.end`, `lor.end`);
//! - a scalar or `void` `?:` whose condition does not fold, unless both
//!   operands are evaluatable constants, which it selects (`cond.end`); a
//!   `void` operand never is. A complex or aggregate `?:` always
//!   (`ComplexExprEmitter` and `AggExprEmitter` do not fold their condition);
//! - a multiplication of two floating complex operands, which checks the result
//!   for NaNs (`complex_mul_cont`);
//! - an atomic compound assignment, increment or decrement that `atomicrmw`
//!   cannot perform, which loops on `cmpxchg` (`atomic_cont`);
//! - an atomic compare-and-exchange builtin (`cmpxchg.continue`), and an atomic
//!   builtin whose memory order is not a constant, which switches over the
//!   orders (`atomic.continue`);
//! - `va_arg` of a scalar on the System V ABI, which reads registers or memory
//!   (`vaarg.end`).
//!
//! Each rule reads the expression's type, so it runs once the expression is
//! analyzed.
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
    TagKind,
    TypeKind,
    Work,
    functions::FunctionWork,
    scopes::Namespace,
    types::{
        Scalar,
        TypeId,
    },
};
use crate::{
    builtins::{
        AtomicFamily,
        AtomicOperation,
        BuiltinId,
        BuiltinOperation,
    },
    target::Target,
    translation_phases::{
        parsing::{
            ExtendedType,
            declaration_syntax::{
                Declaration,
                Declarator,
                DirectDeclarator,
                TypeSpecifiers,
            },
            syntax::{
                BinaryOperator,
                BlockItem,
                ConditionalExpression,
                Constant,
                Expression,
                ExpressionSlot,
                ExpressionType,
                Statement,
                StatementType,
                UnaryOperator,
            },
        },
        preprocessing::KeywordTokenType,
    },
    util::bump::ArenaVec,
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
                operator: BinaryOperator::Multiplication | BinaryOperator::MultiplicationAssignment,
                left_expression,
                right_expression,
            } if self.complex_floating(left_expression)
                && self.complex_floating(right_expression) =>
                true,
            | ExpressionType::Binary {
                operator:
                    operator @ (BinaryOperator::MultiplicationAssignment
                    | BinaryOperator::DivisionAssignment
                    | BinaryOperator::ModuloAssignment
                    | BinaryOperator::AdditionAssignment
                    | BinaryOperator::SubtractionAssignment
                    | BinaryOperator::LeftShiftAssignment
                    | BinaryOperator::RightShiftAssignment
                    | BinaryOperator::BitwiseAndAssignment
                    | BinaryOperator::BitwiseXorAssignment
                    | BinaryOperator::BitwiseOrAssignment),
                left_expression,
                ..
            } => self.atomic_update_loops(left_expression, Update::Assignment(operator)),
            | ExpressionType::Unary {
                operator:
                    operator @ (UnaryOperator::PreIncrement
                    | UnaryOperator::PostIncrement
                    | UnaryOperator::PreDecrement
                    | UnaryOperator::PostDecrement),
                operand_expression,
            } => self.atomic_update_loops(
                operand_expression,
                if matches!(
                    operator,
                    UnaryOperator::PreIncrement | UnaryOperator::PostIncrement
                ) {
                    Update::Increment
                } else {
                    Update::Decrement
                },
            ),
            | ExpressionType::Call {
                function_expression,
                arguments,
            } => self.atomic_call_branches(function_expression, arguments.as_slice()),
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

    /// Whether an update of atomic `target` loops on `cmpxchg` because
    /// `atomicrmw` cannot perform it (`ScalarExprEmitter`). An increment
    /// of `_Bool` stores `true`, and one of an integer or of a floating
    /// type of power-of-two width adds; a decrement of `_Bool` loops. A
    /// compound assignment of an integer other than `_Bool` by `+`, `-`,
    /// `&`, `^` or `|` does not loop.
    fn atomic_update_loops(&self, target: &'tu Expression<'tu>, update: Update) -> bool {
        use BinaryOperator as B;
        let ty = self.expression_info(target).ty;
        let TypeKind::Atomic(value) = self.types.nodes[ty.index] else {
            return false;
        };
        let TypeKind::Scalar(scalar) = self.types.nodes[value.index] else {
            return true;
        };
        let boolean = scalar == Scalar::Bool;
        let integer = scalar.integer() && !boolean;
        match update {
            | Update::Increment if boolean => false,
            | Update::Increment | Update::Decrement =>
                !(integer
                    || matches!(scalar, Scalar::Float | Scalar::Double | Scalar::Float128)
                    || (scalar == Scalar::LongDouble && self.types.target.long_double_is_double())),
            | Update::Assignment(
                B::AdditionAssignment
                | B::SubtractionAssignment
                | B::BitwiseAndAssignment
                | B::BitwiseXorAssignment
                | B::BitwiseOrAssignment,
            ) => !integer,
            | Update::Assignment(
                B::MultiplicationAssignment
                | B::DivisionAssignment
                | B::ModuloAssignment
                | B::LeftShiftAssignment
                | B::RightShiftAssignment,
            ) => true,
            | Update::Assignment(_) => false,
        }
    }

    /// Whether a call of `function` with `arguments` is an atomic builtin
    /// whose code generation branches and joins: a compare-and-exchange
    /// (`cmpxchg.continue`), or an operation whose memory order is not a
    /// constant, which switches over the orders (`atomic.continue`), unless
    /// its object is copied by a library call: one whose size is not a
    /// power of two, or exceeds 16 bytes (`CodeGenFunction::EmitAtomicExpr`).
    /// C11: §7.17.3 paragraph 1, p. 275; PDF p. 293.
    fn atomic_call_branches(
        &mut self,
        function: &'tu Expression<'tu>,
        arguments: &[&'tu Expression<'tu>],
    ) -> bool {
        use AtomicOperation as O;
        let Some(binding) = self.expression_info(function).binding else {
            return false;
        };
        let origin = self.bindings[binding].origin;
        if matches!(
            origin,
            BindingOrigin::Intrinsic(
                BuiltinId::AtomicC11CompareExchangeStrong
                    | BuiltinId::AtomicC11CompareExchangeWeak
                    | BuiltinId::AtomicGnuCompareExchangeN
                    | BuiltinId::AtomicGnuCompareExchange
            )
        ) {
            return true;
        }
        let Some(BuiltinOperation::Atomic(atomic)) =
            origin.descriptor().map(|descriptor| descriptor.operation)
        else {
            return false;
        };
        if atomic.family == AtomicFamily::Sync
            || matches!(
                atomic.operation,
                O::Init | O::AlwaysLockFree | O::IsLockFree
            )
            || atomic.count == 0
            || arguments.len() != atomic.count
        {
            return false;
        }
        if self.evaluates_to_number(arguments[atomic.count - 1]) {
            return false;
        }
        if matches!(
            atomic.operation,
            O::ThreadFence | O::SignalFence | O::TestSet | O::Clear
        ) {
            return true;
        }
        let pointer = self.expression_info(arguments[0]).ty;
        let pointer = self.types.non_atomic(pointer);
        let TypeKind::Pointer(object) = self.types.nodes[pointer.index] else {
            return false;
        };
        self.types
            .layout(object)
            .is_some_and(|layout| layout.size.is_power_of_two() && layout.size <= 16)
    }

    /// Whether the System V ABI passes a `va_arg` of type `ty` in registers,
    /// which Clang reads with a branch (`X86_64ABIInfo::EmitVAArg`): an
    /// integer, pointer, `float`, `double`, `__float128` or vector, a complex
    /// `float` or `double`, or a structure or union of at most 16 bytes, but
    /// nothing with an x87 class (`long double`), a misaligned member or a
    /// flexible array member, nor an empty one, which are read from memory.
    fn register_class(&self, ty: TypeId) -> bool {
        let Some(layout) = self.types.layout(ty) else {
            return false;
        };
        if !(1..=16).contains(&layout.size) {
            return false;
        }
        // Each object within `ty`, with its offset.
        let mut work = ArenaVec::new_in(self.scratch);
        work.push((ty, 0));
        while let Some((ty, offset)) = work.pop() {
            let ty = self.types.non_atomic(ty);
            let aligned = self
                .types
                .layout(ty)
                .is_some_and(|layout| layout.align == 0 || offset % layout.align == 0);
            match self.types.nodes[ty.index] {
                | TypeKind::Scalar(
                    Scalar::LongDouble | Scalar::ComplexLongDouble | Scalar::Void,
                ) => return false,
                | TypeKind::Scalar(_) | TypeKind::Pointer(_) | TypeKind::Vector { .. } =>
                    if !aligned {
                        return false;
                    },
                | TypeKind::Array(element, _) => work.push((element, offset)),
                | TypeKind::Tag(id) => {
                    let tag = &self.types.tags[id];
                    if tag.contains_flexible.get() || !aligned {
                        return false;
                    }
                    if tag.kind != TagKind::Enum {
                        for member in tag.members.get() {
                            // A bit-field's own alignment is not checked.
                            if member.width.is_none() {
                                work.push((member.ty, offset + member.offset));
                            }
                        }
                    }
                },
                | _ => return false,
            }
        }
        true
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
                    self.functions.emission.orphan = None;
                    if !self.variably_modified_ahead() {
                        self.functions.dead = true;
                    }
                },
        }
    }

    /// Whether a declaration that may have a variably modified type is
    /// still to be analyzed in the blocks around the code: an item of
    /// theirs, or one inside a block, label or attribute such an item
    /// holds. Clang's `EmitVariablyModifiedType` ensures an insertion point
    /// at a block-scope object or typedef of such a type, which resumes the
    /// code after it, but whether Clang reaches it at all depends on folds
    /// this model does not make. So a block a call that does not return
    /// left is not erased then: the code after the call stays emitted, as
    /// it is without the model of such calls. A bound that is not an
    /// integer literal, a `typeof` specifier and a typedef name of a
    /// variably modified or unknown type count.
    /// C99: §6.7.5.2 paragraphs 2 and 4, p. 116; PDF p. 128; §6.8
    /// paragraph 3, p. 131; PDF p. 143.
    fn variably_modified_ahead(&self) -> bool {
        let mut items = ArenaVec::new_in(self.scratch);
        for work in self.work.iter() {
            if let Work::BlockItem(item) = *work {
                items.push(item);
            }
        }
        while let Some(item) = items.pop() {
            let mut statement = match item {
                | BlockItem::Declaration(declaration) => {
                    if self.declaration_may_be_variably_modified(declaration) {
                        return true;
                    }
                    continue;
                },
                | BlockItem::Statement(statement) => statement,
                | BlockItem::FunctionDefinition(_) => continue,
            };
            loop {
                match statement.kind {
                    | StatementType::Label(_, child)
                    | StatementType::Case(_, child)
                    | StatementType::Default(child) => statement = child,
                    | StatementType::CaseRange(range) => statement = range.statement,
                    | StatementType::Attributed(attributed) => statement = attributed.statement,
                    | StatementType::Compound { items: inner } => {
                        for &inner in inner {
                            items.push(inner);
                        }
                        break;
                    },
                    | StatementType::Declaration(declaration) => {
                        if self.declaration_may_be_variably_modified(declaration) {
                            return true;
                        }
                        break;
                    },
                    | _ => break,
                }
            }
        }
        false
    }

    /// Whether `declaration` may declare something of a variably modified
    /// type, judged from its syntax and the typedefs visible now.
    fn declaration_may_be_variably_modified(&self, declaration: &'tu Declaration<'tu>) -> bool {
        let specifiers = match declaration.declaration_specifiers.type_specifiers {
            | TypeSpecifiers::Extended(ExtendedType::Typeof { .. }) => true,
            | TypeSpecifiers::TypedefName(name) => self
                .lookup(Namespace::Ordinary, name.name)
                .is_none_or(|entry| {
                    self.types
                        .variably_modified(self.bindings[entry.binding].ty)
                }),
            | _ => false,
        };
        specifiers
            || declaration
                .init_declarators
                .iter()
                .any(|init| self.declarator_has_variable_bound(init.declarator))
    }

    /// Whether `declarator` has an array bound that is not an integer
    /// literal, outside function parameters.
    fn declarator_has_variable_bound(&self, declarator: Declarator<'tu>) -> bool {
        let mut declarators = ArenaVec::new_in(self.scratch);
        declarators.push(declarator);
        while let Some(declarator) = declarators.pop() {
            for direct in declarator.kind {
                match *direct {
                    | DirectDeclarator::Array {
                        assignment_expression: Some(bound),
                        ..
                    } => {
                        let mut bound = bound;
                        while let ExpressionType::Parenthesized { expression } = bound.kind {
                            bound = expression;
                        }
                        if !matches!(bound.kind, ExpressionType::Constant(Constant::Integer(_))) {
                            return true;
                        }
                    },
                    | DirectDeclarator::Parenthesized(inner) => declarators.push(inner.declarator),
                    | _ => {},
                }
            }
        }
        false
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

/// An update of an atomic object.
#[derive(Clone, Copy)]
enum Update {
    Increment,
    Decrement,
    /// A compound assignment by this operator.
    Assignment(BinaryOperator),
}
