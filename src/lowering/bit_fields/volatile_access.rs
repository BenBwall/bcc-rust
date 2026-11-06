//! `-fstrict-volatile-bitfields`: the loads and stores of a `volatile`
//! bit-field. The number and widths of a `volatile` object's accesses are what
//! a memory-mapped device sees, so they follow GCC's, which GCC 13.3 and 14.2
//! agree on at `-O0` and `-O2` (x86-64). A [`Plan`] lists them.
//!
//! **The strict access.** GCC's `strict_volatile_bitfield_p` (`expmed.cc`)
//! gives a field one access of its declared type when the type is at most a
//! word, the field does not straddle a type-sized and -aligned unit of the
//! object, the object is aligned to the type and, for a write, the unit stays
//! in the field's region. A write of the whole unit reads nothing.
//!
//! **The fallback** runs when any of that fails, and has no case of its own in
//! GCC: it is the ordinary access, with volatile's preference for wide modes.
//! - A field wider than a word goes a word at a time.
//! - A byte-aligned field as wide as its access mode is one load or store of
//!   it, unaligned if need be. A read's mode comes from the field's width, so
//!   `char c; unsigned a : 16` in a packed record loads 16 bits. A write's is
//!   the declared type: it stores `short a : 16` that way and no other width.
//! - A write of a whole byte that is the second of a 16-bit unit in the region
//!   is one 16-bit access, however aligned: x86's `insv` pattern handles `%ah`.
//! - Otherwise one access of the widest integer mode, up to a word, that holds
//!   the field in one aligned unit, is no more aligned than the object and, for
//!   a write, stays in the region (`get_best_mode` in `stor-layout.cc`). `a :
//!   12` alone is a 16-bit read-modify-write, since its region is 16 bits. The
//!   read comes even if the write replaces the whole mode.
//! - If no mode does, it splits (`store_split_bit_field`) into units of the
//!   object's alignment, up to a word, halving one that would pass the region.
//!
//! A write measures from its region (`store_bit_field`): the alignment drops to
//! that of the region's offset, so `char c; unsigned a : 12` writes two bytes
//! though it reads one word. The declared type caps the mode of a region at the
//! start of the object, and the widest that fits the region caps any other.
//!
//! What GCC bases all of this on is the outermost object that constant offsets
//! reach ([`FunctionLowerer::volatile_root`]), not the innermost record.
//!
//! **Refused** shapes are compile errors, since another access would be silent:
//! a record of variable size or a `_BitInt` above 128 bits; a whole 128-bit
//! field, whose widths and order in GCC change with the level and the value; an
//! access that would leave a known object, as GCC's read of a small packed
//! `extern` object aligned beyond its size does; and, only where the accesses
//! depend on it, an object GCC aligns further than its type asks (a definition
//! of eight bytes or more, or an automatic object) or an array at an offset
//! that hides how aligned its elements are.
//!
//! GNU extension: GCC Code Gen Options, `-fstrict-volatile-bitfields`.
//! <https://gcc.gnu.org/onlinedocs/gcc/Code-Gen-Options.html>
//! C99: §6.7.3 paragraph 6, p. 109; PDF p. 121 (what an access to a volatile
//! object is, implementation-defined).

use super::{
    BitField,
    FunctionLowerer,
    LoweringError,
    TypeId,
    types,
};
use crate::{
    ir::{
        Align,
        Entity,
        InstData,
        InstFlags,
        Opcode,
        Type,
        Value,
    },
    lowering::{
        Construct,
        alignment::at_offset,
        work::VolatileAccess,
    },
    translation_phases::{
        SourceVectors,
        semantic_analysis::{
            Field,
            Member,
            TagKind,
            TypeKind,
            Types,
        },
    },
};

/// Bits in x86-64's word, the widest ordinary access.
const WORD: u64 = 64;

/// The size from which GCC aligns an object it defines to a word or more,
/// whatever its type asks (probed: `ix86_data_alignment` and `align_variable`
/// raise it only then, without a declared alignment).
const DATA_ALIGNED: u64 = 8;

impl FunctionLowerer<'_, '_, '_, '_, '_> {
    /// The access to the `volatile` bit-field `field`, of declared type `ty`
    /// in the tag `record`, whose place is `address`: `None` without the
    /// option or for another field, an error if GCC's accesses cannot be
    /// matched. Both a read's and a write's are checked, whichever comes.
    pub(in super::super) fn volatile_access(
        &self,
        ty: TypeId,
        record: usize,
        field: &Field<'_>,
        address: Value,
        source: SourceVectors,
    ) -> Result<Option<VolatileAccess>, LoweringError> {
        let types = &self.unit.sema.types;
        if !self
            .unit
            .context
            .configuration
            .code_options()
            .strict_volatile_bit_fields
            || !types::is_volatile(ty)
        {
            return Ok(None);
        }
        let refused = || LoweringError::unsupported(Construct::StrictVolatileBitField, source);
        let Some(width) = field.width else {
            return Ok(None);
        };
        if types.tags[record].variable.get().is_some() || self.wide_bit_int(ty).is_some() {
            return Err(refused());
        }
        let declared = types::layout(types, ty)
            .ok()
            .and_then(|layout| u32::try_from(layout.size * 8).ok())
            .ok_or_else(refused)?;
        if declared == 128 && width == 128 && field.bit_offset == 0 {
            return Err(refused());
        }
        let root = self.volatile_root(address);
        let (start, end) = memory_location(types, record, field).ok_or_else(refused)?;
        let record_start = root.offset.checked_sub(field.offset).ok_or_else(refused)? * 8;
        let at = |align: u32| VolatileAccess {
            position: root.offset * 8 + u64::from(field.bit_offset),
            width,
            declared,
            align,
            region_start: record_start + start,
            region_end: record_start + end,
        };
        let access = at(root.least);
        let (read, write) = (access.read(), access.write());
        let mut align = root.least;
        while align < root.most {
            align *= 2;
            let other = at(align);
            if other.read() != read || other.write() != write {
                return Err(refused());
            }
        }
        if let Some(size) = root.size
            && [read, write]
                .iter()
                .flat_map(Plan::pieces)
                .any(|piece| piece.start / 8 + u64::from(piece.bytes) > size)
        {
            return Err(refused());
        }
        Ok(Some(access))
    }

    /// The object GCC measures `address` from: GCC folds constant offsets of
    /// members and elements into the field's position, down to what it
    /// addresses through a pointer or by name. A variable index leaves the
    /// position alone and limits the alignment to its stride's. Retained
    /// alignments say that only if the array's offset is aligned no less.
    fn volatile_root(&self, address: Value) -> VolatileRoot {
        let retained = |value: Value| self.alignments.get(&value).copied();
        let bits = |align: Align| {
            u32::try_from(align.bytes() * 8).map_or(WORD as u32, |bits| bits.min(WORD as u32))
        };
        let mut root = address;
        let mut offset = 0_u64;
        let mut stride = u32::MAX;
        let mut hidden = u32::MAX;
        while let Some(InstData::Binary {
            opcode: Opcode::PtrAdd,
            args: [base, step],
            ..
        }) = self.draft.defining_inst(root)
            && let Some(below) = retained(base)
        {
            if let Some(constant) = self.draft.constant(step) {
                let Ok(constant) = u64::try_from(constant) else {
                    break;
                };
                offset += constant;
            } else {
                let here = retained(root).map_or(8, bits);
                if here < bits(below) {
                    stride = stride.min(here);
                } else {
                    hidden = hidden.min(here);
                }
            }
            root = base;
        }
        let known = retained(root).map_or(8, bits).min(stride);
        let (size, grows) = match self.draft.defining_inst(root) {
            | Some(InstData::StackAddr { slot }) =>
                (Some(u64::from(self.draft.slots[slot.index()].size)), true),
            | Some(InstData::GlobalAddr { global }) => {
                let global = self.unit.module.global(global);
                let defined = global.init.is_some() || self.unit.defines_object_named(global.name);
                (
                    (global.size != 0).then_some(global.size),
                    defined && global.size >= DATA_ALIGNED,
                )
            },
            | _ => (None, false),
        };
        VolatileRoot {
            offset,
            least: known.min(hidden),
            most: if grows { WORD as u32 } else { known },
            size,
        }
    }

    /// The field at `address` through the volatile loads of its plan.
    pub(super) fn load_volatile_access(
        &mut self,
        address: Value,
        field: BitField,
        access: VolatileAccess,
        result: Type,
    ) -> Value {
        let plan = access.read();
        if let [piece] = plan.pieces() {
            let (address, place) = self.piece_place(address, field, access, *piece);
            return self.load_bit_field(address, place, true, result);
        }
        let working = int_holding(u64::from(field.width));
        let mut joined = self.iconst(working, 0);
        for &piece in plan.pieces() {
            let (at_piece, place) = self.piece_place(address, field, access, piece);
            let ty = types::int_type(piece.bytes * 8);
            let mut part = self.load_piece(at_piece, ty, place, 0, true);
            if piece.shift != 0 {
                part = self.shift_by(Opcode::Lshr, part, piece.shift);
            }
            if piece.shift + piece.bits < ty.bits() {
                part = self.mask_low(part, piece.bits);
            }
            part = self.extend(part, false, working);
            if piece.low != 0 {
                part = self.shift_by(Opcode::Shl, part, piece.low);
            }
            joined = self.binary_inst(Opcode::Or, InstFlags::empty(), joined, part);
        }
        let field = BitField {
            span: working.bits() / 8,
            shift: 0,
            ..field
        };
        self.extract_bit_field(joined, field, result)
    }

    /// Writes `value` to the field at `address` through its plan's volatile
    /// accesses. Returns the field's value as it now reads. A wider load is
    /// not frozen: it cannot be poison, and a freeze of a load that was would
    /// destroy the neighbors. A byte's is, which is sound under any reading of
    /// an unwritten byte.
    pub(super) fn store_volatile_access(
        &mut self,
        address: Value,
        field: BitField,
        access: VolatileAccess,
        value: Value,
        result: Type,
    ) -> Value {
        let plan = access.write();
        let widest = plan.pieces().iter().map(|piece| u64::from(piece.bytes) * 8);
        let working = int_holding(widest.fold(u64::from(field.width), u64::max));
        let narrowed = self.extend(value, false, working);
        let low = if field.width == working.bits() {
            narrowed
        } else {
            self.mask_low(narrowed, field.width)
        };
        for &piece in plan.pieces() {
            let (at_piece, place) = self.piece_place(address, field, access, piece);
            let ty = types::int_type(piece.bytes * 8);
            let mut part = low;
            if piece.low != 0 {
                part = self.shift_by(Opcode::Lshr, part, piece.low);
            }
            if piece.low + piece.bits < field.width {
                part = self.mask_low(part, piece.bits);
            }
            part = self.extend(part, false, ty);
            if piece.reads {
                if piece.shift != 0 {
                    part = self.shift_by(Opcode::Shl, part, piece.shift);
                }
                let mut old = self.load_piece(at_piece, ty, place, 0, true);
                if ty == Type::I8 {
                    old = self.convert(Opcode::Freeze, Type::I8, old);
                }
                let all = u128::MAX >> (128 - ty.bits());
                let keep = all & !((u128::MAX >> (128 - piece.bits)) << piece.shift);
                if keep != 0 {
                    let keep = self.iconst(ty, keep as i128);
                    let kept = self.binary_inst(Opcode::And, InstFlags::empty(), old, keep);
                    part = self.binary_inst(Opcode::Or, InstFlags::empty(), kept, part);
                }
            }
            self.store_piece(at_piece, part, place, 0, true);
        }
        self.value_after_store(low, field, result)
    }

    /// The address of `piece` and the field's bits in it, from the `address`
    /// of `field`.
    fn piece_place(
        &mut self,
        address: Value,
        field: BitField,
        access: VolatileAccess,
        piece: Piece,
    ) -> (Value, BitField) {
        let align = Align::from_bytes(u64::from(access.align) / 8).unwrap_or(Align::BYTE);
        let align = at_offset(align, u128::from(piece.start / 8));
        let distance = i128::from(piece.start / 8) - i128::from(access.position / 8);
        let address = if distance == 0 {
            address
        } else {
            let distance = self.iconst(Type::I64, distance);
            let start = self.ptr_add(address, distance);
            // The field's own address says less than the object's does.
            self.retain_alignment(start, align);
            start
        };
        let place = BitField {
            span: piece.bytes,
            shift: piece.shift,
            width: piece.bits,
            align,
            volatile_access: None,
            ..field
        };
        (address, place)
    }
}

/// See [`FunctionLowerer::volatile_root`]. The alignments are in bits.
struct VolatileRoot {
    /// How many bytes into the object the place is.
    offset: u64,
    /// The alignment that is certain.
    least:  u32,
    /// The alignment GCC may know of.
    most:   u32,
    /// The object's size, if it is known to be one of these bytes.
    size:   Option<u64>,
}

/// The integer type of the fewest bytes, a power of two, that holds `bits`.
fn int_holding(bits: u64) -> Type {
    let bytes = u32::try_from(bits.div_ceil(8)).unwrap_or(16);
    types::int_type(bytes.next_power_of_two().min(16) * 8)
}

/// One volatile access of a [`Plan`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct Piece {
    /// The access's first bit in the object, a multiple of eight.
    start: u64,
    /// The access's size.
    bytes: u32,
    /// The lowest of the field's bits here, within the access.
    shift: u32,
    /// How many of the field's bits are here.
    bits:  u32,
    /// Which of the field's bits comes first here.
    low:   u32,
    /// Whether a write loads before it stores.
    reads: bool,
}

/// The accesses of a read or a write, lowest bits first. The most any field
/// needs is one a byte, with one byte twice where a 128-bit field splits at a
/// word: 17 and one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Plan {
    pieces: [Piece; 18],
    len:    usize,
}

impl Plan {
    fn new() -> Self {
        Self {
            pieces: [Piece::default(); 18],
            len:    0,
        }
    }

    fn push(&mut self, piece: Piece) {
        self.pieces[self.len] = piece;
        self.len += 1;
    }

    fn pieces(&self) -> &[Piece] {
        &self.pieces[..self.len]
    }
}

impl VolatileAccess {
    /// The accesses of a read (`extract_bit_field` in GCC's `expmed.cc`).
    fn read(self) -> Plan {
        let mut plan = Plan::new();
        let width = u64::from(self.width);
        if self.is_strict(0, u64::MAX) {
            plan.push(self.strict_piece());
        } else if width > WORD {
            for word in 0..width.div_ceil(WORD) {
                let bits = (width - word * WORD).min(WORD);
                self.read_bits(self.position + word * WORD, bits, &mut plan);
            }
        } else {
            self.read_bits(self.position, width, &mut plan);
        }
        plan
    }

    /// A field of at most a word, or a word of a wider one
    /// (`extract_fixed_bit_field`, `extract_split_bit_field`).
    fn read_bits(self, position: u64, bits: u64, plan: &mut Plan) {
        if position.is_multiple_of(8) && matches!(bits, 8 | 16 | 32 | 64) {
            plan.push(self.piece(position, bits, position, bits, false));
            return;
        }
        let align = u64::from(self.align);
        if let Some(mode) = best_mode(bits, position, u64::MAX, align, WORD) {
            let start = position - position % mode;
            plan.push(self.piece(start, mode, position, bits, false));
            return;
        }
        let unit = align.min(WORD);
        let mut done = 0;
        while done < bits {
            let here = position + done;
            let size = (bits - done).min(unit - here % unit);
            let mode = best_mode(size, here, u64::MAX, align, WORD).unwrap_or(unit);
            plan.push(self.piece(here - here % mode, mode, here, size, false));
            done += size;
        }
    }

    /// The accesses of a write (`store_bit_field` in GCC's `expmed.cc`).
    fn write(self) -> Plan {
        let mut plan = Plan::new();
        if self.is_strict(self.region_start, self.region_end) {
            let piece = self.strict_piece();
            plan.push(Piece {
                reads: self.width != self.declared,
                ..piece
            });
            return plan;
        }
        let width = u64::from(self.width);
        let region = self.region_start;
        let position = self.position - region;
        let end = self.region_end - region;
        let writer = if region == 0 {
            Writer {
                access: self,
                align: u64::from(self.align),
                mode: u64::from(self.declared),
                end,
            }
        } else {
            let align = u64::from(self.align);
            let own = align.min(1 << region.trailing_zeros());
            Writer {
                access: self,
                align: own,
                mode: best_mode(width, position, end, align, u64::MAX)
                    .unwrap_or(u64::from(self.declared)),
                end,
            }
        };
        let declared = u64::from(self.declared);
        if width > WORD {
            for word in 0..width.div_ceil(WORD) {
                let bits = (width - word * WORD).min(WORD);
                writer.bits(position + word * WORD, bits, WORD, &mut plan);
            }
        } else {
            writer.bits(position, width, declared, &mut plan);
        }
        plan
    }

    /// Whether GCC's strict access applies, for a write within the region
    /// `start..end` (`strict_volatile_bitfield_p`).
    fn is_strict(self, start: u64, end: u64) -> bool {
        let unit = u64::from(self.declared);
        let (width, position) = (u64::from(self.width), self.position);
        let first = position - position % unit;
        unit <= WORD
            && position % unit + width <= unit
            && u64::from(self.align) >= unit
            && first >= start
            && first + unit <= end
    }

    fn strict_piece(self) -> Piece {
        let unit = u64::from(self.declared);
        let start = self.position - self.position % unit;
        self.piece(start, unit, self.position, u64::from(self.width), false)
    }

    /// The access of `mode` bits at `start` that holds `bits` bits of the
    /// field from `position`.
    fn piece(self, start: u64, mode: u64, position: u64, bits: u64, reads: bool) -> Piece {
        Piece {
            start,
            bytes: (mode / 8) as u32,
            shift: (position - start) as u32,
            bits: bits as u32,
            low: (position - self.position) as u32,
            reads,
        }
    }
}

/// A write, in bits from the start of its region.
#[derive(Clone, Copy)]
struct Writer {
    access: VolatileAccess,
    /// The alignment at the region's start.
    align:  u64,
    /// The mode of the region's address, which caps a mode and a unit.
    mode:   u64,
    /// The bit after the region.
    end:    u64,
}

impl Writer {
    /// `store_bit_field_1`, with `field_mode` the width of a full store.
    fn bits(self, position: u64, bits: u64, field_mode: u64, plan: &mut Plan) {
        if position.is_multiple_of(8) && bits == field_mode {
            plan.push(self.piece(position, bits, position, bits, false));
            return;
        }
        // x86's `insv` pattern inserts a register's second byte (`%ah`), which
        // GCC tries first in the narrowest mode that has it, and in memory
        // however aligned (`adjust_bit_field_mem_for_reg`).
        if bits == 8 && position % 16 == 8 && position + 8 <= self.end {
            plan.push(self.piece(position - 8, 16, position, bits, true));
            return;
        }
        let largest = self.mode.min(WORD);
        if let Some(mode) = best_mode(bits, position, self.end, self.align, largest) {
            plan.push(self.piece(position - position % mode, mode, position, bits, true));
            return;
        }
        let mut unit = self.align.min(largest);
        let mut done = 0;
        while done < bits {
            let here = position + done;
            let within = here % unit;
            if unit > 8 && here - within + unit > self.end {
                unit /= 2;
                continue;
            }
            let size = (bits - done).min(unit - within);
            let mode = best_mode(size, here, self.end, self.align, largest).unwrap_or(unit);
            plan.push(self.piece(here - here % mode, mode, here, size, true));
            done += size;
        }
    }

    fn piece(self, start: u64, mode: u64, position: u64, bits: u64, reads: bool) -> Piece {
        let region = self.access.region_start;
        self.access
            .piece(start + region, mode, position + region, bits, reads)
    }
}

/// The widest integer mode, up to `largest` bits, whose aligned unit holds the
/// `bits` bits at `position`, stops by `end` and is no more aligned than
/// `align` (`get_best_mode` and `bit_field_mode_iterator`, in GCC's
/// `stor-layout.cc`, for a volatile field on a target that does not narrow
/// them). The search ends at the first mode that passes `end` or `align`.
fn best_mode(bits: u64, position: u64, end: u64, align: u64, largest: u64) -> Option<u64> {
    let mut best = None;
    for mode in [8, 16, 32, 64, 128] {
        if mode > WORD && best.is_some() {
            break;
        }
        let within = position % mode;
        if within + bits > mode {
            continue;
        }
        if position - within + mode > end || mode > align || mode > largest {
            break;
        }
        best = Some(mode);
    }
    best
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
