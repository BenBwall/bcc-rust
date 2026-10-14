//! Branch threading: edges that carry a constant condition skip the block
//! that branches on it, and a branch whose arm only tests a second condition
//! for the same destination merges both tests.
//!
//! **Constant conditions.** A block with no instructions that ends in `brif`
//! on one of its own parameters decides the branch from the argument each
//! incoming edge passes. An edge passing a constant goes straight to the
//! chosen successor instead, with the block's parameters in that successor's
//! arguments replaced by the edge's arguments; any other argument is defined
//! above the block and so dominates every predecessor. `&&` and `||` joins
//! (`block(v: i1): brif v, ...`) reached from a branch that already decided
//! them are the common case. A loop header is left alone, so threading never
//! enters a loop around its header, and so is a block whose parameters
//! blocks below it still use, which a threaded path would not define, and an
//! edge whose new target would
//! get a second, different set of arguments from one terminator. Poison is
//! not a constant here: branching on it stays undefined behaviour.
//!
//! **Merged conditions.** In
//!
//! ```text
//! block0:  brif v0, block1, block3(v5)     block0:  v1 = icmp ...
//! block1:  v1 = icmp ...                            v2 = select.i1 v0, v1, v6
//!          brif v1, block2, block3(v5)              brif v2, block2, block3(v5)
//! ```
//!
//! (`v6` false) the second test only runs when the first holds and fails to
//! the same edge, so both become one `select`, the lowered form of `&&`; the
//! mirror image with the first test's true edge shared becomes `select v0,
//! true, v1`, the form of `||`. The arm's instructions move up, so they must
//! be cheap and safe to speculate (at most [`MERGE_BUDGET`] units by the
//! select-formation cost, no memory, division or call). They keep their
//! flags: where the first test fails, a poison they produce reaches only the
//! `select`'s unchosen operand, and every other use stays behind the same
//! two tests. This is LLVM's `FoldBranchToCommonDest`. With the range-check
//! rule of `combine`, `c >= 'a' && c <= 'z'` becomes one comparison.
//!
//! Each threaded edge and each merge is one rewrite for bisecting.
//!
//! C99: §6.5.13 paragraph 4 and §6.5.14 paragraph 4, pp. 89-90; PDF pp.
//! 101-102 (the second operand of `&&` and `||` is evaluated only when the
//! first does not decide the result, which speculation preserves because the
//! moved instructions have no effects); §5.1.2.3 paragraph 5, pp. 13-14; PDF
//! pp. 25-26.

use super::{
    Draft,
    Edge,
    OptimizationReport,
    Term,
    select::cheap_arm_within,
};
use crate::{
    ir::{
        Block,
        DominatorTree,
        Entity,
        InstData,
        Type,
        Value,
    },
    optimizer::draft::Constant,
    util::bump::ArenaVec,
};

/// The most cost units a merged arm may speculate.
const MERGE_BUDGET: u32 = 2;

/// Threads edges that pass a constant condition past the block that tests
/// it. Returns whether any edge moved.
pub(super) fn thread_constant_branches(
    draft: &mut Draft<'_, '_>,
    report: &mut OptimizationReport,
) -> bool {
    let cfg = draft.control_flow_graph();
    let tree = DominatorTree::from_cfg(&cfg, draft.scratch);
    // Threading only redirects edges, so it adds no use of a parameter
    // outside its block; counts taken now stay safe.
    let uses = draft.use_counts();
    let mut changed = false;
    for index in 1..draft.blocks.len() {
        let block = Block::new(index);
        let data = draft.block(block);
        if !data.alive
            || !tree.is_reachable(block)
            || data.insts.iter().any(|&inst| !draft.is_removed(inst))
        {
            continue;
        }
        let Term::Brif {
            cond,
            then,
            otherwise,
        } = &data.term
        else {
            continue;
        };
        let cond = draft.resolve(*cond);
        let Some(position) = data.params.iter().position(|&param| param == cond) else {
            continue;
        };
        // A parameter used past the block (by a block it dominates) would
        // be undefined on a threaded path.
        let own_uses = |param: Value| {
            usize::from(param == cond)
                + data
                    .term
                    .edges()
                    .flat_map(|edge| &edge.args)
                    .filter(|&&arg| draft.resolve(arg) == param)
                    .count()
        };
        if data
            .params
            .iter()
            .any(|&param| uses[param.index()] as usize != own_uses(param))
        {
            continue;
        }
        let (then, otherwise) = (then.clone(), otherwise.clone());
        if [&then, &otherwise]
            .iter()
            .any(|edge| edge.target == block || edge.target == Draft::entry())
        {
            continue;
        }
        let predecessors = cfg.predecessors(block);
        if predecessors
            .iter()
            .any(|&predecessor| tree.dominates(block, predecessor))
        {
            continue;
        }
        let mut predecessor_list = ArenaVec::new_in(draft.scratch);
        predecessor_list.extend_from_slice(predecessors);
        for predecessor in predecessor_list {
            // A switch keeps its edges: threading one of many cases would need
            // an index of its destinations to stay linear.
            if !matches!(
                draft.block(predecessor).term,
                Term::Jump(_) | Term::Brif { .. }
            ) {
                continue;
            }
            let mut term =
                std::mem::replace(&mut draft.block_mut(predecessor).term, Term::Unreachable);
            for edge_index in 0..2 {
                let Some(edge) = term.edges().nth(edge_index) else {
                    break;
                };
                if edge.target != block {
                    continue;
                }
                let Some(Constant::Int(_, bits)) = draft.constant(edge.args[position]) else {
                    continue;
                };
                let chosen = if bits != 0 { &then } else { &otherwise };
                let params = &draft.block(block).params;
                let mut args = ArenaVec::with_capacity_in(chosen.args.len(), draft.scratch);
                for &arg in &chosen.args {
                    let arg = draft.resolve(arg);
                    args.push(
                        params
                            .iter()
                            .position(|&param| param == arg)
                            .map_or(arg, |i| draft.resolve(edge.args[i])),
                    );
                }
                let candidate = Edge {
                    target: chosen.target,
                    args,
                };
                let conflicts = term.edges().enumerate().any(|(other_index, other)| {
                    other_index != edge_index
                        && other.target == candidate.target
                        && !draft.same_edge(other, &candidate)
                });
                if conflicts || !report.allow() {
                    continue;
                }
                *term.edges_mut().nth(edge_index).expect("counted above") = candidate;
                changed = true;
            }
            draft.block_mut(predecessor).term = term;
            if report.limit_reached() {
                return changed;
            }
        }
    }
    changed
}

/// Merges a branch with an arm that only tests a second condition for one
/// of its destinations. Returns whether it merged any.
pub(super) fn merge_branch_conditions(
    draft: &mut Draft<'_, '_>,
    report: &mut OptimizationReport,
) -> bool {
    let mut changed = false;
    let incoming = draft.incoming_edge_counts();
    for index in 0..draft.blocks.len() {
        let block = Block::new(index);
        if !draft.block(block).alive {
            continue;
        }
        let Term::Brif {
            cond,
            then,
            otherwise,
        } = &draft.block(block).term
        else {
            continue;
        };
        let (first, then, otherwise) = (draft.resolve(*cond), then.clone(), otherwise.clone());
        if then.target == otherwise.target {
            continue;
        }
        // (the arm's edge, the shared edge, whether the arm is the true side)
        for (arm_edge, shared, and) in [(&then, &otherwise, true), (&otherwise, &then, false)] {
            let arm = arm_edge.target;
            if arm == block
                || arm == Draft::entry()
                || !draft.block(arm).alive
                || incoming[arm.index()] != 1
                || !cheap_arm_within(draft, arm, MERGE_BUDGET)
            {
                continue;
            }
            let Term::Brif {
                cond: second,
                then: arm_then,
                otherwise: arm_otherwise,
            } = &draft.block(arm).term
            else {
                continue;
            };
            // The arm's edge to the shared destination, and its other edge.
            let (arm_shared, arm_other) = if and {
                (arm_otherwise, arm_then)
            } else {
                (arm_then, arm_otherwise)
            };
            let arm_shared = substitute(draft, arm, arm_edge, arm_shared);
            if !draft.same_edge(&arm_shared, shared)
                || arm_other.target == arm
                || arm_other.target == block
            {
                continue;
            }
            let arm_other = substitute(draft, arm, arm_edge, arm_other);
            if arm_other.target == shared.target && !draft.same_edge(&arm_other, shared) {
                continue;
            }
            let second = draft.resolve(*second);
            if !report.allow() {
                return changed;
            }
            let params = draft.block(arm).params.clone();
            for (&param, &arg) in params.iter().zip(&arm_edge.args) {
                draft.replace(param, arg);
            }
            while let Some(&inst) = draft.block(arm).insts.first() {
                draft.move_inst(inst, arm, block);
            }
            draft.block_mut(arm).alive = false;
            let constant = draft.add_constant(Constant::Int(Type::I1, u128::from(!and)));
            let args = if and {
                [first, second, constant]
            } else {
                [first, constant, second]
            };
            let merged = add_select(draft, block, args);
            let shared = shared.clone();
            draft.block_mut(block).term = if and {
                Term::Brif {
                    cond:      merged,
                    then:      arm_other,
                    otherwise: shared,
                }
            } else {
                Term::Brif {
                    cond:      merged,
                    then:      shared,
                    otherwise: arm_other,
                }
            };
            changed = true;
            break;
        }
    }
    changed
}

/// `edge` leaving `arm` as it would leave the block that `into` enters
/// `arm` from: `arm`'s parameters replaced by `into`'s arguments.
fn substitute<'s>(draft: &Draft<'_, 's>, arm: Block, into: &Edge<'_>, edge: &Edge<'_>) -> Edge<'s> {
    let params = &draft.block(arm).params;
    let mut args = ArenaVec::with_capacity_in(edge.args.len(), draft.scratch);
    for &arg in &edge.args {
        let arg = draft.resolve(arg);
        args.push(
            params
                .iter()
                .position(|&param| param == arg)
                .map_or(arg, |i| draft.resolve(into.args[i])),
        );
    }
    Edge {
        target: edge.target,
        args,
    }
}

fn add_select(draft: &mut Draft<'_, '_>, block: Block, args: [Value; 3]) -> Value {
    let position = draft.block(block).insts.len();
    draft
        .add_inst(
            block,
            position,
            InstData::Select { ty: Type::I1, args },
            Some(Type::I1),
        )
        .expect("`select` has a result")
}
