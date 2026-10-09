//! Phase-7 integer constant evaluation used by enums, bounds and bit-fields.
//! C99: §6.6, pp. 95-96; PDF pp. 107-108; integer conversions §6.3.1.1-3,
//! pp. 42-43; PDF pp. 54-55. Typed expression folding shares this arithmetic.

use super::{
    Analyzer,
    BinaryOperator,
    BindingKind,
    Constant,
    Expression,
    ExpressionType,
    Namespace,
    TagKind,
    TypeId,
    TypeKind,
    UnaryOperator,
    Work,
};

/// Target integer value, including width and signedness for
/// overflow/conversions. C99: §6.6p4, p. 95; PDF p. 107; §6.3.1.3, p. 43; PDF
/// p. 55.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Integer {
    pub(crate) value:  i128,
    pub(crate) bits:   u32,
    pub(crate) signed: bool,
}

impl Integer {
    pub(crate) const fn int(value: i128) -> Self {
        Self {
            value,
            bits: 32,
            signed: true,
        }
    }

    fn maximum(self) -> i128 {
        (1_i128 << (self.bits - u32::from(self.signed))) - 1
    }

    fn minimum(self) -> i128 {
        if self.signed {
            -(1_i128 << (self.bits - 1))
        } else {
            0
        }
    }

    /// C99: §6.6p4, p. 95; PDF p. 107.
    fn checked(self, value: i128) -> Option<Self> {
        if self.signed {
            (value >= self.minimum() && value <= self.maximum()).then_some(Self { value, ..self })
        } else {
            Some(self.cast_value(value))
        }
    }

    /// C99: §6.3.1.3p2-3, p. 43; PDF p. 55.
    fn cast_value(self, value: i128) -> Self {
        let mask = (1_i128 << self.bits) - 1;
        let mut value = value & mask;
        if self.signed && value > self.maximum() {
            value -= 1_i128 << self.bits;
        }
        Self { value, ..self }
    }

    /// C99: §6.3.1.3p1-3, p. 43; PDF p. 55.
    pub(crate) fn cast(self, bits: u32, signed: bool) -> Self {
        Self {
            bits,
            signed,
            ..self
        }
        .cast_value(self.value)
    }

    /// The next implicit enumerator value, which never wraps.
    /// C99: §6.7.2.2p3, p. 105; PDF p. 117.
    pub(crate) fn increment(self) -> Option<Self> {
        let value = self.value.checked_add(1)?;
        (value <= self.maximum()).then_some(Self { value, ..self })
    }

    /// C99: §6.3.1.1p2, p. 42; PDF p. 54.
    fn promote(self) -> Self {
        if self.bits < 32 {
            Self::int(self.value)
        } else {
            self
        }
    }

    /// C99: §6.5.3.3p1-5, pp. 79-80; PDF pp. 91-92.
    pub(crate) fn unary(self, op: UnaryOperator) -> Option<Self> {
        let value = self.promote();
        match op {
            | UnaryOperator::Plus | UnaryOperator::Extension => Some(value),
            | UnaryOperator::Minus => value.checked(-value.value),
            | UnaryOperator::BitwiseNot => Some(value.cast_value(!value.value)),
            | UnaryOperator::LogicalNot => Some(Self::int(i128::from(value.value == 0))),
            | _ => None,
        }
    }

    /// GCC defines a left shift of a nonnegative signed value into, but not
    /// past, the sign bit as its two's-complement result, which C99 leaves
    /// undefined (§6.5.7p4, p. 84; PDF p. 96). Every other failed shift
    /// stays exceptional.
    pub(crate) fn sign_bit_shift(self, op: BinaryOperator, right: Self) -> Option<Self> {
        let (l, r) = (self.promote(), right.promote());
        if op != BinaryOperator::LeftShift || !l.signed || l.value < 0 {
            return None;
        }
        let shift = u32::try_from(r.value).ok().filter(|&n| n < l.bits)?;
        let shifted = l.value.checked_shl(shift)?;
        (shifted < 1_i128 << l.bits).then(|| l.cast_value(shifted))
    }

    /// Usual integer arithmetic conversions, including LP64 rank distinctions.
    /// C99: §6.3.1.8p1, p. 45; PDF p. 57.
    pub(crate) fn binary(self, op: BinaryOperator, right: Self) -> Option<Self> {
        use BinaryOperator as B;
        let (mut l, mut r) = (self.promote(), right.promote());
        if matches!(op, B::LogicalAnd | B::LogicalOr) {
            return Some(Self::int(i128::from(if op == B::LogicalAnd {
                l.value != 0 && r.value != 0
            } else {
                l.value != 0 || r.value != 0
            })));
        }
        if matches!(op, B::LeftShift | B::RightShift) {
            let shift = u32::try_from(r.value).ok().filter(|&n| n < l.bits)?;
            if op == B::RightShift {
                return Some(Self {
                    value: l.value >> shift,
                    ..l
                });
            }
            if l.signed && l.value < 0 {
                return None;
            }
            return l.checked(l.value.checked_shl(shift)?);
        }
        let bits = l.bits.max(r.bits);
        let signed = if l.signed == r.signed {
            l.signed
        } else {
            let (signed, unsigned) = if l.signed { (l, r) } else { (r, l) };
            signed.bits > unsigned.bits
        };
        l = l.cast(bits, signed);
        r = r.cast(bits, signed);
        let value = match op {
            | B::Addition => l.value.checked_add(r.value)?,
            | B::Subtraction => l.value.checked_sub(r.value)?,
            | B::Multiplication if !l.signed => {
                let mask = (1_u128 << l.bits) - 1;
                i128::try_from(l.value.unsigned_abs().wrapping_mul(r.value.unsigned_abs()) & mask)
                    .ok()?
            },
            | B::Multiplication => {
                let magnitude = u128::from(u64::try_from(l.value.unsigned_abs()).ok()?)
                    .wrapping_mul(u128::from(u64::try_from(r.value.unsigned_abs()).ok()?));
                let value = i128::try_from(magnitude).ok()?;
                if (l.value < 0) == (r.value < 0) {
                    value
                } else {
                    -value
                }
            },
            | B::Division | B::Modulo => {
                if r.value == 0 || (l.signed && l.value == l.minimum() && r.value == -1) {
                    return None;
                }
                // C99 §6.5.5p6: like Rust's, the quotient truncates toward
                // zero and the remainder takes the dividend's sign.
                if op == B::Division {
                    l.value / r.value
                } else {
                    l.value % r.value
                }
            },
            | B::BitwiseAnd => l.value & r.value,
            | B::BitwiseOr => l.value | r.value,
            | B::BitwiseXor => l.value ^ r.value,
            | B::LessThan => return Some(Self::int(i128::from(l.value < r.value))),
            | B::LessThanOrEqual => return Some(Self::int(i128::from(l.value <= r.value))),
            | B::GreaterThan => return Some(Self::int(i128::from(l.value > r.value))),
            | B::GreaterThanOrEqual => return Some(Self::int(i128::from(l.value >= r.value))),
            | B::Equal => return Some(Self::int(i128::from(l.value == r.value))),
            | B::NotEqual => return Some(Self::int(i128::from(l.value != r.value))),
            | _ => return None,
        };
        l.checked(value)
    }
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// ICE operand restrictions apply even in an unselected conditional arm.
    /// Cache by syntax identity and immediate-floating-cast context so deep
    /// conditionals do not repeatedly validate their descendants.
    /// C99: §6.6p6, p. 95; PDF p. 107.
    fn valid_ice_operands(&mut self, expression: &'tu Expression<'tu>) -> bool {
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
                        .get(&target_type.source_vectors)
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

    /// Immediate floating operands are allowed in an integer-constant cast.
    /// C99: §6.6p6, p. 95; PDF p. 107; §6.3.1.4p1, p. 44; PDF p. 56.
    pub(super) fn float_cast(
        &self,
        ty: TypeId,
        value: super::super::preprocessing::FloatTokenType,
    ) -> Option<Integer> {
        use super::super::preprocessing::FloatTokenType as F;
        let (bits, signed) = self.integer_type(ty)?;
        let (negative, mantissa, shift) = match value {
            | F::Float(value) => float_parts(f64::from(value)),
            | F::Double(value) => float_parts(value.get()),
            | F::LongDouble(value) => {
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
            | _ => return None,
        };
        if matches!(
            self.types.nodes[ty.index],
            TypeKind::Scalar(super::Scalar::Bool)
        ) {
            return Some(Integer {
                value:  i128::from(mantissa != 0),
                bits:   1,
                signed: false,
            });
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
        let magnitude = i128::try_from(magnitude).ok()?;
        Integer {
            value: 0,
            bits,
            signed,
        }
        .checked(if negative { -magnitude } else { magnitude })
        .filter(|v| signed || v.value == if negative { -magnitude } else { magnitude })
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

    /// C99: §6.3.1.1p2, p. 42; PDF p. 54; §6.3.1.8, p. 45; PDF p. 57.
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
                        | I::Long(_) | I::LongLong(_) => Some((64, true)),
                        | I::UnsignedLong(_) | I::UnsignedLongLong(_) => Some((64, false)),
                        | _ => None,
                    }
                },
                | ExpressionType::Constant(Constant::Char(_)) | ExpressionType::Boolean(_) =>
                    Some((32, true)),
                | ExpressionType::Identifier(name) => self
                    .lookup(Namespace::Ordinary, name.name)
                    .and_then(|e| self.integer_type(self.bindings[e.binding].ty)),
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
                    if let Some(&ty) = self.resolved_type_names.get(&target_type.source_vectors) {
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
                        | S::Long
                        | S::LongInt
                        | S::SignedLong
                        | S::SignedLongInt
                        | S::LongLong
                        | S::LongLongInt
                        | S::SignedLongLong
                        | S::SignedLongLongInt => Some((64, true)),
                        | S::UnsignedLong
                        | S::UnsignedLongInt
                        | S::UnsignedLongLong
                        | S::UnsignedLongLongInt => Some((64, false)),
                        | S::TypedefName(name) => self
                            .lookup(Namespace::Ordinary, name.name)
                            .and_then(|e| self.integer_type(self.bindings[e.binding].ty)),
                        | _ => None,
                    }
                },
                | _ => None,
            };
            _ = self.integer_models.insert(key, model);
            values.push(model);
        }
        values.pop().flatten()
    }

    /// C99: §6.2.5p2-9, pp. 33-34; PDF pp. 45-46.
    pub(super) fn integer_type(&self, ty: TypeId) -> Option<(u32, bool)> {
        match self.types.nodes[ty.index] {
            | TypeKind::Scalar(super::Scalar::Bool) => Some((1, false)),
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
                    Constant::Integer(
                        super::super::preprocessing::IntegerTokenType::BitInt(..)
                        | super::super::preprocessing::IntegerTokenType::Imaginary(..),
                    )
                    | Constant::Float(
                        super::super::preprocessing::FloatTokenType::ImaginaryFloat(_)
                        | super::super::preprocessing::FloatTokenType::ImaginaryDouble(_)
                        | super::super::preprocessing::FloatTokenType::ImaginaryLongDouble(_),
                    ),
                )
                | ExpressionType::Generic(_)
                | ExpressionType::Countof(_)
                | ExpressionType::StatementExpression(_)
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
                        .get(&target_type.source_vectors)
                        .is_some_and(|&ty| self.types.unanalyzed(ty))
                    {
                        return true;
                    }
                    if matches!(
                        target_type.declaration_specifiers.type_specifiers,
                        super::TypeSpecifiers::Extended(_)
                    ) {
                        return true;
                    }
                    pending.push(operand_expression);
                },
                | ExpressionType::SizeofType(name) | ExpressionType::AlignofType(name) => {
                    if self
                        .resolved_type_names
                        .get(&name.source_vectors)
                        .is_some_and(|&ty| self.types.unanalyzed(ty))
                    {
                        return true;
                    }
                    if matches!(
                        name.declaration_specifiers.type_specifiers,
                        super::TypeSpecifiers::Extended(_)
                    ) {
                        return true;
                    }
                },
                | _ => {},
            }
        }
        false
    }

    /// Evaluation schedules continuations instead of recursing into
    /// operands/types. C99: §6.6p3-6, p. 95; PDF p. 107.
    pub(super) fn evaluate(&mut self, expression: &'tu Expression<'tu>) {
        use ExpressionType as E;
        let info = self.expression_info(expression);
        if self.types.unanalyzed(info.ty) {
            self.integers.push(None);
            return;
        }
        if self.context.configuration.gnu_extensions()
            && info.constant == super::ConstantClass::Arithmetic
            && info.integer.is_some()
        {
            // GCC folds arithmetic constants that are not integer constant
            // expressions (§6.6p6) wherever one is required.
            if !info.ice && !self.tainted {
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
                    | I::Long(_) | I::LongLong(_) => (64, true),
                    | I::UnsignedLong(_) | I::UnsignedLongLong(_) => (64, false),
                    | I::BitInt(..) | I::Imaginary(..) => {
                        self.integers.push(None);
                        return;
                    },
                };
                self.integers.push(Some(Integer {
                    value: value.into(),
                    bits,
                    signed,
                }));
            },
            | E::Constant(Constant::Char(value)) => {
                let mut value = i128::from(i64::from(value));
                if matches!(
                    expression.kind,
                    ExpressionType::Constant(Constant::Char(
                        super::super::preprocessing::CharacterTokenType::Char(_)
                    ))
                ) && self.types.target.char_signed
                    && (128..=255).contains(&value)
                {
                    value -= 256;
                }
                self.integers.push(Some(Integer::int(value)));
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
                    .push(Work::Unary(operator, expression.source_vectors));
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
                        .push(Work::Binary(operator, expression.source_vectors));
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
                self.work.push(Work::TypeName(target_type));
            },
            | E::SizeofType(name) | E::AlignofType(name) => {
                self.work.push(Work::SizeofDone(
                    matches!(expression.kind, E::AlignofType(_)),
                    expression.source_vectors,
                ));
                self.work.push(Work::TypeName(name));
            },
            | E::Builtin(_) | E::SizeofExpr(_) | E::AlignofExpr(_) => {
                let info = self.expression_info(expression);
                self.integers
                    .push(if info.ice { info.integer } else { None });
            },
            | _ => self.integers.push(None),
        }
    }
}

fn common(mut left: (u32, bool), mut right: (u32, bool)) -> (u32, bool) {
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
