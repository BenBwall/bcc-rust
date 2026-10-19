//! Liveness, dead-instruction removal, live intervals and fixed ranges.
//!
//! Live-in and live-out sets are bit sets per block, computed by backward
//! iterative dataflow (`in = use ∪ (out − def)`, `out = ∪ in(successor)`)
//! until nothing changes. A backward walk over each block then removes pure
//! instructions whose results are dead, which can make more instructions
//! dead, so the two repeat until no instruction is removed.

use super::intervals::{
    FixedRanges,
    Interval,
    Range,
    push_range,
};
use crate::{
    backend::native::mir::{
        MBlock,
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
    /// The live-in set of each block.
    live_in:  ArenaVec<'a, u64>,
    /// The number of the first instruction of each block, then the total.
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
                for successor in func.successors(MBlock::new(block)) {
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
        first.push(count);
        Self {
            words,
            live_out,
            live_in,
            first,
        }
    }

    /// Removes pure instructions whose results are dead, and arithmetic
    /// whose results and flags are; whether any was. (A load folds a
    /// constant `ptr_add` into its address, leaving the `mov` and `add`
    /// that computed it dead.)
    fn remove_dead(&self, func: &mut MFunction<'_>, arena: &Bump) -> bool {
        let mut removed = false;
        let mut live = ArenaVec::with_capacity_in(self.words, arena);
        for (block, data) in func.blocks.iter_mut().enumerate() {
            live.clear();
            live.extend_from_slice(self.out(block));
            let mut keep = ArenaVec::with_capacity_in(data.insts.len(), arena);
            keep.resize(data.insts.len(), true);
            // Whether an instruction after this point reads the flags
            // before one overwrites them; never across blocks.
            let mut flags_read = false;
            for (index, inst) in data.insts.iter().enumerate().rev() {
                let mut inst = *inst;
                if inst.is_pure() || (!flags_read && inst.is_pure_but_flags()) {
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
                let (reads, writes) = inst.flags();
                flags_read = reads || (flags_read && !writes);
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

    /// Each virtual register's live interval, indexed by register, with
    /// its lifetime holes, and the fixed ranges of the physical registers.
    /// Built backward over the blocks (Wimmer and Franz, "Linear Scan
    /// Register Allocation on SSA Form", CGO 2010, `BuildIntervals`): a
    /// register live out of a block is live through it until a definition
    /// shortens it, and each use extends it back to the block's start.
    pub(super) fn intervals(
        &self,
        func: &MFunction<'_>,
        arena: &'a Bump,
    ) -> (ArenaVec<'a, Interval<'a>>, FixedRanges<'a>) {
        let count = func.vreg_count();
        let mut intervals = ArenaVec::with_capacity_in(count, arena);
        for (index, &class) in func.classes.iter().enumerate() {
            intervals.push(Interval::new(VReg::new(index), class, arena));
        }
        // Ranges and uses are collected latest first, then reversed.
        let add =
            |interval: &mut Interval<'_>, start: u32, end: u32| match interval.ranges.last_mut() {
                | Some(last) if last.0 <= end => last.0 = last.0.min(start),
                | _ => interval.ranges.push((start, end)),
            };
        for (block, data) in func.blocks.iter().enumerate().rev() {
            let first = self.first[block];
            let start = 2 * first;
            let end = 2 * (first + u32::try_from(data.insts.len()).expect("fits"));
            members(self.out(block), |vreg| {
                add(&mut intervals[vreg.index()], start, end);
            });
            for (index, inst) in data.insts.iter().enumerate().rev() {
                let position = 2 * (first + u32::try_from(index).expect("fits"));
                let mut inst = *inst;
                inst.visit_registers(|reg, role| {
                    if let Reg::Virt(vreg) = *reg
                        && role.writes()
                    {
                        let interval = &mut intervals[vreg.index()];
                        let def = position + 1;
                        match interval.ranges.last_mut() {
                            // Live after the definition: it starts here.
                            | Some(last) if last.0 <= def && def < last.1 => last.0 = def,
                            // A definition nobody reads.
                            | _ => interval.ranges.push((def, def + 1)),
                        }
                        interval.uses.push(def);
                    }
                });
                // A value the instruction can read from its spill slot does
                // not need a register there.
                let in_memory = inst.memory_source_vreg();
                inst.visit_registers(|reg, role| {
                    if let Reg::Virt(vreg) = *reg
                        && role.reads()
                    {
                        let interval = &mut intervals[vreg.index()];
                        add(interval, start, position + 1);
                        if in_memory == Some(vreg) {
                            interval.weak.push(position);
                        } else {
                            interval.uses.push(position);
                        }
                    }
                });
            }
        }
        for interval in &mut intervals {
            interval.ranges.reverse();
            interval.uses.reverse();
            interval.uses.dedup();
            interval.weak.reverse();
        }
        (intervals, self.fixed_ranges(func, arena))
    }

    /// The ranges of each physical register: from a precolored definition
    /// to its last use in the block, or the definition's position alone
    /// when nothing reads it (a clobber). A register read in a block before
    /// any definition there (an incoming argument) is live from the
    /// block's start.
    fn fixed_ranges(&self, func: &MFunction<'_>, arena: &'a Bump) -> FixedRanges<'a> {
        let mut ranges: [ArenaVec<'a, Range>; PReg::COUNT] =
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
                        let end = live_end[register.index()].take().unwrap_or(position + 2);
                        ranges[register.index()].push((position + 1, end));
                    }
                });
                inst.visit_registers(|reg, role| {
                    if let Reg::Phys(register) = *reg
                        && role.reads()
                    {
                        _ = live_end[register.index()].get_or_insert(position + 1);
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
            let mut merged = ArenaVec::with_capacity_in(ranges.len(), arena);
            for &range in ranges.iter() {
                push_range(&mut merged, range);
            }
            *ranges = merged;
        }
        FixedRanges { ranges }
    }

    /// The live-in set of `block`.
    pub(super) fn live_in(&self, block: usize) -> &[u64] {
        &self.live_in[block * self.words..(block + 1) * self.words]
    }

    /// The position of the first instruction of `block`, and of the end of
    /// the last block for `block` equal to the number of blocks.
    pub(super) fn block_start(&self, block: usize) -> u32 {
        2 * self.first[block]
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
pub(super) fn members(set: &[u64], mut visit: impl FnMut(VReg)) {
    for (word, &bits) in set.iter().enumerate() {
        let mut bits = bits;
        while bits != 0 {
            let bit = bits.trailing_zeros() as usize;
            visit(VReg::new(word * 64 + bit));
            bits &= bits - 1;
        }
    }
}
