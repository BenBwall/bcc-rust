//! Control-flow edges of a body as an analysis.
//!
//! The IR keeps no predecessor lists; a pass that needs them computes a
//! [`ControlFlowGraph`] in its scratch arena. Each block's successors come
//! from its terminator, with repeated targets listed once, and predecessors
//! are the reverse. Both are stored in compressed rows: one flat array of
//! blocks with a start offset per block.

use super::{
    Body,
    entities::{
        Block,
        Entity,
    },
};
use crate::util::bump::{
    ArenaVec,
    Bump,
};

/// The successors and predecessors of every block of one body.
#[derive(Debug)]
pub(crate) struct ControlFlowGraph<'s> {
    /// Block `b`'s successors are `successors[successor_starts[b]..
    /// successor_starts[b + 1]]`.
    successor_starts:   ArenaVec<'s, u32>,
    successors:         ArenaVec<'s, Block>,
    predecessor_starts: ArenaVec<'s, u32>,
    predecessors:       ArenaVec<'s, Block>,
}

impl<'s> ControlFlowGraph<'s> {
    /// Computes the edges of `body` in `scratch`. A block without a
    /// terminator has no successors.
    pub(crate) fn compute(body: &Body<'_>, scratch: &'s Bump) -> Self {
        let blocks = body.block_count();
        let mut successor_starts = ArenaVec::with_capacity_in(blocks + 1, scratch);
        let mut successors = ArenaVec::new_in(scratch);
        let mut predecessor_counts = ArenaVec::with_capacity_in(blocks + 1, scratch);
        predecessor_counts.resize(blocks + 1, 0_u32);
        for block in body.blocks() {
            successor_starts.push(offset(successors.len()));
            let first = successors.len();
            if let Some(terminator) = body.terminator(block) {
                body.visit_destinations(terminator, |call| {
                    let (target, _) = body.block_call(call);
                    if !successors[first..].contains(&target) && target.index() < blocks {
                        successors.push(target);
                        predecessor_counts[target.index() + 1] += 1;
                    }
                });
            }
        }
        successor_starts.push(offset(successors.len()));

        // Prefix sums turn the counts into start offsets; filling each row
        // then advances a cursor per block.
        for index in 1..predecessor_counts.len() {
            predecessor_counts[index] += predecessor_counts[index - 1];
        }
        let predecessor_starts = predecessor_counts;
        let mut cursors = ArenaVec::with_capacity_in(blocks, scratch);
        cursors.extend_from_slice(&predecessor_starts[..blocks]);
        let mut predecessors = ArenaVec::with_capacity_in(successors.len(), scratch);
        predecessors.resize(successors.len(), Block::new(0));
        for block in body.blocks() {
            let row = successor_starts[block.index()] as usize
                ..successor_starts[block.index() + 1] as usize;
            for &target in &successors[row] {
                let cursor = &mut cursors[target.index()];
                predecessors[*cursor as usize] = block;
                *cursor += 1;
            }
        }
        Self {
            successor_starts,
            successors,
            predecessor_starts,
            predecessors,
        }
    }

    /// The distinct blocks `block`'s terminator branches to, in the order it
    /// names them.
    pub(crate) fn successors(&self, block: Block) -> &[Block] {
        let index = block.index();
        &self.successors
            [self.successor_starts[index] as usize..self.successor_starts[index + 1] as usize]
    }

    /// The distinct blocks that branch to `block`, in layout order.
    pub(crate) fn predecessors(&self, block: Block) -> &[Block] {
        let index = block.index();
        &self.predecessors
            [self.predecessor_starts[index] as usize..self.predecessor_starts[index + 1] as usize]
    }

    pub(crate) fn block_count(&self) -> usize {
        self.successor_starts.len() - 1
    }
}

fn offset(len: usize) -> u32 {
    u32::try_from(len).expect("too many control-flow edges")
}
