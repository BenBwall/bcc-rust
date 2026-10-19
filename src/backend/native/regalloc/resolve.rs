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
    },
    liveness::{
        Liveness,
        members,
    },
};
use crate::{
    backend::native::mir::{
        MBlock,
        MFunction,
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
    let mut slot = ArenaVec::with_capacity_in(count, arena);
    slot.resize(count, None);
    let mut spill_slots = 0;
    let mut pieces = ArenaVec::with_capacity_in(order.len(), arena);
    let mut piece_index = ArenaVec::with_capacity_in(count + 1, arena);
    let mut next = 0;
    for vreg in 0..count {
        piece_index.push(u32::try_from(pieces.len()).expect("pieces fit u32"));
        while next < order.len() && order[next].0.index() == vreg {
            let (_, from, id) = order[next];
            let location = match intervals[id].loc {
                | Loc::Reg(register) => Location::Reg(register),
                | Loc::Spill => Location::Spill(*slot[vreg].get_or_insert_with(|| {
                    spill_slots += 1;
                    spill_slots - 1
                })),
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
            members(liveness.live_in(successor.index()), |vreg| {
                let vreg = allocation.rep[vreg.index()];
                if seen[vreg.index()] == edge {
                    return;
                }
                seen[vreg.index()] = edge;
                if allocation.location(vreg, end) == allocation.location(vreg, start) {
                    return;
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
            });
        }
    }
    allocation
        .edits
        .sort_by_key(|edit| (edit.block, edit.index, edit.phase));
    allocation
}

fn index(block: usize) -> u32 {
    u32::try_from(block).expect("block numbers fit u32")
}
