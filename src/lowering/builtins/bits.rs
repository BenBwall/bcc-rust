//! The type-generic bit builtins, computed at the width of the operation
//! type semantic analysis retained. Each derives from a few counts of the
//! operand's bits: its ones, and its leading or trailing zeros or ones,
//! which are the width for an operand of all the other bit. A value of at
//! most 128 bits is an IR integer kept extended from its width
//! ([`super::super::types::Repr::Int`]), counted with `popcount`,
//! `clz_defined` and `ctz_defined` on its IR type and corrected for the
//! bits above its width; a wider `_BitInt` is counted by the runtime of
//! `wide_bit_int.rs`. `__builtin_clzg` and `__builtin_ctzg` with no result
//! for zero use `clz` and `ctz`, undefined at zero as the builtins are.
//!
//! GNU extension: GCC manual, "Bit Operation Builtins".
//! <https://gcc.gnu.org/onlinedocs/gcc/Bit-Operation-Builtins.html>
//! C23: §7.18.3-§7.18.16, pp. 305-311; PDF pp. 318-324.

use super::super::{
    FunctionLowerer,
    Item,
    LoweringError,
    Operand,
    types::Repr,
};
use crate::{
    builtins::GenericBit,
    ir::{
        IntCC,
        Opcode,
        Type,
        Value,
    },
    translation_phases::{
        SourceVectors,
        semantic_analysis::BuiltinCallFacts,
    },
};

/// The operand of a type-generic bit operation: an IR integer holding a
/// value of `width` bits, or the address of a wider `_BitInt`.
#[derive(Clone, Copy)]
enum Bits {
    Narrow { value: Value, width: u32 },
    Wide { address: Value, width: u32 },
}

impl Bits {
    const fn width(self) -> u32 {
        match self {
            | Self::Narrow { width, .. } | Self::Wide { width, .. } => width,
        }
    }
}

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    /// The type-generic bit operation `operation` on the evaluated
    /// operands `items`, as a value of the call's result type: the operand
    /// is converted to the operation type, whose width the counts are
    /// relative to, and a result of the operand's type converted back.
    /// GNU extension: GCC manual, "Bit Operation Builtins".
    /// C23: §7.18.3-§7.18.16, pp. 305-311; PDF pp. 318-324.
    pub(super) fn generic_bit(
        &mut self,
        operation: GenericBit,
        instance: &'tu BuiltinCallFacts<'tu>,
        items: &[Item],
        source: SourceVectors,
    ) -> Result<Operand, LoweringError> {
        use GenericBit as G;
        let ty = instance.operation_type.ok_or_else(|| {
            LoweringError::missing("a bit operation has no operation type", source)
        })?;
        let operand = self.convert_value(items[0], ty, source)?;
        let bits = if let Some((width, _)) = self.wide_bit_int(ty) {
            Bits::Wide {
                address: super::super::wide_bit_int::wide_address(operand, source)?,
                width,
            }
        } else {
            let width = match self.repr(ty, source)? {
                | Repr::Int { width, .. } => u32::from(width),
                | Repr::Bool => 1,
                | _ =>
                    return Err(LoweringError::missing(
                        "a bit operation's operand is no integer",
                        source,
                    )),
            };
            Bits::Narrow {
                value: self.value(operand, source)?,
                width,
            }
        };
        let width = bits.width();
        let second = items.get(1).copied();
        let int = |this: &mut Self, value: u32| this.iconst(Type::I32, i128::from(value));
        let value = match operation {
            | G::BitFloor | G::BitCeil | G::RotateLeft | G::RotateRight => {
                let rotation = match second {
                    | Some(count) if operation.rotate() =>
                        Some(self.rotation(count, width, source)?),
                    | _ => None,
                };
                let result = self.bit_value(operation, bits, rotation, ty, operand, source)?;
                let result = self.convert_value(result, instance.result_type, source)?;
                return Ok(result.operand);
            },
            | G::Popcount | G::CountOnes => self.ones(bits, source)?,
            | G::Parity => {
                let ones = self.ones(bits, source)?;
                let one = int(self, 1);
                self.plain(Opcode::And, ones, one)
            },
            | G::CountZeros => {
                let ones = self.ones(bits, source)?;
                let width = int(self, width);
                self.plain(Opcode::Isub, width, ones)
            },
            | G::HasSingleBit => {
                let ones = self.ones(bits, source)?;
                let one = int(self, 1);
                let single = self.icmp(IntCC::Eq, ones, one);
                return Ok(Operand::Value(self.extend(single, false, Type::I8)));
            },
            | G::Clz | G::Ctz => {
                let leading = operation == G::Clz;
                match second {
                    | Some(fallback) => {
                        let count = self.edge(bits, leading, false, source)?;
                        let zero = self.is_zero(bits, source)?;
                        let fallback = self.value(fallback, source)?;
                        self.choose(zero, fallback, count)
                    },
                    | None => self.undefined_at_zero(bits, leading, source)?,
                }
            },
            | G::LeadingZeros => self.edge(bits, true, false, source)?,
            | G::LeadingOnes => self.edge(bits, true, true, source)?,
            | G::TrailingZeros => self.edge(bits, false, false, source)?,
            | G::TrailingOnes => self.edge(bits, false, true, source)?,
            | G::FirstLeadingZero
            | G::FirstLeadingOne
            | G::FirstTrailingZero
            | G::FirstTrailingOne
            | G::Ffs => {
                let leading = matches!(operation, G::FirstLeadingZero | G::FirstLeadingOne);
                let ones = matches!(operation, G::FirstLeadingZero | G::FirstTrailingZero);
                // The index is past the run of the other bit, which fills
                // a value with none.
                let count = self.edge(bits, leading, ones, source)?;
                let full = int(self, width);
                let none = self.icmp(IntCC::Eq, count, full);
                let one = int(self, 1);
                let index = self.plain(Opcode::Iadd, count, one);
                let zero = int(self, 0);
                self.choose(none, zero, index)
            },
            | G::BitWidth => {
                let zeros = self.edge(bits, true, false, source)?;
                let width = int(self, width);
                self.plain(Opcode::Isub, width, zeros)
            },
            | G::Clrsb => self.redundant_sign_bits(bits, source)?,
        };
        Ok(Operand::Value(value))
    }

    /// The number of one bits, as an `i32`.
    fn ones(&mut self, bits: Bits, source: SourceVectors) -> Result<Value, LoweringError> {
        Ok(match bits {
            | Bits::Narrow { value, .. } => {
                let ty = self.draft.value_type(value);
                let count = self.convert(Opcode::Popcount, ty, value);
                self.extend(count, false, Type::I32)
            },
            | Bits::Wide { address, width } => self.wide_ones(address, width, source)?,
        })
    }

    /// The number of leading (else trailing) bits equal to `one`, as an
    /// `i32`: the width when every bit is. A narrow value's leading count
    /// on its IR type includes the bits above its width, which are zero
    /// (for ones, the bits below the width are flipped first); a trailing
    /// count stops at a one set at the width.
    fn edge(
        &mut self,
        bits: Bits,
        leading: bool,
        one: bool,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let Bits::Narrow { value, width } = bits else {
            let Bits::Wide { address, width } = bits else {
                unreachable!("a bit operand is narrow or wide")
            };
            return self.wide_edge(address, width, leading, one, source);
        };
        let ty = self.draft.value_type(value);
        let spare = ty.bits() - width;
        let value = if one {
            let mask = self.iconst(ty, (u128::MAX >> (128 - width)).cast_signed());
            self.plain(Opcode::Xor, value, mask)
        } else {
            value
        };
        Ok(if leading {
            let count = self.convert(Opcode::ClzDefined, ty, value);
            let count = self.extend(count, false, Type::I32);
            let spare = self.iconst(Type::I32, i128::from(spare));
            self.plain(Opcode::Isub, count, spare)
        } else {
            let value = if spare > 0 {
                let stop = self.iconst(ty, 1 << width);
                self.plain(Opcode::Or, value, stop)
            } else {
                value
            };
            let count = self.convert(Opcode::CtzDefined, ty, value);
            self.extend(count, false, Type::I32)
        })
    }

    /// `__builtin_clzg` or `__builtin_ctzg` without a result for zero, as
    /// `clz` or `ctz`, which a zero operand leaves undefined.
    fn undefined_at_zero(
        &mut self,
        bits: Bits,
        leading: bool,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let Bits::Narrow { value, width } = bits else {
            return self.edge(bits, leading, false, source);
        };
        let ty = self.draft.value_type(value);
        let opcode = if leading { Opcode::Clz } else { Opcode::Ctz };
        let count = self.convert(opcode, ty, value);
        let count = self.extend(count, false, Type::I32);
        Ok(if leading {
            let spare = self.iconst(Type::I32, i128::from(ty.bits() - width));
            self.plain(Opcode::Isub, count, spare)
        } else {
            count
        })
    }

    /// Whether the operand is zero, as an `i1`.
    fn is_zero(&mut self, bits: Bits, source: SourceVectors) -> Result<Value, LoweringError> {
        Ok(match bits {
            | Bits::Narrow { value, .. } => {
                let ty = self.draft.value_type(value);
                let zero = self.iconst(ty, 0);
                self.icmp(IntCC::Eq, value, zero)
            },
            | Bits::Wide { width, .. } => {
                let zeros = self.edge(bits, true, false, source)?;
                let width = self.iconst(Type::I32, i128::from(width));
                self.icmp(IntCC::Eq, zeros, width)
            },
        })
    }

    /// GCC's `__builtin_clrsbg`: the number of bits after the sign bit
    /// equal to it, of a signed value kept sign-extended, as an `i32`.
    fn redundant_sign_bits(
        &mut self,
        bits: Bits,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let Bits::Narrow { value, width } = bits else {
            let Bits::Wide { address, width } = bits else {
                unreachable!("a bit operand is narrow or wide")
            };
            return self.wide_redundant_sign_bits(address, width, source);
        };
        let ty = self.draft.value_type(value);
        let shift = self.iconst(ty, i128::from(ty.bits() - 1));
        let sign = self.plain(Opcode::Ashr, value, shift);
        let flipped = self.plain(Opcode::Xor, value, sign);
        let count = self.convert(Opcode::ClzDefined, ty, flipped);
        let count = self.extend(count, false, Type::I32);
        let excess = self.iconst(Type::I32, i128::from(ty.bits() - width + 1));
        Ok(self.plain(Opcode::Isub, count, excess))
    }

    /// `__builtin_stdc_bit_floor`, `bit_ceil` or a rotation by `rotation`
    /// (already reduced below the width, as an `i64`) of the operand, a
    /// value of the operation type `ty` held by `operand`.
    fn bit_value(
        &mut self,
        operation: GenericBit,
        bits: Bits,
        rotation: Option<Value>,
        ty: crate::translation_phases::semantic_analysis::TypeId,
        operand: Item,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        use GenericBit as G;
        let Bits::Narrow { value, width } = bits else {
            let result = self.wide_bit_value(operation, operand, rotation, source)?;
            return Ok(result);
        };
        let vty = self.draft.value_type(value);
        let limit = self.iconst(Type::I32, i128::from(width));
        let one = self.iconst(vty, 1);
        let zero = self.iconst(vty, 0);
        // A shift count masked to the IR type, for a power of two the
        // `choose` beside it discards when the count would reach the
        // width.
        let power = |this: &mut Self, exponent: Value| {
            let exponent = this.extend(exponent, false, vty);
            let mask = this.iconst(vty, i128::from(vty.bits() - 1));
            let exponent = this.plain(Opcode::And, exponent, mask);
            this.plain(Opcode::Shl, one, exponent)
        };
        let result = match operation {
            | G::BitFloor => {
                let zeros = self.edge(bits, true, false, source)?;
                let length = self.plain(Opcode::Isub, limit, zeros);
                let i32_one = self.iconst(Type::I32, 1);
                let exponent = self.plain(Opcode::Isub, length, i32_one);
                let floor = power(self, exponent);
                let is_zero = self.icmp(IntCC::Eq, value, zero);
                self.choose(is_zero, zero, floor)
            },
            | G::BitCeil => {
                // 1 for 0 and 1; else 2 to the width of value - 1, which
                // is 0 when that reaches the width.
                let below = self.plain(Opcode::Isub, value, one);
                let zeros = self.edge(
                    Bits::Narrow {
                        value: below,
                        width,
                    },
                    true,
                    false,
                    source,
                )?;
                let length = self.plain(Opcode::Isub, limit, zeros);
                let ceiling = power(self, length);
                let overflows = self.icmp(IntCC::Eq, length, limit);
                let ceiling = self.choose(overflows, zero, ceiling);
                let small = self.icmp(IntCC::Ule, value, one);
                self.choose(small, one, ceiling)
            },
            | _ => {
                let count = rotation
                    .ok_or_else(|| LoweringError::missing("a rotation has no count", source))?;
                let count = self.extend(count, false, vty);
                let width_value = self.iconst(vty, i128::from(width));
                let opposite = self.plain(Opcode::Isub, width_value, count);
                let unrotated = self.icmp(IntCC::Eq, count, zero);
                let opposite = self.choose(unrotated, zero, opposite);
                let (first, second) = if operation == G::RotateLeft {
                    (Opcode::Shl, Opcode::Lshr)
                } else {
                    (Opcode::Lshr, Opcode::Shl)
                };
                let near = self.plain(first, value, count);
                let far = self.plain(second, value, opposite);
                let rotated = self.plain(Opcode::Or, near, far);
                let mask = self.iconst(vty, (u128::MAX >> (128 - width)).cast_signed());
                self.plain(Opcode::And, rotated, mask)
            },
        };
        Ok(Item {
            operand: Operand::Value(result),
            ty,
        })
    }

    /// A rotation count reduced modulo `width` to `0..width`, a negative
    /// count turning the other way, as an `i64`: in the IR for a count of
    /// at most 128 bits, by the runtime for a wider one.
    /// GNU extension: GCC manual, "Bit Operation Builtins".
    fn rotation(
        &mut self,
        count: Item,
        width: u32,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        if self.wide_bit_int(count.ty).is_some() {
            return self.wide_residue(count, width, source);
        }
        let signed = self.repr(count.ty, source)?.signed();
        let value = self.value(count, source)?;
        let ty = if self.draft.value_type(value) == Type::I128 {
            Type::I128
        } else {
            Type::I64
        };
        let value = self.extend(value, signed, ty);
        let modulus = self.iconst(ty, i128::from(width));
        let remainder = self.plain(
            if signed { Opcode::Srem } else { Opcode::Urem },
            value,
            modulus,
        );
        let remainder = if signed {
            let zero = self.iconst(ty, 0);
            let negative = self.icmp(IntCC::Slt, remainder, zero);
            let raised = self.plain(Opcode::Iadd, remainder, modulus);
            self.choose(negative, raised, remainder)
        } else {
            remainder
        };
        Ok(self.extend(remainder, false, Type::I64))
    }
}
