//! Liveness, dead-instruction removal, live intervals and fixed ranges.
//!
//! Live-in and live-out sets are bit sets per block, computed by backward
//! iterative dataflow (`in = use ∪ (out − def)`, `out = ∪ in(successor)`)
//! until nothing changes. A backward walk over each block then removes pure
//! instructions whose results are dead, which can make more instructions
//! dead, so the two repeat until no instruction is removed.

use super::{
    FixedRanges,
    Interval,
};
use crate::{
    backend::native::mir::{
        MFunction,
        MInst,
        PReg,
        Reg,
        Role,
        VReg,
    },
    ir::Entity,
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// The live-out sets of a function's blocks and its instruction numbering.
pub(super) struct Liveness<'a> {
    /// `u64` words per set.
    words:    usize,
    /// The live-out set of each block, `words` words each.
    live_out: ArenaVec<'a, u64>,
    /// The number of the first instruction of each block.
    first:    ArenaVec<'a, u32>,
}

impl<'a> Liveness<'a> {
    /// Computes liveness, removing dead pure instructions until none is
    /// left.
    pub(super) fn compute(func: &mut MFunction<'_>, arena: &'a Bump) -> Self {
        loop {
            let liveness = Self::dataflow(func, arena);
            if !liveness.remove_dead(func, arena) {
                return liveness;
            }
        }
    }

    /// One dataflow solution for the current instructions.
    fn dataflow(func: &MFunction<'_>, arena: &'a Bump) -> Self {
        let blocks = func.blocks.len();
        let words = func.vreg_count().div_ceil(64).max(1);
        let zeroed = |len: usize| {
            let mut vec = ArenaVec::with_capacity_in(len, arena);
            vec.resize(len, 0_u64);
            vec
        };
        let mut uses = zeroed(blocks * words);
        let mut defs = zeroed(blocks * words);
        let mut live_in = zeroed(blocks * words);
        let mut live_out = zeroed(blocks * words);
        let mut first = ArenaVec::with_capacity_in(blocks, arena);
        let mut count = 0_u32;
        for (block, data) in func.blocks.iter().enumerate() {
            first.push(count);
            count += u32::try_from(data.insts.len()).expect("instruction numbers fit u32");
            let (uses, defs) = (
                &mut uses[block * words..(block + 1) * words],
                &mut defs[block * words..(block + 1) * words],
            );
            for inst in &data.insts {
                let mut inst = *inst;
                inst.visit_registers(|reg, role| {
                    if let Reg::Virt(vreg) = *reg
                        && role.reads()
                        && !contains(defs, vreg)
                    {
                        insert(uses, vreg);
                    }
                });
                inst.visit_registers(|reg, role| {
                    if let Reg::Virt(vreg) = *reg
                        && role.writes()
                    {
                        insert(defs, vreg);
                    }
                });
            }
        }
        let mut changed = true;
        while changed {
            changed = false;
            for block in (0..blocks).rev() {
                let range = block * words..(block + 1) * words;
                let successors = func.blocks[block]
                    .insts
                    .last()
                    .map(MInst::successors)
                    .into_iter()
                    .flatten();
                for successor in successors {
                    let from = successor.index() * words;
                    for word in 0..words {
                        live_out[range.start + word] |= live_in[from + word];
                    }
                }
                for word in range {
                    let new = uses[word] | (live_out[word] & !defs[word]);
                    if new != live_in[word] {
                        live_in[word] = new;
                        changed = true;
                    }
                }
            }
        }
        Self {
            words,
            live_out,
            first,
        }
    }

    /// Removes pure instructions whose results are dead; whether any was.
    fn remove_dead(&self, func: &mut MFunction<'_>, arena: &Bump) -> bool {
        let mut removed = false;
        let mut live = ArenaVec::with_capacity_in(self.words, arena);
        for (block, data) in func.blocks.iter_mut().enumerate() {
            live.clear();
            live.extend_from_slice(self.out(block));
            let mut keep = ArenaVec::with_capacity_in(data.insts.len(), arena);
            keep.resize(data.insts.len(), true);
            for (index, inst) in data.insts.iter().enumerate().rev() {
                let mut inst = *inst;
                if inst.is_pure() {
                    let mut dead = true;
                    inst.visit_registers(|reg, role| {
                        if let Reg::Virt(vreg) = *reg
                            && role.writes()
                            && contains(&live, vreg)
                        {
                            dead = false;
                        }
                    });
                    if dead {
                        keep[index] = false;
                        removed = true;
                        continue;
                    }
                }
                transfer(&mut inst, &mut live);
            }
            let mut index = 0;
            data.insts.retain(|_| {
                index += 1;
                keep[index - 1]
            });
        }
        removed
    }

    /// The live-out set of `block`.
    fn out(&self, block: usize) -> &[u64] {
        &self.live_out[block * self.words..(block + 1) * self.words]
    }

    /// Each virtual register's interval: every position where it is
    /// defined, used, or live across a block boundary.
    pub(super) fn intervals(
        &self,
        func: &MFunction<'_>,
        arena: &'a Bump,
    ) -> ArenaVec<'a, Interval> {
        let count = func.vreg_count();
        let mut bounds = ArenaVec::with_capacity_in(count, arena);
        bounds.resize(count, (u32::MAX, 0_u32));
        let mut extend = |vreg: VReg, position: u32| {
            let (start, end) = &mut bounds[vreg.index()];
            *start = (*start).min(position);
            *end = (*end).max(position);
        };
        let mut live = ArenaVec::with_capacity_in(self.words, arena);
        for (block, data) in func.blocks.iter().enumerate() {
            let first = self.first[block];
            let start = 2 * first;
            let end = 2 * (first + u32::try_from(data.insts.len()).expect("fits")) - 1;
            live.clear();
            live.extend_from_slice(self.out(block));
            members(&live, |vreg| extend(vreg, end));
            for (index, inst) in data.insts.iter().enumerate().rev() {
                let position = 2 * (first + u32::try_from(index).expect("fits"));
                let mut inst = *inst;
                inst.visit_registers(|reg, role| {
                    if let Reg::Virt(vreg) = *reg {
                        if role.writes() {
                            extend(vreg, position + 1);
                        }
                        if role.reads() {
                            extend(vreg, position);
                        }
                    }
                });
                transfer(&mut inst, &mut live);
            }
            members(&live, |vreg| extend(vreg, start));
        }
        let mut intervals = ArenaVec::new_in(arena);
        for (index, &(start, end)) in bounds.iter().enumerate() {
            if start <= end {
                intervals.push(Interval {
                    vreg: VReg::new(index),
                    start,
                    end,
                });
            }
        }
        intervals
    }

    /// The ranges of each physical register: from a precolored definition
    /// to its last use in the block, or a single position for a definition
    /// nobody reads (a clobber). A register read at a block's start without
    /// a definition (an incoming argument) is live from the block's start.
    pub(super) fn fixed_ranges(&self, func: &MFunction<'_>, arena: &'a Bump) -> FixedRanges<'a> {
        let mut ranges: [ArenaVec<'a, (u32, u32)>; PReg::COUNT] =
            std::array::from_fn(|_| ArenaVec::new_in(arena));
        for (block, data) in func.blocks.iter().enumerate() {
            let first = self.first[block];
            let mut live_end: [Option<u32>; PReg::COUNT] = [None; PReg::COUNT];
            for (index, inst) in data.insts.iter().enumerate().rev() {
                let position = 2 * (first + u32::try_from(index).expect("fits"));
                let mut inst = *inst;
                inst.visit_registers(|reg, role| {
                    if let Reg::Phys(register) = *reg
                        && role.writes()
                    {
                        let end = live_end[register.index()].take().unwrap_or(position + 1);
                        ranges[register.index()].push((position + 1, end));
                    }
                });
                inst.visit_registers(|reg, role| {
                    if let Reg::Phys(register) = *reg
                        && role.reads()
                    {
                        _ = live_end[register.index()].get_or_insert(position);
                    }
                });
            }
            for register in PReg::ALL {
                if let Some(end) = live_end[register.index()] {
                    ranges[register.index()].push((2 * first, end));
                }
            }
        }
        for ranges in &mut ranges {
            ranges.sort_unstable();
            // Merge overlapping ranges so the lookup's binary search holds.
            let mut merged = 0;
            for index in 0..ranges.len() {
                let range = ranges[index];
                if merged > 0 && range.0 <= ranges[merged - 1].1 + 1 {
                    ranges[merged - 1].1 = ranges[merged - 1].1.max(range.1);
                } else {
                    ranges[merged] = range;
                    merged += 1;
                }
            }
            ranges.truncate(merged);
        }
        FixedRanges { ranges }
    }
}

/// Updates `live` backward across one instruction: its definitions die,
/// its uses become live.
fn transfer(inst: &mut MInst, live: &mut [u64]) {
    inst.visit_registers(|reg, role| {
        if let Reg::Virt(vreg) = *reg
            && role == Role::Def
        {
            remove(live, vreg);
        }
    });
    inst.visit_registers(|reg, role| {
        if let Reg::Virt(vreg) = *reg
            && role.reads()
        {
            insert(live, vreg);
        }
    });
}

fn contains(set: &[u64], vreg: VReg) -> bool {
    set[vreg.index() / 64] & (1 << (vreg.index() % 64)) != 0
}

fn insert(set: &mut [u64], vreg: VReg) {
    set[vreg.index() / 64] |= 1 << (vreg.index() % 64);
}

fn remove(set: &mut [u64], vreg: VReg) {
    set[vreg.index() / 64] &= !(1 << (vreg.index() % 64));
}

/// Calls `visit` with each member of a set.
fn members(set: &[u64], mut visit: impl FnMut(VReg)) {
    for (word, &bits) in set.iter().enumerate() {
        let mut bits = bits;
        while bits != 0 {
            let bit = bits.trailing_zeros() as usize;
            visit(VReg::new(word * 64 + bit));
            bits &= bits - 1;
        }
    }
}
