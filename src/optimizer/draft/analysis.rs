//! Analyses over a draft, computed on demand and never kept: no pass keeps
//! one across an edit, so none can go stale.

use super::Draft;
use crate::{
    ir::{
        Block,
        Entity,
    },
    util::bump::ArenaVec,
};

impl<'s> Draft<'_, 's> {
    /// The blocks reachable from the entry, in reverse post-order, found by
    /// a depth-first search on an explicit stack. An entry `(block, true)`
    /// closes a block whose successors were pushed.
    pub(in crate::optimizer) fn reverse_post_order(&self) -> ArenaVec<'s, Block> {
        let mut order = ArenaVec::new_in(self.scratch);
        let mut visited = ArenaVec::with_capacity_in(self.blocks.len(), self.scratch);
        visited.resize(self.blocks.len(), false);
        let mut stack = ArenaVec::new_in(self.scratch);
        stack.push((Self::entry(), false));
        while let Some((block, closing)) = stack.pop() {
            if closing {
                order.push(block);
                continue;
            }
            if visited[block.index()] {
                continue;
            }
            visited[block.index()] = true;
            stack.push((block, true));
            // The first successor must be visited first, so it goes on last.
            let first = stack.len();
            for edge in self.block(block).term.edges() {
                if !visited[edge.target.index()] {
                    stack.push((edge.target, false));
                }
            }
            stack[first..].reverse();
        }
        order.reverse();
        order
    }

    /// Whether each block can be reached from the entry.
    pub(in crate::optimizer) fn reachable(&self) -> ArenaVec<'s, bool> {
        let mut reachable = ArenaVec::with_capacity_in(self.blocks.len(), self.scratch);
        reachable.resize(self.blocks.len(), false);
        for block in self.reverse_post_order() {
            reachable[block.index()] = true;
        }
        reachable
    }

    /// How many edges lead to each block from the reachable blocks. A
    /// `switch` with two cases to one block counts two.
    pub(in crate::optimizer) fn incoming_edge_counts(&self) -> ArenaVec<'s, u32> {
        let mut counts = ArenaVec::with_capacity_in(self.blocks.len(), self.scratch);
        counts.resize(self.blocks.len(), 0_u32);
        for block in self.reverse_post_order() {
            for edge in self.block(block).term.edges() {
                counts[edge.target.index()] += 1;
            }
        }
        counts
    }

    /// The distinct live predecessors of every block, reachable or not, as
    /// rows of one array.
    pub(in crate::optimizer) fn predecessors(&self) -> Predecessors<'s> {
        let blocks = self.blocks.len();
        let mut starts = ArenaVec::with_capacity_in(blocks + 1, self.scratch);
        starts.resize(blocks + 1, 0_u32);
        let mut pairs: ArenaVec<'s, (Block, Block)> = ArenaVec::new_in(self.scratch);
        for block in self.alive_blocks() {
            let first = pairs.len();
            for edge in self.block(block).term.edges() {
                if !pairs[first..]
                    .iter()
                    .any(|&(target, _)| target == edge.target)
                {
                    pairs.push((edge.target, block));
                    starts[edge.target.index() + 1] += 1;
                }
            }
        }
        for index in 1..starts.len() {
            starts[index] += starts[index - 1];
        }
        let mut cursors = ArenaVec::with_capacity_in(blocks, self.scratch);
        cursors.extend_from_slice(&starts[..blocks]);
        let mut rows = ArenaVec::with_capacity_in(pairs.len(), self.scratch);
        rows.resize(pairs.len(), Block::new(0));
        for &(target, source) in &pairs {
            let cursor = &mut cursors[target.index()];
            rows[*cursor as usize] = source;
            *cursor += 1;
        }
        Predecessors { starts, rows }
    }

    /// How many times each original value is used by the instructions and
    /// terminators of the live blocks, after replacement.
    pub(in crate::optimizer) fn use_counts(&self) -> ArenaVec<'s, u32> {
        let mut counts = ArenaVec::with_capacity_in(self.body.value_count(), self.scratch);
        counts.resize(self.body.value_count(), 0_u32);
        for block in self.alive_blocks() {
            let data = self.block(block);
            for &inst in &data.insts {
                if !self.is_removed(inst) {
                    self.for_each_operand(inst, |value| counts[value.index()] += 1);
                }
            }
            data.term
                .for_each_control_operand(|value| counts[self.resolve(value).index()] += 1);
            for edge in data.term.edges() {
                for &arg in &edge.args {
                    counts[self.resolve(arg).index()] += 1;
                }
            }
        }
        counts
    }
}

/// The predecessors of every block of a draft.
#[derive(Debug)]
pub(in crate::optimizer) struct Predecessors<'s> {
    starts: ArenaVec<'s, u32>,
    rows:   ArenaVec<'s, Block>,
}

impl Predecessors<'_> {
    /// The distinct live blocks with an edge to `block`.
    pub(in crate::optimizer) fn of(&self, block: Block) -> &[Block] {
        let index = block.index();
        &self.rows[self.starts[index] as usize..self.starts[index + 1] as usize]
    }
}
