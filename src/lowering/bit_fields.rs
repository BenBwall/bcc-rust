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
//! GCC's `-fstrict-volatile-bitfields` asks that a `volatile` field be read
//! and written with one access of its declared type's width, for
//! memory-mapped registers that answer only to that width. Such an access,
//! a [`VolatileUnit`], holds the bytes of the field's neighbors too. It is
//! sound because the IR makes a volatile load never poison: it reads the
//! bytes as they are in memory, an unwritten byte is arbitrary and every
//! other byte keeps its value (`middle-end.md`, "Undefined behaviour").
//! `freeze` could not do this, since it replaces all the bits of a load that
//! is poison as a whole, the neighbors' and the field's own. A flag on these
//! loads alone would give every pass and back end one more form to carry, to
//! keep poison for the other volatile loads, where nothing relies on it: no
//! pass reasons about what a volatile load returns. So a read loads the unit
//! and extracts the field, and a write loads the unit, without a `freeze`,
//! merges the field's bits and stores it, or stores it without a load when
//! the field fills it.
//!
//! GCC's rule (`strict_volatile_bitfield_p` in its `expmed.cc`, checked by
//! compiling with GCC 13.3 and 14.2) has no case for a field whose unit
//! would not work; the access then is an ordinary one, here the field's own
//! bytes. [`FunctionLowerer::volatile_unit`] lists the conditions. A write
//! obeys one more than a read: it may not touch what lies outside the field's
//! memory location (C11 §3.14), so `struct { volatile int a : 8; char c; }`
//! reads `a` with 32 bits and writes it with 8.
//!
//! C99: §6.7.2.1 paragraphs 3-4 and 10-11, pp. 101-102; PDF pp. 113-114.
//! §6.5.16 paragraph 3, p. 91; PDF p. 103.
//! C99: §6.7.3 paragraph 6, p. 109; PDF p. 121 (what an access to a volatile
//! object is, implementation-defined).
//! GNU extension: GCC Common Attributes, packed permits straddling fields.
//! <https://gcc.gnu.org/onlinedocs/gcc/Common-Attributes.html>
//! GNU extension: GCC Code Gen Options, `-fstrict-volatile-bitfields`.
//! <https://gcc.gnu.org/onlinedocs/gcc/Code-Gen-Options.html>

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
        VolatileUnit,
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
        semantic_analysis::{
            Field,
            Member,
            TagKind,
            TypeId,
            TypeKind,
            Types,
        },
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
            volatile_unit: None,
        })
    }

    /// The access unit of the `volatile` bit-field `field` of the tag
    /// `record` at `base`, under `-fstrict-volatile-bitfields`: the object of
    /// the size of its declared type `ty`, aligned as that type is from the
    /// record's start, that holds the field. As in GCC, there is none for a
    /// type wider than a machine word, when the field straddles the unit, or
    /// when `base` may be less aligned than the type (a packed record, unless
    /// the object it names is aligned enough). GCC would also read beyond a
    /// record that ends within the unit or start one before a record that
    /// starts within it, which would be out of bounds of an object in the IR,
    /// so those keep the ordinary access; and so does a record of variable
    /// size. `VolatileUnit::writes` tells whether a write may use the unit.
    /// C99: §6.7.3 paragraph 6, p. 109; PDF p. 121 (implementation-defined).
    pub(super) fn volatile_unit(
        &self,
        ty: TypeId,
        record: usize,
        field: &Field<'_>,
        base: Value,
    ) -> Option<VolatileUnit> {
        let types = &self.unit.sema.types;
        let tag = &types.tags[record];
        if !self
            .unit
            .context
            .configuration
            .code_options()
            .strict_volatile_bit_fields
            || !types::is_volatile(ty)
            || tag.variable.get().is_some()
        {
            return None;
        }
        let width = u64::from(field.width?);
        let size = types::layout(types, ty).ok()?.size;
        if !matches!(size, 1 | 2 | 4 | 8) {
            return None;
        }
        let position = field.offset * 8 + u64::from(field.bit_offset);
        let start = position / (size * 8) * size;
        if position - start * 8 + width > size * 8
            || start + size > tag.layout.get()?.size
            || self.place_alignment(base, Align::BYTE).bytes() < size
        {
            return None;
        }
        let (start_bit, end_bit) = memory_location(types, record, field)?;
        Some(VolatileUnit {
            before: u32::try_from(field.offset - start).ok()?,
            size:   u32::try_from(size).ok()?,
            writes: start_bit <= start * 8 && (start + size) * 8 <= end_bit,
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
        if let Some(unit) = field.volatile_unit {
            let (address, field) = self.unit_place(address, field, unit);
            return self.load_bit_field(address, field, true, result);
        }
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
        if let Some(unit) = field.volatile_unit
            && unit.writes
        {
            let (address, field) = self.unit_place(address, field, unit);
            let low = self.store_volatile_unit(address, field, value);
            return self.value_after_store(low, field, result);
        }
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
        self.value_after_store(low, field, result)
    }

    /// The address of `unit` and the field as the bits of its bytes, from
    /// the `address` and `field` of a [`BitField`] that has that unit.
    fn unit_place(
        &mut self,
        address: Value,
        field: BitField,
        unit: VolatileUnit,
    ) -> (Value, BitField) {
        let align = Align::from_bytes(u64::from(unit.size)).unwrap_or(Align::BYTE);
        let address = if unit.before == 0 {
            address
        } else {
            let back = self.iconst(Type::I64, -i128::from(unit.before));
            let start = self.ptr_add(address, back);
            // The field's own address says less than the unit's does.
            self.retain_alignment(start, align);
            start
        };
        let place = BitField {
            span: unit.size,
            shift: field.shift + unit.before * 8,
            align,
            volatile_unit: None,
            ..field
        };
        (address, place)
    }

    /// Writes `value` into `field`, which spans a whole [`VolatileUnit`] at
    /// `address`, with a single volatile store of the unit, after a volatile
    /// load unless the field fills it. Returns the field's low bits, which
    /// the store wrote. The load is not frozen, as it cannot be poison.
    fn store_volatile_unit(&mut self, address: Value, field: BitField, value: Value) -> Value {
        let working = working_type(field);
        let narrowed = self.extend(value, false, working);
        if field.width == working.bits() {
            self.store_piece(address, narrowed, field, 0, true);
            return narrowed;
        }
        let low = self.mask_low(narrowed, field.width);
        let placed = if field.shift == 0 {
            low
        } else {
            self.shift_by(Opcode::Shl, low, field.shift)
        };
        let old = self.load_piece(address, working, field, 0, true);
        let mask = (u128::MAX >> (128 - field.width)) << field.shift;
        let keep = !mask & (u128::MAX >> (128 - working.bits()));
        let keep = self.iconst(working, keep as i128);
        let kept = self.binary_inst(Opcode::And, InstFlags::empty(), old, keep);
        let merged = self.binary_inst(Opcode::Or, InstFlags::empty(), kept, placed);
        self.store_piece(address, merged, field, 0, true);
        low
    }

    /// The new value of `field` from `low`, the low bits written, read back
    /// as the field reads: an unsigned field is already masked.
    fn value_after_store(&mut self, low: Value, field: BitField, result: Type) -> Value {
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

/// The bits, from the start of the record `tag`, of the memory location of
/// `field`, the region a write of it may touch, where GCC's differs from C's
/// in leaving some adjacent padding out. C11 §3.14 makes a maximal run of
/// adjacent bit-fields one memory location, which a zero-width bit-field or
/// another member ends; each bit-field of a union is its own. GCC's region
/// (`finish_bitfield_representative` in its `stor-layout.cc`) starts at the
/// byte of the run's first bit and spans the smallest integer of a machine
/// mode that holds the run, if that stops before the next member or the
/// record's end, and otherwise the whole bytes of the run itself. So the
/// region of `struct { volatile int a : 4; }` is one byte, not the padding.
/// C11: §3.14, p. 5; PDF p. 23.
fn memory_location(types: &Types<'_>, tag: usize, field: &Field<'_>) -> Option<(u64, u64)> {
    let (declaring, index) = declaring_member(types, tag, field)?;
    let record = &types.tags[declaring];
    let members = record.members.get();
    let own = members.get(index)?;
    // The offset of the declaring record in the record `tag`.
    let shift = (field.offset - own.offset) * 8;
    let is_field = |member: &Member| member.width.is_some_and(|width| width != 0);
    let in_union = record.kind == TagKind::Union;
    let first = if in_union {
        index
    } else {
        members[..index]
            .iter()
            .rposition(|member| !is_field(member))
            .map_or(0, |before| before + 1)
    };
    let next = if in_union {
        None
    } else {
        members[index..]
            .iter()
            .position(|member| !is_field(member))
            .map(|after| index + after)
    };
    let last = if in_union {
        own
    } else {
        &members[next.unwrap_or(members.len()) - 1]
    };
    let start = members[first].offset * 8;
    let run_end = last.offset * 8 + u64::from(last.bit_offset) + u64::from(last.width?);
    let run = (run_end - start).next_multiple_of(8);
    let limit = match next.map(|next| &members[next]) {
        | Some(member) => (member.offset * 8 + u64::from(member.bit_offset)).next_multiple_of(8),
        | None => record.layout.get()?.size * 8,
    } - start;
    let region = match [8, 16, 32, 64, 128].into_iter().find(|&bits| bits >= run) {
        | Some(bits) if bits <= limit => bits,
        | _ => run,
    };
    Some((shift + start, shift + start + region))
}

/// The record that declares `field` of the record `tag`, through its anonymous
/// members, and the member's index there.
fn declaring_member(types: &Types<'_>, tag: usize, field: &Field<'_>) -> Option<(usize, usize)> {
    let mut record = tag;
    let mut path = field.path;
    while let Some(next) = path.next {
        let member = types.tags[record].members.get().get(path.index)?;
        let TypeKind::Tag(inner) = types.kind(member.ty) else {
            return None;
        };
        record = inner;
        path = next;
    }
    Some((record, path.index))
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
