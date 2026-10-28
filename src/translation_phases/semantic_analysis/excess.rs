//! GCC's classes of `_Float16`, `__bf16` and `_Complex _Float16`
//! expressions, which decide where it rounds a conversion between the two
//! half types. GCC evaluates both in `float` and converts a plain value of
//! one to the other as its exact `float` value; it rounds an excess-precision
//! expression's value at a cast, and it folds a cast of a literal at parse
//! time, rounding that conversion where its code would not. The GCC identity
//! folds (`expressions/typing.rs`) and lowers (`lowering/half.rs`) by these
//! classes; Clang rounds every such conversion.
//!
//! C99: §5.2.4.2.2 paragraph 7, p. 24; PDF p. 36 (evaluation formats);
//! §6.3.1.5 paragraph 2, p. 44; PDF p. 56. GCC: `EXCESS_PRECISION_EXPR` and
//! `c_parser_cast_expression`.

use super::ExpressionInfo;
use crate::translation_phases::{
    parsing::syntax::{
        BinaryOperator,
        Constant,
        Expression,
        ExpressionType as E,
        UnaryOperator as U,
    },
    preprocessing::{
        FloatTokenType,
        KeywordTokenType,
    },
};

/// Whether GCC evaluates `expression` with excess precision: a `+`, `-`,
/// `*` or `/` operation or an `f16` or `bf16` constant, through
/// parentheses, `__extension__`, unary `+` and `-` (which are exact and
/// keep their operand's class), the comma operator's right operand, the
/// association `_Generic` selects, the operand `__builtin_choose_expr`
/// selects and either arm of `?:`. Any other half expression is plain: a
/// cast, a variable's value, or a statement expression, whose value GCC
/// rounds. `info` gives an expression's semantic facts.
pub(crate) fn excess_expression<'tu>(
    expression: &'tu Expression<'tu>,
    info: &dyn Fn(&'tu Expression<'tu>) -> Option<ExpressionInfo<'tu>>,
) -> bool {
    let mut expression = expression;
    loop {
        match expression.kind {
            | E::Parenthesized { expression: inner }
            | E::Unary {
                operator: U::Extension | U::Plus | U::Minus,
                operand_expression: inner,
            }
            | E::Binary {
                operator: BinaryOperator::Comma,
                right_expression: inner,
                ..
            } => expression = inner,
            | E::Generic(_) => match info(expression).and_then(|info| info.selected_expression) {
                | Some(selected) => expression = selected,
                | None => return false,
            },
            | E::Builtin(builtin) if builtin.keyword == KeywordTokenType::BuiltinChooseExpr =>
                match info(expression).and_then(|info| info.selected_expression) {
                    | Some(selected) => expression = selected,
                    | None => return false,
                },
            | E::Binary { operator, .. } =>
                return matches!(
                    operator,
                    BinaryOperator::Addition
                        | BinaryOperator::Subtraction
                        | BinaryOperator::Multiplication
                        | BinaryOperator::Division
                ),
            | E::Constant(Constant::Float(
                FloatTokenType::Float16(_) | FloatTokenType::BFloat16(_),
            )) => return true,
            | E::Conditional(conditional) =>
                return excess_expression(conditional.then_expression, info)
                    || excess_expression(conditional.else_expression, info),
            | _ => return false,
        }
    }
}

/// Whether GCC folds `expression`, a half value, to a constant as it parses
/// it: a cast of a floating literal, of an integer constant expression, of
/// a `?:` between such operands or of such a cast, or `__builtin_complex`
/// of such casts, through parentheses, unary `+`, `__extension__` and the
/// selections of `_Generic` and `__builtin_choose_expr`. A cast of a negated or
/// computed floating operand, such as `(__bf16)-0x1p20`, is folded only later,
/// and GCC converts it as a plain run-time value.
pub(crate) fn folded_constant<'tu>(
    expression: &'tu Expression<'tu>,
    info: &dyn Fn(&'tu Expression<'tu>) -> Option<ExpressionInfo<'tu>>,
) -> bool {
    match strip(expression, info).kind {
        | E::Cast {
            operand_expression, ..
        } => literal(operand_expression, info),
        // A builtin call that folds, `__builtin_complex`, folds as its
        // operands do.
        | E::Call { arguments, .. } =>
            info(expression).is_some_and(|info| info.floating().is_some())
                && arguments
                    .iter()
                    .all(|&argument| folded_constant(argument, info)),
        | _ => false,
    }
}

/// Whether GCC's parser holds `expression` as a literal constant.
fn literal<'tu>(
    expression: &'tu Expression<'tu>,
    info: &dyn Fn(&'tu Expression<'tu>) -> Option<ExpressionInfo<'tu>>,
) -> bool {
    let expression = strip(expression, info);
    match expression.kind {
        | E::Constant(_) => true,
        // A cast of a folded constant folds too.
        | E::Cast { .. } => folded_constant(expression, info),
        | E::Conditional(conditional) =>
            literal(conditional.then_expression, info) && literal(conditional.else_expression, info),
        // An integer constant expression folds as GCC parses it.
        | _ => info(expression).is_some_and(|info| info.ice && info.integer.is_some()),
    }
}

/// `expression` through parentheses, unary `+`, `__extension__` and the
/// operands `_Generic` and `__builtin_choose_expr` select, which GCC's
/// parser replaces by what they hold.
fn strip<'tu>(
    expression: &'tu Expression<'tu>,
    info: &dyn Fn(&'tu Expression<'tu>) -> Option<ExpressionInfo<'tu>>,
) -> &'tu Expression<'tu> {
    let mut expression = expression;
    loop {
        match expression.kind {
            | E::Parenthesized { expression: inner }
            | E::Unary {
                operator: U::Plus | U::Extension,
                operand_expression: inner,
            } => expression = inner,
            | E::Generic(_) => match info(expression).and_then(|info| info.selected_expression) {
                | Some(selected) => expression = selected,
                | None => return expression,
            },
            | E::Builtin(builtin) if builtin.keyword == KeywordTokenType::BuiltinChooseExpr =>
                match info(expression).and_then(|info| info.selected_expression) {
                    | Some(selected) => expression = selected,
                    | None => return expression,
                },
            | _ => return expression,
        }
    }
}
