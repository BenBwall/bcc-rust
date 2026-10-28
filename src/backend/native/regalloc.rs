//! Register allocation: linear scan over split live intervals with
//! lifetime holes and fixed-register ranges (Wimmer and Mössenböck,
//! "Optimized Interval Splitting in a Linear Scan Register Allocator", VEE
//! 2005), after copy coalescing, proven by a symbolic checker.
//!
//! Instruction `i` (numbered in block layout order) reads at position `2i`
//! and writes at `2i+1`, so a value that dies at an instruction may share a
//! register with one it defines. Liveness is solved backward over the
//! blocks, one register at a time; each virtual register's interval is the
//! set of positions where its value is needed, with holes where it is not.
//! Each physical register gets the ranges where a precolored value lives in
//! it (an argument between its `mov` and the `call`, `%rax` between a
//! `call` and the copy of its result) and a point at each clobber. An
//! interval may hold a register only where it meets none of that
//! register's fixed ranges, so a value live across a call, which meets
//! every caller-saved clobber, is split around the call or lands in a
//! callee-saved register, without a special case.
//!
//! In order:
//!
//! 1. `edges.rs` splits critical edges, so moves can go on any edge.
//! 2. `liveness.rs` removes dead pure instructions and builds the intervals and
//!    fixed ranges.
//! 3. `coalesce.rs` joins the two sides of copies whose intervals do not
//!    overlap.
//! 4. `scan.rs` assigns registers, splitting intervals where a register is free
//!    only for a while and spilling the parts that are not needed soon.
//! 5. `resolve.rs` lists each register's pieces and the moves between them,
//!    within blocks and on edges.
//! 6. `rewrite.rs` replaces virtual registers by locations and inserts the
//!    moves; in debug builds `checker.rs` then proves the result against the
//!    virtual-register program. Identity copies go, `peephole.rs` combines
//!    copies and additions into `lea`s, and `edges.rs` removes the blocks that
//!    only jump and copies short tests over the jumps to them (loop rotation).
//!
//! Read [`allocate`], then the files in that order.

// Control flow
mod edges;

// Analysis
mod coalesce;
mod intervals;
mod liveness;

// Assignment
mod resolve;
mod scan;

// Rewriting, its proof and cleanups
mod checker;
mod peephole;
mod rewrite;

pub(crate) use rewrite::rewrite;

#[cfg(test)]
pub(crate) use self::{
    checker::{
        CheckError,
        ErrorKind,
        INSERTED,
        Loc,
        Value,
        check,
    },
    rewrite::rewrite_checked,
};
use self::{
    liveness::Liveness,
    scan::{
        Layout,
        Scan,
    },
};
use super::{
    conv::CallConv,
    mir::{
        MBlock,
        MFunction,
        MInst,
        PReg,
        Reg,
        VReg,
    },
};
use crate::{
    ir::Entity,
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// Allocates every virtual register of `func`, after splitting its critical
/// edges and deleting the pure instructions whose results are dead.
pub(crate) fn allocate<'a>(
    func: &mut MFunction<'a>,
    conv: CallConv,
    arena: &'a Bump,
) -> Allocation<'a> {
    edges::split_critical_edges(func, arena);
    let liveness = Liveness::compute(func, arena);
    let (mut intervals, fixed) = liveness.intervals(func, arena);
    let depth = loop_depths(func, arena);
    let mut copy_hints = ArenaVec::with_capacity_in(func.vreg_count(), arena);
    copy_hints.resize(func.vreg_count(), None);
    for (index, hint) in func.hints.iter().enumerate() {
        match *hint {
            | Some(Reg::Phys(register)) => intervals[index].hint = Some(register),
            | Some(Reg::Virt(other)) => copy_hints[index] = Some(other),
            | None => {},
        }
    }
    let pinned = liveness.live_across_returns_twice(func, arena);
    let rep = coalesce::coalesce(func, &mut intervals, &depth, &pinned, arena);
    // A representative takes the first copy hint of the registers it
    // joined that names another representative.
    let mut rep_hints = ArenaVec::with_capacity_in(copy_hints.len(), arena);
    rep_hints.resize(copy_hints.len(), None);
    for (index, hint) in copy_hints.iter().enumerate() {
        if let Some(other) = *hint {
            let (vreg, other) = (rep[index], rep[other.index()]);
            if vreg != other && rep_hints[vreg.index()].is_none() {
                rep_hints[vreg.index()] = Some(other);
            }
        }
    }
    mark_unsplittable(func, &liveness, &rep, &mut intervals);
    let mut block_starts = ArenaVec::with_capacity_in(func.blocks.len() + 1, arena);
    block_starts.extend((0..=func.blocks.len()).map(|block| liveness.block_start(block)));
    let mut loops = ArenaVec::new_in(arena);
    for block in 0..func.blocks.len() {
        for successor in func.successors(MBlock::new(block)) {
            if successor.index() <= block {
                loops.push((block_starts[successor.index()], block_starts[block + 1]));
            }
        }
    }
    loops.sort_unstable();
    let layout = Layout {
        depth:        &depth,
        block_starts: &block_starts,
        loops:        &loops,
    };
    let mut scan = Scan::new(
        intervals,
        &fixed,
        conv,
        func.dynamic,
        &rep_hints,
        &layout,
        arena,
    );
    scan::run(&mut scan);
    let fallbacks = scan.fallbacks;
    resolve::resolve(func, &liveness, &scan.intervals, rep, fallbacks, arena)
}

/// The size of the liveness allocation solves for `func` (after splitting
/// its critical edges and removing dead instructions): the registers its
/// live-in sets hold together, and the steps the solution took.
#[cfg(test)]
pub(crate) fn liveness_size<'a>(func: &mut MFunction<'a>, arena: &'a Bump) -> (usize, usize) {
    edges::split_critical_edges(func, arena);
    let liveness = Liveness::compute(func, arena);
    (liveness.stored(), liveness.steps())
}

/// Where each virtual register lives at each position, and the moves the
/// rewrite inserts between its locations.
pub(crate) struct Allocation<'a> {
    /// Each virtual register's representative: the registers coalescing
    /// joined share it, and only representatives have pieces.
    rep:                    ArenaVec<'a, VReg>,
    /// Each representative's pieces as `(from, location)`: the location
    /// holds the value from that position until the next piece's.
    pieces:                 ArenaVec<'a, (u32, Location)>,
    /// Where each register's pieces start in `pieces`, then their end.
    piece_index:            ArenaVec<'a, u32>,
    /// The moves to insert, by block, instruction and phase.
    edits:                  ArenaVec<'a, Edit>,
    /// Spill slots used, 8 bytes each.
    pub(crate) spill_slots: u32,
    /// Intervals left in a spill slot whose uses go through the scratch
    /// registers.
    pub(crate) fallbacks:   u32,
}

impl Allocation<'_> {
    /// Where `vreg` lives at `position`, a position where it is live.
    pub(crate) fn location(&self, vreg: VReg, position: u32) -> Location {
        let rep = self.rep[vreg.index()];
        let pieces = self.pieces(rep);
        let index = pieces.partition_point(|&(from, _)| from <= position);
        index
            .checked_sub(1)
            .map_or(Location::None, |index| pieces[index].1)
    }

    /// The pieces of a representative.
    fn pieces(&self, rep: VReg) -> &[(u32, Location)] {
        let start = self.piece_index[rep.index()] as usize;
        let end = self.piece_index[rep.index() + 1] as usize;
        &self.pieces[start..end]
    }

    /// Every location `vreg` has anywhere.
    pub(crate) fn locations(&self, vreg: VReg) -> impl Iterator<Item = Location> + '_ {
        self.pieces(self.rep[vreg.index()])
            .iter()
            .map(|&(_, location)| location)
    }

    /// The register that represents `vreg` after coalescing.
    #[cfg(test)]
    pub(crate) fn representative(&self, vreg: VReg) -> VReg {
        self.rep[vreg.index()]
    }

    /// Puts every piece of `vreg` in `location`, to show that the checker
    /// rejects a wrong allocation.
    #[cfg(test)]
    pub(crate) fn force(&mut self, vreg: VReg, location: Location) {
        let rep = self.rep[vreg.index()];
        let start = self.piece_index[rep.index()] as usize;
        let end = self.piece_index[rep.index() + 1] as usize;
        for piece in &mut self.pieces[start..end] {
            piece.1 = location;
        }
    }
}

/// A virtual register's home over one piece.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[expect(
    variant_size_differences,
    reason = "A spill slot is one word; locations are a dense per-piece table."
)]
pub(crate) enum Location {
    /// The register is not live there.
    None,
    Reg(PReg),
    Spill(u32),
}

/// A move of one register's value between two of its locations, inserted
/// before instruction `index` of `block`. Moves at one place form a
/// parallel copy per phase; a block's end has the split moves (phase 0)
/// before the edge moves (phase 1) that read their result.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Edit {
    block: u32,
    index: u32,
    phase: u8,
    vreg:  VReg,
    /// The position whose location the value moves from.
    src:   u32,
    /// The position whose location it moves to.
    dst:   u32,
}

/// The loop depth of each block, approximated from the layout: an edge to
/// an earlier (or the same) block closes a loop over the blocks between.
/// Only split heuristics read it.
fn loop_depths<'a>(func: &MFunction<'_>, arena: &'a Bump) -> ArenaVec<'a, u32> {
    let blocks = func.blocks.len();
    let mut depth = ArenaVec::with_capacity_in(blocks, arena);
    depth.resize(blocks, 0_u32);
    for block in 0..blocks {
        for successor in func.successors(MBlock::new(block)) {
            if successor.index() <= block {
                for inner in &mut depth[successor.index()..=block] {
                    *inner += 1;
                }
            }
        }
    }
    depth
}

/// Marks the intervals live into a block that an indirect jump reaches:
/// such an edge cannot be split, so the value keeps one location.
fn mark_unsplittable(
    func: &MFunction<'_>,
    liveness: &Liveness<'_>,
    rep: &[VReg],
    intervals: &mut [intervals::Interval<'_>],
) {
    for block in &func.blocks {
        if let Some(&MInst::JmpIndirect { dests, .. }) = block.insts.last() {
            for &target in &func.indirect[dests as usize] {
                for &vreg in liveness.live_in(target.index()) {
                    intervals[rep[vreg.index()].index()].unsplittable = true;
                }
            }
        }
    }
}
