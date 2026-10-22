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
//! Only an evaluated, emitted expression leaves a block without
//! predecessors: neither one in a `sizeof`, `alignof` or `typeof` operand
//! nor one in an operand a folded condition skips. A join in an operand
//! that code generation may emit still counts, so the model never skips
//! code for a join it cannot see: a variably modified `sizeof` or `typeof`
//! operand, and an operand of `__builtin_constant_p` or the object-size
//! builtins without side effects, are emitted. A statement expression's
//! value statement is emitted even after a jump when the expression has a
//! value, and keeps its block. A statement expression whose body ends
//! skipped leaves a block without predecessors, but only when it is
//! emitted itself, not in such an operand.
//!
//! Code generation emits some operands in another order than this
//! analysis: the right operand of a scalar or complex assignment or of a
//! compound assignment first, and on the Microsoft ABI a call's arguments
//! from the last. When a call that does not return leaves the code after
//! such an expression in a block without predecessors, an operand emitted
//! after it that may join paths ends that state.
//! C99: §6.8.3 paragraph 2, p. 132; PDF p. 144; §6.5.13-§6.5.15,
//! pp. 89-91; PDF pp. 101-103; §6.5.5 paragraph 3, p. 82; PDF p. 94.
//! C11: §6.5.16.2 paragraph 3, pp. 103-104; PDF pp. 121-122 (atomic
//! compound assignment); §7.17.7.4 paragraphs 1-5, pp. 283-284;
//! PDF pp. 301-302.
//! GNU extension: GCC manual, "Statement Exprs".

use super::super::{
    Analyzer,
    ArrayBound,
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
                StorageClass,
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
    /// one with predecessors. In a `sizeof`, `alignof` or `typeof` operand,
    /// only a join counts (see the module map). The operands of a call of
    /// `__builtin_constant_p`, `__builtin_object_size` or
    /// `__builtin_dynamic_object_size` are not emitted when they have side
    /// effects, as a call that does not return has, and those of
    /// `__builtin_classify_type` never are, so they leave the blocks as they
    /// were when the call's callee, analyzed before them, ended, unless they
    /// may have joined paths.
    /// C99: §6.5.2.2 paragraph 10, p. 72; PDF p. 84.
    /// GNU extension: GCC manual, "Other Builtins" and "Object Size
    /// Checking".
    pub(in crate::translation_phases::semantic_analysis) fn expression_emitted(
        &mut self,
        e: &'tu Expression<'tu>,
    ) {
        self.finish_conditional_arms(e);
        if self.functions.dead {
            return;
        }
        if self.functions.sizeof.is_some() || self.functions.reference_context.is_some() {
            // Whether the operand is evaluated is known once it is analyzed
            // ([`Self::end_expression_statement`]).
            if self.joins_paths(e) {
                self.functions.emission.context_joins.push((
                    self.functions.emission.serial,
                    self.functions.sizeof,
                    self.functions.reference_context,
                ));
            }
            return;
        }
        if self.functions.emission.orphan.is_some() && self.emitted_later_may_join(e) {
            self.functions.emission.orphan = None;
        }
        if let ExpressionType::Call {
            function_expression,
            ..
        } = e.kind
        {
            let callee = unparenthesized(function_expression);
            let key = std::ptr::from_ref(callee).addr();
            if let Some(before) = self.functions.emission.unevaluated_calls.remove(&key) {
                let classify = self.expression_info(callee).binding.is_some_and(|binding| {
                    self.bindings[binding].origin
                        == BindingOrigin::Intrinsic(BuiltinId::ClassifyType)
                });
                let after = self.functions.emission.orphan;
                self.functions.emission.orphan = if classify { before } else { after.and(before) };
            } else if self.call_does_not_return(function_expression) {
                self.functions.emission.orphan = Some(self.functions.emission.serial);
            } else if self.joins_paths(e) {
                self.functions.emission.orphan = None;
            }
        } else if matches!(e.kind, ExpressionType::Identifier(_))
            && self.expression_info(e).binding.is_some_and(|binding| {
                matches!(
                    self.bindings[binding].origin,
                    BindingOrigin::Intrinsic(
                        BuiltinId::ConstantP
                            | BuiltinId::ObjectSize
                            | BuiltinId::DynamicObjectSize
                            | BuiltinId::ClassifyType
                    )
                )
            })
        {
            _ = self
                .functions
                .emission
                .unevaluated_calls
                .insert(std::ptr::from_ref(e).addr(), self.functions.emission.orphan);
        } else if self.joins_paths(e) {
            self.functions.emission.orphan = None;
        }
    }

    /// Record an arm boundary and start its neighbor in the condition's
    /// insertion state, so one arm's jumps do not affect the other.
    pub(in crate::translation_phases::semantic_analysis) fn mark_conditional_arm(
        &mut self,
        expression: &'tu Expression<'tu>,
        index: usize,
        outer: super::super::functions::Skipped,
        outer_elided: bool,
    ) {
        let mark = (
            self.reference_count(),
            self.functions.emission.orphan,
            self.functions.emission.context_joins.len(),
        );
        let marks = self
            .functions
            .emission
            .conditional_marks
            .entry(std::ptr::from_ref(expression).addr())
            .or_insert([(0, None, 0); 3]);
        marks[index] = mark;
        if index == 1 {
            self.functions.emission.orphan = marks[0].1;
        }
        self.functions.dead = outer.dead;
        self.functions.unvisited = outer.unvisited;
        self.functions.elided = outer_elided;
    }

    /// Scalar conditional emission skips an unlabeled unselected arm;
    /// complex and aggregate emission generates both. Resolve this after
    /// typing, preserving each arm's own skipped references and insertion
    /// state rather than undoing skips made while analyzing its statements.
    /// C99: §6.5.15 paragraphs 3-6, pp. 90-91; PDF pp. 102-103.
    fn finish_conditional_arms(&mut self, expression: &'tu Expression<'tu>) {
        let Some(marks) = self
            .functions
            .emission
            .conditional_marks
            .remove(&std::ptr::from_ref(expression).addr())
        else {
            return;
        };
        let (ExpressionType::Conditional(conditional)
        | ExpressionType::OmittedConditional(conditional)) = expression.kind
        else {
            return;
        };
        let info = self.expression_info(conditional.condition_expression);
        let constant_context = self
            .functions
            .initializer
            .is_some_and(|binding| self.bindings[binding].duration.is_static_or_thread());
        let value = if info.runtime_query && constant_context {
            Self::constant_truth(info)
        } else {
            self.folded_condition(conditional.condition_expression)
        };
        if !self
            .functions
            .emission
            .branches
            .contains_key(&std::ptr::from_ref(expression).addr())
            && self.folded_conditional_emission(expression)
            && let Some(taken) = value
        {
            let skipped = if taken {
                conditional.else_expression
            } else {
                conditional.then_expression
            };
            if !self.expression_contains_label(skipped) {
                let index = usize::from(taken);
                self.skip_references(marks[index].0..marks[index + 1].0);
                drop(
                    self.functions
                        .emission
                        .context_joins
                        .drain(marks[index].2..marks[index + 1].2),
                );
            }
            self.functions.emission.orphan = marks[if taken { 1 } else { 2 }].1;
        }
    }

    /// Only scalar and void conditional emitters fold their condition.
    fn conditional_has_scalar_emitter(&self, expression: &'tu Expression<'tu>) -> bool {
        let ty = self.types.non_atomic(self.expression_info(expression).ty);
        !matches!(self.types.nodes[ty.index],
            TypeKind::Tag(tag) if self.types.tags[tag].kind != TagKind::Enum
        ) && !matches!(
            self.types.nodes[ty.index],
            TypeKind::Scalar(
                Scalar::ComplexFloat
                    | Scalar::ComplexDouble
                    | Scalar::ComplexLongDouble
                    | Scalar::ComplexFloat128
            )
        )
    }

    /// Whether an operand of `e` that this analysis visits first, but code
    /// generation emits last, may join paths: the left operand of a scalar
    /// or complex assignment or of a compound assignment, which
    /// `ScalarExprEmitter` and `ComplexExprEmitter` emit after the right,
    /// or on the Microsoft ABI the arguments of a call before its last,
    /// which `EmitCallArgs` emits from the last. A structure assignment
    /// emits its left operand first.
    /// C99: §6.5.16 paragraph 4, p. 91; PDF p. 103; §6.5.2.2 paragraph 10,
    /// p. 72; PDF p. 84.
    fn emitted_later_may_join(&mut self, e: &'tu Expression<'tu>) -> bool {
        let mut work = ArenaVec::new_in(self.scratch);
        match e.kind {
            | ExpressionType::Binary {
                operator: BinaryOperator::Assignment,
                left_expression,
                ..
            } if !matches!(
                self.types.nodes[self.types.non_atomic(self.expression_info(e).ty).index],
                TypeKind::Tag(_)
            ) =>
                work.push(left_expression),
            | ExpressionType::Binary {
                operator:
                    BinaryOperator::MultiplicationAssignment
                    | BinaryOperator::DivisionAssignment
                    | BinaryOperator::ModuloAssignment
                    | BinaryOperator::AdditionAssignment
                    | BinaryOperator::SubtractionAssignment
                    | BinaryOperator::LeftShiftAssignment
                    | BinaryOperator::RightShiftAssignment
                    | BinaryOperator::BitwiseAndAssignment
                    | BinaryOperator::BitwiseXorAssignment
                    | BinaryOperator::BitwiseOrAssignment,
                left_expression,
                ..
            } => work.push(left_expression),
            | ExpressionType::Call { arguments, .. }
                if self.context.configuration.target() == Target::WindowsMsvc =>
                if let Some((_, before)) = arguments.as_slice().split_last() {
                    work.extend_from_slice(before);
                },
            | _ => return false,
        }
        work.into_iter().any(|operand| self.may_join(operand))
    }

    /// Whether `e` or an expression in it may join paths: one that
    /// [`Self::joins_paths`], or a statement expression, which may in its
    /// statements. Memoized for each expression it holds, in postorder, so
    /// that nested operands are examined once.
    fn may_join(&mut self, e: &'tu Expression<'tu>) -> bool {
        let mut work = ArenaVec::new_in(self.scratch);
        work.push((e, false));
        while let Some((current, children_done)) = work.pop() {
            let key = std::ptr::from_ref(current).addr();
            if self.functions.emission.may_join.contains_key(&key) {
                continue;
            }
            let statement = matches!(current.kind, ExpressionType::StatementExpression(_));
            if statement || children_done {
                let mut joins = statement || self.joins_paths(current);
                if !statement {
                    super::labels::expression_children(self.scratch, current, |child| {
                        joins |= self
                            .functions
                            .emission
                            .may_join
                            .get(&std::ptr::from_ref(child).addr())
                            .copied()
                            .unwrap_or(true);
                    });
                }
                _ = self.functions.emission.may_join.insert(key, joins);
            } else {
                work.push((current, true));
                super::labels::expression_children(self.scratch, current, |child| {
                    work.push((child, false));
                });
            }
        }
        self.functions
            .emission
            .may_join
            .get(&std::ptr::from_ref(e).addr())
            .copied()
            .unwrap_or(true)
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
            } =>
                self.atomic_call_branches(function_expression, arguments.as_slice())
                    || self.fpclassify_branches(e, function_expression),
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
        if !self.conditional_has_scalar_emitter(e) {
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

    /// Whether a call of `function` is `__builtin_fpclassify`, whose code
    /// generation branches over the classes and joins in `fpclassify_end`,
    /// unless the call evaluates to a constant, which Clang emits instead.
    /// C99: §7.12.3.1 paragraph 3, p. 216; PDF p. 228.
    fn fpclassify_branches(
        &mut self,
        e: &'tu Expression<'tu>,
        function: &'tu Expression<'tu>,
    ) -> bool {
        self.expression_info(function)
            .binding
            .is_some_and(|binding| {
                self.bindings[binding].origin == BindingOrigin::Intrinsic(BuiltinId::Fpclassify)
            })
            && !self.evaluatable(e)
    }

    /// Whether the System V ABI passes a `va_arg` of type `ty` in registers,
    /// which Clang reads with a branch (`X86_64ABIInfo::EmitVAArg`): an
    /// integer, pointer, `float`, `double`, `__float128` or vector, a complex
    /// `float` or `double`, or a structure or union of at most 16 bytes, but
    /// nothing with an x87 class (`long double`), a misaligned member or a
    /// flexible array member, which are read from memory, nor one with no
    /// class at all, which is ignored. An array of no elements and an
    /// unnamed bit-field have no class; a named bit-field is an integer.
    fn register_class(&self, ty: TypeId) -> bool {
        let Some(layout) = self.types.layout(ty) else {
            return false;
        };
        if !(1..=16).contains(&layout.size) {
            return false;
        }
        let mut classified = false;
        // Each object within `ty`, with its offset.
        let mut work = ArenaVec::new_in(self.scratch);
        work.push((ty, 0));
        while let Some((ty, offset)) = work.pop() {
            let ty = self.types.non_atomic(ty);
            // The ABI classifies a member by its type's natural alignment,
            // whatever a typedef's `aligned` makes of its storage.
            let aligned = self
                .types
                .natural_alignment(ty)
                .is_some_and(|align| align == 0 || offset % align == 0);
            match self.types.nodes[ty.index] {
                | TypeKind::Scalar(
                    Scalar::LongDouble | Scalar::ComplexLongDouble | Scalar::Void,
                ) => return false,
                | TypeKind::Scalar(_) | TypeKind::Pointer(_) | TypeKind::Vector { .. } => {
                    if !aligned {
                        return false;
                    }
                    classified = true;
                },
                | TypeKind::Array(element, bound) => {
                    if !aligned {
                        return false;
                    }
                    if bound != ArrayBound::Constant(0) {
                        work.push((element, offset));
                    }
                },
                | TypeKind::Tag(id) => {
                    let tag = &self.types.tags[id];
                    if tag.contains_flexible.get() || !aligned {
                        return false;
                    }
                    if tag.kind == TagKind::Enum {
                        classified = true;
                        continue;
                    }
                    for member in tag.members.get() {
                        // A bit-field's own alignment is not checked.
                        if member.width.is_none() {
                            work.push((member.ty, offset + member.offset));
                        } else if member.name.is_some() {
                            classified = true;
                        }
                    }
                },
                | _ => return false,
            }
        }
        classified
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
        emission.open_statements += 1;
        let serial = emission.serial;
        let value = emission
            .value_statements
            .remove(&std::ptr::from_ref(statement).addr())
            .map(|()| {
                let mut position = self.emission_position();
                if let Some(id) = self.control.statement_id(statement) {
                    position.statements = id.0;
                }
                (self.functions.dead, position)
            });
        if let Some((dead, _)) = value {
            self.functions.dead = false;
            if dead {
                self.functions.emission.orphan = Some(serial);
            }
        }
        // Code generation skips the statement whole, without visiting it.
        if self.functions.dead && !self.functions.unvisited {
            self.work
                .push(Work::FunctionWork(FunctionWork::RestoreUnvisited(false)));
            self.functions.unvisited = true;
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
        value: Option<(bool, super::Position)>,
    ) {
        let void = matches!(
            self.types.nodes[self.expression_info(expression).ty.index],
            TypeKind::Scalar(Scalar::Void)
        );
        match value {
            | Some(_) if !void => {},
            | Some((true, start)) => {
                self.elide_references(start.references..self.reference_count());
                for definition in &mut self.definitions[start.definitions..] {
                    if definition.kind == super::super::DefinitionKind::Object {
                        definition.elided = true;
                    }
                }
                for statement in &mut self.control.statements[start.statements..] {
                    statement.elided = true;
                }
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
                    // A join in an operand that turned out evaluated, a
                    // variably modified `sizeof` or `typeof` operand, may
                    // have followed the call.
                    let joined = self.functions.emission.context_joins.iter().any(
                        |&(at, sizeof, context)| {
                            at >= serial && self.context_evaluated(sizeof, context)
                        },
                    );
                    if !joined && !self.variably_modified_ahead() {
                        self.functions.dead = true;
                    }
                },
        }
        let emission = &mut self.functions.emission;
        emission.open_statements -= 1;
        if emission.open_statements == 0 {
            emission.context_joins.clear();
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
    /// C99: §6.7.5.2 paragraphs 2 and 4, pp. 116-117; PDF pp. 128-129;
    /// §6.8 paragraph 3, p. 131; PDF p. 143.
    ///
    /// The work stack holds the items still to be analyzed. The answer for
    /// each is kept with its position: the work below an item stays as it
    /// is until the item is popped, so only the work above the lowest the
    /// stack has been since the last question is examined
    /// (`State::work_low`). Each block item's own answer is kept by
    /// address, judged so that typedefs coming into scope later cannot
    /// change it ([`Self::declaration_may_be_variably_modified`]), so the
    /// items of nested blocks are examined once, and a run of calls that do
    /// not return costs time linear in the function.
    fn variably_modified_ahead(&mut self) -> bool {
        // The work below the lowest the stack has been since the last
        // question is as it was then, and was examined then or before.
        let start = self.functions.emission.work_low.min(self.work.len());
        let cache = &mut self.functions.emission.variably_modified;
        while cache
            .last()
            .is_some_and(|&(position, ..)| position >= start)
        {
            _ = cache.pop();
        }
        let mut ahead = cache.last().is_some_and(|&(position, key, ahead)| {
            debug_assert!(
                matches!(self.work.get(position), Some(&Work::BlockItem(item)) if item_key(item) == key),
                "work below the low-water mark is unchanged"
            );
            ahead
        });
        self.functions.emission.work_low = self.work.len();
        for position in start..self.work.len() {
            if let Work::BlockItem(item) = self.work[position] {
                ahead = ahead || self.item_may_be_variably_modified(item);
                self.functions
                    .emission
                    .variably_modified
                    .push((position, item_key(item), ahead));
            }
        }
        ahead
    }

    /// Whether block item `item` may declare something of a variably
    /// modified type: see [`Self::variably_modified_ahead`]. Memoized for
    /// each item it holds, in postorder.
    fn item_may_be_variably_modified(&mut self, item: BlockItem<'tu>) -> bool {
        // Each item, with whether the items of the block it holds are
        // judged.
        let mut items = ArenaVec::new_in(self.scratch);
        items.push((item, false));
        while let Some((item, inner_judged)) = items.pop() {
            let key = item_key(item);
            if self
                .functions
                .emission
                .variably_modified_items
                .contains_key(&key)
            {
                continue;
            }
            let (declaration, block) = item_parts(item);
            if inner_judged {
                let mut ahead =
                    declaration.is_some_and(|d| self.declaration_may_be_variably_modified(d));
                if let Some(StatementType::Compound { items: inner }) = block.map(|b| b.kind) {
                    for &inner in inner {
                        ahead = ahead
                            || self
                                .functions
                                .emission
                                .variably_modified_items
                                .get(&item_key(inner))
                                .copied()
                                .unwrap_or(true);
                    }
                }
                _ = self
                    .functions
                    .emission
                    .variably_modified_items
                    .insert(key, ahead);
            } else {
                items.push((item, true));
                if let Some(StatementType::Compound { items: inner }) = block.map(|b| b.kind) {
                    for &inner in inner {
                        items.push((inner, false));
                    }
                }
            }
        }
        self.functions
            .emission
            .variably_modified_items
            .get(&item_key(item))
            .copied()
            .unwrap_or(true)
    }

    /// Whether `declaration` may declare something of a variably modified
    /// type, judged from its syntax and the typedefs visible now. A typedef
    /// name that a block-scope typedef of the function may give a variably
    /// modified type counts wherever it is, since the answer is kept while
    /// typedefs that shadow it come into scope and leave it.
    fn declaration_may_be_variably_modified(&mut self, declaration: &'tu Declaration<'tu>) -> bool {
        let specifiers = match declaration.declaration_specifiers.type_specifiers {
            | TypeSpecifiers::Extended(ExtendedType::Typeof { .. }) => true,
            | TypeSpecifiers::TypedefName(name) =>
                self.block_typedef_may_be_variably_modified(name.name)
                    || self
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

    /// Whether a block-scope typedef of the current function declares
    /// `name` with a type that may be variably modified: a `typeof`
    /// specifier, a typedef name or a bound that is not an integer literal.
    /// The function's typedefs are read once, the first time it asks.
    /// C99: §6.7.7 paragraphs 2-3, p. 123; PDF p. 135.
    fn block_typedef_may_be_variably_modified(
        &mut self,
        name: crate::util::string_cache::StringCacheId,
    ) -> bool {
        let Some(function) = self.functions.current else {
            return false;
        };
        let key = std::ptr::from_ref(function.syntax).addr();
        if self
            .functions
            .emission
            .vm_typedef_functions
            .insert(key, ())
            .is_none()
        {
            let mut typedefs = ArenaVec::new_in(self.scratch);
            super::labels::block_declarations(self.scratch, function.syntax.body, |declaration| {
                if declaration.declaration_specifiers.storage_class == Some(StorageClass::Typedef) {
                    typedefs.push(declaration);
                }
            });
            for declaration in typedefs {
                let specifiers = matches!(
                    declaration.declaration_specifiers.type_specifiers,
                    TypeSpecifiers::Extended(ExtendedType::Typeof { .. })
                        | TypeSpecifiers::TypedefName(_)
                );
                for init in declaration.init_declarators {
                    if (specifiers || self.declarator_has_variable_bound(init.declarator))
                        && let Some(declared) = init.declarator.identifier()
                    {
                        _ = self
                            .functions
                            .emission
                            .vm_typedefs
                            .insert((key, declared.name), ());
                    }
                }
            }
        }
        self.functions
            .emission
            .vm_typedefs
            .contains_key(&(key, name))
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

fn unparenthesized<'tu>(mut expression: &'tu Expression<'tu>) -> &'tu Expression<'tu> {
    while let ExpressionType::Parenthesized { expression: inner } = expression.kind {
        expression = inner;
    }
    expression
}

/// The declaration block item `item` makes, or the block it holds, through
/// labels and attributes.
fn item_parts(item: BlockItem<'_>) -> (Option<&Declaration<'_>>, Option<&Statement<'_>>) {
    let mut statement = match item {
        | BlockItem::Declaration(declaration) => return (Some(declaration), None),
        | BlockItem::Statement(statement) => statement,
        | BlockItem::FunctionDefinition(_) => return (None, None),
    };
    loop {
        match statement.kind {
            | StatementType::Label(_, child)
            | StatementType::Case(_, child)
            | StatementType::Default(child) => statement = child,
            | StatementType::CaseRange(range) => statement = range.statement,
            | StatementType::Attributed(attributed) => statement = attributed.statement,
            | StatementType::Compound { .. } => return (None, Some(statement)),
            | StatementType::Declaration(declaration) => return (Some(declaration), None),
            | _ => return (None, None),
        }
    }
}

/// The address of block item `item`'s syntax, which identifies it.
fn item_key(item: BlockItem<'_>) -> usize {
    match item {
        | BlockItem::Declaration(declaration) => std::ptr::from_ref(declaration).addr(),
        | BlockItem::Statement(statement) => std::ptr::from_ref(statement).addr(),
        | BlockItem::FunctionDefinition(definition) => std::ptr::from_ref(definition).addr(),
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
