//! Scalar replacement of aggregates: splits a stack slot that is accessed
//! only at constant offsets into one slot per field, so that `promote` can
//! turn every field into SSA values.
//!
//! A structure or array local, a by-value argument copy and a call's result
//! area all live in stack slots that the program reads and writes member by
//! member, and copies whole with `copy`. `promote` keeps such a slot in memory
//! because its address is offset and copied. This pass rewrites it into
//! scalar slots that `promote` accepts:
//!
//! 1. **Uses.** Every pointer derived from a slot's `stack_addr` through
//!    `ptr_add`s with constant offsets must be the address of a non-volatile
//!    `load` or `store` inside the slot, an operand of a non-volatile `copy`
//!    that covers the whole slot, or the destination of a non-volatile `fill`
//!    of the whole slot with a constant byte. Any other use (a call or branch
//!    argument, a stored address, a comparison, a variable offset, a partial
//!    `copy`) lets the address escape and keeps the slot as it is.
//! 2. **Components.** A whole-slot `copy` between two slots joins them (with
//!    union-find): both must be split the same way, so that the copy can move
//!    field by field. A component splits only if none of its slots escapes.
//! 3. **Fields.** The loads and stores of the component, by offset and type,
//!    are its fields. Two accesses at one offset must have one type, and
//!    distinct fields must not overlap; a union read through two members, or an
//!    array read at a variable index, keeps its slot.
//! 4. **Rewrite.** Each slot of the component gets one new slot per field.
//!    Loads and stores read and write the field's slot instead; a `copy`
//!    between two slots of the component becomes a load and a store per field;
//!    a `copy` from or to other memory becomes one load and one store per field
//!    at the field's offset there; a `fill` stores each field's constant. The
//!    old `stack_addr`s, offsets and slots are removed.
//!
//! In
//!
//! ```text
//! block0(v0: ptr):                      block0(v0: ptr):
//!     v1 = stack_addr slot0                 v1 = stack_addr slot2
//!     copy v1, v0, v2, align 4              v2 = stack_addr slot3
//!     v3 = ptr_add inbounds v1, v4          v3 = load.i32 v0, align 4
//!     v5 = load.i32 v3, align 4             store.i32 v3, v1, align 4
//!                                           v4 = ptr_add inbounds v0, v5
//!                                           v6 = load.i32 v4, align 4
//!                                           store.i32 v6, v2, align 4
//!                                           v7 = load.i32 v2, align 4
//! ```
//!
//! the eight-byte `slot0` with fields at offsets 0 and 4 becomes `slot2` and
//! `slot3`, which `promote` then removes.
//!
//! **Bytes and poison.** The IR's memory holds bytes, each possibly poison,
//! and a load is poison if any byte it reads is. A field moved as a value is
//! poison as a whole when any of its bytes is, and bytes outside every field
//! are not moved at all. Neither difference is visible as long as the bytes of
//! the component are observed only through its own loads, which read whole
//! fields. They are also observed by a `copy` *out* of the component, so:
//!
//! - a component copied out may not also be copied *in* from other memory
//!   (whose fields could mix poison and defined bytes); its bytes then come
//!   only from whole-field stores, constant fills and copies within the
//!   component, and a field read back as a value has exactly those bytes;
//! - the bytes of a component copied out that no access covers are split into
//!   integer fields too, so that fills and copies still carry them.
//!
//! A field slot is as aligned as the field's offset in the original slot.
//! Each component split is one rewrite for bisecting. Components with more
//! than [`MAX_FIELDS`] fields, and a slot already accessed as one scalar of
//! its size (which `promote` takes as it is), are left alone.
//!
//! This is LLVM's `SROA` pass in miniature (`llvm/lib/Transforms/Scalar/
//! SROA.cpp`): partitions by constant offsets, memory intrinsics split along
//! them, without its splitting of overlapping or vector accesses.
//!
//! C99: §6.2.5 paragraph 20, p. 35; PDF p. 47 (structures and arrays are
//! sequences of member objects); §6.2.6.1 paragraph 6, p. 37; PDF p. 49
//! (padding bytes take unspecified values, which only the copy-out rule above
//! still has to carry); §5.1.2.3 paragraph 5, pp. 13-14; PDF pp. 25-26.

use super::{
    draft::{
        Constant,
        Draft,
    },
    report::OptimizationReport,
};
use crate::{
    ir::{
        Align,
        Bits64,
        Block,
        Entity,
        Inst,
        InstData,
        InstFlags,
        Opcode,
        StackSlot,
        StackSlotData,
        Type,
        Value,
        ValueDef,
    },
    util::bump::ArenaVec,
};

/// The most fields one component is split into; a copy expands to a load
/// and a store per field.
const MAX_FIELDS: usize = 32;

/// Splits every splittable component of stack slots of one function.
/// Returns whether it split any.
pub(super) fn run(draft: &mut Draft<'_, '_>, report: &mut OptimizationReport) -> bool {
    let slots = draft.stack_slot_count();
    if slots == 0 {
        return false;
    }
    let mut uses = Uses::collect(draft);
    let plans = uses.plans(draft);
    let mut changed = false;
    for plan in &plans {
        if !report.allow() {
            break;
        }
        rewrite(draft, &uses, plan);
        changed = true;
    }
    if changed {
        draft.sweep();
    }
    changed
}

/// How the slots of one function are used, gathered in one scan.
struct Uses<'s> {
    /// Union-find parents joining the slots that whole `copy`s connect.
    parent:   ArenaVec<'s, u32>,
    /// Per slot, then per component root after [`Uses::plans`].
    flags:    ArenaVec<'s, SlotFlags>,
    /// Every load and store of a slot, by slot, offset and type.
    accesses: ArenaVec<'s, (StackSlot, u32, Type)>,
    /// Every `copy` and `fill` that touches a slot.
    ranges:   ArenaVec<'s, (Block, Inst)>,
}

/// What a slot's uses allow.
#[derive(Clone, Copy, Default, Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Independent facts about one slot, merged with `|`."
)]
struct SlotFlags {
    escaped:      bool,
    copied_in:    bool,
    copied_out:   bool,
    filled:       bool,
    filled_bytes: bool,
}

/// One component to split: its slots and their common fields.
struct Plan<'s> {
    slots:  ArenaVec<'s, StackSlot>,
    /// Offsets and types, sorted by offset, not overlapping.
    fields: ArenaVec<'s, (u32, Type)>,
}

impl<'s> Uses<'s> {
    /// Scans the live blocks, unreachable ones included.
    fn collect(draft: &Draft<'_, 's>) -> Self {
        let slots = draft.stack_slot_count();
        let mut uses = Self {
            parent:   ArenaVec::with_capacity_in(slots, draft.scratch),
            flags:    ArenaVec::with_capacity_in(slots, draft.scratch),
            accesses: ArenaVec::new_in(draft.scratch),
            ranges:   ArenaVec::new_in(draft.scratch),
        };
        uses.parent.extend((0..slots).map(|index| index as u32));
        uses.flags.resize(slots, SlotFlags::default());
        for (index, flags) in uses.flags.iter_mut().enumerate() {
            flags.escaped = draft.is_stack_slot_removed(StackSlot::new(index));
        }
        for block in draft.alive_blocks() {
            let data = draft.block(block);
            for &inst in &data.insts {
                if draft.is_removed(inst) || draft.inst_constant(inst).is_some() {
                    continue;
                }
                uses.scan(draft, block, inst);
            }
            data.term
                .for_each_control_operand(|value| uses.escape(draft, value));
            for edge in data.term.edges() {
                for &arg in &edge.args {
                    uses.escape(draft, arg);
                }
            }
        }
        uses
    }

    /// Records what one instruction does with slot addresses.
    fn scan(&mut self, draft: &Draft<'_, 's>, block: Block, inst: Inst) {
        match *draft.inst(inst) {
            | InstData::StackAddr { .. } => {},
            | InstData::Binary {
                opcode: Opcode::PtrAdd,
                ..
            } => {
                // The derived address is judged where it is used; a variable
                // offset makes those uses escape.
            },
            | InstData::Load {
                ty, flags, addr, ..
            } => self.access(draft, ty, flags, addr),
            | InstData::Store {
                ty,
                flags,
                args: [value, addr],
                ..
            } => {
                self.escape(draft, value);
                self.access(draft, ty, flags, addr);
            },
            | InstData::MemoryRange {
                opcode,
                flags,
                args: [destination, source, size],
                ..
            } => self.range(
                draft,
                block,
                inst,
                opcode,
                flags,
                [destination, source, size],
            ),
            | _ => draft.for_each_operand(inst, |value| self.escape(draft, value)),
        }
    }

    /// Records a load or store through `addr`.
    fn access(&mut self, draft: &Draft<'_, 's>, ty: Type, flags: InstFlags, addr: Value) {
        let Some((slot, offset)) = derive(draft, addr) else {
            return;
        };
        let size = i64::from(draft.stack_slot(slot).size);
        let inside = offset.filter(|&offset| {
            offset >= 0 && offset + i64::from(ty.bytes()) <= size && ty != Type::I1
        });
        match inside {
            | Some(offset) if !flags.contains(InstFlags::VOLATILE) => self.accesses.push((
                slot,
                u32::try_from(offset).expect("the offset is inside a slot"),
                ty,
            )),
            | _ => self.flags[slot.index()].escaped = true,
        }
    }

    /// Records a `copy` or `fill`, joining whole-slot copies.
    fn range(
        &mut self,
        draft: &Draft<'_, 's>,
        block: Block,
        inst: Inst,
        opcode: Opcode,
        flags: InstFlags,
        [destination, source, size]: [Value; 3],
    ) {
        let size = match draft.constant(size) {
            | Some(Constant::Int(_, bits)) => Some(bits),
            | _ => None,
        };
        let whole = |derived: Option<(StackSlot, Option<i64>)>| {
            derived.and_then(|(slot, offset)| {
                (!flags.contains(InstFlags::VOLATILE)
                    && offset == Some(0)
                    && size == Some(u128::from(draft.stack_slot(slot).size))
                    && size != Some(0))
                .then_some(slot)
            })
        };
        let to = derive(draft, destination);
        if opcode == Opcode::Fill {
            let byte = draft.constant(source);
            match (whole(to), byte) {
                | (Some(slot), Some(Constant::Int(_, byte))) => {
                    let slot_flags = &mut self.flags[slot.index()];
                    slot_flags.filled = true;
                    slot_flags.filled_bytes |= byte != 0;
                    self.ranges.push((block, inst));
                },
                | _ => self.escape(draft, destination),
            }
            return;
        }
        let from = derive(draft, source);
        match (to, from) {
            | (None, None) => {},
            | (Some(_), Some(_)) => match (whole(to), whole(from)) {
                | (Some(a), Some(b)) => {
                    self.union(a, b);
                    self.ranges.push((block, inst));
                },
                | _ => {
                    self.escape(draft, destination);
                    self.escape(draft, source);
                },
            },
            | (Some(_), None) => match whole(to) {
                | Some(slot) => {
                    self.flags[slot.index()].copied_in = true;
                    self.ranges.push((block, inst));
                },
                | None => self.escape(draft, destination),
            },
            | (None, Some(_)) => match whole(from) {
                | Some(slot) => {
                    self.flags[slot.index()].copied_out = true;
                    self.ranges.push((block, inst));
                },
                | None => self.escape(draft, source),
            },
        }
    }

    /// Marks the slot `value` points into, if any, as escaped.
    fn escape(&mut self, draft: &Draft<'_, 's>, value: Value) {
        if draft.value_type(draft.resolve(value)) == Type::Ptr
            && let Some((slot, _)) = derive(draft, value)
        {
            self.flags[slot.index()].escaped = true;
        }
    }

    fn find(&self, slot: StackSlot) -> StackSlot {
        let mut index = slot.index();
        while self.parent[index] as usize != index {
            index = self.parent[index] as usize;
        }
        StackSlot::new(index)
    }

    fn union(&mut self, a: StackSlot, b: StackSlot) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            let (low, high) = if a.index() < b.index() {
                (a, b)
            } else {
                (b, a)
            };
            self.parent[high.index()] = low.index() as u32;
        }
    }

    /// The components worth splitting, with their fields. Merges each slot's
    /// flags into its component's root.
    fn plans(&mut self, draft: &Draft<'_, 's>) -> ArenaVec<'s, Plan<'s>> {
        let slots = self.parent.len();
        let mut members: ArenaVec<'s, (StackSlot, StackSlot)> =
            ArenaVec::with_capacity_in(slots, draft.scratch);
        for index in 0..slots {
            let slot = StackSlot::new(index);
            let root = self.find(slot);
            members.push((root, slot));
            if root != slot {
                let flags = self.flags[index];
                let merged = &mut self.flags[root.index()];
                merged.escaped |= flags.escaped;
                merged.copied_in |= flags.copied_in;
                merged.copied_out |= flags.copied_out;
                merged.filled |= flags.filled;
                merged.filled_bytes |= flags.filled_bytes;
            }
        }
        members.sort_unstable_by_key(|&(root, slot)| (root.index(), slot.index()));
        let mut accesses: ArenaVec<'s, (StackSlot, u32, Type)> =
            ArenaVec::with_capacity_in(self.accesses.len(), draft.scratch);
        accesses.extend(
            self.accesses
                .iter()
                .map(|&(slot, offset, ty)| (self.find(slot), offset, ty)),
        );
        accesses.sort_unstable_by_key(|&(root, offset, ty)| (root.index(), offset, ty as u8));
        accesses.dedup();
        let mut ranged: ArenaVec<'s, bool> = ArenaVec::with_capacity_in(slots, draft.scratch);
        ranged.resize(slots, false);
        for &(_, inst) in &self.ranges {
            let InstData::MemoryRange { args, .. } = *draft.inst(inst) else {
                unreachable!("only copies and fills are ranges");
            };
            if let Some((slot, _)) = derive(draft, args[0]).or_else(|| derive(draft, args[1])) {
                ranged[self.find(slot).index()] = true;
            }
        }
        let mut plans = ArenaVec::new_in(draft.scratch);
        let mut next_member = 0;
        let mut next_access = 0;
        while next_member < members.len() {
            let root = members[next_member].0;
            let member_end = next_member
                + members[next_member..]
                    .iter()
                    .take_while(|&&(other, _)| other == root)
                    .count();
            while next_access < accesses.len() && accesses[next_access].0.index() < root.index() {
                next_access += 1;
            }
            let access_end = next_access
                + accesses[next_access..]
                    .iter()
                    .take_while(|&&(other, ..)| other == root)
                    .count();
            let group = &members[next_member..member_end];
            let fields = &accesses[next_access..access_end];
            if let Some(plan) = self.plan(draft, root, group, fields, ranged[root.index()]) {
                plans.push(plan);
            }
            next_member = member_end;
            next_access = access_end;
        }
        plans
    }

    /// The plan for the component of `root`, or `None` if it stays.
    fn plan(
        &self,
        draft: &Draft<'_, 's>,
        root: StackSlot,
        members: &[(StackSlot, StackSlot)],
        accesses: &[(StackSlot, u32, Type)],
        ranged: bool,
    ) -> Option<Plan<'s>> {
        let flags = self.flags[root.index()];
        if flags.escaped || (flags.copied_in && flags.copied_out) {
            return None;
        }
        let size = draft.stack_slot(root).size;
        // A lone slot read and written as one scalar of its size is
        // `promote`'s; an unused one too.
        if !ranged
            && members.len() == 1
            && (accesses.is_empty() || (accesses.len() == 1 && accesses[0].2.bytes() == size))
        {
            return None;
        }
        let mut fields = ArenaVec::with_capacity_in(accesses.len(), draft.scratch);
        let mut end = 0_u32;
        for &(_, offset, ty) in accesses {
            if offset < end {
                return None;
            }
            if flags.copied_out {
                fill_gap(&mut fields, end, offset);
            }
            fields.push((offset, ty));
            end = offset + ty.bytes();
        }
        if flags.copied_out {
            fill_gap(&mut fields, end, size);
        }
        let fillable = |ty: Type| match ty {
            | Type::Ptr => !flags.filled_bytes,
            | Type::F80 | Type::F128 => false,
            | _ => true,
        };
        if fields.len() > MAX_FIELDS
            || (flags.filled && !fields.iter().all(|&(_, ty)| fillable(ty)))
        {
            return None;
        }
        let mut slots = ArenaVec::with_capacity_in(members.len(), draft.scratch);
        slots.extend(members.iter().map(|&(_, slot)| slot));
        Some(Plan { slots, fields })
    }
}

/// Covers the bytes `start..end` with aligned integer fields of at most
/// eight bytes.
fn fill_gap(fields: &mut ArenaVec<'_, (u32, Type)>, mut start: u32, end: u32) {
    while start < end {
        let ty = [Type::I64, Type::I32, Type::I16, Type::I8]
            .into_iter()
            .find(|ty| start.is_multiple_of(ty.bytes()) && start + ty.bytes() <= end)
            .expect("an i8 always fits");
        fields.push((start, ty));
        start += ty.bytes();
    }
}

/// The slot and constant offset `addr` points to, following `ptr_add`s
/// back to a `stack_addr`; the offset is `None` if one is not constant.
fn derive(draft: &Draft<'_, '_>, addr: Value) -> Option<(StackSlot, Option<i64>)> {
    let mut value = draft.resolve(addr);
    let mut offset = Some(0_i64);
    loop {
        let ValueDef::Result(inst) = draft.value_def(value) else {
            return None;
        };
        if draft.inst_constant(inst).is_some() {
            return None;
        }
        match *draft.inst(inst) {
            | InstData::StackAddr { slot } => return Some((slot, offset)),
            | InstData::Binary {
                opcode: Opcode::PtrAdd,
                args: [base, delta],
                ..
            } => {
                offset = match (offset, draft.constant(delta)) {
                    | (Some(offset), Some(Constant::Int(Type::I64, bits))) =>
                        u64::try_from(bits).ok().and_then(|bits| {
                            offset.checked_add(i64::from_ne_bytes(bits.to_ne_bytes()))
                        }),
                    | _ => None,
                };
                value = draft.resolve(base);
            },
            | _ => return None,
        }
    }
}

/// Splits the component `plan` describes.
fn rewrite(draft: &mut Draft<'_, '_>, uses: &Uses<'_>, plan: &Plan<'_>) {
    let scratch = draft.scratch;
    let fields = plan.fields.len();
    // The new slots' addresses, `fields` per old slot, in the entry block.
    let mut addresses = ArenaVec::with_capacity_in(plan.slots.len() * fields, scratch);
    let mut position = ArenaVec::with_capacity_in(draft.stack_slot_count(), scratch);
    position.resize(draft.stack_slot_count(), u32::MAX);
    for (index, &slot) in plan.slots.iter().enumerate() {
        position[slot.index()] = index as u32;
        let data = draft.stack_slot(slot);
        for &(offset, ty) in &plan.fields {
            let field = draft.add_stack_slot(StackSlotData {
                size:  ty.bytes(),
                align: align_at(data.align, offset),
            });
            let address = draft
                .add_inst(
                    Draft::entry(),
                    0,
                    InstData::StackAddr { slot: field },
                    Some(Type::Ptr),
                )
                .expect("`stack_addr` has a result");
            addresses.push((address, align_at(data.align, offset)));
        }
    }
    // The new slots are not part of the plan.
    position.resize(draft.stack_slot_count(), u32::MAX);
    let field_of = |slot: StackSlot, offset: u32| {
        let field = plan
            .fields
            .binary_search_by_key(&offset, |&(start, _)| start)
            .expect("every access is a field");
        addresses[position[slot.index()] as usize * fields + field]
    };
    // Loads and stores move to their fields' slots.
    let mut blocks = ArenaVec::new_in(scratch);
    blocks.extend(draft.alive_blocks());
    for &block in &blocks {
        for index in 0..draft.block(block).insts.len() {
            let inst = draft.block(block).insts[index];
            if draft.is_removed(inst) || draft.inst_constant(inst).is_some() {
                continue;
            }
            let data = *draft.inst(inst);
            let addr = match data {
                | InstData::Load { addr, .. }
                | InstData::Store {
                    args: [_, addr], ..
                } => addr,
                | _ => continue,
            };
            let Some((slot, Some(offset))) = derive(draft, addr) else {
                continue;
            };
            if position[slot.index()] == u32::MAX {
                continue;
            }
            let offset = u32::try_from(offset).expect("the access is inside the slot");
            let (field, _) = field_of(slot, offset);
            let moved = match data {
                | InstData::Load {
                    ty,
                    flags,
                    align,
                    tag,
                    ..
                } => InstData::Load {
                    ty,
                    flags,
                    align,
                    tag,
                    addr: field,
                },
                | InstData::Store {
                    ty,
                    flags,
                    align,
                    tag,
                    args: [value, _],
                } => InstData::Store {
                    ty,
                    flags,
                    align,
                    tag,
                    args: [value, field],
                },
                | _ => unreachable!("matched above"),
            };
            draft.replace_inst(inst, moved);
        }
    }
    // Copies and fills become a load and a store, or a store, per field.
    for &(block, inst) in &uses.ranges {
        let InstData::MemoryRange {
            opcode,
            align,
            args: [destination, source, _],
            ..
        } = *draft.inst(inst)
        else {
            unreachable!("only copies and fills are ranges");
        };
        let planned = |derived: Option<(StackSlot, Option<i64>)>| {
            derived
                .map(|(slot, _)| slot)
                .filter(|slot| position[slot.index()] != u32::MAX)
        };
        let to = planned(derive(draft, destination));
        let from = if opcode == Opcode::Fill {
            None
        } else {
            planned(derive(draft, source))
        };
        if to.is_none() && from.is_none() {
            continue;
        }
        // Constants go into the entry block first, so that the position
        // found below stays right.
        let mut constants = ArenaVec::with_capacity_in(fields, scratch);
        for &(offset, ty) in &plan.fields {
            constants.push(if opcode == Opcode::Fill {
                let Some(Constant::Int(_, byte)) = draft.constant(source) else {
                    unreachable!("a planned fill has a constant byte");
                };
                Some(fill_constant(draft, ty, (byte & 0xFF) as u8))
            } else if (to.is_none() || from.is_none()) && offset != 0 {
                Some(draft.add_constant(Constant::Int(Type::I64, u128::from(offset))))
            } else {
                None
            });
        }
        let at = draft
            .block(block)
            .insts
            .iter()
            .position(|&other| other == inst)
            .expect("the range is in its block");
        let mut cursor = Cursor { block, at };
        for (field, &(offset, ty)) in plan.fields.iter().enumerate() {
            let value = if opcode == Opcode::Fill {
                constants[field].expect("a fill has a constant per field")
            } else {
                let (address, field_align) = match from {
                    | Some(slot) => field_of(slot, offset),
                    | None => (
                        cursor.offset(draft, source, constants[field]),
                        align_at(align, offset),
                    ),
                };
                cursor
                    .insert(draft, load(ty, field_align, address), Some(ty))
                    .expect("a load has a result")
            };
            let (address, field_align) = match to {
                | Some(slot) => field_of(slot, offset),
                | None => (
                    cursor.offset(draft, destination, constants[field]),
                    align_at(align, offset),
                ),
            };
            _ = cursor.insert(draft, store(ty, field_align, value, address), None);
        }
        draft.remove_inst(inst);
    }
    // The old addresses and offsets are unused now.
    for &block in &blocks {
        for index in 0..draft.block(block).insts.len() {
            let inst = draft.block(block).insts[index];
            if draft.is_removed(inst) || draft.inst_constant(inst).is_some() {
                continue;
            }
            let derived = match *draft.inst(inst) {
                | InstData::StackAddr { slot } => Some(slot),
                | InstData::Binary {
                    opcode: Opcode::PtrAdd,
                    args: [base, _],
                    ..
                } => derive(draft, base).map(|(slot, _)| slot),
                | _ => None,
            };
            if derived.is_some_and(|slot| position[slot.index()] != u32::MAX) {
                draft.remove_inst(inst);
            }
        }
    }
    for &slot in &plan.slots {
        draft.remove_stack_slot(slot);
    }
}

/// Where the expansion of a `copy` or `fill` inserts its instructions.
struct Cursor {
    block: Block,
    at:    usize,
}

impl Cursor {
    fn insert(
        &mut self,
        draft: &mut Draft<'_, '_>,
        data: InstData,
        ty: Option<Type>,
    ) -> Option<Value> {
        let result = draft.add_inst(self.block, self.at, data, ty);
        self.at += 1;
        result
    }

    /// `base` offset by `delta`, or `base` itself when there is no delta.
    fn offset(&mut self, draft: &mut Draft<'_, '_>, base: Value, delta: Option<Value>) -> Value {
        match delta {
            | Some(delta) => self
                .insert(
                    draft,
                    InstData::Binary {
                        opcode: Opcode::PtrAdd,
                        ty:     Type::Ptr,
                        flags:  InstFlags::INBOUNDS,
                        args:   [base, delta],
                    },
                    Some(Type::Ptr),
                )
                .expect("`ptr_add` has a result"),
            | None => base,
        }
    }
}

/// The alignment of the byte at `offset` in an object aligned to `align`.
fn align_at(align: Align, offset: u32) -> Align {
    if offset == 0 {
        return align;
    }
    let offset_align = Align::from_bytes(1 << offset.trailing_zeros())
        .expect("a power of two below 2^32 is an alignment");
    if offset_align.bytes() < align.bytes() {
        offset_align
    } else {
        align
    }
}

/// The value a `fill` with `byte` leaves in a field of type `ty`.
fn fill_constant(draft: &mut Draft<'_, '_>, ty: Type, byte: u8) -> Value {
    let splat = (0..ty.bytes()).fold(0_u128, |bits, _| bits << 8 | u128::from(byte));
    match ty {
        | Type::Ptr => draft
            .add_inst(
                Draft::entry(),
                0,
                InstData::Nullary {
                    opcode: Opcode::Null,
                    ty,
                },
                Some(ty),
            )
            .expect("`null` has a result"),
        | Type::F32 | Type::F64 => draft
            .add_inst(
                Draft::entry(),
                0,
                InstData::Const {
                    opcode: Opcode::Fconst,
                    ty,
                    bits: Bits64::new(splat as u64),
                },
                Some(ty),
            )
            .expect("`fconst` has a result"),
        | _ => draft.add_constant(Constant::Int(ty, splat & ty.mask())),
    }
}

fn load(ty: Type, align: Align, addr: Value) -> InstData {
    InstData::Load {
        ty,
        flags: InstFlags::empty(),
        align,
        tag: None,
        addr,
    }
}

fn store(ty: Type, align: Align, value: Value, addr: Value) -> InstData {
    InstData::Store {
        ty,
        flags: InstFlags::empty(),
        align,
        tag: None,
        args: [value, addr],
    }
}
