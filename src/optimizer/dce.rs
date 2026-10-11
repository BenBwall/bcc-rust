//! Dead code elimination.
//!
//! The IR keeps no use lists, so the pass computes use counts as an analysis
//! ([`Draft::use_counts`]) and then works through two kinds of candidate with
//! an explicit worklist:
//!
//! - an instruction whose result has no uses and which has no side effect;
//! - a block parameter, other than the entry's, with no uses.
//!
//! Removing an instruction releases its operands, which may leave their
//! definitions unused; removing a parameter also removes the matching
//! argument from every edge into its block, which releases those arguments.
//! Each removal asks the report for permission first, and after a refusal
//! nothing more is removed, so every removal that did happen was valid on its
//! own.
//!
//! What has a side effect: stores, `copy`, `fill`, calls, volatile loads and
//! the `va_*` operations (`va_arg` advances the list). A load that is not
//! volatile may be removed even though it could trap: a trapping load is
//! undefined behaviour, which the program may not rely on (C99 §6.5.3.2
//! paragraph 4, p. 79; PDF p. 91). Unused division is removable for the same
//! reason. Branch conditions and returned values are always used.
//!
//! A cycle of values that only feed each other through block parameters keeps
//! each use count positive, so the pass leaves it; that needs an aggressive
//! (mark and sweep) variant, which the prototype omits.
//!
//! C99: §5.1.2.3 paragraph 5, pp. 13-14; PDF pp. 25-26 (only volatile
//! accesses, file writes and interactive I/O are observable).

use super::{
    draft::Draft,
    report::OptimizationReport,
};
use crate::{
    ir::{
        Entity,
        Inst,
        InstData,
        InstFlags,
        Opcode,
        Value,
        ValueDef,
    },
    util::bump::ArenaVec,
};

/// Removes dead instructions and parameters from one function. Returns
/// whether it removed anything.
pub(super) fn run(draft: &mut Draft<'_, '_>, report: &mut OptimizationReport) -> bool {
    let mut counts = draft.use_counts();
    let predecessors = draft.predecessors();
    let mut worklist: ArenaVec<'_, Candidate> = ArenaVec::new_in(draft.scratch);
    for block in draft.alive_blocks() {
        for &inst in draft.block(block).insts.iter().rev() {
            worklist.push(Candidate::Inst(inst));
        }
        if block != Draft::entry() {
            for &param in draft.block(block).params.iter().rev() {
                worklist.push(Candidate::Param(param));
            }
        }
    }
    let mut changed = false;
    while let Some(candidate) = worklist.pop() {
        let released = match candidate {
            | Candidate::Inst(inst) => remove_inst(draft, &mut counts, inst, report),
            | Candidate::Param(param) =>
                remove_param(draft, &predecessors, &mut counts, param, report),
        };
        let Some(released) = released else {
            if report.limit_reached() {
                break;
            }
            continue;
        };
        changed = true;
        for value in released {
            if counts[value.index()] != 0 {
                continue;
            }
            match draft.body.value_def(value) {
                | ValueDef::Result(def) => worklist.push(Candidate::Inst(def)),
                | ValueDef::Param(..) => worklist.push(Candidate::Param(value)),
            }
        }
    }
    if changed {
        draft.sweep();
    }
    changed
}

#[derive(Clone, Copy, Debug)]
enum Candidate {
    Inst(Inst),
    /// A block parameter, named by its value.
    Param(Value),
}

/// Removes `inst` if it is dead. Returns the values it released, or `None`
/// if it stays.
fn remove_inst<'s>(
    draft: &mut Draft<'_, 's>,
    counts: &mut [u32],
    inst: Inst,
    report: &mut OptimizationReport,
) -> Option<ArenaVec<'s, Value>> {
    if draft.is_removed(inst) {
        return None;
    }
    let result = draft.inst_result(inst)?;
    if counts[result.index()] != 0 || has_side_effects(draft.inst(inst)) || !report.allow() {
        return None;
    }
    draft.remove_inst(inst);
    let mut released = ArenaVec::new_in(draft.scratch);
    draft.for_each_operand(inst, |value| {
        counts[value.index()] -= 1;
        released.push(value);
    });
    Some(released)
}

/// Removes the block parameter `param` if it is unused, with the matching
/// argument of every edge into its block. Returns the arguments it released,
/// or `None` if the parameter stays.
fn remove_param<'s>(
    draft: &mut Draft<'_, 's>,
    predecessors: &super::draft::Predecessors<'_>,
    counts: &mut [u32],
    param: Value,
    report: &mut OptimizationReport,
) -> Option<ArenaVec<'s, Value>> {
    let ValueDef::Param(block, _) = draft.body.value_def(param) else {
        return None;
    };
    let position = draft.block(block).params.iter().position(|&p| p == param)?;
    if !draft.block(block).alive
        || block == Draft::entry()
        || counts[param.index()] != 0
        || !report.allow()
    {
        return None;
    }
    _ = draft.block_mut(block).params.remove(position);
    let mut released = ArenaVec::new_in(draft.scratch);
    for &predecessor in predecessors.of(block) {
        // This pass adds no edges, so the predecessors computed up front are
        // still exactly the blocks with an edge into this one.
        for edge in draft.block_mut(predecessor).term.edges_mut() {
            if edge.target == block {
                released.push(edge.args.remove(position));
            }
        }
    }
    for arg in &mut released {
        *arg = draft.resolve(*arg);
        counts[arg.index()] -= 1;
    }
    Some(released)
}

/// Whether removing the instruction would change what the program does besides
/// defining a value.
fn has_side_effects(data: &InstData) -> bool {
    match *data {
        | InstData::Store { .. }
        | InstData::MemoryRange { .. }
        | InstData::Call { .. }
        | InstData::CallIndirect { .. }
        | InstData::Unary {
            opcode: Opcode::VaStart | Opcode::VaArg | Opcode::VaEnd,
            ..
        }
        | InstData::Binary {
            opcode: Opcode::VaCopy,
            ..
        } => true,
        | InstData::Load { flags, .. } => flags.contains(InstFlags::VOLATILE),
        | _ => false,
    }
}
