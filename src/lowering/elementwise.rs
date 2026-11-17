//! Clang's operand-overloaded builtins that bcc's AVX2 headers call: the
//! elementwise `abs`, `min`, `max`, `add_sat`, `sub_sat` and `sqrt`, which
//! apply to a scalar or to each lane of a vector; the non-temporal load and
//! store, which are ordinary accesses here (the hint only steers caches);
//! and `__builtin_nondeterministic_value`, which may give any value and
//! here gives zero. Semantic analysis checked the operand types
//! (`semantic_analysis/vectors.rs`).
//!
//! Clang extension: Clang Language Extensions, "Vector Builtins" (each
//! builtin's table row) and "Non-temporal load/store builtins".
//! <https://clang.llvm.org/docs/LanguageExtensions.html#vector-builtins>

use super::{
    FunctionLowerer,
    LoweringError,
    ssa::Op,
    types::Repr,
    vectors::vector_address,
    work::{
        Item,
        Operand,
    },
    x86::int_of,
};
use crate::{
    builtins::{
        BuiltinId,
        FloatOperation,
    },
    ir::{
        FloatCC,
        IntCC,
        Intrinsic,
        Opcode,
        Type,
        Value,
    },
    translation_phases::{
        SourceVectors,
        semantic_analysis::TypeId,
    },
    util::bump::ArenaVec,
};

/// Whether `id` is one of these builtins.
pub(super) const fn expands(id: BuiltinId) -> bool {
    matches!(
        id,
        BuiltinId::ElementwiseAbs
            | BuiltinId::ElementwiseSqrt
            | BuiltinId::ElementwiseAddSat
            | BuiltinId::ElementwiseSubSat
            | BuiltinId::ElementwiseMin
            | BuiltinId::ElementwiseMax
            | BuiltinId::NondeterministicValue
            | BuiltinId::NontemporalLoad
            | BuiltinId::NontemporalStore
    )
}

impl FunctionLowerer<'_, '_, '_, '_, '_> {
    /// Expands builtin `id` on its operands `items`, giving a value of type
    /// `ty`.
    pub(super) fn elementwise(
        &mut self,
        id: BuiltinId,
        items: &[Item],
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        match id {
            | BuiltinId::NontemporalStore => {
                let pointer = self.value(items[1], source)?;
                self.copy_value(items[0], pointer, source)?;
                return Ok(Item {
                    operand: Operand::Void,
                    ty,
                });
            },
            | BuiltinId::NontemporalLoad => {
                let pointer = self.value(items[0], source)?;
                return self.load_value(pointer, ty, source);
            },
            | _ => {},
        }
        if let Some(shape) = self.vector(ty) {
            let object = self.vector_object(shape, source)?;
            let mut bases = [object; 2];
            for (base, &item) in bases.iter_mut().zip(items) {
                *base = vector_address(item, source)?;
            }
            for lane in 0..shape.count {
                let mut values = [object; 2];
                for (value, (&base, &item)) in values.iter_mut().zip(bases.iter().zip(items)) {
                    let operand = self.vector(item.ty).unwrap_or(shape);
                    *value = self.load_lane(base, operand, lane);
                }
                let value = self.element(id, shape.lane, &values[..items.len()], source)?;
                self.store_lane(value, object, shape, lane);
            }
            return Ok(Item {
                operand: Operand::Aggregate(object),
                ty,
            });
        }
        let repr = self.repr(ty, source)?;
        // x87 extended precision has no integer of its width to clear the
        // sign in, so it takes `fabsl`'s path through the sign byte in
        // memory.
        if id == BuiltinId::ElementwiseAbs && repr.value_type() == Some(Type::F80) {
            let value = self.floating_builtin(FloatOperation::Fabs, items, ty, source)?;
            return Ok(Item { operand: value, ty });
        }
        let mut values = ArenaVec::with_capacity_in(items.len(), self.scratch);
        for &item in items {
            values.push(self.value(item, source)?);
        }
        let value = self.element(id, repr, &values, source)?;
        Ok(Item {
            operand: Operand::Value(value),
            ty,
        })
    }

    /// One element of the result: of a scalar, or of a vector's lane of
    /// representation `repr`.
    fn element(
        &mut self,
        id: BuiltinId,
        repr: Repr,
        values: &[Value],
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let float = matches!(repr, Repr::Float { .. });
        let signed = repr.signed();
        let x = values[0];
        let ty = self.draft.value_type(x);
        Ok(match id {
            // The magnitude: an integer's negation wraps for its minimum, and
            // a floating value's sign bit is cleared, in an integer of its
            // width; a scalar x87 value went through memory (`elementwise`).
            | BuiltinId::ElementwiseAbs if float => {
                if ty == Type::F80 {
                    return Err(LoweringError::unsupported(
                        super::Construct::Builtin,
                        source,
                    ));
                }
                let int = int_of(u64::from(ty.bytes()));
                let bits = self.convert(Opcode::Bitcast, int, x);
                let mask = self.iconst(int, (1_i128 << (ty.bits() - 1)) - 1);
                let magnitude = self.bin(Opcode::And, bits, mask);
                self.convert(Opcode::Bitcast, ty, magnitude)
            },
            | BuiltinId::ElementwiseAbs => {
                let zero = self.iconst(ty, 0);
                let negated = self.bin(Opcode::Isub, zero, x);
                let negative = self.sign_set(x);
                self.choose(negative, negated, x)
            },
            // The smaller or larger; for floating operands, LLVM's `minnum`
            // and `maxnum`, which return the other operand of a NaN.
            | BuiltinId::ElementwiseMin | BuiltinId::ElementwiseMax => {
                let y = values[1];
                let min = id == BuiltinId::ElementwiseMin;
                if float {
                    let cc = if min { FloatCC::Olt } else { FloatCC::Ogt };
                    let second = self.fcmp(cc, y, x);
                    let ordered = self.choose(second, y, x);
                    let nan = self.fcmp(FloatCC::Uno, x, x);
                    self.choose(nan, y, ordered)
                } else {
                    let cc = match (min, signed) {
                        | (true, true) => IntCC::Slt,
                        | (true, false) => IntCC::Ult,
                        | (false, true) => IntCC::Sgt,
                        | (false, false) => IntCC::Ugt,
                    };
                    let first = self.icmp(cc, x, y);
                    self.choose(first, x, y)
                }
            },
            // The sum or difference clamped to the type's range. A signed
            // result wraps in the operands' width, which `__int128` has no
            // wider integer beyond: it overflowed when its sign differs from
            // that of both addends, or for a difference from the minuend's
            // while the operands' signs differ, and then saturates toward
            // the sign of `x`.
            | BuiltinId::ElementwiseAddSat | BuiltinId::ElementwiseSubSat => {
                let y = values[1];
                let add = id == BuiltinId::ElementwiseAddSat;
                if signed {
                    let total = self.bin(if add { Opcode::Iadd } else { Opcode::Isub }, x, y);
                    let changed = self.bin(Opcode::Xor, x, total);
                    let other = if add {
                        self.bin(Opcode::Xor, y, total)
                    } else {
                        self.bin(Opcode::Xor, x, y)
                    };
                    let overflow = self.bin(Opcode::And, changed, other);
                    let overflowed = self.sign_set(overflow);
                    let max = self.iconst(ty, i128::MAX >> (128 - ty.bits()));
                    let min = self.iconst(ty, !(i128::MAX >> (128 - ty.bits())));
                    let negative = self.sign_set(x);
                    let bound = self.choose(negative, min, max);
                    self.choose(overflowed, bound, total)
                } else if add {
                    let sum = self.bin(Opcode::Iadd, x, y);
                    let wrapped = self.icmp(IntCC::Ult, sum, x);
                    let max = self.iconst(ty, -1);
                    self.choose(wrapped, max, sum)
                } else {
                    let difference = self.bin(Opcode::Isub, x, y);
                    let below = self.icmp(IntCC::Ult, x, y);
                    let zero = self.iconst(ty, 0);
                    self.choose(below, zero, difference)
                }
            },
            // The correctly rounded square root.
            | BuiltinId::ElementwiseSqrt => {
                let intrinsic = match ty {
                    | Type::F32 => Intrinsic::SqrtF32,
                    | Type::F64 => Intrinsic::SqrtF64,
                    | _ =>
                        return Err(LoweringError::unsupported(
                            super::Construct::Builtin,
                            source,
                        )),
                };
                let args = self.scratch.alloc_slice_copy(&[x]);
                self.draft
                    .push(Op::Intrinsic(intrinsic, 0, args), Some(ty))
                    .expect("a square root has a result")
            },
            | _ => match ty {
                // Zero bits are positive zero in every floating format.
                | _ if ty.is_float() => self.fconst(ty, 0),
                | Type::Ptr => self.nullary(Opcode::Null),
                | _ => self.iconst(ty, 0),
            },
        })
    }

    /// Stores the value of `item` at `pointer`: a vector's bytes, or a
    /// scalar.
    fn copy_value(
        &mut self,
        item: Item,
        pointer: Value,
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        if let Some(shape) = self.vector(item.ty) {
            let from = vector_address(item, source)?;
            let lane = int_of(shape.size.min(8));
            for index in 0..shape.size / u64::from(lane.bytes()) {
                let value = self.lane(from, lane, index);
                self.set_lane(value, pointer, index);
            }
        } else {
            let value = self.value(item, source)?;
            self.set_lane(value, pointer, 0);
        }
        Ok(())
    }

    /// The value of type `ty` at `pointer`.
    fn load_value(
        &mut self,
        pointer: Value,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let operand = if let Some(shape) = self.vector(ty) {
            let object = self.vector_object(shape, source)?;
            let lane = int_of(shape.size.min(8));
            for index in 0..shape.size / u64::from(lane.bytes()) {
                let value = self.lane(pointer, lane, index);
                self.set_lane(value, object, index);
            }
            Operand::Aggregate(object)
        } else {
            let value_type = self.value_type(ty, source)?;
            Operand::Value(self.lane(pointer, value_type, 0))
        };
        Ok(Item { operand, ty })
    }
}
