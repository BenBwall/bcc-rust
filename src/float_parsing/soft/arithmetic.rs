//! Exact arithmetic on unpacked values, rounded once, with the special
//! values and NaN choices of the unit that computes the format: the x87 FPU
//! for double-extended precision, SSE for binary32 and binary64.
//!
//! IEC 60559 arithmetic: C99 Annex F.3, pp. 445-446; PDF pp. 457-458.

use std::cmp::Ordering;

use super::value::{
    Format,
    QUIET,
    Soft,
    Value,
    round,
};

/// The four basic operations.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Operation {
    Add,
    Subtract,
    Multiply,
    Divide,
}

/// `left operation right`, rounded to `format`.
pub(super) fn apply(operation: Operation, left: Soft, right: Soft, format: Format) -> Soft {
    if let Some(nan) = propagate_nan(left, right, format) {
        return nan;
    }
    match operation {
        | Operation::Add => add(left, right, format),
        | Operation::Subtract => add(left, right.negated(), format),
        | Operation::Multiply => multiply(left, right, format),
        | Operation::Divide => divide(left, right, format),
    }
}

pub(super) fn add_of(left: Soft, right: Soft, format: Format) -> Soft {
    apply(Operation::Add, left, right, format)
}

pub(super) fn sub_of(left: Soft, right: Soft, format: Format) -> Soft {
    apply(Operation::Subtract, left, right, format)
}

pub(super) fn mul_of(left: Soft, right: Soft, format: Format) -> Soft {
    apply(Operation::Multiply, left, right, format)
}

pub(super) fn div_of(left: Soft, right: Soft, format: Format) -> Soft {
    apply(Operation::Divide, left, right, format)
}

/// The NaN result of an operation with a NaN operand. The x87 FPU returns
/// the quiet operand when only one is quiet, and otherwise the one with the
/// larger significand; SSE returns the first NaN operand. Either quiets it.
/// An x87 encoding the FPU refuses is an invalid operand.
/// Intel SDM vol. 1, §4.8.3.5 and table 4-7, and §8.5.1.2.
fn propagate_nan(left: Soft, right: Soft, format: Format) -> Option<Soft> {
    let quieted = |nan: Soft, fraction: u64| Soft {
        negative: nan.negative,
        value:    Value::Nan {
            fraction: fraction | QUIET,
        },
    };
    match (left.value, right.value) {
        | (Value::Unsupported { .. }, _) | (_, Value::Unsupported { .. }) =>
            Some(Soft::DEFAULT_NAN),
        | (Value::Nan { fraction: first }, Value::Nan { fraction: second }) if format.x87 => {
            let chosen = match ((first & QUIET != 0), (second & QUIET != 0)) {
                | (true, false) => (left, first),
                | (false, true) => (right, second),
                | _ => match (first & !QUIET).cmp(&(second & !QUIET)) {
                    | Ordering::Less => (right, second),
                    // Equal significands: the positive operand.
                    | Ordering::Equal if left.negative && !right.negative => (right, second),
                    | _ => (left, first),
                },
            };
            Some(quieted(chosen.0, chosen.1))
        },
        | (Value::Nan { fraction }, _) => Some(quieted(left, fraction)),
        | (_, Value::Nan { fraction }) => Some(quieted(right, fraction)),
        | _ => None,
    }
}

fn add(left: Soft, right: Soft, format: Format) -> Soft {
    match (left.value, right.value) {
        | (Value::Infinity, Value::Infinity) if left.negative != right.negative =>
            Soft::DEFAULT_NAN,
        | (Value::Zero, Value::Zero) => Soft::zero(left.negative && right.negative),
        // An infinity, or a nonzero value plus zero, is unchanged.
        | (Value::Infinity, _) | (_, Value::Zero) => left,
        | (_, Value::Infinity) | (Value::Zero, _) => right,
        | (
            Value::Finite {
                exponent: left_exponent,
                significand: left_significand,
            },
            Value::Finite {
                exponent: right_exponent,
                significand: right_significand,
            },
        ) => {
            // Order by magnitude, so the difference is never negative.
            let ((big, big_exponent, big_significand), (small, small_exponent, small_significand)) =
                if (left_exponent, left_significand) >= (right_exponent, right_significand) {
                    (
                        (left, left_exponent, left_significand),
                        (right, right_exponent, right_significand),
                    )
                } else {
                    (
                        (right, right_exponent, right_significand),
                        (left, left_exponent, left_significand),
                    )
                };
            // Two bits of headroom for a carry; 62 guard bits below.
            let wide = u128::from(big_significand) << 62;
            let distance = (big_exponent - small_exponent) as u32;
            let small_wide = u128::from(small_significand) << 62;
            let (aligned, sticky) = if distance >= 126 {
                (0, true)
            } else {
                (
                    small_wide >> distance,
                    small_wide & ((1_u128 << distance) - 1) != 0,
                )
            };
            let exponent = big_exponent - 63 - 62;
            if big.negative == small.negative {
                round(big.negative, wide + aligned, exponent, sticky, format)
            } else if wide == aligned && !sticky {
                // An exact zero difference is positive when rounding to
                // nearest (C99 Annex F.8.2, p. 460; PDF p. 472).
                Soft::zero(false)
            } else {
                // The true difference lies strictly between this one and
                // the next integer below it.
                let difference = wide - aligned - u128::from(sticky);
                round(big.negative, difference, exponent, sticky, format)
            }
        },
        | _ => unreachable!("NaN operands are handled before addition"),
    }
}

fn multiply(left: Soft, right: Soft, format: Format) -> Soft {
    let negative = left.negative != right.negative;
    match (left.value, right.value) {
        | (Value::Zero, Value::Infinity) | (Value::Infinity, Value::Zero) => Soft::DEFAULT_NAN,
        | (Value::Infinity, _) | (_, Value::Infinity) => Soft::infinity(negative),
        | (Value::Zero, _) | (_, Value::Zero) => Soft::zero(negative),
        | (
            Value::Finite {
                exponent: left_exponent,
                significand: left_significand,
            },
            Value::Finite {
                exponent: right_exponent,
                significand: right_significand,
            },
        ) => round(
            negative,
            u128::from(left_significand) * u128::from(right_significand),
            left_exponent + right_exponent - 126,
            false,
            format,
        ),
        | _ => unreachable!("NaN operands are handled before multiplication"),
    }
}

fn divide(left: Soft, right: Soft, format: Format) -> Soft {
    let negative = left.negative != right.negative;
    match (left.value, right.value) {
        | (Value::Zero, Value::Zero) | (Value::Infinity, Value::Infinity) => Soft::DEFAULT_NAN,
        | (Value::Infinity, _) | (_, Value::Zero) => Soft::infinity(negative),
        | (_, Value::Infinity) | (Value::Zero, _) => Soft::zero(negative),
        | (
            Value::Finite {
                exponent: left_exponent,
                significand: left_significand,
            },
            Value::Finite {
                exponent: right_exponent,
                significand: right_significand,
            },
        ) => {
            // 72 or 73 quotient bits, then the remainder as the sticky bit.
            let divisor = u128::from(right_significand);
            let dividend = u128::from(left_significand) << 64;
            let (high, remainder) = (dividend / divisor, dividend % divisor);
            let extended = remainder << 8;
            let quotient = (high << 8) | (extended / divisor);
            let sticky = extended % divisor != 0;
            round(
                negative,
                quotient,
                left_exponent - right_exponent - 72,
                sticky,
                format,
            )
        },
        | _ => unreachable!("NaN operands are handled before division"),
    }
}

/// The ordering of two values, or `None` when either is a NaN.
pub(super) fn compare(left: Soft, right: Soft) -> Option<Ordering> {
    if left.is_nan() || right.is_nan() {
        return None;
    }
    let key = |value: Soft| match value.value {
        | Value::Zero => (0, 0, 0),
        | Value::Finite {
            exponent,
            significand,
        } => (1, exponent, significand),
        | _ => (2, 0, 0),
    };
    let magnitude = key(left).cmp(&key(right));
    Some(match (left.negative, right.negative) {
        | _ if left.is_zero() && right.is_zero() => Ordering::Equal,
        | (false, false) => magnitude,
        | (true, true) => magnitude.reverse(),
        | (false, true) => Ordering::Greater,
        | (true, false) => Ordering::Less,
    })
}

/// `left < right`, false when either is a NaN.
pub(super) fn less(left: Soft, right: Soft) -> bool {
    compare(left, right) == Some(Ordering::Less)
}

/// A power of two.
pub(super) fn power_of_two(power: i32) -> Soft {
    Soft {
        negative: false,
        value:    Value::Finite {
            exponent:    power,
            significand: 1 << 63,
        },
    }
}

/// The greatest finite value of `format`.
pub(super) fn max_finite(format: Format) -> Soft {
    Soft {
        negative: false,
        value:    Value::Finite {
            exponent:    format.max_exponent,
            significand: u64::MAX << (64 - format.precision),
        },
    }
}
