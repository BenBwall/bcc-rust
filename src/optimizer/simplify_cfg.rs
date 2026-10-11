//! Control-flow graph simplification.
//!
//! One round runs four steps in order, and the pass repeats rounds until
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
//!    which the verifier forbids.
//! 2. **Fold identical branches**: a `brif` whose two edges agree, or a
//!    `switch` whose edges all agree, becomes a `jump`.
//! 3. **Remove unreachable blocks**, all at once and counted as one rewrite: a
//!    block removed on its own could leave a surviving unreachable block using
//!    its values.
//! 4. **Merge a block into its only predecessor** when that predecessor ends in
//!    a `jump` to it and the edge is the only one into the block. The block's
//!    parameters are replaced by the jump's arguments and its instructions and
//!    terminator are appended to the predecessor.
//!
//! Folding a constant branch is the fold pass's job, not this one's; it
//! leaves the dead arm unreachable for step 3.
//!
//! C99: §5.1.2.3 paragraph 5, pp. 13-14; PDF pp. 25-26 (only the program's
//! observable behaviour must be kept, not its control-flow shape).

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
    util::bump::ArenaVec,
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

/// Step 1: redirects edges around forwarding blocks.
fn thread_jumps(draft: &mut Draft<'_, '_>, report: &mut OptimizationReport) -> bool {
    let mut changed = false;
    let reachable = draft.reachable();
    // Threading only redirects edges, so it never adds a use of a forwarding
    // block's parameter outside that block; counts taken now stay safe.
    let uses = draft.use_counts();
    for index in 0..draft.blocks.len() {
        let block = Block::new(index);
        if !draft.block(block).alive || !reachable[index] {
            continue;
        }
        for edge_index in 0..draft.block(block).term.edges().count() {
            let Some((target, args)) = threaded(draft, &uses, block, edge_index) else {
                continue;
            };
            if !report.allow() {
                return changed;
            }
            let edge = draft
                .block_mut(block)
                .term
                .edges_mut()
                .nth(edge_index)
                .expect("the edge was just read");
            edge.target = target;
            edge.args = args;
            changed = true;
        }
    }
    changed
}

/// Where the `edge_index`th edge of `block` would lead if it skipped every
/// forwarding block on its way, with the arguments it would pass. `None` if
/// it would not move, or if moving would give the terminator two edges to a
/// block with different arguments. `uses` are the draft's use counts.
fn threaded<'s>(
    draft: &Draft<'_, 's>,
    uses: &[u32],
    block: Block,
    edge_index: usize,
) -> Option<(Block, ArenaVec<'s, Value>)> {
    let term = &draft.block(block).term;
    let edge = term.edges().nth(edge_index)?;
    let mut target = edge.target;
    let mut args = ArenaVec::with_capacity_in(edge.args.len(), draft.scratch);
    args.extend(edge.args.iter().map(|&arg| draft.resolve(arg)));
    // The blocks passed through. A cycle of forwarding blocks is an empty
    // infinite loop; threading around it would only rotate its entry, so the
    // edge stays.
    let mut visited = ArenaVec::new_in(draft.scratch);
    visited.push(target);
    loop {
        let forwarder = draft.block(target);
        let Term::Jump(next) = &forwarder.term else {
            break;
        };
        if !forwarder.alive || target == Draft::entry() || !forwarder.insts.is_empty() {
            break;
        }
        if visited.contains(&next.target) {
            return None;
        }
        let own_uses = |param: Value| {
            next.args
                .iter()
                .filter(|&&arg| draft.resolve(arg) == param)
                .count()
        };
        if forwarder
            .params
            .iter()
            .any(|&param| uses[param.index()] as usize != own_uses(param))
        {
            break;
        }
        let mut next_args = ArenaVec::with_capacity_in(next.args.len(), draft.scratch);
        for &arg in &next.args {
            let arg = draft.resolve(arg);
            next_args.push(
                forwarder
                    .params
                    .iter()
                    .position(|&param| param == arg)
                    .map_or(arg, |position| args[position]),
            );
        }
        visited.push(next.target);
        target = next.target;
        args = next_args;
    }
    if target == edge.target {
        return None;
    }
    let agrees = term.edges().enumerate().all(|(other_index, other)| {
        other_index == edge_index
            || other.target != target
            || (other.args.len() == args.len()
                && other
                    .args
                    .iter()
                    .zip(&args)
                    .all(|(&x, &y)| draft.resolve(x) == y))
    });
    agrees.then_some((target, args))
}

/// Step 2: turns branches whose edges all agree into jumps.
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

/// Step 3: drops every block the entry cannot reach, as one rewrite.
fn remove_unreachable_blocks(draft: &mut Draft<'_, '_>, report: &mut OptimizationReport) -> bool {
    let reachable = draft.reachable();
    let unreachable =
        |draft: &Draft<'_, '_>, index: usize| draft.blocks[index].alive && !reachable[index];
    if !(0..draft.blocks.len()).any(|index| unreachable(draft, index)) || !report.allow() {
        return false;
    }
    for index in 0..draft.blocks.len() {
        if unreachable(draft, index) {
            draft.blocks[index].alive = false;
        }
    }
    true
}

/// Step 4: appends a block to its only predecessor.
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
