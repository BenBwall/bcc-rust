//! Copy coalescing: joins the two registers of a copy into one interval
//! when their live ranges do not overlap, so the copy becomes a move
//! between a register and itself, which the rewrite deletes.
//!
//! Copies come from the selector's two-address forms (`mov d, a; add d,
//! b`), from block arguments, and from aliases. Copies in deeper loops are
//! joined first, since they cost the most. A join is a union: the
//! representative's interval absorbs the other's ranges and uses, and
//! later candidates test against the union, so no two joined registers are
//! ever live at once (Chaitin's aggressive coalescing over live ranges,
//! without value-based interference).

use super::{
    intervals::Interval,
    liveness::contains,
};
use crate::{
    backend::native::mir::{
        MFunction,
        MInst,
        Operand,
        Reg,
        VReg,
        Width,
    },
    ir::Entity,
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// Joins copy-related registers of `func` whose intervals do not overlap;
/// returns each register's representative. A joined register's interval
/// is left empty. The registers in the set `pinned`, which are live across
/// a call that may return twice, join nothing.
pub(super) fn coalesce<'a>(
    func: &MFunction<'_>,
    intervals: &mut [Interval<'a>],
    depth: &[u32],
    pinned: &[u64],
    arena: &'a Bump,
) -> ArenaVec<'a, VReg> {
    let mut candidates = ArenaVec::new_in(arena);
    for (block, data) in func.blocks.iter().enumerate() {
        for inst in &data.insts {
            if let MInst::Mov {
                width: Width::B64,
                dst: Reg::Virt(dst),
                src: Operand::Reg(Reg::Virt(src)),
            }
            | MInst::XMov {
                width: Width::B64,
                dst: Reg::Virt(dst),
                src: Operand::Reg(Reg::Virt(src)),
            } = *inst
                && !contains(pinned, dst)
                && !contains(pinned, src)
            {
                candidates.push((depth[block], dst, src));
            }
        }
    }
    // Deepest first; the sort is stable, so layout order breaks ties.
    candidates.sort_by_key(|&(depth, ..)| std::cmp::Reverse(depth));
    let mut parent = ArenaVec::with_capacity_in(intervals.len(), arena);
    parent.extend((0..intervals.len()).map(VReg::new));
    // Whether each representative has absorbed another register.
    let mut merged = ArenaVec::with_capacity_in(intervals.len(), arena);
    merged.resize(intervals.len(), false);
    for &(_, dst, src) in &candidates {
        let (left, right) = (find(&mut parent, dst), find(&mut parent, src));
        if left == right
            || intervals[left.index()].class != intervals[right.index()].class
            || overlaps(&intervals[left.index()], &intervals[right.index()])
        {
            continue;
        }
        // The lower register represents the union.
        let (keep, join) = if left < right {
            (left, right)
        } else {
            (right, left)
        };
        let class = intervals[join.index()].class;
        let joined = std::mem::replace(
            &mut intervals[join.index()],
            Interval::new(join, class, arena),
        );
        intervals[keep.index()].absorb(&joined, merged[keep.index()], arena);
        merged[keep.index()] = true;
        parent[join.index()] = keep;
    }
    for index in 0..parent.len() {
        let rep = find(&mut parent, VReg::new(index));
        parent[index] = rep;
    }
    parent
}

/// The representative of `vreg`, compressing the path to it.
fn find(parent: &mut [VReg], vreg: VReg) -> VReg {
    let mut root = vreg;
    while parent[root.index()] != root {
        root = parent[root.index()];
    }
    let mut node = vreg;
    while parent[node.index()] != root {
        let next = parent[node.index()];
        parent[node.index()] = root;
        node = next;
    }
    root
}

/// Whether two intervals are live at a common position. The search starts
/// where the later one does, so a short interval tests against a long one
/// in time for the ranges they share, not for all of the long one's.
fn overlaps(left: &Interval<'_>, right: &Interval<'_>) -> bool {
    !left.is_empty()
        && !right.is_empty()
        && left
            .next_intersection(&right.ranges, left.start().max(right.start()))
            .is_some()
}
