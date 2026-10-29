//! Bit-fields: member reads and writes as loads, shifts, masks and stores of
//! the bytes that hold the field.
//!
//! Semantic analysis gives each bit-field a byte offset, a bit offset below
//! eight and a width, so the field touches the `span` bytes from that offset
//! that its bits reach. A read loads exactly those bytes, in naturally sized
//! pieces (three bytes are an `i16` and an `i8`), joins them into one
//! integer and extracts the field: for a signed field a left shift that puts
//! the field's top bit in the integer's, then an arithmetic right shift; for
//! an unsigned field or `_Bool`, a logical right shift and a mask.
//!
//! A write replaces the field's bits and keeps the other bits of the bytes it
//! shares. A byte the field covers only in part is loaded, frozen, masked,
//! merged with the new bits and stored; bytes it covers entirely are stored
//! without a load, in naturally sized pieces. The accesses never leave the
//! field's own bytes, because the IR makes a load poison as a whole if any
//! of its bytes was never written: a wider access spanning a byte that a
//! neighbor wrote and a byte nobody wrote yet would freeze to an arbitrary
//! value and destroy the neighbor's bits. `freeze` stands for the
//! indeterminate bits of an unwritten byte, which C leaves unspecified
//! (§6.2.4p5). A `volatile` field is accessed through volatile loads and
//! stores. The value of an assignment is the field's value after the store,
//! so `(s.f = 5)` for a three-bit signed field is -3 (§6.5.16p3).
//!
//! C99: §6.7.2.1 paragraphs 3-4 and 10-11, pp. 101-102; PDF pp. 113-114.
//! §6.5.16 paragraph 3, p. 91; PDF p. 103.
//! GNU extension: GCC Common Attributes, packed permits straddling fields.
//! <https://gcc.gnu.org/onlinedocs/gcc/Common-Attributes.html>

use super::{
    FunctionLowerer,
    LoweringError,
    module_items::at,
    types::{
        self,
        Repr,
    },
    work::{
        BitField,
        BitFieldBits,
    },
};
use crate::{
    ir::{
        Align,
        InstData,
        InstFlags,
        Opcode,
        Type,
        Value,
    },
    translation_phases::{
        SourceVectors,
        semantic_analysis::TypeId,
    },
};

impl FunctionLowerer<'_, '_, '_, '_, '_> {
    /// Where a bit-field of declared type `ty` is, given its member's byte
    /// `offset` in the structure and its `bits`. The type's alignment is the
    /// alignment of the structure, which the offset refines for the field's
    /// own address.
    pub(super) fn bit_field(
        &self,
        ty: TypeId,
        offset: u64,
        bits: BitFieldBits,
        source: SourceVectors,
    ) -> Result<BitField, LoweringError> {
        let types = &self.unit.sema.types;
        let representation = self.repr(ty, source)?;
        let wide = self.wide_bit_int(ty);
        if !matches!(representation, Repr::Int { .. } | Repr::Bool) && wide.is_none() {
            return Err(LoweringError::missing(
                "a bit-field is not an integer",
                source,
            ));
        }
        let declared = types::layout(types, ty).map_err(|kind| at(kind, source))?;
        // The width fits the declared type; GNU packed placement may make
        // its storage span wider, up to 135 bits for a 128-bit field.
        if bits.width == 0
            || u64::from(bits.width) > declared.size * 8
            || (bits.width > 128 && wide.is_none())
            || bits.bit_offset >= 8
        {
            return Err(LoweringError::missing(
                "a bit-field does not fit its declared type",
                source,
            ));
        }
        let end = bits.bit_offset + bits.width;
        let align = super::alignment::at_offset(types::align(declared), u128::from(offset));
        Ok(BitField {
            span: end.div_ceil(8),
            shift: bits.bit_offset,
            width: bits.width,
            signed: wide.map_or(representation.signed(), |(_, signed)| signed),
            align,
        })
    }

    /// The value of the bit-field at `address`, as the declared type
    /// `result`.
    pub(super) fn load_bit_field(
        &mut self,
        address: Value,
        field: BitField,
        volatile: bool,
        result: Type,
    ) -> Value {
        if field.shift + field.width > 128 {
            return self.load_spanning_bit_field(address, field, volatile, result);
        }
        let working = working_type(field);
        let mut joined: Option<Value> = None;
        let mut position = 0;
        while position < field.span {
            let size = piece(field.span - position);
            let piece_type = types::int_type(size * 8);
            let at_piece = self.offset_address(address, u64::from(position));
            let mut part = self.load_piece(at_piece, piece_type, field, position, volatile);
            if working != piece_type {
                part = self.extend(part, false, working);
            }
            if position != 0 {
                part = self.shift_by(Opcode::Shl, part, position * 8);
            }
            joined = Some(match joined {
                | None => part,
                | Some(low) => self.binary_inst(Opcode::Or, InstFlags::empty(), low, part),
            });
            position += size;
        }
        let joined = joined.expect("a bit-field spans a byte");
        self.extract_bit_field(joined, field, result)
    }

    /// Assemble a packed field that touches seventeen bytes into its 128-bit
    /// value. Shift each byte into value coordinates before joining, so the
    /// neighboring bits outside the value never require a wider IR integer.
    /// C99: §6.7.2.1p10-11, p. 102; PDF p. 114 (implementation choice).
    fn load_spanning_bit_field(
        &mut self,
        address: Value,
        field: BitField,
        volatile: bool,
        result: Type,
    ) -> Value {
        let mut joined = self.iconst(Type::I128, 0);
        for position in 0..field.span {
            let at_byte = self.offset_address(address, u64::from(position));
            let byte = self.load_piece(at_byte, Type::I8, field, position, volatile);
            let mut part = self.extend(byte, false, Type::I128);
            part = if position == 0 {
                self.shift_by(Opcode::Lshr, part, field.shift)
            } else {
                self.shift_by(Opcode::Shl, part, position * 8 - field.shift)
            };
            joined = self.binary_inst(Opcode::Or, InstFlags::empty(), joined, part);
        }
        self.extract_bit_field(joined, BitField { shift: 0, ..field }, result)
    }

    /// The value of the field in `joined`, the integer its bytes make, as
    /// the declared type `result`.
    fn extract_bit_field(&mut self, joined: Value, field: BitField, result: Type) -> Value {
        let working_bits = self.draft.value_type(joined).bits();
        let mut value = joined;
        if field.signed {
            // Put the field's top bit in the integer's, then shift it back
            // down replicating that bit.
            let left = working_bits - field.shift - field.width;
            if left != 0 {
                value = self.shift_by(Opcode::Shl, value, left);
            }
            let right = working_bits - field.width;
            if right != 0 {
                value = self.shift_by(Opcode::Ashr, value, right);
            }
        } else {
            if field.shift != 0 {
                value = self.shift_by(Opcode::Lshr, value, field.shift);
            }
            if field.shift + field.width < field.span * 8 {
                value = self.mask_low(value, field.width);
            }
        }
        self.extend(value, field.signed, result)
    }

    /// Stores `value`, of the field's declared type, into the bit-field at
    /// `address`, keeping the other bits of the bytes it shares. Returns the
    /// field's value as it now reads, in the declared type `result`.
    pub(super) fn store_bit_field(
        &mut self,
        address: Value,
        field: BitField,
        volatile: bool,
        value: Value,
        result: Type,
    ) -> Value {
        let working = working_type(field);
        let narrowed = self.extend(value, false, working);
        let low = if field.width == working.bits() {
            narrowed
        } else {
            self.mask_low(narrowed, field.width)
        };
        let placed = if field.shift == 0 {
            low
        } else {
            self.shift_by(Opcode::Shl, low, field.shift)
        };
        let end = field.shift + field.width;
        let mut position = 0;
        while position < field.span {
            let covered = byte_mask(position, field.shift, end);
            if covered == 0xFF {
                // Whole bytes are stored without reading them.
                let mut run = 1;
                while position + run < field.span
                    && byte_mask(position + run, field.shift, end) == 0xFF
                {
                    run += 1;
                }
                let stop = position + run;
                while position < stop {
                    let size = piece(stop - position);
                    let piece_type = types::int_type(size * 8);
                    let part = self.field_piece(low, placed, field, position, piece_type);
                    let at_piece = self.offset_address(address, u64::from(position));
                    self.store_piece(at_piece, part, field, position, volatile);
                    position += size;
                }
            } else {
                // A partly covered byte keeps the bits the field does not
                // own.
                let part = self.field_piece(low, placed, field, position, Type::I8);
                let at_byte = self.offset_address(address, u64::from(position));
                let old = self.load_piece(at_byte, Type::I8, field, position, volatile);
                let old = self.convert(Opcode::Freeze, Type::I8, old);
                let keep = self.iconst(Type::I8, i128::from(!covered));
                let kept = self.binary_inst(Opcode::And, InstFlags::empty(), old, keep);
                let merged = self.binary_inst(Opcode::Or, InstFlags::empty(), kept, part);
                self.store_piece(at_byte, merged, field, position, volatile);
                position += 1;
            }
        }
        // The new value of the field is the low bits written, read back as
        // the field reads: an unsigned field is already masked.
        if field.signed {
            let readable = BitField { shift: 0, ..field };
            self.extract_bit_field(low, readable, result)
        } else {
            self.extend(low, false, result)
        }
    }

    /// A storage piece of the field. A seventeen-byte span is shifted from
    /// the value directly, avoiding loss of its high bits in `placed`.
    fn field_piece(
        &mut self,
        low: Value,
        placed: Value,
        field: BitField,
        position: u32,
        piece_type: Type,
    ) -> Value {
        if field.shift + field.width > 128 && position != 0 {
            let part = self.shift_by(Opcode::Lshr, low, position * 8 - field.shift);
            self.extend(part, false, piece_type)
        } else {
            self.piece_of(placed, position, piece_type)
        }
    }

    /// The `piece_type`-wide part of `placed` that begins at byte
    /// `position`.
    fn piece_of(&mut self, placed: Value, position: u32, piece_type: Type) -> Value {
        let mut part = placed;
        if position != 0 {
            part = self.shift_by(Opcode::Lshr, part, position * 8);
        }
        self.extend(part, false, piece_type)
    }

    fn load_piece(
        &mut self,
        address: Value,
        piece_type: Type,
        field: BitField,
        position: u32,
        volatile: bool,
    ) -> Value {
        self.inst(
            InstData::Load {
                ty:    piece_type,
                flags: access_flags(volatile),
                align: piece_align(field, position),
                tag:   None,
                addr:  address,
            },
            piece_type,
        )
    }

    fn store_piece(
        &mut self,
        address: Value,
        value: Value,
        field: BitField,
        position: u32,
        volatile: bool,
    ) {
        let ty = self.draft.value_type(value);
        _ = self.draft.push(
            self.memory_op(InstData::Store {
                ty,
                flags: access_flags(volatile),
                align: piece_align(field, position),
                tag: None,
                args: [value, address],
            }),
            None,
        );
    }

    fn shift_by(&mut self, opcode: Opcode, value: Value, count: u32) -> Value {
        let ty = self.draft.value_type(value);
        let count = self.iconst(ty, i128::from(count));
        self.binary_inst(opcode, InstFlags::empty(), value, count)
    }

    /// `value` with all but its low `width` bits cleared.
    fn mask_low(&mut self, value: Value, width: u32) -> Value {
        let ty = self.draft.value_type(value);
        let mask = self.iconst(ty, (u128::MAX >> (128 - width)) as i128);
        self.binary_inst(Opcode::And, InstFlags::empty(), value, mask)
    }
}

/// The integer type that holds all the bytes of a field.
fn working_type(field: BitField) -> Type {
    types::int_type(field.span.min(16).next_power_of_two() * 8)
}

/// The size in bytes of the largest natural access within `remaining`
/// bytes.
fn piece(remaining: u32) -> u32 {
    1 << remaining.min(16).ilog2()
}

/// The bits of byte `position` of a field that spans bits `shift..end` of
/// its bytes.
fn byte_mask(position: u32, shift: u32, end: u32) -> u8 {
    let low = shift.saturating_sub(position * 8).min(8);
    let high = end.saturating_sub(position * 8).min(8);
    let ones = (1_u16 << high) - (1_u16 << low);
    u8::try_from(ones).unwrap_or(u8::MAX)
}

fn access_flags(volatile: bool) -> InstFlags {
    if volatile {
        InstFlags::VOLATILE
    } else {
        InstFlags::empty()
    }
}

/// The opaque-pointer default at `position` bytes into the field's place.
/// Known backing storage overrides this default at the memory boundary.
fn piece_align(field: BitField, position: u32) -> Align {
    super::alignment::at_offset(field.align, u128::from(position))
}
