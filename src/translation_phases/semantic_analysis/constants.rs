//! Phase-7 arithmetic constant values, including complex components.
//! C99: §6.6p4-8, pp. 95-96; PDF pp. 107-108; arithmetic conversions
//! §6.3.1.8, pp. 44-45; PDF pp. 56-57. Integer arithmetic remains in
//! integer.rs.

use super::{
    Analyzer,
    expressions::ExpressionInfo,
    integer::Integer,
    types::{
        Scalar,
        TypeId,
        TypeKind,
    },
};
use crate::{
    float_parsing::{
        LongDouble,
        string_to_long_double,
    },
    translation_phases::parsing::syntax::BinaryOperator,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// IEC 60559 arithmetic: a zero divisor gives an infinity or NaN
    /// (Annex F.3). C99: §6.5.5-§6.5.6, pp. 82-84; PDF pp. 94-96.
    pub(super) fn floating_binary(
        &self,
        op: BinaryOperator,
        left: Floating,
        right: Floating,
    ) -> Option<Floating> {
        use BinaryOperator as B;
        let precision = if self.types.target.long_double_is_double() {
            2
        } else {
            3
        };
        let add = |a: LongDouble, b| a.arithmetic(b, 0, precision);
        let sub = |a: LongDouble, b| a.arithmetic(b, 1, precision);
        let mul = |a: LongDouble, b| a.arithmetic(b, 2, precision);
        let div = |a: LongDouble, b| a.arithmetic(b, 3, precision);
        Some(match op {
            | B::Addition => Floating {
                real: add(left.real, right.real),
                imag: add(left.imag, right.imag),
            },
            | B::Subtraction => Floating {
                real: sub(left.real, right.real),
                imag: sub(left.imag, right.imag),
            },
            | B::Multiplication => Floating {
                real: sub(mul(left.real, right.real), mul(left.imag, right.imag)),
                imag: add(mul(left.real, right.imag), mul(left.imag, right.real)),
            },
            | B::Division if left.imag.is_zero() && right.imag.is_zero() =>
                Floating::real(div(left.real, right.real)),
            | B::Division => {
                let denominator = add(mul(right.real, right.real), mul(right.imag, right.imag));
                Floating {
                    real: div(
                        add(mul(left.real, right.real), mul(left.imag, right.imag)),
                        denominator,
                    ),
                    imag: div(
                        sub(mul(left.imag, right.real), mul(left.real, right.imag)),
                        denominator,
                    ),
                }
            },
            | _ => return None,
        })
    }

    /// Folds floating equality and ordering, including unordered NaNs.
    /// C99: §F.3 paragraph 1, pp. 445-447; PDF pp. 457-459.
    /// C99: §6.5.8 paragraph 6, p. 86; PDF p. 98.
    /// C99: §6.5.9 paragraph 3, p. 86; PDF p. 98.
    pub(super) fn floating_comparison(
        op: BinaryOperator,
        left: Floating,
        right: Floating,
    ) -> Option<Integer> {
        use BinaryOperator as B;
        // A NaN operand is unordered: only inequality holds (Annex F.3).
        let comparison = left.real.compare(right.real);
        let imaginary = left.imag.compare(right.imag);
        let unordered = comparison == 2 || imaginary == 2;
        let equal = comparison == 0 && imaginary == 0;
        let result = match op {
            | B::Equal => equal,
            | B::NotEqual => !equal,
            | _ if unordered => false,
            | B::LessThan => comparison < 0,
            | B::LessThanOrEqual => comparison <= 0,
            | B::GreaterThan => comparison > 0,
            | B::GreaterThanOrEqual => comparison >= 0,
            | _ => return None,
        };
        Some(Integer::int(i128::from(result)))
    }

    /// C99: §6.3.1.3-§6.3.1.8, pp. 43-45; PDF pp. 55-57.
    pub(super) fn floating_value(&self, info: ExpressionInfo<'tu>) -> Option<Floating> {
        if let Some(value) = info.floating {
            return Some(value);
        }
        let value = info.integer?;
        let text = crate::diagnostics::format_in!(self.scratch, "{}L\0", value);
        Some(Floating::real(string_to_long_double(text).ok()?))
    }

    /// Tests scalar constant truth for logical and conditional evaluation.
    /// C99: §6.5.3.3 paragraph 5, p. 79; PDF p. 91.
    /// C99: §6.5.15 paragraph 4, p. 90; PDF p. 102.
    pub(super) fn constant_truth(info: ExpressionInfo<'tu>) -> Option<bool> {
        info.integer
            .map(|v| v.value != 0)
            .or_else(|| info.floating.map(Floating::truth))
    }

    /// C99: §6.3.1.5-§6.3.1.8, pp. 44-45; PDF pp. 56-57.
    pub(super) fn round_floating(&self, value: Floating, ty: TypeId) -> Option<Floating> {
        let ty = self.types.non_atomic(ty);
        let TypeKind::Scalar(scalar) = self.types.nodes[ty.index] else {
            return None;
        };
        let precision = match scalar {
            | Scalar::Float | Scalar::ComplexFloat => 1,
            | Scalar::Double | Scalar::ComplexDouble => 2,
            | Scalar::LongDouble | Scalar::ComplexLongDouble =>
                if self.types.target.long_double_is_double() {
                    2
                } else {
                    3
                },
            | _ => return None,
        };
        let complex = matches!(
            scalar,
            Scalar::ComplexFloat | Scalar::ComplexDouble | Scalar::ComplexLongDouble
        );
        Some(Floating {
            real: value.real.arithmetic(LongDouble::ZERO, 4, precision),
            imag: if complex {
                value.imag.arithmetic(LongDouble::ZERO, 4, precision)
            } else {
                LongDouble::ZERO
            },
        })
    }

    /// Negation keeps the sign of zero (Annex F.3).
    /// C99: §F.3 paragraph 1, pp. 445-447; PDF pp. 457-459.
    /// C99: §6.5.3.3 paragraph 3, p. 79; PDF p. 91.
    pub(super) fn floating_negate(value: Floating) -> Floating {
        Floating {
            real: value.real.negate(),
            imag: value.imag.negate(),
        }
    }
}

/// Target real/imaginary constant components; each operation rounds to the
/// result's real component precision. C99: §6.2.5p13, p. 34; PDF p. 46.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Floating {
    pub(crate) real: LongDouble,
    pub(crate) imag: LongDouble,
}

impl Floating {
    /// Tests whether a complex value differs from zero.
    /// C99: §6.3.1.2 paragraph 1, p. 43; PDF p. 55.
    pub(super) fn truth(self) -> bool {
        !self.real.is_zero() || !self.imag.is_zero()
    }

    pub(super) fn real(value: LongDouble) -> Self {
        Self {
            real: value,
            imag: LongDouble::ZERO,
        }
    }
}
