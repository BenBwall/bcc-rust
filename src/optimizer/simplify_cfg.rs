//! Control-flow graph simplification.
//!
//! One round runs seven steps in order, and the pass repeats rounds until
//! nothing changes, because each step exposes work for the others:
//!
//! 1. **Thread jumps** through forwarding blocks: a block with no instructions
//!    and a `jump` terminator. An edge into one is redirected to the block it
//!    forwards to, with the forwarding block's parameters replaced by the
//!    edge's arguments. The arguments the forwarding block passes on may be its
//!    own parameters or values defined above it; the latter dominate the
//!    forwarding block and so every predecessor. A forwarding block whose
//!    parameters are used anywhere but in its own `jump` is not skipped: the
//!    blocks it dominates may use them, and a threaded edge would reach those
//!    blocks without defining them. An edge is left alone when redirecting it
//!    would give one terminator two edges to a block with different arguments,
//!    which the verifier forbids, and so is every edge of an `indirect_br`,
//!    whose targets are the addressed blocks themselves; a labeled block is
//!    never skipped, since its address names it. Block-indexed visit stamps
//!    detect cycles; a per-round cache composes argument mappings backwards
//!    along each path. Each forwarding block is resolved once, independently of
//!    block layout or incoming edge count.
//! 2. **Thread constant conditions** (`thread.rs`): an edge passing a constant
//!    to an empty block that branches on that parameter goes straight to the
//!    chosen successor.
//! 3. **Form selects** for single-entry cheap diamond/triangle arms using
//!    `select.rs`. Each differing join argument is one bisectable rewrite. Each
//!    arm costs at most four units; loads, stores, calls, division and
//!    remainder never speculate. Moved arithmetic loses poison-producing flags.
//! 4. **Merge conditions** (`thread.rs`): a branch whose single-entry,
//!    unlabeled arm only tests a second, cheap condition for one of the
//!    branch's own destinations tests both with one `select`.
//! 5. **Fold identical branches**: a `brif` whose two edges agree, or a
//!    `switch` whose edges all agree, becomes a `jump`.
//! 6. **Remove unreachable blocks**, all at once and counted as one rewrite: a
//!    block removed on its own could leave a surviving unreachable block using
//!    its values. An unreachable labeled block stays as an `unreachable` stub
//!    ([`Draft::retire_block`]), so its address remains valid.
//! 7. **Merge a block into its only predecessor** when that predecessor ends in
//!    a `jump` to it and the edge is the only one into the block, unless a
//!    label names the block. The block's parameters are replaced by the jump's
//!    arguments and its instructions and terminator are appended to the
//!    predecessor.
//!
//! Folding a constant branch is the fold pass's job, not this one's; it
//! leaves the dead arm unreachable for step 6.
//!
//! C99: §5.1.2.3 paragraph 5, pp. 13-14; PDF pp. 25-26 (only the program's
//! observable behaviour must be kept, not its control-flow shape).

mod select;
mod thread;

use super::{
    draft::{
        Draft,
        Edge,
        Term,
    },
    report::OptimizationReport,
};
use crate::{
    ir::{
        Block,
        Entity,
        Value,
    },
    util::bump::{
        ArenaMap,
        ArenaVec,
    },
};

/// The most rounds one run takes. Each round that changes anything removes a
/// block, an edge or a forwarding block, so real functions stop far sooner.
const MAX_ROUNDS: usize = 16;

/// Simplifies the control flow of one function. Returns whether it changed
/// anything.
pub(super) fn run(draft: &mut Draft<'_, '_>, report: &mut OptimizationReport) -> bool {
    let mut changed = false;
    for _ in 0..MAX_ROUNDS {
        let mut round = thread_jumps(draft, report);
        round |= thread::thread_constant_branches(draft, report);
        round |= select::run(draft, report);
        round |= thread::merge_branch_conditions(draft, report);
        round |= fold_identical_branches(draft, report);
        round |= remove_unreachable_blocks(draft, report);
        round |= merge_into_predecessors(draft, report);
        changed |= round;
        if !round || report.limit_reached() {
            break;
        }
    }
    changed
}

/// Step 1: redirects edges around forwarding blocks, indexing the current
/// terminator destinations so duplicate-edge argument checks stay linear.
/// C99: ?5.1.2.3p5, pp. 13-14; PDF pp. 25-26 (preserve observable behaviour).
fn thread_jumps(draft: &mut Draft<'_, '_>, report: &mut OptimizationReport) -> bool {
    let mut changed = false;
    let reachable = draft.reachable();
    // Threading only redirects edges, so it never adds a use of a forwarding
    // block's parameter outside that block; counts taken now stay safe.
    let uses = draft.use_counts();
    let targets = forwarding_targets(draft, &uses, &reachable);
    for index in 0..draft.blocks.len() {
        let block = Block::new(index);
        if !draft.block(block).alive
            || !reachable[index]
            || matches!(draft.block(block).term, Term::IndirectBr { .. })
        {
            continue;
        }
        // Own the terminator while rewriting it so edge traversal stays linear:
        // repeated `nth` calls would themselves rescan a wide switch's prefix.
        let mut term = std::mem::replace(&mut draft.block_mut(block).term, Term::Unreachable);
        let mut destinations = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, draft.scratch);
        for edge in term.edges() {
            let entry = destinations.entry(edge.target).or_insert_with(|| {
                let mut args = ArenaVec::with_capacity_in(edge.args.len(), draft.scratch);
                args.extend(edge.args.iter().map(|&arg| draft.resolve(arg)));
                (0_usize, args)
            });
            entry.0 += 1;
        }
        for edge in term.edges_mut() {
            let Some((target, args)) = threaded(draft, &targets, edge) else {
                continue;
            };
            if destinations
                .get(&target)
                .is_some_and(|(_, canonical)| *canonical != args)
            {
                continue;
            }
            if !report.allow() {
                break;
            }
            if let Some((count, _)) = destinations.get_mut(&edge.target) {
                *count -= 1;
                if *count == 0 {
                    drop(destinations.remove(&edge.target));
                }
            }
            let entry = destinations
                .entry(target)
                .or_insert_with(|| (0, args.clone()));
            entry.0 += 1;
            edge.target = target;
            edge.args = args;
            changed = true;
        }
        draft.block_mut(block).term = term;
        if report.limit_reached() {
            return changed;
        }
    }
    changed
}

/// Resolves forwarding paths without mutating the draft, so bisecting still
/// applies one edge rewrite at a time. Cached arguments are expressed in the
/// starting block's parameters or values defined above it. A cycle, and any
/// path entering it, stays unchanged.
/// C99: §5.1.2.3p5, pp. 13-14; PDF pp. 25-26 (preserve observable behaviour).
fn forwarding_targets<'s>(
    draft: &Draft<'_, 's>,
    uses: &[u32],
    reachable: &[bool],
) -> ArenaVec<'s, ForwardingTarget<'s>> {
    let mut targets = ArenaVec::with_capacity_in(draft.blocks.len(), draft.scratch);
    targets.resize(draft.blocks.len(), ForwardingTarget::Unknown);
    let mut visits = ArenaVec::with_capacity_in(draft.blocks.len(), draft.scratch);
    visits.resize(draft.blocks.len(), 0_usize);
    let mut path = ArenaVec::new_in(draft.scratch);
    for index in 0..draft.blocks.len() {
        if !reachable[index] || !matches!(targets[index], ForwardingTarget::Unknown) {
            continue;
        }
        path.clear();
        let stamp = index + 1;
        let mut target = Block::new(index);
        while matches!(targets[target.index()], ForwardingTarget::Unknown) {
            if visits[target.index()] == stamp {
                targets[target.index()] = ForwardingTarget::Cycle;
                break;
            }
            visits[target.index()] = stamp;
            let forwarder = draft.block(target);
            let Term::Jump(next) = &forwarder.term else {
                targets[target.index()] = ForwardingTarget::Stop;
                break;
            };
            let own_uses = |param: Value| {
                next.args
                    .iter()
                    .filter(|&&arg| draft.resolve(arg) == param)
                    .count()
            };
            if !forwarder.alive
                || target == Draft::entry()
                || draft.is_labeled(target)
                || !forwarder.insts.is_empty()
                || forwarder
                    .params
                    .iter()
                    .any(|&param| uses[param.index()] as usize != own_uses(param))
            {
                targets[target.index()] = ForwardingTarget::Stop;
                break;
            }
            path.push((target, next));
            target = next.target;
        }
        // Compress the whole path, including parameter substitutions. Each
        // cached mapping is reused by every later path and incoming edge.
        for &(block, next) in path.iter().rev() {
            let resolved = match &targets[next.target.index()] {
                | ForwardingTarget::Stop => {
                    let mut args = ArenaVec::with_capacity_in(next.args.len(), draft.scratch);
                    args.extend(next.args.iter().map(|&arg| draft.resolve(arg)));
                    ForwardingTarget::Resolved(Edge {
                        target: next.target,
                        args,
                    })
                },
                | ForwardingTarget::Resolved(destination) => {
                    let params = &draft.block(next.target).params;
                    let mut args =
                        ArenaVec::with_capacity_in(destination.args.len(), draft.scratch);
                    for &arg in &destination.args {
                        args.push(
                            params
                                .iter()
                                .position(|&param| param == arg)
                                .map_or(arg, |position| draft.resolve(next.args[position])),
                        );
                    }
                    ForwardingTarget::Resolved(Edge {
                        target: destination.target,
                        args,
                    })
                },
                | ForwardingTarget::Cycle | ForwardingTarget::Unknown => ForwardingTarget::Cycle,
            };
            targets[block.index()] = resolved;
        }
    }
    targets
}

/// One block's cached forwarding destination and argument mapping.
#[derive(Clone)]
enum ForwardingTarget<'s> {
    Unknown,
    Stop,
    Cycle,
    Resolved(Edge<'s>),
}

/// Where `edge` would lead if it skipped every
/// forwarding block on its way, with the arguments it would pass. `None` if
/// it would not move. The caller checks argument agreement through its
/// destination index. `targets` caches the round's forwarding
/// destinations and parameter substitutions.
fn threaded<'s>(
    draft: &Draft<'_, 's>,
    targets: &[ForwardingTarget<'s>],
    edge: &Edge<'s>,
) -> Option<(Block, ArenaVec<'s, Value>)> {
    let ForwardingTarget::Resolved(destination) = &targets[edge.target.index()] else {
        return None;
    };
    let target = destination.target;
    if target == edge.target {
        return None;
    }
    let params = &draft.block(edge.target).params;
    let mut args = ArenaVec::with_capacity_in(destination.args.len(), draft.scratch);
    for &arg in &destination.args {
        args.push(
            params
                .iter()
                .position(|&param| param == arg)
                .map_or(arg, |position| draft.resolve(edge.args[position])),
        );
    }
    Some((target, args))
}

/// Step 5: turns branches whose edges all agree into jumps.
fn fold_identical_branches(draft: &mut Draft<'_, '_>, report: &mut OptimizationReport) -> bool {
    let mut changed = false;
    for index in 0..draft.blocks.len() {
        let block = Block::new(index);
        if !draft.block(block).alive {
            continue;
        }
        let only: Option<Edge<'_>> = match &draft.block(block).term {
            | Term::Brif {
                then, otherwise, ..
            } if draft.same_edge(then, otherwise) => Some(then.clone()),
            | Term::Switch { default, cases, .. }
                if cases.iter().all(|(_, case)| draft.same_edge(default, case)) =>
                Some(default.clone()),
            | _ => None,
        };
        let Some(only) = only else {
            continue;
        };
        if !report.allow() {
            return changed;
        }
        draft.block_mut(block).term = Term::Jump(only);
        changed = true;
    }
    changed
}

/// Step 6: drops every block the entry cannot reach, as one rewrite. A
/// labeled block becomes an `unreachable` stub instead; one that already is
/// one is not a change.
fn remove_unreachable_blocks(draft: &mut Draft<'_, '_>, report: &mut OptimizationReport) -> bool {
    let reachable = draft.reachable();
    let unreachable = |draft: &Draft<'_, '_>, index: usize| {
        let data = &draft.blocks[index];
        data.alive
            && !reachable[index]
            && !(draft.is_labeled(Block::new(index))
                && data.insts.is_empty()
                && matches!(data.term, Term::Unreachable))
    };
    if !(0..draft.blocks.len()).any(|index| unreachable(draft, index)) || !report.allow() {
        return false;
    }
    for index in 0..draft.blocks.len() {
        if unreachable(draft, index) {
            _ = draft.retire_block(Block::new(index));
        }
    }
    true
}

/// Step 7: appends a block to its only predecessor.
fn merge_into_predecessors(draft: &mut Draft<'_, '_>, report: &mut OptimizationReport) -> bool {
    let mut changed = false;
    let incoming = draft.incoming_edge_counts();
    for index in 0..draft.blocks.len() {
        let block = Block::new(index);
        // Merging a block moves its terminator up, so the same block may
        // absorb a whole chain.
        while draft.block(block).alive
            && let Term::Jump(edge) = &draft.block(block).term
            && edge.target != block
            && edge.target != Draft::entry()
            && !draft.is_labeled(edge.target)
            && draft.block(edge.target).alive
            && incoming[edge.target.index()] == 1
        {
            if !report.allow() {
                return changed;
            }
            let successor = edge.target;
            let mut args = ArenaVec::with_capacity_in(edge.args.len(), draft.scratch);
            args.extend(edge.args.iter().map(|&arg| draft.resolve(arg)));
            let scratch = draft.scratch;
            let params = std::mem::replace(
                &mut draft.block_mut(successor).params,
                ArenaVec::new_in(scratch),
            );
            let insts = std::mem::replace(
                &mut draft.block_mut(successor).insts,
                ArenaVec::new_in(scratch),
            );
            let term = std::mem::replace(&mut draft.block_mut(successor).term, Term::Unreachable);
            draft.block_mut(successor).alive = false;
            for (&param, &arg) in params.iter().zip(&args) {
                draft.replace(param, arg);
            }
            let merged = draft.block_mut(block);
            merged.insts.extend_from_slice(&insts);
            merged.term = term;
            changed = true;
        }
    }
    changed
}
