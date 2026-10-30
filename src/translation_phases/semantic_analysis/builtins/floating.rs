//! Scalar floating GCC calls, preserving signed zero and NaN signs.
//! C99: §7.12.3, pp. 216-218; PDF pp. 228-230; §7.12.11.1, p. 236;
//! PDF p. 248. GNU extension and constant folding: GCC Other Builtins,
//! <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>;
//! C99 §6.6p10, p. 96; PDF p. 108 (additional constant expressions).

use crate::{
    binary128::Binary128,
    builtins::{
        FloatCompare,
        FloatOperation,
    },
    configuration::CompilerIdentity,
    float_parsing::LongDouble,
    translation_phases::{
        parsing::syntax::{
            Expression,
            ExpressionType,
        },
        preprocessing::StringTokenType,
        semantic_analysis::{
            Analyzer,
            ConstantClass,
            ConversionKind,
            ExpressionInfo,
            Integer,
            Scalar,
            SemanticErrorKind,
            TypeId,
            TypeKind,
            constants::{
                Binary128Value,
                Floating,
            },
        },
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// GCC constructors implement the math-header constants as arithmetic
    /// constant expressions. A dynamic NaN tag remains an ordinary libm call;
    /// only a string literal may initialize a static object.
    /// C99: §7.12p3-5, pp. 212-213; PDF pp. 224-225; §7.12.11.2p2,
    /// p. 237; PDF p. 249; §6.6p10, p. 96; PDF p. 108 (GNU extension).
    pub(in crate::translation_phases::semantic_analysis) fn floating_constant_builtin(
        &mut self,
        expression: &'tu Expression<'tu>,
        args: &[&'tu Expression<'tu>],
        scalar: Scalar,
        nan: bool,
    ) -> ExpressionInfo<'tu> {
        let ty = self.types.scalar(scalar);
        let mut result = Self::expression_result(expression, ty);
        let tag = if nan {
            let mut literal = args[0];
            while let ExpressionType::Parenthesized { expression } = literal.kind {
                literal = expression;
            }
            let ExpressionType::StringLiteral(StringTokenType::String(tag)) = literal.kind else {
                return result;
            };
            Some(
                self.context
                    .literal_text_in(self.scratch, tag, false)
                    .unwrap_or(""),
            )
        } else {
            None
        };
        result.constant = ConstantClass::Arithmetic;
        // `long double` is binary128 on WebAssembly, so its constants are
        // binary128 values there, as Clang folds them.
        if self.binary128_type(ty) {
            let value = match tag {
                | Some(tag) => Binary128::nan(true, nan_payload_wide(tag.as_bytes())),
                | None => Binary128::infinity(false),
            };
            result.set_binary128(Some(Binary128Value::real(value)));
            self.mark_unfolded_binary128(&mut result);
            return result;
        }
        let value = match tag {
            | Some(tag) => LongDouble::quiet_nan(
                nan_payload(tag.as_bytes()),
                self.floating_precision(scalar).unwrap_or(3),
            ),
            | None => LongDouble::from_double(f64::INFINITY),
        };
        result.set_floating(Some(Floating::real(value)));
        result
    }

    pub(in crate::translation_phases::semantic_analysis) fn floating_builtin(
        &mut self,
        expression: &'tu Expression<'tu>,
        args: &[&'tu Expression<'tu>],
        operation: FloatOperation,
        scalar: Option<Scalar>,
    ) -> ExpressionInfo<'tu> {
        if let FloatOperation::Compare(compare) = operation {
            return self.floating_compare_builtin(expression, args, compare);
        }
        let (classes, args) = args.split_at(operation.operand_index());
        let int = self.types.scalar(Scalar::Int);
        for &class in classes {
            _ = self.common_argument(class, int);
        }
        let first = self.expression_info(args[0]);
        let operand = if let Some(scalar) = scalar {
            let ty = self.types.scalar(scalar);
            for &argument in args {
                let info = self.expression_info(argument);
                if !self.types.unanalyzed(info.ty) && !self.assignment_compatible(ty, info) {
                    self.error(
                        SemanticErrorKind::InvalidArgumentType,
                        argument.source_vectors,
                        None,
                        None,
                    );
                }
                self.convert(argument, ty, ConversionKind::Assignment);
            }
            ty
        } else {
            let ty = self.converted(first);
            if !self.types.unanalyzed(ty)
                && !matches!(
                    self.types.nodes[ty.index],
                    TypeKind::Scalar(s) if s.floating() && !s.complex()
                )
            {
                return self.invalid_expression(expression, SemanticErrorKind::InvalidArgumentType);
            }
            ty
        };
        let result_type = if operation == FloatOperation::Copysign {
            operand
        } else {
            self.types.scalar(Scalar::Int)
        };
        let mut result = Self::expression_result(expression, result_type);
        result.operation_type = Some(operand);
        if operation == FloatOperation::Issignaling && first.constant == ConstantClass::Arithmetic {
            if let Some(signaling) = self.signaling_constant(first, operand) {
                result.integer = Some(Integer::int(i128::from(signaling)));
                result.ice = true;
                result.constant = ConstantClass::Arithmetic;
            }
            return result;
        }
        if classes
            .iter()
            .chain(args)
            .all(|arg| self.expression_info(arg).constant == ConstantClass::Arithmetic)
            && !self.binary128_type(operand)
            && let Some(value) = self.converted_floating(first, operand)
            && let Some((sign, exponent, significand)) = value.real.x87_parts()
        {
            if operation == FloatOperation::Copysign {
                let second = self.expression_info(args[1]);
                if let Some(sign) = self
                    .converted_floating(second, operand)
                    .and_then(|value| value.real.x87_parts())
                    .map(|parts| parts.0)
                {
                    result.set_floating(Some(Floating::real(value.real.with_sign(sign))));
                    result.constant = ConstantClass::Arithmetic;
                }
            } else if let Some(value) =
                self.classify_constant(operation, classes, operand, (sign, exponent, significand))
            {
                result.integer = Some(Integer::int(value));
                result.ice = true;
                result.constant = ConstantClass::Arithmetic;
            }
        }
        result
    }

    /// Whether the constant `info` of the real floating type `operand` is a
    /// signaling NaN. The value is read as it stands, not rounded to
    /// `operand` again, which would quiet it: `__builtin_nans` and its
    /// negation stay signaling. An x87 `long double` follows the claimed
    /// compiler for the encodings the format leaves unsupported (a pseudo-NaN
    /// or pseudo-infinity, with the explicit integer bit clear, and an
    /// unnormal): GCC calls them signaling, as glibc's `issignaling` does,
    /// and Clang does not.
    /// C23: §7.12.3.8, pp. 240-241; PDF pp. 253-254 (N3220).
    fn signaling_constant(&self, info: ExpressionInfo<'tu>, operand: TypeId) -> Option<bool> {
        if self.binary128_type(operand) {
            let value = self.binary128_value(info)?.real;
            return Some(value.is_nan() && value.to_bits() & 1 << 111 == 0);
        }
        let TypeKind::Scalar(scalar) = self.types.nodes[self.types.non_atomic(operand).index]
        else {
            return None;
        };
        let (_, exponent, significand) = self.floating_value(info)?.real.x87_parts()?;
        let quiet_nan_bits = significand >> 62;
        let nan_fraction = significand & ((1 << 62) - 1) != 0;
        let signaling = exponent == 0x7FFF && quiet_nan_bits == 0b10 && nan_fraction;
        let unsupported = self.floating_precision(scalar) == Some(3)
            && exponent != 0
            && significand >> 63 == 0
            && !matches!(
                self.context.configuration.compiler_identity(),
                CompilerIdentity::Clang | CompilerIdentity::ClangCl
            );
        Some(signaling || unsupported)
    }

    /// `__builtin_isless(x, y)` and its siblings: both operands are real,
    /// at least one floating, and the usual arithmetic conversions bring
    /// them to the type they are compared in, as GCC and Clang check them.
    /// Constant operands fold, a NaN being unordered.
    /// C99: §7.12.14p1, p. 240; PDF p. 252; §6.3.1.8p1, pp. 44-45;
    /// PDF pp. 56-57.
    fn floating_compare_builtin(
        &mut self,
        expression: &'tu Expression<'tu>,
        args: &[&'tu Expression<'tu>],
        compare: FloatCompare,
    ) -> ExpressionInfo<'tu> {
        let (left, right) = (self.expression_info(args[0]), self.expression_info(args[1]));
        let (lt, rt) = (self.converted(left), self.converted(right));
        if self.types.unanalyzed(lt) || self.types.unanalyzed(rt) {
            return Self::expression_result(expression, self.types.unknown());
        }
        let floating = |this: &Self, ty: TypeId| {
            matches!(
                this.types.nodes[this.types.non_atomic(ty).index],
                TypeKind::Scalar(s) if s.floating() && !s.complex()
            )
        };
        if !(self.real(lt) && self.real(rt)) || !(floating(self, lt) || floating(self, rt)) {
            return self.invalid_expression(expression, SemanticErrorKind::InvalidArgumentType);
        }
        let operand = self.common_arithmetic(left, right, lt, rt);
        let mut result = Self::expression_result(expression, self.types.scalar(Scalar::Int));
        result.operation_type = Some(operand);
        if left.constant == ConstantClass::Arithmetic && right.constant == ConstantClass::Arithmetic
        {
            let ordering = if self.binary128_type(operand) {
                self.binary128_value(left)
                    .zip(self.binary128_value(right))
                    .map(|(left, right)| left.real.compare(right.real))
            } else {
                self.converted_floating(left, operand)
                    .zip(self.converted_floating(right, operand))
                    .map(|(left, right)| match left.real.compare(right.real) {
                        | 2 => None,
                        | order => Some(order.cmp(&0)),
                    })
            };
            if let Some(ordering) = ordering {
                result.integer = Some(Integer::int(i128::from(compare.holds(ordering))));
                result.ice = true;
                result.constant = ConstantClass::Arithmetic;
            }
        }
        result
    }

    /// `__builtin_complex(real, imaginary)`: after lvalue conversion both
    /// operands have the same real floating type, without promotion, and the
    /// result has the corresponding complex type. Two arithmetic constants
    /// form an arithmetic constant, so glibc's `CMPLX` initializes static
    /// objects. C99: complex types §6.2.5p11, p. 34; PDF p. 46; arithmetic
    /// constants §6.6p8, p. 96; PDF p. 108; `CMPLX` is C11 §7.3.9.3, p. 197;
    /// PDF p. 215. GNU extension: GCC "Other Builtins"; Clang rejects
    /// operands of different types.
    pub(in crate::translation_phases::semantic_analysis) fn complex_builtin(
        &mut self,
        expression: &'tu Expression<'tu>,
        args: &[&'tu Expression<'tu>],
    ) -> ExpressionInfo<'tu> {
        let mut parts = [None; 2];
        for (part, &argument) in parts.iter_mut().zip(args) {
            let info = self.expression_info(argument);
            let ty = self.converted(info);
            if self.types.unanalyzed(ty) {
                return Self::expression_result(expression, self.types.unknown());
            }
            let ty = self.types.non_atomic(ty);
            let Some(scalar) = (match self.types.nodes[ty.index] {
                | TypeKind::Scalar(s) if s.floating() && !s.complex() => Some(s),
                | _ => None,
            }) else {
                self.error(
                    SemanticErrorKind::InvalidArgumentType,
                    argument.source_vectors,
                    None,
                    None,
                );
                return Self::expression_result(expression, self.types.unknown());
            };
            *part = Some((scalar, ty, info));
        }
        let [Some((scalar, operand, real)), Some((other, _, imaginary))] = parts else {
            return Self::expression_result(expression, self.types.unknown());
        };
        if scalar != other {
            self.error(
                SemanticErrorKind::InvalidArgumentType,
                args[1].source_vectors,
                None,
                None,
            );
            return Self::expression_result(expression, self.types.unknown());
        }
        let complex = scalar.complex_type();
        let mut result = Self::expression_result(expression, self.types.scalar(complex));
        result.operation_type = Some(operand);
        if real.constant == ConstantClass::Arithmetic
            && imaginary.constant == ConstantClass::Arithmetic
        {
            let value = |info| self.converted_floating(info, operand);
            if self.binary128_type(operand) {
                result.set_binary128(
                    self.binary128_value(real)
                        .zip(self.binary128_value(imaginary))
                        .map(|(re, im)| Binary128Value {
                            real: re.real,
                            imag: im.real,
                        }),
                );
                result.constant = ConstantClass::Arithmetic;
                self.mark_unfolded_binary128(&mut result);
            } else if let Some((re, im)) = value(real).zip(value(imaginary)) {
                result.set_floating(Some(Floating {
                    real: re.real,
                    imag: im.real,
                }));
                result.constant = ConstantClass::Arithmetic;
            }
        }
        result
    }

    /// Folds a classification of a constant already rounded to `operand`,
    /// from its exact x87 parts. A value is normal in its own format when its
    /// exponent reaches that format's minimum: binary32 and binary64
    /// subnormals are normal x87 values. C99: §7.12.3, pp. 216-217;
    /// PDF pp. 228-229; normalized numbers §5.2.4.2.2p2, p. 23; PDF p. 35.
    fn classify_constant(
        &self,
        operation: FloatOperation,
        classes: &[&'tu Expression<'tu>],
        operand: TypeId,
        (sign, exponent, significand): (bool, u16, u64),
    ) -> Option<i128> {
        let TypeKind::Scalar(scalar) = self.types.nodes[self.types.non_atomic(operand).index]
        else {
            return None;
        };
        let minimum_exponent = match self.floating_precision(scalar)? {
            // A `_Float16` is normal from 2^-14, whatever precision it was
            // evaluated to; a `__bf16` from 2^-126, as a `float`.
            | _ if matches!(scalar, Scalar::Float16 | Scalar::Fp16) => 16383 - 14,
            | 1 => 16383 - 126,
            | 2 => 16383 - 1022,
            | _ => 1,
        };
        let infinite = exponent == 0x7FFF && significand == 1 << 63;
        let nan = exponent == 0x7FFF && !infinite;
        let zero = significand == 0;
        let normal = exponent != 0x7FFF && !zero && exponent >= minimum_exponent;
        let class = |index: usize| {
            let info = self.expression_info(classes[index]);
            info.integer.filter(|_| info.ice).map(|value| value.value)
        };
        Some(match operation {
            | FloatOperation::Signbit => i128::from(sign),
            | FloatOperation::Isinf => i128::from(infinite),
            | FloatOperation::Isnan => i128::from(nan),
            | FloatOperation::Isfinite => i128::from(exponent != 0x7FFF),
            | FloatOperation::Isnormal => i128::from(normal),
            | FloatOperation::IsinfSign => match (infinite, sign) {
                | (false, _) => 0,
                | (true, false) => 1,
                | (true, true) => -1,
            },
            | FloatOperation::Fpclassify => class(if nan {
                0
            } else if infinite {
                1
            } else if normal {
                2
            } else if zero {
                4
            } else {
                3
            })?,
            // Folded apart, from the value before rounding.
            | FloatOperation::Issignaling
            | FloatOperation::Copysign
            | FloatOperation::Compare(_) => return None,
        })
    }
}

/// GCC/glibc numeric NaN tags accept an unsigned integer in base 0. An empty
/// or unrecognized n-char sequence selects the implementation's default NaN.
/// C99: §7.12.11.2p2, p. 237; PDF p. 249 (implementation-defined payload).
pub(in crate::translation_phases::semantic_analysis) fn nan_payload(tag: &[u8]) -> u64 {
    let Some((radix, digits)) = payload_digits(tag) else {
        return 0;
    };
    let mut payload = 0_u64;
    for digit in digits {
        payload = payload
            .saturating_mul(u64::from(radix))
            .saturating_add(u64::from(digit));
    }
    payload
}

/// The payload a binary128 NaN builtin takes from its tag: the tag's
/// unsigned value in base 0, of which GCC keeps the low bits that fit the
/// fraction, so a payload may exceed 64 bits; an unrecognized tag gives
/// zero.
/// GNU extension: GCC manual, "Other Builtins" (`__builtin_nanf128`).
pub(in crate::translation_phases::semantic_analysis) fn nan_payload_wide(tag: &[u8]) -> u128 {
    let Some((radix, digits)) = payload_digits(tag) else {
        return 0;
    };
    let mut payload = 0_u128;
    for digit in digits {
        payload = payload
            .wrapping_mul(u128::from(radix))
            .wrapping_add(u128::from(digit));
    }
    payload
}

/// The radix and digit values of a NaN tag in base 0, or `None` when a
/// character is not a digit of the radix.
fn payload_digits(tag: &[u8]) -> Option<(u32, impl Iterator<Item = u32>)> {
    let tag = tag.split(|&byte| byte == 0).next().unwrap_or_default();
    let (radix, digits) = if tag.starts_with(b"0x") || tag.starts_with(b"0X") {
        (16, &tag[2..])
    } else if tag.starts_with(b"0") {
        (8, tag)
    } else {
        (10, tag)
    };
    digits
        .iter()
        .all(|&byte| char::from(byte).is_digit(radix))
        .then(|| {
            (
                radix,
                digits
                    .iter()
                    .filter_map(move |&byte| char::from(byte).to_digit(radix)),
            )
        })
}
