//! Phase-7 type selections and GNU math-header primitives.
//! C11: §6.5.1.1p2-3, p. 79; PDF p. 97. GNU extensions follow
//! <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html> and
//! <https://gcc.gnu.org/onlinedocs/gcc/Complex.html>.
//! Children are visited by the ordinary explicit semantic work stack.

use super::{
    Analyzer,
    ConstantClass,
    ExpressionInfo,
    Integer,
    Scalar,
    SemanticErrorKind,
    SyntaxOperand,
    TagKind,
    TypeId,
    TypeKind,
    constants::Floating,
    expressions::ValueCategory,
};
use crate::{
    float_parsing::LongDouble,
    translation_phases::{
        parsing::{
            Builtin,
            syntax::{
                Expression,
                UnaryOperator,
            },
        },
        preprocessing::KeywordTokenType,
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Clang math primitives used by glibc's older-compiler binary128 aliases.
    /// Binary128 operations retain constant eligibility without numerical
    /// folding; infinity/NaN constructors in the aliases have type double.
    pub(super) fn math128_builtin(
        &mut self,
        e: &'tu Expression<'tu>,
        mut function: &'tu Expression<'tu>,
        arguments: &[&'tu Expression<'tu>],
    ) -> Option<ExpressionInfo<'tu>> {
        use super::ExpressionType as E;
        while let E::Parenthesized { expression } = function.kind {
            function = expression;
        }
        let E::Identifier(name) = function.kind else {
            return None;
        };
        let (scalar, count, nan, operation) = match self.context.string_cache.at(name.name) {
            | "__builtin_huge_val" | "__builtin_inf" => (Scalar::Double, 0, false, false),
            | "__builtin_nan" | "__builtin_nans" => (Scalar::Double, 1, true, false),
            | "__builtin_huge_valf128" | "__builtin_inff128" => (Scalar::Float128, 0, false, false),
            | "__builtin_nanf128" | "__builtin_nansf128" => (Scalar::Float128, 1, true, false),
            | "__builtin_fabsf128" => (Scalar::Float128, 1, false, true),
            | "__builtin_copysignf128" => (Scalar::Float128, 2, false, true),
            | _ => return None,
        };
        if arguments.len() != count {
            return Some(self.invalid_expression(e, SemanticErrorKind::InvalidArgumentCount));
        }
        let result_type = self.types.scalar(scalar);
        let tag_type = nan.then(|| {
            let character = self
                .types
                .scalar(Scalar::Char)
                .qualified(super::TypeQualifiers::CONST);
            self.types.intern(TypeKind::Pointer(character))
        });
        let mut constant = true;
        for &argument in arguments {
            let info = self.expression_info(argument);
            let ty = self.converted(info);
            if self.types.unanalyzed(ty) {
                return Some(Self::expression_result(e, self.types.unknown()));
            }
            let mut literal = argument;
            while let E::Parenthesized { expression } = literal.kind {
                literal = expression;
            }
            if (operation && !self.arithmetic(ty))
                || tag_type.is_some_and(|target| !self.assignment_compatible(target, info))
            {
                return Some(self.invalid_expression(e, SemanticErrorKind::InvalidArgumentType));
            }
            if operation {
                constant &= info.constant == ConstantClass::Arithmetic;
                self.convert(
                    argument,
                    result_type,
                    super::expressions::ConversionKind::Assignment,
                );
            }
            if let Some(target) = tag_type {
                constant &= matches!(literal.kind, E::StringLiteral(_));
                self.convert(
                    argument,
                    target,
                    super::expressions::ConversionKind::Assignment,
                );
            }
        }
        let mut result = Self::expression_result(e, result_type);
        if constant {
            result.constant = ConstantClass::Arithmetic;
            if scalar == Scalar::Float128 {
                result.unfolded_binary128 = true;
            } else {
                result.floating = Some(Floating::real(LongDouble::from_double(if nan {
                    f64::NAN
                } else {
                    f64::INFINITY
                })));
            }
        }
        Some(result)
    }

    /// These selections have their own unevaluated-operand ICE rules.
    pub(super) fn type_generic_constant(&self, e: &'tu Expression<'tu>) -> bool {
        use super::ExpressionType as E;
        matches!(e.kind, E::Generic(_))
            || matches!(e.kind, E::Builtin(b) if matches!(b.keyword, KeywordTokenType::BuiltinTypesCompatible | KeywordTokenType::BuiltinChooseExpr))
            || matches!(
                e.kind,
                E::Unary {
                    operator: UnaryOperator::Real | UnaryOperator::Imag,
                    ..
                }
            )
            || matches!(e.kind, E::Call { function_expression, .. } if self.classify_type_callee(function_expression))
    }

    pub(super) fn classify_type_callee(&self, mut function: &'tu Expression<'tu>) -> bool {
        use super::ExpressionType as E;
        while let E::Parenthesized { expression } = function.kind {
            function = expression;
        }
        matches!(function.kind, E::Identifier(name) if self.context.string_cache.at(name.name) == "__builtin_classify_type")
            && self.expression_info(function).binding.is_none_or(|id| matches!(self.types.nodes[self.bindings[id].ty.index], TypeKind::Function { result, .. } if result == self.types.unknown()))
    }

    pub(super) fn binary128_type(&self, ty: TypeId) -> bool {
        matches!(
            self.types.nodes[ty.index],
            TypeKind::Scalar(Scalar::Float128 | Scalar::ComplexFloat128)
        )
    }

    /// GNU real/imag preserves a complex component's lvalue and qualifiers;
    /// on a real operand the imaginary part is a zero rvalue.
    pub(super) fn real_imag(
        &mut self,
        e: &'tu Expression<'tu>,
        op: UnaryOperator,
        operand: ExpressionInfo<'tu>,
    ) -> ExpressionInfo<'tu> {
        let scalar = match self.types.nodes[operand.ty.index] {
            | TypeKind::Scalar(s) => s,
            | TypeKind::Tag(id) if self.types.tags[id].kind == TagKind::Enum =>
                self.types.tags[id].compatible.get(),
            | TypeKind::Unknown => return Self::expression_result(e, operand.ty),
            | _ => return self.invalid_expression(e, SemanticErrorKind::InvalidUnaryOperand),
        };
        if scalar == Scalar::Void {
            return self.invalid_expression(e, SemanticErrorKind::InvalidUnaryOperand);
        }
        let component = match scalar {
            | Scalar::ComplexFloat => Scalar::Float,
            | Scalar::ComplexDouble => Scalar::Double,
            | Scalar::ComplexLongDouble => Scalar::LongDouble,
            | Scalar::ComplexFloat128 => Scalar::Float128,
            | _ => scalar,
        };
        let complex = component != scalar;
        let ty = if complex {
            self.types
                .scalar(component)
                .qualified(operand.ty.qualifiers)
        } else {
            operand.ty
        };
        let mut result = Self::expression_result(e, ty);
        if complex || op == UnaryOperator::Real {
            result.category = operand.category;
            result.register = operand.register;
            result.bit_field = operand.bit_field;
            result.binding = operand.binding;
        }
        if scalar.integer() {
            result.integer = if op == UnaryOperator::Imag {
                Some(Integer::int(0))
            } else {
                operand.integer
            };
            result.ice = op == UnaryOperator::Imag || operand.ice;
            if result.ice {
                result.constant = ConstantClass::Arithmetic;
            }
        } else {
            result.unfolded_binary128 = operand.unfolded_binary128;
            result.floating = operand.floating.map(|value| {
                Floating::real(if op == UnaryOperator::Real {
                    value.real
                } else {
                    value.imag
                })
            });
            if !complex && op == UnaryOperator::Imag {
                result.category = ValueCategory::Rvalue;
                result.constant = ConstantClass::Arithmetic;
                result.unfolded_binary128 = component == Scalar::Float128;
                if !result.unfolded_binary128 {
                    result.floating = Some(Floating::real(LongDouble::ZERO));
                }
            } else {
                result.constant = operand.constant;
            }
        }
        result
    }

    /// GCC Other Builtins: top-level qualifications are ignored by the type
    /// compatibility builtin; `choose_expr` performs no usual conversions.
    pub(super) fn type_generic_builtin(
        &mut self,
        e: &'tu Expression<'tu>,
        builtin: &'tu Builtin<'tu>,
    ) -> ExpressionInfo<'tu> {
        if builtin.recovered {
            return Self::expression_result(e, self.types.unknown());
        }
        if builtin.keyword == KeywordTokenType::BuiltinChooseExpr {
            let [
                SyntaxOperand::Expression(condition),
                SyntaxOperand::Expression(yes),
                SyntaxOperand::Expression(no),
            ] = &*builtin.operands
            else {
                return Self::expression_result(e, self.types.unknown());
            };
            let condition = self.expression_info(condition);
            if !condition.ice || condition.integer.is_none() {
                return self.invalid_expression(e, SemanticErrorKind::InvalidChooseCondition);
            }
            let selected = if condition.integer.is_some_and(|value| value.value != 0) {
                yes
            } else {
                no
            };
            return ExpressionInfo {
                expression: e,
                ..self.expression_info(selected)
            };
        }
        let [left, right] = &*builtin.operands else {
            return Self::expression_result(e, self.types.unknown());
        };
        let left = self.operand_type(*left).unqualified();
        let right = self.operand_type(*right).unqualified();
        let mut result = Self::expression_result(e, self.types.scalar(Scalar::Int));
        if !self.types.unanalyzed(left) && !self.types.unanalyzed(right) {
            result.integer = Some(Integer::int(i128::from(
                self.types.composite(left, right).is_some(),
            )));
            result.ice = true;
            result.constant = ConstantClass::Arithmetic;
        }
        result
    }

    /// GCC Other Builtins: expression arguments undergo default argument
    /// conversions for classification, without evaluating their value.
    pub(super) fn classify_type(
        &mut self,
        e: &'tu Expression<'tu>,
        arguments: &[&'tu Expression<'tu>],
    ) -> ExpressionInfo<'tu> {
        if arguments.len() != 1 {
            return self.invalid_expression(e, SemanticErrorKind::InvalidArgumentCount);
        }
        let class = if let Some(&argument) = arguments.first() {
            let ty = self.converted(self.expression_info(argument));
            match self.types.nodes[ty.index] {
                | TypeKind::Scalar(Scalar::Bool) => 4,
                | TypeKind::Scalar(s) if s.integer() => 1,
                | TypeKind::Scalar(Scalar::Void) => 0,
                | TypeKind::Scalar(
                    Scalar::ComplexFloat
                    | Scalar::ComplexDouble
                    | Scalar::ComplexLongDouble
                    | Scalar::ComplexFloat128,
                ) => 9,
                | TypeKind::Scalar(_) => 8,
                | TypeKind::Pointer(_) => 5,
                | TypeKind::Tag(id) => match self.types.tags[id].kind {
                    | TagKind::Struct => 12,
                    | TagKind::Union => 13,
                    | TagKind::Enum => 1,
                },
                | _ => return Self::expression_result(e, self.types.unknown()),
            }
        } else {
            -1
        };
        let mut result = Self::expression_result(e, self.types.scalar(Scalar::Int));
        result.integer = Some(Integer::int(class));
        result.ice = true;
        result.constant = ConstantClass::Arithmetic;
        result
    }
}
