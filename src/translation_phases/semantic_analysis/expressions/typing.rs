//! Phase-7 operator types, value conversions and expression-result storage.
//! The entry file reduces expressions after their children; these methods check
//! individual operators without traversing child syntax recursively. Statement
//! control flow and initializer subobjects are checked by their own analyzers.
//! C99: §6.3, pp. 42-48; PDF pp. 54-60;
//! §6.5, pp. 67-94; PDF pp. 79-106.

use super::{
    ConstantClass,
    ConstantFolding,
    Conversion,
    ConversionKind,
    ExpressionInfo,
    ValueCategory,
};
use crate::{
    translation_phases::{
        parsing::{
            declaration_syntax::TypeQualifiers,
            syntax::{
                BinaryOperator,
                ConditionalExpression,
                Constant,
                Expression,
                ExpressionSlot,
                ExpressionType,
                Identifier,
                UnaryOperator,
            },
        },
        preprocessing::{
            FloatTokenType,
            LiteralUnit,
            StringTokenType,
        },
        semantic_analysis::{
            Analyzer,
            BindingKind,
            Duration,
            Linkage,
            SemanticErrorKind,
            integer::Integer,
            scopes::Namespace,
            types::{
                Scalar,
                TagKind,
                TypeId,
                TypeKind,
            },
        },
    },
    util::{
        arena_list::ArenaList,
        bump::ArenaVec,
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// C99: §6.5.3.1-§6.5.3.3, pp. 78-80; PDF pp. 90-92;
    /// postfix increments §6.5.2.4, p. 75; PDF p. 87.
    pub(in crate::translation_phases::semantic_analysis) fn type_unary(
        &mut self,
        e: &'tu Expression<'tu>,
        op: UnaryOperator,
        operand: ExpressionInfo<'tu>,
    ) -> ExpressionInfo<'tu> {
        use UnaryOperator as U;
        if self.types.unanalyzed(operand.ty) {
            return Self::expression_result(e, self.types.unknown());
        }
        if op == U::Extension {
            return ExpressionInfo {
                expression: e,
                ..operand
            };
        }
        if op == U::AddressOf {
            if operand.vector_element {
                return self.vector_error(e);
            }
            let indirect = matches!(
                operand.expression.kind,
                ExpressionType::Unary {
                    operator: U::Indirection,
                    ..
                } | ExpressionType::Binary {
                    operator: BinaryOperator::Subscript,
                    ..
                }
            );
            if operand.bit_field.is_some()
                || operand.register
                || (!indirect
                    && !matches!(
                        operand.category,
                        ValueCategory::Lvalue
                            | ValueCategory::ModifiableLvalue
                            | ValueCategory::FunctionDesignator
                    ))
            {
                return self.invalid_expression(e, SemanticErrorKind::InvalidAddressOperand);
            }
            let ty = self.types.intern(TypeKind::Pointer(operand.ty));
            let mut info = Self::expression_result(e, ty);
            if operand.static_address {
                info.constant = ConstantClass::Address;
                info.integer = self
                    .integer_address(operand.expression)
                    .map(|value| Integer::int(value).cast(64, false));
            }
            return info;
        }
        let ty = self.converted(operand);
        if let Some((element, _, _)) = self.vector(ty) {
            if matches!(op, U::Plus | U::Minus)
                || (op == U::BitwiseNot && self.integer_type(element).is_some())
            {
                return Self::expression_result(e, ty);
            }
            return self.vector_error(e);
        }
        if op == U::Indirection {
            let Some(target) = self.pointer_target(ty) else {
                return self.invalid_expression(e, SemanticErrorKind::InvalidUnaryOperand);
            };
            let mut info = Self::expression_result(e, target);
            info.category = if matches!(self.types.nodes[target.index], TypeKind::Function { .. }) {
                ValueCategory::FunctionDesignator
            } else {
                self.object_category(target)
            };
            info.static_address = self.address_value(operand);
            return info;
        }
        if matches!(
            op,
            U::PreIncrement | U::PreDecrement | U::PostIncrement | U::PostDecrement
        ) {
            if operand.category != ValueCategory::ModifiableLvalue {
                return self.invalid_expression(e, SemanticErrorKind::ExpectedModifiableLvalue);
            }
            if !(self.real(ty)
                || self
                    .pointer_target(ty)
                    .is_some_and(|t| self.pointer_arithmetic_target(t)))
            {
                return self.invalid_expression(e, SemanticErrorKind::InvalidUnaryOperand);
            }
            return Self::expression_result(e, ty);
        }
        if !match op {
            | U::Plus | U::Minus => self.arithmetic(ty),
            | U::BitwiseNot => self.integer_type(ty).is_some(),
            | U::LogicalNot => self.scalar_type(ty),
            | _ => true,
        } {
            return self.invalid_expression(e, SemanticErrorKind::InvalidUnaryOperand);
        }
        if matches!(op, U::Real | U::Imag) {
            return self.real_imag(e, op, operand);
        }
        let ty = if op == U::LogicalNot {
            self.types.scalar(Scalar::Int)
        } else {
            self.promote(operand, ty)
        };
        self.convert(operand.expression, ty, ConversionKind::Arithmetic);
        let mut info = Self::expression_result(e, ty);
        info.unfolded_binary128 = operand.unfolded_binary128;
        info.integer = operand.integer.and_then(|v| v.unary(op));
        if op == U::LogicalNot {
            info.integer =
                Self::constant_truth(operand).map(|truth| Integer::int(i128::from(!truth)));
        }
        if self.integer_type(ty).is_none() {
            info.floating = self
                .floating_value(operand)
                .map(|value| {
                    if op == U::Minus {
                        Self::floating_negate(value)
                    } else {
                        value
                    }
                })
                .and_then(|value| self.round_floating(value, ty));
        }
        info.ice = operand.ice;
        if operand.constant == ConstantClass::Arithmetic {
            info.constant = ConstantClass::Arithmetic;
        }
        info
    }

    /// C99: §6.5.5-§6.5.17, pp. 82-94; PDF pp. 94-106.
    pub(in crate::translation_phases::semantic_analysis) fn type_binary(
        &mut self,
        e: &'tu Expression<'tu>,
        op: BinaryOperator,
        left: ExpressionInfo<'tu>,
        right: ExpressionInfo<'tu>,
    ) -> ExpressionInfo<'tu> {
        use BinaryOperator as B;
        if self.types.unanalyzed(left.ty) || self.types.unanalyzed(right.ty) {
            return Self::expression_result(e, self.types.unknown());
        }
        if let Some(result) = self.vector_binary(e, op, left, right) {
            return result;
        }
        let compound = match op {
            | B::MultiplicationAssignment => Some(B::Multiplication),
            | B::DivisionAssignment => Some(B::Division),
            | B::ModuloAssignment => Some(B::Modulo),
            | B::AdditionAssignment => Some(B::Addition),
            | B::SubtractionAssignment => Some(B::Subtraction),
            | B::LeftShiftAssignment => Some(B::LeftShift),
            | B::RightShiftAssignment => Some(B::RightShift),
            | B::BitwiseAndAssignment => Some(B::BitwiseAnd),
            | B::BitwiseXorAssignment => Some(B::BitwiseXor),
            | B::BitwiseOrAssignment => Some(B::BitwiseOr),
            | _ => None,
        };
        if op == B::Assignment || compound.is_some() {
            if left.category != ValueCategory::ModifiableLvalue {
                return self.invalid_expression(e, SemanticErrorKind::ExpectedModifiableLvalue);
            }
            if let Some(operation) = compound {
                // This is one bounded delegation, never a traversal of syntax.
                if matches!(operation, B::Addition | B::Subtraction) {
                    // Compound addition is directional, unlike ordinary +.
                    // C99 §6.5.16.2p1, p. 93; PDF p. 105.
                    let valid = if self.pointer_target(left.ty).is_some() {
                        self.integer_type(right.ty).is_some()
                    } else {
                        self.arithmetic(left.ty) && self.arithmetic(right.ty)
                    };
                    if !valid {
                        return self
                            .invalid_expression(e, SemanticErrorKind::InvalidAdditiveOperands);
                    }
                }
            } else if !self.assignment_compatible(left.ty, right) {
                return self.invalid_expression(e, SemanticErrorKind::InvalidAssignment);
            }
            if compound.is_none() {
                self.convert(
                    right.expression,
                    self.types.non_atomic(left.ty).unqualified(),
                    ConversionKind::Assignment,
                );
                return Self::expression_result(e, self.types.non_atomic(left.ty).unqualified());
            }
        }
        let op = compound.unwrap_or(op);
        let lt = self.converted(left);
        let rt = self.converted(right);
        let int = self.types.scalar(Scalar::Int);
        if op == B::Comma {
            return Self::expression_result(e, rt);
        }
        let mut info = Self::expression_result(e, self.types.unknown());
        let mut arithmetic_operands = None;
        match op {
            | B::Subscript => {
                let pair = if self.integer_type(rt).is_some() {
                    self.pointer_target(lt).map(|t| (t, left))
                } else if self.integer_type(lt).is_some() {
                    self.pointer_target(rt).map(|t| (t, right))
                } else {
                    None
                };
                let Some((target, pointer)) = pair else {
                    return self.invalid_expression(e, SemanticErrorKind::InvalidSubscript);
                };
                if !self.pointer_arithmetic_target(target) {
                    return self.invalid_expression(e, SemanticErrorKind::InvalidSubscript);
                }
                info.ty = target;
                info.category = self.object_category(target);
                info.static_address = self.address_value(pointer)
                    && if std::ptr::eq(pointer.expression, left.expression) {
                        right.ice && right.integer.is_some()
                    } else {
                        left.ice && left.integer.is_some()
                    };
                // Array decay retains the designated register object;
                // reading a register pointer does not transfer its storage.
                // C99 §6.5.3.2p1, p. 78; PDF p. 90.
                info.register = pointer.register
                    && matches!(self.types.nodes[pointer.ty.index], TypeKind::Array(..));
            },
            | B::Multiplication | B::Division | B::Modulo => {
                if !(self.arithmetic(lt) && self.arithmetic(rt))
                    || (op == B::Modulo
                        && (self.integer_type(lt).is_none() || self.integer_type(rt).is_none()))
                {
                    return self
                        .invalid_expression(e, SemanticErrorKind::InvalidArithmeticOperands);
                }
                info.ty = self.common_arithmetic(left, right, lt, rt);
            },
            | B::Addition | B::Subtraction =>
                if self.arithmetic(lt) && self.arithmetic(rt) {
                    info.ty = self.common_arithmetic(left, right, lt, rt);
                } else if let Some(target) = self.pointer_target(lt) {
                    if !self.pointer_arithmetic_target(target) {
                        return self
                            .invalid_expression(e, SemanticErrorKind::InvalidAdditiveOperands);
                    }
                    if self.integer_type(rt).is_some() {
                        info.ty = lt;
                        if self.address_value(left) && right.ice && right.integer.is_some() {
                            info.constant = ConstantClass::Address;
                        }
                    } else if op == B::Subtraction
                        && self.pointer_target(rt).is_some_and(|t| {
                            self.pointer_arithmetic_target(t)
                                && self.pointer_compatible(target, t, false)
                        })
                    {
                        info.ty = self.types.scalar(self.types.target.ptrdiff_t);
                        info.integer = self.address_difference(left, right, target);
                        if info.integer.is_some() {
                            info.constant = ConstantClass::Arithmetic;
                            info.folding = ConstantFolding::AddressDifference;
                        }
                    } else {
                        return self
                            .invalid_expression(e, SemanticErrorKind::InvalidAdditiveOperands);
                    }
                } else if op == B::Addition
                    && self.integer_type(lt).is_some()
                    && self
                        .pointer_target(rt)
                        .is_some_and(|t| self.pointer_arithmetic_target(t))
                {
                    info.ty = rt;
                    if self.address_value(right) && left.ice && left.integer.is_some() {
                        info.constant = ConstantClass::Address;
                    }
                } else {
                    return self.invalid_expression(e, SemanticErrorKind::InvalidAdditiveOperands);
                },
            | B::LeftShift | B::RightShift | B::BitwiseAnd | B::BitwiseOr | B::BitwiseXor => {
                if self.integer_type(lt).is_none() || self.integer_type(rt).is_none() {
                    return self.invalid_expression(e, SemanticErrorKind::InvalidIntegerOperands);
                }
                info.ty = if matches!(op, B::LeftShift | B::RightShift) {
                    let ty = self.promote(right, rt);
                    self.convert(right.expression, ty, ConversionKind::Arithmetic);
                    let ty = self.promote(left, lt);
                    self.convert(left.expression, ty, ConversionKind::Arithmetic);
                    ty
                } else {
                    self.common_arithmetic(left, right, lt, rt)
                };
            },
            | B::LogicalAnd | B::LogicalOr => {
                if !self.scalar_type(lt) || !self.scalar_type(rt) {
                    return self.invalid_expression(e, SemanticErrorKind::InvalidLogicalOperands);
                }
                info.ty = int;
            },
            | B::LessThan
            | B::LessThanOrEqual
            | B::GreaterThan
            | B::GreaterThanOrEqual
            | B::Equal
            | B::NotEqual => {
                let equality = matches!(op, B::Equal | B::NotEqual);
                let arithmetic = if equality {
                    self.arithmetic(lt) && self.arithmetic(rt)
                } else {
                    self.real(lt) && self.real(rt)
                };
                if arithmetic {
                    arithmetic_operands = Some(self.common_arithmetic(left, right, lt, rt));
                } else {
                    let null_pair = equality
                        && ((self.pointer_target(lt).is_some()
                            && self.null_pointer_constant(right))
                            || (self.pointer_target(rt).is_some()
                                && self.null_pointer_constant(left)));
                    // C99 §6.5.9p2, p. 86; PDF p. 98: null alternatives
                    // apply even when the null constant itself is void *.
                    let valid = if null_pair {
                        true
                    } else if let (Some(lp), Some(rp)) =
                        (self.pointer_target(lt), self.pointer_target(rt))
                    {
                        self.pointer_compatible(lp, rp, equality)
                            && (equality
                                || !matches!(self.types.nodes[lp.index], TypeKind::Function { .. }))
                    } else {
                        false
                    };
                    if !valid {
                        return self
                            .invalid_expression(e, SemanticErrorKind::InvalidComparisonOperands);
                    }
                }
                info.ty = int;
            },
            | _ => {},
        }
        if self.arithmetic(info.ty) {
            info.unfolded_binary128 = left.unfolded_binary128
                || right.unfolded_binary128
                || (self.binary128_type(info.ty)
                    && left.constant == ConstantClass::Arithmetic
                    && right.constant == ConstantClass::Arithmetic);
            let floating = left.floating.is_some() || right.floating.is_some();
            if floating
                && let Some((left, right)) = self
                    .floating_value(left)
                    .and_then(|v| self.round_floating(v, arithmetic_operands.unwrap_or(info.ty)))
                    .zip(self.floating_value(right).and_then(|v| {
                        self.round_floating(v, arithmetic_operands.unwrap_or(info.ty))
                    }))
            {
                info.floating = self
                    .floating_binary(op, left, right)
                    .and_then(|value| self.round_floating(value, info.ty));
                info.integer = Self::floating_comparison(op, left, right);
            }
            info.ice = left.ice && right.ice;
            if !floating && let Some((l, r)) = left.integer.zip(right.integer) {
                info.integer = l.binary(op, r);
                if info.integer.is_none()
                    && let Some(value) = l.sign_bit_shift(op, r)
                {
                    info.integer = Some(value);
                    if !self.tainted {
                        self.context.report_extension(
                            crate::configuration::Feature::SignBitShifts,
                            "left shift into the sign bit",
                            e.operator_source_vectors.unwrap_or(e.source_vectors),
                        );
                    }
                }
            }
            if matches!(op, B::LogicalAnd | B::LogicalOr) {
                info.integer = Self::constant_truth(left)
                    .zip(Self::constant_truth(right))
                    .map(|(l, r)| {
                        Integer::int(i128::from(if op == B::LogicalAnd {
                            l && r
                        } else {
                            l || r
                        }))
                    });
            }
            if let Some((bits, signed)) = self.integer_type(info.ty) {
                info.integer = info.integer.map(|v| v.cast(bits, signed));
            }
            if left.constant == ConstantClass::Arithmetic
                && right.constant == ConstantClass::Arithmetic
            {
                info.constant = ConstantClass::Arithmetic;
            }
            if matches!(op, B::LogicalAnd | B::LogicalOr)
                && Self::constant_truth(left)
                    .is_some_and(|v| (op == B::LogicalAnd && !v) || (op == B::LogicalOr && v))
            {
                info.integer = Self::constant_truth(left).map(|v| Integer::int(i128::from(v)));
                // C99 §6.6p3: the unevaluated right operand may contain
                // operators otherwise forbidden in constant expressions.
                info.ice = left.ice;
                if left.constant == ConstantClass::Arithmetic {
                    info.constant = ConstantClass::Arithmetic;
                }
            }
        }
        if compound.is_some() {
            let ty = self.types.non_atomic(left.ty).unqualified();
            self.convert(e, ty, ConversionKind::Assignment);
            let mut result = Self::expression_result(e, ty);
            result.operation_type = Some(info.ty);
            return result;
        }
        info
    }

    /// C99: §6.5.15p2-6, pp. 90-91; PDF pp. 102-103.
    pub(in crate::translation_phases::semantic_analysis) fn type_conditional(
        &mut self,
        e: &'tu Expression<'tu>,
        c: &'tu ConditionalExpression<'tu>,
    ) -> ExpressionInfo<'tu> {
        let condition = self.expression_info(c.condition_expression);
        let left = self.expression_info(c.then_expression);
        let right = self.expression_info(c.else_expression);
        let ct = self.converted(condition);
        let lt = self.converted(left);
        let rt = self.converted(right);
        if self.types.unanalyzed(ct) || self.types.unanalyzed(lt) || self.types.unanalyzed(rt) {
            return Self::expression_result(e, self.types.unknown());
        }
        if !self.scalar_type(ct) {
            return self.invalid_expression(e, SemanticErrorKind::InvalidConditionalOperands);
        }
        let ty = if self.arithmetic(lt) && self.arithmetic(rt) {
            self.common_arithmetic(left, right, lt, rt)
        } else if self.vector(lt).is_some() && self.vector(rt).is_some() {
            // GNU vectors extend the compatible alternatives of C99 §6.5.15p3.
            let Some(ty) = self.types.composite(lt.unqualified(), rt.unqualified()) else {
                return self.invalid_expression(e, SemanticErrorKind::InvalidConditionalOperands);
            };
            ty
        } else if (matches!(self.types.nodes[lt.index], TypeKind::Scalar(Scalar::Void))
            && matches!(self.types.nodes[rt.index], TypeKind::Scalar(Scalar::Void)))
            || (self.pointer_target(lt).is_some() && self.null_pointer_constant(right))
        {
            lt
        } else if self.pointer_target(rt).is_some() && self.null_pointer_constant(left) {
            rt
        } else if let (Some(lp), Some(rp)) = (self.pointer_target(lt), self.pointer_target(rt)) {
            if !self.pointer_compatible(lp, rp, true) {
                return self.invalid_expression(e, SemanticErrorKind::InvalidConditionalOperands);
            }
            let target = if matches!(self.types.nodes[lp.index], TypeKind::Scalar(Scalar::Void)) {
                lp
            } else if matches!(self.types.nodes[rp.index], TypeKind::Scalar(Scalar::Void)) {
                rp
            } else {
                self.types
                    .composite(lp.unqualified(), rp.unqualified())
                    .unwrap_or(lp)
            };
            self.types.intern(TypeKind::Pointer(
                target.qualified(lp.qualifiers | rp.qualifiers),
            ))
        } else if matches!(self.types.nodes[lt.index], TypeKind::Tag(_))
            && self
                .types
                .composite(lt.unqualified(), rt.unqualified())
                .is_some()
        {
            lt.unqualified()
        } else {
            return self.invalid_expression(e, SemanticErrorKind::InvalidConditionalOperands);
        };
        self.convert(left.expression, ty, ConversionKind::Arithmetic);
        self.convert(right.expression, ty, ConversionKind::Arithmetic);
        let mut info = Self::expression_result(e, ty);
        if self.arithmetic(ty)
            && condition.constant == ConstantClass::Arithmetic
            && left.constant == ConstantClass::Arithmetic
            && right.constant == ConstantClass::Arithmetic
            && (condition.unfolded_binary128
                || left.unfolded_binary128
                || right.unfolded_binary128
                || self.binary128_type(ty))
        {
            info.unfolded_binary128 = true;
            info.constant = ConstantClass::Arithmetic;
        }
        if let Some(truth) = Self::constant_truth(condition) {
            let selected = if truth { left } else { right };
            // C99 §6.6p3,6: only the selected arm is evaluated (§6.5.15p4),
            // but an ICE must still have integer type.
            info.ice = self.integer_type(ty).is_some() && condition.ice && selected.ice;
            info.unfolded_binary128 = selected.unfolded_binary128
                || (self.binary128_type(ty) && selected.constant == ConstantClass::Arithmetic);
            info.integer = selected.integer;
            if self.arithmetic(ty) && self.integer_type(ty).is_none() {
                info.floating = self
                    .floating_value(selected)
                    .and_then(|v| self.round_floating(v, ty));
            }
            if let Some((bits, signed)) = self.integer_type(ty) {
                info.integer = info.integer.map(|v| v.cast(bits, signed));
            }
            if condition.constant == ConstantClass::Arithmetic {
                info.constant = if self.address_value(selected) {
                    ConstantClass::Address
                } else {
                    selected.constant
                };
            }
        }
        info
    }

    /// C99: §6.5.2.2p1-7, pp. 71-72; PDF pp. 83-84.
    pub(in crate::translation_phases::semantic_analysis) fn type_call(
        &mut self,
        e: &'tu Expression<'tu>,
        function: ExpressionInfo<'tu>,
        arguments: ArenaList<'tu, &'tu Expression<'tu>>,
    ) -> ExpressionInfo<'tu> {
        self.x86_immediates(function, arguments);
        let ty = self.converted(function);
        if self.types.unanalyzed(ty) {
            return Self::expression_result(e, self.types.unknown());
        }
        let Some(target) = self.pointer_target(ty) else {
            return self.invalid_expression(e, SemanticErrorKind::InvalidCall);
        };
        // C99 §6.5.2.2p1: defer the function constraint for an unmodeled
        // target.
        if self.types.unanalyzed(target) {
            return Self::expression_result(e, self.types.unknown());
        }
        let TypeKind::Function {
            result,
            parameters,
            prototype,
            variadic,
        } = self.types.nodes[target.index]
        else {
            return self.invalid_expression(e, SemanticErrorKind::InvalidCall);
        };
        if prototype
            && (arguments.len() < parameters.len()
                || (!variadic && arguments.len() != parameters.len()))
        {
            self.error(
                SemanticErrorKind::InvalidArgumentCount,
                e.source_vectors,
                None,
                None,
            );
        }
        let mut failed = prototype
            && (arguments.len() < parameters.len()
                || (!variadic && arguments.len() != parameters.len()));
        for (index, &argument) in arguments.iter().enumerate() {
            let info = self.expression_info(argument);
            failed |= self.types.unanalyzed(info.ty);
            if prototype && let Some(&parameter) = parameters.get(index) {
                if !self.assignment_compatible(parameter, info) {
                    failed = true;
                    self.error(
                        SemanticErrorKind::InvalidArgumentType,
                        argument.source_vectors,
                        None,
                        None,
                    );
                }
                self.convert(argument, parameter, ConversionKind::Assignment);
            } else {
                let ty = self.converted(info);
                let ty = if matches!(self.types.nodes[ty.index], TypeKind::Scalar(Scalar::Float)) {
                    self.types.scalar(Scalar::Double)
                } else {
                    self.promote(info, ty)
                };
                self.convert(argument, ty, ConversionKind::DefaultArgument);
            }
            if !self.types.unanalyzed(info.ty)
                && matches!(
                    self.types.nodes[info.ty.index],
                    TypeKind::Scalar(Scalar::Void)
                )
            {
                failed = true;
                self.error(
                    SemanticErrorKind::InvalidArgumentType,
                    argument.source_vectors,
                    None,
                    None,
                );
            }
        }
        if failed {
            return Self::expression_result(e, self.types.unknown());
        }
        if !matches!(
            self.types.nodes[result.index],
            TypeKind::Scalar(Scalar::Void)
        ) && !self.types.unanalyzed(result)
            && !self.complete_object(result)
        {
            return self.invalid_expression(e, SemanticErrorKind::InvalidCall);
        }
        Self::expression_result(e, result.unqualified())
    }

    /// C99: §6.5.2.3p1-4, pp. 72-73; PDF pp. 84-85.
    pub(in crate::translation_phases::semantic_analysis) fn type_member(
        &mut self,
        e: &'tu Expression<'tu>,
        base: ExpressionInfo<'tu>,
        member: Identifier,
        indirect: bool,
    ) -> ExpressionInfo<'tu> {
        if self.types.unanalyzed(base.ty) {
            return Self::expression_result(e, self.types.unknown());
        }
        let mut ty = base.ty;
        if indirect {
            let converted = self.converted(base);
            let Some(target) = self.pointer_target(converted) else {
                return self.invalid_expression(e, SemanticErrorKind::InvalidMemberAccess);
            };
            // An unanalyzed pointee may be a record; its members are unknown.
            if self.types.unanalyzed(target) {
                return Self::expression_result(e, self.types.unknown());
            }
            ty = target;
        }
        // C99 §6.5.2.3p2: defer the record constraint for an unmodeled target.
        if self.types.unanalyzed(ty) {
            return Self::expression_result(e, self.types.unknown());
        }
        let TypeKind::Tag(id) = self.types.nodes[ty.index] else {
            return self.invalid_expression(e, SemanticErrorKind::InvalidMemberAccess);
        };
        let Some(&index) = self.member_indices.get(&(id, member.name)) else {
            return self.invalid_expression(e, SemanticErrorKind::InvalidMemberAccess);
        };
        let Some(field) = self.types.tags[id].fields.get().get(index).copied() else {
            return Self::expression_result(e, self.types.unknown());
        };
        let mut field_ty = field.ty;
        let mut arrays = ArenaVec::new_in(self.scratch);
        while let TypeKind::Array(element, bound) = self.types.nodes[field_ty.index] {
            arrays.push(bound);
            field_ty = element;
        }
        // Qualifiers of the record and of any anonymous members on the path
        // reach the member, and an array's element (§6.5.2.3p3, §6.7.3p8).
        field_ty = field_ty.qualified(ty.qualifiers | field.qualifiers);
        while let Some(bound) = arrays.pop() {
            field_ty = self.types.intern(TypeKind::Array(field_ty, bound));
        }
        let mut info = Self::expression_result(e, field_ty);
        if indirect
            || matches!(
                base.category,
                ValueCategory::Lvalue | ValueCategory::ModifiableLvalue
            )
        {
            info.category = self.object_category(field_ty);
        }
        info.bit_field = field.width;
        info.register = !indirect && base.register;
        info.static_address = if indirect {
            self.address_value(base)
        } else {
            base.static_address
        };
        info
    }

    /// C99: §6.5.4p2-4, p. 81; PDF p. 93.
    pub(in crate::translation_phases::semantic_analysis) fn type_cast(
        &mut self,
        e: &'tu Expression<'tu>,
        target: TypeId,
        operand: ExpressionInfo<'tu>,
    ) -> ExpressionInfo<'tu> {
        let from = self.converted(operand);
        if matches!(self.types.nodes[target.index], TypeKind::Tag(id) if self.types.tags[id].kind == TagKind::Union)
        {
            return Self::expression_result(e, self.types.unknown());
        }
        if self.types.unanalyzed(target) || self.types.unanalyzed(from) {
            return Self::expression_result(e, self.types.unknown());
        }
        if self.vector(target).is_some() || self.vector(from).is_some() {
            // C99 §6.5.4p2: a void target also permits discarding a GNU vector.
            if matches!(
                self.types.nodes[target.index],
                TypeKind::Scalar(Scalar::Void)
            ) {
                self.convert(operand.expression, target, ConversionKind::Assignment);
                return Self::expression_result(e, target.unqualified());
            }
            let permitted = |ty| self.vector(ty).is_some() || self.integer_type(ty).is_some();
            if permitted(target)
                && permitted(from)
                && self.vector_width(target) == self.vector_width(from)
            {
                return Self::expression_result(e, target.unqualified());
            }
            return self.vector_error(e);
        }
        let void = matches!(
            self.types.nodes[target.index],
            TypeKind::Scalar(Scalar::Void)
        );
        if !void
            && (!self.scalar_type(target)
                || !self.scalar_type(from)
                || (self.pointer_target(target).is_some()
                    && self.arithmetic(from)
                    && self.integer_type(from).is_none())
                || (self.pointer_target(from).is_some()
                    && self.arithmetic(target)
                    && self.integer_type(target).is_none()))
        {
            return self.invalid_expression(e, SemanticErrorKind::InvalidCast);
        }
        self.convert(operand.expression, target, ConversionKind::Assignment);
        let mut info = Self::expression_result(e, target.unqualified());
        info.unfolded_binary128 = operand.unfolded_binary128
            || (self.binary128_type(target) && operand.constant == ConstantClass::Arithmetic);
        let mut immediate = operand.expression;
        while let ExpressionType::Parenthesized { expression } = immediate.kind {
            immediate = expression;
        }
        if let Some((bits, signed)) = self.integer_type(target) {
            info.integer = if let ExpressionType::Constant(Constant::Float(value)) = immediate.kind
            {
                self.float_cast(target, value)
            } else {
                operand.integer.map(|v| {
                    if matches!(
                        self.types.nodes[target.index],
                        TypeKind::Scalar(Scalar::Bool)
                    ) {
                        Integer::int(i128::from(v.value != 0)).cast(1, false)
                    } else {
                        v.cast(bits, signed)
                    }
                })
            };
            info.ice = operand.ice
                || matches!(immediate.kind, ExpressionType::Constant(Constant::Float(_)));
        }
        if let Some(value) = operand.floating
            && self.integer_type(target).is_some()
        {
            info.integer = if matches!(
                self.types.nodes[target.index],
                TypeKind::Scalar(Scalar::Bool)
            ) {
                Some(Integer::int(i128::from(value.truth())).cast(1, false))
            } else {
                self.float_cast(target, FloatTokenType::LongDouble(value.real))
            };
        }
        if self.arithmetic(target) && self.integer_type(target).is_none() {
            info.floating = self
                .floating_value(operand)
                .and_then(|value| self.round_floating(value, target));
        }
        if self.arithmetic(target) && operand.constant == ConstantClass::Arithmetic {
            info.constant = ConstantClass::Arithmetic;
        }
        if info.unfolded_binary128 {
            info.integer = None;
            info.floating = None;
            info.ice = false;
        }
        // Known pointer truth is an accepted arithmetic constant form, but
        // not an integer constant expression. C99 §6.3.1.2p1, p. 43;
        // PDF p. 55; implementation latitude §6.6p10, p. 96; PDF p. 108.
        if matches!(
            self.types.nodes[target.index],
            TypeKind::Scalar(Scalar::Bool)
        ) && self.pointer_target(from).is_some()
            && self.address_value(operand)
        {
            let truth = Self::constant_truth(operand).unwrap_or(true);
            info.integer = Some(Integer::int(i128::from(truth)).cast(1, false));
            info.constant = ConstantClass::Arithmetic;
            info.ice = false;
        }
        if self.pointer_target(target).is_some()
            && (self.address_value(operand) || (operand.ice && operand.integer.is_some()))
        {
            info.constant = ConstantClass::Address;
            info.integer = operand.integer.map(|v| v.cast(64, false));
        }
        // GNU modes fold an integer-valued address constant converted to an
        // integer type, which C99 does not count as arithmetic (§6.6p8).
        if self.context.configuration.gnu_extensions()
            && self.integer_type(target).is_some()
            && Self::pointer_value(operand).is_some()
        {
            info.constant = ConstantClass::Arithmetic;
        }
        info
    }

    /// C99: §6.5.3.4p1-5, p. 80; PDF p. 92.
    /// Alignment: C11 §6.5.3.4p3, p. 90; PDF p. 108 (extension in C99).
    pub(in crate::translation_phases::semantic_analysis) fn type_sizeof(
        &mut self,
        e: &'tu Expression<'tu>,
        ty: TypeId,
        bit_field: Option<u32>,
        align: bool,
    ) -> ExpressionInfo<'tu> {
        if self.types.unanalyzed(ty) {
            return Self::expression_result(e, self.types.unknown());
        }
        let gnu = self.context.configuration.gnu_extensions()
            && matches!(
                self.types.nodes[ty.index],
                TypeKind::Scalar(Scalar::Void) | TypeKind::Function { .. }
            );
        if bit_field.is_some() || (!gnu && !self.complete_object(ty)) {
            return self.invalid_expression(e, SemanticErrorKind::InvalidSizeof);
        }
        let result = self.types.scalar(self.types.target.size_t);
        let mut info = Self::expression_result(e, result);
        let value = if align {
            self.types.alignment(ty)
        } else {
            self.types.layout(ty).map(|layout| layout.size)
        };
        if let Some(value) = value.or_else(|| gnu.then_some(1)) {
            info.integer = Some(Integer {
                value:  i128::from(value),
                bits:   64,
                signed: false,
            });
            info.ice = true;
            info.constant = ConstantClass::Arithmetic;
        }
        info
    }

    pub(in crate::translation_phases::semantic_analysis) fn retain_expression(
        &mut self,
        info: ExpressionInfo<'tu>,
    ) {
        let key = std::ptr::from_ref(info.expression).addr();
        if self.expression_indices.contains_key(&key) {
            return;
        }
        _ = self.expression_indices.insert(key, self.expressions.len());
        self.expressions.push(info);
    }

    pub(in crate::translation_phases::semantic_analysis) fn expression_info(
        &self,
        e: &'tu Expression<'tu>,
    ) -> ExpressionInfo<'tu> {
        self.expression_indices
            .get(&std::ptr::from_ref(e).addr())
            .map_or_else(
                || Self::expression_result(e, self.types.unknown()),
                |&i| self.expressions[i],
            )
    }

    pub(in crate::translation_phases::semantic_analysis) fn expression_result(
        e: &'tu Expression<'tu>,
        ty: TypeId,
    ) -> ExpressionInfo<'tu> {
        ExpressionInfo {
            expression: e,
            selected_expression: None,
            ty,
            operation_type: None,
            category: ValueCategory::Rvalue,
            binding: None,
            bit_field: None,
            register: false,
            vector_element: false,
            static_address: false,
            unfolded_binary128: false,
            floating: None,
            integer: None,
            ice: false,
            constant: ConstantClass::None,
            folding: ConstantFolding::Permitted,
        }
    }

    /// C99: §6.3.2.1p1, p. 46; PDF p. 58. A structure/union containing
    /// const members is not modifiable; arrays and incomplete types are
    /// excluded.
    pub(in crate::translation_phases::semantic_analysis) fn object_category(
        &mut self,
        ty: TypeId,
    ) -> ValueCategory {
        if self.complete_object(ty)
            && !matches!(self.types.nodes[ty.index], TypeKind::Array(..))
            && !self.contains_const(ty)
        {
            ValueCategory::ModifiableLvalue
        } else {
            ValueCategory::Lvalue
        }
    }

    /// Checks recursive const membership when classifying modifiable lvalues.
    /// C99: §6.3.2.1 paragraph 1, p. 46; PDF p. 58.
    pub(in crate::translation_phases::semantic_analysis) fn contains_const(
        &mut self,
        ty: TypeId,
    ) -> bool {
        let mut pending = ArenaVec::new_in(self.scratch);
        pending.push((ty, false));
        while let Some((current, ready)) = pending.pop() {
            if !ready && self.const_members.contains_key(&current.index) {
                continue;
            }
            if ready {
                let value = match self.types.nodes[current.index] {
                    | TypeKind::Array(element, _) =>
                        element.qualifiers.contains(TypeQualifiers::CONST)
                            || self
                                .const_members
                                .get(&element.index)
                                .copied()
                                .unwrap_or(false),
                    | TypeKind::Tag(id) if self.types.tags[id].kind != TagKind::Enum =>
                        self.types.tags[id].members.get().iter().any(|m| {
                            m.ty.qualifiers.contains(TypeQualifiers::CONST)
                                || self
                                    .const_members
                                    .get(&m.ty.index)
                                    .copied()
                                    .unwrap_or(false)
                        }),
                    | _ => false,
                };
                _ = self.const_members.insert(current.index, value);
            } else {
                // A provisional answer ends a cycle through an invalid
                // self-containing record.
                _ = self.const_members.insert(current.index, false);
                pending.push((current, true));
                match self.types.nodes[current.index] {
                    | TypeKind::Array(element, _) => pending.push((element, false)),
                    | TypeKind::Tag(id) if self.types.tags[id].kind != TagKind::Enum => {
                        for member in self.types.tags[id].members.get() {
                            pending.push((member.ty, false));
                        }
                    },
                    | _ => {},
                }
            }
        }
        ty.qualifiers.contains(TypeQualifiers::CONST)
            || self.const_members.get(&ty.index).copied().unwrap_or(false)
    }

    /// C99: §6.2.5p22, p. 36; PDF p. 48; §6.7.5.2p1, p. 116; PDF p. 128.
    pub(in crate::translation_phases::semantic_analysis) fn complete_object(
        &self,
        ty: TypeId,
    ) -> bool {
        self.types.complete_object(ty)
    }

    /// Classifies integer and floating types as arithmetic types.
    /// C99: §6.2.5 paragraph 18, p. 35; PDF p. 47.
    pub(in crate::translation_phases::semantic_analysis) fn arithmetic(&self, ty: TypeId) -> bool {
        let ty = self.types.non_atomic(ty);
        self.integer_type(ty).is_some()
            || matches!(self.types.nodes[ty.index], TypeKind::Scalar(s) if s != Scalar::Void)
    }

    /// Classifies integer and real floating types for relational operators.
    /// C99: §6.2.5 paragraph 17, p. 35; PDF p. 47.
    pub(in crate::translation_phases::semantic_analysis) fn real(&self, ty: TypeId) -> bool {
        self.integer_type(ty).is_some()
            || matches!(
                self.types.nodes[ty.index],
                TypeKind::Scalar(
                    Scalar::Float | Scalar::Double | Scalar::LongDouble | Scalar::Float128
                )
            )
    }

    /// Classifies arithmetic and pointer types as scalar types.
    /// C99: §6.2.5 paragraph 21, p. 36; PDF p. 48.
    pub(in crate::translation_phases::semantic_analysis) fn scalar_type(&self, ty: TypeId) -> bool {
        self.arithmetic(ty) || self.pointer_target(ty).is_some()
    }

    /// Pointer arithmetic needs a complete object type (§6.5.6p2); an
    /// unanalyzed target, such as a GNU vector, is not checked.
    /// C99: §6.5.6 paragraphs 2-3, pp. 82-83; PDF pp. 94-95.
    pub(in crate::translation_phases::semantic_analysis) fn pointer_arithmetic_target(
        &self,
        target: TypeId,
    ) -> bool {
        self.complete_object(target) || self.types.unanalyzed(target)
    }

    pub(in crate::translation_phases::semantic_analysis) fn pointer_target(
        &self,
        ty: TypeId,
    ) -> Option<TypeId> {
        let ty = self.types.non_atomic(ty);
        if let TypeKind::Pointer(target) = self.types.nodes[ty.index] {
            Some(target)
        } else {
            None
        }
    }

    pub(in crate::translation_phases::semantic_analysis) fn convert(
        &mut self,
        e: &'tu Expression<'tu>,
        ty: TypeId,
        kind: ConversionKind,
    ) {
        self.conversions.push(Conversion {
            expression: e,
            ty,
            kind,
        });
    }

    /// C99: §6.3.2.1p2-4, p. 46; PDF p. 58.
    pub(in crate::translation_phases::semantic_analysis) fn converted(
        &mut self,
        info: ExpressionInfo<'tu>,
    ) -> TypeId {
        let (ty, kind) = match self.types.nodes[info.ty.index] {
            | TypeKind::Array(element, _) => (
                self.types.intern(TypeKind::Pointer(element)),
                Some(ConversionKind::ArrayDecay),
            ),
            | TypeKind::Function { .. } => (
                self.types.intern(TypeKind::Pointer(info.ty)),
                Some(ConversionKind::FunctionDecay),
            ),
            | _ if matches!(
                info.category,
                ValueCategory::Lvalue | ValueCategory::ModifiableLvalue
            ) =>
                (
                    self.types.non_atomic(info.ty).unqualified(),
                    Some(ConversionKind::Lvalue),
                ),
            | _ => (info.ty.unqualified(), None),
        };
        if let Some(kind) = kind {
            self.convert(info.expression, ty, kind);
        }
        ty
    }

    /// C99: §6.3.1.1p2, pp. 42-43; PDF pp. 54-55. Bit-field width can promote
    /// an
    /// unsigned int field to int when all its values fit.
    pub(in crate::translation_phases::semantic_analysis) fn promote(
        &mut self,
        info: ExpressionInfo<'tu>,
        ty: TypeId,
    ) -> TypeId {
        // An enumeration's compatible type has at least the rank of int, so
        // the enumeration promotes to that type unless it is a narrow
        // bit-field (§6.3.1.1p1-2).
        if let TypeKind::Tag(id) = self.types.nodes[ty.index]
            && self.types.tags[id].kind == TagKind::Enum
        {
            let promoted = if info.bit_field.is_some_and(|width| width < 32) {
                Scalar::Int
            } else {
                self.types.tags[id].compatible.get()
            };
            return self.types.scalar(promoted);
        }
        // Clang's implementation-defined extended bit-field promotions.
        // C99: §6.3.1.1p2, p. 42; PDF p. 54; §6.7.2.1p4, p. 101; PDF p. 113.
        if let Some(width) = info.bit_field
            && width <= 32
            && matches!(
                self.types.nodes[ty.index],
                TypeKind::Scalar(Scalar::Int128 | Scalar::UnsignedInt128)
            )
        {
            let unsigned = width == 32
                && matches!(
                    self.types.nodes[ty.index],
                    TypeKind::Scalar(Scalar::UnsignedInt128)
                );
            return self.types.scalar(if unsigned {
                Scalar::UnsignedInt
            } else {
                Scalar::Int
            });
        }
        if self.integer_type(ty).is_some_and(|(bits, _)| bits < 32)
            || (info.bit_field.is_some_and(|width| width < 32)
                && matches!(
                    self.types.nodes[ty.index],
                    TypeKind::Scalar(Scalar::Int | Scalar::UnsignedInt | Scalar::Bool)
                ))
        {
            self.types.scalar(Scalar::Int)
        } else {
            ty.unqualified()
        }
    }

    /// C99: §6.3.1.8p1, pp. 44-45; PDF pp. 56-57. Preserve ranks even
    /// when long and long long have the same LP64 width.
    pub(in crate::translation_phases::semantic_analysis) fn common_arithmetic(
        &mut self,
        left: ExpressionInfo<'tu>,
        right: ExpressionInfo<'tu>,
        lt: TypeId,
        rt: TypeId,
    ) -> TypeId {
        let lt = self.promote(left, lt);
        let rt = self.promote(right, rt);
        let ls = self.scalar_representation(lt);
        let rs = self.scalar_representation(rt);
        let (Some(ls), Some(rs)) = (ls, rs) else {
            return self.types.unknown();
        };
        let float_rank = |s| match s {
            | Scalar::Float | Scalar::ComplexFloat => 1,
            | Scalar::Double | Scalar::ComplexDouble => 2,
            | Scalar::LongDouble | Scalar::ComplexLongDouble => 3,
            | Scalar::Float128 | Scalar::ComplexFloat128 => 4,
            | _ => 0,
        };
        let rank = float_rank(ls).max(float_rank(rs));
        let complex = matches!(
            ls,
            Scalar::ComplexFloat
                | Scalar::ComplexDouble
                | Scalar::ComplexLongDouble
                | Scalar::ComplexFloat128
        ) || matches!(
            rs,
            Scalar::ComplexFloat
                | Scalar::ComplexDouble
                | Scalar::ComplexLongDouble
                | Scalar::ComplexFloat128
        );
        let scalar = if rank != 0 {
            match (rank, complex) {
                | (1, false) => Scalar::Float,
                | (2, false) => Scalar::Double,
                | (4, false) => Scalar::Float128,
                | (_, false) => Scalar::LongDouble,
                | (1, true) => Scalar::ComplexFloat,
                | (2, true) => Scalar::ComplexDouble,
                | (4, true) => Scalar::ComplexFloat128,
                | (_, true) => Scalar::ComplexLongDouble,
            }
        } else {
            let rank = |s| match s {
                | Scalar::Long | Scalar::UnsignedLong => 2,
                | Scalar::LongLong | Scalar::UnsignedLongLong => 3,
                | Scalar::Int128 | Scalar::UnsignedInt128 => 4,
                | _ => 1,
            };
            let unsigned = |s| {
                matches!(
                    s,
                    Scalar::UnsignedInt
                        | Scalar::UnsignedLong
                        | Scalar::UnsignedLongLong
                        | Scalar::UnsignedInt128
                )
            };
            let (high, low) = if rank(ls) >= rank(rs) {
                (ls, rs)
            } else {
                (rs, ls)
            };
            if unsigned(high) == unsigned(low)
                || unsigned(high)
                || self.types.target.integer(high).map(|m| m.0)
                    > self.types.target.integer(low).map(|m| m.0)
            {
                high
            } else {
                match high {
                    | Scalar::Long => Scalar::UnsignedLong,
                    | Scalar::LongLong => Scalar::UnsignedLongLong,
                    | Scalar::Int128 => Scalar::UnsignedInt128,
                    | _ => Scalar::UnsignedInt,
                }
            }
        };
        let ty = self.types.scalar(scalar);
        self.convert(left.expression, ty, ConversionKind::Arithmetic);
        self.convert(right.expression, ty, ConversionKind::Arithmetic);
        ty
    }

    pub(in crate::translation_phases::semantic_analysis) fn scalar_representation(
        &self,
        ty: TypeId,
    ) -> Option<Scalar> {
        let ty = self.types.non_atomic(ty);
        match self.types.nodes[ty.index] {
            | TypeKind::Scalar(s) => Some(s),
            | TypeKind::Tag(id) if self.types.tags[id].kind == TagKind::Enum =>
                Some(self.types.tags[id].compatible.get()),
            | _ => None,
        }
    }

    /// C99: §6.3.2.3p3, p. 47; PDF p. 59.
    pub(in crate::translation_phases::semantic_analysis) fn null_pointer_constant(
        &self,
        info: ExpressionInfo<'tu>,
    ) -> bool {
        (info.ice && info.integer.is_some_and(|v| v.value == 0))
            || (matches!(self.types.nodes[info.ty.index], TypeKind::Pointer(target) if matches!(self.types.nodes[target.index], TypeKind::Scalar(Scalar::Void)))
                && info.constant == ConstantClass::Address
                && info.integer.is_some_and(|v| v.value == 0))
    }

    /// C99: §6.5.16.1p1, p. 92; PDF p. 104. Qualifier inclusion applies
    /// only to the immediate targets, never recursively to int **.
    pub(in crate::translation_phases::semantic_analysis) fn assignment_compatible(
        &mut self,
        target: TypeId,
        source: ExpressionInfo<'tu>,
    ) -> bool {
        let target = self.types.non_atomic(target);
        let from = self.converted(source);
        if self.types.unanalyzed(target) || self.types.unanalyzed(from) {
            return true;
        }
        if self.vector_assignment(target, from) {
            return true;
        }
        if self.arithmetic(target) && self.arithmetic(from) {
            return true;
        }
        if matches!(
            self.types.nodes[target.index],
            TypeKind::Scalar(Scalar::Bool)
        ) && self.pointer_target(from).is_some()
        {
            return true;
        }
        if let Some(to) = self.pointer_target(target) {
            if self.null_pointer_constant(source) {
                return true;
            }
            if let Some(from) = self.pointer_target(from) {
                if !to.qualifiers.contains(from.qualifiers) {
                    return false;
                }
                return self.pointer_compatible(to, from, true);
            }
            return false;
        }
        matches!(self.types.nodes[target.index], TypeKind::Tag(_))
            && self
                .types
                .composite(target.unqualified(), from.unqualified())
                .is_some()
    }

    /// Checks compatible pointer targets, allowing object/void pairs where
    /// required.
    /// C99: §6.5.16.1 paragraph 1, p. 92; PDF p. 104.
    /// C99: §6.5.9 paragraph 2, p. 86; PDF p. 98.
    pub(in crate::translation_phases::semantic_analysis) fn pointer_compatible(
        &mut self,
        left: TypeId,
        right: TypeId,
        void: bool,
    ) -> bool {
        if self.types.unanalyzed(left) || self.types.unanalyzed(right) {
            return true;
        }
        if void {
            let is_void =
                |ty: TypeId| matches!(self.types.nodes[ty.index], TypeKind::Scalar(Scalar::Void));
            let is_function =
                |ty: TypeId| matches!(self.types.nodes[ty.index], TypeKind::Function { .. });
            if (is_void(left) && !is_function(right)) || (is_void(right) && !is_function(left)) {
                return true;
            }
        }
        self.types
            .composite(left.unqualified(), right.unqualified())
            .is_some()
    }

    /// C89 implicit function declarations are retained in GNU modes.
    /// Strict C99 and later require a declaration.
    /// C89: §3.3.2.2, p. 41; PDF p. 55.
    /// C99: §6.5.1 paragraph 2, p. 69; PDF p. 81.
    pub(in crate::translation_phases::semantic_analysis) fn implicit_function(
        &mut self,
        mut e: &'tu Expression<'tu>,
    ) {
        if self.tainted {
            return;
        }
        while let ExpressionType::Parenthesized { expression } = e.kind {
            e = expression;
        }
        if let ExpressionType::Identifier(name) = e.kind
            && self.lookup(Namespace::Ordinary, name.name).is_none()
            && self.builtin_name(name)
        {
            // Unmodeled GNU compiler intrinsics do not have the C89 implicit
            // int signature. Preserve an opaque result rather than rejecting
            // valid pointer/aggregate returns based on an invented int type.
            let ty = if let Some(ty) = self.x86_builtin_type(name) {
                ty
            } else {
                let result = self.types.unknown();
                self.types.intern(TypeKind::Function {
                    result,
                    parameters: &[],
                    prototype: false,
                    variadic: false,
                })
            };
            self.bind(
                name,
                ty,
                BindingKind::Function,
                Linkage::External,
                Duration::None,
                None,
            );
            return;
        }
        if let ExpressionType::Identifier(name) = e.kind
            && self.lookup(Namespace::Ordinary, name.name).is_none()
            && (self
                .context
                .configuration
                .is_native(crate::configuration::Feature::ImplicitFunctionDeclaration)
                || self.context.configuration.gnu_extensions())
        {
            let result = self.types.scalar(Scalar::Int);
            let ty = self.types.intern(TypeKind::Function {
                result,
                parameters: &[],
                prototype: false,
                variadic: false,
            });
            self.bind(
                name,
                ty,
                BindingKind::Function,
                Linkage::External,
                Duration::None,
                None,
            );
            self.context.report_extension(
                crate::configuration::Feature::ImplicitFunctionDeclaration,
                "implicit function declaration",
                name.source_vectors,
            );
        }
    }

    /// GCC builtins use reserved identifiers declared by the implementation;
    /// an undeclared builtin retains an unmodeled result rather than implicit
    /// int.
    /// C99: §7.1.3 paragraph 1, p. 166; PDF p. 178.
    pub(in crate::translation_phases::semantic_analysis) fn builtin_name(
        &self,
        name: Identifier,
    ) -> bool {
        super::atomics::modeled(self.context.string_cache.at(name.name))
            || self
                .context
                .string_cache
                .at(name.name)
                .starts_with("__builtin_")
    }

    /// GCC's `__builtin_constant_p` folds to whether its operand is an
    /// arithmetic constant; it returns `int` and is valid where a constant
    /// is required. GNU extension; C99: §6.6p10, p. 96; PDF p. 108.
    /// GNU extension: GCC manual, "Other Builtins".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
    pub(in crate::translation_phases::semantic_analysis) fn constant_p(
        &mut self,
        e: &'tu Expression<'tu>,
        function: &'tu Expression<'tu>,
        arguments: ArenaList<'tu, &'tu Expression<'tu>>,
    ) -> Option<ExpressionInfo<'tu>> {
        let mut callee = function;
        while let ExpressionType::Parenthesized { expression } = callee.kind {
            callee = expression;
        }
        let ExpressionType::Identifier(name) = callee.kind else {
            return None;
        };
        let &[argument] = arguments.as_slice() else {
            return None;
        };
        // Undeclared builtins are bound as opaque functions; a user
        // declaration with a known result type is an ordinary function.
        if self.context.string_cache.at(name.name) != "__builtin_constant_p"
            || self
                .lookup(Namespace::Ordinary, name.name)
                .is_some_and(|entry| {
                    !matches!(
                        self.types.nodes[self.bindings[entry.binding].ty.index],
                        TypeKind::Function { result, .. } if result == self.types.unknown()
                    )
                })
        {
            return None;
        }
        let operand = self.expression_info(argument);
        let constant = operand.constant == ConstantClass::Arithmetic
            && (operand.integer.is_some() || operand.floating.is_some());
        let mut info = Self::expression_result(e, self.types.scalar(Scalar::Int));
        info.integer = Some(Integer::int(i128::from(constant)));
        info.ice = true;
        info.constant = ConstantClass::Arithmetic;
        Some(info)
    }

    /// Checks integer switch expressions and scalar selection/iteration
    /// conditions.
    /// C99: §6.8.4.1 paragraph 1, p. 133; PDF p. 145.
    /// C99: §6.8.4.2 paragraph 1, p. 134; PDF p. 146.
    /// C99: §6.8.5 paragraph 2, p. 135; PDF p. 147.
    pub(in crate::translation_phases::semantic_analysis) fn check_condition(
        &mut self,
        mut slot: ExpressionSlot<'tu>,
        integer: bool,
    ) {
        while let ExpressionSlot::Selection(header) = slot {
            let Some(e) = header.expression else {
                return;
            };
            slot = e;
        }
        if let ExpressionSlot::Parsed(e) = slot {
            let info = self.expression_info(e);
            let ty = self.converted(info);
            if !self.types.unanalyzed(ty)
                && if integer {
                    self.integer_type(ty).is_none()
                } else {
                    !self.scalar_type(ty)
                }
            {
                self.error(
                    if integer {
                        SemanticErrorKind::InvalidSwitchExpression
                    } else {
                        SemanticErrorKind::InvalidCondition
                    },
                    e.source_vectors,
                    None,
                    None,
                );
            }
        }
    }

    /// C99: §6.4.5p5-6, pp. 62-63; PDF pp. 74-75. The count includes the final
    /// zero; narrow source characters contribute their UTF-8 code units.
    pub(in crate::translation_phases::semantic_analysis) fn string_type(
        &mut self,
        value: StringTokenType,
    ) -> Option<(TypeId, u64)> {
        let (id, wide) = match value {
            | StringTokenType::String(id) => (id, false),
            | StringTokenType::WideString(id) => (id, true),
            | _ => return None,
        };
        let count = if wide {
            self.context.wide_literal_units(id).count() as u64
        } else {
            self.context
                .literal_units(id)
                .iter()
                .map(|u| match u {
                    | LiteralUnit::Character(c) => c.len_utf8() as u64,
                    | LiteralUnit::Numeric(_) => 1,
                })
                .sum::<u64>()
        } + 1;
        Some((
            self.types.scalar(if wide {
                self.types.target.wchar_t
            } else {
                Scalar::Char
            }),
            count,
        ))
    }

    #[cold]
    #[inline(never)]
    pub(in crate::translation_phases::semantic_analysis) fn invalid_expression(
        &mut self,
        e: &'tu Expression<'tu>,
        kind: SemanticErrorKind,
    ) -> ExpressionInfo<'tu> {
        if !e.recovered {
            self.error(
                kind,
                e.operator_source_vectors.unwrap_or(e.source_vectors),
                None,
                None,
            );
        }
        Self::expression_result(e, self.types.unknown())
    }
}
