//! Dominance frontiers, found from a dominator tree.
//!
//! The dominance frontier of a block `b` is the set of blocks where `b`'s
//! dominance ends: those `b` does not strictly dominate but that have a
//! predecessor `b` dominates. They come from the runner walk of Cooper,
//! Harvey and Kennedy ("A Simple, Fast Dominance Algorithm", 2001), the paper
//! whose algorithm builds the [`DominatorTree`]: for each block with two or
//! more reachable predecessors, every block on the tree path from a
//! predecessor up to (and not including) the block's immediate dominator has
//! the block in its frontier.

use crate::{
    ir::{
        Block,
        ControlFlowGraph,
        DominatorTree,
        Entity,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// The dominance frontier of every block of one function, as rows of one
/// array.
#[derive(Debug)]
pub(super) struct DominanceFrontiers<'s> {
    starts: ArenaVec<'s, u32>,
    rows:   ArenaVec<'s, Block>,
}

impl<'s> DominanceFrontiers<'s> {
    /// Finds the frontiers of the graph `cfg`, whose dominator tree is
    /// `tree`. Unreachable blocks have empty frontiers, and unreachable
    /// predecessors are ignored.
    pub(super) fn compute(
        cfg: &ControlFlowGraph<'_>,
        tree: &DominatorTree<'_>,
        scratch: &'s Bump,
    ) -> Self {
        let blocks = cfg.block_count();
        // (frontier owner, frontier member) pairs, without duplicates per
        // member: a runner stops where an earlier one already recorded it.
        let mut pairs: ArenaVec<'s, (Block, Block)> = ArenaVec::new_in(scratch);
        let mut marked: ArenaVec<'s, Option<Block>> = ArenaVec::with_capacity_in(blocks, scratch);
        marked.resize(blocks, None);
        for &block in tree.reverse_post_order() {
            let reachable = cfg
                .predecessors(block)
                .iter()
                .filter(|&&predecessor| tree.is_reachable(predecessor));
            if reachable.clone().count() < 2 {
                continue;
            }
            let idom = tree
                .idom(block)
                .expect("a block with two predecessors is not the entry");
            for &predecessor in reachable {
                let mut runner = predecessor;
                while runner != idom && marked[runner.index()] != Some(block) {
                    marked[runner.index()] = Some(block);
                    pairs.push((runner, block));
                    // `idom` dominates the predecessor, so the walk meets it
                    // before it could pass the entry.
                    runner = tree
                        .idom(runner)
                        .expect("the walk meets the immediate dominator first");
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
        Self { starts, rows }
    }

    /// The dominance frontier of `block`.
    pub(super) fn of(&self, block: Block) -> &[Block] {
        let index = block.index();
        &self.rows[self.starts[index] as usize..self.starts[index + 1] as usize]
    }
}
