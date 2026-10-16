//! Scoped facts from dominating branch conditions, without use lists.
//!
//! A single incoming edge from a branching immediate dominator establishes
//! facts for that subtree. Enter/leave frames restore both known-zero bits
//! and ordered value pairs, so siblings and loop backedges gain no facts
//! from paths that do not dominate them. Local instructions propagate masks.

use super::{
    draft::{
        Constant,
        Draft,
        Term,
    },
    report::OptimizationReport,
};
use crate::{
    ir::{
        Block,
        DominatorTree,
        Entity,
        Inst,
        InstData,
        InstFlags,
        IntCC,
        Opcode,
        Value,
        ValueDef,
    },
    util::bump::ArenaVec,
};

/// Rewrites power-of-two division and remainder using scoped integer facts.
pub(super) fn run(draft: &mut Draft<'_, '_>, report: &mut OptimizationReport) -> bool {
    let scratch = draft.scratch;
    let cfg = draft.control_flow_graph();
    let tree = DominatorTree::from_cfg(&cfg, scratch);
    let mut children = ArenaVec::with_capacity_in(draft.blocks.len(), scratch);
    children.resize_with(draft.blocks.len(), || ArenaVec::new_in(scratch));
    for &block in tree.reverse_post_order() {
        if let Some(parent) = tree.idom(block) {
            children[parent.index()].push(block);
        }
    }
    let mut zeros = ArenaVec::with_capacity_in(draft.value_count(), scratch);
    zeros.resize(draft.value_count(), 0_u128);
    let mut undo = ArenaVec::new_in(scratch);
    let mut ordered = ArenaVec::new_in(scratch);
    let mut stack = ArenaVec::new_in(scratch);
    stack.push(Visit::Enter {
        block: Draft::entry(),
    });
    let mut changed = false;
    while let Some(visit) = stack.pop() {
        match visit {
            | Visit::Enter { block } => {
                stack.push(Visit::Leave(undo.len(), ordered.len()));
                for &child in children[block.index()].iter().rev() {
                    stack.push(Visit::Enter { block: child });
                }
                if let Some(parent) = tree.idom(block)
                    && cfg.predecessors(block) == [parent]
                    && let Term::Brif {
                        cond,
                        then,
                        otherwise,
                    } = &draft.block(parent).term
                    && then.target != otherwise.target
                {
                    condition(
                        draft,
                        *cond,
                        then.target == block,
                        &mut zeros,
                        &mut undo,
                        &mut ordered,
                    );
                }
                // Single-predecessor parameters carry their edge arguments'
                // facts. Multi-entry phis deliberately remain unknown.
                if let Some(parent) = tree.idom(block)
                    && cfg.predecessors(block) == [parent]
                    && let Some(edge) = draft.block(parent).term.edges().find(|e| e.target == block)
                    && draft
                        .block(parent)
                        .term
                        .edges()
                        .filter(|e| e.target == block)
                        .count()
                        == 1
                {
                    for (&param, &arg) in draft.block(block).params.iter().zip(&edge.args) {
                        let bits = known(draft, arg, &zeros);
                        remember(param, bits, &mut zeros, &mut undo);
                    }
                }
                let insts = draft.block(block).insts.clone();
                for inst in insts {
                    if draft.is_removed(inst) {
                        continue;
                    }
                    if rewrite(draft, inst, &zeros, report) {
                        changed = true;
                        // The rewrite added a constant, which may be a new
                        // value with facts of its own.
                        zeros.resize(draft.value_count(), 0);
                    }
                    if let Some(value) = draft.inst_result(inst) {
                        let bits = propagated(draft, inst, &zeros, &ordered);
                        remember(value, bits, &mut zeros, &mut undo);
                    }
                }
            },
            | Visit::Leave(mark, relations) => {
                while undo.len() > mark {
                    let (value, old) = undo.pop().expect("a saved fact remains");
                    zeros[value.index()] = old;
                }
                ordered.truncate(relations);
            },
        }
    }
    changed
}

#[derive(Clone, Copy)]
#[expect(
    variant_size_differences,
    reason = "Leave saves two arena scope marks on the explicit stack."
)]
enum Visit {
    Enter { block: Block },
    Leave(usize, usize),
}

/// Every fact is conditional on its value being defined, as with LLVM known
/// bits. None of the transformations makes a poison operand defined.
fn known(draft: &Draft<'_, '_>, value: Value, zeros: &[u128]) -> u128 {
    let value = draft.resolve(value);
    match draft.constant(value) {
        | Some(Constant::Int(ty, bits)) => !bits & ty.mask(),
        | _ => zeros[value.index()],
    }
}

fn remember(value: Value, bits: u128, zeros: &mut [u128], undo: &mut ArenaVec<'_, (Value, u128)>) {
    let old = zeros[value.index()];
    if old | bits != old {
        undo.push((value, old));
        zeros[value.index()] |= bits;
    }
}

/// Infers zero bits from equality and sign/order facts from signed compares.
fn condition<'s>(
    draft: &Draft<'_, '_>,
    value: Value,
    truth: bool,
    zeros: &mut [u128],
    undo: &mut ArenaVec<'s, (Value, u128)>,
    ordered: &mut ArenaVec<'s, (Value, Value)>,
) {
    let ValueDef::Result(inst) = draft.value_def(draft.resolve(value)) else {
        return;
    };
    let InstData::IntCompare {
        mut cond,
        ty,
        args: [a, b],
    } = *draft.inst(inst)
    else {
        return;
    };
    if !ty.is_int() {
        return;
    }
    if !truth {
        cond = match cond {
            | IntCC::Eq => IntCC::Ne,
            | IntCC::Ne => IntCC::Eq,
            | IntCC::Slt => IntCC::Sge,
            | IntCC::Sge => IntCC::Slt,
            | IntCC::Sle => IntCC::Sgt,
            | IntCC::Sgt => IntCC::Sle,
            | _ => return,
        };
    }
    let (a, b) = (draft.resolve(a), draft.resolve(b));
    if cond == IntCC::Eq {
        for (x, c) in [(a, b), (b, a)] {
            if let Some(Constant::Int(_, bits)) = draft.constant(c) {
                remember(x, !bits & ty.mask(), zeros, undo);
                if let ValueDef::Result(def) = draft.value_def(x)
                    && let InstData::Binary {
                        opcode: Opcode::And,
                        args: [n, m],
                        ..
                    } = *draft.inst(def)
                {
                    for (n, m) in [(n, m), (m, n)] {
                        if let Some(Constant::Int(_, mask)) = draft.constant(m) {
                            remember(draft.resolve(n), mask & !bits, zeros, undo);
                        }
                    }
                }
            }
        }
    }
    let (hi, lo, strict) = match cond {
        | IntCC::Sgt => (a, b, true),
        | IntCC::Sge => (a, b, false),
        | IntCC::Slt => (b, a, true),
        | IntCC::Sle => (b, a, false),
        | _ => return,
    };
    ordered.push((hi, lo));
    if let Some(Constant::Int(_, bound)) = draft.constant(lo) {
        let sign = 1_u128 << (ty.bits() - 1);
        // hi >= a nonnegative bound, or hi > -1, proves hi nonnegative.
        if bound & sign == 0 || (strict && bound == ty.mask()) {
            remember(hi, sign, zeros, undo);
        }
    }
}

/// Propagates inexpensive masks and the sign of an nsw ordered subtraction.
fn propagated(
    draft: &Draft<'_, '_>,
    inst: Inst,
    zeros: &[u128],
    ordered: &[(Value, Value)],
) -> u128 {
    if let Some(Constant::Int(ty, bits)) = draft.inst_constant(inst) {
        return !bits & ty.mask();
    }
    let k = |v| known(draft, v, zeros);
    match *draft.inst(inst) {
        | InstData::Binary {
            opcode,
            ty,
            flags,
            args: [a, b],
        } => {
            let (a, b) = (draft.resolve(a), draft.resolve(b));
            match opcode {
                | Opcode::And => k(a) | k(b),
                | Opcode::Or => k(a) & k(b),
                | Opcode::Isub if flags.contains(InstFlags::NSW) && ordered.contains(&(a, b)) =>
                    1 << (ty.bits() - 1),
                | Opcode::Lshr =>
                    if let Some(Constant::Int(_, shift)) = draft.constant(b) {
                        if shift < u128::from(ty.bits()) {
                            (k(a) >> shift) | (ty.mask() ^ (ty.mask() >> shift))
                        } else {
                            0
                        }
                    } else {
                        0
                    },
                | _ => 0,
            }
        },
        | InstData::Unary {
            opcode: Opcode::Zext,
            ty,
            arg,
        } => k(arg) | (ty.mask() ^ draft.value_type(draft.resolve(arg)).mask()),
        | InstData::Unary {
            opcode: Opcode::Trunc,
            ty,
            arg,
        } => k(arg) & ty.mask(),
        | InstData::Select {
            args: [_, a, b], ..
        } => k(a) & k(b),
        | _ => 0,
    }
}

/// Signed division needs an exact multiple or a nonnegative dividend.
/// Unsigned power-of-two operations are valid for any dividend.
fn rewrite(
    draft: &mut Draft<'_, '_>,
    inst: Inst,
    zeros: &[u128],
    report: &mut OptimizationReport,
) -> bool {
    if draft.inst_constant(inst).is_some() {
        return false;
    }
    let InstData::Binary {
        opcode,
        ty,
        flags,
        args: [x, divisor],
    } = *draft.inst(inst)
    else {
        return false;
    };
    if !matches!(
        opcode,
        Opcode::Sdiv | Opcode::Udiv | Opcode::Srem | Opcode::Urem
    ) {
        return false;
    }
    let Some(Constant::Int(_, bits)) = draft.constant(divisor) else {
        return false;
    };
    if !bits.is_power_of_two() {
        return false;
    }
    let signed = matches!(opcode, Opcode::Sdiv | Opcode::Srem);
    if signed && bits & (1 << (ty.bits() - 1)) != 0 {
        return false;
    }
    let zero = known(draft, x, zeros);
    let nonnegative = zero & (1 << (ty.bits() - 1)) != 0;
    let exact = zero & (bits - 1) == bits - 1;
    let (op, immediate, new_flags) = match opcode {
        | Opcode::Sdiv if exact => (
            Opcode::Ashr,
            u128::from(bits.trailing_zeros()),
            InstFlags::EXACT,
        ),
        | Opcode::Sdiv | Opcode::Udiv if !signed || nonnegative => (
            Opcode::Lshr,
            u128::from(bits.trailing_zeros()),
            flags & InstFlags::EXACT,
        ),
        | Opcode::Srem | Opcode::Urem if !signed || nonnegative =>
            (Opcode::And, bits - 1, InstFlags::empty()),
        | _ => return false,
    };
    if !report.allow() {
        return false;
    }
    let rhs = draft.add_constant(Constant::Int(ty, immediate));
    draft.replace_inst(
        inst,
        InstData::Binary {
            opcode: op,
            ty,
            flags: new_flags,
            args: [draft.resolve(x), rhs],
        },
    );
    true
}
