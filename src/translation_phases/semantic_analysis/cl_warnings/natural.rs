//! The width `cl` takes an integer operand's value to have, which decides
//! C4244 rather than the width of its type: `c + 1` of a `char` stays as
//! wide as a `char`, `i & 0xff` and `i >> 24` of an `int` as wide as a byte,
//! a constant as wide as its magnitude, or its two's complement if negative
//! (as `cl` reads `~0ull`, -1), and a `?:` whose condition is a constant as
//! wide as the operand it selects. Measured against `cl` 19.44; a
//! form not measured has no width, and draws no warning.
//! C99: §6.3.1.1 paragraph 2, p. 42; PDF p. 54 (the promotions that widen
//! the operands); §6.5.7, p. 84; PDF p. 96; §6.5.10, p. 87; PDF p. 99;
//! §6.5.15 paragraph 4, p. 90; PDF p. 102.

use crate::translation_phases::{
    parsing::syntax::{
        BinaryOperator as B,
        Expression,
        ExpressionType as E,
        UnaryOperator as U,
    },
    semantic_analysis::{
        Analyzer,
        types::{
            Scalar,
            TypeKind,
        },
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// The bits `cl` takes the integer operand `e` to need.
    pub(in crate::translation_phases::semantic_analysis) fn cl_natural_bits(
        &self,
        e: &'tu Expression<'tu>,
    ) -> Option<u32> {
        let info = self.expression_info(e);
        if info.ice
            && let Some(value) = info.integer.filter(|value| value.wide.is_none())
        {
            return Some(constant_bits(cl_signed_reading(value.value)));
        }
        match e.kind {
            | E::Parenthesized { expression } => self.cl_natural_bits(expression),
            | E::Identifier(_)
            | E::DirectMember { .. }
            | E::IndirectMember { .. }
            | E::Call { .. }
            | E::Unary {
                operator: U::Indirection,
                ..
            }
            | E::Binary {
                operator: B::Subscript,
                ..
            } => self.cl_type_bits(e),
            // A cast keeps no more of its operand than its type holds.
            | E::Cast {
                operand_expression, ..
            } => Some(
                self.cl_natural_bits(operand_expression)?
                    .min(self.cl_type_bits(e)?),
            ),
            | E::Unary {
                operator:
                    U::Plus
                    | U::Minus
                    | U::BitwiseNot
                    | U::PreIncrement
                    | U::PreDecrement
                    | U::PostIncrement
                    | U::PostDecrement,
                operand_expression,
            } => self.cl_natural_bits(operand_expression),
            | E::Unary {
                operator: U::LogicalNot,
                ..
            }
            | E::Binary {
                operator:
                    B::LessThan
                    | B::GreaterThan
                    | B::LessThanOrEqual
                    | B::GreaterThanOrEqual
                    | B::Equal
                    | B::NotEqual
                    | B::LogicalAnd
                    | B::LogicalOr,
                ..
            } => Some(1),
            // A difference of pointers is as wide as its type.
            | E::Binary {
                operator: B::Subtraction,
                left_expression,
                ..
            } if matches!(
                self.types.nodes[self
                    .types
                    .non_atomic(self.expression_info(left_expression).ty)
                    .index],
                TypeKind::Pointer(_) | TypeKind::Array(..)
            ) =>
                self.cl_type_bits(e),
            | E::Binary {
                operator:
                    B::Addition
                    | B::Subtraction
                    | B::Multiplication
                    | B::Division
                    | B::BitwiseOr
                    | B::BitwiseXor,
                left_expression,
                right_expression,
            } => Some(
                self.cl_natural_bits(left_expression)?
                    .max(self.cl_natural_bits(right_expression)?),
            ),
            | E::Binary {
                operator: B::BitwiseAnd,
                left_expression,
                right_expression,
            } => Some(
                self.cl_natural_bits(left_expression)?
                    .min(self.cl_natural_bits(right_expression)?),
            ),
            // A remainder is smaller than a constant divisor.
            | E::Binary {
                operator: B::Modulo,
                left_expression,
                right_expression,
            } => {
                let divisor = self.expression_info(right_expression);
                let right = match divisor
                    .integer
                    .filter(|value| divisor.ice && value.wide.is_none())
                {
                    | Some(value) => remainder_bits(cl_signed_reading(value.value)),
                    | None => self.cl_natural_bits(right_expression)?,
                };
                Some(self.cl_natural_bits(left_expression)?.min(right))
            },
            | E::Binary {
                operator: B::LeftShift,
                left_expression,
                ..
            } => self.cl_natural_bits(left_expression),
            // A constant count shifts out as many bits.
            | E::Binary {
                operator: B::RightShift,
                left_expression,
                right_expression,
            } => {
                let left = self.cl_natural_bits(left_expression)?;
                let count = self.expression_info(right_expression);
                Some(
                    match count
                        .integer
                        .filter(|value| count.ice && value.wide.is_none())
                    {
                        | Some(value) => left.saturating_sub(u32::try_from(value.value).ok()?),
                        | None => left,
                    },
                )
            },
            | E::Binary {
                operator: B::Comma,
                right_expression,
                ..
            } => self.cl_natural_bits(right_expression),
            | E::Binary {
                operator:
                    B::Assignment
                    | B::MultiplicationAssignment
                    | B::DivisionAssignment
                    | B::ModuloAssignment
                    | B::AdditionAssignment
                    | B::SubtractionAssignment
                    | B::LeftShiftAssignment
                    | B::RightShiftAssignment
                    | B::BitwiseAndAssignment
                    | B::BitwiseXorAssignment
                    | B::BitwiseOrAssignment,
                left_expression,
                ..
            } => self.cl_type_bits(left_expression),
            // `cl` measures only the operand a constant condition selects.
            | E::Conditional(conditional) => match self.cl_condition(conditional) {
                | super::ClCondition::Folded(selected) => self.cl_natural_bits(selected),
                | super::ClCondition::Unknown => None,
                | super::ClCondition::Runtime => Some(
                    self.cl_natural_bits(conditional.then_expression)?
                        .max(self.cl_natural_bits(conditional.else_expression)?),
                ),
            },
            | _ => None,
        }
    }

    /// The width of the integer type of `e`: `int`'s for an enumeration,
    /// and its declared type's for a bit-field, whatever its width.
    pub(in crate::translation_phases::semantic_analysis) fn cl_type_bits(
        &self,
        e: &'tu Expression<'tu>,
    ) -> Option<u32> {
        let ty = self.types.non_atomic(self.expression_info(e).ty);
        match self.types.nodes[ty.index] {
            | TypeKind::Scalar(Scalar::Bool) => Some(1),
            | TypeKind::Tag(_) => self.integer_type(ty).map(|_| 32),
            | _ => self.integer_type(ty).map(|(bits, _)| bits),
        }
    }
}

/// The value `cl` reads for the integer constant `value`: a 64-bit unsigned
/// constant of 2^63 or more as the signed one with its bits, so
/// `0xFFFFFFFFFFFFFF80ull` is -128 and `~0ull` is -1; a narrower unsigned
/// constant as itself.
pub(super) const fn cl_signed_reading(value: i128) -> i128 {
    if value >= 1 << 63 && value < 1 << 64 {
        value - (1 << 64)
    } else {
        value
    }
}

/// The bits `cl` takes the constant `value` to need: a non-negative one
/// its magnitude's, without a sign (8 for 255, 9 for 256), and a negative
/// one its two's complement's, with the sign (8 for -128, 9 for -129).
const fn constant_bits(value: i128) -> u32 {
    if value < 0 {
        (!value).unsigned_abs().bit_width() + 1
    } else {
        value.unsigned_abs().bit_width()
    }
}

/// The bits `cl` takes a remainder by the constant `divisor` to need: the
/// magnitude's of a smaller value for a positive divisor (8 for 256), and
/// for a negative one its magnitude's and a sign (8 for -127, 9 for -128).
const fn remainder_bits(divisor: i128) -> u32 {
    if divisor < 0 {
        divisor.unsigned_abs().bit_width() + 1
    } else {
        divisor.unsigned_abs().saturating_sub(1).bit_width()
    }
}
