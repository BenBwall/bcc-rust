//! Bit-precise integers of more than 128 bits. Such a `_BitInt(N)` has no
//! IR type: like a structure it lives in memory, as the x86-64 psABI lays it
//! out, ceil(N / 64) 64-bit limbs, least significant first, 8-aligned
//! ([`Repr::Aggregate`](super::types::Repr)). Its value is the address of
//! an object holding it, so objects, members, arrays, calls, results and
//! `va_arg` treat it as they treat a structure, and the calling conventions
//! classify it as the array of limbs it is (System V passes it in memory,
//! Microsoft x64 by reference, both return it through `sret`).
//!
//! An operation writes its result to a fresh object. Conversions to and
//! from integers of at most 128 bits load or build the low limbs inline;
//! everything else calls a routine of the runtime in `wide_bit_int/runtime.c`
//! with the operands' addresses, the width N and the signedness. Lowering
//! declares the routines it calls as internal functions without bodies, and
//! [`runtime::link`] gives them the bodies bcc compiles from that C source,
//! so every back end runs the same code and no program needs libgcc's
//! `_BitInt` routines (which GCC 13's libgcc lacks). A result is stored
//! with its padding bits extended from bit N, as GCC and Clang store them;
//! an operand's padding bits are not read.
//!
//! C23: §6.2.5 paragraphs 5-8, p. 39; PDF p. 52; §6.2.6.2 paragraphs 1-2,
//! pp. 42-43; PDF pp. 55-56; §6.3.1.3-§6.3.1.4, p. 43; PDF p. 55;
//! §6.5.5-§6.5.12, pp. 82-88; PDF pp. 94-100 (the operators, by C99's
//! clause numbers, which C23 keeps).

pub(crate) mod runtime;

use super::{
    Construct,
    FunctionLowerer,
    LoweringError,
    expressions::comparison,
    module_items::at,
    ssa::{
        Op,
        Terminator,
    },
    types::{
        self,
        Repr,
    },
    work::{
        BitField,
        Item,
        Operand,
        Place,
    },
};
use crate::{
    builtins::OverflowOperation,
    ir::{
        Align,
        Block,
        InstData,
        InstFlags,
        IntCC,
        Linkage,
        Opcode,
        Signature,
        Symbol,
        Type,
        Value,
    },
    translation_phases::{
        SourceVectors,
        parsing::syntax::{
            BinaryOperator,
            UnaryOperator,
        },
        semantic_analysis::{
            Integer,
            TypeId,
            TypeKind,
        },
    },
    util::bump::ArenaVec,
};

impl FunctionLowerer<'_, '_, '_, '_, '_> {
    /// The width and signedness of `ty`, after its qualifiers and
    /// atomicity, if it is a bit-precise integer type of more than 128 bits.
    pub(super) fn wide_bit_int(&self, ty: TypeId) -> Option<(u32, bool)> {
        wide_bit_int(&self.unit.sema.types, ty)
    }

    /// The binary operator `operator` with a wide operand or result, on
    /// converted operands, with result type `ty`: a comparison calls the
    /// runtime's three-way comparison, a shift takes its count's value,
    /// and an arithmetic or bitwise operator stores its result in a fresh
    /// object. A pointer offset by a wide integer and a narrow value
    /// shifted by a wide count take the count's low bits, which hold every
    /// defined count.
    /// C99: §6.5.5-§6.5.12, pp. 82-88; PDF pp. 94-100.
    pub(super) fn wide_binary(
        &mut self,
        operator: BinaryOperator,
        left: Item,
        right: Item,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        use BinaryOperator as B;
        let Some((width, signed)) = self.wide_bit_int(left.ty) else {
            // Only the right operand is wide: a shift count or an index,
            // whose low 64 bits hold every defined value. As a value it is
            // an unsigned integer to `binary`, which reads it no wider.
            let address = self.wide_address(right, source)?;
            let low = self.load_limbs_value(address, Type::I64);
            let right = Item {
                operand: Operand::Value(low),
                ty:      right.ty,
            };
            return self.binary(operator, left, right, ty, source);
        };
        if let Some(cc) = comparison(operator) {
            let a = self.wide_address(left, source)?;
            let b = self.wide_address(right, source)?;
            let bits = self.iconst(Type::I32, i128::from(width));
            let sign = self.iconst(Type::I32, i128::from(signed));
            let order = self.wide_call(Helper::Compare, &[a, b, bits, sign], source)?;
            let zero = self.iconst(Type::I32, 0);
            // The three-way result orders as the operands do.
            let cc = match cc {
                | IntCC::Ult => IntCC::Slt,
                | IntCC::Ule => IntCC::Sle,
                | IntCC::Ugt => IntCC::Sgt,
                | IntCC::Uge => IntCC::Sge,
                | cc => cc,
            };
            let truth = self.icmp(cc, order, zero);
            return Ok(Item {
                operand: Operand::Truth(truth),
                ty,
            });
        }
        let a = self.wide_address(left, source)?;
        let bits = self.iconst(Type::I32, i128::from(width));
        let sign = self.iconst(Type::I32, i128::from(signed));
        let result = self.wide_object(left.ty, source)?;
        if matches!(operator, B::LeftShift | B::RightShift) {
            // The count is promoted on its own (§6.5.7p3); a defined one is
            // below N, which a `u64` holds.
            let count = match self.wide_bit_int(right.ty) {
                | Some(_) => {
                    let address = self.wide_address(right, source)?;
                    self.load_limbs_value(address, Type::I64)
                },
                | None => {
                    let count_signed = self.repr(right.ty, source)?.signed();
                    let value = self.value(right, source)?;
                    self.extend(value, count_signed, Type::I64)
                },
            };
            let helper = if operator == B::LeftShift {
                Helper::ShiftLeft
            } else {
                Helper::ShiftRight
            };
            _ = self.wide_call(helper, &[result, a, count, bits, sign], source)?;
            return Ok(Item {
                operand: Operand::Aggregate(result),
                ty,
            });
        }
        let helper = match operator {
            | B::Addition => Helper::Add,
            | B::Subtraction => Helper::Subtract,
            | B::Multiplication => Helper::Multiply,
            | B::Division => Helper::Divide,
            | B::Modulo => Helper::Remainder,
            | B::BitwiseAnd => Helper::And,
            | B::BitwiseOr => Helper::Or,
            | B::BitwiseXor => Helper::Xor,
            | _ =>
                return Err(LoweringError::missing(
                    "a wide binary operator has no lowering",
                    source,
                )),
        };
        let b = self.wide_address(right, source)?;
        _ = self.wide_call(helper, &[result, a, b, bits, sign], source)?;
        Ok(Item {
            operand: Operand::Aggregate(result),
            ty,
        })
    }

    /// Unary `-` or `~` of a wide operand, into a fresh object.
    /// C99: §6.5.3.3 paragraphs 3-4, p. 79; PDF p. 91.
    pub(super) fn wide_unary(
        &mut self,
        operator: UnaryOperator,
        operand: Item,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let (width, signed) = self
            .wide_bit_int(ty)
            .ok_or_else(|| LoweringError::missing("a wide operand has no width", source))?;
        let helper = match operator {
            | UnaryOperator::Minus => Helper::Negate,
            | UnaryOperator::BitwiseNot => Helper::Not,
            | _ =>
                return Err(LoweringError::missing(
                    "a wide unary operator has no lowering",
                    source,
                )),
        };
        let a = self.wide_address(operand, source)?;
        let result = self.wide_object(ty, source)?;
        let bits = self.iconst(Type::I32, i128::from(width));
        let sign = self.iconst(Type::I32, i128::from(signed));
        _ = self.wide_call(helper, &[result, a, bits, sign], source)?;
        Ok(Item {
            operand: Operand::Aggregate(result),
            ty,
        })
    }

    /// The value one more or one less than the wide `item`, as `++` and
    /// `--` compute it, in a fresh object.
    /// C99: §6.5.2.4 paragraph 2, p. 75; PDF p. 87; §6.5.3.1 paragraph 2,
    /// p. 78; PDF p. 90.
    pub(super) fn wide_step(
        &mut self,
        item: Item,
        increment: bool,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let (width, signed) = self
            .wide_bit_int(item.ty)
            .ok_or_else(|| LoweringError::missing("a wide operand has no width", source))?;
        let one = self.wide_object(item.ty, source)?;
        let bits = self.iconst(Type::I32, i128::from(width));
        let sign = self.iconst(Type::I32, i128::from(signed));
        let low = self.iconst(Type::I64, 1);
        let high = self.iconst(Type::I64, 0);
        _ = self.wide_call(Helper::FromInteger, &[one, bits, sign, low, high], source)?;
        let a = self.wide_address(item, source)?;
        let result = self.wide_object(item.ty, source)?;
        let helper = if increment {
            Helper::Add
        } else {
            Helper::Subtract
        };
        _ = self.wide_call(helper, &[result, a, one, bits, sign], source)?;
        Ok(Item {
            operand: Operand::Aggregate(result),
            ty: item.ty,
        })
    }

    /// A copy of the wide value `item` in a fresh object, which later
    /// stores to the object `item` names do not change: the value of a
    /// postfix `++` or `--`.
    /// C99: §6.5.2.4 paragraph 2, p. 75; PDF p. 87.
    pub(super) fn wide_snapshot(
        &mut self,
        item: Item,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let from = self.wide_address(item, source)?;
        let layout =
            types::layout(&self.unit.sema.types, item.ty).map_err(|kind| at(kind, source))?;
        let copy = self.wide_object(item.ty, source)?;
        let size = self.iconst(Type::I64, i128::from(layout.size));
        _ = self.draft.push(
            self.memory_op(InstData::MemoryRange {
                opcode: Opcode::Copy,
                flags:  InstFlags::empty(),
                align:  Align::from_bytes(8).unwrap_or(Align::BYTE),
                args:   [copy, from, size],
            }),
            None,
        );
        Ok(Item {
            operand: Operand::Aggregate(copy),
            ty:      item.ty,
        })
    }

    /// Whether the wide value at `address`, of type `ty`, is nonzero, as an
    /// `i1`.
    /// C99: §6.3.1.2 paragraph 1, p. 43; PDF p. 55.
    pub(super) fn wide_truth(
        &mut self,
        address: Value,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let (width, _) = self
            .wide_bit_int(ty)
            .ok_or_else(|| LoweringError::missing("a wide operand has no width", source))?;
        let bits = self.iconst(Type::I32, i128::from(width));
        let nonzero = self.wide_call(Helper::Test, &[address, bits], source)?;
        let zero = self.iconst(Type::I32, 0);
        Ok(self.icmp(IntCC::Ne, nonzero, zero))
    }

    /// GCC's overflow builtins with a wide operand or destination: the
    /// operands, converted to a signed type wide enough for the exact
    /// result, are added, subtracted or multiplied there; the result is
    /// that value converted to `destination`, which overflowed unless it
    /// converts back to the exact value. Returns the result and the
    /// overflow as an `i1`.
    /// GNU extension: GCC manual, "Integer Overflow Builtins".
    pub(super) fn wide_overflow(
        &mut self,
        operation: OverflowOperation,
        a: Item,
        b: Item,
        destination: TypeId,
        source: SourceVectors,
    ) -> Result<(Item, Value), LoweringError> {
        let (a_width, a_signed) = self.integer_width(a.ty, source)?;
        let (b_width, b_signed) = self.integer_width(b.ty, source)?;
        let (to_width, to_signed) = self.integer_width(destination, source)?;
        // Magnitudes, then a sign bit: a sum or difference needs one bit
        // more than its wider operand, a product the sum of their bits.
        let a_bits = a_width + u32::from(!a_signed);
        let b_bits = b_width + u32::from(!b_signed);
        let exact = match operation {
            | OverflowOperation::Mul => a_bits + b_bits,
            | OverflowOperation::Add | OverflowOperation::Sub => a_bits.max(b_bits) + 1,
        }
        .max(to_width + 1)
        .max(129);
        let size = u64::from(exact.div_ceil(64)) * 8;
        let align = Align::from_bytes(8).unwrap_or(Align::BYTE);
        let left = self.fresh_object(size, align, source)?;
        self.exact_operand(left, exact, a, source)?;
        let right = self.fresh_object(size, align, source)?;
        self.exact_operand(right, exact, b, source)?;
        let value = self.fresh_object(size, align, source)?;
        let helper = match operation {
            | OverflowOperation::Add => Helper::Add,
            | OverflowOperation::Sub => Helper::Subtract,
            | OverflowOperation::Mul => Helper::Multiply,
        };
        let bits = self.iconst(Type::I32, i128::from(exact));
        let one = self.iconst(Type::I32, 1);
        _ = self.wide_call(helper, &[value, left, right, bits, one], source)?;
        let exact_item = |ty| Item {
            operand: Operand::Aggregate(value),
            ty,
        };
        // The result, then back to the exact width.
        let (result, back) = if self.wide_bit_int(destination).is_some() {
            let result = self.fresh_object(size.max(u64::from(to_width.div_ceil(64)) * 8), align, source)?;
            let args = [
                result,
                self.iconst(Type::I32, i128::from(to_width)),
                self.iconst(Type::I32, i128::from(to_signed)),
                value,
                bits,
                one,
            ];
            _ = self.wide_call(Helper::Convert, &args, source)?;
            let back = self.fresh_object(size, align, source)?;
            let args = [
                back,
                bits,
                one,
                result,
                self.iconst(Type::I32, i128::from(to_width)),
                self.iconst(Type::I32, i128::from(to_signed)),
            ];
            _ = self.wide_call(Helper::Convert, &args, source)?;
            (
                Item {
                    operand: Operand::Aggregate(result),
                    ty:      destination,
                },
                back,
            )
        } else {
            let target = self.repr(destination, source)?;
            let operand = self.narrow_from_wide(exact_item(a.ty), (exact, true), target, source)?;
            let result = Item {
                operand,
                ty: destination,
            };
            let back = self.fresh_object(size, align, source)?;
            self.exact_operand(back, exact, result, source)?;
            (result, back)
        };
        let order = self.wide_call(Helper::Compare, &[value, back, bits, one], source)?;
        let zero = self.iconst(Type::I32, 0);
        Ok((result, self.icmp(IntCC::Ne, order, zero)))
    }

    /// Stores the integer `item` at `address` as a signed integer of
    /// `width` bits, more than 128, which holds its value.
    fn exact_operand(
        &mut self,
        address: Value,
        width: u32,
        item: Item,
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        let bits = self.iconst(Type::I32, i128::from(width));
        let one = self.iconst(Type::I32, 1);
        if let Some((from_width, from_signed)) = self.wide_bit_int(item.ty) {
            let from = self.wide_address(item, source)?;
            let args = [
                address,
                bits,
                one,
                from,
                self.iconst(Type::I32, i128::from(from_width)),
                self.iconst(Type::I32, i128::from(from_signed)),
            ];
            _ = self.wide_call(Helper::Convert, &args, source)?;
            return Ok(());
        }
        let signed = self.repr(item.ty, source)?.signed();
        let value = self.value(item, source)?;
        let value = self.extend(value, signed, Type::I128);
        let low = self.convert(Opcode::Trunc, Type::I64, value);
        let shift = self.iconst(Type::I128, 64);
        let high = self.binary_inst(Opcode::Lshr, InstFlags::empty(), value, shift);
        let high = self.convert(Opcode::Trunc, Type::I64, high);
        _ = self.wide_call(Helper::FromInteger, &[address, bits, one, low, high], source)?;
        Ok(())
    }

    /// The width and signedness of the integer type `ty`.
    fn integer_width(&self, ty: TypeId, source: SourceVectors) -> Result<(u32, bool), LoweringError> {
        if let Some(wide) = self.wide_bit_int(ty) {
            return Ok(wide);
        }
        match self.repr(ty, source)? {
            | Repr::Int { signed, width, .. } => Ok((u32::from(width), signed)),
            | Repr::Bool => Ok((1, false)),
            | _ => Err(LoweringError::missing("an overflow operand is no integer", source)),
        }
    }

    /// The value of the bit-field `field` at `address`, whose declared
    /// type `ty` is wide, in a fresh object of that type.
    /// C99: §6.7.2.1 paragraph 9, p. 102; PDF p. 114.
    pub(super) fn wide_bit_field_read(
        &mut self,
        address: Value,
        field: BitField,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Operand, LoweringError> {
        let (width, signed) = self
            .wide_bit_int(ty)
            .ok_or_else(|| LoweringError::missing("a wide bit-field has no width", source))?;
        let result = self.wide_object(ty, source)?;
        let args = [
            result,
            self.iconst(Type::I32, i128::from(width)),
            self.iconst(Type::I32, i128::from(signed)),
            address,
            self.iconst(Type::I32, i128::from(field.shift)),
            self.iconst(Type::I32, i128::from(field.width)),
        ];
        _ = self.wide_call(Helper::FieldGet, &args, source)?;
        Ok(Operand::Aggregate(result))
    }

    /// Stores the wide `value` into the bit-field `field` at `address`,
    /// of declared type `ty`, and returns the field's new value.
    /// C99: §6.5.16.1 paragraph 2, p. 92; PDF p. 104.
    pub(super) fn wide_bit_field_write(
        &mut self,
        address: Value,
        field: BitField,
        value: Item,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Operand, LoweringError> {
        let from = self.wide_address(value, source)?;
        // The bytes the field shares with its neighbours are read and kept,
        // so each is frozen first, as narrow bit-field stores freeze theirs.
        for byte in [0, field.span - 1] {
            let at = self.offset_address(address, u64::from(byte));
            let old = self.inst(
                InstData::Load {
                    ty:    Type::I8,
                    flags: InstFlags::empty(),
                    align: Align::BYTE,
                    tag:   None,
                    addr:  at,
                },
                Type::I8,
            );
            let frozen = self.convert(Opcode::Freeze, Type::I8, old);
            _ = self.draft.push(
                self.memory_op(InstData::Store {
                    ty:    Type::I8,
                    flags: InstFlags::empty(),
                    align: Align::BYTE,
                    tag:   None,
                    args:  [frozen, at],
                }),
                None,
            );
        }
        let args = [
            address,
            self.iconst(Type::I32, i128::from(field.shift)),
            self.iconst(Type::I32, i128::from(field.width)),
            from,
        ];
        _ = self.wide_call(Helper::FieldSet, &args, source)?;
        self.wide_bit_field_read(address, field, ty, source)
    }

    /// Dispatches a `switch` on the wide value at `address`, of type `ty`:
    /// a comparison with each case's constant, or two with a GNU case
    /// range's bounds, in source order, and `default` when none matches.
    /// C99: §6.8.4.2 paragraphs 4-5, p. 134; PDF p. 146. GNU extension:
    /// <https://gcc.gnu.org/onlinedocs/gcc/Case-Ranges.html>.
    pub(super) fn wide_dispatch(
        &mut self,
        address: Value,
        ty: TypeId,
        cases: &[(Integer, Option<Integer>, Block)],
        default: Block,
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        let (width, signed) = self
            .wide_bit_int(ty)
            .ok_or_else(|| LoweringError::missing("a wide switch has no width", source))?;
        for &(lower, upper, target) in cases {
            let next = self.draft.create_block(&[]);
            let bounds = match upper {
                | Some(upper) => [(lower, IntCC::Sge), (upper, IntCC::Sle)],
                | None => [(lower, IntCC::Eq), (lower, IntCC::Eq)],
            };
            for (index, (bound, cc)) in bounds.into_iter().enumerate() {
                if index == 1 && upper.is_none() {
                    break;
                }
                let constant = self.wide_constant(bound, ty, source)?;
                let bits = self.iconst(Type::I32, i128::from(width));
                let sign = self.iconst(Type::I32, i128::from(signed));
                let order =
                    self.wide_call(Helper::Compare, &[address, constant, bits, sign], source)?;
                let zero = self.iconst(Type::I32, 0);
                let condition = self.icmp(cc, order, zero);
                if index == 0 && upper.is_some() {
                    let within = self.draft.create_block(&[]);
                    self.draft.brif(condition, within, next);
                    self.draft.seal(within);
                    self.draft.switch_to(within);
                } else {
                    self.draft.brif(condition, target, next);
                }
            }
            self.draft.seal(next);
            self.draft.switch_to(next);
        }
        let edge = self.draft.edge(default, &[]);
        self.draft
            .terminate_and_continue_dead(Terminator::Jump(edge));
        Ok(())
    }

    /// The folded wide constant `value` of type `ty`: the address of an
    /// internal constant object holding its limbs, which nothing writes.
    /// C99: §6.6 paragraph 6, p. 95; PDF p. 107.
    pub(super) fn wide_constant(
        &mut self,
        value: Integer,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let layout = types::layout(&self.unit.sema.types, ty).map_err(|kind| at(kind, source))?;
        let count = usize::try_from(layout.size / 8)
            .map_err(|_| LoweringError::missing("a wide constant has no size", source))?;
        let mut limbs = ArenaVec::with_capacity_in(count, self.scratch);
        limbs.resize(count, 0_u64);
        self.unit.sema.integer_limbs(value, &mut limbs);
        if let Some((width, signed)) = self.wide_bit_int(ty) {
            extend_limbs(&mut limbs, width, signed);
        }
        let mut bytes = ArenaVec::with_capacity_in(count * 8, self.scratch);
        for limb in &limbs {
            bytes.extend_from_slice(&limb.to_le_bytes());
        }
        let global = self.unit.internal_constant(".bitint", &bytes, 8);
        Ok(self.inst(InstData::GlobalAddr { global }, Type::Ptr))
    }

    /// `item` converted to the type `to`, either of which is a wide
    /// bit-precise type: between wide types the runtime truncates or
    /// extends by the source's signedness; a narrower integer, `_Bool`
    /// or pointer is the low limbs of a wide value, and becomes one
    /// extended by its signedness; a floating value converts in the
    /// runtime, truncating toward zero one way and rounding to nearest the
    /// other, a 16-bit type through `float`, which holds every integer
    /// binary16 does not overflow on and every 16-bit value exactly.
    /// C99: §6.3.1.2-§6.3.1.4, pp. 43-44; PDF pp. 55-56; §6.3.2.3
    /// paragraphs 5-6, p. 47; PDF p. 59.
    pub(super) fn wide_convert(
        &mut self,
        item: Item,
        to: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let target = self.repr(to, source)?;
        if target == Repr::Void {
            return Ok(Item {
                operand: Operand::Void,
                ty:      to,
            });
        }
        let from = self.wide_bit_int(item.ty);
        let into = self.wide_bit_int(to);
        let operand = match (from, into) {
            | (Some(from), Some(into)) if from == into => item.operand,
            | (Some((from_width, from_signed)), Some((width, signed))) => {
                let a = self.wide_address(item, source)?;
                let result = self.wide_object(to, source)?;
                let args = [
                    result,
                    self.iconst(Type::I32, i128::from(width)),
                    self.iconst(Type::I32, i128::from(signed)),
                    a,
                    self.iconst(Type::I32, i128::from(from_width)),
                    self.iconst(Type::I32, i128::from(from_signed)),
                ];
                _ = self.wide_call(Helper::Convert, &args, source)?;
                Operand::Aggregate(result)
            },
            | (Some((width, signed)), None) =>
                self.narrow_from_wide(item, (width, signed), target, source)?,
            | (None, Some((width, signed))) => {
                let result = self.wide_object(to, source)?;
                let bits = self.iconst(Type::I32, i128::from(width));
                let sign = self.iconst(Type::I32, i128::from(signed));
                let from = self.repr(item.ty, source)?;
                match from {
                    | Repr::Float { ty } => {
                        let (value, ty) = if ty.half_format().is_some() {
                            let value = self.value(item, source)?;
                            (self.widen_half(value), Type::F32)
                        } else {
                            (self.value(item, source)?, ty)
                        };
                        _ = self.wide_call(
                            Helper::FromFloat(ty),
                            &[result, bits, sign, value],
                            source,
                        )?;
                    },
                    | Repr::Int { .. } | Repr::Bool | Repr::Pointer => {
                        let value = self.value(item, source)?;
                        let value = if from == Repr::Pointer {
                            self.convert(Opcode::Ptrtoint, Type::I64, value)
                        } else {
                            value
                        };
                        let value = self.extend(value, from.signed(), Type::I128);
                        let low = self.convert(Opcode::Trunc, Type::I64, value);
                        let shift = self.iconst(Type::I128, 64);
                        let high = self.binary_inst(Opcode::Lshr, InstFlags::empty(), value, shift);
                        let high = self.convert(Opcode::Trunc, Type::I64, high);
                        _ = self.wide_call(
                            Helper::FromInteger,
                            &[result, bits, sign, low, high],
                            source,
                        )?;
                    },
                    | _ =>
                        return Err(LoweringError::unsupported(
                            Construct::WideBitIntConversion,
                            source,
                        )),
                }
                Operand::Aggregate(result)
            },
            | (None, None) =>
                return Err(LoweringError::missing(
                    "a wide conversion has no wide type",
                    source,
                )),
        };
        Ok(Item { operand, ty: to })
    }

    /// The wide `item` of `width` and signedness converted to the narrower
    /// type `to`, whose representation is `target`.
    fn narrow_from_wide(
        &mut self,
        item: Item,
        (width, signed): (u32, bool),
        target: Repr,
        source: SourceVectors,
    ) -> Result<Operand, LoweringError> {
        let address = self.wide_address(item, source)?;
        Ok(match target {
            | Repr::Bool => {
                let truth = self.wide_truth(address, item.ty, source)?;
                Operand::Value(self.extend(truth, false, Type::I8))
            },
            // The low bits of the value, extended again from a narrow
            // bit-precise destination's width (§6.3.1.3).
            | Repr::Int { ty, .. } => {
                let load = if ty == Type::I128 {
                    Type::I128
                } else {
                    Type::I64
                };
                let low = self.load_limbs_value(address, load);
                let low = if ty == load {
                    low
                } else {
                    self.convert(Opcode::Trunc, ty, low)
                };
                Operand::Value(self.normalize(low, target))
            },
            | Repr::Pointer => {
                let low = self.load_limbs_value(address, Type::I64);
                Operand::Value(self.convert(Opcode::Inttoptr, Type::Ptr, low))
            },
            | Repr::Float { ty } => {
                let compute = if ty.half_format().is_some() {
                    Type::F32
                } else {
                    ty
                };
                let bits = self.iconst(Type::I32, i128::from(width));
                let sign = self.iconst(Type::I32, i128::from(signed));
                let value = self.wide_call(Helper::ToFloat(compute), &[address, bits, sign], source)?;
                Operand::Value(if compute == ty {
                    value
                } else {
                    self.convert(Opcode::Fptrunc, ty, value)
                })
            },
            | _ =>
                return Err(LoweringError::unsupported(
                    Construct::WideBitIntConversion,
                    source,
                )),
        })
    }

    /// The `ty` value in the low bits of the limbs at `address`.
    fn load_limbs_value(&mut self, address: Value, ty: Type) -> Value {
        self.inst(
            InstData::Load {
                ty,
                flags: InstFlags::empty(),
                align: Align::from_bytes(8).unwrap_or(Align::BYTE),
                tag: None,
                addr: address,
            },
            ty,
        )
    }

    /// The address of the object holding the wide value `item`.
    fn wide_address(&mut self, item: Item, source: SourceVectors) -> Result<Value, LoweringError> {
        match item.operand {
            | Operand::Aggregate(address) | Operand::Place(Place::Memory(address)) => Ok(address),
            | _ => Err(LoweringError::missing("a wide value has no address", source)),
        }
    }

    /// A fresh object of the wide type `ty`.
    fn wide_object(&mut self, ty: TypeId, source: SourceVectors) -> Result<Value, LoweringError> {
        let layout = types::layout(&self.unit.sema.types, ty).map_err(|kind| at(kind, source))?;
        self.fresh_object(layout.size, Align::from_bytes(8).unwrap_or(Align::BYTE), source)
    }

    /// Calls the runtime routine `helper` with `args`, declaring it on its
    /// first use, and returns its result.
    fn wide_call(
        &mut self,
        helper: Helper,
        args: &[Value],
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let name = helper.name(source)?;
        let function = match self.unit.module.symbol(name) {
            | Some(Symbol::Function(function)) => function,
            | Some(_) =>
                return Err(LoweringError::missing(
                    "a _BitInt runtime symbol collides",
                    source,
                )),
            | None => {
                let signature = Signature {
                    params:           helper.params(),
                    aggregates:       &[],
                    noundef:          &[],
                    result:           helper.result(),
                    variadic:         false,
                    variadic_fixed:   None,
                    extensions:       &[],
                    result_extension: None,
                    result_noundef:   false,
                    variadic_noundef: false,
                };
                let signature = self.unit.module.intern(signature);
                self.unit
                    .module
                    .declare_function(name, signature, Linkage::Internal)
            },
        };
        let args = self.scratch.alloc_slice_copy(args);
        let result = self.draft.push(Op::Call(function, args), helper.result());
        Ok(result.unwrap_or_else(|| self.iconst(Type::I32, 0)))
    }

}

/// The width and signedness of `ty`, after its qualifiers and atomicity,
/// if it is a bit-precise integer type of more than 128 bits.
pub(super) fn wide_bit_int(
    types: &crate::translation_phases::semantic_analysis::Types<'_>,
    ty: TypeId,
) -> Option<(u32, bool)> {
    match types.kind(types.non_atomic(ty)) {
        | TypeKind::BitInt { width, signed } if width > 128 => Some((width, signed)),
        | _ => None,
    }
}

/// Extends the top limb of `limbs`, a value of `width` bits, from that
/// width by its signedness: the value converted to the type (§6.3.1.3), as
/// GCC and Clang store it.
/// C23: §6.2.6.2 paragraphs 1-2, pp. 42-43; PDF pp. 55-56.
pub(super) fn extend_limbs(limbs: &mut [u64], width: u32, signed: bool) {
    let spare = (64 - width % 64) % 64;
    if let Some(top) = limbs.last_mut()
        && spare > 0
    {
        *top = if signed {
            ((*top << spare).cast_signed() >> spare).cast_unsigned()
        } else {
            (*top << spare) >> spare
        };
    }
}

/// A routine of the runtime (`wide_bit_int/runtime.c`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Helper {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    And,
    Or,
    Xor,
    Not,
    Negate,
    ShiftLeft,
    ShiftRight,
    Compare,
    Test,
    Convert,
    FromInteger,
    FieldGet,
    FieldSet,
    ToFloat(Type),
    FromFloat(Type),
}

impl Helper {
    fn name(self, source: SourceVectors) -> Result<&'static str, LoweringError> {
        Ok(match self {
            | Self::Add => "__bcc_bitint_add",
            | Self::Subtract => "__bcc_bitint_sub",
            | Self::Multiply => "__bcc_bitint_mul",
            | Self::Divide => "__bcc_bitint_div",
            | Self::Remainder => "__bcc_bitint_rem",
            | Self::And => "__bcc_bitint_and",
            | Self::Or => "__bcc_bitint_or",
            | Self::Xor => "__bcc_bitint_xor",
            | Self::Not => "__bcc_bitint_not",
            | Self::Negate => "__bcc_bitint_neg",
            | Self::ShiftLeft => "__bcc_bitint_shl",
            | Self::ShiftRight => "__bcc_bitint_shr",
            | Self::Compare => "__bcc_bitint_cmp",
            | Self::Test => "__bcc_bitint_test",
            | Self::Convert => "__bcc_bitint_convert",
            | Self::FromInteger => "__bcc_bitint_from_int",
            | Self::FieldGet => "__bcc_bitint_field_get",
            | Self::FieldSet => "__bcc_bitint_field_set",
            | Self::ToFloat(Type::F32) => "__bcc_bitint_to_f32",
            | Self::ToFloat(Type::F64) => "__bcc_bitint_to_f64",
            | Self::ToFloat(Type::F80) => "__bcc_bitint_to_f80",
            | Self::ToFloat(Type::F128) => "__bcc_bitint_to_f128",
            | Self::FromFloat(Type::F32) => "__bcc_bitint_from_f32",
            | Self::FromFloat(Type::F64) => "__bcc_bitint_from_f64",
            | Self::FromFloat(Type::F80) => "__bcc_bitint_from_f80",
            | Self::FromFloat(Type::F128) => "__bcc_bitint_from_f128",
            | Self::ToFloat(_) | Self::FromFloat(_) =>
                return Err(LoweringError::unsupported(
                    Construct::WideBitIntConversion,
                    source,
                )),
        })
    }

    /// The pre-ABI parameter types; each matches the C prototype in
    /// `runtime.c` (`unsigned` and `int` are `i32`, a limb `i64`).
    fn params(self) -> &'static [Type] {
        use Type::{
            F32,
            F64,
            F80,
            F128,
            I32,
            I64,
            Ptr,
        };
        match self {
            | Self::Add
            | Self::Subtract
            | Self::Multiply
            | Self::Divide
            | Self::Remainder
            | Self::And
            | Self::Or
            | Self::Xor => &[Ptr, Ptr, Ptr, I32, I32],
            | Self::Not | Self::Negate | Self::Compare => &[Ptr, Ptr, I32, I32],
            | Self::ShiftLeft | Self::ShiftRight => &[Ptr, Ptr, I64, I32, I32],
            | Self::Test => &[Ptr, I32],
            | Self::Convert => &[Ptr, I32, I32, Ptr, I32, I32],
            | Self::FromInteger => &[Ptr, I32, I32, I64, I64],
            | Self::FieldGet => &[Ptr, I32, I32, Ptr, I32, I32],
            | Self::FieldSet => &[Ptr, I32, I32, Ptr],
            | Self::ToFloat(_) => &[Ptr, I32, I32],
            | Self::FromFloat(F32) => &[Ptr, I32, I32, F32],
            | Self::FromFloat(F64) => &[Ptr, I32, I32, F64],
            | Self::FromFloat(F80) => &[Ptr, I32, I32, F80],
            | Self::FromFloat(_) => &[Ptr, I32, I32, F128],
        }
    }

    fn result(self) -> Option<Type> {
        match self {
            | Self::Compare | Self::Test => Some(Type::I32),
            | Self::ToFloat(ty) => Some(ty),
            | _ => None,
        }
    }
}
