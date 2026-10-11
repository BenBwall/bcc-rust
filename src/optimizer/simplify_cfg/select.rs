//! Bounded if-conversion of two-entry joins. Speculated operations lose
//! poison-producing flags; memory, division, calls and freeze never move.

use super::{
    Draft,
    Edge,
    OptimizationReport,
    Term,
};
use crate::ir::{
    Block,
    Entity,
    InstData,
    InstFlags,
    Opcode,
};

/// Forms one select per differing argument of a diamond or triangle.
/// Partial bisect runs keep the branch and use each granted select on both
/// outgoing paths. Moving the arms is part of the first select rewrite.
pub(super) fn run(draft: &mut Draft<'_, '_>, report: &mut OptimizationReport) -> bool {
    let mut changed = false;
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
        let (cond, then, otherwise) = (draft.resolve(*cond), then.clone(), otherwise.clone());
        let predecessors = draft.predecessors();
        let candidate = |edge: &Edge<'_>| match &draft.block(edge.target).term {
            | Term::Jump(next)
                if edge.target != block
                    && edge.target != Draft::entry()
                    && predecessors.of(edge.target) == [block]
                    && cheap_arm(draft, edge.target) =>
                Some(next.target),
            | _ => None,
        };
        let join = if then.target == otherwise.target {
            Some(then.target)
        } else {
            candidate(&then)
                .filter(|&join| join == otherwise.target || candidate(&otherwise) == Some(join))
                .or_else(|| candidate(&otherwise).filter(|&join| join == then.target))
        };
        let Some(join) = join else {
            continue;
        };
        if join == block || join == Draft::entry() {
            continue;
        }
        let arms = [then.target, otherwise.target];
        if predecessors
            .of(join)
            .iter()
            .any(|&pred| pred != block && !arms.contains(&pred))
        {
            continue;
        }
        // A direct edge is the triangle's empty arm. Other arms must be
        // single-entry blocks ending in a jump to this join.
        if [&then, &otherwise]
            .iter()
            .any(|edge| edge.target != join && candidate(edge) != Some(join))
        {
            continue;
        }
        let mut a = outgoing(draft, &then, join);
        let mut b = outgoing(draft, &otherwise, join);
        // Resolve arm parameters locally before comparing incoming values.
        for (out, edge) in [(&mut a, &then), (&mut b, &otherwise)] {
            if edge.target != join {
                for arg in &mut out.args {
                    let value = draft.resolve(*arg);
                    *arg = draft
                        .block(edge.target)
                        .params
                        .iter()
                        .position(|&p| p == value)
                        .map_or(value, |i| draft.resolve(edge.args[i]));
                }
            }
        }
        let mut moved = false;
        let mut complete = true;
        for i in 0..a.args.len() {
            let (x, y) = (draft.resolve(a.args[i]), draft.resolve(b.args[i]));
            if x == y {
                continue;
            }
            if !report.allow() {
                complete = false;
                break;
            }
            if !moved {
                for edge in [&then, &otherwise] {
                    if edge.target == join {
                        continue;
                    }
                    let params = draft.block(edge.target).params.clone();
                    for (&param, &arg) in params.iter().zip(&edge.args) {
                        draft.replace(param, arg);
                    }
                    while let Some(&inst) = draft.block(edge.target).insts.first() {
                        if matches!(draft.inst(inst), InstData::Binary { .. }) {
                            draft.set_flags(inst, InstFlags::empty());
                        }
                        draft.move_inst(inst, edge.target, block);
                    }
                }
                moved = true;
            }
            let ty = draft.value_type(x);
            let data = if matches!(
                draft.constant(x),
                Some(super::super::draft::Constant::Int(_, 1))
            ) && matches!(
                draft.constant(y),
                Some(super::super::draft::Constant::Int(_, 0))
            ) && ty.is_int()
                && ty != crate::ir::Type::I1
            {
                InstData::Unary {
                    opcode: Opcode::Zext,
                    ty,
                    arg: cond,
                }
            } else {
                InstData::Select {
                    ty,
                    args: [cond, x, y],
                }
            };
            let position = draft.block(block).insts.len();
            let value = draft
                .add_inst(block, position, data, Some(ty))
                .expect("select defines a value");
            a.args[i] = value;
            b.args[i] = value;
            changed = true;
        }
        if !moved {
            continue;
        }
        if complete {
            draft.block_mut(block).term = Term::Jump(a);
            for arm in arms {
                if arm != join {
                    draft.block_mut(arm).alive = false;
                }
            }
        } else {
            // Keep unconverted arguments path-dependent at the bisect limit.
            for (edge, outgoing) in [(&then, a), (&otherwise, b)] {
                if edge.target != join {
                    draft.block_mut(edge.target).term = Term::Jump(outgoing);
                } else if let Term::Brif {
                    then: t,
                    otherwise: f,
                    ..
                } = &mut draft.block_mut(block).term
                {
                    if then.target == join {
                        *t = outgoing;
                    } else {
                        *f = outgoing;
                    }
                }
            }
        }
    }
    changed
}

/// LLVM's two-entry phi folding budget is four cost units per arm.
/// Constants and addresses cost zero; each permitted computation costs one.
fn cheap_arm(draft: &Draft<'_, '_>, block: Block) -> bool {
    let mut cost = 0;
    for &inst in &draft.block(block).insts {
        if draft.is_removed(inst) {
            continue;
        }
        if draft.inst_constant(inst).is_some() {
            continue;
        }
        let unit = match *draft.inst(inst) {
            | InstData::Binary {
                opcode:
                    Opcode::Iadd
                    | Opcode::Isub
                    | Opcode::Imul
                    | Opcode::And
                    | Opcode::Or
                    | Opcode::Xor
                    | Opcode::Shl
                    | Opcode::Lshr
                    | Opcode::Ashr
                    | Opcode::PtrAdd,
                ..
            }
            | InstData::IntCompare { .. }
            | InstData::Select { .. }
            | InstData::Unary {
                opcode:
                    Opcode::Zext
                    | Opcode::Sext
                    | Opcode::Trunc
                    | Opcode::Ptrtoint
                    | Opcode::Inttoptr
                    | Opcode::Bitcast,
                ..
            } => 1,
            | InstData::Const { .. }
            | InstData::WideConst { .. }
            | InstData::Nullary { .. }
            | InstData::StackAddr { .. }
            | InstData::GlobalAddr { .. }
            | InstData::FuncAddr { .. } => 0,
            | _ => return false,
        };
        cost += unit;
        if cost > 4 {
            return false;
        }
    }
    true
}

fn outgoing<'s>(draft: &Draft<'_, 's>, edge: &Edge<'s>, join: Block) -> Edge<'s> {
    if edge.target == join {
        edge.clone()
    } else {
        let Term::Jump(next) = &draft.block(edge.target).term else {
            unreachable!()
        };
        next.clone()
    }
}
