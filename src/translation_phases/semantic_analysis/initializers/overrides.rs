//! Which subobjects one initializer has written, so that a later initializer
//! for the same subobject is reported as Clang's `-Winitializer-overrides`
//! reports it.
//!
//! Each written subobject is a node under its containing subobject, reached
//! by its member or element index. A write by a brace-enclosed list or an
//! expression replaces its node, which forgets everything written inside
//! it. A write inside a subobject an expression initialized as a whole
//! overrides part of that expression, which keeps covering the rest, so
//! every later write inside it does too. Selecting another member of a union
//! replaces the member written before (§6.7.8p17), and only a union's
//! last-selected member stays recorded. The elements of a GNU range
//! designator share one node, so that writes inside the range's
//! initializer belong to each element; a later write through one element
//! gives that element its own copy first.
//!
//! For `{ .p = { 1, 2 }, .p.x = 3 }` the first element writes the node for
//! `p` as a list, inside which `1` and `2` write `x` and `y`; `.p.x = 3`
//! then reaches the written `x` and overrides it. For
//! `{ .p = q, .p.x = 3, .p.y = 4 }` both later elements pass through `p`,
//! which the expression `q` wrote, and override part of it, as Clang reports
//! at `.x` and `.y`. A subobject that is written only inside, like `p` in
//! `{ .p.x = 1, .p = q }`, refers back to where its writes begin, the
//! designator `.x`, as Clang's implicit list for it does.
//!
//! C99: §6.7.8 paragraph 19, p. 127; PDF p. 139 (each initializer overrides
//! any previously listed initializer for the same subobject); §6.7.8
//! paragraph 17, pp. 126-127; PDF pp. 138-139 (a union's current member).
//! GNU extension: GCC manual, "Designated Initializers" (ranges).
//! <https://gcc.gnu.org/onlinedocs/gcc/Designated-Inits.html>

use crate::{
    translation_phases::SourceVectors,
    util::bump::{
        ArenaMap,
        ArenaVec,
        Bump,
    },
};

/// The subobjects an initializer has written, from the object's own node
/// ([`Writes::ROOT`]).
pub(super) struct Writes<'s> {
    scratch:  &'s Bump,
    nodes:    ArenaVec<'s, Node>,
    /// The node of each written subobject by its container's node and its
    /// index there.
    children: ArenaMap<'s, (u32, u64), u32>,
    /// Every `children` entry ever made, linked per container from
    /// [`Node::edges`], so that a shared node can be copied with what is
    /// written inside it; an edge `children` no longer holds is stale.
    edges:    ArenaVec<'s, Edge>,
}

impl<'s> Writes<'s> {
    /// The node of the object being initialized.
    pub(super) const ROOT: u32 = 0;

    /// Records that the subobject `steps` lead to from the node `base` is
    /// written by `kind` at `source`. With `range`, the step it names holds
    /// a range designator's first index and the step's own index its last,
    /// and every element of the range is written. Returns the written node,
    /// to which the elements of a brace-enclosed list belong, and the
    /// earlier writes it overrides.
    pub(super) fn write(
        &mut self,
        base: u32,
        steps: &[Step],
        range: Option<(usize, u64)>,
        kind: Write,
        source: SourceVectors,
    ) -> (u32, Overrides) {
        let mut found = Overrides::default();
        let Some(last) = steps.len().checked_sub(1) else {
            return (base, found);
        };
        let state = match kind {
            | Write::List => State::Written,
            | Write::Expression => State::Expression,
        };
        let written = self.node(state, source);
        let (at, lower) = range.unwrap_or((last, steps[last].index));
        let mut node = (base, source);
        for &step in &steps[..at] {
            node = self.enter(node, step, &mut found);
        }
        let range_step = steps[at];
        for index in lower..=range_step.index {
            let step = Step {
                index,
                ..range_step
            };
            let mut parent = node;
            if at < last {
                parent = self.enter(parent, step, &mut found);
                for &step in &steps[at + 1..last] {
                    parent = self.enter(parent, step, &mut found);
                }
                self.place(parent, steps[last], written, &mut found);
            } else {
                self.place(parent, step, written, &mut found);
            }
        }
        // One node written for several elements stands for each of them.
        if lower < range_step.index {
            self.nodes[written as usize].shared = true;
        }
        (written, found)
    }

    /// Enters subobject `step` of `node`, whose writes begin at `at`:
    /// passing through a subobject an expression wrote overrides part of
    /// it, and a shared node is first copied for this element. Creates the
    /// node, unwritten, if there is none. Every node entered becomes written
    /// in part.
    fn enter(
        &mut self,
        (node, at): (u32, SourceVectors),
        step: Step,
        found: &mut Overrides,
    ) -> (u32, SourceVectors) {
        self.pass(node, at, found);
        self.select(node, step, found);
        let child = match self.children.get(&(node, step.index)).copied() {
            | Some(child) if self.nodes[child as usize].shared => {
                let copy = self.copy(child);
                self.link(node, step.index, copy);
                copy
            },
            | Some(child) => child,
            | None => {
                let child = self.node(State::Unwritten, step.inner);
                self.link(node, step.index, child);
                child
            },
        };
        let data = &mut self.nodes[child as usize];
        if data.state == State::Unwritten {
            data.state = State::Written;
        }
        (child, step.inner)
    }

    /// Makes `written` subobject `step` of `parent`, replacing what was
    /// written there before.
    fn place(
        &mut self,
        (parent, at): (u32, SourceVectors),
        step: Step,
        written: u32,
        found: &mut Overrides,
    ) {
        self.pass(parent, at, found);
        self.select(parent, step, found);
        if let Some(&previous) = self.children.get(&(parent, step.index)) {
            let previous = self.nodes[previous as usize];
            if found.whole.is_none() && previous.state != State::Unwritten {
                found.whole = Some(previous.source);
            }
        }
        self.link(parent, step.index, written);
    }

    /// A write inside `node`, beginning at `at`, overrides part of an
    /// expression that wrote it.
    fn pass(&mut self, node: u32, at: SourceVectors, found: &mut Overrides) {
        let data = self.nodes[node as usize];
        if data.state == State::Expression && found.partial.is_none() {
            found.partial = Some((at, data.source));
        }
    }

    /// Selects member `step` of `node` if it is a union: a member other than
    /// the one written before overrides and forgets that one.
    fn select(&mut self, node: u32, step: Step, found: &mut Overrides) {
        if !step.union {
            return;
        }
        let previous = self.nodes[node as usize].member.replace(step.index);
        if let Some(previous) = previous.filter(|&previous| previous != step.index)
            && let Some(old) = self.children.remove(&(node, previous))
        {
            let old = self.nodes[old as usize];
            if found.member.is_none() && old.state != State::Unwritten {
                found.member = Some(old.source);
            }
        }
    }

    /// A copy of the shared node `node` and of everything written inside
    /// it, for one of the elements that share it.
    fn copy(&mut self, node: u32) -> u32 {
        let mut copies = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, self.scratch);
        let mut work = ArenaVec::new_in(self.scratch);
        let root = self.duplicate(node, false);
        work.push((node, root));
        while let Some((old, new)) = work.pop() {
            let mut edge = self.nodes[old as usize].edges;
            while let Some(index) = edge {
                let Edge { index, child, next } = self.edges[index as usize];
                edge = next;
                if self.children.get(&(old, index)) != Some(&child) {
                    continue;
                }
                let copy = if let Some(&copy) = copies.get(&child) {
                    copy
                } else {
                    let shared = self.nodes[child as usize].shared;
                    let copy = self.duplicate(child, shared);
                    _ = copies.insert(child, copy);
                    work.push((child, copy));
                    copy
                };
                self.link(new, index, copy);
            }
        }
        root
    }

    /// A node with the state of `node` and nothing written inside.
    fn duplicate(&mut self, node: u32, shared: bool) -> u32 {
        let data = self.nodes[node as usize];
        let copy = self.node(data.state, data.source);
        let copied = &mut self.nodes[copy as usize];
        copied.member = data.member;
        copied.shared = shared;
        copy
    }

    /// Makes `child` subobject `index` of `parent`.
    fn link(&mut self, parent: u32, index: u64, child: u32) {
        _ = self.children.insert((parent, index), child);
        let edge = u32::try_from(self.edges.len()).unwrap_or(u32::MAX);
        self.edges.push(Edge {
            index,
            child,
            next: self.nodes[parent as usize].edges,
        });
        self.nodes[parent as usize].edges = Some(edge);
    }

    fn node(&mut self, state: State, source: SourceVectors) -> u32 {
        let index = u32::try_from(self.nodes.len()).unwrap_or(u32::MAX);
        self.nodes.push(Node {
            state,
            source,
            member: None,
            shared: false,
            edges: None,
        });
        index
    }

    pub(super) fn new_in(scratch: &'s Bump, source: SourceVectors) -> Self {
        let mut writes = Self {
            scratch,
            nodes: ArenaVec::new_in(scratch),
            children: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            edges: ArenaVec::new_in(scratch),
        };
        _ = writes.node(State::Written, source);
        writes
    }
}

/// One step from a subobject to a member or element of it.
#[derive(Clone, Copy, Debug)]
pub(super) struct Step {
    pub(super) index: u64,
    /// Whether the step selects a member of a union.
    pub(super) union: bool,
    /// Where writes inside the subobject begin: the designator after the
    /// one that names it, or the element brace elision descends with.
    pub(super) inner: SourceVectors,
}

/// How an element writes its subobject.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Write {
    /// By a brace-enclosed list, whose elements write inside it.
    List,
    /// By an expression, including a string literal or a structure value
    /// that initializes the whole subobject.
    Expression,
}

/// The earlier writes a new one overrides, each with that write's source.
/// Clang reports each kind once per element.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(super) struct Overrides {
    /// A subobject written before, which the new write replaces.
    pub(super) whole:   Option<SourceVectors>,
    /// Where the new write enters a subobject an expression wrote as a
    /// whole, and that expression.
    pub(super) partial: Option<(SourceVectors, SourceVectors)>,
    /// The member of a union selected before, which the new element's
    /// designation replaces.
    pub(super) member:  Option<SourceVectors>,
}

#[derive(Clone, Copy)]
struct Node {
    state:  State,
    /// The write that gave the node its state, or where writes inside it
    /// begin.
    source: SourceVectors,
    /// For a union, the member last selected.
    member: Option<u64>,
    /// Whether several range elements share the node.
    shared: bool,
    /// The latest of the node's edges.
    edges:  Option<u32>,
}

/// One entry of [`Writes::children`]: subobject `index` of the edge list's
/// node is `child`.
#[derive(Clone, Copy)]
struct Edge {
    index: u64,
    child: u32,
    next:  Option<u32>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Unwritten,
    /// Written by a list or inside.
    Written,
    /// Written as a whole by an expression.
    Expression,
}
