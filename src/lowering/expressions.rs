//! Expressions: operators, calls, conditions and the conversions semantic
//! analysis recorded.
//!
//! [`FunctionLowerer::evaluate_raw`] schedules an expression's operands and
//! a [`Task::Finish`] that completes its operator; [`Task::Evaluate`] adds
//! the conversions the parent recorded on the node, which
//! [`FunctionLowerer::convert_recorded`] applies in order: an lvalue
//! conversion reads the place, a decay takes its address, and an
//! arithmetic, assignment or default-argument conversion changes the
//! value's type. An integer constant expression is folded to its value, and a
//! floating constant to its bits.
//! `&&`, `||` and `?:` become blocks with a merge parameter; a condition
//! branches directly on comparisons and logical operators.
//!
//! C99: §6.3, pp. 42-48; PDF pp. 54-60; §6.5, pp. 67-94; PDF pp. 79-106.

use super::{
    Construct,
    FunctionLowerer,
    LoweringError,
    floats,
    module_items::at,
    ssa::{
        Op,
        Terminator,
    },
    types::{
        self,
        Repr,
        repr,
    },
    work::{
        BitFieldBits,
        Item,
        Operand,
        Place,
        Task,
    },
};
use crate::{
    ir::{
        Align,
        Block,
        FloatCC,
        InstData,
        InstFlags,
        IntCC,
        Opcode,
        ParamAbi,
        Type,
        Value,
    },
    translation_phases::{
        SourceVectors,
        parsing::syntax::{
            BinaryOperator,
            ConditionalExpression,
            Expression,
            ExpressionType,
            UnaryOperator,
        },
        semantic_analysis::{
            ArgumentFacts,
            ArrayBound,
            CallFacts,
            Callee,
            ConversionKind,
            ExpressionInfo,
            SubobjectArgument,
            TypeId,
            TypeKind,
        },
    },
    util::bump::ArenaVec,
};

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    /// Schedules `expression`'s operator after its operands, leaving its
    /// unconverted result: a place for an lvalue.
    pub(super) fn evaluate_raw(
        &mut self,
        expression: &'tu Expression<'tu>,
    ) -> Result<(), LoweringError> {
        use ExpressionType as E;
        let source = expression.source_vectors;
        if let Some(construct) = unsupported(expression) {
            return Err(LoweringError::unsupported(construct, source));
        }
        let info = self.info(expression)?;
        debug_assert!(
            !info.elided,
            "lowering reached an operand semantic analysis elided"
        );
        if self.unit.sema.types.unanalyzed(info.ty) {
            return Err(LoweringError::unanalyzed(
                &self.unit.sema.types,
                info.ty,
                source,
            ));
        }
        if let Some(operand) = self.unit.sema.branch_operand(expression) {
            let merge = self.draft.create_block(&[Type::I1]);
            self.tasks.push(Task::Merge {
                merge,
                ty: info.ty,
                truth: true,
            });
            self.tasks.push(Task::JumpWithValue { merge, truth: true });
            self.tasks.push(Task::Evaluate(operand));
            return Ok(());
        }
        match self.folded(expression)? {
            | Some(Folded::Live(operand)) => {
                self.tasks.push(Task::Evaluate(operand));
                return Ok(());
            },
            | Some(Folded::Truth(truth)) => {
                let value = self.iconst(Type::I32, i128::from(truth));
                self.push_item(Operand::Value(value), info.ty);
                return Ok(());
            },
            | None => {},
        }
        // An integer constant expression has an exact value (§6.6p6), unless
        // function code computes a constant query differently.
        if info.ice
            && !info.runtime_query
            && let Some(value) = info.integer
            && let Ok(representation @ (Repr::Int { .. } | Repr::Bool)) =
                repr(&self.unit.sema.types, info.ty)
        {
            let ty = representation.value_type().expect("integers have values");
            let value = if representation == Repr::Bool {
                i128::from(value.value != 0)
            } else {
                value.value
            };
            let value = self.iconst(ty, value);
            self.push_item(Operand::Value(value), info.ty);
            return Ok(());
        }
        // A folded binary128 constant has exact bits too.
        if let Some(folded) = info.binary128().filter(|_| !info.runtime_query) {
            match repr(&self.unit.sema.types, info.ty) {
                | Ok(Repr::Complex { component }) => {
                    let parts = [folded.real, folded.imag].map(|part| {
                        floats::binary128_bits(part, component)
                            .ok_or_else(|| LoweringError::missing("complex constant bits", source))
                    });
                    let [real, imag] = parts;
                    let values = [self.fconst(component, real?), self.fconst(component, imag?)];
                    let operand = self.complex_pair(values);
                    self.push_item(operand, info.ty);
                    return Ok(());
                },
                | Ok(Repr::Float { ty }) => {
                    let bits = floats::binary128_bits(folded.real, ty).ok_or_else(|| {
                        LoweringError::missing("a floating constant has no bits", source)
                    })?;
                    let value = self.fconst(ty, bits);
                    self.push_item(Operand::Value(value), info.ty);
                    return Ok(());
                },
                | _ => {},
            }
        }
        if let Some(folded) = info.floating().filter(|_| !info.runtime_query)
            && let Ok(Repr::Complex { component }) = repr(&self.unit.sema.types, info.ty)
        {
            // A folded `_Complex _Float16` keeps its excess, in `f32` parts.
            let component = if self.complex_half(info.ty) {
                Type::F32
            } else {
                component
            };
            let real = floats::constant_bits(&folded.real, component)
                .ok_or_else(|| LoweringError::missing("complex constant bits", source))?;
            let imag = floats::constant_bits(&folded.imag, component)
                .ok_or_else(|| LoweringError::missing("complex constant bits", source))?;
            let values = [self.fconst(component, real), self.fconst(component, imag)];
            let operand = self.complex_pair(values);
            self.push_item(operand, info.ty);
            return Ok(());
        }
        // A folded `_Float16` keeps the excess its consumer may read.
        // GCC keeps a plain one (a cast's, say) in its own format, since it
        // rounds such a constant where it rounds no run-time value.
        if let Some(folded) = info.floating().filter(|_| !info.runtime_query)
            && let Some(half) = self.half(info.ty)
        {
            let operand = if self.half_in_float() && !self.excess_expression(expression) {
                self.plain_constant(expression, &folded.real, half, source)?
            } else {
                self.excess_constant(&folded.real, source)?
            };
            self.push_item(operand, info.ty);
            return Ok(());
        }
        // A folded floating constant has exact bits (§6.6 paragraph 8).
        if let Some(folded) = info.floating().filter(|_| !info.runtime_query)
            && let Ok(Repr::Float { ty }) = repr(&self.unit.sema.types, info.ty)
        {
            let bits = floats::constant_bits(&folded.real, ty)
                .ok_or_else(|| LoweringError::missing("a floating constant has no bits", source))?;
            let value = self.fconst(ty, bits);
            self.push_item(Operand::Value(value), info.ty);
            return Ok(());
        }
        match expression.kind {
            | E::Parenthesized { expression: inner }
            | E::Unary {
                operator: UnaryOperator::Extension,
                operand_expression: inner,
            } => self.tasks.push(Task::Evaluate(inner)),
            | E::Generic(_) => {
                let selected = info
                    .selected_expression
                    .ok_or_else(|| LoweringError::missing("a selection has no operand", source))?;
                self.tasks.push(Task::Evaluate(selected));
            },
            | E::Identifier(_) => {
                let item = self.identifier(info, source)?;
                self.operands.push(item);
            },
            | E::Constant(_) => {
                return Err(match repr(&self.unit.sema.types, info.ty) {
                    | Err(kind) => at(kind, source),
                    | Ok(_) => LoweringError::missing("a constant has no value", source),
                });
            },
            | E::StringLiteral(_) => {
                let global = self.unit.string(expression)?;
                let address = self.inst(InstData::GlobalAddr { global }, Type::Ptr);
                self.push_item(Operand::Place(Place::Memory(address)), info.ty);
            },
            | E::Binary {
                operator,
                left_expression,
                right_expression,
            } => match operator {
                | BinaryOperator::LogicalAnd | BinaryOperator::LogicalOr => {
                    let right = self.draft.create_block(&[]);
                    let merge = self.draft.create_block(&[Type::I1]);
                    self.tasks.push(Task::Merge {
                        merge,
                        ty: info.ty,
                        truth: true,
                    });
                    self.tasks.push(Task::JumpWithValue { merge, truth: true });
                    self.tasks.push(Task::Evaluate(right_expression));
                    self.tasks.push(Task::Logical {
                        and: operator == BinaryOperator::LogicalAnd,
                        right,
                        merge,
                    });
                    self.tasks.push(Task::Evaluate(left_expression));
                },
                | BinaryOperator::Comma => {
                    // Clang's comma rounds a `_Float16` operand.
                    if self.excess_type(info.ty) && !self.half_in_float() {
                        self.tasks.push(Task::Settle);
                    }
                    self.tasks.push(Task::Evaluate(right_expression));
                    self.tasks.push(Task::Discard);
                    self.tasks.push(Task::Evaluate(left_expression));
                },
                | BinaryOperator::Assignment => {
                    self.tasks.push(Task::Finish(expression));
                    self.tasks.push(Task::Evaluate(right_expression));
                    self.tasks.push(Task::EvaluateRaw(left_expression));
                },
                | operator if compound_operator(operator).is_some() => {
                    // Choose RHS-first evaluation, then evaluate and update
                    // the lvalue once. In particular, do not retain its old
                    // value across a call that can modify the object.
                    // C99 §6.5.16.2p3 (p. 93; PDF p. 105) permits this order;
                    // C11 §6.5.16.2p3 (pp. 103-104; PDF pp. 121-122) requires
                    // the update to be one evaluation relative to such a call.
                    self.tasks.push(Task::Finish(expression));
                    self.tasks.push(Task::ReadForUpdate(left_expression));
                    self.tasks.push(Task::EvaluateRaw(left_expression));
                    self.tasks.push(Task::Evaluate(right_expression));
                },
                | _ => {
                    self.tasks.push(Task::Finish(expression));
                    self.tasks.push(Task::Evaluate(right_expression));
                    self.tasks.push(Task::Evaluate(left_expression));
                },
            },
            | E::Unary {
                operator,
                operand_expression,
            } => {
                self.tasks.push(Task::Finish(expression));
                match operator {
                    | UnaryOperator::AddressOf | UnaryOperator::Real | UnaryOperator::Imag =>
                        self.tasks.push(Task::EvaluateRaw(operand_expression)),
                    | UnaryOperator::PreIncrement
                    | UnaryOperator::PreDecrement
                    | UnaryOperator::PostIncrement
                    | UnaryOperator::PostDecrement => {
                        self.tasks.push(Task::ReadForUpdate(operand_expression));
                        self.tasks.push(Task::EvaluateRaw(operand_expression));
                    },
                    | _ => self.tasks.push(Task::Evaluate(operand_expression)),
                }
            },
            | E::Call {
                function_expression,
                arguments,
            } => {
                let call = self.call_facts(expression)?;
                if call.arguments.len() != arguments.len() {
                    return Err(LoweringError::missing(
                        "call argument facts are not retained",
                        source,
                    ));
                }
                if self.schedule_builtin(expression)? {
                    return Ok(());
                }
                self.tasks.push(Task::Finish(expression));
                let mut arguments = call.arguments;
                if let Some(pack) = self.forwarded_pack(call)? {
                    self.tasks.push(Task::ForwardPack(pack));
                    arguments = &arguments[..arguments.len() - 1];
                }
                for argument in arguments.iter().rev() {
                    let expression = self
                        .unit
                        .sema
                        .expressions
                        .get(argument.expression.0)
                        .ok_or_else(|| {
                            LoweringError::missing("a call argument is not retained", source)
                        })?
                        .expression;
                    self.tasks.push(Task::SnapshotArgument(expression));
                    self.tasks.push(Task::Evaluate(expression));
                }
                if self
                    .resolved_direct_callee(call, function_expression.source_vectors)?
                    .is_none()
                    && self.packed_callee(expression).is_none()
                {
                    let Callee::Indirect(index) = call.callee else {
                        self.tasks.push(Task::Evaluate(function_expression));
                        return Ok(());
                    };
                    let callee = self
                        .unit
                        .sema
                        .expressions
                        .get(index.0)
                        .ok_or_else(|| {
                            LoweringError::missing("an indirect callee is not retained", source)
                        })?
                        .expression;
                    self.tasks.push(Task::Evaluate(callee));
                }
            },
            | E::DirectMember {
                base_expression, ..
            } => {
                self.tasks.push(Task::Finish(expression));
                self.tasks.push(Task::EvaluateRaw(base_expression));
            },
            | E::IndirectMember {
                base_expression, ..
            }
            | E::Cast {
                operand_expression: base_expression,
                ..
            } => {
                self.tasks.push(Task::Finish(expression));
                self.tasks.push(Task::Evaluate(base_expression));
            },
            | E::Conditional(conditional) => {
                let then = self.draft.create_block(&[]);
                let otherwise = self.draft.create_block(&[]);
                let merge_params = self.merge_params(info.ty, source)?;
                let merge = self.draft.create_block(merge_params);
                self.tasks.push(Task::ConditionalArms {
                    conditional,
                    then,
                    otherwise,
                    merge,
                    ty: info.ty,
                });
                self.tasks.push(Task::Condition {
                    expression: conditional.condition_expression,
                    then,
                    otherwise,
                });
            },
            | E::SizeofType(_) | E::SizeofExpr(_) => {
                let query = self.unit.sema.size_query_site(expression).ok_or_else(|| {
                    LoweringError::missing("sizeof evaluation facts are not retained", source)
                })?;
                let formula = query.formula.ok_or_else(|| {
                    LoweringError::missing("sizeof size formula is not retained", source)
                })?;
                self.tasks.push(Task::Sizeof(formula, info.ty, source));
                let mut tasks = ArenaVec::new_in(self.scratch);
                Self::prepare_site_tasks(query.site, &mut tasks);
                self.prepare_bound_tasks(query.site.bounds, &mut tasks, source)?;
                if let Some(operand) = query.operand {
                    tasks.push(Task::EvaluateRaw(operand));
                    tasks.push(Task::Discard);
                }
                self.push_all(&tasks);
            },
            | E::AlignofType(_) | E::AlignofExpr(_) =>
                return Err(LoweringError::unsupported(
                    Construct::VariableLengthArray,
                    source,
                )),
            | E::CompoundLiteral { initializer, .. } =>
                self.compound_literal(info.ty, initializer, source)?,
            | E::Builtin(b) => self.schedule_keyword(expression, b)?,
            | E::OmittedConditional(c) => {
                let merge_params = self.merge_params(info.ty, source)?;
                let merge = self.draft.create_block(merge_params);
                let otherwise = self.draft.create_block(&[]);
                self.tasks.push(Task::OmittedConditional {
                    expression,
                    conditional: c,
                    otherwise,
                    merge,
                });
                self.tasks.push(Task::EvaluateRaw(c.condition_expression));
            },
            | E::StatementExpression(_) =>
                self.statement_expression(expression, super::StatementMode::Value)?,
            | E::LabelAddress(_) => {
                let label = self
                    .unit
                    .sema
                    .control
                    .label_address(expression)
                    .ok_or_else(|| {
                        LoweringError::missing("a label address has no resolved label", source)
                    })?;
                let label = self.block_label(label, source)?;
                let address = self.inst(InstData::BlockAddr { label }, Type::Ptr);
                self.push_item(Operand::Value(address), info.ty);
            },
            | E::Nullptr => {
                let value = self.nullary(Opcode::Null);
                self.push_item(Operand::Value(value), info.ty);
            },
            | E::Countof(_) | E::Boolean(_) | E::Error =>
                return Err(LoweringError::missing(
                    "an expression was not parsed",
                    source,
                )),
        }
        self.prepare_expression_actions(expression)?;
        Ok(())
    }

    /// Completes an operator whose operands are on the stack.
    pub(super) fn finish(&mut self, expression: &'tu Expression<'tu>) -> Result<(), LoweringError> {
        use ExpressionType as E;
        let source = expression.source_vectors;
        let info = self.info(expression)?;
        match expression.kind {
            | E::Binary {
                operator: BinaryOperator::Assignment,
                ..
            } => {
                let value = self.pop();
                let place = self.pop();
                let result = self.write(place, value, source)?;
                self.push_item(result.operand, info.ty);
            },
            | E::Binary { operator, .. } if let Some(operation) = compound_operator(operator) => {
                let left = self.pop();
                let place = self.pop();
                let right = self.pop_excess();
                let operation_type = info.operation_type.ok_or_else(|| {
                    LoweringError::missing("a compound assignment has no type", source)
                })?;
                if matches!(self.unit.sema.types.kind(place.ty), TypeKind::Atomic(_)) {
                    let back = self.unit.sema.types.non_atomic(place.ty).unqualified();
                    let (_, stored) = self.atomic_update(place, left, source, |this, old| {
                        let old = this.convert_value(old, operation_type, source)?;
                        let result =
                            this.atomic_binary(operation, old, right, operation_type, source)?;
                        this.convert_value(result, back, source)
                    })?;
                    self.push_item(stored.operand, info.ty);
                    return Ok(());
                }
                let result = self.binary(operation, left, right, operation_type, source)?;
                // The conversion back to the left operand's type is recorded
                // on the compound assignment itself (§6.5.16.2p3).
                let back = self
                    .conversions(expression)
                    .first()
                    .filter(|conversion| conversion.kind == ConversionKind::Assignment)
                    .map_or(info.ty, |conversion| conversion.ty);
                let converted = self.convert_value(result, back, source)?;
                let stored = self.write(place, converted, source)?;
                self.push_item(stored.operand, info.ty);
            },
            | E::Binary {
                operator: BinaryOperator::Subscript,
                ..
            } => {
                let right = self.pop();
                let left = self.pop();
                let (pointer, index) = if self.is_pointer(left.ty) {
                    (left, right)
                } else {
                    (right, left)
                };
                let address = self.offset_pointer(pointer, index, false, source)?;
                self.push_item(Operand::Place(Place::Memory(address)), info.ty);
            },
            | E::Binary { operator, .. } => {
                let right = self.pop_excess();
                let left = self.pop_excess();
                let result = self.binary(operator, left, right, info.ty, source)?;
                self.operands.push(result);
            },
            | E::Unary { operator, .. } => self.unary(operator, info, source)?,
            | E::Call { arguments, .. } => self.call(arguments.len(), info, source)?,
            | E::DirectMember { .. } | E::IndirectMember { .. } => {
                let base = self.pop();
                let base_ty = base.ty;
                let field = self.unit.sema.selected_field(expression).ok_or_else(|| {
                    LoweringError::missing("a member access has no field", source)
                })?;
                let (base, rvalue) = match base.operand {
                    | Operand::Place(Place::Memory(address)) | Operand::Value(address) =>
                        (address, false),
                    | Operand::Aggregate(address) => (address, true),
                    | _ =>
                        return Err(LoweringError::missing(
                            "a member base has no address",
                            source,
                        )),
                };
                let member = self.unit.sema.member_facts(expression).ok_or_else(|| {
                    LoweringError::missing("member facts are not retained", source)
                })?;
                let guaranteed = member.alignment;
                debug_assert_eq!(
                    member.bit_field, field.width,
                    "member access retains its field width"
                );
                self.member_base(base, base_ty, source)?;
                let address = self.offset_address(base, field.offset);
                let align = self.place_alignment(
                    address,
                    Align::from_bytes(u64::from(guaranteed)).unwrap_or(Align::BYTE),
                );
                self.retain_alignment(address, align);
                let access_ty = member.access_type;
                let place = if let Some(width) = field.width {
                    let bits = BitFieldBits {
                        bit_offset: field.bit_offset,
                        width,
                    };
                    let bit_field = self.bit_field(access_ty, field.offset, bits, source)?;
                    Place::BitField(address, bit_field)
                } else {
                    Place::Memory(address)
                };
                if rvalue
                    && let TypeKind::Array(_, ArrayBound::Incomplete) =
                        self.unit.sema.types.kind(access_ty)
                {
                    // A flexible array member of a structure rvalue has no
                    // size to copy; it stays an array at its offset in the
                    // temporary and decays from there.
                    // C99: §6.7.2.1p16, p. 103; PDF p. 115.
                    self.push_item(Operand::Aggregate(address), access_ty);
                } else if rvalue {
                    // A member of a structure rvalue is not an lvalue
                    // (§6.5.2.3p3), so it is read at once.
                    let item = self.read(place, access_ty, info.ty.unqualified(), source)?;
                    self.operands.push(item);
                } else {
                    self.push_item(Operand::Place(place), access_ty);
                }
            },
            | E::Cast { .. } => {
                // A cast rounds an excess, but not GCC's widened value.
                let item = self.pop_excess();
                let item = if self.widened(item) {
                    item
                } else {
                    self.settle(item)
                };
                if let Some(member) = info.operation_type {
                    let layout = types::layout(&self.unit.sema.types, info.ty)
                        .map_err(|kind| at(kind, source))?;
                    let address = self.fresh_object(
                        layout.size,
                        super::alignment::storage_alignment(&self.unit.sema.types, info.ty)
                            .map_err(|kind| at(kind, source))?,
                        source,
                    )?;
                    // GCC Cast-to-Union: this place is backed by the union
                    // temporary, whose guarantee may be weaker than the
                    // member's. <https://gcc.gnu.org/onlinedocs/gcc/Cast-to-Union.html>
                    _ = self.write(
                        Item {
                            operand: Operand::Place(Place::Memory(address)),
                            ty:      member,
                        },
                        item,
                        source,
                    )?;
                    self.push_item(Operand::Aggregate(address), info.ty);
                    return Ok(());
                }
                let operand = if repr(&self.unit.sema.types, info.ty)
                    .map_err(|kind| at(kind, source))?
                    == Repr::Void
                {
                    Operand::Void
                } else {
                    item.operand
                };
                self.push_item(operand, info.ty);
            },
            | _ =>
                return Err(LoweringError::missing(
                    "an operator has no completion",
                    source,
                )),
        }
        Ok(())
    }

    /// Applies the conversions recorded on `expression`, from the `skip`th,
    /// to the operand on top of the stack.
    pub(super) fn convert_recorded(
        &mut self,
        expression: &'tu Expression<'tu>,
        skip: usize,
    ) -> Result<(), LoweringError> {
        let source = expression.source_vectors;
        let mut item = self.pop_excess();
        let address = std::ptr::from_ref(expression).addr();
        let arm = self
            .conditional_arms
            .iter()
            .find(|(arm, _)| *arm == address)
            .map(|&(_, excess)| excess);
        self.plain_arm = arm == Some(false);
        for conversion in self.conversions(expression).iter().skip(skip) {
            if self.unit.sema.types.unanalyzed(conversion.ty) {
                return Err(LoweringError::unanalyzed(
                    &self.unit.sema.types,
                    conversion.ty,
                    source,
                ));
            }
            item = match conversion.kind {
                | ConversionKind::Lvalue => match item.operand {
                    | Operand::Place(place) => self.read(place, item.ty, conversion.ty, source)?,
                    | operand => Item {
                        operand,
                        ty: conversion.ty,
                    },
                },
                | ConversionKind::ArrayDecay | ConversionKind::FunctionDecay => {
                    let address = match item.operand {
                        | Operand::Place(Place::Memory(address))
                        | Operand::Aggregate(address)
                        | Operand::Value(address) => address,
                        | Operand::Place(Place::Function(func)) =>
                            self.inst(InstData::FuncAddr { func }, Type::Ptr),
                        | _ =>
                            return Err(LoweringError::missing(
                                "a decayed operand has no address",
                                source,
                            )),
                    };
                    Item {
                        operand: Operand::Value(address),
                        ty:      conversion.ty,
                    }
                },
                // GCC converts an integer operand of a `_Float16` operation
                // straight to `float`, and the other half type's operand too:
                // `__bf16 + _Float16` is evaluated in `float`, so a `__bf16`
                // beyond binary16's range stays finite.
                | ConversionKind::Arithmetic
                    if self.half_in_float()
                        && self.half(conversion.ty).is_some()
                        && self.half(item.ty).is_some()
                        && self.half(item.ty) != self.half(conversion.ty) =>
                {
                    match self.widen_plain(item, conversion.ty) {
                        | Some(widened) => widened,
                        | None => Item {
                            operand: Operand::Excess(self.excess(item, source)?),
                            ty:      conversion.ty,
                        },
                    }
                },
                | ConversionKind::Arithmetic
                    if self.half_in_float()
                        && self.half(conversion.ty).is_some()
                        && matches!(self.repr(item.ty, source)?, Repr::Int { .. } | Repr::Bool) =>
                {
                    let from = self.repr(item.ty, source)?;
                    let value = self.value(item, source)?;
                    let value = self.float_from(value, from, Type::F32, source)?;
                    Item {
                        operand: Operand::Excess(value),
                        ty:      conversion.ty,
                    }
                },
                | ConversionKind::Arithmetic
                | ConversionKind::Assignment
                | ConversionKind::DefaultArgument =>
                    self.convert_value(item, conversion.ty, source)?,
                | ConversionKind::TransparentUnion =>
                    self.transparent_union(item, conversion.ty, source)?,
            };
        }
        self.plain_arm = false;
        // A plain constant arm of a conditional without an excess arm is
        // rounded, as GCC folds its conversion to the result type.
        if arm == Some(false) && self.widened_constant(item) {
            item = self.settle(item);
        }
        self.operands.push(item);
        Ok(())
    }

    /// A union of type `union` whose member at offset zero, of the type of
    /// `item`, holds the value of `item`: a GNU transparent union argument.
    /// Every member of a union starts at its beginning (§6.7.2.1p14, p. 103;
    /// PDF p. 115), and semantic analysis selected the member.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Common-Type-Attributes.html>
    fn transparent_union(
        &mut self,
        item: Item,
        union: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let layout =
            types::layout(&self.unit.sema.types, union).map_err(|kind| at(kind, source))?;
        let address = self.fresh_object(layout.size, types::align(layout), source)?;
        let member = Item {
            operand: Operand::Place(Place::Memory(address)),
            ty:      item.ty,
        };
        _ = self.write(member, item, source)?;
        Ok(Item {
            operand: Operand::Aggregate(address),
            ty:      union,
        })
    }

    /// Converts a value to type `to` as by assignment (§6.3.1.2-§6.3.1.4,
    /// §6.3.2.3): integers are sign- or zero-extended from their own
    /// signedness or truncated, `_Bool` compares with zero, and integers and
    /// pointers convert to each other through `i64`, and floating types convert
    /// to integers by truncation and among themselves by widening or rounding.
    /// C99: §6.3.1.2-§6.3.1.5, pp. 43-44; PDF pp. 55-56; §6.3.2.3, pp. 47-48;
    /// PDF pp. 59-60.
    pub(super) fn convert_value(
        &mut self,
        item: Item,
        to: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let target = self.repr(to, source)?;
        if let Some(widened) = self.widen_plain(item, to) {
            return Ok(widened);
        }
        // GCC rounds a widened value converted to `_Complex _Float16` as an
        // arm of a `?:` neither of whose arms keeps excess precision; a cast
        // leaves it.
        let item = if self.complex_half(to) && self.widened(item) && self.plain_arm {
            self.settle(item)
        } else {
            item
        };
        // A `_Complex _Float16` excess converts to its own type unrounded,
        // and for GCC to any other type from its `float` value.
        if self.complex_excess(item).is_some() {
            if self.complex_half(to) {
                return Ok(Item { ty: to, ..item });
            }
            if self.half_in_float() {
                return self.convert_complex(item, to, source);
            }
        }
        let item = match (item.operand, target) {
            // An excess converts to its own type, or for GCC to the other
            // half type, without rounding.
            | (Operand::Excess(_), _)
                if self.half(to).is_some()
                    && (self.half_in_float() || self.half(to) == self.half(item.ty)) =>
                return Ok(Item { ty: to, ..item }),
            | (Operand::Excess(_), _) if self.half_in_float() =>
                return self.convert_excess(item, to, target, source),
            | _ => self.settle(item),
        };
        let types = &self.unit.sema.types;
        if matches!(target, Repr::Complex { .. })
            || matches!(repr(types, item.ty), Ok(Repr::Complex { .. }))
        {
            return self.convert_complex(item, to, source);
        }
        if matches!(item.operand, Operand::Value(_)) && repr(types, item.ty) == Ok(target) {
            return Ok(Item { ty: to, ..item });
        }
        let operand = match (item.operand, target) {
            | (_, Repr::Void) => Operand::Void,
            | (Operand::Aggregate(_), Repr::Aggregate) => item.operand,
            | (Operand::Value(_) | Operand::Truth(_), Repr::Bool) => {
                let truth = self.truth(item, source)?;
                Operand::Value(self.extend(truth, false, Type::I8))
            },
            | (Operand::Truth(truth), Repr::Int { ty, .. }) =>
                Operand::Value(self.extend(truth, false, ty)),
            | (Operand::Value(value), Repr::Int { ty, signed }) => {
                let from = repr(types, item.ty).map_err(|kind| at(kind, source))?;
                Operand::Value(match from {
                    | Repr::Pointer => self.convert(Opcode::Ptrtoint, ty, value),
                    // A 16-bit floating value widens exactly first.
                    | Repr::Float { ty: half } if half.half_format().is_some() => {
                        let wide = self.widen_half(value);
                        let opcode = if signed {
                            Opcode::Fptosi
                        } else {
                            Opcode::Fptoui
                        };
                        self.convert(opcode, ty, wide)
                    },
                    // A floating value truncates toward zero (§6.3.1.4p1).
                    | Repr::Float { .. } => {
                        let opcode = if signed {
                            Opcode::Fptosi
                        } else {
                            Opcode::Fptoui
                        };
                        self.convert(opcode, ty, value)
                    },
                    | _ => self.extend(value, from.signed(), ty),
                })
            },
            // A 16-bit type takes the truth value through `float`.
            | (Operand::Truth(truth), Repr::Float { ty }) if ty.half_format().is_some() => {
                let single = self.convert(Opcode::Uitofp, Type::F32, truth);
                Operand::Value(self.round_single(single, ty))
            },
            | (Operand::Truth(truth), Repr::Float { ty }) =>
                Operand::Value(self.convert(Opcode::Uitofp, ty, truth)),
            | (Operand::Value(value), Repr::Float { ty }) => {
                let from = repr(types, item.ty).map_err(|kind| at(kind, source))?;
                Operand::Value(self.float_from(value, from, ty, source)?)
            },
            | (Operand::Truth(_) | Operand::Value(_), Repr::Pointer) => {
                let from = repr(types, item.ty).map_err(|kind| at(kind, source))?;
                let value = self.value(item, source)?;
                Operand::Value(match from {
                    | Repr::Pointer => value,
                    | Repr::Float { .. } =>
                        return Err(LoweringError::missing(
                            "a floating value converts to a pointer",
                            source,
                        )),
                    | _ => self.int_to_pointer(value, from.signed()),
                })
            },
            | _ =>
                return Err(LoweringError::missing(
                    "a conversion has no lowering",
                    source,
                )),
        };
        Ok(Item { operand, ty: to })
    }

    /// A value of representation `from` converted to the floating type `to`:
    /// an integer converts exactly where it can and otherwise rounds to
    /// nearest (`sitofp`, `uitofp`), and a floating type widens (`fpext`) or
    /// narrows (`fptrunc`). An integer constant folds.
    /// C99: §6.3.1.4 paragraph 2, pp. 43-44; PDF pp. 55-56; §6.3.1.5 paragraph
    /// 1, p. 44; PDF p. 56.
    pub(super) fn float_from(
        &mut self,
        value: Value,
        from: Repr,
        to: Type,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        Ok(match from {
            | Repr::Float { ty } if ty == to => value,
            // A `bf16` widens to anything but `f32`, and one 16-bit format
            // converts to the other, through `f32`, which holds both exactly.
            | Repr::Float { ty }
                if ty.half_format().is_some()
                    && to != Type::F32
                    && (ty == Type::BF16 || to.half_format().is_some()) =>
            {
                let single = self.widen_half(value);
                self.float_from(single, Repr::Float { ty: Type::F32 }, to, source)?
            },
            | Repr::Float { ty } if ty.bits() < to.bits() => self.convert(Opcode::Fpext, to, value),
            | Repr::Float { .. } => self.convert(Opcode::Fptrunc, to, value),
            | Repr::Int { .. } | Repr::Bool => {
                let signed = from.signed();
                let ty = self.draft.value_type(value);
                // A constant rounds once from its exact value, as GCC and
                // Clang fold it; an unsigned `i128` from its magnitude.
                if let Some(constant) = self.draft.constant(value) {
                    let exact = self.cast_constant(constant, ty, signed, Type::I128);
                    let bits = if signed || ty != Type::I128 {
                        floats::from_integer(exact, to)
                    } else {
                        floats::from_unsigned(exact.cast_unsigned(), to)
                    };
                    if let Some(bits) = bits {
                        return Ok(self.fconst(to, bits));
                    }
                }
                let opcode = if signed {
                    Opcode::Sitofp
                } else {
                    Opcode::Uitofp
                };
                // An integer becomes a `_Float16` through `float`, exactly
                // where binary16 is finite and otherwise as an infinity
                // either way, as GCC and Clang convert it.
                if to == Type::F16 {
                    let single = self.convert(opcode, Type::F32, value);
                    return Ok(self.convert(Opcode::Fptrunc, Type::F16, single));
                }
                if to == Type::BF16 {
                    return Ok(self.integer_to_bfloat(opcode, value, ty));
                }
                self.convert(opcode, to, value)
            },
            | _ =>
                return Err(LoweringError::missing(
                    "a conversion to a floating type has no lowering",
                    source,
                )),
        })
    }

    /// An integer of type `ty` converted to `bf16` as the claimed compiler
    /// converts it. Clang converts to `float` and rounds that, as LLVM
    /// lowers the conversion, so a wide integer rounds twice. GCC rounds
    /// once: through `double` or the x87 format, which hold the integer
    /// exactly, and an `i128` through `__floattibf`.
    /// C99: §6.3.1.4 paragraph 2, pp. 43-44; PDF pp. 55-56.
    fn integer_to_bfloat(&mut self, opcode: Opcode, value: Value, ty: Type) -> Value {
        let through = if !self.half_in_float() {
            Type::F32
        } else if ty.bits() <= 32 {
            Type::F64
        } else if ty == Type::I64 {
            Type::F80
        } else {
            return self.convert(opcode, Type::BF16, value);
        };
        let wide = self.convert(opcode, through, value);
        self.convert(Opcode::Fptrunc, Type::BF16, wide)
    }

    /// The IR value of a scalar operand, materializing a comparison's `i1`
    /// as its `int` type.
    pub(super) fn value(
        &mut self,
        item: Item,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        match item.operand {
            | Operand::Value(value) => Ok(value),
            | Operand::Truth(truth) => {
                let ty = self.value_type(item.ty, source)?;
                Ok(self.extend(truth, false, ty))
            },
            // A vector lvalue reaches its value use unread: vector values
            // have no IR representation yet, though their addresses do.
            // GNU extension: GCC manual, "Vector Extensions".
            | _ if matches!(self.unit.sema.types.kind(item.ty), TypeKind::Vector { .. }) =>
                Err(LoweringError::unsupported(Construct::Vector, source)),
            | _ => Err(LoweringError::missing("an operand has no value", source)),
        }
    }

    /// Whether a scalar operand is nonzero, as an `i1`. A null pointer is
    /// zero.
    /// C99: §6.3.1.2 paragraph 1, p. 43; PDF p. 55; §6.5.3.3 paragraph 5,
    /// p. 79; PDF p. 91.
    pub(super) fn truth(
        &mut self,
        item: Item,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let value = match item.operand {
            | Operand::Truth(truth) => return Ok(truth),
            | Operand::Complex(id) => {
                let [mut real, mut imag] = self.complex_values[id as usize];
                if self.draft.value_type(real).half_format().is_some() {
                    real = self.widen_half(real);
                    imag = self.widen_half(imag);
                }
                let zero = self.fconst(self.draft.value_type(real), 0);
                let a = self.fcmp(FloatCC::Une, real, zero);
                let b = self.fcmp(FloatCC::Une, imag, zero);
                return Ok(self.binary_inst(Opcode::Or, InstFlags::empty(), a, b));
            },
            | Operand::Value(value) if self.draft.value_type(value).half_format().is_some() =>
                self.widen_half(value),
            | Operand::Excess(value) | Operand::Value(value) => value,
            | _ => return Err(LoweringError::missing("a condition has no value", source)),
        };
        let ty = self.draft.value_type(value);
        if ty.is_float() {
            // A NaN is nonzero: `une` is true when either operand is unordered.
            // C99: §6.3.1.2 paragraph 1, p. 43; PDF p. 55.
            let zero = self.fconst(ty, 0);
            return Ok(self.fcmp(FloatCC::Une, value, zero));
        }
        if let Some(constant) = self.draft.constant(value) {
            let nonzero = (constant as u128) & ty.mask() != 0;
            return Ok(self.iconst(Type::I1, i128::from(nonzero)));
        }
        let zero = if ty == Type::Ptr {
            self.nullary(Opcode::Null)
        } else {
            self.iconst(ty, 0)
        };
        Ok(self.icmp(IntCC::Ne, value, zero))
    }

    /// What remains of `expression` once semantic analysis elided one of its
    /// operands: every consumer of an operator whose operand is elided goes
    /// through here, so lowering never reaches code whose references
    /// reachability dropped. A folded `?:` keeps only its live arm, and a
    /// folded `&&` or `||` only its short-circuit value; the condition, whose
    /// fold has no side effects, is not evaluated (Clang
    /// `ConstantFoldsToSimpleInteger`). The common operand of a GNU `x ?: y`
    /// is evaluated even then. C99: §6.5.13p3-4, §6.5.14p3-4, pp. 89-90;
    /// PDF pp. 101-102; §6.5.15p4, p. 90; PDF p. 102.
    pub(super) fn folded(
        &self,
        expression: &'tu Expression<'tu>,
    ) -> Result<Option<Folded<'tu>>, LoweringError> {
        use ExpressionType as E;
        if let Some(operand) = self.unit.sema.branch_operand(expression) {
            return Ok(Some(Folded::Live(operand)));
        }
        Ok(match expression.kind {
            | E::Conditional(c) if self.info(c.then_expression)?.elided =>
                Some(Folded::Live(c.else_expression)),
            | E::Conditional(c) if self.info(c.else_expression)?.elided =>
                Some(Folded::Live(c.then_expression)),
            | E::OmittedConditional(c) if self.info(c.else_expression)?.elided =>
                Some(Folded::Live(c.condition_expression)),
            | E::Binary {
                operator: operator @ (BinaryOperator::LogicalAnd | BinaryOperator::LogicalOr),
                right_expression,
                ..
            } if self.info(right_expression)?.elided =>
                Some(Folded::Truth(operator == BinaryOperator::LogicalOr)),
            | _ => None,
        })
    }

    /// Evaluates a condition, branching straight on `&&`, `||` and `!`
    /// without materializing their `int` results.
    /// C99: §6.5.13-§6.5.14, pp. 89-90; PDF pp. 101-102.
    pub(super) fn condition(
        &mut self,
        expression: &'tu Expression<'tu>,
        then: Block,
        otherwise: Block,
    ) -> Result<(), LoweringError> {
        let info = self.info(expression)?;
        debug_assert!(
            !info.elided,
            "lowering reached a condition semantic analysis elided"
        );
        if let Some(operand) = self.unit.sema.branch_operand(expression) {
            self.tasks.push(Task::Condition {
                expression: operand,
                then,
                otherwise,
            });
            return Ok(());
        }
        let folded = info.integer.is_some();
        if self.conversions(expression).is_empty() && !folded {
            match self.folded(expression)? {
                | Some(Folded::Live(operand)) => {
                    self.tasks.push(Task::Condition {
                        expression: operand,
                        then,
                        otherwise,
                    });
                    return Ok(());
                },
                | Some(Folded::Truth(truth)) => {
                    self.draft.jump(if truth { then } else { otherwise }, &[]);
                    return Ok(());
                },
                | None => {},
            }
            match expression.kind {
                | ExpressionType::Parenthesized { expression: inner }
                | ExpressionType::Unary {
                    operator: UnaryOperator::Extension,
                    operand_expression: inner,
                } => {
                    self.tasks.push(Task::Condition {
                        expression: inner,
                        then,
                        otherwise,
                    });
                    return Ok(());
                },
                | ExpressionType::Unary {
                    operator: UnaryOperator::LogicalNot,
                    operand_expression,
                } => {
                    self.tasks.push(Task::Condition {
                        expression: operand_expression,
                        then:       otherwise,
                        otherwise:  then,
                    });
                    return Ok(());
                },
                | ExpressionType::Binary {
                    operator: operator @ (BinaryOperator::LogicalAnd | BinaryOperator::LogicalOr),
                    left_expression,
                    right_expression,
                } => {
                    let middle = self.draft.create_block(&[]);
                    self.tasks.push(Task::Condition {
                        expression: right_expression,
                        then,
                        otherwise,
                    });
                    self.tasks.push(Task::Enter {
                        block: middle,
                        seal:  true,
                    });
                    let (then, otherwise) = if operator == BinaryOperator::LogicalAnd {
                        (middle, otherwise)
                    } else {
                        (then, middle)
                    };
                    self.tasks.push(Task::Condition {
                        expression: left_expression,
                        then,
                        otherwise,
                    });
                    return Ok(());
                },
                | ExpressionType::Conditional(c)
                    if !self.info(c.then_expression)?.elided
                        && !self.info(c.else_expression)?.elided =>
                {
                    let yes = self.draft.create_block(&[]);
                    let no = self.draft.create_block(&[]);
                    self.tasks.push(Task::Condition {
                        expression: c.else_expression,
                        then,
                        otherwise,
                    });
                    self.tasks.push(Task::Enter {
                        block: no,
                        seal:  true,
                    });
                    self.tasks.push(Task::Condition {
                        expression: c.then_expression,
                        then,
                        otherwise,
                    });
                    self.tasks.push(Task::Enter {
                        block: yes,
                        seal:  true,
                    });
                    self.tasks.push(Task::Condition {
                        expression: c.condition_expression,
                        then:       yes,
                        otherwise:  no,
                    });
                    return Ok(());
                },
                | _ => {},
            }
        }
        self.tasks.push(Task::Branch { then, otherwise });
        self.tasks.push(Task::Evaluate(expression));
        Ok(())
    }

    /// Pops a scalar and branches on it; a constant condition jumps.
    pub(super) fn branch(&mut self, then: Block, otherwise: Block) -> Result<(), LoweringError> {
        let item = self.pop_truth();
        let truth = self.truth(item, SourceVectors::empty())?;
        match self.draft.constant(truth) {
            | Some(0) => self.draft.jump(otherwise, &[]),
            | Some(_) => self.draft.jump(then, &[]),
            | None => self.draft.brif(truth, then, otherwise),
        }
        Ok(())
    }

    /// After the left operand of `&&` or `||`: a false left operand of `&&`
    /// or a true one of `||` decides the result without the right operand.
    /// C99: §6.5.13 paragraph 4 and §6.5.14 paragraph 4, pp. 89-90;
    /// PDF pp. 101-102.
    pub(super) fn logical(
        &mut self,
        and: bool,
        right: Block,
        merge: Block,
    ) -> Result<(), LoweringError> {
        let item = self.pop_truth();
        let truth = self.truth(item, SourceVectors::empty())?;
        let decided = self.iconst(Type::I1, i128::from(!and));
        let (then, otherwise) = if and {
            (
                self.draft.edge(right, &[]),
                self.draft.edge(merge, &[decided]),
            )
        } else {
            (
                self.draft.edge(merge, &[decided]),
                self.draft.edge(right, &[]),
            )
        };
        self.draft
            .terminate(Terminator::Brif(truth, [then, otherwise]));
        self.draft.seal(right);
        self.draft.switch_to(right);
        Ok(())
    }

    /// Passes the operand on top of the stack to `merge`'s parameter, as an
    /// `i1` truth value if `truth`.
    pub(super) fn jump_with_value(
        &mut self,
        merge: Block,
        truth: bool,
    ) -> Result<(), LoweringError> {
        let source = SourceVectors::empty();
        // GCC's `?:` keeps a `_Float16` excess in an `f32` parameter.
        if !truth
            && let &[param] = self.draft.block_params(merge)
            && self.draft.value_type(param) == Type::F32
        {
            let item = self.pop_excess();
            if self.half(item.ty).is_some() {
                let value = self.excess(item, source)?;
                self.draft.jump(merge, &[value]);
                return Ok(());
            }
            self.operands.push(item);
        }
        // And a `_Complex _Float16` excess in a pair of them.
        if !truth
            && self.half_in_float()
            && let &[param, _] = self.draft.block_params(merge)
            && self.draft.value_type(param) == Type::F32
        {
            let item = self.pop_excess();
            if self.complex_half(item.ty) {
                let item = self.single_operand(item, true, source)?;
                let parts = self.complex_parts(item, source)?;
                self.draft.jump(merge, &parts);
                return Ok(());
            }
            self.operands.push(item);
        }
        let item = if truth { self.pop_truth() } else { self.pop() };
        if let Operand::Complex(id) = item.operand
            && !truth
        {
            self.draft.jump(merge, &self.complex_values[id as usize]);
            return Ok(());
        }
        let args = match self.draft.block_params(merge).len() {
            | 0 => None,
            | _ if truth => Some(self.truth(item, source)?),
            | _ => Some(match item.operand {
                | Operand::Aggregate(address) => address,
                | _ => self.value(item, source)?,
            }),
        };
        self.draft.jump(merge, args.as_slice());
        Ok(())
    }

    /// Continues in a merge block, whose parameter is the result.
    pub(super) fn merge(&mut self, merge: Block, ty: TypeId, truth: bool) {
        self.draft.seal(merge);
        self.draft.switch_to(merge);
        if matches!(repr(&self.unit.sema.types, ty), Ok(Repr::Complex { .. })) && !truth {
            let pair = [
                self.draft.block_params(merge)[0],
                self.draft.block_params(merge)[1],
            ];
            let operand = self.complex_pair(pair);
            self.push_item(operand, ty);
            return;
        }
        let operand = match self.draft.block_params(merge).first() {
            | None => Operand::Void,
            | Some(&param) if truth => Operand::Truth(param),
            | Some(&param)
                if self.draft.value_type(param) == Type::Ptr
                    && repr(&self.unit.sema.types, ty) == Ok(Repr::Aggregate) =>
            // Control joins may select storage with different guarantees.
            {
                self.retain_alignment(param, Align::BYTE);
                Operand::Aggregate(param)
            },
            | Some(&param)
                if self.half(ty).is_some() && self.draft.value_type(param) == Type::F32 =>
                Operand::Excess(param),
            | Some(&param) => Operand::Value(param),
        };
        self.push_item(operand, ty);
    }

    /// Schedules the arms of `?:` after its condition branched; only the
    /// selected arm is evaluated.
    /// C99: §6.5.15 paragraph 4, p. 90; PDF p. 102.
    pub(super) fn conditional_arms(
        &mut self,
        conditional: &'tu ConditionalExpression<'tu>,
        then: Block,
        otherwise: Block,
        merge: Block,
        ty: TypeId,
    ) {
        // GCC converts both arms to the excess type when either arm is
        // evaluated with excess precision, and otherwise to the result type.
        if self.half_in_float() {
            let excess = self.excess_expression(conditional.then_expression)
                || self.excess_expression(conditional.else_expression);
            for arm in [conditional.then_expression, conditional.else_expression] {
                self.conditional_arms
                    .push((std::ptr::from_ref(arm).addr(), excess));
            }
        }
        self.tasks.push(Task::Merge {
            merge,
            ty,
            truth: false,
        });
        self.tasks.push(Task::JumpWithValue {
            merge,
            truth: false,
        });
        self.tasks.push(Task::Evaluate(conditional.else_expression));
        self.tasks.push(Task::Enter {
            block: otherwise,
            seal:  true,
        });
        self.tasks.push(Task::JumpWithValue {
            merge,
            truth: false,
        });
        self.tasks.push(Task::Evaluate(conditional.then_expression));
        self.tasks.push(Task::Enter {
            block: then,
            seal:  true,
        });
    }
}

// Operators

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    /// Atomic add/sub compound updates follow the wrapping RMW operations.
    /// Other operators retain their ordinary C arithmetic flags.
    fn atomic_binary(
        &mut self,
        operator: BinaryOperator,
        left: Item,
        right: Item,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        if matches!(
            operator,
            BinaryOperator::Addition | BinaryOperator::Subtraction
        ) && matches!(self.repr(ty, source)?, Repr::Int { .. })
        {
            let a = self.value(left, source)?;
            let b = self.value(right, source)?;
            let op = if operator == BinaryOperator::Addition {
                Opcode::Iadd
            } else {
                Opcode::Isub
            };
            return Ok(Item {
                operand: Operand::Value(self.binary_inst(op, InstFlags::empty(), a, b)),
                ty,
            });
        }
        self.binary(operator, left, right, ty, source)
    }

    /// A binary operator on converted operands, with result type `ty`.
    /// C99: §6.5.5-§6.5.12, pp. 82-88; PDF pp. 94-100.
    fn binary(
        &mut self,
        operator: BinaryOperator,
        left: Item,
        right: Item,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        use BinaryOperator as B;
        if self.complex_half(left.ty) || self.complex_half(right.ty) {
            return self.complex_half_binary(operator, left, right, ty, source);
        }
        if self.half(left.ty).is_some()
            && (comparison(operator).is_some() || self.half(ty).is_some())
        {
            return self.half_binary(operator, left, right, ty, source);
        }
        let (left, right) = (self.settle(left), self.settle(right));
        if matches!(left.operand, Operand::Complex(_))
            || matches!(right.operand, Operand::Complex(_))
        {
            return self.complex_binary(operator, left, right, ty, source);
        }
        let left_pointer = self.is_pointer(left.ty);
        let right_pointer = self.is_pointer(right.ty);
        if let Some(cc) = comparison(operator) {
            if matches!(self.repr(left.ty, source)?, Repr::Float { .. }) {
                // C99 Annex F: `!=` is true for a NaN operand; the rest are
                // ordered and false.
                // C99: §6.5.8 paragraph 6, p. 86; PDF p. 98; §F.3 paragraph 1,
                // pp. 445-447; PDF pp. 457-459.
                let a = self.value(left, source)?;
                let b = self.value(right, source)?;
                let truth = self.fcmp(float_comparison(cc), a, b);
                return Ok(Item {
                    operand: Operand::Truth(truth),
                    ty,
                });
            }
            let (a, b, signed) = if left_pointer || right_pointer {
                (
                    self.pointer_operand(left, source)?,
                    self.pointer_operand(right, source)?,
                    false,
                )
            } else {
                let signed = self.repr(left.ty, source)?.signed();
                (
                    self.value(left, source)?,
                    self.value(right, source)?,
                    signed,
                )
            };
            let cc = if signed { cc } else { unsigned(cc) };
            let truth = self.icmp(cc, a, b);
            return Ok(Item {
                operand: Operand::Truth(truth),
                ty,
            });
        }
        match operator {
            | B::Addition | B::Subtraction if left_pointer && right_pointer => {
                // C99 §6.5.6p9: the difference of two pointers into one array
                // is the difference of their subscripts.
                let a = self.value(left, source)?;
                let a = self.convert(Opcode::Ptrtoint, Type::I64, a);
                let b = self.value(right, source)?;
                let b = self.convert(Opcode::Ptrtoint, Type::I64, b);
                let mut difference = self.binary_inst(Opcode::Isub, InstFlags::empty(), a, b);
                if self.static_element_size(left.ty) != Some(1) {
                    let size = self.element_size(left.ty, source)?;
                    difference = self.binary_inst(Opcode::Sdiv, InstFlags::EXACT, difference, size);
                }
                let result_ty = self.value_type(ty, source)?;
                let difference = self.extend(difference, true, result_ty);
                return Ok(Item {
                    operand: Operand::Value(difference),
                    ty,
                });
            },
            | B::Addition | B::Subtraction if left_pointer || right_pointer => {
                let (pointer, index) = if left_pointer {
                    (left, right)
                } else {
                    (right, left)
                };
                let address =
                    self.offset_pointer(pointer, index, operator == B::Subtraction, source)?;
                return Ok(Item {
                    operand: Operand::Value(address),
                    ty,
                });
            },
            | _ => {},
        }
        let representation = self.repr(ty, source)?;
        let signed = representation.signed();
        let a = self.value(left, source)?;
        let mut b = self.value(right, source)?;
        if matches!(representation, Repr::Float { .. }) {
            let opcode = match operator {
                | B::Multiplication => Opcode::Fmul,
                | B::Addition => Opcode::Fadd,
                | B::Subtraction => Opcode::Fsub,
                | B::Division => Opcode::Fdiv,
                | _ =>
                    return Err(LoweringError::missing(
                        "a floating operator has no lowering",
                        source,
                    )),
            };
            let value = self.binary_inst(opcode, InstFlags::empty(), a, b);
            return Ok(Item {
                operand: Operand::Value(value),
                ty,
            });
        }
        let wrap = if signed {
            self.signed_wrap()
        } else {
            InstFlags::empty()
        };
        let (opcode, flags) = match operator {
            | B::Multiplication => (Opcode::Imul, wrap),
            | B::Addition => (Opcode::Iadd, wrap),
            | B::Subtraction => (Opcode::Isub, wrap),
            | B::Division if signed => (Opcode::Sdiv, InstFlags::empty()),
            | B::Division => (Opcode::Udiv, InstFlags::empty()),
            | B::Modulo if signed => (Opcode::Srem, InstFlags::empty()),
            | B::Modulo => (Opcode::Urem, InstFlags::empty()),
            | B::BitwiseAnd => (Opcode::And, InstFlags::empty()),
            | B::BitwiseOr => (Opcode::Or, InstFlags::empty()),
            | B::BitwiseXor => (Opcode::Xor, InstFlags::empty()),
            | B::LeftShift | B::RightShift => {
                // Each shift operand is promoted on its own (§6.5.7p3), so
                // the count takes the shifted value's width.
                let count_signed = self.repr(right.ty, source)?.signed();
                b = self.extend(b, count_signed, self.draft.value_type(a));
                match operator {
                    // C99 §6.5.7p4: a signed left shift that overflows is
                    // undefined.
                    | B::LeftShift => (Opcode::Shl, wrap),
                    | _ if signed => (Opcode::Ashr, InstFlags::empty()),
                    | _ => (Opcode::Lshr, InstFlags::empty()),
                }
            },
            | _ =>
                return Err(LoweringError::missing(
                    "a binary operator has no lowering",
                    source,
                )),
        };
        let value = self.binary_inst(opcode, flags, a, b);
        Ok(Item {
            operand: Operand::Value(value),
            ty,
        })
    }

    /// A unary operator on the operand(s) on the stack.
    /// C99: §6.5.3, pp. 78-80; PDF pp. 90-92; §6.5.2.4, p. 75; PDF p. 87.
    fn unary(
        &mut self,
        operator: UnaryOperator,
        info: &ExpressionInfo<'tu>,
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        use UnaryOperator as U;
        // GCC's `+` and `-` are exact: a plain operand's result is plain,
        // widened rather than excess.
        if matches!(operator, U::Plus | U::Minus) && self.complex_half(info.ty) {
            let operand = self.pop_excess();
            let plain = self.plain_half(operand);
            let operand = self.single_operand(operand, true, source)?;
            let mut pair = self.complex_parts(operand, source)?;
            if operator == U::Minus {
                pair = pair.map(|part| self.convert(Opcode::Fneg, Type::F32, part));
            }
            if plain {
                self.widened.push(pair[0]);
            }
            let result = self.complex_pair(pair);
            self.push_item(result, info.ty);
            return Ok(());
        }
        // GCC's `__real__` and `__imag__` read a `_Complex _Float16` excess
        // (C99 §6.2.5p13 and GCC "Complex Numbers").
        if matches!(operator, U::Real | U::Imag) && self.half_in_float() {
            let operand = self.pop_excess();
            if let Some(parts) = self.complex_excess(operand) {
                let part = parts[usize::from(operator == U::Imag)];
                self.push_item(Operand::Excess(part), info.ty);
                return Ok(());
            }
            // And `__real__` of a real excess is that excess.
            if operator == U::Real
                && let Operand::Excess(value) = operand.operand
            {
                self.push_item(Operand::Excess(value), info.ty);
                return Ok(());
            }
            self.operands.push(operand);
        }
        if matches!(operator, U::Plus | U::Minus) && self.half(info.ty).is_some() {
            let operand = self.pop_excess();
            let plain = self.plain_half(operand);
            let value = self.excess(operand, source)?;
            let result = if operator == U::Minus {
                self.convert(Opcode::Fneg, Type::F32, value)
            } else {
                value
            };
            if plain {
                self.widened.push(result);
            }
            self.push_item(Operand::Excess(result), info.ty);
            return Ok(());
        }
        let operand = if operator == U::LogicalNot {
            self.pop_truth()
        } else {
            self.pop()
        };
        let result = match operator {
            | U::AddressOf => match operand.operand {
                | Operand::Place(Place::Memory(address)) => Operand::Value(address),
                | Operand::Place(Place::Function(func)) =>
                    Operand::Value(self.inst(InstData::FuncAddr { func }, Type::Ptr)),
                | _ =>
                    return Err(LoweringError::missing(
                        "an address operand is not in memory",
                        source,
                    )),
            },
            | U::Indirection => {
                let address = self.value(operand, source)?;
                Operand::Place(Place::Memory(address))
            },
            | U::Plus | U::Extension => operand.operand,
            | U::Minus if matches!(operand.operand, Operand::Complex(_)) => {
                let [real, imag] = self.complex_parts(operand, source)?;
                let ty = self.draft.value_type(real);
                let pair = [
                    self.convert(Opcode::Fneg, ty, real),
                    self.convert(Opcode::Fneg, ty, imag),
                ];
                self.complex_pair(pair)
            },
            | U::Minus => {
                let value = self.value(operand, source)?;
                let ty = self.draft.value_type(value);
                if ty.is_float() {
                    let negated = self.convert(Opcode::Fneg, ty, value);
                    self.push_item(Operand::Value(negated), info.ty);
                    return Ok(());
                }
                let zero = self.iconst(ty, 0);
                let flags = if self.repr(info.ty, source)?.signed() {
                    self.signed_wrap()
                } else {
                    InstFlags::empty()
                };
                Operand::Value(self.binary_inst(Opcode::Isub, flags, zero, value))
            },
            | U::BitwiseNot => {
                let value = self.value(operand, source)?;
                let ty = self.draft.value_type(value);
                let ones = self.iconst(ty, -1);
                Operand::Value(self.binary_inst(Opcode::Xor, InstFlags::empty(), value, ones))
            },
            | U::LogicalNot => match operand.operand {
                | Operand::Value(value) if self.draft.value_type(value).is_float() => {
                    let value = if self.draft.value_type(value).half_format().is_some() {
                        self.widen_half(value)
                    } else {
                        value
                    };
                    let ty = self.draft.value_type(value);
                    let zero = self.fconst(ty, 0);
                    Operand::Truth(self.fcmp(FloatCC::Oeq, value, zero))
                },
                | Operand::Value(value) if self.draft.constant(value).is_none() => {
                    let ty = self.draft.value_type(value);
                    let zero = if ty == Type::Ptr {
                        self.nullary(Opcode::Null)
                    } else {
                        self.iconst(ty, 0)
                    };
                    Operand::Truth(self.icmp(IntCC::Eq, value, zero))
                },
                | _ => {
                    let truth = self.truth(operand, source)?;
                    let one = self.iconst(Type::I1, 1);
                    Operand::Truth(self.binary_inst(Opcode::Xor, InstFlags::empty(), truth, one))
                },
            },
            | U::PreIncrement | U::PreDecrement | U::PostIncrement | U::PostDecrement => {
                let place = self.pop();
                let increment = matches!(operator, U::PreIncrement | U::PostIncrement);
                if matches!(self.unit.sema.types.kind(place.ty), TypeKind::Atomic(_)) {
                    let (old, stored) =
                        self.atomic_update(place, operand, source, |this, old| {
                            this.step_value(old, increment, true, source)
                        })?;
                    let result = if matches!(operator, U::PreIncrement | U::PreDecrement) {
                        stored
                    } else {
                        old
                    };
                    self.push_item(result.operand, info.ty);
                    return Ok(());
                }
                let updated = self.step_value(operand, increment, false, source)?;
                let stored = self.write(place, updated, source)?;
                if matches!(operator, U::PreIncrement | U::PreDecrement) {
                    stored.operand
                } else {
                    operand.operand
                }
            },
            | U::Real | U::Imag =>
                self.complex_component(operand, info.ty, operator == U::Imag, source)?,
        };
        self.push_item(result, info.ty);
        Ok(())
    }

    /// The value one more or one less than `item`, as `++` and `--` compute
    /// it: `_Bool` saturates, pointers step by an element, and narrow
    /// integers wrap as the conversion back from `int` does here.
    /// C99: §6.5.2.4 paragraph 2, p. 75; PDF p. 87; §6.5.3.1 paragraph 2,
    /// p. 78; PDF p. 90.
    fn step_value(
        &mut self,
        item: Item,
        increment: bool,
        atomic: bool,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let value = self.value(item, source)?;
        let operand = match self.repr(item.ty, source)? {
            | Repr::Bool if increment => Operand::Value(self.iconst(Type::I8, 1)),
            | Repr::Bool => {
                let one = self.iconst(Type::I8, 1);
                Operand::Value(self.binary_inst(Opcode::Xor, InstFlags::empty(), value, one))
            },
            | Repr::Pointer => {
                let offset = if let Some(size) = self.static_element_size(item.ty) {
                    self.iconst(
                        Type::I64,
                        if increment {
                            i128::from(size)
                        } else {
                            -i128::from(size)
                        },
                    )
                } else {
                    let size = self.element_size(item.ty, source)?;
                    if increment {
                        size
                    } else {
                        let zero = self.iconst(Type::I64, 0);
                        self.binary_inst(Opcode::Isub, InstFlags::empty(), zero, size)
                    }
                };
                Operand::Value(self.ptr_add(value, offset))
            },
            // A 16-bit floating type steps in `float` and rounds once.
            | Repr::Float { ty } if ty.half_format().is_some() => {
                let wide = self.widen_half(value);
                let one = self.fconst(Type::F32, u128::from(1.0_f32.to_bits()));
                let opcode = if increment {
                    Opcode::Fadd
                } else {
                    Opcode::Fsub
                };
                let stepped = self.binary_inst(opcode, InstFlags::empty(), wide, one);
                Operand::Value(self.convert(Opcode::Fptrunc, ty, stepped))
            },
            | Repr::Float { ty } => {
                let bits = floats::from_integer(1, ty)
                    .ok_or_else(|| LoweringError::missing("a floating type has no one", source))?;
                let one = self.fconst(ty, bits);
                let opcode = if increment {
                    Opcode::Fadd
                } else {
                    Opcode::Fsub
                };
                Operand::Value(self.binary_inst(opcode, InstFlags::empty(), value, one))
            },
            | Repr::Int { ty, signed } => {
                let one = self.iconst(ty, 1);
                // Types narrower than int are promoted (§6.3.1.1p2), so their
                // arithmetic cannot overflow; the conversion back wraps.
                let flags = if signed && ty.bits() >= self.int_bits() && !atomic {
                    self.signed_wrap()
                } else {
                    InstFlags::empty()
                };
                let opcode = if increment {
                    Opcode::Iadd
                } else {
                    Opcode::Isub
                };
                Operand::Value(self.binary_inst(opcode, flags, value, one))
            },
            | _ =>
                return Err(LoweringError::missing(
                    "an increment has no lowering",
                    source,
                )),
        };
        Ok(Item {
            operand,
            ty: item.ty,
        })
    }

    /// Snapshots the evaluated argument once, before later arguments can
    /// modify its backing object. The call then consumes this private copy.
    /// C99: §6.5.16p3, p. 91; PDF p. 103; §6.5.2.2p4,p10, pp. 71-72;
    /// PDF pp. 83-84. Argument order is unspecified; lowering chooses left to
    /// right.
    pub(super) fn snapshot_argument(
        &mut self,
        expression: &'tu Expression<'tu>,
    ) -> Result<(), LoweringError> {
        let source = expression.source_vectors;
        let item = self.pop();
        let operand = if self.repr(item.ty, source)? == Repr::Aggregate {
            self.copy_argument(item, source)?
        } else {
            item.operand
        };
        self.push_item(operand, item.ty);
        Ok(())
    }

    /// A call, with its arguments (and an indirect callee) on the stack.
    /// Arguments were converted to the parameter types, or promoted for an
    /// unprototyped callee or the variable part of a variadic one. A call
    /// whose signature differs from the declared function's IR signature,
    /// as through an unprototyped declaration, calls its address with the
    /// call site's signature.
    ///
    /// A structure or union argument is copied into a fresh slot whose
    /// address is passed, since the parameter is a new object the callee
    /// may modify (§6.9.1p10). A structure or union result is stored by the
    /// callee into a fresh slot passed first, which becomes the call's
    /// value.
    ///
    /// A last argument `__builtin_va_arg_pack ()` stands for the anonymous
    /// arguments of the copy being lowered, which are on the stack in its
    /// place, and a direct call of a function using the pack calls its
    /// copy for these arguments (`argument_pack.rs`). GNU extension: GCC
    /// manual, "Constructing Calls".
    /// C99: §6.5.2.2 paragraphs 4-7, pp. 71-72; PDF pp. 83-84.
    fn call(
        &mut self,
        count: usize,
        info: &ExpressionInfo<'tu>,
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        let call = self.call_facts(info.expression)?;
        let mut arguments = call.arguments;
        let mut pack: &[Item] = &[];
        let mut count = count;
        if self.forwarded_pack(call)?.is_some() {
            pack = self.pack.unwrap_or_default();
            arguments = &arguments[..arguments.len() - 1];
            count = count - 1 + pack.len();
        }
        let start = self.operands.len() - count;
        let mut items = ArenaVec::with_capacity_in(count, self.scratch);
        items.extend_from_slice(&self.operands[start..]);
        self.operands.truncate(start);
        for item in &mut items {
            *item = self.settle(*item);
        }
        let direct = self.resolved_direct_callee(call, source)?;
        // `(*f)(...)` of a function using the pack calls its copy too.
        let packed_callee = direct
            .is_none()
            .then(|| self.packed_callee(info.expression))
            .flatten();
        let callee_value = if direct.is_some() || packed_callee.is_some() {
            None
        } else {
            let item = self.pop();
            Some(self.value(item, source)?)
        };
        let function_type = call.function_type.ok_or_else(|| {
            LoweringError::missing("a call has no point-of-use function type", source)
        })?;
        let TypeKind::Function { result, .. } = self
            .unit
            .sema
            .types
            .kind(self.unit.sema.types.non_atomic(function_type))
        else {
            return Err(LoweringError::missing(
                "a call has no function type",
                source,
            ));
        };
        if items.len() != arguments.len() + pack.len()
            || items
                .iter()
                .zip(
                    arguments
                        .iter()
                        .map(|argument| argument.ty)
                        .chain(pack.iter().map(|item| item.ty)),
                )
                .any(|(item, ty)| item.ty != ty)
        {
            return Err(LoweringError::missing(
                "call argument conversions are not retained",
                source,
            ));
        }
        self.mark_subobject_arguments(direct.or(packed_callee), arguments, &mut items, source)?;
        let open_external = direct.is_some_and(|binding| self.unit.unprototyped_external(binding));
        let extension_call = self.unit.sema.function_extensions.call(info.expression);
        let hidden_count = extension_call.map_or(0, |call| call.sizes.len());
        if let Some(extension_call) = extension_call {
            let extension = self
                .unit
                .sema
                .function_extensions
                .function(extension_call.binding);
            if !extension_call.sizes.is_empty() {
                let size_type = extension
                    .ok_or_else(|| {
                        LoweringError::missing("selected function contract is not retained", source)
                    })?
                    .size_type;
                let mut expanded =
                    ArenaVec::with_capacity_in(items.len() + hidden_count, self.scratch);
                for (index, &item) in items.iter().enumerate() {
                    expanded.push(item);
                    if let Some(size) = extension_call
                        .sizes
                        .iter()
                        .find(|size| size.parameter as usize == index)
                    {
                        let pointer = self.value(item, source)?;
                        let value = self.emit_object_size(size.value, Some(pointer), source)?;
                        expanded.push(Item {
                            operand: Operand::Value(value),
                            ty:      size_type,
                        });
                    }
                }
                items = expanded;
            }
        }
        let fixed = if open_external {
            0
        } else {
            call.fixed + hidden_count
        };
        let variadic = open_external || (call.prototype && call.variadic);
        let described = items[fixed..].iter().any(|item| {
            matches!(
                repr(&self.unit.sema.types, item.ty),
                Ok(Repr::Aggregate | Repr::Complex { .. })
            )
        });
        let mut signature = self
            .unit
            .signature_of(
                items[..if described { items.len() } else { fixed }]
                    .iter()
                    .map(|item| item.ty),
                result,
                variadic,
                self.scratch,
            )
            .map_err(|kind| at(kind, source))?;
        if described {
            signature.variadic_fixed = Some(
                u32::try_from(fixed + usize::from(signature.result_param().is_some()))
                    .map_err(|_| LoweringError::missing("too many call arguments", source))?,
            );
            signature.variadic_noundef = false;
        }
        let sig = self.unit.module.intern(signature);
        let mut args = ArenaVec::with_capacity_in(count + 1, self.scratch);
        let mut result_object = None;
        if let Some(ParamAbi::Result(layout)) = signature.result_param() {
            let object = self.fresh_object(layout.size, layout.align, source)?;
            args.push(object);
            result_object = Some(object);
        }
        for &item in &items {
            let value = if let Operand::Complex(_) = item.operand {
                self.materialize_complex(item, source)?
            } else if let Operand::Aggregate(copy) = item.operand {
                copy
            } else {
                self.value(item, source)?
            };
            args.push(value);
        }
        let args = self.scratch.alloc_slice_copy(&args);
        let result_type = result;
        let result = signature.result;
        if let Some(binding) = direct {
            let ExpressionType::Call {
                function_expression: mut callee,
                ..
            } = info.expression.kind
            else {
                unreachable!("call reduction has call syntax")
            };
            loop {
                callee = match callee.kind {
                    | ExpressionType::Parenthesized { expression } => expression,
                    | ExpressionType::Unary {
                        operator: UnaryOperator::Extension,
                        operand_expression,
                    } => operand_expression,
                    | ExpressionType::Generic(_) | ExpressionType::Builtin(_) => {
                        let Some(selected) = self.info(callee)?.selected_expression else {
                            break;
                        };
                        selected
                    },
                    | _ => break,
                };
            }
            if matches!(callee.kind, ExpressionType::Identifier(_))
                && let Some(diagnostic) = self
                    .unit
                    .sema
                    .entity_attributes(binding)
                    .and_then(|a| a.call_diagnostic)
            {
                let diagnostic = self.scratch.alloc(super::CallDiagnostic {
                    message: diagnostic.message,
                    declaration: diagnostic.source,
                    source,
                    warning: diagnostic.warning,
                });
                _ = self.draft.push(Op::Diagnostic(diagnostic), None);
            }
        }
        let packed = direct
            .filter(|_| !open_external)
            .or(packed_callee)
            .and_then(|binding| Some((binding, self.unit.pack_function(binding)?)));
        let value = match (direct, callee_value) {
            | _ if let Some((binding, record)) = packed => {
                let types = self
                    .scratch
                    .alloc_slice_fill_iter(items.iter().map(|item| item.ty));
                let func = self
                    .unit
                    .pack_copy(binding, record, types, fixed, result_type, source)
                    .map_err(|kind| at(kind, source))?;
                self.draft.push(Op::Call(func, args), result)
            },
            | (Some(binding), _) => {
                let func = self
                    .unit
                    .direct_callee(binding)
                    .map_err(|kind| at(kind, source))?;
                if self.unit.module.function(func).signature == sig {
                    self.draft.push(Op::Call(func, args), result)
                } else {
                    let address = self.inst(InstData::FuncAddr { func }, Type::Ptr);
                    self.draft
                        .push(Op::CallIndirect(sig, address, args), result)
                }
            },
            | (None, Some(pointer)) => self
                .draft
                .push(Op::CallIndirect(sig, pointer, args), result),
            | (None, None) => unreachable!("an indirect callee was popped"),
        };
        let operand = match (result_object, value) {
            | (Some(object), _) if matches!(self.repr(info.ty, source)?, Repr::Complex { .. }) =>
                self.load_complex(object, info.ty, source)?,
            | (Some(object), _) => Operand::Aggregate(object),
            | (None, Some(value)) => Operand::Value(value),
            | (None, None) => Operand::Void,
        };
        self.push_item(operand, info.ty);
        Ok(())
    }

    /// Passes each pointer argument of a call to an `always_inline` function
    /// (or an argument pack copy, which is one) through a `subobject` that
    /// records the closest surrounding subobject it designates, as semantic
    /// analysis found it (`FunctionExtensions::subobject_argument`): once
    /// the call is expanded, the callee's `object_size` queries of modes 1
    /// and 3 find it, as GCC finds the member an inlined argument names.
    /// An argument that is this function's own parameter offset by pointer
    /// arithmetic needs none: the hint its caller passed still holds
    /// ([`Self::subobject_hints_pass`]). Any other gets a hint that the
    /// subobject is unknown, so no hint from further out, which a member
    /// access here may have narrowed, answers for it.
    /// GNU extension: GCC manual, "Object Size Checking".
    fn mark_subobject_arguments(
        &mut self,
        callee: Option<usize>,
        arguments: &[ArgumentFacts],
        items: &mut [Item],
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        let sema = self.unit.sema;
        if !callee.is_some_and(|binding| self.unit.hints_subobjects(binding)) {
            return Ok(());
        }
        for (item, argument) in items.iter_mut().zip(arguments) {
            if !matches!(
                sema.types.kind(sema.types.non_atomic(argument.ty)),
                TypeKind::Pointer(_)
            ) {
                continue;
            }
            let fact = sema
                .expressions
                .get(argument.expression.0)
                .and_then(|info| sema.function_extensions.subobject_argument(info.expression));
            let size = match fact {
                | Some(SubobjectArgument::Size(size)) => size,
                | Some(SubobjectArgument::Through(parameter))
                    if self.subobject_hints_pass(parameter) =>
                    continue,
                | _ => u64::MAX,
            };
            let pointer = self.value(*item, source)?;
            let size = self.iconst(Type::I64, i128::from(size));
            let marked = self.inst(
                InstData::Binary {
                    opcode: Opcode::Subobject,
                    ty:     Type::Ptr,
                    flags:  InstFlags::empty(),
                    args:   [pointer, size],
                },
                Type::Ptr,
            );
            item.operand = Operand::Value(marked);
        }
        Ok(())
    }

    /// Whether a `subobject` hint the caller passed for `parameter` still
    /// names the closest subobject of a pointer this function computes from
    /// it by pointer arithmetic: the function is one whose calls carry hints
    /// for every pointer argument (`always_inline`, or an argument pack
    /// copy), and nothing writes the parameter, directly, through its
    /// address, or in inline assembly. Elsewhere, and for a written
    /// parameter, an inlined call could reach a hint from further out
    /// across a narrowing member access.
    pub(super) fn subobject_hints_pass(&self, parameter: usize) -> bool {
        !self.record.assembly
            && self
                .record
                .binding
                .is_some_and(|binding| self.unit.hints_subobjects(binding))
            && !self
                .unit
                .sema
                .function_extensions
                .parameter_written(parameter)
    }

    /// A copy of the structure or union argument `item` in a fresh slot,
    /// which the callee receives as its parameter object.
    /// C99: §6.9.1 paragraph 10, p. 142; PDF p. 154.
    pub(super) fn copy_argument(
        &mut self,
        item: Item,
        source: SourceVectors,
    ) -> Result<Operand, LoweringError> {
        let layout =
            types::layout(&self.unit.sema.types, item.ty).map_err(|kind| at(kind, source))?;
        let align = super::alignment::storage_alignment(&self.unit.sema.types, item.ty)
            .map_err(|kind| at(kind, source))?;
        let (from, source_align) = match item.operand {
            | Operand::Aggregate(from) => (from, self.place_alignment(from, Align::BYTE)),
            | Operand::Place(Place::Memory(from)) | Operand::Value(from) => (from, align),
            | _ =>
                return Err(LoweringError::missing(
                    "a structure argument has no address",
                    source,
                )),
        };
        let copy = self.fresh_object(layout.size, align, source)?;
        let size = self.iconst(Type::I64, i128::from(layout.size));
        _ = self.draft.push(
            self.memory_op(InstData::MemoryRange {
                opcode: Opcode::Copy,
                flags:  InstFlags::empty(),
                align:  source_align,
                args:   [copy, from, size],
            }),
            None,
        );
        Ok(Operand::Aggregate(copy))
    }

    fn call_facts(&self, call: &'tu Expression<'tu>) -> Result<CallFacts<'tu>, LoweringError> {
        self.unit.sema.call_facts(call).ok_or_else(|| {
            LoweringError::missing("call facts are not retained", call.source_vectors)
        })
    }

    /// Backend capability checks use the retained callee classification.
    /// A returns-twice callee needs nothing here: declaring it marks
    /// its IR declaration from the retained fact. The builtin spelling
    /// contract moves to M4/M7.
    fn resolved_direct_callee(
        &self,
        call: CallFacts<'tu>,
        source: SourceVectors,
    ) -> Result<Option<usize>, LoweringError> {
        let (Callee::Direct { binding, entity }
        | Callee::Builtin {
            binding: Some(binding),
            direct: Some(entity),
            ..
        }) = call.callee
        else {
            return Ok(None);
        };
        let data = self.unit.sema.bindings[binding];
        debug_assert_eq!(
            data.name.name, self.unit.sema.bindings[entity.0].name.name,
            "direct calls retain their binding entity"
        );
        if data.origin.is_intrinsic() && data.origin.library().is_none() {
            return Err(LoweringError::unsupported(Construct::Builtin, source));
        }
        Ok(Some(binding))
    }

    /// `pointer` moved by `index` elements, backwards if `subtract`.
    /// C99: §6.5.6 paragraph 8, p. 83; PDF p. 95.
    fn offset_pointer(
        &mut self,
        pointer: Item,
        index: Item,
        subtract: bool,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let base = self.value(pointer, source)?;
        let signed = self.repr(index.ty, source)?.signed();
        let index = self.value(index, source)?;
        if let Some(size) = self.static_element_size(pointer.ty) {
            let stride = size;
            let size = i128::from(size);
            let offset = if let Some(constant) = self.draft.constant(index) {
                let constant =
                    self.cast_constant(constant, self.draft.value_type(index), signed, Type::I64);
                let scaled = if subtract {
                    -constant * size
                } else {
                    constant * size
                };
                if scaled == 0 {
                    return Ok(base);
                }
                self.iconst(Type::I64, scaled)
            } else {
                let wide = self.extend(index, signed, Type::I64);
                let scaled = if size == 1 {
                    wide
                } else {
                    let size = self.iconst(Type::I64, size);
                    self.binary_inst(Opcode::Imul, InstFlags::NSW, wide, size)
                };
                if subtract {
                    let zero = self.iconst(Type::I64, 0);
                    self.binary_inst(Opcode::Isub, InstFlags::NSW, zero, scaled)
                } else {
                    scaled
                }
            };
            return self.indexed_address(base, offset, pointer.ty, Some(stride), source);
        }
        let size = self.element_size(pointer.ty, source)?;
        let wide = self.extend(index, signed, Type::I64);
        let scaled = self.binary_inst(Opcode::Imul, InstFlags::NSW, wide, size);
        let offset = if subtract {
            let zero = self.iconst(Type::I64, 0);
            self.binary_inst(Opcode::Isub, InstFlags::NSW, zero, scaled)
        } else {
            scaled
        };
        self.indexed_address(base, offset, pointer.ty, None, source)
    }

    /// A pointer comparison operand: a pointer, or an integer (in practice
    /// a null pointer constant) converted to one.
    /// C99: §6.5.9 paragraph 5, p. 87; PDF p. 99.
    fn pointer_operand(
        &mut self,
        item: Item,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let value = self.value(item, source)?;
        if self.draft.value_type(value) == Type::Ptr {
            return Ok(value);
        }
        let signed = self.repr(item.ty, source)?.signed();
        Ok(self.int_to_pointer(value, signed))
    }

    fn int_to_pointer(&mut self, value: Value, signed: bool) -> Value {
        if self.draft.constant(value) == Some(0) {
            return self.nullary(Opcode::Null);
        }
        let wide = self.extend(value, signed, Type::I64);
        self.convert(Opcode::Inttoptr, Type::Ptr, wide)
    }

    /// The size of the element a pointer or array type designates; 1 for
    /// `void`, as GNU pointer arithmetic counts it.
    /// GNU extension: GCC "Pointer Arith".
    fn static_element_size(&self, pointer: TypeId) -> Option<u64> {
        let element = types::target_type(&self.unit.sema.types, pointer)?;
        types::arithmetic_size(&self.unit.sema.types, element)
    }

    /// The saved runtime stride for a variably modified pointer or array.
    fn element_size(
        &mut self,
        pointer: TypeId,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let element = types::target_type(&self.unit.sema.types, pointer)
            .ok_or_else(|| LoweringError::missing("pointer arithmetic on a non-pointer", source))?;
        self.runtime_size(element, source)
    }

    fn merge_params(
        &self,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<&'static [Type], LoweringError> {
        Ok(match self.repr(ty, source)? {
            | Repr::Void => &[],
            | Repr::Aggregate | Repr::Pointer => &[Type::Ptr],
            | Repr::Bool => &[Type::I8],
            // GCC's `?:` keeps a `_Complex _Float16` excess in `f32` parts.
            | Repr::Complex {
                component: Type::F16,
            } if self.half_in_float() => &[Type::F32, Type::F32],
            | Repr::Complex {
                component: Type::F16,
            } => &[Type::F16, Type::F16],
            | Repr::Complex {
                component: Type::F32,
            } => &[Type::F32, Type::F32],
            | Repr::Complex {
                component: Type::F64,
            } => &[Type::F64, Type::F64],
            | Repr::Complex {
                component: Type::F128,
            } => &[Type::F128, Type::F128],
            | Repr::Complex { .. } => &[Type::F80, Type::F80],
            | Repr::Float { ty } => match ty {
                | Type::F16 | Type::BF16 if self.half_in_float() => &[Type::F32],
                | Type::F16 => &[Type::F16],
                | Type::BF16 => &[Type::BF16],
                | Type::F32 => &[Type::F32],
                | Type::F64 => &[Type::F64],
                | Type::F128 => &[Type::F128],
                | _ => &[Type::F80],
            },
            | Repr::Int { ty, .. } => match ty {
                | Type::I8 => &[Type::I8],
                | Type::I16 => &[Type::I16],
                | Type::I32 => &[Type::I32],
                | Type::I64 => &[Type::I64],
                | _ => &[Type::I128],
            },
            | Repr::Function =>
                return Err(LoweringError::missing(
                    "a conditional yields a function",
                    source,
                )),
        })
    }
}

/// The construct an expression form is, if lowering does not support it.
fn unsupported(expression: &Expression<'_>) -> Option<Construct> {
    Some(match expression.kind {
        | ExpressionType::CompoundLiteral { type_name, .. } if literal_storage(type_name, true) =>
            Construct::CompoundLiteralStorage,
        | ExpressionType::Builtin(b) if !matches!(b.keyword,crate::translation_phases::preprocessing::KeywordTokenType::BuiltinVaArg|crate::translation_phases::preprocessing::KeywordTokenType::BuiltinVaStart|crate::translation_phases::preprocessing::KeywordTokenType::BuiltinVaEnd|crate::translation_phases::preprocessing::KeywordTokenType::BuiltinVaCopy|crate::translation_phases::preprocessing::KeywordTokenType::BuiltinChooseExpr|crate::translation_phases::preprocessing::KeywordTokenType::BuiltinTypesCompatible|crate::translation_phases::preprocessing::KeywordTokenType::BuiltinOffsetof) => Construct::Builtin,
        | ExpressionType::Countof(_) => Construct::Extension,
        | _ => return None,
    })
}

/// Whether a compound literal's storage-class specifiers give it a storage
/// duration lowering does not allocate: `thread_local` anywhere, or
/// `static` in a function, whose object outlives the block. Lowering refuses
/// these by name rather than allocating them in the frame.
/// C23 (N3220): §6.5.3.6 paragraph 7, p. 79; PDF p. 92.
pub(super) fn literal_storage(
    type_name: &crate::translation_phases::parsing::declaration_syntax::TypeName<'_>,
    in_function: bool,
) -> bool {
    let specifiers = type_name.declaration_specifiers;
    let mut extension = specifiers.extensions;
    while let Some(item) = extension {
        if matches!(
            item.kind,
            crate::translation_phases::parsing::SpecifierExtensionKind::ThreadLocal(_)
        ) {
            return true;
        }
        extension = item.next;
    }
    in_function
        && specifiers.storage_class
            == Some(crate::translation_phases::parsing::syntax::StorageClass::Static)
}

/// The IR comparison of a C relational or equality operator, signed form.
pub(super) fn comparison(operator: BinaryOperator) -> Option<IntCC> {
    Some(match operator {
        | BinaryOperator::LessThan => IntCC::Slt,
        | BinaryOperator::GreaterThan => IntCC::Sgt,
        | BinaryOperator::LessThanOrEqual => IntCC::Sle,
        | BinaryOperator::GreaterThanOrEqual => IntCC::Sge,
        | BinaryOperator::Equal => IntCC::Eq,
        | BinaryOperator::NotEqual => IntCC::Ne,
        | _ => return None,
    })
}

/// The IR comparison of a floating relational or equality operator, given
/// as its signed integer form: ordered, except `!=`.
pub(super) fn float_comparison(cc: IntCC) -> FloatCC {
    match cc {
        | IntCC::Slt => FloatCC::Olt,
        | IntCC::Sgt => FloatCC::Ogt,
        | IntCC::Sle => FloatCC::Ole,
        | IntCC::Sge => FloatCC::Oge,
        | IntCC::Eq => FloatCC::Oeq,
        | _ => FloatCC::Une,
    }
}

fn unsigned(cc: IntCC) -> IntCC {
    match cc {
        | IntCC::Slt => IntCC::Ult,
        | IntCC::Sgt => IntCC::Ugt,
        | IntCC::Sle => IntCC::Ule,
        | IntCC::Sge => IntCC::Uge,
        | other => other,
    }
}

/// The operator a compound assignment applies.
/// C99: §6.5.16.2, p. 93; PDF p. 105.
pub(super) fn compound_operator(operator: BinaryOperator) -> Option<BinaryOperator> {
    use BinaryOperator as B;
    Some(match operator {
        | B::MultiplicationAssignment => B::Multiplication,
        | B::DivisionAssignment => B::Division,
        | B::ModuloAssignment => B::Modulo,
        | B::AdditionAssignment => B::Addition,
        | B::SubtractionAssignment => B::Subtraction,
        | B::LeftShiftAssignment => B::LeftShift,
        | B::RightShiftAssignment => B::RightShift,
        | B::BitwiseAndAssignment => B::BitwiseAnd,
        | B::BitwiseXorAssignment => B::BitwiseXor,
        | B::BitwiseOrAssignment => B::BitwiseOr,
        | _ => return None,
    })
}

/// The part of an operator lowering still emits after semantic analysis
/// elided an operand ([`FunctionLowerer::folded`]).
#[derive(Clone, Copy)]
#[expect(
    variant_size_differences,
    reason = "A live operand is a syntax reference; a short circuit is its truth"
)]
pub(super) enum Folded<'tu> {
    /// The operand that stands for the whole expression.
    Live(&'tu Expression<'tu>),
    /// The short-circuit truth of a folded `&&` or `||`.
    Truth(bool),
}
