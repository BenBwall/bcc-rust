//! The value operations: integer and floating arithmetic, comparisons and
//! conversions, each with the IR's poison rules.
//!
//! Poison in an operand makes the result poison, except that a division by
//! poison or zero is undefined behaviour. A violated `nsw`, `nuw` or
//! `exact`, a shift by the width or more, and a float-to-integer conversion
//! out of range all produce poison. `f80` and `f128` operations are not
//! supported yet.

use super::{
    memory::truncate_u64,
    trap::{
        Fault,
        UbKind,
        Unsupported,
    },
    value::RuntimeValue,
};
use crate::ir::{
    FloatCC,
    InstFlags,
    IntCC,
    Opcode,
    Type,
};

/// `iadd` through `ashr` on operands of type `ty`.
///
/// Division and remainder by zero, by poison, and the signed division of
/// the minimum value by -1 are undefined. Since a poison dividend could be
/// the minimum value, a signed division of poison by -1 is undefined too.
pub(crate) fn int_binary(
    opcode: Opcode,
    ty: Type,
    flags: InstFlags,
    a: RuntimeValue,
    b: RuntimeValue,
) -> Result<RuntimeValue, Fault> {
    let divides = matches!(
        opcode,
        Opcode::Sdiv | Opcode::Udiv | Opcode::Srem | Opcode::Urem
    );
    if divides {
        let divisor = b.as_int().ok_or(UbKind::DivisionByPoison)?;
        if divisor == 0 {
            return Err(UbKind::DivisionByZero.into());
        }
        let signed = matches!(opcode, Opcode::Sdiv | Opcode::Srem);
        let minimum = 1_u128 << (ty.bits() - 1);
        if signed && divisor == ty.mask() && a.as_int().is_none_or(|a| a == minimum) {
            return Err(UbKind::SignedDivisionOverflow.into());
        }
    }
    let (RuntimeValue::Int(a), RuntimeValue::Int(b)) = (a, b) else {
        return Ok(RuntimeValue::Poison);
    };
    let mask = ty.mask();
    let nsw = flags.contains(InstFlags::NSW);
    let nuw = flags.contains(InstFlags::NUW);
    let exact = flags.contains(InstFlags::EXACT);
    let (sa, sb) = (sext(ty, a), sext(ty, b));
    let fits_signed =
        |value: Option<i128>| value.is_some_and(|value| sext(ty, value as u128 & mask) == value);
    let fits_unsigned = |value: Option<u128>| value.is_some_and(|value| value <= mask);
    let bits = ty.bits();
    let (result, poison) = match opcode {
        | Opcode::Iadd => (
            a.wrapping_add(b),
            (nsw && !fits_signed(sa.checked_add(sb))) || (nuw && !fits_unsigned(a.checked_add(b))),
        ),
        | Opcode::Isub => (
            a.wrapping_sub(b),
            (nsw && !fits_signed(sa.checked_sub(sb))) || (nuw && a < b),
        ),
        | Opcode::Imul => (
            a.wrapping_mul(b),
            (nsw && !fits_signed(sa.checked_mul(sb))) || (nuw && !fits_unsigned(a.checked_mul(b))),
        ),
        | Opcode::Udiv => (a / b, exact && a % b != 0),
        | Opcode::Sdiv => (
            sa.wrapping_div(sb) as u128,
            exact && sa.wrapping_rem(sb) != 0,
        ),
        | Opcode::Urem => (a % b, false),
        | Opcode::Srem => (sa.wrapping_rem(sb) as u128, false),
        | Opcode::And => (a & b, false),
        | Opcode::Or => (a | b, false),
        | Opcode::Xor => (a ^ b, false),
        | Opcode::Shl | Opcode::Lshr | Opcode::Ashr if b >= u128::from(bits) =>
            return Ok(RuntimeValue::Poison),
        | Opcode::Shl => {
            let result = (a << b) & mask;
            let lost_unsigned = result >> b != a;
            let lost_signed = sext(ty, result) >> b != sa;
            (result, (nuw && lost_unsigned) || (nsw && lost_signed))
        },
        | Opcode::Lshr => (a >> b, exact && (a >> b) << b != a),
        | Opcode::Ashr => ((sa >> b) as u128, exact && (a >> b) << b != a),
        | _ => unreachable!("{opcode} is not an integer binary operation"),
    };
    Ok(if poison {
        RuntimeValue::Poison
    } else {
        RuntimeValue::int(ty, result)
    })
}

/// `fadd` through `frem` on operands of type `ty`. `frem` is C's `fmod`.
pub(crate) fn float_binary(
    opcode: Opcode,
    ty: Type,
    a: RuntimeValue,
    b: RuntimeValue,
) -> Result<RuntimeValue, Fault> {
    let apply32 = |a: f32, b: f32| match opcode {
        | Opcode::Fadd => a + b,
        | Opcode::Fsub => a - b,
        | Opcode::Fmul => a * b,
        | Opcode::Fdiv => a / b,
        | _ => a % b,
    };
    let apply64 = |a: f64, b: f64| match opcode {
        | Opcode::Fadd => a + b,
        | Opcode::Fsub => a - b,
        | Opcode::Fmul => a * b,
        | Opcode::Fdiv => a / b,
        | _ => a % b,
    };
    supported_float(ty)?;
    Ok(match (a, b) {
        | (RuntimeValue::F32(a), RuntimeValue::F32(b)) => RuntimeValue::F32(apply32(a, b)),
        | (RuntimeValue::F64(a), RuntimeValue::F64(b)) => RuntimeValue::F64(apply64(a, b)),
        | _ => RuntimeValue::Poison,
    })
}

/// `fneg`.
pub(crate) fn float_negate(ty: Type, a: RuntimeValue) -> Result<RuntimeValue, Fault> {
    supported_float(ty)?;
    Ok(match a {
        | RuntimeValue::F32(a) => RuntimeValue::F32(-a),
        | RuntimeValue::F64(a) => RuntimeValue::F64(-a),
        | _ => RuntimeValue::Poison,
    })
}

/// `icmp` on integers or pointers; pointers compare by address.
pub(crate) fn int_compare(cond: IntCC, ty: Type, a: RuntimeValue, b: RuntimeValue) -> RuntimeValue {
    let (a, b) = match (a, b) {
        | (RuntimeValue::Int(a), RuntimeValue::Int(b)) => (a, b),
        | (RuntimeValue::Ptr(a), RuntimeValue::Ptr(b)) =>
            (u128::from(a.address), u128::from(b.address)),
        | _ => return RuntimeValue::Poison,
    };
    let ty = if ty == Type::Ptr { Type::I64 } else { ty };
    let (sa, sb) = (sext(ty, a), sext(ty, b));
    boolean(match cond {
        | IntCC::Eq => a == b,
        | IntCC::Ne => a != b,
        | IntCC::Slt => sa < sb,
        | IntCC::Sle => sa <= sb,
        | IntCC::Sgt => sa > sb,
        | IntCC::Sge => sa >= sb,
        | IntCC::Ult => a < b,
        | IntCC::Ule => a <= b,
        | IntCC::Ugt => a > b,
        | IntCC::Uge => a >= b,
    })
}

/// `fcmp`: an ordered condition is false if either operand is NaN, an
/// unordered one true.
#[expect(
    clippy::float_cmp,
    reason = "fcmp compares exactly, as IEEE 754 comparisons do."
)]
pub(crate) fn float_compare(
    cond: FloatCC,
    ty: Type,
    a: RuntimeValue,
    b: RuntimeValue,
) -> Result<RuntimeValue, Fault> {
    supported_float(ty)?;
    let (a, b) = match (a, b) {
        | (RuntimeValue::F32(a), RuntimeValue::F32(b)) => (f64::from(a), f64::from(b)),
        | (RuntimeValue::F64(a), RuntimeValue::F64(b)) => (a, b),
        | _ => return Ok(RuntimeValue::Poison),
    };
    let unordered = a.is_nan() || b.is_nan();
    let ordered = !unordered;
    Ok(boolean(match cond {
        | FloatCC::Oeq => ordered && a == b,
        | FloatCC::One => ordered && a != b,
        | FloatCC::Olt => ordered && a < b,
        | FloatCC::Ole => ordered && a <= b,
        | FloatCC::Ogt => ordered && a > b,
        | FloatCC::Oge => ordered && a >= b,
        | FloatCC::Ord => ordered,
        | FloatCC::Ueq => unordered || a == b,
        | FloatCC::Une => unordered || a != b,
        | FloatCC::Ult => unordered || a < b,
        | FloatCC::Ule => unordered || a <= b,
        | FloatCC::Ugt => unordered || a > b,
        | FloatCC::Uge => unordered || a >= b,
        | FloatCC::Uno => unordered,
    }))
}

/// A conversion of `a` from type `from` to type `to`, other than
/// `inttoptr`, which needs memory. A float-to-integer conversion whose
/// truncated value does not fit the result is poison.
#[expect(
    clippy::cast_precision_loss,
    reason = "sitofp and uitofp round to the nearest representable value."
)]
pub(crate) fn convert(
    opcode: Opcode,
    from: Type,
    to: Type,
    a: RuntimeValue,
) -> Result<RuntimeValue, Fault> {
    if from.is_float() {
        supported_float(from)?;
    }
    if to.is_float() {
        supported_float(to)?;
    }
    if a.is_poison() {
        return Ok(RuntimeValue::Poison);
    }
    Ok(match (opcode, a) {
        | (Opcode::Zext | Opcode::Trunc, RuntimeValue::Int(a)) => RuntimeValue::int(to, a),
        | (Opcode::Sext, RuntimeValue::Int(a)) => RuntimeValue::int(to, sext(from, a) as u128),
        | (Opcode::Fpext, RuntimeValue::F32(a)) => RuntimeValue::F64(f64::from(a)),
        | (Opcode::Fptrunc, RuntimeValue::F64(a)) => {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "fptrunc rounds to the narrower format."
            )]
            let narrowed = a as f32;
            RuntimeValue::F32(narrowed)
        },
        | (Opcode::Fptosi | Opcode::Fptoui, RuntimeValue::F32(a)) =>
            float_to_int(opcode == Opcode::Fptosi, to, f64::from(a)),
        | (Opcode::Fptosi | Opcode::Fptoui, RuntimeValue::F64(a)) =>
            float_to_int(opcode == Opcode::Fptosi, to, a),
        | (Opcode::Sitofp, RuntimeValue::Int(a)) =>
            int_to_float(to, sext(from, a) as f64, sext(from, a) as f32),
        | (Opcode::Uitofp, RuntimeValue::Int(a)) => int_to_float(to, a as f64, a as f32),
        | (Opcode::Ptrtoint, RuntimeValue::Ptr(pointer)) =>
            RuntimeValue::int(to, u128::from(pointer.address)),
        | (Opcode::Bitcast, value) => bitcast(to, value),
        | _ => unreachable!("{opcode} from {from} to {to} is not a legal conversion"),
    })
}

/// A `bitcast`: the same bits viewed as another type of the same width.
fn bitcast(to: Type, value: RuntimeValue) -> RuntimeValue {
    let bits = match value {
        | RuntimeValue::Ptr(_) | RuntimeValue::Poison => return value,
        | RuntimeValue::Int(bits) => bits,
        | RuntimeValue::F32(value) => u128::from(value.to_bits()),
        | RuntimeValue::F64(value) => u128::from(value.to_bits()),
    };
    match to {
        | Type::F32 => {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "The source is 32 bits wide."
            )]
            let bits = bits as u32;
            RuntimeValue::F32(f32::from_bits(bits))
        },
        | Type::F64 => RuntimeValue::F64(f64::from_bits(truncate_u64(bits))),
        | _ => RuntimeValue::int(to, bits),
    }
}

/// `fptosi` or `fptoui`: the value truncated toward zero, or poison if it
/// is NaN or does not fit `to`.
fn float_to_int(signed: bool, to: Type, value: f64) -> RuntimeValue {
    let truncated = value.trunc();
    let bits = to.bits();
    let (low, high) = if signed {
        (-(2_f64.powi(bits as i32 - 1)), 2_f64.powi(bits as i32 - 1))
    } else {
        (0.0, 2_f64.powi(bits as i32))
    };
    if truncated.is_nan() || truncated < low || truncated >= high {
        return RuntimeValue::Poison;
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "The value was checked to fit the result type."
    )]
    let result = if signed {
        truncated as i128 as u128
    } else {
        truncated as u128
    };
    RuntimeValue::int(to, result)
}

/// The result of `sitofp` or `uitofp`, from the value already rounded to
/// each format.
const fn int_to_float(to: Type, wide: f64, narrow: f32) -> RuntimeValue {
    match to {
        | Type::F32 => RuntimeValue::F32(narrow),
        | _ => RuntimeValue::F64(wide),
    }
}

/// Fails for the float types the interpreter does not implement.
fn supported_float(ty: Type) -> Result<(), Fault> {
    match ty {
        | Type::F80 | Type::F128 => Err(Fault::Unsupported(Unsupported::WideFloat)),
        | _ => Ok(()),
    }
}

/// The `i1` for a truth value.
const fn boolean(value: bool) -> RuntimeValue {
    RuntimeValue::Int(value as u128)
}

/// The signed value of the integer `bits` of type `ty`.
pub(crate) const fn sext(ty: Type, bits: u128) -> i128 {
    let shift = 128 - ty.bits();
    ((bits << shift) as i128) >> shift
}
