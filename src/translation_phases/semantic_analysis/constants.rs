//! Phase-7 arithmetic constant values, including complex components.
//! Standard floating constants compute in the host `long double` carrier,
//! rounded to their type's precision; binary128 constants compute with the
//! software IEEE binary128 arithmetic of `crate::binary128`, so a folded
//! value is the one libgcc's routines give at run time.
//! C99: §6.6p4-8, pp. 95-96; PDF pp. 107-108; arithmetic conversions
//! §6.3.1.8, pp. 44-45; PDF pp. 56-57. Integer arithmetic remains in
//! integer.rs.

use std::cmp::Ordering;

use super::{
    Analyzer,
    expressions::{
        ConstantClass,
        ExpressionInfo,
    },
    integer::Integer,
    types::{
        Scalar,
        TypeId,
        TypeKind,
    },
};
use crate::{
    binary128::Binary128,
    float_parsing::LongDouble,
    half_precision::HalfFormat,
    translation_phases::{
        parsing::syntax::BinaryOperator,
        preprocessing::HalfConstant,
    },
    x87::X87,
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

    /// C99: §6.3.1.3-§6.3.1.8, pp. 43-45; PDF pp. 55-57. A `_Float16` or
    /// `__bf16` value is the one its consumer reads: see
    /// [`Self::half_operand`].
    pub(super) fn floating_value(&self, info: ExpressionInfo<'tu>) -> Option<Floating> {
        if let Some(value) = info.floating() {
            if let Some(format) = self.half_format(info.ty)
                && !self.half_in_float()
            {
                return Some(round_halves(value, format));
            }
            return Some(value);
        }
        // The x87 carrier's 64 bits, rounded once; a conversion to a
        // narrower type reads the integer itself (`converted_floating`).
        integer_value(info.integer?, crate::ir::Type::F80).map(Floating::real)
    }

    /// `info`'s value converted to the arithmetic type `ty`. An integer
    /// rounds once from its exact value: [`Self::floating_value`] holds it
    /// at the x87 format's 64 bits, and rounding that again to a narrower
    /// format would round a 128-bit integer twice.
    /// C99: §6.3.1.4 paragraph 2, pp. 43-44; PDF pp. 55-56; §6.3.1.5,
    /// p. 44; PDF p. 56.
    pub(super) fn converted_floating(
        &self,
        info: ExpressionInfo<'tu>,
        ty: TypeId,
    ) -> Option<Floating> {
        if info.floating().is_none()
            && let Some(integer) = info.integer
        {
            return self.integer_floating(integer, ty);
        }
        self.floating_value(info)
            .and_then(|value| self.round_floating(value, ty))
    }

    /// The integer constant `integer` converted to the floating type `ty`,
    /// rounded once from its exact value. Every folded conversion of an
    /// integer to a floating type goes through here.
    /// C99: §6.3.1.4 paragraph 2, pp. 43-44; PDF pp. 55-56.
    pub(super) fn integer_floating(&self, integer: Integer, ty: TypeId) -> Option<Floating> {
        let TypeKind::Scalar(scalar) = self.types.nodes[self.types.non_atomic(ty).index] else {
            return None;
        };
        match self.carrier_type(scalar.real_type()) {
            | Some(format) => integer_value(integer, format).map(Floating::real),
            // Binary128 constants fold apart; the carrier is the nearest.
            | None => integer_value(integer, crate::ir::Type::F80)
                .map(Floating::real)
                .and_then(|value| self.round_floating(value, ty)),
        }
    }

    /// The IR format a real floating scalar's value has, for those the x87
    /// carrier holds exactly; `None` for binary128 and every other type.
    fn carrier_type(&self, scalar: Scalar) -> Option<crate::ir::Type> {
        use crate::ir::Type;
        Some(match scalar.representation() {
            | Scalar::Float16 | Scalar::Fp16 => Type::F16,
            | Scalar::BFloat16 => Type::BF16,
            | Scalar::Float => Type::F32,
            | Scalar::Double => Type::F64,
            | Scalar::LongDouble if self.types.target.long_double_is_double() => Type::F64,
            | Scalar::LongDouble => Type::F80,
            | _ => return None,
        })
    }

    /// Tests scalar constant truth for logical and conditional evaluation.
    /// C99: §6.5.3.3 paragraph 5, p. 79; PDF p. 91.
    /// C99: §6.5.15 paragraph 4, p. 90; PDF p. 102.
    pub(super) fn constant_truth(info: ExpressionInfo<'tu>) -> Option<bool> {
        info.integer
            .map(|v| v.value != 0)
            .or_else(|| info.floating().map(Floating::truth))
            .or_else(|| info.binary128().map(Binary128Value::truth))
    }

    /// C99: §6.3.1.5-§6.3.1.8, pp. 44-45; PDF pp. 56-57.
    pub(super) fn round_floating(&self, value: Floating, ty: TypeId) -> Option<Floating> {
        let ty = self.types.non_atomic(ty);
        let TypeKind::Scalar(scalar) = self.types.nodes[ty.index] else {
            return None;
        };
        if let Some(format) = scalar.real_type().half_format() {
            return Some(Floating {
                real: round_half(value.real, format),
                imag: if scalar.complex() {
                    round_half(value.imag, format)
                } else {
                    LongDouble::ZERO
                },
            });
        }
        let precision = self.floating_precision(scalar)?;
        let complex = scalar.complex();
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
    /// scalar: 1 for binary32 and the 16-bit formats, 2 for binary64 and 3
    /// for x87 extended.
    /// Binary128 has no carrier precision. An interchange or extended type
    /// rounds as the standard type whose representation it shares.
    /// C99: implementation-defined floating representations §6.2.5p10,
    /// p. 34; PDF p. 46; characteristics §5.2.4.2.2, pp. 23-27; PDF pp. 35-39.
    pub(super) fn floating_precision(&self, scalar: Scalar) -> Option<i32> {
        Some(match scalar.representation() {
            // `_Float16` and `__bf16` arithmetic computes in `float`; see
            // [`Self::half_operand`].
            | Scalar::Float
            | Scalar::ComplexFloat
            | Scalar::Float16
            | Scalar::ComplexFloat16
            | Scalar::BFloat16
            | Scalar::Fp16 => 1,
            | Scalar::Double | Scalar::ComplexDouble => 2,
            | Scalar::LongDouble | Scalar::ComplexLongDouble =>
                if self.types.target.long_double_is_double() {
                    2
                } else if self.types.target.long_double_is_binary128() {
                    return None;
                } else {
                    3
                },
            | _ => return None,
        })
    }

    /// Whether every `_Float16` and `__bf16` value is evaluated to the range
    /// and precision of `float` and rounded only by a conversion to its type
    /// (an assignment, a cast, an argument or a return), as GCC evaluates it
    /// on x86-64 (`-fexcess-precision=standard`): constants, the arithmetic
    /// operators, `?:` and the comma operator keep the excess, and a
    /// conversion to another type converts the excess value. Otherwise, as
    /// Clang evaluates it (`-ffloat16-excess-precision=standard` and
    /// `-fbfloat16-excess-precision=standard`), only an operand of `+`, `-`,
    /// `*`, `/` or unary `+` and `-` of its own type that is itself one of
    /// those keeps its excess, and every other use, a conversion between the
    /// two types included, reads the value rounded to its type.
    /// C99: §5.2.4.2.2 paragraph 7, p. 24; PDF p. 36 (evaluation formats,
    /// implementation-defined); §6.3.1.8 paragraph 2, p. 45; PDF p. 57.
    pub(super) fn half_in_float(&self) -> bool {
        self.context.configuration.half_in_float()
    }

    /// Whether `ty` is `_Float16`, `_Complex _Float16` or `__bf16`, which
    /// compute in `float`.
    pub(super) fn half_type(&self, ty: TypeId) -> bool {
        self.half_format(ty).is_some()
    }

    /// The format of `_Float16`, `_Complex _Float16` or `__bf16`; `None` for
    /// any other type, `__fp16` included, whose values promote to `float`
    /// instead.
    pub(super) fn half_format(&self, ty: TypeId) -> Option<HalfFormat> {
        match self.types.nodes[self.types.non_atomic(ty).index] {
            | TypeKind::Scalar(Scalar::Float16 | Scalar::ComplexFloat16) =>
                Some(HalfFormat::Binary16),
            | TypeKind::Scalar(Scalar::BFloat16) => Some(HalfFormat::BFloat16),
            | _ => None,
        }
    }

    /// Whether GCC's cast of `operand` to `target` keeps the operand's value
    /// unrounded: a plain value of one half type converted to the other
    /// (real or complex), which GCC folds only after it parses it, unless
    /// the operand is a literal it folds at once
    /// ([`super::excess::folded_constant`]). See `lowering/half.rs`.
    /// C99: §6.3.1.5 paragraph 2, p. 44; PDF p. 56; §5.2.4.2.2 paragraph 7,
    /// p. 24; PDF p. 36.
    pub(super) fn unrounded_half_cast(&self, operand: ExpressionInfo<'tu>, target: TypeId) -> bool {
        let to = match self.types.nodes[self.types.non_atomic(target).index] {
            | TypeKind::Scalar(Scalar::ComplexFloat16) => Some(HalfFormat::Binary16),
            | _ => self.half_format(target),
        };
        let lookup = |e| Some(self.expression_info(e));
        self.half_in_float()
            && to.is_some()
            && self.half_format(operand.ty).is_some()
            && self.half_format(operand.ty) != to
            && operand.floating().is_some()
            && !super::excess::excess_expression(operand.expression, &lookup)
            && !super::excess::folded_constant(operand.expression, &lookup, &|ty| match self
                .types
                .nodes[self.types.non_atomic(ty).index]
            {
                | TypeKind::Scalar(scalar) => Some(scalar),
                | _ => None,
            })
    }

    /// Whether `ty` is a complex floating type.
    fn complex_type(&self, ty: TypeId) -> bool {
        matches!(
            self.types.nodes[self.types.non_atomic(ty).index],
            TypeKind::Scalar(scalar) if scalar.complex()
        )
    }

    /// Whether `ty` is Clang's storage-only `__fp16`.
    pub(super) fn storage_half(&self, ty: TypeId) -> bool {
        matches!(
            self.types.nodes[self.types.non_atomic(ty).index],
            TypeKind::Scalar(Scalar::Fp16)
        )
    }

    /// The value an operand of a `_Float16` or `__bf16` operator of type
    /// `ty` contributes: its own folded value, excess included, when it has
    /// that type; for GCC, any other half operand's excess and an integer
    /// converted to `float`; for Clang, their values rounded to `ty`. The
    /// result of the operator keeps its `float` precision; see
    /// [`Self::half_in_float`].
    /// C99: §6.3.1.8 paragraph 1, pp. 44-45; PDF pp. 56-57.
    pub(super) fn half_operand(&self, info: ExpressionInfo<'tu>, ty: TypeId) -> Option<Floating> {
        let format = self.half_format(ty)?;
        if self.half_format(info.ty) == Some(format)
            || (self.half_in_float() && self.half_type(info.ty))
        {
            return info.floating();
        }
        // An integer rounds once, to `float` for GCC and to the type for
        // Clang.
        if info.floating().is_none()
            && let Some(integer) = info.integer
        {
            let to = match (self.half_in_float(), format) {
                | (true, _) => crate::ir::Type::F32,
                | (false, HalfFormat::Binary16) => crate::ir::Type::F16,
                | (false, HalfFormat::BFloat16) => crate::ir::Type::BF16,
            };
            return integer_value(integer, to).map(Floating::real);
        }
        let value = self.floating_value(info)?;
        Some(if self.half_in_float() {
            Floating {
                real: value.real.arithmetic(LongDouble::ZERO, 4, 1),
                imag: value.imag.arithmetic(LongDouble::ZERO, 4, 1),
            }
        } else {
            round_halves(value, format)
        })
    }

    /// The folded value of a `_Float16` arithmetic operator computed in
    /// `float` as `value`. GCC keeps it. Clang's constant evaluator rounds
    /// each operation to `_Float16` (`static_assert ((int) (60000.0f16 * 2 /
    /// 4) == 30000)` fails there), while its function code keeps the excess
    /// through the chain, so the rounded value is the constant-context one
    /// and function code evaluates the operator itself
    /// ([`ExpressionInfo::runtime_query`]).
    pub(super) fn half_result(&self, value: Floating, info: &mut ExpressionInfo<'tu>) -> Floating {
        let Some(format) = self.half_format(info.ty).filter(|_| !self.half_in_float()) else {
            return value;
        };
        info.runtime_query = true;
        round_halves(value, format)
    }

    /// The literal value of an `f16` (or GCC's `bf16`) constant: binary32
    /// for GCC, its own format for Clang.
    pub(super) fn half_constant(&self, constant: HalfConstant, format: HalfFormat) -> f32 {
        if self.half_in_float() {
            constant.single
        } else {
            format.widen(constant.bits)
        }
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

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// An arithmetic constant converted to binary128: its folded binary128
    /// value, its standard floating value, which binary128 holds exactly,
    /// or its integer value rounded to nearest.
    /// C99: §6.3.1.4 paragraph 2, pp. 43-44; PDF pp. 55-56; §6.3.1.5
    /// paragraph 1, p. 44; PDF p. 56.
    pub(super) fn binary128_value(info: ExpressionInfo<'tu>) -> Option<Binary128Value> {
        if let Some(value) = info.binary128() {
            return Some(value);
        }
        if let Some(value) = info.floating() {
            return Some(Binary128Value {
                real: binary128_from_carrier(value.real)?,
                imag: binary128_from_carrier(value.imag)?,
            });
        }
        let integer = info.integer?;
        let magnitude = if integer.signed {
            integer.value.unsigned_abs()
        } else {
            integer.value.cast_unsigned()
        };
        Some(Binary128Value::real(Binary128::from_integer(
            integer.signed && integer.value < 0,
            magnitude,
        )))
    }

    /// Records `operand`'s arithmetic constant converted to `ty` in `info`
    /// when either type is binary128: as a binary128 value for a binary128
    /// type, and from a binary128 value as a standard floating, integer or
    /// `_Bool` value. A binary128 result that cannot be folded is marked
    /// unfolded.
    /// C99: §6.3.1.2, p. 43; PDF p. 55; §6.3.1.4-§6.3.1.5, pp. 43-44;
    /// PDF pp. 55-56; §6.3.1.7, p. 44; PDF p. 56.
    pub(super) fn convert_binary128(
        &self,
        operand: ExpressionInfo<'tu>,
        ty: TypeId,
        info: &mut ExpressionInfo<'tu>,
    ) {
        let ty = self.types.non_atomic(ty);
        if self.binary128_type(ty) {
            let complex = matches!(
                self.types.nodes[ty.index],
                TypeKind::Scalar(Scalar::ComplexFloat128)
            );
            info.set_binary128(Self::binary128_value(operand).map(|value| {
                if complex {
                    value
                } else {
                    Binary128Value::real(value.real)
                }
            }));
        } else if let Some(value) = operand.binary128() {
            if matches!(self.types.nodes[ty.index], TypeKind::Scalar(Scalar::Bool)) {
                info.integer = Some(Integer::int(i128::from(value.truth())).cast(1, false));
            } else if let Some((bits, signed)) = self.integer_type(ty) {
                info.integer = value
                    .real
                    .to_integer(signed, bits)
                    .map(|bits_value| Integer::int(bits_value.cast_signed()).cast(bits, signed));
            } else if self.arithmetic(ty) {
                // A real operand's imaginary part is +0, whatever sign
                // negating it gave the folded zero (C99 §6.3.1.7p1).
                let value = if self.complex_type(operand.ty) {
                    value
                } else {
                    Binary128Value::real(value.real)
                };
                info.set_floating(self.floating_from_binary128(value, ty));
            }
        }
        self.mark_unfolded_binary128(info);
    }

    /// Keeps only the value that belongs to a constant's type. An operator
    /// whose operands were converted (a conditional's arms, a usual
    /// arithmetic conversion) may have copied an operand's integer value or
    /// a value of another floating format; the constant's value is the
    /// converted one, rounded as its type rounds, so a floating constant
    /// keeps no integer value and only a binary128 one keeps a binary128
    /// value.
    /// C99: §6.3.1.4 paragraph 2, pp. 43-44; PDF pp. 55-56; §6.3.1.5,
    /// p. 44; PDF p. 56; §6.3.1.8 paragraph 1, pp. 44-45; PDF pp. 56-57.
    pub(super) fn constrain_constant(&self, info: &mut ExpressionInfo<'tu>) {
        let ty = self.types.non_atomic(info.ty);
        let TypeKind::Scalar(scalar) = self.types.nodes[ty.index] else {
            return;
        };
        if !scalar.floating() {
            info.set_binary128(None);
        } else if self.binary128_type(ty) {
            info.integer = None;
            info.set_floating(None);
        } else {
            info.integer = None;
            info.set_binary128(None);
        }
    }

    /// Marks a binary128 arithmetic constant whose value was not folded, so
    /// it is not mistaken for a value that needs no evaluation.
    pub(super) fn mark_unfolded_binary128(&self, info: &mut ExpressionInfo<'tu>) {
        if self.binary128_type(info.ty)
            && info.constant == ConstantClass::Arithmetic
            && info.binary128().is_none()
        {
            info.unfolded_binary128 = true;
        }
    }

    /// A binary128 constant converted to the standard real or complex
    /// floating type `ty`, each component rounded once, as `__trunctfdf2`
    /// and its relatives round it (`__trunctfhf2` and `__trunctfbf2` for
    /// the 16-bit types, not through binary32). `None` when the host
    /// carrier cannot hold the x87 result.
    /// C99: §6.3.1.5 paragraph 2, p. 44; PDF p. 56.
    pub(super) fn floating_from_binary128(
        &self,
        value: Binary128Value,
        ty: TypeId,
    ) -> Option<Floating> {
        let TypeKind::Scalar(scalar) = self.types.nodes[self.types.non_atomic(ty).index] else {
            return None;
        };
        let precision = self.floating_precision(scalar)?;
        let half = scalar.real_type().half_format();
        let convert = |component: Binary128| match (half, precision) {
            | (Some(format), _) => Some(LongDouble::from_double(f64::from(
                format.widen(format.round_binary128(component)),
            ))),
            | (_, 1) => Some(LongDouble::from_double(f64::from(component.to_f32()))),
            | (_, 2) => Some(LongDouble::from_double(component.to_f64())),
            | _ => LongDouble::from_x87(component.to_x87()),
        };
        Some(Floating {
            real: convert(value.real)?,
            imag: if scalar.complex() {
                convert(value.imag)?
            } else {
                LongDouble::ZERO
            },
        })
    }

    /// Folds binary128 `+`, `-`, `*` and `/`. `domains` has bit 0 set for a
    /// complex left operand and bit 1 for a complex right one. A mixed real
    /// and complex operation computes only the components the real operand
    /// takes part in (C99 Annex G.5.1); the product and quotient of two
    /// complex values, which libgcc's `__multc3` and `__divtc3` compute with
    /// their own recovery of infinities, stay unfolded.
    /// C99: §6.5.5-§6.5.6, pp. 82-84; PDF pp. 94-96; §G.5.1 paragraphs
    /// 2-3, pp. 474-475; PDF pp. 486-487.
    pub(super) fn binary128_binary(
        op: BinaryOperator,
        left: Binary128Value,
        right: Binary128Value,
        domains: u8,
    ) -> Option<Binary128Value> {
        use BinaryOperator as B;
        let operation = match op {
            | B::Addition => Binary128::add,
            | B::Subtraction => Binary128::sub,
            | B::Multiplication => Binary128::mul,
            | B::Division => Binary128::div,
            | _ => return None,
        };
        let real = operation(left.real, right.real);
        Some(match (domains, op) {
            | (0, _) => Binary128Value::real(real),
            | (3, B::Addition | B::Subtraction) => Binary128Value {
                real,
                imag: operation(left.imag, right.imag),
            },
            | (1, B::Addition | B::Subtraction) => Binary128Value {
                real,
                imag: left.imag,
            },
            | (2, B::Addition) => Binary128Value {
                real,
                imag: right.imag,
            },
            | (2, B::Subtraction) => Binary128Value {
                real,
                imag: right.imag.negate(),
            },
            | (1, B::Multiplication | B::Division) => Binary128Value {
                real,
                imag: operation(left.imag, right.real),
            },
            | (2, B::Multiplication) => Binary128Value {
                real,
                imag: operation(left.real, right.imag),
            },
            | _ => return None,
        })
    }

    /// Folds binary128 equality and ordering; a NaN operand is unordered,
    /// so only inequality holds.
    /// C99: §F.3 paragraph 1, pp. 445-447; PDF pp. 457-459; §6.5.8
    /// paragraph 6, p. 86; PDF p. 98; §6.5.9 paragraph 3, p. 86; PDF p. 98.
    pub(super) fn binary128_comparison(
        op: BinaryOperator,
        left: Binary128Value,
        right: Binary128Value,
    ) -> Option<Integer> {
        use BinaryOperator as B;
        let comparison = left.real.compare(right.real);
        let imaginary = left.imag.compare(right.imag);
        let equal =
            comparison.is_some_and(Ordering::is_eq) && imaginary.is_some_and(Ordering::is_eq);
        let result = match op {
            | B::Equal => equal,
            | B::NotEqual => !equal,
            | B::LessThan => comparison.is_some_and(Ordering::is_lt),
            | B::LessThanOrEqual => comparison.is_some_and(Ordering::is_le),
            | B::GreaterThan => comparison.is_some_and(Ordering::is_gt),
            | B::GreaterThanOrEqual => comparison.is_some_and(Ordering::is_ge),
            | _ => return None,
        };
        Some(Integer::int(i128::from(result)))
    }
}

/// The exact binary128 value of a standard floating constant's carrier.
/// The value of `integer` rounded once to the floating format `ty`, held
/// exactly in the x87 carrier.
/// C99: §6.3.1.4 paragraph 2, pp. 43-44; PDF pp. 55-56.
pub(super) fn integer_value(integer: Integer, ty: crate::ir::Type) -> Option<LongDouble> {
    use crate::{
        float_parsing::bits,
        ir::Type,
    };
    let bits = if integer.signed {
        bits::from_integer(integer.value, ty)?
    } else {
        bits::from_unsigned(integer.value.cast_unsigned(), ty)?
    };
    let narrow = |bits: u128| u32::try_from(bits).ok();
    Some(match ty {
        | Type::F16 | Type::BF16 => {
            let format = ty.half_format()?;
            LongDouble::from_double(f64::from(format.widen(u16::try_from(bits).ok()?)))
        },
        | Type::F32 => LongDouble::from_double(f64::from(f32::from_bits(narrow(bits)?))),
        | Type::F64 => LongDouble::from_double(f64::from_bits(u64::try_from(bits).ok()?)),
        | _ => LongDouble::from_x87(X87::from_bits(bits))?,
    })
}

/// Both parts of `value` rounded to binary16 or bfloat16.
/// C99: §6.3.1.5 paragraph 2, p. 44; PDF p. 56; §6.3.1.6, p. 44; PDF p. 56.
fn round_halves(value: Floating, format: HalfFormat) -> Floating {
    Floating {
        real: round_half(value.real, format),
        imag: round_half(value.imag, format),
    }
}

/// `value` rounded to binary16 or bfloat16, to nearest with ties to even.
/// C99: §6.3.1.5 paragraph 2, p. 44; PDF p. 56.
pub(super) fn round_half(value: LongDouble, format: HalfFormat) -> LongDouble {
    let ty = match format {
        | HalfFormat::Binary16 => crate::ir::Type::F16,
        | HalfFormat::BFloat16 => crate::ir::Type::BF16,
    };
    crate::float_parsing::bits::constant_bits(&value, ty)
        .and_then(|bits| u16::try_from(bits).ok())
        .map_or(value, |bits| {
            LongDouble::from_double(f64::from(format.widen(bits)))
        })
}

fn binary128_from_carrier(value: LongDouble) -> Option<Binary128> {
    let (negative, exponent, significand) = value.x87_parts()?;
    let bits =
        u128::from(significand) | u128::from(exponent | if negative { 0x8000 } else { 0 }) << 64;
    Some(Binary128::from_x87(X87::from_bits(bits)))
}

/// A folded binary128 real or complex constant (GNU extension: GCC manual,
/// "Additional Floating Types"). A real value has a zero imaginary part.
/// C99: §6.2.5 paragraph 13, p. 34; PDF p. 46.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Binary128Value {
    pub(crate) real: Binary128,
    pub(crate) imag: Binary128,
}

impl Binary128Value {
    /// The two components' little-endian bytes, real first.
    pub(super) fn to_bytes(self) -> [u8; 32] {
        let mut bytes = [0; 32];
        bytes[..16].copy_from_slice(&self.real.to_bits().to_le_bytes());
        bytes[16..].copy_from_slice(&self.imag.to_bits().to_le_bytes());
        bytes
    }

    pub(super) fn from_bytes(bytes: [u8; 32]) -> Self {
        let component = |range: std::ops::Range<usize>| {
            let mut part = [0; 16];
            part.copy_from_slice(&bytes[range]);
            Binary128::from_bits(u128::from_le_bytes(part))
        };
        Self {
            real: component(0..16),
            imag: component(16..32),
        }
    }

    pub(crate) const fn real(value: Binary128) -> Self {
        Self {
            real: value,
            imag: Binary128::zero(false),
        }
    }

    /// Tests whether a value differs from zero; a NaN does.
    /// C99: §6.3.1.2 paragraph 1, p. 43; PDF p. 55.
    pub(super) const fn truth(self) -> bool {
        !self.real.is_zero() || !self.imag.is_zero()
    }

    /// The additive inverse of both components.
    /// C99: §6.5.3.3 paragraph 3, p. 79; PDF p. 91.
    pub(super) const fn negate(self) -> Self {
        Self {
            real: self.real.negate(),
            imag: self.imag.negate(),
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
