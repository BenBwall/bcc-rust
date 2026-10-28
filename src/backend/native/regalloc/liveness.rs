//! Liveness, dead-instruction removal, live intervals and fixed ranges.
//!
//! The live-in set of each block is a sorted list of the virtual registers
//! live into it, so the sets take space in proportion to what is live
//! rather than to blocks × registers. They are solved one register at a
//! time by path exploration (Brandner et al., "Computing Liveness Sets for
//! SSA-Form Programs", INRIA RR-7503, 2011), which needs no SSA form when it
//! stops at every definition: from each block that reads the register
//! before writing it, a backward walk over the predecessors marks it live
//! into every block it reaches, and stops at the blocks that write it. That
//! is the least solution of `in = use ∪ (out − def)`, `out = ∪
//! in(successor)`, the one iterative dataflow reaches, in time linear in the
//! instructions plus the sets' sizes times the predecessors, with no
//! iteration to a fixed point. Live-out sets are not stored: a walk over a
//! block gathers its successors' live-in sets in a [`LiveSet`].
//!
//! A backward walk over each block then removes pure instructions whose
//! results are dead, which can make more instructions dead, so the two
//! repeat until no instruction is removed.

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

/// The live-in sets of a function's blocks and its instruction numbering.
pub(super) struct Liveness<'a> {
    /// The live-in sets of the blocks one after another, each sorted.
    live_in: ArenaVec<'a, VReg>,
    /// Where the live-in set of each block starts in `live_in`, then the
    /// end of the last.
    starts:  ArenaVec<'a, u32>,
    /// The number of the first instruction of each block, then the total.
    first:   ArenaVec<'a, u32>,
    /// The number of virtual registers, the size of a [`LiveSet`].
    vregs:   usize,
    /// The work the solution took: the registers each block reads or
    /// writes, and each predecessor a walk looked at.
    steps:   usize,
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
        Self::of_program(
            func,
            func.blocks.iter().map(|block| &block.insts[..]),
            arena,
        )
    }

    /// The dataflow solution for `program`, one instruction list per block
    /// of `func`, whose terminators give the control flow. The checker
    /// solves it for the virtual-register program it compares the
    /// rewritten `func` against.
    pub(super) fn of_program<'i>(
        func: &MFunction<'_>,
        program: impl Iterator<Item = &'i [MInst]>,
        arena: &'a Bump,
    ) -> Self {
        let blocks = func.blocks.len();
        let vregs = func.vreg_count();
        // The blocks that read each register before writing it, and those
        // that write it, each block once per register.
        let mut reads = ArenaVec::new_in(arena);
        let mut writes = ArenaVec::new_in(arena);
        // The block (plus one) that last read or wrote each register.
        let mut read_in = zeroed(vregs, arena);
        let mut written_in = zeroed(vregs, arena);
        let mut first = ArenaVec::with_capacity_in(blocks + 1, arena);
        let mut count = 0_u32;
        for (block, insts) in program.enumerate() {
            first.push(count);
            count += u32::try_from(insts.len()).expect("instruction numbers fit u32");
            let mark = index(block + 1);
            for inst in insts {
                let mut inst = *inst;
                inst.visit_registers(|reg, role| {
                    if let Reg::Virt(vreg) = *reg
                        && role.reads()
                        && written_in[vreg.index()] != mark
                        && read_in[vreg.index()] != mark
                    {
                        read_in[vreg.index()] = mark;
                        reads.push((vreg, index(block)));
                    }
                });
                inst.visit_registers(|reg, role| {
                    if let Reg::Virt(vreg) = *reg
                        && role.writes()
                        && written_in[vreg.index()] != mark
                    {
                        written_in[vreg.index()] = mark;
                        writes.push((vreg, index(block)));
                    }
                });
            }
        }
        first.push(count);
        let mut steps = reads.len() + writes.len();
        let reads = Grouped::new(&reads, vregs, arena);
        let writes = Grouped::new(&writes, vregs, arena);
        let mut edges = ArenaVec::new_in(arena);
        for block in 0..blocks {
            for successor in func.successors(MBlock::new(block)) {
                edges.push((successor, index(block)));
            }
        }
        let preds = Grouped::new(&edges, blocks, arena);
        // Walk back from each register's reads, one register at a time.
        // The marks name the register (plus one) a block was last marked
        // for: `kills` that the block writes it, `live` that it is live
        // into the block.
        let mut kills = zeroed(blocks, arena);
        let mut live = zeroed(blocks, arena);
        let mut stack = ArenaVec::new_in(arena);
        // The blocks each register is live into, register after register,
        // and where each register's blocks end.
        let mut live_blocks = ArenaVec::new_in(arena);
        let mut ends = ArenaVec::with_capacity_in(vregs, arena);
        for vreg in 0..vregs {
            let mark = index(vreg + 1);
            for &block in writes.of(vreg) {
                kills[block as usize] = mark;
            }
            for &block in reads.of(vreg) {
                live[block as usize] = mark;
                live_blocks.push(block);
                stack.push(block);
            }
            while let Some(block) = stack.pop() {
                let preds = preds.of(block as usize);
                steps += preds.len();
                for &pred in preds {
                    if live[pred as usize] != mark && kills[pred as usize] != mark {
                        live[pred as usize] = mark;
                        live_blocks.push(pred);
                        stack.push(pred);
                    }
                }
            }
            ends.push(index(live_blocks.len()));
        }
        // Turn the sets by register into sets by block; taking the
        // registers in order sorts each block's.
        let mut starts = zeroed(blocks + 1, arena);
        for &block in &live_blocks {
            starts[block as usize + 1] += 1;
        }
        for block in 0..blocks {
            starts[block + 1] += starts[block];
        }
        let mut next = ArenaVec::with_capacity_in(blocks, arena);
        next.extend_from_slice(&starts[..blocks]);
        let mut live_in = ArenaVec::with_capacity_in(live_blocks.len(), arena);
        live_in.resize(live_blocks.len(), VReg::new(0));
        let mut vreg = 0;
        for (position, &block) in live_blocks.iter().enumerate() {
            while ends[vreg] as usize <= position {
                vreg += 1;
            }
            let slot = &mut next[block as usize];
            live_in[*slot as usize] = VReg::new(vreg);
            *slot += 1;
        }
        Self {
            live_in,
            starts,
            first,
            vregs,
            steps,
        }
    }

    /// Removes pure instructions whose results are dead, and arithmetic
    /// whose results and flags are; whether any was. (A load folds a
    /// constant `ptr_add` into its address, leaving the `mov` and `add`
    /// that computed it dead.)
    fn remove_dead(&self, func: &mut MFunction<'_>, arena: &Bump) -> bool {
        let mut removed = false;
        let mut live = LiveSet::new(self.vregs, arena);
        for block in 0..func.blocks.len() {
            self.live_out(func, block, &mut live);
            let data = &mut func.blocks[block];
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
                            && live.contains(vreg)
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

    /// Makes `live` the live-out set of `block`: the union of its
    /// successors' live-in sets.
    pub(super) fn live_out(&self, func: &MFunction<'_>, block: usize, live: &mut LiveSet<'_>) {
        live.clear();
        for successor in func.successors(MBlock::new(block)) {
            for &vreg in self.live_in(successor.index()) {
                live.insert(vreg);
            }
        }
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
        let mut live_out = LiveSet::new(self.vregs, arena);
        for (block, data) in func.blocks.iter().enumerate().rev() {
            let first = self.first[block];
            let start = 2 * first;
            let end = 2 * (first + u32::try_from(data.insts.len()).expect("fits"));
            self.live_out(func, block, &mut live_out);
            for &vreg in live_out.members() {
                add(&mut intervals[vreg.index()], start, end);
            }
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

    /// The virtual registers live across a call that may return twice or a
    /// [`MInst::SjljSetjmp`], as a bit set. A second return resumes after
    /// the call with the callee-saved registers as they were at the first
    /// (`longjmp` restores them; after `__builtin_longjmp` nothing lives in
    /// a register), but with memory as the program left it, so such a value
    /// must keep a spill slot of its own: coalescing would let another
    /// register's later definitions overwrite it. C99: §7.13.2.1 paragraph
    /// 3, p. 244; PDF p. 256.
    pub(super) fn live_across_returns_twice(
        &self,
        func: &MFunction<'_>,
        arena: &'a Bump,
    ) -> ArenaVec<'a, u64> {
        let mut across = ArenaVec::with_capacity_in(self.vregs.div_ceil(64), arena);
        across.resize(self.vregs.div_ceil(64), 0_u64);
        let mut live = LiveSet::new(self.vregs, arena);
        for (block, data) in func.blocks.iter().enumerate() {
            let calls = |inst: &MInst| {
                matches!(
                    inst,
                    MInst::Call {
                        returns_twice: true,
                        ..
                    } | MInst::SjljSetjmp { .. }
                )
            };
            if !data.insts.iter().any(calls) {
                continue;
            }
            self.live_out(func, block, &mut live);
            for inst in data.insts.iter().rev() {
                if calls(inst) {
                    for &vreg in live.members() {
                        across[vreg.index() / 64] |= 1 << (vreg.index() % 64);
                    }
                }
                transfer(&mut { *inst }, &mut live);
            }
        }
        across
    }

    /// The live-in set of `block`, sorted.
    pub(super) fn live_in(&self, block: usize) -> &[VReg] {
        &self.live_in[self.starts[block] as usize..self.starts[block + 1] as usize]
    }

    /// The position of the first instruction of `block`, and of the end of
    /// the last block for `block` equal to the number of blocks.
    pub(super) fn block_start(&self, block: usize) -> u32 {
        2 * self.first[block]
    }

    /// The number of registers the live-in sets hold together, which is
    /// the space they take.
    #[cfg(test)]
    pub(super) fn stored(&self) -> usize {
        self.live_in.len()
    }

    /// The work the solution took: each register a block reads or writes,
    /// and each predecessor edge a walk followed.
    #[cfg(test)]
    pub(super) const fn steps(&self) -> usize {
        self.steps
    }
}

/// Updates `live` backward across one instruction: its definitions die,
/// its uses become live.
pub(super) fn transfer(inst: &mut MInst, live: &mut LiveSet<'_>) {
    inst.visit_registers(|reg, role| {
        if let Reg::Virt(vreg) = *reg
            && role == Role::Def
        {
            live.remove(vreg);
        }
    });
    inst.visit_registers(|reg, role| {
        if let Reg::Virt(vreg) = *reg
            && role.reads()
        {
            live.insert(vreg);
        }
    });
}

/// Whether the bit set `set` holds `vreg`.
pub(super) fn contains(set: &[u64], vreg: VReg) -> bool {
    set[vreg.index() / 64] & (1 << (vreg.index() % 64)) != 0
}

/// A set of a function's virtual registers for one walk over a block,
/// taking constant time to insert, remove, test and clear (Briggs and
/// Torczon, "An Efficient Representation for Sparse Sets", LOPLAS 1993), so
/// starting each block costs its live-out set, not the function's
/// registers.
pub(super) struct LiveSet<'a> {
    /// The members, in no order.
    dense:  ArenaVec<'a, VReg>,
    /// Each register's position in `dense`, meaningful for members only.
    sparse: ArenaVec<'a, u32>,
}

impl<'a> LiveSet<'a> {
    /// An empty set of the registers below `vregs`.
    pub(super) fn new(vregs: usize, arena: &'a Bump) -> Self {
        Self {
            dense:  ArenaVec::new_in(arena),
            sparse: zeroed(vregs, arena),
        }
    }

    pub(super) fn contains(&self, vreg: VReg) -> bool {
        let position = self.sparse[vreg.index()] as usize;
        self.dense.get(position) == Some(&vreg)
    }

    pub(super) fn insert(&mut self, vreg: VReg) {
        if !self.contains(vreg) {
            self.sparse[vreg.index()] = index(self.dense.len());
            self.dense.push(vreg);
        }
    }

    pub(super) fn remove(&mut self, vreg: VReg) {
        if self.contains(vreg) {
            let position = self.sparse[vreg.index()] as usize;
            let last = self.dense.pop().expect("a member");
            if last != vreg {
                self.dense[position] = last;
                self.sparse[last.index()] = index(position);
            }
        }
    }

    pub(super) fn clear(&mut self) {
        self.dense.clear();
    }

    /// The members, in no particular order.
    pub(super) fn members(&self) -> &[VReg] {
        &self.dense
    }
}

/// Pairs `(key, value)` grouped by key, each key's values in the order the
/// pairs gave them (a counting sort).
struct Grouped<'a> {
    /// Where each key's values start, then the end.
    starts: ArenaVec<'a, u32>,
    values: ArenaVec<'a, u32>,
}

impl<'a> Grouped<'a> {
    fn new<K: Entity>(pairs: &[(K, u32)], keys: usize, arena: &'a Bump) -> Self {
        let mut starts = zeroed(keys + 1, arena);
        for &(key, _) in pairs {
            starts[key.index() + 1] += 1;
        }
        for key in 0..keys {
            starts[key + 1] += starts[key];
        }
        let mut next = ArenaVec::with_capacity_in(keys, arena);
        next.extend_from_slice(&starts[..keys]);
        let mut values = zeroed(pairs.len(), arena);
        for &(key, value) in pairs {
            let slot = &mut next[key.index()];
            values[*slot as usize] = value;
            *slot += 1;
        }
        Self { starts, values }
    }

    /// The values of `key`.
    fn of(&self, key: usize) -> &[u32] {
        &self.values[self.starts[key] as usize..self.starts[key + 1] as usize]
    }
}

/// `len` zeros.
fn zeroed(len: usize, arena: &Bump) -> ArenaVec<'_, u32> {
    let mut vec = ArenaVec::with_capacity_in(len, arena);
    vec.resize(len, 0);
    vec
}

/// A block, register or position number as stored.
fn index(number: usize) -> u32 {
    u32::try_from(number).expect("liveness numbers fit u32")
}
