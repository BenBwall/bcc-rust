//! Resolution: turns the scan's pieces into the [`Allocation`] the rewrite
//! reads, with the moves that carry a value from one piece to the next.
//!
//! A move is needed where a split falls inside a block while the value is
//! live on both sides of it (the piece is *carried*), and on each control
//! flow edge into a block where the value is live but its location at the
//! end of the predecessor differs from that at the start of the
//! successor. An edge's moves go at the start of a successor with no other
//! predecessor, else at the end of a predecessor with no other successor;
//! `edges.rs` split every other (critical) edge before allocation, except
//! those of indirect jumps, across which no interval is split.

use super::{
    Allocation,
    Edit,
    Location,
    intervals::{
        Interval,
        Loc,
        Range,
    },
    liveness::Liveness,
};
use crate::{
    backend::native::mir::{
        MBlock,
        MFunction,
        MInst,
        VReg,
    },
    ir::Entity,
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// The allocation of `func` from its allocated intervals.
pub(super) fn resolve<'a>(
    func: &MFunction<'_>,
    liveness: &Liveness<'_>,
    intervals: &[Interval<'_>],
    rep: ArenaVec<'a, VReg>,
    fallbacks: u32,
    arena: &'a Bump,
) -> Allocation<'a> {
    let count = func.vreg_count();
    let mut order = ArenaVec::with_capacity_in(intervals.len(), arena);
    order.extend(
        intervals
            .iter()
            .enumerate()
            .filter(|(_, interval)| !interval.is_empty())
            .map(|(id, interval)| (interval.vreg, interval.from, id)),
    );
    order.sort_unstable();
    let (slot, spill_slots) = spill_slots(func, intervals, arena);
    let mut pieces = ArenaVec::with_capacity_in(order.len(), arena);
    let mut piece_index = ArenaVec::with_capacity_in(count + 1, arena);
    let mut next = 0;
    for vreg in 0..count {
        piece_index.push(u32::try_from(pieces.len()).expect("pieces fit u32"));
        while next < order.len() && order[next].0.index() == vreg {
            let (_, from, id) = order[next];
            let location = match intervals[id].loc {
                | Loc::Reg(register) => Location::Reg(register),
                | Loc::Spill => Location::Spill(slot[vreg]),
                | Loc::None => unreachable!("the scan places every interval"),
            };
            pieces.push((from, location));
            next += 1;
        }
    }
    piece_index.push(u32::try_from(pieces.len()).expect("pieces fit u32"));
    let mut allocation = Allocation {
        rep,
        pieces,
        piece_index,
        edits: ArenaVec::new_in(arena),
        spill_slots,
        fallbacks,
    };
    let blocks = func.blocks.len();
    let block_of = |position: u32| {
        let mut low = 0;
        let mut high = blocks;
        while high - low > 1 {
            let middle = usize::midpoint(low, high);
            if liveness.block_start(middle) <= position {
                low = middle;
            } else {
                high = middle;
            }
        }
        low
    };
    for interval in intervals {
        if interval.is_empty() || !interval.carried {
            continue;
        }
        let block = block_of(interval.from);
        if liveness.block_start(block) == interval.from {
            continue;
        }
        let vreg = interval.vreg;
        if allocation.location(vreg, interval.from - 1) != allocation.location(vreg, interval.from)
        {
            allocation.edits.push(Edit {
                block: index(block),
                index: (interval.from - liveness.block_start(block)) / 2,
                phase: 0,
                vreg,
                src: interval.from - 1,
                dst: interval.from,
            });
        }
    }
    let mut predecessors = ArenaVec::with_capacity_in(blocks, arena);
    predecessors.resize(blocks, 0_u32);
    // The entry is also entered from the caller.
    predecessors[0] = 1;
    for block in 0..blocks {
        for successor in func.successors(MBlock::new(block)) {
            predecessors[successor.index()] += 1;
        }
    }
    let mut seen = ArenaVec::with_capacity_in(count, arena);
    seen.resize(count, u32::MAX);
    let mut edge = 0_u32;
    for block in 0..blocks {
        let successors = func.successors(MBlock::new(block)).count();
        let end = liveness.block_start(block + 1) - 1;
        let last = (liveness.block_start(block + 1) - liveness.block_start(block)) / 2 - 1;
        for successor in func.successors(MBlock::new(block)) {
            edge += 1;
            let start = liveness.block_start(successor.index());
            for &vreg in liveness.live_in(successor.index()) {
                let vreg = allocation.rep[vreg.index()];
                if seen[vreg.index()] == edge {
                    continue;
                }
                seen[vreg.index()] = edge;
                if allocation.location(vreg, end) == allocation.location(vreg, start) {
                    continue;
                }
                let (at, index, phase) = if predecessors[successor.index()] == 1 {
                    (successor.index(), 0, 0)
                } else {
                    assert!(
                        successors == 1,
                        "{vreg:?} changes location on a critical edge bb{block} -> {successor:?}"
                    );
                    (block, last, 1)
                };
                allocation.edits.push(Edit {
                    block: self::index(at),
                    index,
                    phase,
                    vreg,
                    src: end,
                    dst: start,
                });
            }
        }
    }
    allocation
        .edits
        .sort_by_key(|edit| (edit.block, edit.index, edit.phase));
    allocation
}

/// Each spilled register's spill slot, and the number of slots.
///
/// Registers that are never live at once share a slot, as LLVM's
/// `StackSlotColoring` shares them: a slot holds a register's value only
/// where the register is live, and each register, in the order their
/// lifetimes begin, takes the first slot none of whose registers is live
/// where it is, else a new one. A lifetime here is every range of every
/// piece of the register, whether in a register or spilled. The rewrite
/// reads a slot only where its register is live, and stores into it at a
/// definition, which starts a range, or at a move, where the register is
/// live on both sides (a move before instruction `k` carries a value live
/// at `2k - 1` and `2k`), so a register live elsewhere never sees the
/// store.
///
/// A function that calls one that returns twice (`setjmp`) keeps a slot per
/// register: a value live across the call is dead on the path to the
/// `longjmp` that resumes it, so another register could take its slot
/// there, as LLVM's pass also refuses for such functions.
fn spill_slots<'a>(
    func: &MFunction<'_>,
    intervals: &[Interval<'_>],
    arena: &'a Bump,
) -> (ArenaVec<'a, u32>, u32) {
    /// The most slots a register is tried against before it takes a new
    /// one, which bounds the work of a function with very many.
    const TRIES: usize = 256;
    const NONE: u32 = u32::MAX;
    let count = func.vreg_count();
    let mut spilled = ArenaVec::with_capacity_in(count, arena);
    spilled.resize(count, false);
    for interval in intervals {
        spilled[interval.vreg.index()] |= interval.loc == Loc::Spill;
    }
    // Every live range of each spilled register, by register and position.
    let mut ranges = ArenaVec::new_in(arena);
    for interval in intervals {
        if spilled[interval.vreg.index()] {
            ranges.extend(
                interval
                    .ranges
                    .iter()
                    .map(|&range| (interval.vreg.index(), range)),
            );
        }
    }
    ranges.sort_unstable();
    let mut slot = ArenaVec::with_capacity_in(count, arena);
    slot.resize(count, NONE);
    let mut slots = 0;
    // Each spilled register's ranges in `ranges`, by where they begin.
    let mut order = ArenaVec::new_in(arena);
    let mut at = 0;
    while at < ranges.len() {
        let vreg = ranges[at].0;
        let end = at + ranges[at..].partition_point(|&(other, _)| other == vreg);
        order.push((ranges[at].1.0, vreg, at..end));
        at = end;
    }
    if returns_twice(func) {
        for (_, vreg, _) in &order {
            slot[*vreg] = slots;
            slots += 1;
        }
        return (slot, slots);
    }
    order.sort_unstable_by_key(|&(start, vreg, _)| (start, vreg));
    // Each slot's occupied ranges, sorted and disjoint.
    let mut occupied: ArenaVec<'_, ArenaVec<'_, Range>> = ArenaVec::new_in(arena);
    for (_, vreg, own) in order {
        let own = &ranges[own];
        let fits = |taken: &[Range]| {
            own.iter().all(|&(_, (start, end))| {
                let index = taken.partition_point(|&(_, taken_end)| taken_end <= start);
                taken
                    .get(index)
                    .is_none_or(|&(taken_start, _)| end <= taken_start)
            })
        };
        let chosen = occupied.iter().take(TRIES).position(|taken| fits(taken));
        let chosen = chosen.unwrap_or_else(|| {
            occupied.push(ArenaVec::new_in(arena));
            occupied.len() - 1
        });
        let taken = &mut occupied[chosen];
        for &(_, range) in own {
            let index = taken.partition_point(|&(start, _)| start < range.0);
            taken.insert(index, range);
        }
        slot[vreg] = u32::try_from(chosen).expect("spill slots fit u32");
        slots = slots.max(slot[vreg] + 1);
    }
    (slot, slots)
}

/// Whether `func` calls a function that may return twice.
fn returns_twice(func: &MFunction<'_>) -> bool {
    func.blocks
        .iter()
        .flat_map(|block| &block.insts)
        .any(|inst| {
            matches!(
                inst,
                MInst::Call {
                    returns_twice: true,
                    ..
                } | MInst::SjljSetjmp { .. }
            )
        })
}

fn index(block: usize) -> u32 {
    u32::try_from(block).expect("block numbers fit u32")
}
