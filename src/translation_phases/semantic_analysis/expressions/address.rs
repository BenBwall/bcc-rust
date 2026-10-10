//! Phase-7 address-constant bases, byte offsets, pointer values, and
//! differences. C99: §6.6p9, p. 96; PDF p. 108.

use super::{
    super::{
        Analyzer,
        BinaryOperator,
        Expression,
        ExpressionType,
        Integer,
        TypeId,
        TypeKind,
        UnaryOperator,
    },
    ConstantClass,
    ExpressionInfo,
};
use crate::translation_phases::preprocessing::StringTokenType;

/// The object an address constant is based on. C99: §6.6p9, p. 96; PDF
/// p. 108.
#[derive(Debug, Clone, Copy)]
pub(crate) enum AddressBase<'tu> {
    /// An integer-valued pointer constant such as a null pointer.
    Absolute,
    Binding(usize),
    String(&'tu Expression<'tu>),
}

impl<'tu> Analyzer<'_, 'tu, '_> {
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

    /// Splits an address into its base object and byte offset, following
    /// member, subscript, cast and integer-offset steps without recursion.
    /// Implementation choice: GCC-style address differences are accepted as
    /// additional constant expressions.
    /// C99: §6.6 paragraphs 9-10, p. 96; PDF p. 108.
    pub(in crate::translation_phases::semantic_analysis) fn address_parts(
        &self,
        mut e: &'tu Expression<'tu>,
        mut lvalue: bool,
    ) -> Option<(AddressBase<'tu>, i128)> {
        let mut offset = 0_i128;
        loop {
            if let ExpressionType::Parenthesized { expression } = e.kind {
                e = expression;
                continue;
            }
            let info = self.expression_info(e);
            if let Some(selected) = info.selected_expression {
                e = selected;
                continue;
            }
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
                        let size = i128::from(self.types.layout(target)?.size);
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
                    let size = i128::from(self.types.layout(info.ty)?.size);
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
                | _ => false,
            },
            | _ => false,
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
        let size = self.types.layout(target)?.size;
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

    pub(super) fn pointer_value(info: ExpressionInfo<'tu>) -> Option<i128> {
        (info.constant == ConstantClass::Address)
            .then_some(info.integer)
            .flatten()
            .map(|v| v.value)
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
}
