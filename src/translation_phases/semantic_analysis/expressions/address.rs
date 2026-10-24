//! Phase-7 address-constant bases, byte offsets, pointer values, and
//! differences. C99: §6.6p9, p. 96; PDF p. 108.

use super::{
    super::{
        Analyzer,
        LabelId,
        integer::Integer,
        types::{
            TypeId,
            TypeKind,
        },
    },
    ConstantClass,
    ExpressionInfo,
};
use crate::translation_phases::{
    parsing::syntax::{
        BinaryOperator,
        Expression,
        ExpressionType,
        UnaryOperator,
    },
    preprocessing::StringTokenType,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Splits an address into its base object and byte offset, following
    /// member, subscript, cast and integer-offset steps without recursion.
    /// Implementation choice: GCC-style address differences are accepted as
    /// additional constant expressions.
    /// C99: §6.6 paragraphs 9-10, p. 96; PDF p. 108.
    pub(in crate::translation_phases::semantic_analysis) fn address_parts(
        &self,
        e: &'tu Expression<'tu>,
        lvalue: bool,
    ) -> Option<(AddressBase<'tu>, i128)> {
        self.address_parts_with_casts(e, lvalue, false)
    }

    /// A string start remains a constant query value only through pointers
    /// and integers with exactly the pointer width at every cast.
    /// GNU extension: `__builtin_constant_p`.
    /// C99: §6.3.2.3 paragraph 6, p. 47; PDF p. 59.
    pub(in crate::translation_phases::semantic_analysis) fn constant_query_string_address(
        &self,
        expression: &'tu Expression<'tu>,
    ) -> bool {
        matches!(
            self.address_parts_with_casts(expression, false, true),
            Some((AddressBase::String(_), 0))
        )
    }

    /// Shared address walk; queries additionally require preserving casts.
    /// C99: §6.6 paragraphs 9-10, p. 96; PDF p. 108.
    fn address_parts_with_casts(
        &self,
        mut e: &'tu Expression<'tu>,
        mut lvalue: bool,
        preserving_casts: bool,
    ) -> Option<(AddressBase<'tu>, i128)> {
        let mut offset = 0_i128;
        loop {
            e = self.selected_designator(e);
            let info = self.expression_info(e);
            if !lvalue {
                if matches!(
                    self.types.nodes[info.ty.index],
                    TypeKind::Array(..) | TypeKind::Function { .. }
                ) {
                    // The decayed value addresses the designated object.
                    lvalue = true;
                    continue;
                }
                match e.kind {
                    | ExpressionType::Cast {
                        operand_expression, ..
                    } => {
                        if preserving_casts
                            && self.pointer_target(info.ty).is_none()
                            && !self.integer_type(info.ty).is_some_and(|(bits, _)| {
                                u64::from(bits) == self.types.target.pointer.size * 8
                            })
                        {
                            return None;
                        }
                        let operand = self.expression_info(operand_expression);
                        if operand.ice
                            && let Some(value) = operand.integer
                        {
                            return Some((
                                AddressBase::Absolute,
                                offset.checked_add(if self.pointer_target(info.ty).is_some() {
                                    info.integer?.to_i128()?
                                } else {
                                    value.to_i128()?
                                })?,
                            ));
                        }
                        e = operand_expression;
                    },
                    | ExpressionType::Unary {
                        operator: UnaryOperator::AddressOf,
                        operand_expression,
                    } => {
                        lvalue = true;
                        e = operand_expression;
                    },
                    | ExpressionType::Binary {
                        operator:
                            operator @ (BinaryOperator::Addition | BinaryOperator::Subtraction),
                        left_expression,
                        right_expression,
                    } => {
                        let left = self.expression_info(left_expression);
                        let right = self.expression_info(right_expression);
                        let (pointer, index) = if right.ice {
                            (left, right)
                        } else {
                            (right, left)
                        };
                        let index = index.integer.filter(|_| index.ice)?.to_i128()?;
                        let target = self.pointer_target(info.ty)?;
                        let size = i128::from(self.arithmetic_size(target)?);
                        let step = index.checked_mul(size)?;
                        offset = if operator == BinaryOperator::Addition {
                            offset.checked_add(step)?
                        } else if std::ptr::eq(pointer.expression, left_expression) {
                            offset.checked_sub(step)?
                        } else {
                            return None;
                        };
                        e = pointer.expression;
                    },
                    | _ => {
                        let value = Self::pointer_value(info)?;
                        return Some((AddressBase::Absolute, offset.checked_add(value)?));
                    },
                }
                continue;
            }
            match e.kind {
                | ExpressionType::Identifier(_) =>
                    return Some((AddressBase::Binding(info.binding?), offset)),
                | ExpressionType::StringLiteral(_) =>
                    return Some((AddressBase::String(e), offset)),
                | ExpressionType::DirectMember {
                    base_expression,
                    member,
                }
                | ExpressionType::IndirectMember {
                    base_expression,
                    member,
                } => {
                    let base = self.expression_info(base_expression);
                    let indirect = matches!(e.kind, ExpressionType::IndirectMember { .. });
                    let record = if indirect {
                        self.pointer_target(base.ty)?
                    } else {
                        base.ty
                    };
                    let TypeKind::Tag(id) = self.types.nodes[record.index] else {
                        return None;
                    };
                    let index = *self.member_indices.get(&(id, member.name))?;
                    let field = self.types.tags[id].fields.get().get(index)?;
                    offset = offset.checked_add(i128::from(field.offset))?;
                    lvalue = !indirect;
                    e = base_expression;
                },
                | ExpressionType::Binary {
                    operator: BinaryOperator::Subscript,
                    left_expression,
                    right_expression,
                } => {
                    let left = self.expression_info(left_expression);
                    let right = self.expression_info(right_expression);
                    let (pointer, index) = if right.ice {
                        (left, right)
                    } else {
                        (right, left)
                    };
                    let index = index.integer.filter(|_| index.ice)?.to_i128()?;
                    let size = i128::from(self.arithmetic_size(info.ty)?);
                    offset = offset.checked_add(index.checked_mul(size)?)?;
                    lvalue = false;
                    e = pointer.expression;
                },
                | ExpressionType::Unary {
                    operator: UnaryOperator::Indirection,
                    operand_expression,
                } => {
                    lvalue = false;
                    e = operand_expression;
                },
                | _ => return None,
            }
        }
    }

    /// The element distance between two addresses of one object.
    /// C99: §6.5.6p9, pp. 83-84; PDF pp. 95-96.
    pub(super) fn address_difference(
        &self,
        left: ExpressionInfo<'tu>,
        right: ExpressionInfo<'tu>,
        target: TypeId,
    ) -> Option<Integer> {
        let (left_base, left_offset) = self.address_parts(left.expression, false)?;
        let (right_base, right_offset) = self.address_parts(right.expression, false)?;
        let size = self.arithmetic_size(target)?;
        let bytes = left_offset.checked_sub(right_offset)?;
        // Divide 64-bit magnitudes; i128 division is unavailable (see
        // integer.rs).
        let magnitude = u64::try_from(bytes.unsigned_abs()).ok()?;
        if !self.same_address_base(left_base, right_base) || size == 0 || magnitude % size != 0 {
            return None;
        }
        let elements = i128::from(magnitude / size);
        Some(Integer::int(if bytes < 0 { -elements } else { elements }).cast(64, true))
    }

    /// The labels of `&&label - &&base` when both name labels of one
    /// function: a constant whose value only code layout determines.
    /// GNU extension: GCC "Labels as Values".
    pub(super) fn label_difference(
        &self,
        left: ExpressionInfo<'tu>,
        right: ExpressionInfo<'tu>,
    ) -> Option<(LabelId, LabelId)> {
        let label = |mut e: &'tu Expression<'tu>| {
            while let ExpressionType::Parenthesized { expression } = e.kind {
                e = expression;
            }
            self.control.address_label(e)
        };
        let (label, base) = (label(left.expression)?, label(right.expression)?);
        (self.control.labels[label.0].function == self.control.labels[base.0].function)
            .then_some((label, base))
    }

    /// Determines whether an expression forms an address constant without
    /// reading an object.
    /// C99: §6.6 paragraph 9, p. 96; PDF p. 108.
    pub(in super::super) fn address_value(&self, info: ExpressionInfo<'tu>) -> bool {
        info.constant == ConstantClass::Address
            || (info.static_address
                && matches!(
                    self.types.nodes[info.ty.index],
                    TypeKind::Array(..) | TypeKind::Function { .. }
                ))
    }

    pub(super) fn pointer_value(info: ExpressionInfo<'tu>) -> Option<i128> {
        (info.constant == ConstantClass::Address)
            .then_some(info.integer)
            .flatten()
            .map(|v| v.value)
    }

    /// Fold arithmetic on an absolute pointer without inventing a symbolic
    /// base. A null value plus zero remains zero data, with no relocation.
    /// Implementation latitude: absolute pointer arithmetic follows the
    /// target's element size and 64-bit pointer representation.
    /// C99: §6.6p9-10, p. 96; PDF p. 108 (address constants and extra forms).
    pub(super) fn absolute_pointer_offset(
        &self,
        pointer: ExpressionInfo<'tu>,
        index: ExpressionInfo<'tu>,
        target: TypeId,
        subtract: bool,
    ) -> Option<Integer> {
        let base = Self::pointer_value(pointer)?;
        let offset = index
            .integer?
            .to_i128()?
            .checked_mul(i128::from(self.arithmetic_size(target)?))?;
        let value = if subtract {
            base.checked_sub(offset)?
        } else {
            base.checked_add(offset)?
        };
        Some(Integer::int(value).cast(64, false))
    }

    /// Implementation-defined pointer-to-integer folding for the address of
    /// an lvalue reached from an integer-valued pointer constant, as in the
    /// classic `offsetof` macro.
    /// C99: §6.3.2.3 paragraph 6, p. 47; PDF p. 59.
    pub(super) fn integer_address(&self, e: &'tu Expression<'tu>) -> Option<i128> {
        match self.address_parts(e, true)? {
            | (AddressBase::Absolute, offset) => Some(offset),
            | _ => None,
        }
    }

    /// Whether two address bases designate the same object; the implementation
    /// chooses to merge identical string literals, as GCC does.
    /// C99: §6.4.5 paragraph 6, p. 63; PDF p. 75.
    fn same_address_base(&self, left: AddressBase<'tu>, right: AddressBase<'tu>) -> bool {
        match (left, right) {
            | (AddressBase::Absolute, AddressBase::Absolute) => true,
            | (AddressBase::Binding(l), AddressBase::Binding(r)) => l == r,
            | (AddressBase::String(l), AddressBase::String(r)) => match (l.kind, r.kind) {
                | (
                    ExpressionType::StringLiteral(StringTokenType::String(l)),
                    ExpressionType::StringLiteral(StringTokenType::String(r)),
                )
                | (
                    ExpressionType::StringLiteral(StringTokenType::WideString(l)),
                    ExpressionType::StringLiteral(StringTokenType::WideString(r)),
                ) => self.context.literal_units(l) == self.context.literal_units(r),
                | (
                    ExpressionType::StringLiteral(StringTokenType::EncodedString(l, left)),
                    ExpressionType::StringLiteral(StringTokenType::EncodedString(r, right)),
                ) =>
                    left == right && self.context.literal_units(l) == self.context.literal_units(r),
                | _ => false,
            },
            | _ => false,
        }
    }
}

/// The object an address constant is based on. C99: §6.6p9, p. 96; PDF
/// p. 108.
#[derive(Debug, Clone, Copy)]
pub(crate) enum AddressBase<'tu> {
    /// An integer-valued pointer constant such as a null pointer.
    Absolute,
    Binding(usize),
    String(&'tu Expression<'tu>),
}
