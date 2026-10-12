//! Analyses over a draft, computed on demand. A pass may reuse CFG data
//! while live blocks and edge targets stay unchanged; graph edits invalidate
//! it.

use super::Draft;
use crate::{
    ir::{
        Block,
        ControlFlowGraph,
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
        Predecessors {
            cfg: self.control_flow_graph(),
        }
    }

    /// The edges between the live blocks, numbered as in the draft, for
    /// [`DominatorTree::from_cfg`](crate::ir::DominatorTree::from_cfg). A
    /// removed block has no edges.
    pub(in crate::optimizer) fn control_flow_graph(&self) -> ControlFlowGraph<'s> {
        ControlFlowGraph::from_successors(self.blocks.len(), self.scratch, |block, add| {
            let data = self.block(block);
            if data.alive {
                data.term.edges().for_each(|edge| add(edge.target));
            }
        })
    }

    /// How many times each original value is used by the instructions and
    /// terminators of the live blocks, after replacement.
    pub(in crate::optimizer) fn use_counts(&self) -> ArenaVec<'s, u32> {
        let mut counts = ArenaVec::with_capacity_in(self.value_count(), self.scratch);
        counts.resize(self.value_count(), 0_u32);
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
    cfg: ControlFlowGraph<'s>,
}

impl Predecessors<'_> {
    /// The distinct live blocks with an edge to `block`.
    pub(in crate::optimizer) fn of(&self, block: Block) -> &[Block] {
        self.cfg.predecessors(block)
    }
}
