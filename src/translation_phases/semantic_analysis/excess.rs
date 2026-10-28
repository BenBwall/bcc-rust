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

use super::{
    Analyzer,
    ExpressionInfo,
    Scalar,
    TypeId,
};
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
                FloatTokenType::Float16(_)
                | FloatTokenType::BFloat16(_)
                | FloatTokenType::ImaginaryFloat16(_),
            )) => return true,
            | E::Conditional(conditional) =>
                return excess_expression(conditional.then_expression, info)
                    || excess_expression(conditional.else_expression, info),
            | _ => return false,
        }
    }
}

/// Whether GCC folds `expression`, a half value, to a constant as it parses
/// it, so that its conversion to the other half type rounds. GCC's parser
/// folds a cast exactly when its operand is already a constant node: a
/// literal, an integer constant expression, a cast of a constant node, a
/// `?:` with a constant condition whose selected operand is one,
/// `__builtin_complex` of constant nodes, or an operation on constant nodes
/// narrowed to a standard floating type (`(float)(0x1p19 + 0x1p19)`), which
/// GCC shortens and folds. A constant condition may be integer or floating,
/// and a comma after a constant is transparent. It leaves a
/// negated or computed floating operand, as in `(__bf16)-0x1p20`, to be folded
/// later and converts it as a plain run-time value. Parentheses, unary `+`,
/// `__extension__` and the selections of `_Generic` and
/// `__builtin_choose_expr` are transparent. `scalar` gives the scalar type a
/// type names.
/// GCC: `build_c_cast` and `convert_to_real`.
pub(crate) fn folded_constant<'tu>(
    expression: &'tu Expression<'tu>,
    info: &dyn Fn(&'tu Expression<'tu>) -> Option<ExpressionInfo<'tu>>,
    scalar: &dyn Fn(TypeId) -> Option<Scalar>,
) -> bool {
    let expression = strip(expression, info);
    match expression.kind {
        | E::Constant(_) => true,
        | E::Cast {
            operand_expression, ..
        } =>
            folded_constant(operand_expression, info, scalar)
                || converted_arithmetic(expression, operand_expression, info, scalar),
        // A constant condition, integer or floating, selects its operand
        // as GCC parses it: the truth-value conversion folds it.
        | E::Conditional(conditional) => {
            let condition = conditional.condition_expression;
            match info(condition)
                .filter(|_| folded_constant(condition, info, scalar))
                .and_then(Analyzer::constant_truth)
            {
                | Some(truth) => folded_constant(
                    if truth {
                        conditional.then_expression
                    } else {
                        conditional.else_expression
                    },
                    info,
                    scalar,
                ),
                | None => false,
            }
        },
        // A builtin call that folds, `__builtin_complex`, folds as its
        // operands do.
        | E::Call { arguments, .. } =>
            info(expression).is_some_and(|info| info.floating().is_some())
                && arguments
                    .iter()
                    .all(|&argument| folded_constant(argument, info, scalar)),
        // An integer constant expression folds as GCC parses it.
        | _ => info(expression).is_some_and(|info| info.ice && info.integer.is_some()),
    }
}

/// Whether `cast` narrows an operation on two constant nodes to a standard
/// floating type of less precision, which GCC's shortening of a narrowed
/// operation folds as it parses it; a widening conversion, or an operation
/// on computed operands, it leaves. (GCC shortens only when both constants
/// are exact in the narrower type; that condition is not modelled.)
fn converted_arithmetic<'tu>(
    cast: &'tu Expression<'tu>,
    operand: &'tu Expression<'tu>,
    info: &dyn Fn(&'tu Expression<'tu>) -> Option<ExpressionInfo<'tu>>,
    scalar: &dyn Fn(TypeId) -> Option<Scalar>,
) -> bool {
    let operand = strip(operand, info);
    // The standard floating types' precisions on x86-64.
    let precision = |s: Scalar| match s {
        | Scalar::Float => Some(24),
        | Scalar::Double => Some(53),
        | Scalar::LongDouble => Some(64),
        | Scalar::Float128 => Some(113),
        | _ => None,
    };
    // The shortening needs both operands to be constants already.
    let arithmetic = match operand.kind {
        | E::Binary {
            operator:
                BinaryOperator::Addition
                | BinaryOperator::Subtraction
                | BinaryOperator::Multiplication
                | BinaryOperator::Division,
            left_expression,
            right_expression,
        } =>
            folded_constant(left_expression, info, scalar)
                && folded_constant(right_expression, info, scalar),
        | _ => false,
    };
    if let (true, Some(cast), Some(operand)) = (arithmetic, info(cast), info(operand))
        && (operand.floating().is_some() || operand.binary128().is_some())
        && let (Some(to), Some(from)) = (
            scalar(cast.ty).and_then(precision),
            scalar(operand.ty).and_then(precision),
        )
    {
        to < from
    } else {
        false
    }
}

/// `expression` through parentheses, unary `+`, `__extension__`, the
/// operands `_Generic` and `__builtin_choose_expr` select and a comma's right
/// operand after a constant, which GCC's parser replaces by what they hold.
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
            // GCC drops a comma's left operand without side effects, here a
            // constant, and keeps the right one.
            | E::Binary {
                operator: BinaryOperator::Comma,
                left_expression,
                right_expression,
            } if info(left_expression).is_some_and(|left| {
                left.integer.is_some() || left.floating().is_some() || left.binary128().is_some()
            }) =>
                expression = right_expression,
            | _ => return expression,
        }
    }
}
