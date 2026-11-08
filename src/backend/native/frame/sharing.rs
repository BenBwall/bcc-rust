//! Stack slot sharing: IR stack slots that are never in use at the same
//! time share one frame offset, as LLVM's `StackColoring` shares allocas.
//!
//! The IR carries no lifetime markers, so a slot's lifetime is computed from
//! its accesses, and only for slots whose address never leaves the function's
//! view: every value derived from its `stack_addr` (through `ptr_add`,
//! `subobject` and `select`) is only the address of a `load`, a `store`, an
//! atomic access, or an operand of `copy` or `fill`. A slot whose address
//! escapes (to a call, into memory, into a comparison or an integer, across
//! an edge) may be reached through a pointer anywhere until its block ends
//! (C99 §6.2.4p5, p. 32; PDF p. 44), which the IR does not record, so it
//! keeps an offset of its own for the whole function.
//!
//! A local slot is *live* where its contents may still be read: at a point
//! that some access of it reaches, and from which some path reaches a read
//! of it without passing a write of the whole slot (a store, `copy` or
//! `fill` at its start at least as large as it), as a register is live
//! until its next definition. Before every access the slot holds nothing
//! it wrote. Its lifetime is the instructions it is live after or that
//! access it. If two slots' lifetimes are disjoint, neither writes where
//! the other holds a value it will read, so they can share. Each slot is given
//! the first offset group, largest slots first, whose members' lifetimes it
//! misses; each group's offset has the size and alignment of its largest
//! member. A slot aligned above 16 bytes keeps its own offset, since its
//! address is computed at run time.
//!
//! A function that calls one that returns twice (`setjmp`) shares nothing: a
//! slot read after the second return may be dead on the path to the
//! `longjmp`, which the graph does not show.

use crate::{
    ir::{
        Block,
        Body,
        ControlFlowGraph,
        Entity,
        Inst,
        InstData,
        Module,
        Opcode,
        StackSlot,
        Type,
        Value,
        ValueDef,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// No root, in the table of each value's slot.
const NONE: u32 = u32::MAX;

/// An unknown offset from a slot's start.
const UNKNOWN: i64 = i64::MIN;

/// The most offset groups a slot is tried against before it starts its own,
/// which bounds the work of a function with very many slots.
const TRIES: usize = 256;

/// The most instruction visits the lifetimes may take, 64 slots per visit;
/// slots beyond the budget keep their own offsets.
const WORK: usize = 1 << 24;

/// The offset group of each IR stack slot of `body`: the slot whose offset
/// it takes, itself unless it shares another's. `rpo` is the reverse
/// post-order of the reachable blocks; `indirect_returns_twice` says whether
/// an indirect call may return twice.
pub(crate) fn share_stack_slots<'a>(
    module: &Module<'_>,
    body: &Body<'_>,
    cfg: &ControlFlowGraph<'_>,
    rpo: &[Block],
    indirect_returns_twice: bool,
    arena: &'a Bump,
) -> ArenaVec<'a, u32> {
    let count = body.stack_slot_count();
    let mut homes = ArenaVec::with_capacity_in(count, arena);
    homes.extend((0..count).map(index));
    if count < 2 || returns_twice(module, body, indirect_returns_twice) {
        return homes;
    }
    let Some(mut accesses) = local_accesses(body, arena) else {
        return homes;
    };
    // Slots beyond the work budget keep their offsets.
    let chunks = (WORK / body.inst_count().max(1)).max(1);
    accesses.locals.truncate(64 * chunks);
    let solved = accesses.locals.len();
    accesses
        .list
        .retain(|&(local, ..)| (local as usize) < solved);
    let lives = Lifetimes::compute(body, cfg, rpo, &accesses, arena);
    // Largest first, so a group's first member is its largest.
    let mut order = ArenaVec::with_capacity_in(solved, arena);
    order.extend(0..solved);
    order.sort_by_key(|&local| {
        let slot = accesses.locals[local];
        (core::cmp::Reverse(slot_size(body, slot)), slot)
    });
    let mut groups: ArenaVec<'_, (u32, ArenaVec<'_, Range>)> = ArenaVec::new_in(arena);
    for local in order {
        let slot = accesses.locals[local];
        let ranges = lives.ranges(local);
        let group = groups
            .iter()
            .take(TRIES)
            .position(|(_, members)| !intersect(members, ranges));
        if let Some(group) = group {
            let (home, members) = &mut groups[group];
            homes[slot as usize] = *home;
            *members = union(members, ranges, arena);
        } else {
            let mut members = ArenaVec::with_capacity_in(ranges.len(), arena);
            members.extend_from_slice(ranges);
            groups.push((slot, members));
        }
    }
    homes
}

/// Whether `body` calls a function that may return twice.
fn returns_twice(module: &Module<'_>, body: &Body<'_>, indirect: bool) -> bool {
    body.blocks()
        .flat_map(|block| body.block_insts(block))
        .any(|&inst| match *body.inst(inst) {
            | InstData::Call { func, .. } => module.returns_twice(func),
            | InstData::CallIndirect { .. } => indirect,
            | ref data => data.opcode() == Opcode::SjljSetjmp,
        })
}

fn slot_size(body: &Body<'_>, slot: u32) -> u32 {
    body.stack_slot(StackSlot::new(slot as usize)).size
}

/// A half-open range of instruction positions, numbered across the blocks
/// in body order.
type Range = (u32, u32);

/// What an instruction does to a slot: bits of [`READ`], [`WRITE`] and
/// [`KILL`].
type Effect = u8;

/// The instruction may read the slot.
const READ: Effect = 1;
/// The instruction may write the slot.
const WRITE: Effect = 2;
/// The instruction writes the whole slot.
const KILL: Effect = 4;

/// The slots whose address stays local, and their accesses.
struct Accesses<'a> {
    /// The local slots, ascending.
    locals: ArenaVec<'a, u32>,
    /// Each access as `(local, instruction, effect)`.
    list:   ArenaVec<'a, (u32, Inst, Effect)>,
}

/// Finds the slots whose derived addresses are only accessed through, and
/// their accesses; `None` when there are fewer than two.
fn local_accesses<'a>(body: &Body<'_>, arena: &'a Bump) -> Option<Accesses<'a>> {
    let values = body.value_count();
    // Each value's users, as a compressed table.
    let mut starts = ArenaVec::with_capacity_in(values + 1, arena);
    starts.resize(values + 1, 0_u32);
    for block in body.blocks() {
        for &inst in body.block_insts(block) {
            body.visit_operands(inst, |value| starts[value.index() + 1] += 1);
        }
    }
    for value in 0..values {
        starts[value + 1] += starts[value];
    }
    let mut fill = ArenaVec::with_capacity_in(values, arena);
    fill.extend_from_slice(&starts[..values]);
    let mut users = ArenaVec::with_capacity_in(starts[values] as usize, arena);
    users.resize(starts[values] as usize, Inst::new(0));
    for block in body.blocks() {
        for &inst in body.block_insts(block) {
            body.visit_operands(inst, |value| {
                users[fill[value.index()] as usize] = inst;
                fill[value.index()] += 1;
            });
        }
    }
    let slots = body.stack_slot_count();
    let mut escaped = ArenaVec::with_capacity_in(slots, arena);
    for (_, data) in body.stack_slots() {
        escaped.push(data.align.bytes() > 16);
    }
    // Each derived value's slot and its offset into it.
    let mut root = ArenaVec::with_capacity_in(values, arena);
    root.resize(values, (NONE, UNKNOWN));
    let mut work = ArenaVec::new_in(arena);
    for block in body.blocks() {
        for &inst in body.block_insts(block) {
            if let InstData::StackAddr { slot } = *body.inst(inst)
                && let Some(result) = body.inst_result(inst)
            {
                root[result.index()] = (index(slot.index()), 0);
                work.push(result);
            }
        }
    }
    let mut found = ArenaVec::new_in(arena);
    while let Some(value) = work.pop() {
        let (slot, offset) = root[value.index()];
        let range = starts[value.index()] as usize..starts[value.index() + 1] as usize;
        for &user in &users[range] {
            match use_of(body, user, value, offset, slot_size(body, slot)) {
                | Use::Access(effect) => found.push((slot, user, effect)),
                | Use::Derive(moved) => {
                    let derived = body.inst_result(user).expect("derivations have results");
                    let offset = match (offset, moved) {
                        | (UNKNOWN, _) | (_, None) => UNKNOWN,
                        | (offset, Some(moved)) => offset.checked_add(moved).unwrap_or(UNKNOWN),
                    };
                    match root[derived.index()] {
                        | (NONE, _) => {
                            root[derived.index()] = (slot, offset);
                            work.push(derived);
                        },
                        // The other operand of a `select` into the slot,
                        // whose users are already followed.
                        | (other, _) if other == slot => {},
                        // A pointer to either of two slots: an access
                        // through it could touch both.
                        | (other, _) => {
                            escaped[other as usize] = true;
                            escaped[slot as usize] = true;
                        },
                    }
                },
                | Use::Escape => escaped[slot as usize] = true,
            }
        }
    }
    let mut local_of = ArenaVec::with_capacity_in(slots, arena);
    let mut locals = ArenaVec::new_in(arena);
    for (slot, &escaped) in escaped.iter().enumerate() {
        if escaped {
            local_of.push(NONE);
        } else {
            local_of.push(index(locals.len()));
            locals.push(index(slot));
        }
    }
    if locals.len() < 2 {
        return None;
    }
    let mut list = ArenaVec::with_capacity_in(found.len(), arena);
    list.extend(
        found
            .iter()
            .filter(|&&(slot, ..)| local_of[slot as usize] != NONE)
            .map(|&(slot, inst, effect)| (local_of[slot as usize], inst, effect)),
    );
    Some(Accesses { locals, list })
}

/// How an instruction uses a value derived from a slot's address.
enum Use {
    /// It reads or writes memory at the address.
    Access(Effect),
    /// Its result is another address into the same slot, this many bytes
    /// further if known.
    Derive(Option<i64>),
    /// It lets the address go where the analysis does not follow.
    Escape,
}

/// How `user` uses `value`, an address `offset` bytes into a slot of `size`
/// bytes.
fn use_of(body: &Body<'_>, user: Inst, value: Value, offset: i64, size: u32) -> Use {
    let write = |bytes: Option<u64>| {
        if offset == 0 && bytes.is_some_and(|bytes| bytes >= u64::from(size)) {
            WRITE | KILL
        } else {
            WRITE
        }
    };
    match *body.inst(user) {
        | InstData::Load { .. } => Use::Access(READ),
        | InstData::Store {
            ty,
            args: [stored, _],
            ..
        } if stored != value => Use::Access(write(Some(u64::from(ty.bytes())))),
        | InstData::MemoryRange {
            opcode,
            args: [destination, source, bytes],
            ..
        } if bytes != value && (opcode == Opcode::Copy || source != value) => {
            let mut effect = 0;
            if destination == value {
                effect |= write(constant(body, bytes));
            }
            if source == value {
                effect |= READ;
            }
            Use::Access(effect)
        },
        | InstData::Atomic { args, .. }
            if body
                .value_list(args)
                .iter()
                .skip(1)
                .all(|&arg| arg != value) =>
            Use::Access(READ | WRITE),
        | InstData::Binary {
            opcode: Opcode::PtrAdd,
            args: [_, moved],
            ..
        } if moved != value =>
            Use::Derive(constant(body, moved).map(|bits| i64::from_ne_bytes(bits.to_ne_bytes()))),
        | InstData::Binary {
            opcode: Opcode::Subobject,
            args: [_, bytes],
            ..
        } if bytes != value => Use::Derive(Some(0)),
        | InstData::Select {
            args: [condition, ..],
            ..
        } if condition != value => Use::Derive(None),
        | _ => Use::Escape,
    }
}

/// The bits of `value` if it is an `i64` constant.
fn constant(body: &Body<'_>, value: Value) -> Option<u64> {
    let ValueDef::Result(inst) = body.value_def(value) else {
        return None;
    };
    if body.value_type(value) != Type::I64 {
        return None;
    }
    body.const_bits(body.inst(inst))
        .and_then(|bits| u64::try_from(bits).ok())
}

/// Where each local slot's lifetime lies, as sorted, disjoint ranges.
struct Lifetimes<'a> {
    /// Each local's ranges, in `ranges` from `starts[local]`.
    starts: ArenaVec<'a, u32>,
    ranges: ArenaVec<'a, Range>,
}

impl<'a> Lifetimes<'a> {
    /// Solves each local slot's lifetime, 64 slots at a time: what is live
    /// out of each block, then backward through each block's instructions.
    fn compute(
        body: &Body<'_>,
        cfg: &ControlFlowGraph<'_>,
        rpo: &[Block],
        accesses: &Accesses<'_>,
        arena: &'a Bump,
    ) -> Self {
        let blocks = body.block_count();
        let insts = body.inst_count();
        // Instruction positions, and each block's first one.
        let mut position = ArenaVec::with_capacity_in(insts, arena);
        position.resize(insts, 0_u32);
        let mut first = ArenaVec::with_capacity_in(blocks + 1, arena);
        let mut next = 0_u32;
        for block in body.blocks() {
            first.push(next);
            for &inst in body.block_insts(block) {
                position[inst.index()] = next;
                next += 1;
            }
        }
        first.push(next);
        let locals = accesses.locals.len();
        // The accesses by position, descending, as a backward walk meets
        // them.
        let mut by_position = ArenaVec::with_capacity_in(accesses.list.len(), arena);
        by_position.extend(
            accesses
                .list
                .iter()
                .map(|&(local, inst, effect)| (position[inst.index()], local, effect)),
        );
        by_position.sort_unstable_by(|left, right| right.cmp(left));
        let mut post_order = ArenaVec::with_capacity_in(rpo.len(), arena);
        post_order.extend(rpo.iter().rev().copied());
        let mut found = ArenaVec::new_in(arena);
        let mut kills = ArenaVec::with_capacity_in(blocks, arena);
        let mut gens = ArenaVec::with_capacity_in(blocks, arena);
        let mut touched = ArenaVec::with_capacity_in(blocks, arena);
        let mut live_out = ArenaVec::with_capacity_in(blocks, arena);
        let mut reached_in = ArenaVec::with_capacity_in(blocks, arena);
        let mut block_effects = ArenaVec::new_in(arena);
        let mut open_end = [0_u32; 64];
        for chunk in (0..locals).step_by(64) {
            let chunk = Chunk {
                first:       chunk,
                by_position: &by_position,
            };
            // Each block as `live_in = gen | (live_out & !kill)`, composed
            // from its last instruction to its first, and the slots it
            // accesses.
            for table in [
                &mut kills,
                &mut gens,
                &mut touched,
                &mut live_out,
                &mut reached_in,
            ] {
                table.clear();
                table.resize(blocks, 0_u64);
            }
            let mut cursor = 0;
            for block in body.blocks().rev() {
                let b = block.index();
                while let Some((_, effect)) = chunk.next(&mut cursor, first[b]) {
                    gens[b] = (gens[b] & !effect.kills) | effect.reads;
                    kills[b] |= effect.kills;
                    touched[b] |= effect.reads | effect.writes;
                }
            }
            // Backward: live out of a block. Forward: reached by an access.
            fixed_point(&post_order, &mut live_out, |live, block| {
                cfg.successors(block).iter().fold(0, |bits, &successor| {
                    let s = successor.index();
                    bits | gens[s] | (live[s] & !kills[s])
                })
            });
            fixed_point(rpo, &mut reached_in, |reached, block| {
                cfg.predecessors(block)
                    .iter()
                    .fold(0, |bits, &predecessor| {
                        bits | reached[predecessor.index()] | touched[predecessor.index()]
                    })
            });
            // Backward through each block: a lifetime holds an instruction
            // that accesses its slot, or after which the slot is live and
            // has been accessed.
            let mut cursor = 0;
            for block in body.blocks().rev() {
                let b = block.index();
                block_effects.clear();
                while let Some(entry) = chunk.next(&mut cursor, first[b]) {
                    block_effects.push((entry, 0_u64));
                }
                // The slots each access is the block's first of.
                let mut seen = reached_in[b];
                for ((_, effect), firsts) in block_effects.iter_mut().rev() {
                    *firsts = (effect.reads | effect.writes) & !seen;
                    seen |= *firsts;
                }
                let mut live = live_out[b];
                let mut reached = reached_in[b] | touched[b];
                let mut open = 0_u64;
                let mut pending = block_effects.iter().copied().peekable();
                for at in (first[b]..first[b + 1]).rev() {
                    let ((_, effect), firsts) = pending
                        .next_if(|&((position, _), _)| position == at)
                        .unwrap_or_default();
                    let present = (live & reached) | effect.reads | effect.writes;
                    for bit in bits(present & !open) {
                        open_end[bit] = at + 1;
                    }
                    for bit in bits(open & !present) {
                        found.push((index(chunk.first + bit), at + 1, open_end[bit]));
                    }
                    open = present;
                    live = (live & !effect.kills) | effect.reads;
                    reached &= !firsts;
                }
                for bit in bits(open) {
                    found.push((index(chunk.first + bit), first[b], open_end[bit]));
                }
            }
        }
        found.sort_unstable();
        let mut starts = ArenaVec::with_capacity_in(locals + 1, arena);
        let mut ranges: ArenaVec<'_, Range> = ArenaVec::with_capacity_in(found.len(), arena);
        let mut entry = 0;
        for local in 0..locals {
            starts.push(index(ranges.len()));
            let begin = ranges.len();
            while entry < found.len() && found[entry].0 as usize == local {
                let (_, start, end) = found[entry];
                let joins = ranges.len() > begin;
                match ranges.last_mut() {
                    | Some(last) if joins && last.1 == start => last.1 = end,
                    | _ => ranges.push((start, end)),
                }
                entry += 1;
            }
        }
        starts.push(index(ranges.len()));
        Self { starts, ranges }
    }

    /// The ranges of local slot `local`.
    fn ranges(&self, local: usize) -> &[Range] {
        &self.ranges[self.starts[local] as usize..self.starts[local + 1] as usize]
    }
}

/// Solves `value[b] = OR value[b], step(value, b)` over `order` to its least
/// fixed point.
fn fixed_point(order: &[Block], value: &mut [u64], step: impl Fn(&[u64], Block) -> u64) {
    let mut changed = true;
    while changed {
        changed = false;
        for &block in order {
            let bits = value[block.index()] | step(value, block);
            if bits != value[block.index()] {
                value[block.index()] = bits;
                changed = true;
            }
        }
    }
}

/// What one instruction does to the 64 slots of a chunk, one bit each.
#[derive(Clone, Copy, Default)]
struct Effect64 {
    reads:  u64,
    writes: u64,
    /// Whole writes of slots the instruction does not also read.
    kills:  u64,
}

/// The 64 local slots from `first` on, and every access by descending
/// position.
struct Chunk<'s> {
    first:       usize,
    by_position: &'s [(u32, u32, Effect)],
}

impl Chunk<'_> {
    /// The next instruction at or after `start` that accesses the chunk's
    /// slots, from `by_position[*cursor]` on, and what it does to them.
    fn next(&self, cursor: &mut usize, start: u32) -> Option<(u32, Effect64)> {
        let mut found: Option<(u32, Effect64)> = None;
        while let Some(&(at, local, effect)) = self.by_position.get(*cursor)
            && at >= start
            && found.is_none_or(|(position, _)| position == at)
        {
            *cursor += 1;
            let Some(offset) = (local as usize)
                .checked_sub(self.first)
                .filter(|&offset| offset < 64)
            else {
                continue;
            };
            let bit = 1_u64 << offset;
            let entry = &mut found.get_or_insert((at, Effect64::default())).1;
            let pick = |flag: Effect| if effect & flag != 0 { bit } else { 0 };
            entry.reads |= pick(READ);
            entry.writes |= pick(WRITE);
            entry.kills |= pick(KILL);
        }
        found.map(|(at, mut effect)| {
            effect.kills &= !effect.reads;
            (at, effect)
        })
    }
}

/// The indices of the set bits of `bits`, ascending.
fn bits(mut bits: u64) -> impl Iterator<Item = usize> {
    core::iter::from_fn(move || {
        (bits != 0).then(|| {
            let bit = bits.trailing_zeros() as usize;
            bits &= bits - 1;
            bit
        })
    })
}

/// Whether two sorted lists of disjoint ranges overlap.
fn intersect(left: &[Range], right: &[Range]) -> bool {
    let (mut i, mut j) = (0, 0);
    while i < left.len() && j < right.len() {
        if left[i].1 <= right[j].0 {
            i += 1;
        } else if right[j].1 <= left[i].0 {
            j += 1;
        } else {
            return true;
        }
    }
    false
}

/// The union of two sorted lists of disjoint ranges that do not overlap.
fn union<'a>(left: &[Range], right: &[Range], arena: &'a Bump) -> ArenaVec<'a, Range> {
    let mut merged: ArenaVec<'a, Range> =
        ArenaVec::with_capacity_in(left.len() + right.len(), arena);
    let (mut i, mut j) = (0, 0);
    while i < left.len() || j < right.len() {
        let take_left = j == right.len() || (i < left.len() && left[i].0 < right[j].0);
        let range = if take_left {
            i += 1;
            left[i - 1]
        } else {
            j += 1;
            right[j - 1]
        };
        match merged.last_mut() {
            | Some(last) if last.1 == range.0 => last.1 = range.1,
            | _ => merged.push(range),
        }
    }
    merged
}

fn index(value: usize) -> u32 {
    u32::try_from(value).expect("IR entities are numbered in u32")
}
