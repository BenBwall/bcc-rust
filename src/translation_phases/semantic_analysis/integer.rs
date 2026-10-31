//! Phase-7 integer constant evaluation used by enums, bounds and bit-fields.
//! C99: §6.6, pp. 95-96; PDF pp. 107-108; integer conversions §6.3.1.1-3,
//! pp. 42-43; PDF pp. 54-55. Typed expression folding shares this arithmetic.

use super::{
    Analyzer,
    BindingKind,
    Work,
    scopes::Namespace,
    types::{
        Scalar,
        TagKind,
        TypeId,
        TypeKind,
    },
};
use crate::translation_phases::parsing::syntax::{
    BinaryOperator,
    Constant,
    Expression,
    ExpressionType,
    UnaryOperator,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Evaluation schedules continuations instead of recursing into
    /// operands/types. C99: §6.6p3-6, p. 95; PDF p. 107.
    /// Modeled constant builtin calls are an extension under §6.6p10,
    /// p. 96; PDF p. 108.
    pub(super) fn evaluate(&mut self, expression: &'tu Expression<'tu>) {
        use ExpressionType as E;
        let info = self.expression_info(expression);
        // An out-of-range conversion has a value only as an initializer.
        if self.types.unanalyzed(info.ty) || info.atomic_cast() || info.folding.overflowed() {
            self.integers.push(None);
            return;
        }
        if self.type_generic_constant(expression) && !info.folded_address() {
            self.integers
                .push(if info.ice { info.integer } else { None });
            return;
        }
        if (self.context.configuration.gnu_extensions() || info.folded_address())
            && info.constant == super::ConstantClass::Arithmetic
            && info.integer.is_some()
        {
            // GCC folds arithmetic constants that are not integer constant
            // expressions (§6.6p6) wherever one is required.
            if !info.ice && !self.tainted && info.folded_address() {
                // Bounds apply their folding policy in ArrayDone. An abstract
                // type-name bound remains a VLA even if its value is known.
                let group = crate::configuration::DiagnosticGroup::GnuFoldingConstant;
                if !self.runtime_bound
                    && (self.context.configuration.extension_policy()
                        != crate::configuration::ExtensionPolicy::Allow
                        || self.context.configuration.diagnostic_requested(group))
                {
                    self.error(
                        super::SemanticErrorKind::FoldedIntegerConstant,
                        expression.source_vectors,
                        None,
                        None,
                    );
                }
            } else if !info.ice && !self.tainted {
                self.context.report_extension(
                    crate::configuration::Feature::ConstantFolding,
                    "folded integer constant expression",
                    expression.source_vectors,
                );
            }
            self.integers.push(info.integer);
            return;
        }
        if !self.context.configuration.gnu_extensions() && !self.valid_ice_operands(expression) {
            self.integers.push(None);
            return;
        }
        if expression.recovered {
            self.integers.push(None);
            return;
        }
        match expression.kind {
            | E::Constant(Constant::Integer(value)) => {
                use super::super::preprocessing::IntegerTokenType as I;
                let (bits, signed) = match value {
                    | I::Int(_) => (32, true),
                    | I::UnsignedInt(_) => (32, false),
                    | I::Long(_) => self.types.target.integer(Scalar::Long).unwrap(),
                    | I::LongLong(_) => (64, true),
                    | I::UnsignedLong(_) =>
                        self.types.target.integer(Scalar::UnsignedLong).unwrap(),
                    | I::UnsignedLongLong(_) => (64, false),
                    | I::BitInt(_, width, unsigned) => (u32::from(width), !unsigned),
                    | I::WideBitInt(spelling, width, unsigned) => {
                        let spelling = self.context.string_cache.at(spelling);
                        let value = self.wide.literal(spelling, width, !unsigned);
                        self.integers.push(Some(value));
                        return;
                    },
                    | I::Imaginary(..) => {
                        self.integers.push(None);
                        return;
                    },
                };
                self.integers.push(Some(Integer {
                    value: value.into(),
                    bits,
                    signed,
                    wide: None,
                }));
            },
            | E::Constant(Constant::Char(value)) => {
                let scalar = if matches!(
                    value,
                    super::super::preprocessing::CharacterTokenType::WideChar(_)
                ) {
                    self.types.target.wchar_t
                } else {
                    Scalar::Int
                };
                let (bits, signed) = self.types.target.integer(scalar).unwrap();
                let value = i128::from(value.target_value(&self.types.target));
                self.integers.push(Some(Integer {
                    value,
                    bits,
                    signed,
                    wide: None,
                }));
            },
            | E::Boolean(value) => self.integers.push(Some(Integer::int(i128::from(value)))),
            | E::Identifier(name) => {
                let value = self.lookup(Namespace::Ordinary, name.name).and_then(|e| {
                    let binding = self.bindings[e.binding];
                    (binding.kind == BindingKind::Enumerator)
                        .then_some(binding.value)
                        .flatten()
                });
                self.integers.push(value);
            },
            | E::Parenthesized { expression } => self.work.push(Work::Eval(expression)),
            | E::Unary {
                operator,
                operand_expression,
            } => {
                if !matches!(
                    operator,
                    UnaryOperator::Plus
                        | UnaryOperator::Minus
                        | UnaryOperator::BitwiseNot
                        | UnaryOperator::LogicalNot
                        | UnaryOperator::Extension
                ) {
                    self.integers.push(None);
                    return;
                }
                self.work
                    .push(Work::Unary(operator, expression.source_vectors, info.ty));
                self.work.push(Work::Eval(operand_expression));
            },
            | E::Binary {
                operator,
                left_expression,
                right_expression,
            } => {
                if matches!(
                    operator,
                    BinaryOperator::LogicalAnd | BinaryOperator::LogicalOr
                ) {
                    self.work.push(Work::Logical(
                        operator,
                        right_expression,
                        expression.source_vectors,
                    ));
                    self.work.push(Work::Eval(left_expression));
                } else if matches!(
                    operator,
                    BinaryOperator::Comma
                        | BinaryOperator::Subscript
                        | BinaryOperator::Assignment
                        | BinaryOperator::MultiplicationAssignment
                        | BinaryOperator::DivisionAssignment
                        | BinaryOperator::ModuloAssignment
                        | BinaryOperator::AdditionAssignment
                        | BinaryOperator::SubtractionAssignment
                        | BinaryOperator::LeftShiftAssignment
                        | BinaryOperator::RightShiftAssignment
                        | BinaryOperator::BitwiseAndAssignment
                        | BinaryOperator::BitwiseXorAssignment
                        | BinaryOperator::BitwiseOrAssignment
                ) {
                    self.integers.push(None);
                } else {
                    self.work
                        .push(Work::Binary(operator, expression.source_vectors, info.ty));
                    self.work.push(Work::Eval(right_expression));
                    self.work.push(Work::Eval(left_expression));
                }
            },
            | E::Conditional(c) | E::OmittedConditional(c) => {
                self.work.push(Work::Conditional(c));
                self.work.push(Work::Eval(c.condition_expression));
            },
            | E::Cast {
                target_type,
                operand_expression,
            } => {
                self.work.push(Work::CastBase(
                    operand_expression,
                    expression.source_vectors,
                ));
                self.work.push(Work::TypeName(target_type, false));
            },
            | E::SizeofType(name) | E::AlignofType(name) => {
                self.work.push(Work::SizeofDone(
                    matches!(expression.kind, E::AlignofType(_)),
                    expression.source_vectors,
                ));
                self.work.push(Work::TypeName(name, false));
            },
            | E::Builtin(_)
            | E::Call { .. }
            | E::SizeofExpr(_)
            | E::AlignofExpr(_)
            | E::Generic(_) => {
                let info = self.expression_info(expression);
                self.integers
                    .push(if info.ice { info.integer } else { None });
            },
            | _ => self.integers.push(None),
        }
    }

    /// ICE operand restrictions apply even in an unselected conditional arm.
    /// Cache by syntax identity and immediate-floating-cast context so deep
    /// conditionals do not repeatedly validate their descendants.
    /// C99: §6.6p6, p. 95; PDF p. 107.
    /// Modeled constant builtin calls are an extension under §6.6p10,
    /// p. 96; PDF p. 108.
    pub(super) fn valid_ice_operands(&mut self, expression: &'tu Expression<'tu>) -> bool {
        let mut work = super::ArenaVec::new_in(self.scratch);
        let mut values = super::ArenaVec::new_in(self.scratch);
        work.push((expression, false, None));
        while let Some((expression, floating, ready)) = work.pop() {
            let key = (std::ptr::from_ref(expression).addr(), floating);
            if let Some(children) = ready {
                let mut valid = true;
                for _ in 0..children {
                    valid &= values.pop().unwrap_or(false);
                }
                valid &= match expression.kind {
                    | ExpressionType::Identifier(name) => self
                        .lookup(Namespace::Ordinary, name.name)
                        .is_some_and(|e| self.bindings[e.binding].kind == BindingKind::Enumerator),
                    | ExpressionType::Constant(Constant::Float(_)) => floating,
                    | ExpressionType::Cast { target_type, .. } => !self
                        .resolved_type_names
                        .get(&super::type_name_identity(target_type))
                        .is_some_and(|ty| {
                            matches!(self.types.nodes[ty.index], TypeKind::Atomic(_))
                        }),
                    | _ => true,
                };
                _ = self.ice_operands.insert(key, valid);
                values.push(valid);
                continue;
            }
            if let Some(&valid) = self.ice_operands.get(&key) {
                values.push(valid);
                continue;
            }
            if matches!(expression.kind, ExpressionType::Generic(_))
                || self.type_generic_constant(expression)
            {
                let info = self.expression_info(expression);
                let valid = info.ice && info.integer.is_some();
                _ = self.ice_operands.insert(key, valid);
                values.push(valid);
                continue;
            }
            if matches!(expression.kind, ExpressionType::Call { .. })
                && self.expression_info(expression).ice
            {
                values.push(true);
                continue;
            }
            let children = match expression.kind {
                | ExpressionType::Parenthesized { .. }
                | ExpressionType::Unary { .. }
                | ExpressionType::Cast { .. }
                | ExpressionType::DirectMember { .. }
                | ExpressionType::IndirectMember { .. } => 1,
                | ExpressionType::Binary { .. } => 2,
                | ExpressionType::Conditional(_) | ExpressionType::OmittedConditional(_) => 3,
                | ExpressionType::Call { arguments, .. } => arguments.len() + 1,
                | _ => 0,
            };
            work.push((expression, floating, Some(children)));
            match expression.kind {
                | ExpressionType::Parenthesized { expression } =>
                    work.push((expression, floating, None)),
                | ExpressionType::Unary {
                    operand_expression, ..
                } => work.push((operand_expression, false, None)),
                | ExpressionType::Cast {
                    target_type,
                    operand_expression,
                } => {
                    let floating = self
                        .resolved_type_names
                        .get(&super::type_name_identity(target_type))
                        .is_some_and(|&ty| self.integer_type(ty).is_some());
                    work.push((operand_expression, floating, None));
                },
                | ExpressionType::DirectMember {
                    base_expression, ..
                }
                | ExpressionType::IndirectMember {
                    base_expression, ..
                } => work.push((base_expression, false, None)),
                | ExpressionType::Binary {
                    left_expression,
                    right_expression,
                    ..
                } => {
                    work.push((right_expression, false, None));
                    work.push((left_expression, false, None));
                },
                | ExpressionType::Conditional(c) | ExpressionType::OmittedConditional(c) => {
                    work.extend([
                        (c.else_expression, false, None),
                        (c.then_expression, false, None),
                        (c.condition_expression, false, None),
                    ]);
                },
                | ExpressionType::Call {
                    function_expression,
                    arguments,
                } => {
                    for &argument in arguments.iter().rev() {
                        work.push((argument, false, None));
                    }
                    work.push((function_expression, false, None));
                },
                | _ => {},
            }
        }
        values.pop().unwrap_or(false)
    }

    /// C99: §6.3.1.1p2, pp. 42-43; PDF pp. 54-55; §6.3.1.8, p. 45; PDF p. 57;
    /// §6.5.17p2, p. 94; PDF p. 106 (comma keeps the right operand's type).
    fn integer_model(&mut self, expression: &'tu Expression<'tu>) -> Option<(u32, bool)> {
        let mut pending = super::ArenaVec::new_in(self.scratch);
        let mut values = super::ArenaVec::new_in(self.scratch);
        pending.push((expression, false));
        while let Some((expression, ready)) = pending.pop() {
            let key = std::ptr::from_ref(expression).addr();
            if !ready {
                if let Some(&model) = self.integer_models.get(&key) {
                    values.push(model);
                    continue;
                }
                pending.push((expression, true));
                match expression.kind {
                    | ExpressionType::Parenthesized { expression }
                    | ExpressionType::Unary {
                        operand_expression: expression,
                        ..
                    } => pending.push((expression, false)),
                    | ExpressionType::Binary {
                        left_expression,
                        right_expression,
                        ..
                    } => {
                        pending.push((right_expression, false));
                        pending.push((left_expression, false));
                    },
                    | ExpressionType::Conditional(c) | ExpressionType::OmittedConditional(c) => {
                        pending.push((c.else_expression, false));
                        pending.push((c.then_expression, false));
                    },
                    | _ => {},
                }
                continue;
            }
            let model = match expression.kind {
                | ExpressionType::Constant(Constant::Integer(value)) => {
                    use super::super::preprocessing::IntegerTokenType as I;
                    match value {
                        | I::Int(_) => Some((32, true)),
                        | I::UnsignedInt(_) => Some((32, false)),
                        | I::Long(_) => self.types.target.integer(Scalar::Long),
                        | I::LongLong(_) => Some((64, true)),
                        | I::UnsignedLong(_) => self.types.target.integer(Scalar::UnsignedLong),
                        | I::UnsignedLongLong(_) => Some((64, false)),
                        | _ => None,
                    }
                },
                | ExpressionType::Constant(Constant::Char(
                    super::super::preprocessing::CharacterTokenType::WideChar(_),
                )) => self.types.target.integer(self.types.target.wchar_t),
                | ExpressionType::Constant(Constant::Char(_)) | ExpressionType::Boolean(_) =>
                    Some((32, true)),
                | ExpressionType::Identifier(name) => self
                    .lookup(Namespace::Ordinary, name.name)
                    .and_then(|e| self.integer_type(self.bindings[e.binding].ty)),
                | ExpressionType::Call { .. } if self.expression_info(expression).ice =>
                    self.integer_type(self.expression_info(expression).ty),
                | ExpressionType::Parenthesized { .. } => values.pop().flatten(),
                | ExpressionType::Unary { operator, .. } => values.pop().flatten().map(|model| {
                    if operator == UnaryOperator::LogicalNot || model.0 < 32 {
                        (32, true)
                    } else {
                        model
                    }
                }),
                | ExpressionType::Binary { operator, .. } => {
                    let right = values.pop().flatten();
                    let left = values.pop().flatten();
                    if operator == BinaryOperator::Comma {
                        _ = self.integer_models.insert(key, right);
                        values.push(right);
                        continue;
                    }
                    left.zip(right).map(|(left, right)| {
                        if matches!(
                            operator,
                            BinaryOperator::LogicalAnd
                                | BinaryOperator::LogicalOr
                                | BinaryOperator::Equal
                                | BinaryOperator::NotEqual
                                | BinaryOperator::LessThan
                                | BinaryOperator::LessThanOrEqual
                                | BinaryOperator::GreaterThan
                                | BinaryOperator::GreaterThanOrEqual
                        ) {
                            (32, true)
                        } else if matches!(
                            operator,
                            BinaryOperator::LeftShift | BinaryOperator::RightShift
                        ) {
                            if left.0 < 32 { (32, true) } else { left }
                        } else {
                            common(left, right)
                        }
                    })
                },
                | ExpressionType::Conditional(_) | ExpressionType::OmittedConditional(_) => {
                    let right = values.pop().flatten();
                    let left = values.pop().flatten();
                    left.zip(right).map(|(left, right)| common(left, right))
                },
                | ExpressionType::SizeofType(_)
                | ExpressionType::AlignofType(_)
                | ExpressionType::SizeofExpr(_)
                | ExpressionType::AlignofExpr(_) =>
                    self.types.target.integer(self.types.target.size_t),
                | ExpressionType::Cast { target_type, .. } => {
                    use super::TypeSpecifiers as S;
                    if let Some(&ty) = self
                        .resolved_type_names
                        .get(&super::type_name_identity(target_type))
                    {
                        let model = self.integer_type(ty);
                        _ = self.integer_models.insert(key, model);
                        values.push(model);
                        continue;
                    }
                    if target_type.declarator.is_some() {
                        _ = self.integer_models.insert(key, None);
                        values.push(None);
                        continue;
                    }
                    match target_type.declaration_specifiers.type_specifiers {
                        | S::Bool => Some((1, false)),
                        | S::Char | S::SignedChar => Some((8, true)),
                        | S::UnsignedChar => Some((8, false)),
                        | S::Short | S::ShortInt | S::SignedShort | S::SignedShortInt =>
                            Some((16, true)),
                        | S::UnsignedShort | S::UnsignedShortInt => Some((16, false)),
                        | S::Int | S::Signed | S::SignedInt => Some((32, true)),
                        | S::Unsigned | S::UnsignedInt => Some((32, false)),
                        | S::Long | S::LongInt | S::SignedLong | S::SignedLongInt =>
                            self.types.target.integer(Scalar::Long),
                        | S::LongLong
                        | S::LongLongInt
                        | S::SignedLongLong
                        | S::SignedLongLongInt => Some((64, true)),
                        | S::UnsignedLong | S::UnsignedLongInt =>
                            self.types.target.integer(Scalar::UnsignedLong),
                        | S::UnsignedLongLong | S::UnsignedLongLongInt => Some((64, false)),
                        | S::Extended(super::ExtendedType::Int128 { signedness }) =>
                            Some((128, signedness != &Some(false))),
                        | S::TypedefName(name) => self
                            .lookup(Namespace::Ordinary, name.name)
                            .and_then(|e| self.integer_type(self.bindings[e.binding].ty)),
                        | _ => None,
                    }
                },
                | ExpressionType::Generic(_) | ExpressionType::Call { .. } =>
                    self.integer_type(self.expression_info(expression).ty),
                | _ => None,
            };
            _ = self.integer_models.insert(key, model);
            values.push(model);
        }
        values.pop().flatten()
    }

    /// Computes the conditional's integer common type without evaluating either
    /// arm. C99: §6.5.15p5, p. 90; PDF p. 102; §6.3.1.8, p. 45; PDF p. 57.
    pub(super) fn conditional_model(
        &mut self,
        conditional: &'tu super::ConditionalExpression<'tu>,
    ) -> Option<(u32, bool)> {
        Some(common(
            self.integer_model(conditional.then_expression)?,
            self.integer_model(conditional.else_expression)?,
        ))
    }

    /// Immediate floating operands are allowed in an integer-constant cast.
    /// C99: §6.6p6, p. 95; PDF p. 107; §6.3.1.4p1, p. 43; PDF p. 55.
    pub(super) fn float_cast(
        &self,
        ty: TypeId,
        value: super::super::preprocessing::FloatTokenType,
    ) -> Option<Integer> {
        use super::super::preprocessing::FloatTokenType as F;
        let (bits, signed) = self.integer_type(ty)?;
        let (negative, mantissa, shift) = match value {
            | F::Float(value) | F::Float32(value) => float_parts(f64::from(value)),
            | F::Float16(value) => float_parts(f64::from(
                self.half_constant(value, crate::half_precision::HalfFormat::Binary16),
            )),
            | F::BFloat16(value) => float_parts(f64::from(
                self.half_constant(value, crate::half_precision::HalfFormat::BFloat16),
            )),
            | F::Double(value) | F::Float64(value) | F::Float32x(value) => float_parts(value.get()),
            | F::LongDouble(value) | F::Float64x(value) => {
                let text = crate::diagnostics::format_in!(self.scratch, "{value}");
                let negative = text.starts_with('-');
                let text = text.trim_start_matches('-').strip_prefix("0x")?;
                let (digits, exponent) = text.split_once('p')?;
                let exponent = exponent.parse::<i32>().ok()?;
                let mut mantissa = 0_u128;
                let mut fractional = 0;
                let mut after_point = false;
                for digit in digits.chars() {
                    if digit == '.' {
                        after_point = true;
                        continue;
                    }
                    if mantissa > (u128::MAX >> 4) {
                        return None;
                    }
                    mantissa = (mantissa << 4).checked_add(u128::from(digit.to_digit(16)?))?;
                    if after_point {
                        fractional += 4;
                    }
                }
                (negative, mantissa, exponent - fractional)
            },
            | F::Float128(value) | F::LongDouble128(value) => match value.class() {
                | crate::binary128::Class::Zero => (value.is_negative(), 0, 0),
                | crate::binary128::Class::Finite {
                    exponent,
                    significand,
                } => (value.is_negative(), significand, exponent),
                | _ => return None,
            },
            | _ => return None,
        };
        if matches!(self.types.nodes[ty.index], TypeKind::Scalar(Scalar::Bool)) {
            return Some(Integer {
                value:  i128::from(mantissa != 0),
                bits:   1,
                signed: false,
                wide:   None,
            });
        }
        if bits > 128 {
            return self.wide.truncated(negative, mantissa, shift, bits, signed);
        }
        let magnitude = if shift >= 0 {
            let shift = u32::try_from(shift).ok()?;
            if shift >= 128 || mantissa > (u128::MAX >> shift) {
                return None;
            }
            mantissa << shift
        } else if shift <= -128 {
            0
        } else {
            mantissa >> shift.unsigned_abs()
        };
        let model = Integer {
            value: 0,
            bits,
            signed,
            wide: None,
        };
        let limit = if signed {
            1_u128 << (bits - 1)
        } else {
            model.mask()
        };
        if magnitude > limit
            || (signed && !negative && magnitude == limit)
            || (!signed && negative && magnitude != 0)
        {
            return None;
        }
        let raw = if negative {
            magnitude.wrapping_neg()
        } else {
            magnitude
        };
        Some(model.cast_value(raw as i128))
    }

    /// The value GCC and Clang store for a floating constant converted to
    /// the integer type `ty`, and whether it was out of range: the value
    /// truncated toward zero when it fits, else 0 for a NaN and the
    /// nearest bound of the range of `width` bits (of `ty` when `None`),
    /// as both compilers fold the conversion (GCC
    /// `fold_convert_const_int_from_real`, Clang's evaluator through
    /// `APFloat::convertToInteger`). The behavior is undefined in C; this is
    /// the value both choose. `_Bool` takes 1 for any nonzero value.
    /// C99: §6.3.1.4 paragraph 1, p. 43; PDF p. 55; §6.3.1.2 paragraph 1,
    /// p. 43; PDF p. 55.
    pub(super) fn saturated_float_cast(
        &self,
        ty: TypeId,
        value: super::super::preprocessing::FloatTokenType,
        width: Option<u32>,
    ) -> Option<(Integer, bool)> {
        use super::super::preprocessing::FloatTokenType as F;
        let (full, signed) = self.integer_type(ty)?;
        let bits = width.unwrap_or(full).min(full);
        let (negative, nan) = match value {
            | F::Float(value) | F::Float32(value) => (value.is_sign_negative(), value.is_nan()),
            | F::Float16(value) => {
                let value = self.half_constant(value, crate::half_precision::HalfFormat::Binary16);
                (value.is_sign_negative(), value.is_nan())
            },
            | F::BFloat16(value) => {
                let value = self.half_constant(value, crate::half_precision::HalfFormat::BFloat16);
                (value.is_sign_negative(), value.is_nan())
            },
            | F::Double(value) | F::Float64(value) | F::Float32x(value) =>
                (value.get().is_sign_negative(), value.get().is_nan()),
            | F::LongDouble(value) | F::Float64x(value) => {
                let (negative, exponent, significand) = value.x87_parts()?;
                (negative, exponent == 0x7FFF && significand != 1 << 63)
            },
            | F::Float128(value) | F::LongDouble128(value) => (
                value.is_negative(),
                matches!(value.class(), crate::binary128::Class::Nan { .. }),
            ),
            | _ => return None,
        };
        if matches!(
            self.types.nodes[self.types.non_atomic(ty).index],
            TypeKind::Scalar(Scalar::Bool)
        ) {
            // Only a NaN or an infinity is no finite value float_cast reads.
            return Some((
                self.float_cast(ty, value).unwrap_or(Integer {
                    value:  1,
                    bits:   1,
                    signed: false,
                    wide:   None,
                }),
                false,
            ));
        }
        if let Some(converted) = self.float_cast(ty, value)
            && (width.is_none()
                || self
                    .wide
                    .compare(self.wide.convert(converted, bits, signed), converted)
                    .is_eq())
        {
            return Some((converted, false));
        }
        let bound = if nan {
            Integer::int(0)
        } else if bits > 128 {
            self.wide.bound(bits, signed, negative)
        } else {
            let value = if negative {
                if signed { i128::MIN >> (128 - bits) } else { 0 }
            } else if signed {
                i128::MAX >> (128 - bits)
            } else {
                (u128::MAX >> (128 - bits)).cast_signed()
            };
            Integer {
                value,
                bits,
                signed,
                wide: None,
            }
        };
        Some((self.wide.convert(bound, full, signed), true))
    }

    /// The spelling of an arithmetic type in a conversion diagnostic: its
    /// keywords, or `enum` for an enumeration.
    pub(super) fn conversion_spelling(&self, ty: TypeId) -> &'static str {
        match self.types.nodes[self.types.non_atomic(ty).index] {
            | TypeKind::Scalar(scalar) => scalar.spelling(),
            | TypeKind::BitInt { signed: true, .. } => "_BitInt",
            | TypeKind::BitInt { signed: false, .. } => "unsigned _BitInt",
            | TypeKind::Tag(_) => "enum",
            | _ => "integer",
        }
    }

    /// The width and signedness of an integer type; a bit-precise type's
    /// width is the N it names, not its storage's.
    /// C99: §6.2.5p2-9, pp. 33-34; PDF pp. 45-46.
    /// C23: §6.2.5 paragraphs 5-8, p. 39; PDF p. 52.
    pub(super) fn integer_type(&self, ty: TypeId) -> Option<(u32, bool)> {
        let ty = self.types.non_atomic(ty);
        match self.types.nodes[ty.index] {
            | TypeKind::Scalar(Scalar::Bool) => Some((1, false)),
            | TypeKind::BitInt { width, signed } => Some((width, signed)),
            | TypeKind::Scalar(s) => self.types.target.integer(s),
            | TypeKind::Tag(id)
                if self.types.tags[id].kind == TagKind::Enum
                    && !self.types.tags[id].tainted.get() =>
                self.types
                    .target
                    .integer(self.types.tags[id].compatible.get()),
            | _ => None,
        }
    }

    /// Unsupported expression semantics must not be mistaken for a VLA or an
    /// invalid ICE. C99: §6.6p6, p. 95; PDF p. 107; extensions follow §4p6,
    /// p. 7; PDF p. 19.
    pub(super) fn unanalyzed_constant(&self, expression: &'tu Expression<'tu>) -> bool {
        if self
            .expression_indices
            .get(&std::ptr::from_ref(expression).addr())
            .is_some_and(|&i| self.types.unanalyzed(self.expressions[i].ty))
        {
            return true;
        }
        let mut pending = super::ArenaVec::new_in(self.scratch);
        pending.push(expression);
        while let Some(expression) = pending.pop() {
            match expression.kind {
                | ExpressionType::Constant(
                    Constant::Integer(super::super::preprocessing::IntegerTokenType::Imaginary(..))
                    | Constant::Float(
                        super::super::preprocessing::FloatTokenType::ImaginaryFloat(_)
                        | super::super::preprocessing::FloatTokenType::ImaginaryDouble(_)
                        | super::super::preprocessing::FloatTokenType::ImaginaryLongDouble(_)
                        | super::super::preprocessing::FloatTokenType::ImaginaryLongDouble128(_)
                        | super::super::preprocessing::FloatTokenType::ImaginaryFloat16(_)
                        | super::super::preprocessing::FloatTokenType::ImaginaryFloat32(_)
                        | super::super::preprocessing::FloatTokenType::ImaginaryFloat64(_)
                        | super::super::preprocessing::FloatTokenType::ImaginaryFloat32x(_)
                        | super::super::preprocessing::FloatTokenType::ImaginaryFloat64x(_),
                    ),
                )
                | ExpressionType::Countof(_)
                | ExpressionType::Nullptr => return true,
                | ExpressionType::Builtin(b) if !super::builtins::modeled(b.keyword) =>
                    return true,
                | ExpressionType::Identifier(name) => {
                    if self
                        .lookup(Namespace::Ordinary, name.name)
                        .is_some_and(|e| self.bindings[e.binding].ty.index == 0)
                    {
                        return true;
                    }
                },
                | ExpressionType::Parenthesized { expression }
                | ExpressionType::Unary {
                    operand_expression: expression,
                    ..
                } => pending.push(expression),
                | ExpressionType::Binary {
                    left_expression,
                    right_expression,
                    ..
                } => {
                    pending.push(left_expression);
                    pending.push(right_expression);
                },
                | ExpressionType::Conditional(c) | ExpressionType::OmittedConditional(c) => {
                    pending.extend([c.condition_expression, c.then_expression, c.else_expression]);
                },
                | ExpressionType::Cast {
                    target_type,
                    operand_expression,
                } => {
                    if self
                        .resolved_type_names
                        .get(&super::type_name_identity(target_type))
                        .is_some_and(|&ty| self.types.unanalyzed(ty))
                    {
                        return true;
                    }
                    if unanalyzed_extended(target_type.declaration_specifiers.type_specifiers) {
                        return true;
                    }
                    pending.push(operand_expression);
                },
                | ExpressionType::SizeofType(name) | ExpressionType::AlignofType(name) => {
                    if self
                        .resolved_type_names
                        .get(&super::type_name_identity(name))
                        .is_some_and(|&ty| self.types.unanalyzed(ty))
                    {
                        return true;
                    }
                    if unanalyzed_extended(name.declaration_specifiers.type_specifiers) {
                        return true;
                    }
                },
                | _ => {},
            }
        }
        false
    }
}

/// Target integer value, including width and signedness for
/// overflow/conversions. C99: §6.6p4, p. 95; PDF p. 107; §6.3.1.3, p. 43; PDF
/// p. 55.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Integer {
    /// Sign-extended signed value, or the raw unsigned 128-bit pattern of a
    /// type of at most 128 bits. A type wider than 128 bits keeps its exact
    /// value here when `i128` holds it, else the low 128 bits of its two's
    /// complement, with the whole value in `wide`. Interpret together with
    /// `signed`; use Display and checked accessors.
    pub(crate) value:  i128,
    pub(crate) bits:   u32,
    pub(crate) signed: bool,
    /// The exact value's entry in the unit's table, for a bit-precise type
    /// wider than 128 bits whose value `i128` does not hold
    /// ([`super::WideIntegers`]).
    pub(crate) wide:   Option<super::WideId>,
}

impl Integer {
    /// Usual integer conversions and checked signed/modular unsigned
    /// arithmetic on operands of at most 128 bits; `None` for a wider one,
    /// which [`super::WideIntegers::binary`] folds.
    /// C99: §6.3.1.8p1, pp. 44-45; PDF pp. 56-57; §6.5.5p6, p. 82; PDF p. 94.
    pub(crate) fn binary(self, op: BinaryOperator, right: Self) -> Option<Self> {
        use BinaryOperator as B;
        if self.wide() || right.wide() {
            return None;
        }
        let (mut l, mut r) = (self.promote(), right.promote());
        if matches!(op, B::LogicalAnd | B::LogicalOr) {
            return Some(Self::int(i128::from(if op == B::LogicalAnd {
                l.value != 0 && r.value != 0
            } else {
                l.value != 0 || r.value != 0
            })));
        }
        if matches!(op, B::LeftShift | B::RightShift) {
            let shift = u32::try_from(r.to_i128()?).ok().filter(|&n| n < l.bits)?;
            if op == B::RightShift {
                return Some(l.cast_value(if l.signed {
                    l.value >> shift
                } else {
                    ((l.value as u128) >> shift) as i128
                }));
            }
            if l.signed && (l.value < 0 || (l.value as u128) > ((l.maximum() as u128) >> shift)) {
                return None;
            }
            return Some(l.cast_value(((l.value as u128) << shift) as i128));
        }
        let (bits, signed) = common((l.bits, l.signed), (r.bits, r.signed));
        l = l.cast(bits, signed);
        r = r.cast(bits, signed);
        let comparison = l.order_key().cmp(&r.order_key());
        let predicate = match op {
            | B::LessThan => Some(comparison.is_lt()),
            | B::LessThanOrEqual => Some(!comparison.is_gt()),
            | B::GreaterThan => Some(comparison.is_gt()),
            | B::GreaterThanOrEqual => Some(!comparison.is_lt()),
            | B::Equal => Some(comparison.is_eq()),
            | B::NotEqual => Some(!comparison.is_eq()),
            | _ => None,
        };
        if let Some(result) = predicate {
            return Some(Self::int(i128::from(result)));
        }
        let (a, b) = (l.value as u128, r.value as u128);
        let value = match op {
            | B::Addition if signed => l.value.checked_add(r.value)?,
            | B::Subtraction if signed => l.value.checked_sub(r.value)?,
            | B::Multiplication if signed => l.value.checked_mul(r.value)?,
            | B::Addition => a.wrapping_add(b) as i128,
            | B::Subtraction => a.wrapping_sub(b) as i128,
            | B::Multiplication => a.wrapping_mul(b) as i128,
            | B::Division | B::Modulo => {
                if b == 0 || (signed && l.value == l.minimum() && r.value == -1) {
                    return None;
                }
                if signed {
                    if op == B::Division {
                        l.value.checked_div(r.value)?
                    } else {
                        l.value.checked_rem(r.value)?
                    }
                } else if op == B::Division {
                    (a / b) as i128
                } else {
                    (a % b) as i128
                }
            },
            | B::BitwiseAnd => l.value & r.value,
            | B::BitwiseOr => l.value | r.value,
            | B::BitwiseXor => l.value ^ r.value,
            | _ => return None,
        };
        l.checked(value)
    }

    /// The unary operator on a value of at most 128 bits; `None` for a
    /// wider one, which [`super::WideIntegers::unary`] folds.
    /// C99: §6.5.3.3p1-5, p. 79; PDF p. 91.
    pub(crate) fn unary(self, op: UnaryOperator) -> Option<Self> {
        if self.wide() {
            return None;
        }
        let v = self.promote();
        match op {
            | UnaryOperator::Plus | UnaryOperator::Extension => Some(v),
            | UnaryOperator::Minus if v.signed => v.checked(v.value.checked_neg()?),
            | UnaryOperator::Minus => Some(v.cast_value(v.value.wrapping_neg())),
            | UnaryOperator::BitwiseNot => Some(v.cast_value(!v.value)),
            | UnaryOperator::LogicalNot => Some(Self::int(i128::from(v.value == 0))),
            | _ => None,
        }
    }

    /// The value converted to an integer type of at most 128 bits, which
    /// reads only its low 128 bits, so it needs no table for a wide value.
    /// C99: §6.3.1.3p1-3, p. 43; PDF p. 55.
    pub(crate) fn cast(self, bits: u32, signed: bool) -> Self {
        debug_assert!(
            bits <= 128,
            "a conversion to a wider bit-precise type uses WideIntegers::convert"
        );
        Self {
            bits,
            signed,
            wide: None,
            ..self
        }
        .cast_value(self.value)
    }

    /// Whether this value belongs to a bit-precise type wider than 128
    /// bits, whose arithmetic takes [`super::WideIntegers`].
    pub(crate) const fn wide(self) -> bool {
        self.bits > 128
    }

    /// Whether the value is zero; a value only the table holds never is.
    pub(crate) const fn is_zero(self) -> bool {
        self.wide.is_none() && self.value == 0
    }

    /// C99: §6.3.1.1p2, pp. 42-43; PDF pp. 54-55.
    fn promote(self) -> Self {
        if self.bits < 32 {
            Self::int(self.value)
        } else {
            self
        }
    }

    /// The next implicit enumerator value, which never wraps.
    /// C99: §6.7.2.2p3, p. 105; PDF p. 117.
    pub(crate) fn increment(self) -> Option<Self> {
        // Enumerator values of more than 64 bits are refused before this.
        if self.wide() {
            return None;
        }
        if self.signed {
            self.checked(self.value.checked_add(1)?)
        } else {
            let raw = (self.value as u128).checked_add(1)?;
            (raw <= self.mask()).then_some(Self {
                value: raw as i128,
                ..self
            })
        }
    }

    /// Truncate bits, then sign-extend only signed results. At width 128,
    /// value is a raw two's-complement pattern for unsigned high-half values.
    /// C99: §6.3.1.3p2-3, p. 43; PDF p. 55.
    fn cast_value(self, value: i128) -> Self {
        let raw = (value as u128) & self.mask();
        let value = if self.signed {
            ((raw << (128 - self.bits)) as i128) >> (128 - self.bits)
        } else {
            raw as i128
        };
        Self { value, ..self }
    }

    /// C99: §6.6p4, p. 95; PDF p. 107.
    fn checked(self, value: i128) -> Option<Self> {
        if self.signed {
            (value >= self.minimum() && value <= self.maximum()).then_some(Self { value, ..self })
        } else {
            Some(self.cast_value(value))
        }
    }

    /// GNU extension: folds a nonnegative signed left shift into the sign bit.
    /// GNU extension: GCC manual, "Integers".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Integers-implementation.html>
    /// C99: §6.5.7 paragraph 4, p. 85; PDF p. 97.
    pub(crate) fn sign_bit_shift(self, op: BinaryOperator, right: Self) -> Option<Self> {
        if self.wide() || right.wide() {
            return None;
        }
        let (l, r) = (self.promote(), right.promote());
        if op != BinaryOperator::LeftShift || !l.signed || l.value < 0 {
            return None;
        }
        let shift = u32::try_from(r.to_i128()?).ok().filter(|&n| n < l.bits)?;
        let raw = l.value as u128;
        (raw <= (l.mask() >> shift)).then(|| l.cast_value((raw << shift) as i128))
    }

    /// GNU extension: folds a signed left shift of a negative value, or one
    /// past the sign bit, to its two's-complement result, which GCC
    /// documents and computes in constant expressions too (Clang folds it
    /// with a warning). The second result is whether the value was
    /// negative. Counts outside the width stay no constant.
    /// GNU extension: GCC manual, "Integers".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Integers-implementation.html>
    /// C99: §6.5.7 paragraph 4, p. 85; PDF p. 97.
    pub(crate) fn wrapping_shift(self, op: BinaryOperator, right: Self) -> Option<(Self, bool)> {
        if self.wide() || right.wide() {
            return None;
        }
        let (l, r) = (self.promote(), right.promote());
        if op != BinaryOperator::LeftShift || !l.signed {
            return None;
        }
        let shift = u32::try_from(r.to_i128()?).ok().filter(|&n| n < l.bits)?;
        Some((
            l.cast_value(((l.value as u128) << shift) as i128),
            l.value < 0,
        ))
    }

    pub(crate) const fn int(value: i128) -> Self {
        Self {
            value,
            bits: 32,
            signed: true,
            wide: None,
        }
    }

    /// Mathematical value when it fits i128. Unsigned high-half values must
    /// never masquerade as negative bounds, addresses or enumerators.
    /// C99: §6.3.1.3, p. 43; PDF p. 55.
    pub(crate) fn to_i128(self) -> Option<i128> {
        (self.wide.is_none() && (self.signed || self.value >= 0)).then_some(self.value)
    }

    /// Checked nonnegative size/index extraction, without unsigned truncation.
    /// C99: §6.6p4, p. 95; PDF p. 107; §6.3.1.3, p. 43; PDF p. 55.
    pub(crate) fn to_u64(self) -> Option<u64> {
        self.to_i128().and_then(|n| u64::try_from(n).ok())
    }

    /// Ordered key retaining all unsigned bits, for switch case intervals.
    /// C99: §6.8.4.2p3, p. 134; PDF p. 146.
    pub(crate) fn order_key(self) -> i128 {
        if self.signed {
            self.value
        } else {
            self.value ^ i128::MIN
        }
    }

    fn mask(self) -> u128 {
        u128::MAX >> (128 - self.bits)
    }

    fn maximum(self) -> i128 {
        i128::MAX >> (128 - self.bits)
    }

    fn minimum(self) -> i128 {
        i128::MIN >> (128 - self.bits)
    }
}

/// Extension type specifiers this phase leaves unanalyzed; MSVC sized
/// integers and GNU 128-bit integers have modeled integer types.
/// C99: §6.6p6, p. 95; PDF p. 107 (integer constant expressions).
/// C99: §4p6, p. 7; PDF p. 19 (extensions).
fn unanalyzed_extended(specifiers: super::TypeSpecifiers<'_>) -> bool {
    matches!(
        specifiers,
        super::TypeSpecifiers::Extended(extended)
            if !matches!(
                extended,
                super::ExtendedType::MsInteger { .. }
                    | super::ExtendedType::Int128 { .. }
                    | super::ExtendedType::BitInt { .. }
                    | super::ExtendedType::Atomic(_)
                    | super::ExtendedType::Float128 { .. }
                    | super::ExtendedType::InterchangeFloat { .. }
                    | super::ExtendedType::ComplexInteger(_)
                    | super::ExtendedType::Typeof { .. }
            )
    )
}

/// Applies the usual arithmetic conversions to promoted integer values.
/// C99: §6.3.1.8 paragraph 1, pp. 44-45; PDF pp. 56-57.
pub(super) fn common(mut left: (u32, bool), mut right: (u32, bool)) -> (u32, bool) {
    if left.0 < 32 {
        left = (32, true);
    }
    if right.0 < 32 {
        right = (32, true);
    }
    let bits = left.0.max(right.0);
    let signed = if left.1 == right.1 {
        left.1
    } else {
        let (signed, unsigned) = if left.1 { (left, right) } else { (right, left) };
        signed.0 > unsigned.0
    };
    (bits, signed)
}

fn float_parts(value: f64) -> (bool, u128, i32) {
    let raw = value.to_bits();
    let negative = raw >> 63 != 0;
    let exponent = i32::try_from((raw >> 52) & 0x7FF).unwrap_or(0);
    let mantissa = u128::from(raw & ((1_u64 << 52) - 1));
    if exponent == 0 {
        (negative, mantissa, -1074)
    } else {
        (negative, mantissa | (1_u128 << 52), exponent - 1023 - 52)
    }
}

impl std::fmt::Display for Integer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.signed {
            self.value.fmt(f)
        } else {
            (self.value as u128).fmt(f)
        }
    }
}
