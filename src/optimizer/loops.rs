//! Natural loops and their nesting, found from a dominator tree.
//!
//! An edge `latch -> header` is a *back edge* when `header` dominates
//! `latch`. The natural loop of a header is the header plus every block that
//! reaches one of its latches without passing through the header; back edges
//! into one header form one loop. Two natural loops with different headers
//! are disjoint or nested, so the loops form a forest. A retreating edge to a
//! block that does not dominate its source belongs to an irreducible cycle,
//! which is not a natural loop and is ignored.
//!
//! [`LoopForest::compute`] visits the headers in post-order of the dominator
//! walk (reverse of reverse post-order), so an inner loop is found before the
//! loops around it: an enclosing header dominates the inner one and comes
//! earlier in reverse post-order. Each loop's body is found by a backward
//! walk on an explicit stack from its latches. When the walk meets a block
//! that already belongs to an inner loop, it jumps to the outermost loop
//! found so far around that block, makes the current loop its parent, and
//! continues from that loop's header, so each block is claimed by its
//! innermost loop and each walk visits an inner loop only once. This is the
//! scheme of Cranelift's `loop_analysis.rs` and of LLVM's `LoopInfo`.

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

/// The natural loops of one function.
#[derive(Debug)]
pub(super) struct LoopForest<'s> {
    /// Every loop, inner loops before the loops that contain them.
    loops:     ArenaVec<'s, LoopData>,
    /// The innermost loop of each block, by block index; blocks past the end
    /// are in no loop.
    innermost: ArenaVec<'s, Option<Loop>>,
}

/// A loop of a [`LoopForest`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Loop(u32);

#[derive(Clone, Copy, Debug)]
struct LoopData {
    header: Block,
    /// The innermost loop around this one.
    parent: Option<Loop>,
}

impl<'s> LoopForest<'s> {
    /// Finds the natural loops of the graph `cfg`, whose dominator tree is
    /// `tree`. Unreachable blocks are in no loop.
    pub(super) fn compute(
        cfg: &ControlFlowGraph<'_>,
        tree: &DominatorTree<'_>,
        scratch: &'s Bump,
    ) -> Self {
        let mut forest = Self {
            loops:     ArenaVec::new_in(scratch),
            innermost: ArenaVec::with_capacity_in(cfg.block_count(), scratch),
        };
        forest.innermost.resize(cfg.block_count(), None);
        let mut stack = ArenaVec::new_in(scratch);
        for &header in tree.reverse_post_order().iter().rev() {
            stack.extend(
                cfg.predecessors(header)
                    .iter()
                    .copied()
                    .filter(|&latch| tree.dominates(header, latch)),
            );
            if stack.is_empty() {
                continue;
            }
            let current = Loop(u32::try_from(forest.loops.len()).expect("too many loops"));
            forest.loops.push(LoopData {
                header,
                parent: None,
            });
            forest.innermost[header.index()] = Some(current);
            while let Some(block) = stack.pop() {
                let next = match forest.innermost[block.index()] {
                    | None => {
                        forest.innermost[block.index()] = Some(current);
                        block
                    },
                    | Some(inner) => {
                        let outermost = forest.outermost(inner);
                        if outermost == current {
                            continue;
                        }
                        forest.loops[outermost.index()].parent = Some(current);
                        forest.header(outermost)
                    },
                };
                stack.extend(
                    cfg.predecessors(next)
                        .iter()
                        .copied()
                        .filter(|&predecessor| tree.is_reachable(predecessor)),
                );
            }
        }
        forest
    }

    /// Every loop, each before the loops that contain it.
    pub(super) fn loops(&self) -> impl Iterator<Item = Loop> + use<> {
        let count = u32::try_from(self.loops.len()).expect("too many loops");
        (0..count).map(Loop)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.loops.is_empty()
    }

    /// The block every edge into the loop from outside leads to, which
    /// dominates the loop's blocks.
    pub(super) fn header(&self, lp: Loop) -> Block {
        self.loops[lp.index()].header
    }

    /// The innermost loop that contains `lp`, if any.
    pub(super) fn parent(&self, lp: Loop) -> Option<Loop> {
        self.loops[lp.index()].parent
    }

    /// The innermost loop that contains `block`, if any.
    pub(super) fn innermost(&self, block: Block) -> Option<Loop> {
        self.innermost.get(block.index()).copied().flatten()
    }

    /// Whether `block` belongs to `lp` or to a loop inside it.
    pub(super) fn contains(&self, lp: Loop, block: Block) -> bool {
        let mut current = self.innermost(block);
        while let Some(candidate) = current {
            if candidate == lp {
                return true;
            }
            current = self.parent(candidate);
        }
        false
    }

    /// Records that `block`, added to the function after the analysis, has
    /// `lp` as its innermost loop, as a preheader has its loop's parent.
    pub(super) fn add_block(&mut self, block: Block, lp: Option<Loop>) {
        if self.innermost.len() <= block.index() {
            self.innermost.resize(block.index() + 1, None);
        }
        self.innermost[block.index()] = lp;
    }

    /// The outermost loop found so far that contains `lp`.
    fn outermost(&self, mut lp: Loop) -> Loop {
        while let Some(parent) = self.parent(lp) {
            lp = parent;
        }
        lp
    }
}

impl Loop {
    const fn index(self) -> usize {
        self.0 as usize
    }
}
