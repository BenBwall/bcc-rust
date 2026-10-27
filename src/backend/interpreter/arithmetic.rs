//! The value operations: integer and floating arithmetic, comparisons and
//! conversions, each with the IR's poison rules.
//!
//! Poison in an operand makes the result poison, except that a division by
//! poison or zero is undefined behaviour. A violated `nsw`, `nuw` or
//! `exact`, a shift by the width or more, and a float-to-integer conversion
//! out of range all produce poison. `f80` values compute as the x87 unit
//! does, through the software model in `crate::x87`, and `f128` values as
//! libgcc's soft-float routines do, through `crate::binary128`.

use super::{
    memory::truncate_u64,
    trap::{
        Fault,
        UbKind,
    },
    value::RuntimeValue,
};
use crate::{
    binary128::Binary128,
    ir::{
        FloatCC,
        InstFlags,
        IntCC,
        Opcode,
        Type,
    },
    x87::X87,
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
/// C99: §6.5.5 paragraphs 4-5, p. 82; PDF p. 94; §6.5.6 paragraphs 5-6,
/// p. 82; PDF p. 94; §7.12.10.1, p. 235; PDF p. 247.
pub(crate) fn float_binary(
    opcode: Opcode,
    ty: Type,
    a: RuntimeValue,
    b: RuntimeValue,
) -> RuntimeValue {
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
    let apply80 = |a: X87, b: X87| match opcode {
        | Opcode::Fadd => a.add(b),
        | Opcode::Fsub => a.sub(b),
        | Opcode::Fmul => a.mul(b),
        | Opcode::Fdiv => a.div(b),
        | _ => a.rem(b),
    };
    let apply128 = |a: u128, b: u128| {
        let (a, b) = (Binary128::from_bits(a), Binary128::from_bits(b));
        match opcode {
            | Opcode::Fadd => a.add(b),
            | Opcode::Fsub => a.sub(b),
            | Opcode::Fmul => a.mul(b),
            | Opcode::Fdiv => a.div(b),
            | _ => a.rem(b),
        }
        .to_bits()
    };
    debug_assert!(ty.is_float(), "{opcode} on {ty}");
    match (a, b) {
        | (RuntimeValue::F32(a), RuntimeValue::F32(b)) => RuntimeValue::F32(apply32(a, b)),
        | (RuntimeValue::F64(a), RuntimeValue::F64(b)) => RuntimeValue::F64(apply64(a, b)),
        | (RuntimeValue::F80(a), RuntimeValue::F80(b)) => RuntimeValue::F80(apply80(a, b)),
        | (RuntimeValue::F128(a), RuntimeValue::F128(b)) => RuntimeValue::F128(apply128(a, b)),
        | _ => RuntimeValue::Poison,
    }
}

/// `fneg`: the sign bit flipped, NaNs included.
pub(crate) fn float_negate(ty: Type, a: RuntimeValue) -> RuntimeValue {
    debug_assert!(ty.is_float(), "fneg on {ty}");
    match a {
        | RuntimeValue::F32(a) => RuntimeValue::F32(-a),
        | RuntimeValue::F64(a) => RuntimeValue::F64(-a),
        | RuntimeValue::F80(a) => RuntimeValue::F80(a.negate()),
        | RuntimeValue::F128(a) => RuntimeValue::F128(Binary128::from_bits(a).negate().to_bits()),
        | _ => RuntimeValue::Poison,
    }
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
/// unordered one true. An invalid x87 encoding compares unordered, as
/// `fucomi` reports it.
/// C99: §6.5.8 paragraphs 3-6, p. 85; PDF p. 97; §6.5.9 paragraph 3,
/// p. 86; PDF p. 98.
pub(crate) fn float_compare(
    cond: FloatCC,
    ty: Type,
    a: RuntimeValue,
    b: RuntimeValue,
) -> RuntimeValue {
    debug_assert!(ty.is_float(), "fcmp on {ty}");
    let order = match (a, b) {
        | (RuntimeValue::F32(a), RuntimeValue::F32(b)) => a.partial_cmp(&b),
        | (RuntimeValue::F64(a), RuntimeValue::F64(b)) => a.partial_cmp(&b),
        | (RuntimeValue::F80(a), RuntimeValue::F80(b)) => a.compare(b),
        | (RuntimeValue::F128(a), RuntimeValue::F128(b)) =>
            Binary128::from_bits(a).compare(Binary128::from_bits(b)),
        | _ => return RuntimeValue::Poison,
    };
    let Some(order) = order else {
        return boolean(matches!(
            cond,
            FloatCC::Ueq
                | FloatCC::Une
                | FloatCC::Ult
                | FloatCC::Ule
                | FloatCC::Ugt
                | FloatCC::Uge
                | FloatCC::Uno
        ));
    };
    boolean(match cond {
        | FloatCC::Oeq | FloatCC::Ueq => order.is_eq(),
        | FloatCC::One | FloatCC::Une => order.is_ne(),
        | FloatCC::Olt | FloatCC::Ult => order.is_lt(),
        | FloatCC::Ole | FloatCC::Ule => order.is_le(),
        | FloatCC::Ogt | FloatCC::Ugt => order.is_gt(),
        | FloatCC::Oge | FloatCC::Uge => order.is_ge(),
        | FloatCC::Ord => true,
        | FloatCC::Uno => false,
    })
}

/// A conversion of `a` from type `from` to type `to`, other than
/// `inttoptr`, which needs memory. A float-to-integer conversion whose
/// truncated value does not fit the result is poison.
#[expect(
    clippy::cast_precision_loss,
    reason = "sitofp and uitofp round to the nearest representable value."
)]
pub(crate) fn convert(opcode: Opcode, from: Type, to: Type, a: RuntimeValue) -> RuntimeValue {
    if a.is_poison() {
        return RuntimeValue::Poison;
    }
    if let Some(value) = binary128_conversion(opcode, from, to, a) {
        return value;
    }
    match (opcode, a) {
        | (Opcode::Zext | Opcode::Trunc, RuntimeValue::Int(a)) => RuntimeValue::int(to, a),
        | (Opcode::Sext, RuntimeValue::Int(a)) => RuntimeValue::int(to, sext(from, a) as u128),
        | (Opcode::Fpext, RuntimeValue::F32(a)) if to == Type::F64 =>
            RuntimeValue::F64(f64::from(a)),
        | (Opcode::Fpext, RuntimeValue::F32(a)) => RuntimeValue::F80(X87::from_f32(a)),
        | (Opcode::Fpext, RuntimeValue::F64(a)) => RuntimeValue::F80(X87::from_f64(a)),
        | (Opcode::Fptrunc, RuntimeValue::F80(a)) if to == Type::F32 =>
            RuntimeValue::F32(a.to_f32()),
        | (Opcode::Fptrunc, RuntimeValue::F80(a)) => RuntimeValue::F64(a.to_f64()),
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
        | (Opcode::Fptosi | Opcode::Fptoui, RuntimeValue::F80(a)) => a
            .to_integer(opcode == Opcode::Fptosi, to.bits())
            .map_or(RuntimeValue::Poison, |bits| RuntimeValue::int(to, bits)),
        | (Opcode::Sitofp, RuntimeValue::Int(a)) if to == Type::F80 => {
            let value = sext(from, a);
            RuntimeValue::F80(X87::from_integer(value < 0, value.unsigned_abs()))
        },
        | (Opcode::Uitofp, RuntimeValue::Int(a)) if to == Type::F80 =>
            RuntimeValue::F80(X87::from_integer(false, a)),
        | (Opcode::Sitofp, RuntimeValue::Int(a)) =>
            int_to_float(to, sext(from, a) as f64, sext(from, a) as f32),
        | (Opcode::Uitofp, RuntimeValue::Int(a)) => int_to_float(to, a as f64, a as f32),
        | (Opcode::Ptrtoint, RuntimeValue::Ptr(pointer)) =>
            RuntimeValue::int(to, u128::from(pointer.address)),
        | (Opcode::Bitcast, value) => bitcast(to, value),
        | _ => unreachable!("{opcode} from {from} to {to} is not a legal conversion"),
    }
}

/// A conversion to or from `f128`, as libgcc's `__extenddftf2`,
/// `__trunctfdf2`, `__fixtfdi`, `__floatditf` and their relatives give it;
/// `None` for any other conversion. An out-of-range conversion to an
/// integer is poison.
/// C99: §6.3.1.4-§6.3.1.5, pp. 43-44; PDF pp. 55-56.
fn binary128_conversion(
    opcode: Opcode,
    from: Type,
    to: Type,
    a: RuntimeValue,
) -> Option<RuntimeValue> {
    let quad = |value: Binary128| RuntimeValue::F128(value.to_bits());
    Some(match (opcode, a) {
        | (Opcode::Fpext, RuntimeValue::F32(a)) if to == Type::F128 => quad(Binary128::from_f32(a)),
        | (Opcode::Fpext, RuntimeValue::F64(a)) if to == Type::F128 => quad(Binary128::from_f64(a)),
        | (Opcode::Fpext, RuntimeValue::F80(a)) if to == Type::F128 => quad(Binary128::from_x87(a)),
        | (Opcode::Sitofp, RuntimeValue::Int(a)) if to == Type::F128 => {
            let value = sext(from, a);
            quad(Binary128::from_integer(value < 0, value.unsigned_abs()))
        },
        | (Opcode::Uitofp, RuntimeValue::Int(a)) if to == Type::F128 =>
            quad(Binary128::from_integer(false, a)),
        | (Opcode::Fptrunc, RuntimeValue::F128(a)) => {
            let a = Binary128::from_bits(a);
            match to {
                | Type::F32 => RuntimeValue::F32(a.to_f32()),
                | Type::F64 => RuntimeValue::F64(a.to_f64()),
                | _ => RuntimeValue::F80(a.to_x87()),
            }
        },
        | (Opcode::Fptosi | Opcode::Fptoui, RuntimeValue::F128(a)) => Binary128::from_bits(a)
            .to_integer(opcode == Opcode::Fptosi, to.bits())
            .map_or(RuntimeValue::Poison, |bits| RuntimeValue::int(to, bits)),
        | _ => return None,
    })
}

/// A `bitcast`: the same bits viewed as another type of the same width.
fn bitcast(to: Type, value: RuntimeValue) -> RuntimeValue {
    let bits = match value {
        | RuntimeValue::Ptr(_) | RuntimeValue::Poison => return value,
        | RuntimeValue::Int(bits) | RuntimeValue::F128(bits) => bits,
        | RuntimeValue::F32(value) => u128::from(value.to_bits()),
        | RuntimeValue::F64(value) => u128::from(value.to_bits()),
        | RuntimeValue::F80(value) => value.to_bits(),
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
        | Type::F80 => RuntimeValue::F80(X87::from_bits(bits)),
        | Type::F128 => RuntimeValue::F128(bits),
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

/// The `i1` for a truth value.
const fn boolean(value: bool) -> RuntimeValue {
    RuntimeValue::Int(value as u128)
}

/// The signed value of the integer `bits` of type `ty`.
pub(crate) const fn sext(ty: Type, bits: u128) -> i128 {
    let shift = 128 - ty.bits();
    ((bits << shift) as i128) >> shift
}

/// The `minnum` or `maxnum` of two x87 values, which atomic `fmin` and
/// `fmax` compute: a NaN operand gives the other operand.
pub(crate) fn extended_min_max(minimum: bool, a: X87, b: X87) -> X87 {
    let take_b = match a.compare(b) {
        | None => a.compare(a).is_none(),
        | Some(order) => order.is_gt() == minimum,
    };
    if take_b { b } else { a }
}
