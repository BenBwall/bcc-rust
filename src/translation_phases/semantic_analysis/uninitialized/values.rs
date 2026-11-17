//! The values Clang's `runUninitializedVariablesAnalysis` computes over the
//! graph, and what it says of each use: whether a variable is initialized at
//! the end of each block, and for a use that is not always initialized, the
//! branches after which it is not (`TransferFunctions::getUninitUse`).
//! C99: §6.2.4p5, p. 32; PDF p. 44; §6.7.8p10, p. 126; PDF p. 138 (an
//! automatic object without an initializer has an indeterminate value).

use super::cfg::{
    BlockId,
    Cfg,
    Class,
    Element,
    Label,
    Terminator,
};
use crate::{
    translation_phases::parsing::syntax::Expression,
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// Two bits per variable, which merge by `|` (Clang's `Value`).
pub(super) const INITIALIZED: u64 = 0b01;
pub(super) const UNINITIALIZED: u64 = 0b10;
pub(super) const MAY_UNINITIALIZED: u64 = 0b11;

/// Every variable's value at the end of every block.
pub(super) struct Values<'b> {
    words:   usize,
    vars:    usize,
    exits:   &'b mut [u64],
    /// Whether the analysis reached the block (`wasAnalyzed`).
    reached: &'b mut [bool],
}

/// How sure the analysis is that a use reads an uninitialized value, from
/// least to most (`UninitUse::Kind`).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(super) enum Kind {
    Maybe,
    Sometimes,
    AfterDecl,
    AfterCall,
    Always,
}

/// A branch after which a use is uninitialized: the block that ends in it
/// and the successor it takes.
#[derive(Clone, Copy)]
pub(super) struct Branch {
    pub(super) block:     BlockId,
    pub(super) output:    u32,
    pub(super) successor: BlockId,
}

/// A use of an uninitialized variable (`UninitUse`).
#[derive(Clone, Copy)]
pub(super) struct Use<'tu> {
    pub(super) user:      &'tu Expression<'tu>,
    pub(super) kind:      Kind,
    /// The use passes the variable's address as a pointer to `const`.
    pub(super) const_ptr: bool,
    /// Its branches, in the branch list.
    pub(super) branches:  (u32, u32),
}

/// What the final pass found, by variable in the order each was first met.
pub(super) struct Uses<'b, 'tu> {
    pub(super) order:     ArenaVec<'b, u32>,
    /// Each use and its variable, in the order the pass met them.
    pub(super) uses:      ArenaVec<'b, (u32, Use<'tu>)>,
    pub(super) self_init: &'b mut [bool],
    pub(super) branches:  ArenaVec<'b, Branch>,
}

impl Values<'_> {
    pub(super) fn get(&self, block: BlockId, var: u32) -> u64 {
        let word = self.exits[block as usize * self.words + var as usize / 32];
        (word >> (var % 32 * 2)) & 0b11
    }
}

fn get(vector: &[u64], var: u32) -> u64 {
    (vector[var as usize / 32] >> (var % 32 * 2)) & 0b11
}

fn set(vector: &mut [u64], var: u32, value: u64) {
    let word = &mut vector[var as usize / 32];
    let shift = var % 32 * 2;
    *word = (*word & !(0b11 << shift)) | (value << shift);
}

/// Every variable set to `value`.
fn fill(vector: &mut [u64], vars: usize, value: u64) {
    let pattern = (0..32).fold(0_u64, |word, index| word | (value << (index * 2)));
    for (index, word) in vector.iter_mut().enumerate() {
        let count = vars.saturating_sub(index * 32).min(32);
        let mask = if count == 32 {
            u64::MAX
        } else {
            (1_u64 << (count * 2)) - 1
        };
        *word = pattern & mask;
    }
}

/// The analysis to its fixed point, as Clang's worklist computes it: a
/// block merges the values its reached predecessors end with and applies
/// its elements; the entry leaves every variable uninitialized.
pub(super) fn solve<'b>(cfg: &Cfg<'_, '_>, arena: &'b Bump) -> Values<'b> {
    let blocks = cfg.blocks.len();
    let vars = cfg.vars.len();
    let words = vars.div_ceil(32).max(1);
    let exits = arena.alloc_slice_fill_iter(std::iter::repeat_n(0_u64, blocks * words));
    let reached = arena.alloc_slice_fill_iter(std::iter::repeat_n(false, blocks));
    let values = Values {
        words,
        vars,
        exits,
        reached,
    };
    let entry = cfg.entry as usize;
    fill(
        &mut values.exits[entry * words..(entry + 1) * words],
        vars,
        UNINITIALIZED,
    );
    let order = reverse_post_order(cfg, arena);
    for &block in &order {
        values.reached[block as usize] = true;
    }
    let scratch = arena.alloc_slice_fill_iter(std::iter::repeat_n(0_u64, words));
    loop {
        let mut changed = false;
        for &block in order.iter().filter(|&&block| block != cfg.entry) {
            merge(cfg, &values, block, scratch);
            for element in cfg.elements(block) {
                transfer(element, scratch, vars, |_, _, _| {});
            }
            let start = block as usize * words;
            if values.exits[start..start + words] != *scratch {
                values.exits[start..start + words].copy_from_slice(scratch);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    values
}

/// The blocks the entry reaches through edges that are not pruned, in
/// reverse post-order.
fn reverse_post_order<'b>(cfg: &Cfg<'_, '_>, arena: &'b Bump) -> ArenaVec<'b, BlockId> {
    let blocks = cfg.blocks.len();
    let visited = arena.alloc_slice_fill_iter(std::iter::repeat_n(false, blocks));
    let mut post: ArenaVec<'b, BlockId> = ArenaVec::new_in(arena);
    let mut stack: ArenaVec<'b, (BlockId, usize)> = ArenaVec::new_in(arena);
    visited[cfg.entry as usize] = true;
    stack.push((cfg.entry, 0));
    while let Some(&mut (block, ref mut next)) = stack.last_mut() {
        let successors = cfg.successors(block);
        if let Some(&successor) = successors.get(*next) {
            *next += 1;
            if let Some(successor) = successor
                && !visited[successor as usize]
            {
                visited[successor as usize] = true;
                stack.push((successor, 0));
            }
        } else {
            post.push(block);
            _ = stack.pop();
        }
    }
    let mut order = ArenaVec::with_capacity_in(post.len(), arena);
    order.extend(post.iter().rev().copied());
    order
}

/// The values at the start of `block`: the `|` of its predecessors' ends.
fn merge(cfg: &Cfg<'_, '_>, values: &Values<'_>, block: BlockId, scratch: &mut [u64]) {
    scratch.fill(0);
    let words = values.words;
    for &pred in cfg.predecessors(block) {
        let start = pred as usize * words;
        for (word, &incoming) in scratch.iter_mut().zip(&values.exits[start..start + words]) {
            *word |= incoming;
        }
    }
}

/// Applies one element (`TransferFunctions`), calling `report` with each
/// reference that reads and the variable's value before it.
fn transfer(
    element: &Element<'_>,
    vector: &mut [u64],
    vars: usize,
    mut report: impl FnMut(u32, Class, u64),
) {
    match *element {
        | Element::Reference { var, class, .. } => match class {
            | Class::Init => set(vector, var, INITIALIZED),
            | Class::Use | Class::ConstPtrUse | Class::SelfInit =>
                report(var, class, get(vector, var)),
            | Class::Ignore => {},
        },
        | Element::Assign(var) => set(vector, var, INITIALIZED),
        | Element::Declare { var, initialized } => set(
            vector,
            var,
            if initialized {
                INITIALIZED
            } else {
                UNINITIALIZED
            },
        ),
        | Element::ReturnsTwice => fill(vector, vars, INITIALIZED),
    }
}

/// The final pass: runs each reached block in the order the builder
/// created them and records each use of an uninitialized value, and each
/// variable initialized with itself (`handleUseOfUninitVariable`,
/// `handleSelfInit`).
pub(super) fn find_uses<'b, 'tu>(
    cfg: &Cfg<'_, 'tu>,
    values: &Values<'_>,
    arena: &'b Bump,
) -> Uses<'b, 'tu> {
    let blocks = cfg.blocks.len();
    let mut uses = Uses {
        order:     ArenaVec::new_in(arena),
        uses:      ArenaVec::new_in(arena),
        self_init: arena.alloc_slice_fill_iter(std::iter::repeat_n(false, values.vars)),
        branches:  ArenaVec::new_in(arena),
    };
    let seen = arena.alloc_slice_fill_iter(std::iter::repeat_n(false, values.vars));
    let scratch = arena.alloc_slice_fill_iter(std::iter::repeat_n(0_u64, values.words));
    let visited = arena.alloc_slice_fill_iter(std::iter::repeat_n(0_u32, blocks));
    let mut queue: ArenaVec<'_, BlockId> = ArenaVec::new_in(arena);
    for block in 0..blocks {
        let block = BlockId::try_from(block).unwrap_or(BlockId::MAX);
        if !values.reached[block as usize] || block == cfg.entry {
            continue;
        }
        merge(cfg, values, block, scratch);
        for element in cfg.elements(block) {
            let mut found = None;
            transfer(element, scratch, values.vars, |var, class, value| {
                found = Some((var, class, value));
            });
            let Some((var, class, value)) = found else {
                continue;
            };
            let Element::Reference { user, .. } = *element else {
                continue;
            };
            let record = match class {
                | Class::SelfInit => {
                    uses.self_init[var as usize] = true;
                    None
                },
                | Class::Use => (value & UNINITIALIZED != 0).then_some(false),
                | _ => (value == UNINITIALIZED).then_some(true),
            };
            if !seen[var as usize] && (record.is_some() || class == Class::SelfInit) {
                seen[var as usize] = true;
                uses.order.push(var);
            }
            if let Some(const_ptr) = record {
                let start = uses.branches.len();
                let kind = if value == UNINITIALIZED {
                    Kind::Always
                } else {
                    frontier(
                        cfg,
                        values,
                        block,
                        var,
                        visited,
                        &mut queue,
                        &mut uses.branches,
                    )
                };
                let count = uses.branches.len() - start;
                uses.uses.push((
                    var,
                    Use {
                        user,
                        kind,
                        const_ptr,
                        branches: (
                            u32::try_from(start).unwrap_or(u32::MAX),
                            u32::try_from(count).unwrap_or(u32::MAX),
                        ),
                    },
                ));
            }
        }
    }
    uses
}

/// `getUninitUse` for a use in `block` that is not always uninitialized:
/// walks back from the use through the blocks every path from which reaches
/// it without initializing the variable, then reports each edge from a block
/// where the variable is uninitialized into that set as a branch. A walk
/// that reaches the entry makes the use uninitialized whenever the function
/// is called; one that meets the declaration after an initialization,
/// whenever the declaration is reached.
fn frontier(
    cfg: &Cfg<'_, '_>,
    values: &Values<'_>,
    block: BlockId,
    var: u32,
    visited: &mut [u32],
    queue: &mut ArenaVec<'_, BlockId>,
    branches: &mut ArenaVec<'_, Branch>,
) -> Kind {
    let start = branches.len();
    visited.fill(0);
    queue.clear();
    let succ_count =
        |block: BlockId| u32::try_from(cfg.successors(block).len()).unwrap_or(u32::MAX);
    queue.push(block);
    visited[block as usize] = succ_count(block);
    let mut after_call = false;
    let mut after_decl = false;
    while let Some(current) = queue.pop() {
        if current == cfg.entry {
            after_call = true;
        }
        for &pred in cfg.predecessors(current) {
            let at_exit = values.get(pred, var);
            if at_exit == INITIALIZED {
                continue;
            }
            if at_exit == MAY_UNINITIALIZED && values.get(current, var) == UNINITIALIZED {
                after_decl = true;
                continue;
            }
            let count = &mut visited[pred as usize];
            if *count == 0 {
                // Pruned successors count as visited.
                *count +=
                    u32::try_from(cfg.successors(pred).iter().filter(|s| s.is_none()).count())
                        .unwrap_or(0);
            }
            *count += 1;
            if *count == succ_count(pred) {
                queue.push(pred);
            }
        }
    }
    for candidate in 0..cfg.blocks.len() {
        let candidate = BlockId::try_from(candidate).unwrap_or(BlockId::MAX);
        if values.get(candidate, var) != UNINITIALIZED {
            continue;
        }
        let count = visited[candidate as usize];
        if count == 0
            || count >= succ_count(candidate)
            || cfg.blocks[candidate as usize].terminator.is_none()
        {
            continue;
        }
        let switch = matches!(
            cfg.blocks[candidate as usize].terminator,
            Some(Terminator::Switch)
        );
        for (output, &successor) in cfg.successors(candidate).iter().enumerate() {
            if let Some(successor) = successor
                && visited[successor as usize] >= succ_count(successor)
            {
                // A switch reports the label its condition reaches, and not
                // an edge no label may take.
                if switch
                    && !matches!(
                        cfg.blocks[successor as usize].label,
                        Some(Label::Case(_) | Label::Default(_))
                    )
                {
                    continue;
                }
                branches.push(Branch {
                    block: candidate,
                    output: u32::try_from(output).unwrap_or(u32::MAX),
                    successor,
                });
            }
        }
    }
    if after_call {
        Kind::AfterCall
    } else if after_decl {
        Kind::AfterDecl
    } else if branches.len() > start {
        Kind::Sometimes
    } else {
        Kind::Maybe
    }
}
