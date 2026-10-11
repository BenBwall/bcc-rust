//! Dominators and dominance frontiers of a draft, computed on demand.
//!
//! The immediate dominators come from the iterative algorithm of Cooper,
//! Harvey and Kennedy ("A Simple, Fast Dominance Algorithm", 2001), as in
//! `ir/dominators.rs`, but over the draft's edges rather than a body's. The
//! frontiers come from the same paper's runner walk: for each block with two
//! or more reachable predecessors, every block on the dominator-tree path from
//! a predecessor up to (and not including) the block's immediate dominator has
//! the block in its frontier.

use super::{
    Draft,
    Predecessors,
};
use crate::{
    ir::{
        Block,
        Entity,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// The immediate dominator of every block the entry reaches, with the
/// reverse post-order it was computed over.
#[derive(Debug)]
pub(in crate::optimizer) struct Dominators<'s> {
    /// The reachable blocks, entry first.
    order:   ArenaVec<'s, Block>,
    /// Each block's index in `order`, or [`UNREACHABLE`].
    numbers: ArenaVec<'s, u32>,
    /// Each reachable block's immediate dominator; the entry's is itself.
    idoms:   ArenaVec<'s, Block>,
}

const UNREACHABLE: u32 = u32::MAX;

impl<'s> Draft<'_, 's> {
    /// The dominator tree of the reachable blocks. `predecessors` must be the
    /// draft's current predecessors.
    pub(in crate::optimizer) fn dominators(
        &self,
        predecessors: &Predecessors<'_>,
    ) -> Dominators<'s> {
        let order = self.reverse_post_order();
        let blocks = self.blocks.len();
        let mut numbers = ArenaVec::with_capacity_in(blocks, self.scratch);
        numbers.resize(blocks, UNREACHABLE);
        for (number, &block) in order.iter().enumerate() {
            numbers[block.index()] = u32::try_from(number).expect("too many blocks");
        }
        let mut idoms = ArenaVec::with_capacity_in(blocks, self.scratch);
        idoms.resize(blocks, Self::entry());
        let mut processed = ArenaVec::with_capacity_in(blocks, self.scratch);
        processed.resize(blocks, false);
        processed[Self::entry().index()] = true;
        let mut changed = true;
        while changed {
            changed = false;
            for &block in order.iter().skip(1) {
                let mut new_idom = None;
                for &predecessor in predecessors.of(block) {
                    if !processed[predecessor.index()] {
                        continue;
                    }
                    new_idom = Some(match new_idom {
                        | None => predecessor,
                        | Some(current) => intersect(&idoms, &numbers, predecessor, current),
                    });
                }
                let new_idom = new_idom.expect("a reachable block has a processed predecessor");
                if !processed[block.index()] || idoms[block.index()] != new_idom {
                    idoms[block.index()] = new_idom;
                    processed[block.index()] = true;
                    changed = true;
                }
            }
        }
        Dominators {
            order,
            numbers,
            idoms,
        }
    }
}

impl<'s> Dominators<'s> {
    /// The reachable blocks in reverse post-order, entry first. Every block
    /// comes after its immediate dominator.
    pub(in crate::optimizer) fn reverse_post_order(&self) -> &[Block] {
        &self.order
    }

    pub(in crate::optimizer) fn is_reachable(&self, block: Block) -> bool {
        self.numbers[block.index()] != UNREACHABLE
    }

    /// The immediate dominator of `block`; `None` for the entry and for
    /// unreachable blocks.
    pub(in crate::optimizer) fn idom(&self, block: Block) -> Option<Block> {
        let number = self.numbers[block.index()];
        (number != UNREACHABLE && number != 0).then(|| self.idoms[block.index()])
    }

    /// The dominance frontier of every reachable block, as rows of one array.
    /// Unreachable predecessors are ignored.
    pub(in crate::optimizer) fn frontiers(
        &self,
        predecessors: &Predecessors<'_>,
        scratch: &'s Bump,
    ) -> Frontiers<'s> {
        let blocks = self.numbers.len();
        // (frontier owner, frontier member) pairs, without duplicates per
        // member: a runner stops where an earlier one already recorded it.
        let mut pairs: ArenaVec<'s, (Block, Block)> = ArenaVec::new_in(scratch);
        let mut marked: ArenaVec<'s, Option<Block>> = ArenaVec::with_capacity_in(blocks, scratch);
        marked.resize(blocks, None);
        for &block in &self.order {
            let reachable = predecessors
                .of(block)
                .iter()
                .filter(|&&predecessor| self.is_reachable(predecessor));
            if reachable.clone().count() < 2 {
                continue;
            }
            let idom = self.idoms[block.index()];
            for &predecessor in reachable {
                let mut runner = predecessor;
                while runner != idom && marked[runner.index()] != Some(block) {
                    marked[runner.index()] = Some(block);
                    pairs.push((runner, block));
                    runner = self.idoms[runner.index()];
                }
            }
        }
        let mut starts = ArenaVec::with_capacity_in(blocks + 1, scratch);
        starts.resize(blocks + 1, 0_u32);
        for &(owner, _) in &pairs {
            starts[owner.index() + 1] += 1;
        }
        for index in 1..starts.len() {
            starts[index] += starts[index - 1];
        }
        let mut cursors = ArenaVec::with_capacity_in(blocks, scratch);
        cursors.extend_from_slice(&starts[..blocks]);
        let mut rows = ArenaVec::with_capacity_in(pairs.len(), scratch);
        rows.resize(pairs.len(), Block::new(0));
        for &(owner, member) in &pairs {
            let cursor = &mut cursors[owner.index()];
            rows[*cursor as usize] = member;
            *cursor += 1;
        }
        Frontiers { starts, rows }
    }
}

/// The dominance frontier of every block of a draft.
#[derive(Debug)]
pub(in crate::optimizer) struct Frontiers<'s> {
    starts: ArenaVec<'s, u32>,
    rows:   ArenaVec<'s, Block>,
}

impl Frontiers<'_> {
    /// The blocks where `block`'s dominance ends: those it does not strictly
    /// dominate but that have a predecessor it dominates.
    pub(in crate::optimizer) fn of(&self, block: Block) -> &[Block] {
        let index = block.index();
        &self.rows[self.starts[index] as usize..self.starts[index + 1] as usize]
    }
}

/// The nearest common dominator of two processed blocks.
fn intersect(idoms: &[Block], numbers: &[u32], mut a: Block, mut b: Block) -> Block {
    while a != b {
        while numbers[a.index()] > numbers[b.index()] {
            a = idoms[a.index()];
        }
        while numbers[b.index()] > numbers[a.index()] {
            b = idoms[b.index()];
        }
    }
    a
}
