//! Numeric instructions (WebAssembly 2.0 §4.3 and §4.4.1): integer
//! arithmetic wraps modulo 2^N, division traps on zero and on the one
//! overflowing quotient, shifts and rotations take their count modulo N,
//! and floats follow IEEE 754 with the spec's choices for `min`, `max`,
//! `nearest` and NaN results. Conversions that cannot represent their
//! result trap, except the saturating ones.
//!
//! Where the spec lets a NaN result be any NaN, the stepper returns the
//! canonical one, so execution is deterministic everywhere.

use super::{
    Trap,
    TrapKind,
};
use crate::value::Value;

/// Apply numeric opcode `op` (0x45..=0xC4) to `args`, in stack order.
///
/// # Errors
/// Integer division by zero or overflow, and float-to-integer truncations
/// whose result does not fit.
#[expect(clippy::too_many_lines, reason = "One arm per opcode.")]
pub fn apply(op: u8, args: &[Value]) -> Result<Value, Trap> {
    use Value::{
        F32,
        F64,
        I32,
        I64,
    };
    let bool32 = |condition: bool| Ok(I32(i32::from(condition)));
    Ok(match (op, args) {
        // i32 comparisons.
        | (0x45, [I32(a)]) => return bool32(*a == 0),
        | (0x46..=0x4F, [I32(a), I32(b)]) => {
            let (a, b) = (*a, *b);
            let (ua, ub) = (a as u32, b as u32);
            return bool32(match op {
                | 0x46 => a == b,
                | 0x47 => a != b,
                | 0x48 => a < b,
                | 0x49 => ua < ub,
                | 0x4A => a > b,
                | 0x4B => ua > ub,
                | 0x4C => a <= b,
                | 0x4D => ua <= ub,
                | 0x4E => a >= b,
                | _ => ua >= ub,
            });
        },
        // i64 comparisons.
        | (0x50, [I64(a)]) => return bool32(*a == 0),
        | (0x51..=0x5A, [I64(a), I64(b)]) => {
            let (a, b) = (*a, *b);
            let (ua, ub) = (a as u64, b as u64);
            return bool32(match op {
                | 0x51 => a == b,
                | 0x52 => a != b,
                | 0x53 => a < b,
                | 0x54 => ua < ub,
                | 0x55 => a > b,
                | 0x56 => ua > ub,
                | 0x57 => a <= b,
                | 0x58 => ua <= ub,
                | 0x59 => a >= b,
                | _ => ua >= ub,
            });
        },
        // Float comparisons: false whenever an operand is NaN, except `ne`.
        | (0x5B..=0x60, [F32(a), F32(b)]) => {
            let (a, b) = (f32::from_bits(*a), f32::from_bits(*b));
            return bool32(compare(op - 0x5B, f64::from(a), f64::from(b)));
        },
        | (0x61..=0x66, [F64(a), F64(b)]) => {
            let (a, b) = (f64::from_bits(*a), f64::from_bits(*b));
            return bool32(compare(op - 0x61, a, b));
        },
        // i32 arithmetic.
        | (0x67, [I32(a)]) => I32(a.leading_zeros() as i32),
        | (0x68, [I32(a)]) => I32(a.trailing_zeros() as i32),
        | (0x69, [I32(a)]) => I32(a.count_ones() as i32),
        | (0x6A..=0x78, [I32(a), I32(b)]) => I32(int32(op, *a, *b)?),
        // i64 arithmetic.
        | (0x79, [I64(a)]) => I64(i64::from(a.leading_zeros())),
        | (0x7A, [I64(a)]) => I64(i64::from(a.trailing_zeros())),
        | (0x7B, [I64(a)]) => I64(i64::from(a.count_ones())),
        | (0x7C..=0x8A, [I64(a), I64(b)]) => I64(int64(op, *a, *b)?),
        // f32 arithmetic. abs, neg and copysign change only the sign bit.
        | (0x8B, [F32(a)]) => F32(a & 0x7FFF_FFFF),
        | (0x8C, [F32(a)]) => F32(a ^ 0x8000_0000),
        | (0x8D..=0x91, [F32(a)]) => Value::f32(unary32(op, f32::from_bits(*a))),
        | (0x92..=0x97, [F32(a), F32(b)]) =>
            Value::f32(binary32(op, f32::from_bits(*a), f32::from_bits(*b))),
        | (0x98, [F32(a), F32(b)]) => F32((a & 0x7FFF_FFFF) | (b & 0x8000_0000)),
        // f64 arithmetic.
        | (0x99, [F64(a)]) => F64(a & 0x7FFF_FFFF_FFFF_FFFF),
        | (0x9A, [F64(a)]) => F64(a ^ 0x8000_0000_0000_0000),
        | (0x9B..=0x9F, [F64(a)]) => Value::f64(unary64(op - 0x0E, f64::from_bits(*a))),
        | (0xA0..=0xA5, [F64(a), F64(b)]) =>
            Value::f64(binary64(op - 0x0E, f64::from_bits(*a), f64::from_bits(*b))),
        | (0xA6, [F64(a), F64(b)]) =>
            F64((a & 0x7FFF_FFFF_FFFF_FFFF) | (b & 0x8000_0000_0000_0000)),
        // Conversions.
        | (0xA7, [I64(a)]) => I32(*a as i32),
        | (0xA8, [F32(a)]) => I32(truncate(f64::from(f32::from_bits(*a)), Target::I32)? as i32),
        | (0xA9, [F32(a)]) => I32(truncate(f64::from(f32::from_bits(*a)), Target::U32)? as i32),
        | (0xAA, [F64(a)]) => I32(truncate(f64::from_bits(*a), Target::I32)? as i32),
        | (0xAB, [F64(a)]) => I32(truncate(f64::from_bits(*a), Target::U32)? as i32),
        | (0xAC, [I32(a)]) => I64(i64::from(*a)),
        | (0xAD, [I32(a)]) => I64(i64::from(*a as u32)),
        | (0xAE, [F32(a)]) => I64(truncate(f64::from(f32::from_bits(*a)), Target::I64)? as i64),
        | (0xAF, [F32(a)]) => I64(truncate(f64::from(f32::from_bits(*a)), Target::U64)? as i64),
        | (0xB0, [F64(a)]) => I64(truncate(f64::from_bits(*a), Target::I64)? as i64),
        | (0xB1, [F64(a)]) => I64(truncate(f64::from_bits(*a), Target::U64)? as i64),
        | (0xB2, [I32(a)]) => Value::f32(*a as f32),
        | (0xB3, [I32(a)]) => Value::f32(*a as u32 as f32),
        | (0xB4, [I64(a)]) => Value::f32(*a as f32),
        | (0xB5, [I64(a)]) => Value::f32(*a as u64 as f32),
        | (0xB6, [F64(a)]) => Value::f32(canonical32(f64::from_bits(*a) as f32)),
        | (0xB7, [I32(a)]) => Value::f64(f64::from(*a)),
        | (0xB8, [I32(a)]) => Value::f64(f64::from(*a as u32)),
        | (0xB9, [I64(a)]) => Value::f64(*a as f64),
        | (0xBA, [I64(a)]) => Value::f64(*a as u64 as f64),
        | (0xBB, [F32(a)]) => Value::f64(canonical64(f64::from(f32::from_bits(*a)))),
        // Reinterpretations keep every bit.
        | (0xBC, [F32(a)]) => I32(*a as i32),
        | (0xBD, [F64(a)]) => I64(*a as i64),
        | (0xBE, [I32(a)]) => F32(*a as u32),
        | (0xBF, [I64(a)]) => F64(*a as u64),
        // Sign extension.
        | (0xC0, [I32(a)]) => I32(i32::from(*a as i8)),
        | (0xC1, [I32(a)]) => I32(i32::from(*a as i16)),
        | (0xC2, [I64(a)]) => I64(i64::from(*a as i8)),
        | (0xC3, [I64(a)]) => I64(i64::from(*a as i16)),
        | (0xC4, [I64(a)]) => I64(i64::from(*a as i32)),
        | _ => {
            return Err(Trap::new(
                TrapKind::Internal,
                format!("stepper error: opcode {op:#04x} got the wrong operands"),
            ));
        },
    })
}

/// `eq`, `ne`, `lt`, `gt`, `le`, `ge` by index.
#[expect(clippy::float_cmp, reason = "IEEE 754 comparison is the instruction.")]
fn compare(index: u8, a: f64, b: f64) -> bool {
    match index {
        | 0 => a == b,
        | 1 => a != b,
        | 2 => a < b,
        | 3 => a > b,
        | 4 => a <= b,
        | _ => a >= b,
    }
}

fn int32(op: u8, a: i32, b: i32) -> Result<i32, Trap> {
    let (ua, ub) = (a as u32, b as u32);
    Ok(match op {
        | 0x6A => a.wrapping_add(b),
        | 0x6B => a.wrapping_sub(b),
        | 0x6C => a.wrapping_mul(b),
        | 0x6D => {
            nonzero(i64::from(b))?;
            if a == i32::MIN && b == -1 {
                return Err(overflow());
            }
            a / b
        },
        | 0x6E => {
            nonzero(i64::from(b))?;
            (ua / ub) as i32
        },
        | 0x6F => {
            nonzero(i64::from(b))?;
            a.wrapping_rem(b)
        },
        | 0x70 => {
            nonzero(i64::from(b))?;
            (ua % ub) as i32
        },
        | 0x71 => a & b,
        | 0x72 => a | b,
        | 0x73 => a ^ b,
        | 0x74 => a.wrapping_shl(ub),
        | 0x75 => a.wrapping_shr(ub),
        | 0x76 => ua.wrapping_shr(ub) as i32,
        | 0x77 => ua.rotate_left(ub % 32) as i32,
        | _ => ua.rotate_right(ub % 32) as i32,
    })
}

fn int64(op: u8, a: i64, b: i64) -> Result<i64, Trap> {
    let (ua, ub) = (a as u64, b as u64);
    Ok(match op {
        | 0x7C => a.wrapping_add(b),
        | 0x7D => a.wrapping_sub(b),
        | 0x7E => a.wrapping_mul(b),
        | 0x7F => {
            nonzero(b)?;
            if a == i64::MIN && b == -1 {
                return Err(overflow());
            }
            a / b
        },
        | 0x80 => {
            nonzero(b)?;
            (ua / ub) as i64
        },
        | 0x81 => {
            nonzero(b)?;
            a.wrapping_rem(b)
        },
        | 0x82 => {
            nonzero(b)?;
            (ua % ub) as i64
        },
        | 0x83 => a & b,
        | 0x84 => a | b,
        | 0x85 => a ^ b,
        | 0x86 => a.wrapping_shl(ub as u32),
        | 0x87 => a.wrapping_shr(ub as u32),
        | 0x88 => ua.wrapping_shr(ub as u32) as i64,
        | 0x89 => ua.rotate_left((ub % 64) as u32) as i64,
        | _ => ua.rotate_right((ub % 64) as u32) as i64,
    })
}

fn nonzero(divisor: i64) -> Result<(), Trap> {
    if divisor == 0 {
        Err(Trap::new(TrapKind::DivideByZero, "integer divide by zero"))
    } else {
        Ok(())
    }
}

fn overflow() -> Trap {
    Trap::new(
        TrapKind::IntegerOverflow,
        "integer overflow: the quotient of the most negative integer by -1 does not fit",
    )
}

/// `ceil`, `floor`, `trunc`, `nearest`, `sqrt` (f32 opcodes 0x8D..=0x91).
fn unary32(op: u8, a: f32) -> f32 {
    canonical32(match op {
        | 0x8D => a.ceil(),
        | 0x8E => a.floor(),
        | 0x8F => a.trunc(),
        | 0x90 => a.round_ties_even(),
        | _ => a.sqrt(),
    })
}

fn unary64(op: u8, a: f64) -> f64 {
    canonical64(match op {
        | 0x8D => a.ceil(),
        | 0x8E => a.floor(),
        | 0x8F => a.trunc(),
        | 0x90 => a.round_ties_even(),
        | _ => a.sqrt(),
    })
}

/// `add`, `sub`, `mul`, `div`, `min`, `max` (f32 opcodes 0x92..=0x97).
fn binary32(op: u8, a: f32, b: f32) -> f32 {
    canonical32(match op {
        | 0x92 => a + b,
        | 0x93 => a - b,
        | 0x94 => a * b,
        | 0x95 => a / b,
        | 0x96 => minimum(f64::from(a), f64::from(b)) as f32,
        | _ => maximum(f64::from(a), f64::from(b)) as f32,
    })
}

fn binary64(op: u8, a: f64, b: f64) -> f64 {
    canonical64(match op {
        | 0x92 => a + b,
        | 0x93 => a - b,
        | 0x94 => a * b,
        | 0x95 => a / b,
        | 0x96 => minimum(a, b),
        | _ => maximum(a, b),
    })
}

/// `min`: NaN if either operand is, and -0 below +0.
#[expect(clippy::float_cmp, reason = "Equal operands are the signed-zero case.")]
fn minimum(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == b {
        if a.is_sign_negative() { a } else { b }
    } else {
        a.min(b)
    }
}

#[expect(clippy::float_cmp, reason = "Equal operands are the signed-zero case.")]
fn maximum(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == b {
        if a.is_sign_positive() { a } else { b }
    } else {
        a.max(b)
    }
}

/// Any NaN result becomes the positive canonical NaN.
fn canonical32(value: f32) -> f32 {
    if value.is_nan() {
        f32::from_bits(0x7FC0_0000)
    } else {
        value
    }
}

fn canonical64(value: f64) -> f64 {
    if value.is_nan() {
        f64::from_bits(0x7FF8_0000_0000_0000)
    } else {
        value
    }
}

#[derive(Clone, Copy)]
enum Target {
    I32,
    U32,
    I64,
    U64,
}

/// A trapping float-to-integer truncation (§4.3.4): the truncated value as
/// an `i128`, which the caller narrows. Every f32 converts to f64 exactly,
/// so one check serves both widths.
fn truncate(value: f64, target: Target) -> Result<i128, Trap> {
    if value.is_nan() {
        return Err(Trap::new(
            TrapKind::InvalidConversion,
            "invalid conversion to integer: the operand is NaN",
        ));
    }
    let truncated = value.trunc();
    let (low, high) = match target {
        | Target::I32 => (-2_147_483_648.0, 2_147_483_648.0),
        | Target::U32 => (0.0, 4_294_967_296.0),
        | Target::I64 => (-9_223_372_036_854_775_808.0, 9_223_372_036_854_775_808.0),
        | Target::U64 => (0.0, 18_446_744_073_709_551_616.0),
    };
    if truncated < low || truncated >= high {
        return Err(Trap::new(
            TrapKind::IntegerOverflow,
            format!(
                "integer overflow: {value} does not fit in the conversion's {} result",
                match target {
                    | Target::I32 => "i32",
                    | Target::U32 => "unsigned i32",
                    | Target::I64 => "i64",
                    | Target::U64 => "unsigned i64",
                }
            ),
        ));
    }
    let integer = truncated as i128;
    Ok(match target {
        | Target::U32 => i128::from(integer as u32 as i32),
        | Target::U64 => i128::from(integer as u64 as i64),
        | Target::I32 | Target::I64 => integer,
    })
}

/// A saturating truncation (`0xFC` 0..=7): NaN becomes 0 and out-of-range
/// values clamp, which is what Rust's `as` does.
///
/// # Errors
/// Only for an operand of the wrong type, which validation rules out.
pub fn trunc_sat(op: u8, operand: Value) -> Result<Value, Trap> {
    let value = match operand {
        | Value::F32(bits) => f64::from(f32::from_bits(bits)),
        | Value::F64(bits) => f64::from_bits(bits),
        | _ =>
            return Err(Trap::new(
                TrapKind::Internal,
                "stepper error: trunc_sat operand",
            )),
    };
    Ok(match op {
        | 0 | 2 => Value::I32(value as i32),
        | 1 | 3 => Value::I32(value as u32 as i32),
        | 4 | 6 => Value::I64(value as i64),
        | _ => Value::I64(value as u64 as i64),
    })
}
