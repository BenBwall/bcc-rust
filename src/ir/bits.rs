//! Exact scalar intrinsic evaluation, shared by folding and interpretation.
//! GNU extension: GCC "Bit Operation Builtins" and "Integer Overflow Builtins".
//! <https://gcc.gnu.org/onlinedocs/gcc/Bit-Operation-Builtins.html>
//! <https://gcc.gnu.org/onlinedocs/gcc/Integer-Overflow-Builtins.html>

use super::{
    Opcode,
    Type,
};

/// Counts use the operand width. None is poison for a zero-undefined count.
pub(crate) fn bit_unary(opcode: Opcode, ty: Type, bits: u128) -> Option<u128> {
    let bits = bits & ty.mask();
    Some(match opcode {
        | Opcode::Clz | Opcode::Ctz if bits == 0 => return None,
        | Opcode::Clz | Opcode::ClzDefined => u128::from(bits.leading_zeros() - (128 - ty.bits())),
        | Opcode::Ctz | Opcode::CtzDefined => u128::from(bits.trailing_zeros().min(ty.bits())),
        | Opcode::Popcount => u128::from(bits.count_ones()),
        | Opcode::Bswap => bits.swap_bytes() >> (128 - ty.bits()),
        | _ => return None,
    })
}

/// Overflow predicates are total; their result is i1, not the operand type.
pub(crate) fn unsigned_overflow(opcode: Opcode, ty: Type, a: u128, b: u128) -> bool {
    let (value, carry) = if opcode == Opcode::UaddOverflow {
        a.overflowing_add(b)
    } else {
        a.overflowing_mul(b)
    };
    carry || value > ty.mask()
}
