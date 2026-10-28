//! Structured control flow from a control-flow graph, for WebAssembly's
//! `block`, `loop`, `if` and `br`.
//!
//! [`stackify`] follows Norman Ramsey, "Beyond Relooper: Recursive
//! Translation of Unstructured Control Flow to Structured Control Flow"
//! (ICFP 2022), which translates any reducible graph along its dominator
//! tree. A node's code is its own instructions followed by the code of the
//! nodes it immediately dominates: a dominated node with one forward
//! in-edge (a non-merge node) is placed inline where its edge leaves, and
//! each merge node (two or more forward in-edges) follows a `block` that
//! encloses the dominating node's code, so every edge into it is a `br` out
//! of that block. Merge children are nested with the latest in reverse
//! post-order outermost, since forward edges go to later nodes. A loop
//! header (the target of a back edge) wraps its code in a `loop`, which
//! back edges continue with `br`. Where the paper recurses, this keeps an
//! explicit work stack, so a deep dominator tree cannot exhaust the native
//! stack.
//!
//! An irreducible graph (a retreating edge whose target does not dominate
//! its source) has no such translation. It gets a dispatch loop instead:
//! one `loop` around a `block` per node, entered through a `br_table` on a
//! label local that every edge sets before it continues the loop. Edges
//! then never branch to a node's code directly.
//!
//! For `while (i < n) i++; return i;` the graph is `0 → 1`, `1 → 2 | 3`,
//! `2 → 1`, and `3` returns. Node 1 has one forward in-edge (the back edge
//! from 2 does not count), so it is no merge node, but it is a loop header;
//! nodes 2 and 3 each have one in-edge from 1, which dominates them. The
//! translation is `body(0); loop { body(1); if { body(2); br 1 } else {
//! body(3); return } end } end`.
//!
//! The graph and the code are the caller's: [`Graph`] describes the
//! terminators and edges, and an [`Emitter`] receives the structure.

use super::u32_of;
use crate::util::bump::{
    ArenaVec,
    Bump,
};

/// How a block ends, as the stackifier needs to know.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Exit {
    /// No successor: a return or an `unreachable`.
    Leave,
    /// One edge.
    Jump,
    /// Two edges, taken when the condition is true and when it is false;
    /// the emitter opens an `if` on the condition.
    Branch,
    /// Any number of edges, one of which the emitter selects with
    /// [`Emitter::select`]: a `switch` or an `indirect_br`.
    Select,
}

/// A control-flow graph: per block, how it ends and the target of each of
/// its edges. Block 0 is the entry. Edges are listed per block in the
/// order the emitter numbers them.
pub(crate) struct Graph<'a> {
    pub(crate) exits:   &'a [Exit],
    /// Block `b`'s edges are `targets[starts[b]..starts[b + 1]]`.
    pub(crate) starts:  &'a [u32],
    pub(crate) targets: &'a [u32],
}

impl Graph<'_> {
    fn edges(&self, block: usize) -> &[u32] {
        &self.targets[self.starts[block] as usize..self.starts[block + 1] as usize]
    }

    fn len(&self) -> usize {
        self.exits.len()
    }
}

/// Where an edge goes once its block parameters are assigned.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Target {
    /// `br` to this depth.
    Br(u32),
    /// Nothing: the target's code follows.
    Inline,
    /// Dispatch mode: set the label local to `index` and `br` to `depth`,
    /// the dispatch loop.
    Dispatch { index: u32, depth: u32 },
}

/// The receiver of the structure, in code order.
pub(crate) trait Emitter {
    /// `block` with no result.
    fn block(&mut self);
    /// `loop` with no result.
    fn r#loop(&mut self);
    /// `if` on the condition of `from`'s two-way branch.
    fn r#if(&mut self, from: u32);
    /// `else`.
    fn r#else(&mut self);
    /// `end`.
    fn end(&mut self);
    /// The block's instructions before its terminator.
    fn body(&mut self, block: u32);
    /// Edge `edge` of `from`: its block arguments, then `target`.
    fn edge(&mut self, from: u32, edge: u32, target: Target);
    /// A terminator without edges.
    fn leave(&mut self, from: u32);
    /// The selection of `from`'s [`Exit::Select`] terminator: edge `i` is a
    /// `br` to `depths[i]`, whose block ends just before the edge's code.
    fn select(&mut self, from: u32, depths: &[u32]);
    /// Dispatch mode's head, inside `count` blocks: a `br_table` on the
    /// label local, whose value `i` branches to depth `i`.
    fn dispatch(&mut self, count: u32);
}

/// Translates `graph` into structured control flow for `emitter`. Blocks
/// that the entry does not reach get no code. Returns whether the graph was
/// irreducible and got the dispatch loop.
pub(crate) fn stackify(graph: &Graph<'_>, emitter: &mut impl Emitter, scratch: &Bump) -> bool {
    let analysis = Analysis::new(graph, scratch);
    if analysis.irreducible {
        dispatch(graph, &analysis, emitter, scratch);
        return true;
    }
    let mut positions = ArenaVec::with_capacity_in(2 * graph.len(), scratch);
    positions.resize(2 * graph.len(), u32::MAX);
    let mut stackifier = Stackifier {
        graph,
        analysis: &analysis,
        frames: ArenaVec::new_in(scratch),
        positions,
        work: ArenaVec::new_in(scratch),
        depths: ArenaVec::new_in(scratch),
    };
    stackifier.work.push(Work::Tree(0));
    while let Some(work) = stackifier.work.pop() {
        stackifier.step(work, emitter);
    }
    false
}

/// What remains to do, innermost last.
#[derive(Clone, Copy)]
enum Work {
    /// The paper's `doTree`: a node and everything it dominates.
    Tree(u32),
    /// The paper's `nodeWithin`: `node` inside the blocks of its merge
    /// children from `next` on, in the analysis's order.
    Within { node: u32, next: u32 },
    /// The node's terminator.
    Terminator(u32),
    /// Edge `edge` of `from`, resolved when it is reached.
    Edge { from: u32, edge: u32 },
    /// `else`.
    Else,
    /// `end`, leaving the innermost frame.
    End,
}

/// What a `br` from inside an enclosing construct reaches.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Frame {
    /// The end of a block that the node's code follows.
    Followed(u32),
    /// The head of the loop the node heads.
    Loop(u32),
    /// A construct no edge targets by name: an `if` or a selection block.
    Anonymous,
}

struct Stackifier<'g, 'a, 's> {
    graph:     &'g Graph<'g>,
    analysis:  &'a Analysis<'s>,
    frames:    ArenaVec<'s, Frame>,
    /// The frame position of each node's `Followed` frame (at `2 * node`)
    /// and `Loop` frame (at `2 * node + 1`), while it is open.
    positions: ArenaVec<'s, u32>,
    work:      ArenaVec<'s, Work>,
    depths:    ArenaVec<'s, u32>,
}

impl Stackifier<'_, '_, '_> {
    fn step(&mut self, work: Work, emitter: &mut impl Emitter) {
        match work {
            | Work::Tree(node) => {
                if self.analysis.loop_header[node as usize] {
                    emitter.r#loop();
                    self.push(Frame::Loop(node));
                    self.work.push(Work::End);
                }
                self.work.push(Work::Within { node, next: 0 });
            },
            | Work::Within { node, next } => {
                let merges = self.analysis.merge_children(node);
                if let Some(&merge) = merges.get(next as usize) {
                    emitter.block();
                    self.push(Frame::Followed(merge));
                    self.work.push(Work::Tree(merge));
                    self.work.push(Work::End);
                    self.work.push(Work::Within {
                        node,
                        next: next + 1,
                    });
                } else {
                    emitter.body(node);
                    self.work.push(Work::Terminator(node));
                }
            },
            | Work::Terminator(node) => self.terminator(node, emitter),
            | Work::Edge { from, edge } => {
                let target = self.graph.edges(from as usize)[edge as usize];
                let resolved =
                    if self.analysis.rpo[target as usize] <= self.analysis.rpo[from as usize] {
                        Target::Br(self.depth(Frame::Loop(target)))
                    } else if self.analysis.merge[target as usize] {
                        Target::Br(self.depth(Frame::Followed(target)))
                    } else {
                        Target::Inline
                    };
                emitter.edge(from, edge, resolved);
                if resolved == Target::Inline {
                    self.work.push(Work::Tree(target));
                }
            },
            | Work::Else => emitter.r#else(),
            | Work::End => {
                emitter.end();
                if let Some(frame) = self.frames.pop()
                    && let Some(slot) = Self::slot(frame)
                {
                    self.positions[slot] = u32::MAX;
                }
            },
        }
    }

    fn terminator(&mut self, node: u32, emitter: &mut impl Emitter) {
        match self.graph.exits[node as usize] {
            | Exit::Leave => emitter.leave(node),
            | Exit::Jump => self.work.push(Work::Edge {
                from: node,
                edge: 0,
            }),
            | Exit::Branch => {
                emitter.r#if(node);
                self.push(Frame::Anonymous);
                self.work.push(Work::End);
                self.work.push(Work::Edge {
                    from: node,
                    edge: 1,
                });
                self.work.push(Work::Else);
                self.work.push(Work::Edge {
                    from: node,
                    edge: 0,
                });
            },
            | Exit::Select => {
                let count = u32_of(self.graph.edges(node as usize).len());
                for _ in 0..count {
                    emitter.block();
                    self.push(Frame::Anonymous);
                }
                self.depths.clear();
                self.depths.extend(0..count);
                emitter.select(node, &self.depths);
                for edge in (0..count).rev() {
                    self.work.push(Work::Edge { from: node, edge });
                    self.work.push(Work::End);
                }
            },
        }
    }

    fn push(&mut self, frame: Frame) {
        if let Some(slot) = Self::slot(frame) {
            self.positions[slot] = u32_of(self.frames.len());
        }
        self.frames.push(frame);
    }

    /// Where `positions` records a named frame.
    const fn slot(frame: Frame) -> Option<usize> {
        match frame {
            | Frame::Followed(node) => Some(2 * node as usize),
            | Frame::Loop(node) => Some(2 * node as usize + 1),
            | Frame::Anonymous => None,
        }
    }

    /// The `br` depth of the open frame `frame`.
    fn depth(&self, frame: Frame) -> u32 {
        let slot = Self::slot(frame).expect("only named frames are targets");
        let position = self.positions[slot];
        assert!(
            position != u32::MAX,
            "a reducible graph's edge targets an enclosing construct"
        );
        u32_of(self.frames.len()) - 1 - position
    }
}

/// Dispatch mode: `loop { block × n { dispatch } code(0) ... code(n-1) }`,
/// with node `i` the `i`th reachable node in reverse post-order.
fn dispatch(
    graph: &Graph<'_>,
    analysis: &Analysis<'_>,
    emitter: &mut impl Emitter,
    scratch: &Bump,
) {
    let count = u32_of(analysis.order.len());
    emitter.r#loop();
    for _ in 0..count {
        emitter.block();
    }
    emitter.dispatch(count);
    let mut depths = ArenaVec::new_in(scratch);
    for (position, &node) in analysis.order.iter().enumerate() {
        emitter.end();
        emitter.body(node);
        // Inside the blocks of the later nodes and the loop.
        let outer = count - 1 - u32_of(position);
        let resolve = |target: u32, extra: u32| Target::Dispatch {
            index: analysis.rpo[target as usize],
            depth: outer + extra,
        };
        let edges = graph.edges(node as usize);
        match graph.exits[node as usize] {
            | Exit::Leave => emitter.leave(node),
            | Exit::Jump => emitter.edge(node, 0, resolve(edges[0], 0)),
            | Exit::Branch => {
                emitter.r#if(node);
                emitter.edge(node, 0, resolve(edges[0], 1));
                emitter.r#else();
                emitter.edge(node, 1, resolve(edges[1], 1));
                emitter.end();
            },
            | Exit::Select => {
                let edge_count = u32_of(edges.len());
                for _ in 0..edge_count {
                    emitter.block();
                }
                depths.clear();
                depths.extend(0..edge_count);
                emitter.select(node, &depths);
                for (edge, &target) in edges.iter().enumerate() {
                    emitter.end();
                    let open = edge_count - 1 - u32_of(edge);
                    emitter.edge(node, u32_of(edge), resolve(target, open));
                }
            },
        }
    }
    emitter.end();
}

/// Reverse post-order, dominators, loop headers and merge nodes.
struct Analysis<'s> {
    /// Each node's position in reverse post-order; `u32::MAX` if the entry
    /// does not reach it.
    rpo:          ArenaVec<'s, u32>,
    /// The reachable nodes in reverse post-order.
    order:        ArenaVec<'s, u32>,
    loop_header:  ArenaVec<'s, bool>,
    merge:        ArenaVec<'s, bool>,
    /// Each node's merge children, latest in reverse post-order first:
    /// `children[child_starts[n]..child_starts[n + 1]]`.
    child_starts: ArenaVec<'s, u32>,
    children:     ArenaVec<'s, u32>,
    irreducible:  bool,
}

impl<'s> Analysis<'s> {
    fn new(graph: &Graph<'_>, scratch: &'s Bump) -> Self {
        let n = graph.len();
        let mut rpo = ArenaVec::with_capacity_in(n, scratch);
        rpo.resize(n, u32::MAX);
        // Iterative depth-first post-order from the entry.
        let mut post = ArenaVec::with_capacity_in(n, scratch);
        let mut seen = ArenaVec::with_capacity_in(n, scratch);
        seen.resize(n, false);
        let mut stack: ArenaVec<'_, (u32, u32)> = ArenaVec::new_in(scratch);
        if n > 0 {
            seen[0] = true;
            stack.push((0, 0));
        }
        while let Some(&mut (node, ref mut next)) = stack.last_mut() {
            let edges = graph.edges(node as usize);
            if let Some(&target) = edges.get(*next as usize) {
                *next += 1;
                if !seen[target as usize] {
                    seen[target as usize] = true;
                    stack.push((target, 0));
                }
            } else {
                post.push(node);
                _ = stack.pop();
            }
        }
        let mut order = ArenaVec::with_capacity_in(post.len(), scratch);
        order.extend(post.iter().rev().copied());
        for (position, &node) in order.iter().enumerate() {
            rpo[node as usize] = u32_of(position);
        }
        // Predecessor edges of reachable nodes.
        let mut idom = ArenaVec::with_capacity_in(n, scratch);
        idom.resize(n, u32::MAX);
        if n > 0 {
            idom[0] = 0;
        }
        let mut pred_starts = ArenaVec::with_capacity_in(n + 1, scratch);
        pred_starts.resize(n + 1, 0_u32);
        for &node in &order {
            for &target in graph.edges(node as usize) {
                pred_starts[target as usize + 1] += 1;
            }
        }
        for index in 0..n {
            pred_starts[index + 1] += pred_starts[index];
        }
        let mut fill = ArenaVec::with_capacity_in(n, scratch);
        fill.extend(pred_starts[..n].iter().copied());
        let mut preds = ArenaVec::with_capacity_in(pred_starts[n] as usize, scratch);
        preds.resize(pred_starts[n] as usize, 0_u32);
        for &node in &order {
            for &target in graph.edges(node as usize) {
                preds[fill[target as usize] as usize] = node;
                fill[target as usize] += 1;
            }
        }
        // Cooper, Harvey and Kennedy, "A Simple, Fast Dominance Algorithm".
        let intersect = |idom: &ArenaVec<'_, u32>, mut a: u32, mut b: u32| {
            while a != b {
                while rpo[a as usize] > rpo[b as usize] {
                    a = idom[a as usize];
                }
                while rpo[b as usize] > rpo[a as usize] {
                    b = idom[b as usize];
                }
            }
            a
        };
        let mut changed = true;
        while changed {
            changed = false;
            for &node in order.iter().skip(1) {
                let mut new = u32::MAX;
                for &pred in &preds
                    [pred_starts[node as usize] as usize..pred_starts[node as usize + 1] as usize]
                {
                    if idom[pred as usize] == u32::MAX {
                        continue;
                    }
                    new = if new == u32::MAX {
                        pred
                    } else {
                        intersect(&idom, pred, new)
                    };
                }
                if idom[node as usize] != new {
                    idom[node as usize] = new;
                    changed = true;
                }
            }
        }
        let dominates = |a: u32, mut b: u32| loop {
            if a == b {
                return true;
            }
            if b == 0 {
                return false;
            }
            b = idom[b as usize];
        };
        let mut loop_header = ArenaVec::with_capacity_in(n, scratch);
        loop_header.resize(n, false);
        let mut forward = ArenaVec::with_capacity_in(n, scratch);
        forward.resize(n, 0_u32);
        let mut irreducible = false;
        for &node in &order {
            for &target in graph.edges(node as usize) {
                if rpo[target as usize] <= rpo[node as usize] {
                    loop_header[target as usize] = true;
                    irreducible |= !dominates(target, node);
                } else {
                    forward[target as usize] += 1;
                }
            }
        }
        let mut merge = ArenaVec::with_capacity_in(n, scratch);
        merge.extend(forward.iter().map(|&count| count >= 2));
        // Merge children by dominator, latest in reverse post-order first.
        let mut child_starts = ArenaVec::with_capacity_in(n + 1, scratch);
        child_starts.resize(n + 1, 0_u32);
        for &node in order.iter().skip(1) {
            if merge[node as usize] {
                child_starts[idom[node as usize] as usize + 1] += 1;
            }
        }
        for index in 0..n {
            child_starts[index + 1] += child_starts[index];
        }
        let mut fill = ArenaVec::with_capacity_in(n, scratch);
        fill.extend(child_starts[..n].iter().copied());
        let mut children = ArenaVec::with_capacity_in(child_starts[n] as usize, scratch);
        children.resize(child_starts[n] as usize, 0_u32);
        for &node in order.iter().skip(1).rev() {
            if merge[node as usize] {
                let parent = idom[node as usize] as usize;
                children[fill[parent] as usize] = node;
                fill[parent] += 1;
            }
        }
        Self {
            rpo,
            order,
            loop_header,
            merge,
            child_starts,
            children,
            irreducible,
        }
    }

    fn merge_children(&self, node: u32) -> &[u32] {
        &self.children[self.child_starts[node as usize] as usize
            ..self.child_starts[node as usize + 1] as usize]
    }
}
