//! Constant expressions in static initializers, worked out while the
//! program is translated: `static int mask = 1 << 3;`. An object with
//! static storage duration has its value before `main` runs, so each of its
//! initializers must be a constant expression (C99 §6.7.8p4). For an
//! arithmetic object that is an arithmetic constant expression: constants
//! and `sizeof` joined by operators, with casts between arithmetic types
//! only, which reads no object and calls nothing (§6.6p3, p8). Its value is
//! computed by the same rules as when the program runs (§6.6p11), and must
//! fit its type (§6.6p4): a result that would be undefined at run time, such
//! as a signed overflow or a division by zero, is no constant, and the
//! program is refused.
//!
//! C99: §6.6 paragraphs 2-4, p. 95; PDF p. 107.
//! C99: §6.6 paragraph 8, p. 96; PDF p. 108.
//! C99: §6.6 paragraph 11, p. 96; PDF p. 108.
//! C99: §6.7.8 paragraph 4, p. 125; PDF p. 137.

use std::cmp::Ordering;

use super::{
    clauses,
    expressions::ExprKind,
    integers::Integer,
    model::{
        Arithmetic,
        Diagnostic,
        SourceRange,
        Type,
        Value,
    },
    parser::Parser,
};

impl Parser<'_> {
    /// Whether `expression` is a null pointer constant: an integer constant
    /// expression with the value 0, such as `0`, `'\0'` or `(1 - 1)`, or
    /// such an expression cast to `void *`. Assigned to, compared with or
    /// converted to a pointer, it becomes a null pointer.
    ///
    /// C99: §6.3.2.3 paragraph 3, p. 47; PDF p. 59.
    pub(super) fn null_pointer_constant(&self, expression: usize) -> bool {
        match self.expressions[expression].kind {
            | ExprKind::Cast(Type::VoidPointer, child) => self.zero_integer_constant(child),
            | _ => self.zero_integer_constant(expression),
        }
    }

    /// Whether `expression` is an integer constant expression whose value
    /// is 0.
    fn zero_integer_constant(&self, expression: usize) -> bool {
        self.integer_constant_expression(expression)
            && matches!(
                self.constant_value(expression),
                Ok((Value::Integer(0), Arithmetic::Integer(_)))
            )
    }

    /// Whether `expression` is an integer constant expression: integer and
    /// character constants and `sizeof`, joined by operators, with a
    /// floating constant only as the immediate operand of a cast to an
    /// integer type. Whether it has a value, without a signed overflow or
    /// a division by zero, is `constant_value`'s question.
    ///
    /// C99: §6.6 paragraph 6, p. 95; PDF p. 107.
    fn integer_constant_expression(&self, expression: usize) -> bool {
        let node = &self.expressions[expression];
        node.ty.integer().is_some()
            && match node.kind {
                | ExprKind::Number(..) | ExprKind::Character(..) | ExprKind::Sizeof(..) => true,
                | ExprKind::Cast(_, child) =>
                    matches!(self.expressions[child].kind, ExprKind::Real(..))
                        || self.integer_constant_expression(child),
                | ExprKind::Unary(_, child) => self.integer_constant_expression(child),
                | ExprKind::Binary(_, left, right) =>
                    self.integer_constant_expression(left)
                        && self.integer_constant_expression(right),
                | ExprKind::Conditional(condition, then, otherwise) =>
                    self.integer_constant_expression(condition)
                        && self.integer_constant_expression(then)
                        && self.integer_constant_expression(otherwise),
                | _ => false,
            }
    }

    /// The value and type of the arithmetic constant expression
    /// `expression`, or why it is not one.
    pub(super) fn constant_value(
        &self,
        expression: usize,
    ) -> Result<(Value, Arithmetic), Diagnostic> {
        let node = &self.expressions[expression];
        let range = node.range;
        let ty = node.ty;
        match &node.kind {
            | &ExprKind::Number(value, integer, _) =>
                Ok((Value::Integer(value), Arithmetic::Integer(integer))),
            | ExprKind::Real(rounded, _) => Ok((
                Value::Floating(rounded.value),
                Arithmetic::Floating(rounded.value.format()),
            )),
            | &ExprKind::Character(value, _) => Ok((Value::Integer(value), Arithmetic::INT)),
            | &ExprKind::Sizeof(size, _) => Ok((
                Value::Integer(i128::try_from(size).unwrap_or(i128::MAX)),
                Arithmetic::Integer(Integer::UnsignedLong),
            )),
            | &ExprKind::Cast(to, child) => {
                let (value, _) = self.constant_value(child)?;
                let Some(to) = to.arithmetic() else {
                    return Err(not_constant(range, "a cast to a pointer type"));
                };
                Ok((convert(&value, to, range)?, to))
            },
            | &ExprKind::Unary(operator, child) => {
                let (value, from) = self.constant_value(child)?;
                let to = arithmetic(ty, range)?;
                let promoted = convert(&value, from.promoted(), range)?;
                let result = match (operator, &promoted) {
                    | ("+", _) => promoted,
                    | ("-", &Value::Floating(real)) => Value::Floating(real.negate()),
                    | ("-", &Value::Integer(integer)) => integer_result(to, -integer, range, "-")?,
                    | ("~", &Value::Integer(integer)) =>
                        Value::Integer(integer_type(to).convert(!integer)),
                    | ("!", value) => Value::Integer(i128::from(is_zero(value))),
                    | _ => return Err(not_constant(range, "this operator")),
                };
                Ok((result, to))
            },
            | &ExprKind::Binary(operator @ ("&&" | "||"), left, right) => {
                let (left, _) = self.constant_value(left)?;
                let decided = is_zero(&left) == (operator == "&&");
                let truth = if decided {
                    operator == "||"
                } else {
                    let (right, _) = self.constant_value(right)?;
                    !is_zero(&right)
                };
                Ok((Value::Integer(i128::from(truth)), Arithmetic::INT))
            },
            | &ExprKind::Binary(operator, left, right) => {
                let (left_value, left_type) = self.constant_value(left)?;
                let (right_value, right_type) = self.constant_value(right)?;
                let to = arithmetic(ty, range)?;
                // A shift computes in its promoted left operand's type; the
                // other operators in the operands' common type (C99
                // §6.5.7p3, §6.3.1.8p1).
                let common = if matches!(operator, "<<" | ">>") {
                    left_type.promoted()
                } else {
                    left_type.promoted().common(right_type.promoted())
                };
                let left = convert(&left_value, common, range)?;
                let right = if matches!(operator, "<<" | ">>") {
                    convert(&right_value, right_type.promoted(), range)?
                } else {
                    convert(&right_value, common, range)?
                };
                Ok((binary(operator, common, &left, &right, range)?, to))
            },
            | &ExprKind::Conditional(condition, then, otherwise) => {
                let (test, _) = self.constant_value(condition)?;
                let chosen = if is_zero(&test) { otherwise } else { then };
                let (value, _) = self.constant_value(chosen)?;
                let to = arithmetic(ty, range)?;
                Ok((convert(&value, to, range)?, to))
            },
            | ExprKind::Name(name, ..) => Err(not_constant(
                range,
                &format!(
                    "a read of the object `{name}`: C99's constant expressions read no object, \
                     not even a const one (§6.6p8); GCC accepts a const one as an extension, \
                     which §6.6p10 allows, but this machine keeps to C99's rule"
                ),
            )),
            | ExprKind::Call(..) | ExprKind::CallPointer(..) | ExprKind::Library(..) =>
                Err(not_constant(
                    range,
                    "a function call, which only the running program makes (§6.6p3)",
                )),
            | ExprKind::Assign { .. } | ExprKind::Update { .. } => Err(not_constant(
                range,
                "an assignment, ++ or --, which store into an object (§6.6p3)",
            )),
            | ExprKind::Comma(..) => Err(not_constant(range, "the comma operator (§6.6p3)")),
            | ExprKind::Index(..) | ExprKind::Member { .. } => Err(not_constant(
                range,
                "an access to an object's element or member (§6.6p8)",
            )),
            | ExprKind::Function(_) => Err(not_constant(
                range,
                "a pointer to a function, where a number is needed",
            )),
        }
    }
}

/// The arithmetic type `ty`, which every operand of an arithmetic constant
/// expression has.
fn arithmetic(ty: Type, range: SourceRange) -> Result<Arithmetic, Diagnostic> {
    ty.arithmetic()
        .ok_or_else(|| not_constant(range, "a value that is not a number"))
}

const fn integer_type(ty: Arithmetic) -> Integer {
    match ty {
        | Arithmetic::Integer(integer) => integer,
        | Arithmetic::Floating(_) => Integer::Int,
    }
}

/// `value` converted to `to` as the running program would convert it.
fn convert(value: &Value, to: Arithmetic, range: SourceRange) -> Result<Value, Diagnostic> {
    to.convert(value).map_err(|_| {
        out_of_range(
            range,
            &format!(
                "the floating value's integral part does not fit in {}",
                super::memory::article(to.spelling())
            ),
        )
    })
}

/// Whether `value` compares equal to 0.
fn is_zero(value: &Value) -> bool {
    match value {
        | Value::Integer(integer) => *integer == 0,
        | Value::Floating(real) => real.is_zero(),
        | _ => false,
    }
}

/// The integer `exact`, the exact result of `operator`, in `ty`: reduced
/// modulo 2^N for an unsigned type, refused for a signed one it does not
/// fit (C99 §6.2.5p9, §6.6p4).
fn integer_result(
    ty: Arithmetic,
    exact: i128,
    range: SourceRange,
    operator: &str,
) -> Result<Value, Diagnostic> {
    let ty = integer_type(ty);
    if !ty.signed() {
        return Ok(Value::Integer(ty.convert(exact)));
    }
    if ty.fits(exact) {
        Ok(Value::Integer(exact))
    } else {
        Err(out_of_range(
            range,
            &format!(
                "`{operator}` gives {exact}, which does not fit in {}: a signed overflow, which \
                 would be undefined while the program runs",
                super::memory::article(ty.spelling())
            ),
        ))
    }
}

/// `left operator right`, both already converted to `ty`.
fn binary(
    operator: &str,
    ty: Arithmetic,
    left: &Value,
    right: &Value,
    range: SourceRange,
) -> Result<Value, Diagnostic> {
    let truth = |holds: bool| Ok(Value::Integer(i128::from(holds)));
    match (left, right) {
        | (&Value::Floating(left), &Value::Floating(right)) => {
            let order = left.compare(right);
            match operator {
                | "==" => truth(order == Some(Ordering::Equal)),
                | "!=" => truth(order != Some(Ordering::Equal)),
                | "<" => truth(order == Some(Ordering::Less)),
                | "<=" => truth(matches!(order, Some(Ordering::Less | Ordering::Equal))),
                | ">" => truth(order == Some(Ordering::Greater)),
                | ">=" => truth(matches!(order, Some(Ordering::Greater | Ordering::Equal))),
                | "+" | "-" | "*" | "/" =>
                    Ok(Value::Floating(left.arithmetic(operator, right).value)),
                | _ => Err(not_constant(range, "this operator on floating values")),
            }
        },
        | (&Value::Integer(left), &Value::Integer(right)) => {
            let integer = integer_type(ty);
            match operator {
                | "==" => truth(left == right),
                | "!=" => truth(left != right),
                | "<" => truth(left < right),
                | "<=" => truth(left <= right),
                | ">" => truth(left > right),
                | ">=" => truth(left >= right),
                | "&" => Ok(Value::Integer(left & right)),
                | "^" => Ok(Value::Integer(left ^ right)),
                | "|" => Ok(Value::Integer(left | right)),
                | "/" | "%" if right == 0 => Err(out_of_range(
                    range,
                    "it divides by zero, which has no value",
                )),
                | "<<" | ">>" => {
                    let bits = integer.bits();
                    let Some(count) = u32::try_from(right).ok().filter(|&count| count < bits)
                    else {
                        return Err(out_of_range(
                            range,
                            &format!(
                                "it shifts {} by {right}, outside 0 to {} (§6.5.7p3)",
                                super::memory::article(integer.spelling()),
                                bits - 1
                            ),
                        ));
                    };
                    if operator == ">>" {
                        return Ok(Value::Integer(left >> count));
                    }
                    if integer.signed() && left < 0 {
                        return Err(out_of_range(
                            range,
                            "it shifts a negative value left, which has no value (§6.5.7p4)",
                        ));
                    }
                    integer_result(ty, left << count, range, operator)
                },
                | "+" => integer_result(ty, left + right, range, operator),
                | "-" => integer_result(ty, left - right, range, operator),
                | "*" => left.checked_mul(right).map_or_else(
                    || {
                        let product = left.unsigned_abs().wrapping_mul(right.unsigned_abs());
                        if integer.signed() {
                            Err(out_of_range(
                                range,
                                "`*` gives a product too large for its type",
                            ))
                        } else {
                            Ok(Value::Integer(
                                i128::try_from(product % (1_u128 << integer.bits()))
                                    .unwrap_or_default(),
                            ))
                        }
                    },
                    |exact| integer_result(ty, exact, range, operator),
                ),
                | "/" => integer_result(ty, left / right, range, operator),
                | "%" if integer.signed() && !integer.fits(left / right) =>
                    Err(out_of_range(range, "the quotient does not fit its type")),
                | "%" => Ok(Value::Integer(left % right)),
                | _ => Err(not_constant(range, "this operator")),
            }
        },
        | _ => Err(not_constant(range, "these operands")),
    }
}

/// A static initializer that is no arithmetic constant expression because
/// of `what`.
fn not_constant(range: SourceRange, what: &str) -> Diagnostic {
    Diagnostic::constraint(
        range,
        &format!(
            "a static object's initializer must be a constant expression, worked out before the \
             program runs (C99 §6.7.8p4), and this one has {what}"
        ),
        &clauses::STATIC_INITIALIZER,
    )
}

/// A constant expression whose value its type cannot represent.
fn out_of_range(range: SourceRange, why: &str) -> Diagnostic {
    Diagnostic::constraint(
        range,
        &format!(
            "this static initializer is no constant: {why}, and a constant expression must \
             evaluate to a value its type can represent (C99 §6.6p4)"
        ),
        &clauses::CONSTANT_IN_RANGE,
    )
}
