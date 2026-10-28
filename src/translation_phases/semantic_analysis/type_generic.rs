//! Phase-7 type selections and GNU math-header primitives.
//! C11: §6.5.1.1p2-3, p. 79; PDF p. 97. GNU extensions follow
//! <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html> and
//! <https://gcc.gnu.org/onlinedocs/gcc/Complex.html>.
//! Children are visited by the ordinary explicit semantic work stack.

use super::{
    Analyzer,
    SemanticErrorKind,
    constants::{
        Binary128Value,
        Floating,
    },
    expressions::{
        ConstantClass,
        ExpressionInfo,
        ValueCategory,
    },
    integer::Integer,
    types::{
        Scalar,
        TagKind,
        TypeId,
        TypeKind,
    },
};
use crate::{
    binary128::Binary128,
    float_parsing::LongDouble,
    translation_phases::{
        parsing::{
            Builtin,
            SyntaxOperand,
            syntax::{
                Expression,
                UnaryOperator,
            },
        },
        preprocessing::KeywordTokenType,
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// GCC Other Builtins: top-level qualifications are ignored by the type
    /// compatibility builtin; `choose_expr` performs no usual conversions.
    /// GNU extension: GCC manual, "Other Builtins".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
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
            // C99 §5.1.1.3p1: suppress a dependent constraint after operand
            // failure.
            if self.types.unanalyzed(condition.ty) {
                let ty = self.types.recovery_type(&[condition.ty]);
                return Self::expression_result(e, ty);
            }
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
                selected_expression: Some(selected),
                ..self.expression_info(selected)
            };
        }
        let [left, right] = &*builtin.operands else {
            return Self::expression_result(e, self.types.unknown());
        };
        let left = self.operand_type(*left);
        let right = self.operand_type(*right);
        let left = self.types.unqualified_array(left);
        let right = self.types.unqualified_array(right);
        let ty = if self.types.unanalyzed(left) || self.types.unanalyzed(right) {
            self.types.recovery_type(&[left, right])
        } else {
            self.types.scalar(Scalar::Int)
        };
        let mut result = Self::expression_result(e, ty);
        if !self.types.unanalyzed(left) && !self.types.unanalyzed(right) {
            result.integer = Some(Integer::int(i128::from(
                self.types.composite(left, right).is_some(),
            )));
            result.ice = true;
            result.constant = ConstantClass::Arithmetic;
        }
        result
    }

    /// These selections have their own unevaluated-operand ICE rules.
    /// GNU extension: GCC manual, "Other Builtins".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
    /// GNU extension: GCC manual, "Complex".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Complex.html>
    /// C11: §6.5.1.1 paragraphs 3-4, p. 79; PDF p. 97.
    pub(super) fn type_generic_constant(&self, e: &'tu Expression<'tu>) -> bool {
        use super::ExpressionType as E;
        matches!(e.kind, E::Generic(_))
            || matches!(e.kind, E::Builtin(b) if keyword_selection(b.keyword))
            || matches!(
                e.kind,
                E::Unary {
                    operator: UnaryOperator::Real | UnaryOperator::Imag,
                    ..
                }
            )
            || matches!(e.kind, E::Call { function_expression, .. } if self.classify_type_callee(function_expression))
    }

    /// Clang math primitives used by glibc's older-compiler binary128 aliases.
    /// Binary128 operations retain constant eligibility without numerical
    /// folding; infinity/NaN constructors in the aliases have type double.
    /// GNU extension: GCC manual, "Floating Types".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Floating-Types.html>
    /// GNU extension: GCC manual, "Other Builtins".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
    pub(super) fn math128_builtin(
        &mut self,
        e: &'tu Expression<'tu>,
        rule: crate::builtins::Math128,
        arguments: &[&'tu Expression<'tu>],
    ) -> ExpressionInfo<'tu> {
        use super::ExpressionType as E;
        let crate::builtins::Math128 {
            result: scalar,
            arguments: count,
            tag: nan,
            arithmetic: operation,
            quiet,
        } = rule;
        if arguments.len() != count {
            return self.invalid_expression(e, SemanticErrorKind::InvalidArgumentCount);
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
        let mut tag = None;
        for &argument in arguments {
            let info = self.expression_info(argument);
            let ty = self.converted(info);
            if self.types.unanalyzed(ty) {
                return Self::expression_result(e, self.types.unknown());
            }
            let mut literal = argument;
            while let E::Parenthesized { expression } = literal.kind {
                literal = expression;
            }
            if (operation && !self.arithmetic(ty))
                || tag_type.is_some_and(|target| !self.assignment_compatible(target, info))
            {
                return self.invalid_expression(e, SemanticErrorKind::InvalidArgumentType);
            }
            if operation {
                constant &= info.constant == ConstantClass::Arithmetic;
                self.convert(argument, result_type, super::ConversionKind::Assignment);
            }
            if let Some(target) = tag_type {
                constant &= matches!(literal.kind, E::StringLiteral(_));
                if let E::StringLiteral(
                    crate::translation_phases::preprocessing::StringTokenType::String(id),
                ) = literal.kind
                {
                    tag = self.context.literal_text_in(self.scratch, id, false);
                }
                self.convert(argument, target, super::ConversionKind::Assignment);
            }
        }
        let mut result = Self::expression_result(e, result_type);
        if constant {
            result.constant = ConstantClass::Arithmetic;
            if self.binary128_type(result_type) {
                // GCC folds these, and `__builtin_nanf128` takes the tag's
                // payload as `nanf128` does (C99 §7.12.11.2p2, p. 237;
                // PDF p. 249). `__builtin_nansl` gives a binary128 NaN
                // where `long double` is binary128 (WebAssembly).
                let payload = super::builtins::nan_payload_wide(tag.unwrap_or("").as_bytes());
                let operand = |index: usize| {
                    arguments
                        .get(index)
                        .and_then(|&argument| Self::binary128_value(self.expression_info(argument)))
                        .map(|value| value.real)
                };
                let folded = match (nan, operation, count) {
                    | (true, ..) => Some(Binary128::nan(quiet, payload)),
                    | (_, true, 1) => operand(0).map(|value| value.with_sign(false)),
                    | (_, true, _) => operand(0)
                        .zip(operand(1))
                        .map(|(value, sign)| value.with_sign(sign.is_negative())),
                    | _ => Some(Binary128::infinity(false)),
                }
                .map(Binary128Value::real);
                result.set_binary128(folded);
                self.mark_unfolded_binary128(&mut result);
            } else if nan {
                // GCC's `__builtin_nans` family: a signaling NaN of the
                // result's format whose payload the tag names.
                let payload = super::builtins::nan_payload(tag.unwrap_or("").as_bytes());
                let precision = self.floating_precision(scalar).unwrap_or(3);
                result.set_floating(Some(Floating::real(LongDouble::signaling_nan(
                    payload, precision,
                ))));
            } else {
                result.set_floating(Some(Floating::real(LongDouble::from_double(f64::INFINITY))));
            }
        }
        result
    }

    /// A callee bound to GNU `__builtin_classify_type`, through selections.
    pub(super) fn classify_type_callee(&self, function: &'tu Expression<'tu>) -> bool {
        let function = self.selected_designator(function);
        self.expression_info(function)
            .binding
            .and_then(|id| self.bindings[id].origin.descriptor())
            .is_some_and(|descriptor| {
                descriptor.type_rule == crate::builtins::TypeRule::TypeClassify
            })
    }

    /// Whether `ty` computes in IEEE binary128: `__float128`, and `long
    /// double` where the target makes it binary128 (WebAssembly).
    /// C99: §6.2.5 paragraph 10, p. 34; PDF p. 46.
    pub(super) fn binary128_type(&self, ty: TypeId) -> bool {
        match self.types.nodes[ty.index] {
            | TypeKind::Scalar(Scalar::Float128 | Scalar::ComplexFloat128) => true,
            | TypeKind::Scalar(Scalar::LongDouble | Scalar::ComplexLongDouble) =>
                self.types.target.long_double_is_binary128(),
            | _ => false,
        }
    }

    /// GNU real/imag preserves a complex component's lvalue and qualifiers;
    /// on a real operand the imaginary part is a zero rvalue.
    /// GNU extension: GCC manual, "Complex".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Complex.html>
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
            | TypeKind::Unknown | TypeKind::Unmodeled(_) =>
                return Self::expression_result(e, operand.ty),
            | _ => return self.invalid_expression(e, SemanticErrorKind::InvalidUnaryOperand),
        };
        if scalar == Scalar::Void {
            return self.invalid_expression(e, SemanticErrorKind::InvalidUnaryOperand);
        }
        let component = scalar.real_type();
        let complex = component != scalar;
        let ty = if complex {
            self.types
                .scalar(component)
                .qualified(operand.ty.qualifiers)
        } else {
            operand.ty
        };
        let mut result = Self::expression_result(e, ty);
        result.folding = operand.folding;
        if complex || op == UnaryOperator::Real {
            result.category = operand.category;
            result.register = operand.register;
            result.bit_field = operand.bit_field;
            result.promotion_width = operand.promotion_width;
            result.binding = operand.binding;
            // GNU component designations preserve C99 §6.6p9 address
            // eligibility.
            result.static_address = operand.static_address;
            result.vector_element = operand.vector_element;
        }
        if scalar.integer() {
            result.integer = if op == UnaryOperator::Imag {
                operand.integer.map(|_| Integer::int(0))
            } else {
                operand.integer
            };
            // A known zero still has its operand's constant restrictions.
            // C99 §6.6p6,8: ICE/arithmetic operands cannot include calls.
            result.ice = operand.ice;
            result.constant = operand.constant;
        } else {
            result.unfolded_binary128 = operand.unfolded_binary128;
            result.set_floating(operand.floating().map(|value| {
                Floating::real(if op == UnaryOperator::Real {
                    value.real
                } else {
                    value.imag
                })
            }));
            result.set_binary128(operand.binary128().map(|value| {
                Binary128Value::real(if op == UnaryOperator::Real {
                    value.real
                } else {
                    value.imag
                })
            }));
            if !complex && op == UnaryOperator::Imag {
                result.category = ValueCategory::Rvalue;
                result.constant = operand.constant;
                if operand.floating().is_some() {
                    result.set_floating(Some(Floating::real(LongDouble::ZERO)));
                }
                if component == Scalar::Float128 && operand.constant == ConstantClass::Arithmetic {
                    result.set_binary128(Some(Binary128Value::real(Binary128::zero(false))));
                }
            } else {
                result.constant = operand.constant;
            }
            self.mark_unfolded_binary128(&mut result);
        }
        result
    }

    /// GCC Other Builtins: expression arguments undergo default argument
    /// conversions for classification, without evaluating their value.
    /// GNU extension: GCC manual, "Other Builtins".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
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
                | TypeKind::Scalar(s) if s.complex() => 9,
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

/// GNU type compatibility and constant selection keep their own
/// unevaluated-operand constant rules.
fn keyword_selection(keyword: KeywordTokenType) -> bool {
    use crate::builtins::TypeRule as R;
    super::builtins::registry::keyword_descriptor(keyword).is_some_and(|descriptor| {
        matches!(descriptor.type_rule, R::TypesCompatible | R::ChooseExpr)
    })
}
