//! Reverse post-order and the dominator tree.
//!
//! Block `a` dominates block `b` if every path from the entry to `b` passes
//! through `a`. The tree is computed with the iterative algorithm of Cooper,
//! Harvey and Kennedy ("A Simple, Fast Dominance Algorithm", 2001): visit the
//! reachable blocks in reverse post-order, set each block's immediate
//! dominator to the intersection of its processed predecessors' dominators,
//! and repeat until nothing changes. The intersection walks two blocks up the
//! tree by their post-order numbers until they meet. The post-order itself
//! comes from a depth-first search on an explicit stack. A depth-first walk
//! of the finished tree numbers each block on entry and exit, so a dominance
//! query compares two intervals instead of walking up the tree.

use super::{
    Body,
    cfg::ControlFlowGraph,
    entities::{
        Block,
        Entity,
    },
};
use crate::util::bump::{
    ArenaVec,
    Bump,
};

/// The immediate dominator of every reachable block, and the reverse
/// post-order it was computed over.
#[derive(Debug)]
pub(crate) struct DominatorTree<'s> {
    /// The reachable blocks, entry first.
    reverse_post_order: ArenaVec<'s, Block>,
    /// Each block's index in `reverse_post_order`, or `UNREACHABLE`.
    rpo_numbers:        ArenaVec<'s, u32>,
    /// Each reachable block's immediate dominator; the entry's is itself.
    idoms:              ArenaVec<'s, Block>,
    /// Each reachable block's preorder number in the dominator tree, and
    /// the largest number among the blocks it dominates.
    intervals:          ArenaVec<'s, (u32, u32)>,
}

const UNREACHABLE: u32 = u32::MAX;

impl<'s> DominatorTree<'s> {
    /// Computes the tree of `body` in `scratch`. The entry is block 0; a
    /// body without blocks has an empty tree.
    pub(crate) fn compute(body: &Body<'_>, cfg: &ControlFlowGraph<'_>, scratch: &'s Bump) -> Self {
        debug_assert_eq!(
            body.block_count(),
            cfg.block_count(),
            "the graph is of another body"
        );
        Self::from_cfg(cfg, scratch)
    }

    /// Computes the tree of the graph `cfg` alone, whose entry is block 0.
    /// A pass whose working copy of a function is not a [`Body`] uses this
    /// with [`ControlFlowGraph::from_successors`].
    pub(crate) fn from_cfg(cfg: &ControlFlowGraph<'_>, scratch: &'s Bump) -> Self {
        let reverse_post_order = reverse_post_order(cfg, scratch);
        let blocks = cfg.block_count();
        let mut rpo_numbers = ArenaVec::with_capacity_in(blocks, scratch);
        rpo_numbers.resize(blocks, UNREACHABLE);
        for (number, &block) in reverse_post_order.iter().enumerate() {
            rpo_numbers[block.index()] = u32::try_from(number).expect("too many blocks");
        }
        let mut idoms = ArenaVec::with_capacity_in(blocks, scratch);
        idoms.resize(blocks, Block::new(0));
        let mut processed = ArenaVec::with_capacity_in(blocks, scratch);
        processed.resize(blocks, false);
        if let Some(&entry) = reverse_post_order.first() {
            processed[entry.index()] = true;
        }

        let mut changed = true;
        while changed {
            changed = false;
            for &block in reverse_post_order.iter().skip(1) {
                let mut new_idom = None;
                for &predecessor in cfg.predecessors(block) {
                    if !processed[predecessor.index()] {
                        continue;
                    }
                    new_idom = Some(match new_idom {
                        | None => predecessor,
                        | Some(current) => intersect(&idoms, &rpo_numbers, predecessor, current),
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
        let intervals = tree_intervals(&reverse_post_order, &idoms, scratch);
        Self {
            reverse_post_order,
            rpo_numbers,
            idoms,
            intervals,
        }
    }

    /// The immediate dominator of `block`; `None` for the entry and for
    /// unreachable blocks.
    pub(crate) fn idom(&self, block: Block) -> Option<Block> {
        let number = *self.rpo_numbers.get(block.index())?;
        (number != UNREACHABLE && number != 0).then(|| self.idoms[block.index()])
    }

    /// Whether every path from the entry to `b` passes through `a`. A block
    /// dominates itself; an unreachable block dominates and is dominated by
    /// nothing.
    pub(crate) fn dominates(&self, a: Block, b: Block) -> bool {
        if !self.is_reachable(a) || !self.is_reachable(b) {
            return false;
        }
        // `a` dominates exactly the blocks numbered within its subtree.
        let (first, last) = self.intervals[a.index()];
        (first..=last).contains(&self.intervals[b.index()].0)
    }

    pub(crate) fn is_reachable(&self, block: Block) -> bool {
        self.rpo_numbers
            .get(block.index())
            .is_some_and(|&number| number != UNREACHABLE)
    }

    /// The reachable blocks in reverse post-order, entry first.
    pub(crate) fn reverse_post_order(&self) -> &[Block] {
        &self.reverse_post_order
    }
}

/// The nearest common dominator of two processed blocks.
fn intersect(idoms: &[Block], rpo_numbers: &[u32], mut a: Block, mut b: Block) -> Block {
    while a != b {
        while rpo_numbers[a.index()] > rpo_numbers[b.index()] {
            a = idoms[a.index()];
        }
        while rpo_numbers[b.index()] > rpo_numbers[a.index()] {
            b = idoms[b.index()];
        }
    }
    a
}

/// Numbers the dominator tree in preorder: each reachable block gets its own
/// number and the largest number in its subtree. Children are gathered by
/// counting, so the walk is linear in the number of blocks.
fn tree_intervals<'s>(
    reverse_post_order: &[Block],
    idoms: &[Block],
    scratch: &'s Bump,
) -> ArenaVec<'s, (u32, u32)> {
    let blocks = idoms.len();
    let mut intervals = ArenaVec::with_capacity_in(blocks, scratch);
    intervals.resize(blocks, (0, 0));
    let Some((&entry, rest)) = reverse_post_order.split_first() else {
        return intervals;
    };
    // `starts[p]..starts[p + 1]` indexes the children of block `p`.
    let mut starts = ArenaVec::with_capacity_in(blocks + 1, scratch);
    starts.resize(blocks + 1, 0_u32);
    for &block in rest {
        starts[idoms[block.index()].index() + 1] += 1;
    }
    for index in 0..blocks {
        starts[index + 1] += starts[index];
    }
    let mut filled = ArenaVec::with_capacity_in(blocks, scratch);
    filled.extend_from_slice(&starts[..blocks]);
    let mut children = ArenaVec::with_capacity_in(rest.len(), scratch);
    children.resize(rest.len(), entry);
    for &block in rest {
        let parent = idoms[block.index()].index();
        children[filled[parent] as usize] = block;
        filled[parent] += 1;
    }
    let mut next = 0_u32;
    let mut stack: ArenaVec<'_, (Block, u32)> = ArenaVec::new_in(scratch);
    intervals[entry.index()].0 = next;
    stack.push((entry, starts[entry.index()]));
    while let Some(top) = stack.last_mut() {
        let (block, child) = *top;
        if child < starts[block.index() + 1] {
            top.1 += 1;
            let child = children[child as usize];
            next += 1;
            intervals[child.index()].0 = next;
            stack.push((child, starts[child.index()]));
        } else {
            intervals[block.index()].1 = next;
            _ = stack.pop();
        }
    }
    intervals
}

/// The blocks reachable from block 0 in reverse post-order, by a depth-first
/// search whose stack holds each open block with its next successor index.
fn reverse_post_order<'s>(cfg: &ControlFlowGraph<'_>, scratch: &'s Bump) -> ArenaVec<'s, Block> {
    let mut order = ArenaVec::new_in(scratch);
    if cfg.block_count() == 0 {
        return order;
    }
    let entry = Block::new(0);
    let mut visited = ArenaVec::with_capacity_in(cfg.block_count(), scratch);
    visited.resize(cfg.block_count(), false);
    let mut stack: ArenaVec<'_, (Block, usize)> = ArenaVec::new_in(scratch);
    visited[entry.index()] = true;
    stack.push((entry, 0));
    while let Some(top) = stack.last_mut() {
        let (block, next) = *top;
        if let Some(&successor) = cfg.successors(block).get(next) {
            top.1 += 1;
            if !visited[successor.index()] {
                visited[successor.index()] = true;
                stack.push((successor, 0));
            }
        } else {
            order.push(block);
            _ = stack.pop();
        }
    }
    order.reverse();
    order
}
