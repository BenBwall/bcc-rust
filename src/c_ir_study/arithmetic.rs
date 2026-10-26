//! Target-width integer semantics, independent of LLVM and the old evaluator.
//! C99: §6.2.5p9, p. 34; PDF p. 46; §6.3.1.3, p. 43; PDF p. 55;
//! §6.5p5, p. 67; PDF p. 79; §6.5.5-6.5.7, pp. 82-84;
//! PDF pp. 94-96. Right shift of negative values and narrowing to signed
//! types explicitly select bcc's arithmetic-shift/two's-complement policy.

use super::model::{
    Binary,
    Failure,
    IntType,
    Unary,
    Undefined,
};
use crate::target::Scalar;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Value {
    pub(super) ty:   IntType,
    pub(super) bits: u64,
}

impl Value {
    pub(super) const fn new(ty: IntType, bits: u64) -> Self {
        Self {
            ty,
            bits: bits & ty.mask(),
        }
    }

    pub(super) fn number(self) -> i128 {
        let raw = i128::from(self.bits);
        if self.ty.signed && self.bits & (1 << (self.ty.bits - 1)) != 0 {
            raw - (1_i128 << self.ty.bits)
        } else {
            raw
        }
    }

    /// C99: §6.3.1.2-6.3.1.3, p. 43; PDF p. 55.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "Integer conversion selects the low target-width bits, including for negative \
                  values."
    )]
    pub(super) fn cast(self, ty: IntType) -> Self {
        Self::new(
            ty,
            if ty.scalar == Scalar::Bool {
                u64::from(self.bits != 0)
            } else {
                self.number() as u64
            },
        )
    }
}

pub(super) fn unary(op: Unary, value: Value, ty: IntType) -> Result<Value, Failure> {
    match op {
        | Unary::Not => Ok(Value::new(ty, u64::from(value.bits == 0))),
        | Unary::Invert => Ok(Value::new(ty, !value.cast(ty).bits)),
        | Unary::Negate => {
            let value = value.cast(ty);
            if ty.signed {
                checked(ty, -value.number())
            } else {
                Ok(Value::new(ty, value.bits.wrapping_neg()))
            }
        },
    }
}

/// Every partial C operation reports its own failure, even if unused later.
#[expect(
    clippy::cast_possible_truncation,
    reason = "Signed results are encoded as low bits; Value::new applies the target-width mask."
)]
pub(super) fn binary(op: Binary, a: Value, b: Value, result: IntType) -> Result<Value, Failure> {
    use Binary as B;
    let (left, right) = (a.number(), b.number());
    let predicate = match op {
        | B::Equal => Some(left == right),
        | B::NotEqual => Some(left != right),
        | B::Less => Some(left < right),
        | B::LessEqual => Some(left <= right),
        | B::Greater => Some(left > right),
        | B::GreaterEqual => Some(left >= right),
        | _ => None,
    };
    if let Some(p) = predicate {
        return Ok(Value::new(result, u64::from(p)));
    }
    if matches!(op, B::ShiftLeft | B::ShiftRight) {
        let shift = u32::try_from(right)
            .ok()
            .filter(|&n| n < a.ty.bits)
            .ok_or(Failure::Undefined(Undefined::InvalidShift))?;
        if op == B::ShiftRight {
            return Ok(Value::new(
                result,
                if a.ty.signed {
                    (left >> shift) as u64
                } else {
                    a.bits >> shift
                },
            ));
        }
        if a.ty.signed {
            if left < 0 {
                return Err(Failure::Undefined(Undefined::InvalidShift));
            }
            return checked(a.ty, left << shift).map(|v| v.cast(result));
        }
        return Ok(Value::new(result, a.bits << shift));
    }
    if matches!(op, B::Divide | B::Remainder) {
        if right == 0 {
            return Err(Failure::Undefined(Undefined::DivisionByZero));
        }
        if a.ty.signed && left == a.ty.minimum() && right == -1 {
            return Err(Failure::Undefined(Undefined::SignedOverflow));
        }
        return Ok(Value::new(
            result,
            if op == B::Divide {
                (left / right) as u64
            } else {
                (left % right) as u64
            },
        ));
    }
    let bits = match op {
        | B::And => a.bits & b.bits,
        | B::Or => a.bits | b.bits,
        | B::Xor => a.bits ^ b.bits,
        | B::Add | B::Subtract | B::Multiply if a.ty.signed => {
            let value = match op {
                | B::Add => left + right,
                | B::Subtract => left - right,
                | _ => left * right,
            };
            return checked(a.ty, value).map(|v| v.cast(result));
        },
        | B::Add => a.bits.wrapping_add(b.bits),
        | B::Subtract => a.bits.wrapping_sub(b.bits),
        | B::Multiply => a.bits.wrapping_mul(b.bits),
        | _ => return Err(Failure::InvalidIr),
    };
    Ok(Value::new(result, bits))
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "The checked signed value is encoded as low bits, including for negative values."
)]
fn checked(ty: IntType, value: i128) -> Result<Value, Failure> {
    if !(ty.minimum()..=ty.maximum()).contains(&value) {
        return Err(Failure::Undefined(Undefined::SignedOverflow));
    }
    Ok(Value::new(ty, value as u64))
}
