//! The width `cl` takes an integer operand's value to have, which decides
//! C4244 rather than the width of its type: `c + 1` of a `char` stays as
//! wide as a `char`, `i & 0xff` and `i >> 24` of an `int` as wide as a byte,
//! and a constant as wide as its magnitude. Measured against `cl` 19.44; a
//! form not measured has no width, and draws no warning.
//! C99: §6.3.1.1 paragraph 2, p. 42; PDF p. 54 (the promotions that widen
//! the operands); §6.5.7, p. 84; PDF p. 96; §6.5.10, p. 87; PDF p. 99.

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
            return Some(magnitude_bits(value.value.unsigned_abs()));
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
                let right = match divisor.integer.filter(|value| divisor.ice && value.wide.is_none())
                {
                    | Some(value) => magnitude_bits(value.value.unsigned_abs().saturating_sub(1)),
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
                    match count.integer.filter(|value| count.ice && value.wide.is_none()) {
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
            | E::Conditional(conditional) => Some(
                self.cl_natural_bits(conditional.then_expression)?
                    .max(self.cl_natural_bits(conditional.else_expression)?),
            ),
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

/// The bits a magnitude needs, without a sign: 8 for 255, 9 for 256.
fn magnitude_bits(magnitude: u128) -> u32 {
    u128::BITS - magnitude.leading_zeros()
}
