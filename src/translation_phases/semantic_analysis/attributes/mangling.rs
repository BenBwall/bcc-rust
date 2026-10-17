//! Itanium C overload names used by Clang's `overloadable`/`enable_if`
//! extensions. C99: §6.2.2p2, p. 30; PDF p. 42 (entity identity, extended
//! here). <https://clang.llvm.org/docs/AttributeReference.html#overloadable>
//! <https://itanium-cxx-abi.github.io/cxx-abi/abi.html#mangling>

use super::super::{
    Analyzer,
    ArrayBound,
    BinaryOperator,
    Expression,
    ExpressionType,
    FunctionExtension,
    PredicateKind,
    Scalar,
    TypeId,
    TypeKind,
    UnaryOperator,
};
use crate::{
    translation_phases::parsing::declaration_syntax::TypeQualifiers,
    util::{
        bump::{
            ArenaString,
            ArenaVec,
        },
        string_cache::StringCacheId,
    },
};

enum Part<'tu> {
    Text(&'static str),
    Type(TypeId),
    Register(TypeId),
    Argument(&'tu Expression<'tu>),
    Expression(&'tu Expression<'tu>),
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    pub(super) fn mangle_function(
        &mut self,
        name: StringCacheId,
        ty: TypeId,
        extension: FunctionExtension<'tu>,
        internal: bool,
    ) -> StringCacheId {
        let mut output = ArenaString::new_in(self.scratch);
        output.push_str(if internal { "_ZL" } else { "_Z" });
        let name = self.context.string_cache.at(name);
        output.push_str(crate::diagnostics::format_in!(
            self.scratch,
            "{}",
            name.len()
        ));
        output.push_str(name);
        let enabled = extension
            .predicates
            .iter()
            .filter(|p| p.kind == PredicateKind::Enable)
            .count();
        let mut parts = ArenaVec::new_in(self.scratch);
        let mut substitutions = ArenaVec::new_in(self.scratch);
        let mut valid = true;
        let TypeKind::Function {
            parameters,
            variadic,
            ..
        } = self.types.nodes[ty.index]
        else {
            return extension.identity;
        };
        if variadic {
            parts.push(Part::Text("z"));
        }
        if parameters.is_empty() && !variadic {
            parts.push(Part::Text("v"));
        }
        for (index, &parameter) in parameters.iter().enumerate().rev() {
            if let Some(size) = extension.sizes.iter().find(|s| s.index as usize == index) {
                // The vendor qualifier follows the written pointer type in
                // Clang's ABI, and does not encode the hidden size_t itself.
                parts.push(Part::Text(match (size.dynamic, size.mode) {
                    | (false, 0) => "U17pass_object_size0",
                    | (false, 1) => "U17pass_object_size1",
                    | (false, 2) => "U17pass_object_size2",
                    | (false, _) => "U17pass_object_size3",
                    | (true, 0) => "U25pass_dynamic_object_size0",
                    | (true, 1) => "U25pass_dynamic_object_size1",
                    | (true, 2) => "U25pass_dynamic_object_size2",
                    | (true, _) => "U25pass_dynamic_object_size3",
                }));
            }
            parts.push(Part::Type(parameter.unqualified()));
        }
        if enabled != 0 {
            parts.push(Part::Text("E"));
            for predicate in extension
                .predicates
                .iter()
                .rev()
                .filter(|p| p.kind == PredicateKind::Enable)
            {
                parts.push(Part::Argument(predicate.expression));
            }
            parts.push(Part::Text("Ua9enable_ifI"));
        }
        while let Some(part) = parts.pop() {
            match part {
                | Part::Text(text) => output.push_str(text),
                | Part::Register(ty) => substitutions.push(ty),
                | Part::Type(ty) => {
                    let ty = TypeId { alignment: 0, ..ty };
                    let substitute = !ty.qualifiers.is_empty()
                        || !matches!(self.types.nodes[ty.index], TypeKind::Scalar(_));
                    if substitute {
                        if let Some(index) =
                            substitutions.iter().position(|&previous| previous == ty)
                        {
                            output.push('S');
                            if index > 0 {
                                base36(&mut output, index - 1);
                            }
                            output.push('_');
                            continue;
                        }
                        parts.push(Part::Register(ty));
                    }
                    if !ty.qualifiers.is_empty() {
                        if ty.qualifiers.contains(TypeQualifiers::RESTRICT) {
                            output.push('r');
                        }
                        if ty.qualifiers.contains(TypeQualifiers::VOLATILE) {
                            output.push('V');
                        }
                        if ty.qualifiers.contains(TypeQualifiers::CONST) {
                            output.push('K');
                        }
                        parts.push(Part::Type(ty.unqualified()));
                        continue;
                    }
                    match self.types.nodes[ty.index] {
                        | TypeKind::Scalar(scalar) => output.push_str(scalar_code(scalar)),
                        | TypeKind::Pointer(inner) => {
                            output.push('P');
                            parts.push(Part::Type(inner));
                        },
                        | TypeKind::Array(inner, bound) => {
                            output.push('A');
                            if let ArrayBound::Constant(n) = bound {
                                output
                                    .push_str(crate::diagnostics::format_in!(self.scratch, "{n}"));
                            }
                            output.push('_');
                            parts.push(Part::Type(inner));
                        },
                        | TypeKind::Tag(tag) => {
                            let name = self.types.tags[tag]
                                .name
                                .map_or("$_anonymous", |name| self.context.string_cache.at(name));
                            output.push_str(crate::diagnostics::format_in!(
                                self.scratch,
                                "{}{}",
                                name.len(),
                                name
                            ));
                        },
                        | TypeKind::Function {
                            result,
                            parameters,
                            variadic,
                            ..
                        } => {
                            output.push('F');
                            parts.push(Part::Text("E"));
                            if variadic {
                                parts.push(Part::Text("z"));
                            }
                            if parameters.is_empty() && !variadic {
                                parts.push(Part::Text("v"));
                            }
                            for &p in parameters.iter().rev() {
                                parts.push(Part::Type(p));
                            }
                            parts.push(Part::Type(result));
                        },
                        | TypeKind::Nullptr => output.push_str("Dn"),
                        | TypeKind::Atomic(inner) => {
                            output.push_str("U7_Atomic");
                            parts.push(Part::Type(inner));
                        },
                        | TypeKind::Vector { element, count, .. } => {
                            output.push_str(crate::diagnostics::format_in!(
                                self.scratch,
                                "Dv{count}_"
                            ));
                            parts.push(Part::Type(element));
                        },
                        | TypeKind::Unknown | TypeKind::Unmodeled(_) => {
                            valid = false;
                        },
                    }
                },
                | Part::Argument(mut expression) => {
                    while let ExpressionType::Parenthesized { expression: inner } = expression.kind
                    {
                        expression = inner;
                    }
                    if !matches!(
                        expression.kind,
                        ExpressionType::Constant(_) | ExpressionType::Boolean(_)
                    ) {
                        output.push('X');
                        parts.push(Part::Text("E"));
                    }
                    parts.push(Part::Expression(expression));
                },
                | Part::Expression(expression) => {
                    let info = self.expression_info(expression);
                    if matches!(
                        expression.kind,
                        ExpressionType::Constant(_) | ExpressionType::Boolean(_)
                    ) && let Some(value) = info.integer
                    {
                        output.push('L');
                        output.push_str(match self.types.nodes[info.ty.index] {
                            | TypeKind::Scalar(s) => scalar_code(s),
                            | _ => "i",
                        });
                        if value.value < 0 {
                            output.push('n');
                        }
                        output.push_str(crate::diagnostics::format_in!(
                            self.scratch,
                            "{}",
                            value.value.unsigned_abs()
                        ));
                        output.push('E');
                        continue;
                    }
                    match expression.kind {
                        | ExpressionType::Parenthesized { expression } =>
                            parts.push(Part::Expression(expression)),
                        | ExpressionType::Identifier(identifier) => {
                            if let Some(value) = info.integer {
                                output.push('L');
                                output.push_str(match self.types.nodes[info.ty.index] {
                                    | TypeKind::Scalar(s) => scalar_code(s),
                                    | _ => "i",
                                });
                                if value.value < 0 {
                                    output.push('n');
                                }
                                output.push_str(crate::diagnostics::format_in!(
                                    self.scratch,
                                    "{}E",
                                    value.value.unsigned_abs()
                                ));
                                continue;
                            }
                            if let Some(index) = extension
                                .parameters
                                .iter()
                                .position(|&p| p.is_some() && p == info.binding)
                            {
                                output.push_str("fL0p");
                                if index > 0 {
                                    output.push_str(crate::diagnostics::format_in!(
                                        self.scratch,
                                        "{}",
                                        index - 1
                                    ));
                                }
                                output.push('_');
                            } else {
                                let name = self.context.string_cache.at(identifier.name);
                                output.push_str(crate::diagnostics::format_in!(
                                    self.scratch,
                                    "L_Z{}{}E",
                                    name.len(),
                                    name
                                ));
                            }
                        },
                        | ExpressionType::Binary {
                            operator,
                            left_expression,
                            right_expression,
                        } => {
                            let Some(code) = binary_code(operator) else {
                                valid = false;
                                continue;
                            };
                            output.push_str(code);
                            parts.push(Part::Expression(right_expression));
                            parts.push(Part::Expression(left_expression));
                        },
                        | ExpressionType::Unary {
                            operator,
                            operand_expression,
                        } => {
                            output.push_str(match operator {
                                | UnaryOperator::LogicalNot => "nt",
                                | UnaryOperator::BitwiseNot => "co",
                                | UnaryOperator::Minus => "ng",
                                | UnaryOperator::Plus => "ps",
                                | UnaryOperator::AddressOf => "ad",
                                | UnaryOperator::Indirection => "de",
                                | _ => {
                                    valid = false;
                                    "ps"
                                },
                            });
                            parts.push(Part::Expression(operand_expression));
                        },
                        | ExpressionType::Conditional(conditional) => {
                            output.push_str("qu");
                            parts.push(Part::Expression(conditional.else_expression));
                            parts.push(Part::Expression(conditional.then_expression));
                            parts.push(Part::Expression(conditional.condition_expression));
                        },
                        | ExpressionType::Call {
                            function_expression,
                            arguments,
                        } => {
                            output.push_str("cl");
                            parts.push(Part::Text("E"));
                            for &argument in arguments.iter().rev() {
                                parts.push(Part::Expression(argument));
                            }
                            parts.push(Part::Expression(function_expression));
                        },
                        | ExpressionType::Cast {
                            operand_expression, ..
                        } => {
                            output.push_str("cv");
                            parts.push(Part::Expression(operand_expression));
                            parts.push(Part::Type(info.ty));
                        },
                        | ExpressionType::SizeofExpr(_)
                        | ExpressionType::SizeofType(_)
                        | ExpressionType::AlignofExpr(_)
                        | ExpressionType::AlignofType(_)
                            if info.integer.is_some() =>
                        {
                            let value = info.integer.expect("checked folded size query");
                            output.push('L');
                            output.push_str(match self.types.nodes[info.ty.index] {
                                | TypeKind::Scalar(s) => scalar_code(s),
                                | _ => "m",
                            });
                            output.push_str(crate::diagnostics::format_in!(
                                self.scratch,
                                "{}E",
                                value.value
                            ));
                        },
                        | _ => {
                            valid = false;
                        },
                    }
                },
            }
        }
        if !valid
            && let Some(predicate) = extension
                .predicates
                .iter()
                .find(|p| p.kind == PredicateKind::Enable)
        {
            let name = self.context.string_cache.intern("enable_if");
            self.error(
                super::super::SemanticErrorKind::AttributeArguments,
                predicate.message.source,
                Some(name),
                None,
            );
        }
        self.context.string_cache.intern(output.as_str())
    }
}

fn scalar_code(scalar: Scalar) -> &'static str {
    match scalar {
        | Scalar::Void => "v",
        | Scalar::Bool => "b",
        | Scalar::Char => "c",
        | Scalar::SignedChar => "a",
        | Scalar::UnsignedChar => "h",
        | Scalar::Short => "s",
        | Scalar::UnsignedShort => "t",
        | Scalar::Int => "i",
        | Scalar::UnsignedInt => "j",
        | Scalar::Long => "l",
        | Scalar::UnsignedLong => "m",
        | Scalar::LongLong => "x",
        | Scalar::UnsignedLongLong => "y",
        | Scalar::Int128 => "n",
        | Scalar::UnsignedInt128 => "o",
        | Scalar::Float => "f",
        | Scalar::Double => "d",
        | Scalar::LongDouble => "e",
        | Scalar::Float128 => "g",
        | Scalar::ComplexFloat => "Cf",
        | Scalar::ComplexDouble => "Cd",
        | Scalar::ComplexLongDouble => "Ce",
        | Scalar::ComplexFloat128 => "Cg",
    }
}
fn binary_code(operator: BinaryOperator) -> Option<&'static str> {
    Some(match operator {
        | BinaryOperator::Addition => "pl",
        | BinaryOperator::Subtraction => "mi",
        | BinaryOperator::Multiplication => "ml",
        | BinaryOperator::Division => "dv",
        | BinaryOperator::Modulo => "rm",
        | BinaryOperator::LeftShift => "ls",
        | BinaryOperator::RightShift => "rs",
        | BinaryOperator::LessThan => "lt",
        | BinaryOperator::GreaterThan => "gt",
        | BinaryOperator::LessThanOrEqual => "le",
        | BinaryOperator::GreaterThanOrEqual => "ge",
        | BinaryOperator::Equal => "eq",
        | BinaryOperator::NotEqual => "ne",
        | BinaryOperator::BitwiseAnd => "an",
        | BinaryOperator::BitwiseXor => "eo",
        | BinaryOperator::BitwiseOr => "or",
        | BinaryOperator::LogicalAnd => "aa",
        | BinaryOperator::LogicalOr => "oo",
        | BinaryOperator::Comma => "cm",
        | _ => return None,
    })
}

fn base36(output: &mut ArenaString<'_>, mut value: usize) {
    let mut digits = [0_u8; 16];
    let mut used = 0;
    loop {
        digits[used] = u8::try_from(value % 36).expect("a base-36 digit fits in u8");
        used += 1;
        value /= 36;
        if value == 0 {
            break;
        }
    }
    for &digit in digits[..used].iter().rev() {
        output.push(char::from(if digit < 10 {
            b'0' + digit
        } else {
            b'A' + digit - 10
        }));
    }
}
