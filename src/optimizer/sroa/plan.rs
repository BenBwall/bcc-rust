//! The uses of every stack slot, gathered in one scan, and the components
//! that can be split, with their fields. Read [`Uses::plans`] first, then
//! [`Component::plan`].

use super::{
    MAX_FIELDS,
    Move,
    MoveKind,
    Plan,
    SlotPlan,
};
use crate::{
    ir::{
        Block,
        Entity,
        Inst,
        InstData,
        InstFlags,
        Opcode,
        StackSlot,
        Type,
        Value,
        ValueDef,
    },
    optimizer::draft::{
        Constant,
        Draft,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// How the slots of one function are used.
pub(super) struct Uses<'s> {
    /// Union-find parents joining the slots that `copy`s connect.
    parent:           ArenaVec<'s, u32>,
    escaped:          ArenaVec<'s, bool>,
    /// Every load and store of a slot: the slot, offset, type and whether
    /// it is a load.
    accesses:         ArenaVec<'s, (StackSlot, u32, Type, bool)>,
    /// Every `copy` and `fill` that touches a slot.
    pub(super) moves: ArenaVec<'s, Move>,
}

impl<'s> Uses<'s> {
    /// Scans the live blocks, unreachable ones included.
    pub(super) fn collect(draft: &Draft<'_, 's>) -> Self {
        let slots = draft.stack_slot_count();
        let mut uses = Self {
            parent:   ArenaVec::with_capacity_in(slots, draft.scratch),
            escaped:  ArenaVec::with_capacity_in(slots, draft.scratch),
            accesses: ArenaVec::new_in(draft.scratch),
            moves:    ArenaVec::new_in(draft.scratch),
        };
        uses.parent
            .extend((0..slots).map(|index| u32::try_from(index).expect("fewer than 2^32 slots")));
        uses.escaped
            .extend((0..slots).map(|index| draft.is_stack_slot_removed(StackSlot::new(index))));
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
            // A derived address is judged where it is used; a variable
            // offset makes those uses escape.
            | InstData::StackAddr { .. }
            | InstData::Binary {
                opcode: Opcode::PtrAdd,
                ..
            } => {},
            | InstData::Load {
                ty, flags, addr, ..
            } => self.access(draft, ty, flags, addr, true),
            | InstData::Store {
                ty,
                flags,
                args: [value, addr],
                ..
            } => {
                self.escape(draft, value);
                self.access(draft, ty, flags, addr, false);
            },
            | InstData::MemoryRange {
                opcode,
                flags,
                args,
                ..
            } => self.range(draft, block, inst, opcode, flags, args),
            | _ => draft.for_each_operand(inst, |value| self.escape(draft, value)),
        }
    }

    /// Records a load or store through `addr`.
    fn access(
        &mut self,
        draft: &Draft<'_, 's>,
        ty: Type,
        flags: InstFlags,
        addr: Value,
        load: bool,
    ) {
        let Some(derived) = derive(draft, addr) else {
            return;
        };
        match inside(draft, derived, ty.bytes()) {
            | Some((slot, offset)) if !flags.contains(InstFlags::VOLATILE) && ty != Type::I1 =>
                self.accesses.push((slot, offset, ty, load)),
            | _ => self.escaped[derived.0.index()] = true,
        }
    }

    /// Records a `copy` or `fill` of a constant range inside slots.
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
            | Some(Constant::Int(_, bits)) => u32::try_from(bits).ok().filter(|&size| size > 0),
            | _ => None,
        }
        .filter(|_| !flags.contains(InstFlags::VOLATILE));
        let inside = |derived: Option<(StackSlot, Option<i64>)>| {
            size.and_then(|size| inside(draft, derived?, size))
        };
        let push = |uses: &mut Self, kind| {
            uses.moves.push(Move {
                block,
                inst,
                size: size.expect("a recorded move has a size"),
                kind,
            });
        };
        let to = derive(draft, destination);
        if opcode == Opcode::Fill {
            match (inside(to), draft.constant(source)) {
                | (Some(to), Some(Constant::Int(_, byte))) => push(
                    self,
                    MoveKind::Fill {
                        to,
                        byte: u8::try_from(byte & 0xFF).expect("masked to a byte"),
                    },
                ),
                | _ => self.escape(draft, destination),
            }
            return;
        }
        let from = derive(draft, source);
        match (to, from) {
            | (None, None) => {},
            | (Some(_), Some(_)) => match (inside(to), inside(from)) {
                | (Some(to), Some(from)) => {
                    self.union(to.0, from.0);
                    push(self, MoveKind::Copy { to, from });
                },
                | _ => {
                    self.escape(draft, destination);
                    self.escape(draft, source);
                },
            },
            | (Some(_), None) => match inside(to) {
                | Some(to) => push(self, MoveKind::CopyIn { to }),
                | None => self.escape(draft, destination),
            },
            | (None, Some(_)) => match inside(from) {
                | Some(from) => push(self, MoveKind::CopyOut { from }),
                | None => self.escape(draft, source),
            },
        }
    }

    /// Marks the slot `value` points into, if any, as escaped.
    fn escape(&mut self, draft: &Draft<'_, 's>, value: Value) {
        if draft.value_type(draft.resolve(value)) == Type::Ptr
            && let Some((slot, _)) = derive(draft, value)
        {
            self.escaped[slot.index()] = true;
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
            self.parent[high.index()] = u32::try_from(low.index()).expect("fewer than 2^32 slots");
        }
    }

    /// The components worth splitting, with their fields.
    pub(super) fn plans(&self, draft: &Draft<'_, 's>) -> ArenaVec<'s, Plan<'s>> {
        let scratch = draft.scratch;
        let slots = self.parent.len();
        let mut broken = ArenaVec::with_capacity_in(slots, scratch);
        broken.resize(slots, false);
        let mut members = ArenaVec::with_capacity_in(slots, scratch);
        for index in 0..slots {
            let slot = StackSlot::new(index);
            let root = self.find(slot);
            broken[root.index()] |= self.escaped[index];
            members.push((root, slot));
        }
        members.sort_unstable_by_key(|&(root, slot)| (root.index(), slot.index()));
        let mut accesses = ArenaVec::with_capacity_in(self.accesses.len(), scratch);
        accesses.extend(
            self.accesses
                .iter()
                .map(|&(slot, offset, ty, load)| (self.find(slot), slot, offset, ty, load)),
        );
        accesses.sort_unstable_by_key(|&(root, slot, offset, ty, load)| {
            (root.index(), slot.index(), offset, ty as u8, load)
        });
        let mut moves = ArenaVec::with_capacity_in(self.moves.len(), scratch);
        moves.extend(self.moves.iter().map(|&mv| (self.find(mv.kind.slot()), mv)));
        moves.sort_by_key(|&(root, _)| root.index());
        let mut local = ArenaVec::with_capacity_in(slots, scratch);
        local.resize(slots, u32::MAX);
        let mut plans = ArenaVec::new_in(scratch);
        let (mut next_member, mut next_access, mut next_move) = (0, 0, 0);
        while next_member < members.len() {
            let root = members[next_member].0;
            let group = |start: usize, len: usize, key: &dyn Fn(usize) -> StackSlot| {
                let mut start = start;
                while start < len && key(start).index() < root.index() {
                    start += 1;
                }
                let mut end = start;
                while end < len && key(end) == root {
                    end += 1;
                }
                (start, end)
            };
            let (member_start, member_end) =
                group(next_member, members.len(), &|index| members[index].0);
            let (access_start, access_end) =
                group(next_access, accesses.len(), &|index| accesses[index].0);
            let (move_start, move_end) = group(next_move, moves.len(), &|index| moves[index].0);
            next_member = member_end;
            next_access = access_end;
            next_move = move_end;
            if broken[root.index()] {
                continue;
            }
            for (position, &(_, slot)) in members[member_start..member_end].iter().enumerate() {
                local[slot.index()] = u32::try_from(position).expect("fewer than 2^32 slots");
            }
            let component = Component {
                draft,
                local: &local,
                members: &members[member_start..member_end],
                accesses: &accesses[access_start..access_end],
                moves: &moves[move_start..move_end],
            };
            if let Some(plan) = component.plan(scratch) {
                plans.push(plan);
            }
        }
        plans
    }
}

impl MoveKind {
    /// A slot the move touches; both of a copy's slots are in one component.
    pub(super) const fn slot(self) -> StackSlot {
        match self {
            | Self::Copy { to, .. } | Self::CopyIn { to } | Self::Fill { to, .. } => to.0,
            | Self::CopyOut { from } => from.0,
        }
    }
}

/// One component's uses while its fields are worked out.
struct Component<'a, 'b, 's> {
    draft:    &'a Draft<'b, 's>,
    /// Each member's position in `members`, by slot index.
    local:    &'a [u32],
    members:  &'a [(StackSlot, StackSlot)],
    accesses: &'a [(StackSlot, StackSlot, u32, Type, bool)],
    moves:    &'a [(StackSlot, Move)],
}

/// Two fields overlap, or one straddles a moved range: the component stays.
struct Conflict;

impl<'s> Component<'_, '_, 's> {
    /// The fields of every member, or `None` if the component stays whole or
    /// gains nothing from a split.
    fn plan(&self, scratch: &'s Bump) -> Option<Plan<'s>> {
        let count = self.members.len();
        let mut fields: ArenaVec<'s, ArenaVec<'s, (u32, Type)>> =
            ArenaVec::with_capacity_in(count, scratch);
        fields.extend((0..count).map(|_| ArenaVec::new_in(scratch)));
        for &(_, slot, offset, ty, _) in self.accesses {
            _ = insert_field(&mut fields[self.at(slot)], offset, ty).ok()?;
        }
        let (copied_in, copied_out) = self.reach(scratch);
        self.spread(&mut fields, scratch).ok()?;
        for (position, &(_, slot)) in self.members.iter().enumerate() {
            if copied_out[position] {
                fill_gaps(&mut fields[position], self.draft.stack_slot(slot).size);
            }
        }
        self.spread(&mut fields, scratch).ok()?;
        if fields.iter().any(|list| list.len() > MAX_FIELDS) {
            return None;
        }
        let retained = |position: usize| copied_in[position] && copied_out[position];
        for &(_, mv) in self.moves {
            let (slot, start) = match mv.kind {
                | MoveKind::Copy { to, from } => {
                    // Storing a split slot's fields into a retained slot's
                    // memory needs every byte of the range.
                    if retained(self.at(to.0))
                        && !retained(self.at(from.0))
                        && !covers(&fields[self.at(from.0)], from.1, mv.size)
                    {
                        return None;
                    }
                    continue;
                },
                | MoveKind::CopyIn { to } => to,
                | MoveKind::CopyOut { from } => {
                    if !retained(self.at(from.0))
                        && !covers(&fields[self.at(from.0)], from.1, mv.size)
                    {
                        return None;
                    }
                    from
                },
                | MoveKind::Fill { to, byte } => {
                    let list = &fields[self.at(to.0)];
                    if within(list, to.1, mv.size).any(|(_, ty)| !fillable(ty, byte)) {
                        return None;
                    }
                    to
                },
            };
            if straddles(&fields[self.at(slot)], start, mv.size) {
                return None;
            }
        }
        if !self.worth_splitting(&fields, &retained) {
            return None;
        }
        let mut slots = ArenaVec::with_capacity_in(count, scratch);
        for (position, (&(_, slot), fields)) in self.members.iter().zip(fields).enumerate() {
            slots.push(SlotPlan {
                slot,
                retained: retained(position),
                fields,
            });
        }
        Some(Plan { slots })
    }

    /// Which members other memory's bytes can reach through copies, and
    /// which can reach other memory.
    fn reach(&self, scratch: &'s Bump) -> (ArenaVec<'s, bool>, ArenaVec<'s, bool>) {
        let count = self.members.len();
        let mut copied_in = ArenaVec::with_capacity_in(count, scratch);
        copied_in.resize(count, false);
        let mut copied_out = ArenaVec::with_capacity_in(count, scratch);
        copied_out.resize(count, false);
        for &(_, mv) in self.moves {
            match mv.kind {
                | MoveKind::CopyIn { to } => copied_in[self.at(to.0)] = true,
                | MoveKind::CopyOut { from } => copied_out[self.at(from.0)] = true,
                | MoveKind::Copy { .. } | MoveKind::Fill { .. } => {},
            }
        }
        loop {
            let mut changed = false;
            for &(_, mv) in self.moves {
                if let MoveKind::Copy { to, from } = mv.kind {
                    let (to, from) = (self.at(to.0), self.at(from.0));
                    if copied_in[from] && !copied_in[to] {
                        copied_in[to] = true;
                        changed = true;
                    }
                    if copied_out[to] && !copied_out[from] {
                        copied_out[from] = true;
                        changed = true;
                    }
                }
            }
            if !changed {
                return (copied_in, copied_out);
            }
        }
    }

    /// Spreads the fields inside each copied range to the other side until
    /// no copy adds one.
    fn spread(
        &self,
        fields: &mut ArenaVec<'s, ArenaVec<'s, (u32, Type)>>,
        scratch: &'s Bump,
    ) -> Result<(), Conflict> {
        let mut moved = ArenaVec::new_in(scratch);
        loop {
            let mut changed = false;
            for &(_, mv) in self.moves {
                let MoveKind::Copy { to, from } = mv.kind else {
                    continue;
                };
                for (source, target) in [(from, to), (to, from)] {
                    moved.clear();
                    let list = &fields[self.at(source.0)];
                    if straddles(list, source.1, mv.size) {
                        return Err(Conflict);
                    }
                    moved.extend(within(list, source.1, mv.size));
                    for &(offset, ty) in &moved {
                        let shifted = offset - source.1 + target.1;
                        changed |= insert_field(&mut fields[self.at(target.0)], shifted, ty)?;
                    }
                }
                if fields[self.at(to.0)].len() > MAX_FIELDS {
                    return Err(Conflict);
                }
            }
            if !changed {
                return Ok(());
            }
        }
    }

    /// Whether the split removes anything: a lone scalar slot is
    /// `promote`'s, and retained slots that nothing loads stay as they are.
    fn worth_splitting(
        &self,
        fields: &[ArenaVec<'s, (u32, Type)>],
        retained: &dyn Fn(usize) -> bool,
    ) -> bool {
        if self.members.len() == 1 && self.moves.is_empty() {
            let size = self.draft.stack_slot(self.members[0].1).size;
            return match fields[0].as_slice() {
                | [] => false,
                | [(_, ty)] => ty.bytes() != size,
                | _ => true,
            };
        }
        (0..self.members.len()).any(|position| !retained(position))
            || self.accesses.iter().any(|&(_, _, _, _, load)| load)
    }

    fn at(&self, slot: StackSlot) -> usize {
        self.local[slot.index()] as usize
    }
}

/// Adds the field `(offset, ty)` to a sorted list; returns whether it was
/// new.
fn insert_field(
    list: &mut ArenaVec<'_, (u32, Type)>,
    offset: u32,
    ty: Type,
) -> Result<bool, Conflict> {
    let position = list.partition_point(|&(start, _)| start < offset);
    if let Some(&(start, existing)) = list.get(position)
        && start == offset
    {
        return if existing == ty {
            Ok(false)
        } else {
            Err(Conflict)
        };
    }
    let end = offset + ty.bytes();
    let overlaps_previous = position
        .checked_sub(1)
        .is_some_and(|previous| list[previous].0 + list[previous].1.bytes() > offset);
    let overlaps_next = list.get(position).is_some_and(|&(start, _)| start < end);
    if overlaps_previous || overlaps_next {
        return Err(Conflict);
    }
    list.insert(position, (offset, ty));
    Ok(true)
}

/// The fields that start inside `start..start + size`.
fn within(list: &[(u32, Type)], start: u32, size: u32) -> impl Iterator<Item = (u32, Type)> + '_ {
    let first = list.partition_point(|&(offset, _)| offset < start);
    list[first..]
        .iter()
        .copied()
        .take_while(move |&(offset, _)| offset < start + size)
}

/// Whether a field crosses either end of `start..start + size`.
fn straddles(list: &[(u32, Type)], start: u32, size: u32) -> bool {
    let end = start + size;
    list.iter().any(|&(offset, ty)| {
        let field_end = offset + ty.bytes();
        (offset < start && field_end > start) || (offset < end && field_end > end)
    })
}

/// Whether the fields cover every byte of `start..start + size`.
fn covers(list: &[(u32, Type)], start: u32, size: u32) -> bool {
    let mut next = start;
    for (offset, ty) in within(list, start, size) {
        if offset != next {
            return false;
        }
        next = offset + ty.bytes();
    }
    next >= start + size
}

/// Covers the bytes of `0..size` that no field covers with aligned integer
/// fields of at most eight bytes.
fn fill_gaps(list: &mut ArenaVec<'_, (u32, Type)>, size: u32) {
    let mut start = 0;
    let mut position = 0;
    while start < size {
        let end = list.get(position).map_or(size, |&(offset, _)| offset);
        while start < end {
            let ty = [Type::I64, Type::I32, Type::I16, Type::I8]
                .into_iter()
                .find(|ty| start.is_multiple_of(ty.bytes()) && start + ty.bytes() <= end)
                .expect("an i8 always fits");
            list.insert(position, (start, ty));
            position += 1;
            start += ty.bytes();
        }
        if let Some(&(offset, ty)) = list.get(position) {
            start = offset + ty.bytes();
            position += 1;
        }
    }
}

/// Whether a `fill` with `byte` can be stored into a field of type `ty` as a
/// constant.
const fn fillable(ty: Type, byte: u8) -> bool {
    match ty {
        | Type::Ptr => byte == 0,
        | Type::F80 | Type::F128 => false,
        | _ => true,
    }
}

/// The slot and offset of `derived` if the `size` bytes there lie inside
/// the slot.
fn inside(
    draft: &Draft<'_, '_>,
    (slot, offset): (StackSlot, Option<i64>),
    size: u32,
) -> Option<(StackSlot, u32)> {
    let offset = u32::try_from(offset?).ok()?;
    (u64::from(offset) + u64::from(size) <= u64::from(draft.stack_slot(slot).size))
        .then_some((slot, offset))
}

/// The slot and constant offset `addr` points to, following `ptr_add`s
/// back to a `stack_addr`; the offset is `None` if one is not constant.
pub(super) fn derive(draft: &Draft<'_, '_>, addr: Value) -> Option<(StackSlot, Option<i64>)> {
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
