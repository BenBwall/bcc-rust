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
        op: BinaryOperator,
        left: Floating,
        right: Floating,
        precision: i32,
        domains: u8,
    ) -> Option<Floating> {
        use BinaryOperator as B;
        let operation = match op {
            | B::Addition => 0,
            | B::Subtraction => 1,
            | B::Multiplication => 2,
            | B::Division => 3,
            | _ => return None,
        };
        let arithmetic = |a: LongDouble, b, operation| a.arithmetic(b, operation, precision);
        if domains == 1 || domains == 2 {
            let imaginary = if domains == 1 { left.imag } else { right.imag };
            let real_operand = if domains == 1 { right.real } else { left.real };
            match op {
                | B::Addition =>
                    return Some(Floating {
                        real: arithmetic(left.real, right.real, 0),
                        imag: imaginary,
                    }),
                | B::Subtraction =>
                    return Some(Floating {
                        real: arithmetic(left.real, right.real, 1),
                        imag: if domains == 1 {
                            imaginary
                        } else {
                            imaginary.negate()
                        },
                    }),
                | B::Multiplication =>
                    return Some(Floating {
                        real: arithmetic(left.real, right.real, 2),
                        imag: arithmetic(imaginary, real_operand, 2),
                    }),
                | B::Division if domains == 1 =>
                    return Some(Floating {
                        real: arithmetic(left.real, right.real, 3),
                        imag: arithmetic(imaginary, right.real, 3),
                    }),
                | _ => {},
            }
        }
        if domains != 0 {
            let [real, imag] = LongDouble::complex_arithmetic(
                [left.real, left.imag],
                [right.real, right.imag],
                operation,
                precision,
            );
            Some(Floating { real, imag })
        } else {
            Some(Floating::real(
                left.real.arithmetic(right.real, operation, precision),
            ))
        }
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
        let precision = self.floating_precision(scalar)?;
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

    /// The carrier's rounding precision for a real or complex floating
    /// scalar: 1 for binary32, 2 for binary64 and 3 for x87 extended.
    /// Binary128 has no carrier precision.
    /// C99: implementation-defined floating representations §6.2.5p10,
    /// p. 34; PDF p. 46; characteristics §5.2.4.2.2, pp. 23-27; PDF pp. 35-39.
    pub(super) fn floating_precision(&self, scalar: Scalar) -> Option<i32> {
        Some(match scalar {
            | Scalar::Float | Scalar::ComplexFloat => 1,
            | Scalar::Double | Scalar::ComplexDouble => 2,
            | Scalar::LongDouble | Scalar::ComplexLongDouble =>
                if self.types.target.long_double_is_double() {
                    2
                } else {
                    3
                },
            | _ => return None,
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
