//! Scalar floating GCC calls, preserving signed zero and NaN signs.
//! C99: §7.12.3, pp. 216-218; PDF pp. 228-230; §7.12.11.1, p. 236;
//! PDF p. 248. GNU extension and constant folding: GCC Other Builtins,
//! <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>;
//! C99 §6.6p10, p. 96; PDF p. 108 (additional constant expressions).

use super::FloatOperation;
use crate::translation_phases::{
    parsing::syntax::Expression,
    semantic_analysis::{
        Analyzer,
        ConstantClass,
        ConversionKind,
        ExpressionInfo,
        Integer,
        Scalar,
        SemanticErrorKind,
        TypeKind,
        constants::Floating,
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    pub(in crate::translation_phases::semantic_analysis) fn floating_builtin(
        &mut self,
        expression: &'tu Expression<'tu>,
        args: &[&'tu Expression<'tu>],
        operation: FloatOperation,
        scalar: Option<Scalar>,
    ) -> ExpressionInfo<'tu> {
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
                    TypeKind::Scalar(
                        Scalar::Float | Scalar::Double | Scalar::LongDouble | Scalar::Float128
                    )
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
        if args
            .iter()
            .all(|arg| self.expression_info(arg).constant == ConstantClass::Arithmetic)
            && !self.binary128_type(operand)
            && let Some(value) = self
                .floating_value(first)
                .and_then(|value| self.round_floating(value, operand))
            && let Some((sign, exponent, significand)) = value.real.x87_parts()
        {
            if operation == FloatOperation::Copysign {
                let second = self.expression_info(args[1]);
                if let Some(sign) = self
                    .floating_value(second)
                    .and_then(|value| self.round_floating(value, operand))
                    .and_then(|value| value.real.x87_parts())
                    .map(|parts| parts.0)
                {
                    result.floating = Some(Floating::real(value.real.with_sign(sign)));
                    result.constant = ConstantClass::Arithmetic;
                }
            } else {
                let value = if operation == FloatOperation::Signbit {
                    sign
                } else {
                    exponent == 0x7FFF && significand == 1 << 63
                };
                result.integer = Some(Integer::int(i128::from(value)));
                result.ice = true;
                result.constant = ConstantClass::Arithmetic;
            }
        }
        result
    }
}
