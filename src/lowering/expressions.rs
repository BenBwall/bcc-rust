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
            BindingKind,
            ConversionKind,
            ExpressionInfo,
            TypeId,
            TypeKind,
            library_builtin_name,
            named_builtin,
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
        if self.unit.sema.types.unanalyzed(info.ty) {
            return Err(LoweringError::unanalyzed(source));
        }
        // An integer constant expression has an exact value (§6.6p6).
        if info.ice
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
        // A folded floating constant has exact bits (§6.6 paragraph 8).
        if let Some(folded) = info.floating
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
                    | UnaryOperator::AddressOf =>
                        self.tasks.push(Task::EvaluateRaw(operand_expression)),
                    | UnaryOperator::PreIncrement
                    | UnaryOperator::PreDecrement
                    | UnaryOperator::PostIncrement
                    | UnaryOperator::PostDecrement => {
                        self.tasks.push(Task::ReadForUpdate(operand_expression));
                        self.tasks.push(Task::EvaluateRaw(operand_expression));
                    },
                    | UnaryOperator::Real | UnaryOperator::Imag =>
                        return Err(LoweringError::unsupported(
                            Construct::ComplexArithmetic,
                            source,
                        )),
                    | _ => self.tasks.push(Task::Evaluate(operand_expression)),
                }
            },
            | E::Call {
                function_expression,
                arguments,
            } => {
                if self.schedule_builtin(expression, function_expression, arguments) {
                    return Ok(());
                }
                self.tasks.push(Task::Finish(expression));
                for &argument in arguments.iter().rev() {
                    self.tasks.push(Task::SnapshotArgument(argument));
                    self.tasks.push(Task::Evaluate(argument));
                }
                if self.direct_callee(function_expression)?.is_none() {
                    self.tasks.push(Task::Evaluate(function_expression));
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
            | E::SizeofType(_) | E::SizeofExpr(_) | E::AlignofType(_) | E::AlignofExpr(_) =>
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
            | E::StatementExpression(statement) => self.statement_expression(statement, info.ty),
            | E::LabelAddress(_) | E::Countof(_) | E::Boolean(_) | E::Nullptr | E::Error =>
                return Err(LoweringError::missing(
                    "an expression was not parsed",
                    source,
                )),
        }
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
                let right = self.pop();
                let operation_type = info.operation_type.ok_or_else(|| {
                    LoweringError::missing("a compound assignment has no type", source)
                })?;
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
                let right = self.pop();
                let left = self.pop();
                let result = self.binary(operator, left, right, info.ty, source)?;
                self.operands.push(result);
            },
            | E::Unary { operator, .. } => self.unary(operator, info, source)?,
            | E::Call {
                function_expression,
                arguments,
            } => self.call(function_expression, arguments.len(), info, source)?,
            | E::DirectMember { .. } | E::IndirectMember { .. } => {
                let base = self.pop();
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
                let place = if let Some(width) = field.width {
                    let bits = BitFieldBits {
                        bit_offset: field.bit_offset,
                        width,
                    };
                    let bit_field = self.bit_field(field.ty, field.offset, bits, source)?;
                    Place::BitField(self.offset_address(base, field.offset), bit_field)
                } else {
                    Place::Memory(self.offset_address(base, field.offset))
                };
                if rvalue {
                    // A member of a structure rvalue is not an lvalue
                    // (§6.5.2.3p3), so it is read at once.
                    let item = self.read(place, info.ty, info.ty.unqualified(), source)?;
                    self.operands.push(item);
                } else {
                    self.push_item(Operand::Place(place), info.ty);
                }
            },
            | E::Cast { .. } => {
                let item = self.pop();
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
        let mut item = self.pop();
        for conversion in self.conversions(expression).iter().skip(skip) {
            if self.unit.sema.types.unanalyzed(conversion.ty) {
                return Err(LoweringError::unanalyzed(source));
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
                | ConversionKind::Arithmetic
                | ConversionKind::Assignment
                | ConversionKind::DefaultArgument =>
                    self.convert_value(item, conversion.ty, source)?,
            };
        }
        self.operands.push(item);
        Ok(())
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
        let types = &self.unit.sema.types;
        let target = repr(types, to).map_err(|kind| at(kind, source))?;
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
    fn float_from(
        &mut self,
        value: Value,
        from: Repr,
        to: Type,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        Ok(match from {
            | Repr::Float { ty } if ty == to => value,
            | Repr::Float { ty } if ty.bits() < to.bits() => self.convert(Opcode::Fpext, to, value),
            | Repr::Float { .. } => self.convert(Opcode::Fptrunc, to, value),
            | Repr::Int { .. } | Repr::Bool => {
                let signed = from.signed();
                let ty = self.draft.value_type(value);
                if let Some(constant) = self.draft.constant(value)
                    && (signed || ty != Type::I128)
                {
                    let exact = self.cast_constant(constant, ty, signed, Type::I128);
                    if let Some(bits) = floats::from_integer(exact, to) {
                        return Ok(self.fconst(to, bits));
                    }
                }
                let opcode = if signed {
                    Opcode::Sitofp
                } else {
                    Opcode::Uitofp
                };
                self.convert(opcode, to, value)
            },
            | _ =>
                return Err(LoweringError::missing(
                    "a conversion to a floating type has no lowering",
                    source,
                )),
        })
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
            | Operand::Value(value) => value,
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

    /// Evaluates a condition, branching straight on `&&`, `||` and `!`
    /// without materializing their `int` results.
    /// C99: §6.5.13-§6.5.14, pp. 89-90; PDF pp. 101-102.
    pub(super) fn condition(
        &mut self,
        expression: &'tu Expression<'tu>,
        then: Block,
        otherwise: Block,
    ) -> Result<(), LoweringError> {
        let folded = self.info(expression)?.integer.is_some();
        if self.conversions(expression).is_empty() && !folded {
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
                | _ => {},
            }
        }
        self.tasks.push(Task::Branch { then, otherwise });
        self.tasks.push(Task::Evaluate(expression));
        Ok(())
    }

    /// Pops a scalar and branches on it; a constant condition jumps.
    pub(super) fn branch(&mut self, then: Block, otherwise: Block) -> Result<(), LoweringError> {
        let item = self.pop();
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
        let item = self.pop();
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
        let item = self.pop();
        let source = SourceVectors::empty();
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
        let operand = match self.draft.block_params(merge).first() {
            | None => Operand::Void,
            | Some(&param) if truth => Operand::Truth(param),
            | Some(&param)
                if self.draft.value_type(param) == Type::Ptr
                    && repr(&self.unit.sema.types, ty) == Ok(Repr::Aggregate) =>
                Operand::Aggregate(param),
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
                let size = self.element_size(left.ty, source)?;
                if size != 1 {
                    let size = self.iconst(Type::I64, i128::from(size));
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
            InstFlags::NSW
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
        let operand = self.pop();
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
                    InstFlags::NSW
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
                let updated = self.step_value(operand, increment, source)?;
                let stored = self.write(place, updated, source)?;
                if matches!(operator, U::PreIncrement | U::PreDecrement) {
                    stored.operand
                } else {
                    operand.operand
                }
            },
            | U::Real | U::Imag =>
                return Err(LoweringError::unsupported(
                    Construct::ComplexArithmetic,
                    source,
                )),
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
                let size = self.element_size(item.ty, source)?;
                let offset = self.iconst(
                    Type::I64,
                    if increment {
                        i128::from(size)
                    } else {
                        -i128::from(size)
                    },
                );
                Operand::Value(self.ptr_add(value, offset))
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
                let flags = if signed && ty.bits() >= self.int_bits() {
                    InstFlags::NSW
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
            Operand::Aggregate(self.copy_argument(item, source)?)
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
    /// C99: §6.5.2.2 paragraphs 4-7, pp. 71-72; PDF pp. 83-84.
    fn call(
        &mut self,
        callee: &'tu Expression<'tu>,
        count: usize,
        info: &ExpressionInfo<'tu>,
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        let start = self.operands.len() - count;
        let mut items = ArenaVec::with_capacity_in(count, self.scratch);
        items.extend_from_slice(&self.operands[start..]);
        self.operands.truncate(start);
        let direct = self.direct_callee(callee)?;
        let (function_type, callee_value) = match direct {
            | Some(binding) => (self.unit.sema.bindings[binding].ty, None),
            | None => {
                let item = self.pop();
                let pointer = self.value(item, source)?;
                let target = types::target_type(&self.unit.sema.types, item.ty)
                    .ok_or_else(|| LoweringError::missing("a callee is not a pointer", source))?;
                (target, Some(pointer))
            },
        };
        let TypeKind::Function {
            result,
            parameters,
            prototype,
            variadic,
        } = self.unit.sema.types.kind(function_type)
        else {
            return Err(LoweringError::missing(
                "a callee has no function type",
                source,
            ));
        };
        let open_external = direct.is_some_and(|binding| self.unit.unprototyped_external(binding));
        let fixed = if open_external {
            0
        } else if prototype {
            parameters.len().min(items.len())
        } else {
            items.len()
        };
        let variadic = open_external || (prototype && variadic);
        for item in &items[fixed..] {
            if self.repr(item.ty, source)? == Repr::Aggregate {
                return Err(LoweringError::unsupported(
                    Construct::VariadicAggregate,
                    source,
                ));
            }
        }
        let signature = self
            .unit
            .signature_of(
                items[..fixed].iter().map(|item| item.ty),
                result,
                variadic,
                self.scratch,
            )
            .map_err(|kind| at(kind, source))?;
        let sig = self.unit.module.intern(signature);
        let mut args = ArenaVec::with_capacity_in(count + 1, self.scratch);
        let mut result_object = None;
        if let Some(ParamAbi::Result(layout)) = signature.result_param() {
            let object = self.fresh_object(layout.size, layout.align, source)?;
            args.push(object);
            result_object = Some(object);
        }
        for &item in &items {
            let value = if let Operand::Aggregate(copy) = item.operand {
                copy
            } else {
                self.value(item, source)?
            };
            args.push(value);
        }
        let args = self.scratch.alloc_slice_copy(&args);
        let result = signature.result;
        let value = match (direct, callee_value) {
            | (Some(binding), _) => {
                let func = self
                    .unit
                    .function(binding)
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
            | (Some(object), _) => Operand::Aggregate(object),
            | (None, Some(value)) => Operand::Value(value),
            | (None, None) => Operand::Void,
        };
        self.push_item(operand, info.ty);
        Ok(())
    }

    /// A copy of the structure or union argument `item` in a fresh slot,
    /// which the callee receives as its parameter object.
    /// C99: §6.9.1 paragraph 10, p. 142; PDF p. 154.
    fn copy_argument(&mut self, item: Item, source: SourceVectors) -> Result<Value, LoweringError> {
        let (Operand::Aggregate(from) | Operand::Place(Place::Memory(from)) | Operand::Value(from)) =
            item.operand
        else {
            return Err(LoweringError::missing(
                "a structure argument has no address",
                source,
            ));
        };
        let layout =
            types::layout(&self.unit.sema.types, item.ty).map_err(|kind| at(kind, source))?;
        let align = types::align(layout);
        let copy = self.fresh_object(layout.size, align, source)?;
        let size = self.iconst(Type::I64, i128::from(layout.size));
        _ = self.draft.push(
            Op::Inst(InstData::MemoryRange {
                opcode: Opcode::Copy,
                flags: InstFlags::empty(),
                align,
                args: [copy, from, size],
            }),
            None,
        );
        Ok(copy)
    }

    /// The address of a fresh stack slot of `size` bytes.
    pub(super) fn fresh_object(
        &mut self,
        size: u64,
        align: Align,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let size = u32::try_from(size).map_err(|_| {
            LoweringError::missing("an automatic object is larger than 4 GiB", source)
        })?;
        let slot = self.draft.create_slot(size, align);
        Ok(self.inst(InstData::StackAddr { slot }, Type::Ptr))
    }

    /// The function binding a callee names directly, if it is an
    /// identifier (in parentheses or not) that decays to its function.
    fn direct_callee(&self, callee: &'tu Expression<'tu>) -> Result<Option<usize>, LoweringError> {
        let conversions = self.conversions(callee);
        if conversions.len() != 1 || conversions[0].kind != ConversionKind::FunctionDecay {
            return Ok(None);
        }
        let mut inner = callee;
        loop {
            match inner.kind {
                | ExpressionType::Parenthesized { expression }
                | ExpressionType::Unary {
                    operator: UnaryOperator::Extension,
                    operand_expression: expression,
                } => {
                    if !self.conversions(expression).is_empty() {
                        return Ok(None);
                    }
                    inner = expression;
                },
                | ExpressionType::Identifier(name) => {
                    let Some(binding) = self.info(inner)?.binding else {
                        return Ok(None);
                    };
                    if self.unit.sema.bindings[binding].kind != BindingKind::Function {
                        return Ok(None);
                    }
                    // A function that returns twice needs facts the IR does
                    // not carry yet (C99 §7.13).
                    let text = self.unit.context.string_cache.at(name.name);
                    if matches!(
                        text,
                        "setjmp" | "_setjmp" | "sigsetjmp" | "__sigsetjmp" | "vfork"
                    ) {
                        return Err(LoweringError::unsupported(
                            Construct::Setjmp,
                            callee.source_vectors,
                        ));
                    }
                    // A typed compiler builtin is an operation, not a symbol
                    // to call.
                    if (text.starts_with("__builtin_") || named_builtin(text))
                        && library_builtin_name(text).is_none()
                    {
                        return Err(LoweringError::unsupported(
                            Construct::Builtin,
                            callee.source_vectors,
                        ));
                    }
                    return Ok(Some(binding));
                },
                | _ => return Ok(None),
            }
        }
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
        let size = i128::from(self.element_size(pointer.ty, source)?);
        let signed = self.repr(index.ty, source)?.signed();
        let index = self.value(index, source)?;
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
        Ok(self.ptr_add(base, offset))
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

    /// The size of the element a pointer or array type designates.
    fn element_size(&self, pointer: TypeId, source: SourceVectors) -> Result<u64, LoweringError> {
        let types = &self.unit.sema.types;
        let element = types::target_type(types, pointer)
            .ok_or_else(|| LoweringError::missing("pointer arithmetic on a non-pointer", source))?;
        Ok(types::layout(types, element)
            .map_err(|kind| at(kind, source))?
            .size)
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
            | Repr::Float { ty } => match ty {
                | Type::F32 => &[Type::F32],
                | Type::F64 => &[Type::F64],
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
        | ExpressionType::Builtin(b) if !matches!(b.keyword,crate::translation_phases::preprocessing::KeywordTokenType::BuiltinVaArg|crate::translation_phases::preprocessing::KeywordTokenType::BuiltinVaStart|crate::translation_phases::preprocessing::KeywordTokenType::BuiltinVaEnd|crate::translation_phases::preprocessing::KeywordTokenType::BuiltinVaCopy|crate::translation_phases::preprocessing::KeywordTokenType::BuiltinChooseExpr|crate::translation_phases::preprocessing::KeywordTokenType::BuiltinTypesCompatible|crate::translation_phases::preprocessing::KeywordTokenType::BuiltinOffsetof) => Construct::Builtin,
        | ExpressionType::LabelAddress(_) => Construct::LabelAddress,
        | ExpressionType::Countof(_) | ExpressionType::Nullptr => Construct::Extension,
        | _ => return None,
    })
}

/// The IR comparison of a C relational or equality operator, signed form.
fn comparison(operator: BinaryOperator) -> Option<IntCC> {
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
fn float_comparison(cc: IntCC) -> FloatCC {
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
