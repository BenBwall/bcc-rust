//! Exact evaluation of integer operations on constants.
//!
//! Operands are the constants' bits, masked to the operation's type. Results
//! follow the IR's poison rules (`middle-end.md`, "Undefined behaviour"): a
//! violated `nsw`, `nuw` or `exact` flag and a shift by at least the width
//! give poison. Division and remainder by zero, and the signed quotient or
//! remainder of the most negative number by -1, are undefined behaviour, not
//! poison; evaluation returns `None` for them so the caller leaves the
//! instruction alone.
//!
//! C99: §6.5 paragraph 5, p. 67; PDF p. 79 (signed overflow is undefined, so
//! `nsw` operations may be treated as never overflowing); §6.5.5 paragraph 5,
//! p. 82; PDF p. 94 (division by zero); §6.5.7 paragraph 3, p. 84; PDF p. 96
//! (oversized shift counts).

use crate::ir::{
    InstFlags,
    IntCC,
    Opcode,
    Type,
};

/// The value of an operation on constants.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::optimizer) enum Folded {
    /// The result's bits, masked to its type.
    Int(u128),
    Poison,
}

/// `bits` of type `ty` as a signed number.
pub(super) const fn signed(bits: u128, ty: Type) -> i128 {
    let shift = 128 - ty.bits();
    ((bits << shift) as i128) >> shift
}

/// Evaluates an integer binary operation of type `ty`, or returns `None` if
/// it is undefined behaviour or not an integer binary operation.
pub(in crate::optimizer) fn binary(
    opcode: Opcode,
    ty: Type,
    flags: InstFlags,
    a: u128,
    b: u128,
) -> Option<Folded> {
    let mask = ty.mask();
    let width = ty.bits();
    let poison_if = |violated: bool, result: u128| {
        if violated {
            Folded::Poison
        } else {
            Folded::Int(result)
        }
    };
    Some(match opcode {
        | Opcode::Iadd => {
            let (sum, carry) = a.overflowing_add(b);
            let result = sum & mask;
            let unsigned = flags.contains(InstFlags::NUW) && (carry || sum > mask);
            let (wide, overflow) = signed(a, ty).overflowing_add(signed(b, ty));
            let signed_wrap =
                flags.contains(InstFlags::NSW) && (overflow || wide != signed(result, ty));
            poison_if(unsigned || signed_wrap, result)
        },
        | Opcode::Isub => {
            let result = a.wrapping_sub(b) & mask;
            let unsigned = flags.contains(InstFlags::NUW) && a < b;
            let (wide, overflow) = signed(a, ty).overflowing_sub(signed(b, ty));
            let signed_wrap =
                flags.contains(InstFlags::NSW) && (overflow || wide != signed(result, ty));
            poison_if(unsigned || signed_wrap, result)
        },
        | Opcode::Imul => {
            let (product, carry) = a.overflowing_mul(b);
            let result = product & mask;
            let unsigned = flags.contains(InstFlags::NUW) && (carry || product > mask);
            let (wide, overflow) = signed(a, ty).overflowing_mul(signed(b, ty));
            let signed_wrap =
                flags.contains(InstFlags::NSW) && (overflow || wide != signed(result, ty));
            poison_if(unsigned || signed_wrap, result)
        },
        | Opcode::Udiv => {
            if b == 0 {
                return None;
            }
            poison_if(
                flags.contains(InstFlags::EXACT) && !a.is_multiple_of(b),
                (a / b) & mask,
            )
        },
        | Opcode::Urem => {
            if b == 0 {
                return None;
            }
            Folded::Int((a % b) & mask)
        },
        | Opcode::Sdiv | Opcode::Srem => {
            let (x, y) = (signed(a, ty), signed(b, ty));
            if y == 0 || (y == -1 && x == signed(1 << (width - 1), ty)) {
                return None;
            }
            if opcode == Opcode::Sdiv {
                poison_if(
                    flags.contains(InstFlags::EXACT) && x % y != 0,
                    ((x / y) as u128) & mask,
                )
            } else {
                Folded::Int(((x % y) as u128) & mask)
            }
        },
        | Opcode::And => Folded::Int(a & b),
        | Opcode::Or => Folded::Int(a | b),
        | Opcode::Xor => Folded::Int(a ^ b),
        | Opcode::Shl | Opcode::Lshr | Opcode::Ashr => {
            if b >= u128::from(width) {
                return Some(Folded::Poison);
            }
            let amount = u32::try_from(b).expect("the amount is below the width");
            match opcode {
                | Opcode::Shl => {
                    let result = (a << amount) & mask;
                    let unsigned = flags.contains(InstFlags::NUW) && (result >> amount) != a;
                    let signed_wrap = flags.contains(InstFlags::NSW)
                        && (signed(result, ty) >> amount) != signed(a, ty);
                    poison_if(unsigned || signed_wrap, result)
                },
                | Opcode::Lshr => poison_if(
                    flags.contains(InstFlags::EXACT) && a & ((1 << amount) - 1) != 0,
                    a >> amount,
                ),
                | _ => poison_if(
                    flags.contains(InstFlags::EXACT) && a & ((1 << amount) - 1) != 0,
                    ((signed(a, ty) >> amount) as u128) & mask,
                ),
            }
        },
        | _ => return None,
    })
}

/// Evaluates `icmp` on operands of type `ty`.
pub(in crate::optimizer) fn compare(cond: IntCC, ty: Type, a: u128, b: u128) -> bool {
    let (x, y) = (signed(a, ty), signed(b, ty));
    match cond {
        | IntCC::Eq => a == b,
        | IntCC::Ne => a != b,
        | IntCC::Slt => x < y,
        | IntCC::Sle => x <= y,
        | IntCC::Sgt => x > y,
        | IntCC::Sge => x >= y,
        | IntCC::Ult => a < b,
        | IntCC::Ule => a <= b,
        | IntCC::Ugt => a > b,
        | IntCC::Uge => a >= b,
    }
}

/// Evaluates `zext`, `sext` or `trunc` of `bits` of type `from` to type
/// `to`, or returns `None` for any other conversion.
pub(in crate::optimizer) fn convert(
    opcode: Opcode,
    from: Type,
    to: Type,
    bits: u128,
) -> Option<u128> {
    match opcode {
        | Opcode::Zext => Some(bits),
        | Opcode::Sext => Some((signed(bits, from) as u128) & to.mask()),
        | Opcode::Trunc => Some(bits & to.mask()),
        | _ => None,
    }
}
