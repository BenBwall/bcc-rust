//! Rewriting one planned component: new field slots, loads and stores moved
//! to them, and copies and fills expanded field by field.

use super::{
    MoveKind,
    Plan,
    plan::{
        Uses,
        derive,
    },
};
use crate::{
    ir::{
        Align,
        Bits64,
        Block,
        Entity,
        InstData,
        InstFlags,
        Opcode,
        StackSlot,
        StackSlotData,
        Type,
        Value,
    },
    optimizer::draft::{
        Constant,
        Draft,
    },
    util::bump::ArenaVec,
};

/// Splits the component `plan` describes.
pub(super) fn rewrite(draft: &mut Draft<'_, '_>, uses: &Uses<'_>, plan: &Plan<'_>) {
    let fields = Fields::create(draft, plan);
    let mut blocks = ArenaVec::new_in(draft.scratch);
    blocks.extend(draft.alive_blocks());
    move_accesses(draft, plan, &fields, &blocks);
    for mv in &uses.moves {
        if fields.local(mv.kind.slot()).is_some() {
            expand(draft, plan, &fields, *mv);
        }
    }
    // The split slots' addresses and offsets are unused now.
    for &block in &blocks {
        for index in 0..draft.block(block).insts.len() {
            let inst = draft.block(block).insts[index];
            if draft.is_removed(inst) || draft.inst_constant(inst).is_some() {
                continue;
            }
            let slot = match *draft.inst(inst) {
                | InstData::StackAddr { slot } => Some(slot),
                | InstData::Binary {
                    opcode: Opcode::PtrAdd,
                    args: [base, _],
                    ..
                } => derive(draft, base).map(|(slot, _)| slot),
                | _ => None,
            };
            if let Some(slot) = slot
                && fields
                    .local(slot)
                    .is_some_and(|local| !plan.slots[local].retained)
            {
                draft.remove_inst(inst);
            }
        }
    }
    for slot_plan in &plan.slots {
        if !slot_plan.retained {
            draft.remove_stack_slot(slot_plan.slot);
        }
    }
}

/// Points every load of a planned slot, and every store to a split one, at
/// the field's slot; a store to a retained slot also stores the shadow.
fn move_accesses(
    draft: &mut Draft<'_, '_>,
    plan: &Plan<'_>,
    fields: &Fields<'_>,
    blocks: &[Block],
) {
    for &block in blocks {
        let mut index = 0;
        while index < draft.block(block).insts.len() {
            let inst = draft.block(block).insts[index];
            index += 1;
            if draft.is_removed(inst) || draft.inst_constant(inst).is_some() {
                continue;
            }
            let data = *draft.inst(inst);
            let (InstData::Load { addr, .. }
            | InstData::Store {
                args: [_, addr], ..
            }) = data
            else {
                continue;
            };
            let Some((slot, Some(offset))) = derive(draft, addr) else {
                continue;
            };
            let Some(local) = fields.local(slot) else {
                continue;
            };
            let offset = u32::try_from(offset).expect("the access is inside the slot");
            let (field, field_align) = fields.address(plan, local, offset);
            match data {
                | InstData::Load {
                    ty,
                    flags,
                    align,
                    tag,
                    ..
                } => draft.replace_inst(
                    inst,
                    InstData::Load {
                        ty,
                        flags,
                        align,
                        tag,
                        addr: field,
                    },
                ),
                | InstData::Store {
                    ty,
                    args: [value, _],
                    ..
                } if plan.slots[local].retained => {
                    _ = draft.add_inst(block, index, store(ty, field_align, value, field), None);
                    index += 1;
                },
                | InstData::Store {
                    ty,
                    flags,
                    align,
                    tag,
                    args: [value, _],
                } => draft.replace_inst(
                    inst,
                    InstData::Store {
                        ty,
                        flags,
                        align,
                        tag,
                        args: [value, field],
                    },
                ),
                | _ => unreachable!("matched above"),
            }
        }
    }
}

/// Expands one `copy` or `fill` of the component field by field, keeping
/// the instruction itself where it writes a retained slot's memory (or
/// copies a retained slot's bytes out).
fn expand(draft: &mut Draft<'_, '_>, plan: &Plan<'_>, fields: &Fields<'_>, mv: super::Move) {
    let InstData::MemoryRange {
        align,
        args: [destination, source, _],
        ..
    } = *draft.inst(mv.inst)
    else {
        unreachable!("a move is a copy or a fill");
    };
    let retained = |slot: StackSlot| {
        fields
            .local(slot)
            .is_some_and(|local| plan.slots[local].retained)
    };
    // The fields of the range, on the side whose layout is planned (the
    // source's for a copy), and where each lands relative to the range.
    let (side, start) = match mv.kind {
        | MoveKind::Copy { from, .. } | MoveKind::CopyOut { from } => from,
        | MoveKind::CopyIn { to } | MoveKind::Fill { to, .. } => to,
    };
    let side_local = fields.local(side).expect("the move is planned");
    let mut moved = ArenaVec::new_in(draft.scratch);
    moved.extend(
        plan.slots[side_local]
            .fields
            .iter()
            .copied()
            .filter(|&(offset, _)| offset >= start && offset < start + mv.size)
            .map(|(offset, ty)| (offset - start, ty)),
    );
    // Constants go into the entry block before the position is found.
    let memory_base = match mv.kind {
        | MoveKind::Copy { to, from } if retained(to.0) && !retained(from.0) =>
            Some((fields.base(to.0), to.1, draft.stack_slot(to.0).align)),
        | MoveKind::CopyIn { .. } => Some((source, 0, align)),
        | MoveKind::CopyOut { from } if !retained(from.0) => Some((destination, 0, align)),
        | _ => None,
    };
    let mut constants = ArenaVec::with_capacity_in(moved.len(), draft.scratch);
    for &(relative, ty) in &moved {
        constants.push(match (mv.kind, memory_base) {
            | (MoveKind::Fill { byte, .. }, _) => Some(fill_constant(draft, ty, byte)),
            | (_, Some((_, base_offset, _))) if relative + base_offset != 0 => Some(
                draft.add_constant(Constant::Int(Type::I64, u128::from(relative + base_offset))),
            ),
            | _ => None,
        });
    }
    let at = draft
        .block(mv.block)
        .insts
        .iter()
        .position(|&inst| inst == mv.inst)
        .expect("the move is in its block");
    let mut cursor = Cursor {
        block: mv.block,
        at,
    };
    let keep = match mv.kind {
        | MoveKind::Copy { to, from } => {
            let (from_local, to_local) = (
                fields.local(from.0).expect("planned"),
                fields.local(to.0).expect("planned"),
            );
            // Every load before every store: the ranges may overlap.
            let mut values = ArenaVec::with_capacity_in(moved.len(), draft.scratch);
            for &(relative, ty) in &moved {
                let (address, field_align) = fields.address(plan, from_local, from.1 + relative);
                values.push(
                    cursor
                        .insert(draft, load(ty, field_align, address), Some(ty))
                        .expect("a load has a result"),
                );
            }
            for (index, (&(relative, ty), &value)) in moved.iter().zip(&values).enumerate() {
                let (address, field_align) = fields.address(plan, to_local, to.1 + relative);
                _ = cursor.insert(draft, store(ty, field_align, value, address), None);
                if let Some((base, base_offset, base_align)) = memory_base {
                    let address = cursor.offset(draft, base, constants[index]);
                    let field_align = align_at(base_align, base_offset + relative);
                    _ = cursor.insert(draft, store(ty, field_align, value, address), None);
                }
            }
            retained(to.0) && retained(from.0)
        },
        | MoveKind::CopyIn { to } => {
            let to_local = fields.local(to.0).expect("planned");
            for (index, &(relative, ty)) in moved.iter().enumerate() {
                let address = cursor.offset(draft, source, constants[index]);
                let value = cursor
                    .insert(
                        draft,
                        load(ty, align_at(align, relative), address),
                        Some(ty),
                    )
                    .expect("a load has a result");
                let (address, field_align) = fields.address(plan, to_local, to.1 + relative);
                _ = cursor.insert(draft, store(ty, field_align, value, address), None);
            }
            retained(to.0)
        },
        | MoveKind::CopyOut { from } if retained(from.0) => true,
        | MoveKind::CopyOut { from } => {
            let from_local = fields.local(from.0).expect("planned");
            for (index, &(relative, ty)) in moved.iter().enumerate() {
                let (address, field_align) = fields.address(plan, from_local, from.1 + relative);
                let value = cursor
                    .insert(draft, load(ty, field_align, address), Some(ty))
                    .expect("a load has a result");
                let address = cursor.offset(draft, destination, constants[index]);
                _ = cursor.insert(
                    draft,
                    store(ty, align_at(align, relative), value, address),
                    None,
                );
            }
            false
        },
        | MoveKind::Fill { to, .. } => {
            let to_local = fields.local(to.0).expect("planned");
            for (index, &(relative, ty)) in moved.iter().enumerate() {
                let (address, field_align) = fields.address(plan, to_local, to.1 + relative);
                let value = constants[index].expect("a fill has a constant per field");
                _ = cursor.insert(draft, store(ty, field_align, value, address), None);
            }
            retained(to.0)
        },
    };
    if !keep {
        draft.remove_inst(mv.inst);
    }
}

/// The new field slots of a component and their addresses.
struct Fields<'s> {
    /// Each planned slot's position in the plan, by slot index; `u32::MAX`
    /// for the others.
    local:     ArenaVec<'s, u32>,
    /// Each planned slot's first entry in `addresses`.
    starts:    ArenaVec<'s, u32>,
    /// Each field's address and alignment, slot by slot.
    addresses: ArenaVec<'s, (Value, Align)>,
    /// A retained slot's own address, for stores of split fields into it.
    bases:     ArenaVec<'s, Option<Value>>,
}

impl<'s> Fields<'s> {
    /// Adds the field slots and their `stack_addr`s at the top of the entry
    /// block.
    fn create(draft: &mut Draft<'_, 's>, plan: &Plan<'_>) -> Self {
        let scratch = draft.scratch;
        let mut fields = Self {
            local:     ArenaVec::with_capacity_in(draft.stack_slot_count(), scratch),
            starts:    ArenaVec::with_capacity_in(plan.slots.len(), scratch),
            addresses: ArenaVec::new_in(scratch),
            bases:     ArenaVec::with_capacity_in(plan.slots.len(), scratch),
        };
        fields.local.resize(draft.stack_slot_count(), u32::MAX);
        for (position, slot_plan) in plan.slots.iter().enumerate() {
            fields.local[slot_plan.slot.index()] =
                u32::try_from(position).expect("fewer than 2^32 slots");
            fields
                .starts
                .push(u32::try_from(fields.addresses.len()).expect("fewer than 2^32 fields"));
            let data = draft.stack_slot(slot_plan.slot);
            for &(offset, ty) in &slot_plan.fields {
                let align = align_at(data.align, offset);
                let field = draft.add_stack_slot(StackSlotData {
                    size: ty.bytes(),
                    align,
                });
                fields.addresses.push((stack_addr(draft, field), align));
            }
            fields.bases.push(
                slot_plan
                    .retained
                    .then(|| stack_addr(draft, slot_plan.slot)),
            );
        }
        // The new slots belong to no plan.
        fields.local.resize(draft.stack_slot_count(), u32::MAX);
        fields
    }

    fn local(&self, slot: StackSlot) -> Option<usize> {
        let local = *self.local.get(slot.index())?;
        (local != u32::MAX).then_some(local as usize)
    }

    /// The address and alignment of the field of planned slot `local` at
    /// `offset`.
    fn address(&self, plan: &Plan<'_>, local: usize, offset: u32) -> (Value, Align) {
        let field = plan.slots[local]
            .fields
            .binary_search_by_key(&offset, |&(start, _)| start)
            .expect("every access and moved field is a field");
        self.addresses[self.starts[local] as usize + field]
    }

    /// The address a retained slot keeps for stores of split fields.
    fn base(&self, slot: StackSlot) -> Value {
        self.bases[self.local(slot).expect("planned")].expect("the slot is retained")
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

fn stack_addr(draft: &mut Draft<'_, '_>, slot: StackSlot) -> Value {
    draft
        .add_inst(
            Draft::entry(),
            0,
            InstData::StackAddr { slot },
            Some(Type::Ptr),
        )
        .expect("`stack_addr` has a result")
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
    let constant = |opcode, bits| InstData::Const {
        opcode,
        ty,
        bits: Bits64::new(bits),
    };
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
        | Type::F16 | Type::BF16 | Type::F32 | Type::F64 => draft
            .add_inst(
                Draft::entry(),
                0,
                constant(
                    Opcode::Fconst,
                    u64::try_from(splat).expect("at most eight bytes"),
                ),
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
