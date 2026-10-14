//! Bounded if-conversion of two-entry joins. Speculated operations lose
//! poison-producing flags; memory, division, calls and freeze never move.

use super::{
    Draft,
    Edge,
    OptimizationReport,
    Term,
};
use crate::{
    ir::{
        Block,
        ControlFlowGraph,
        Entity,
        InstData,
        InstFlags,
        Opcode,
    },
    util::bump::Bump,
};

/// Forms one select per differing argument of a diamond or triangle.
/// Partial bisect runs keep the branch and use each granted select on both
/// outgoing paths. Moving the arms is part of the first select rewrite.
pub(super) fn run(draft: &mut Draft<'_, '_>, report: &mut OptimizationReport) -> bool {
    run_in(draft, report, &mut Bump::new())
}

/// Cached predecessors live separately from edits and are reclaimed on a
/// graph change. C99: §6.8.4.1 paragraph 2, pp. 133-134; PDF pp. 145-146.
fn run_in(draft: &mut Draft<'_, '_>, report: &mut OptimizationReport, analysis: &mut Bump) -> bool {
    let mut cached = None;
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
        let candidate = |edge: &Edge<'_>| match &draft.block(edge.target).term {
            | Term::Jump(next)
                if edge.target != block
                    && edge.target != Draft::entry()
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
        let predecessors = cached.get_or_insert_with(|| {
            ControlFlowGraph::from_successors(draft.blocks.len(), analysis, |block, add| {
                let data = draft.block(block);
                if data.alive {
                    data.term.edges().for_each(|edge| add(edge.target));
                }
            })
        });
        if arms
            .iter()
            .any(|&arm| arm != join && predecessors.predecessors(arm) != [block])
        {
            continue;
        }
        if predecessors
            .predecessors(join)
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
            cached = None;
            analysis.reset();
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
fn cheap_arm(draft: &Draft<'_, '_>, block: Block) -> bool {
    cheap_arm_within(draft, block, 4)
}

/// Whether `block`'s instructions are safe to speculate and cost at most
/// `budget` units. Constants and addresses cost zero; each permitted
/// computation costs one.
pub(super) fn cheap_arm_within(draft: &Draft<'_, '_>, block: Block, budget: u32) -> bool {
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
            | InstData::Nullary {
                opcode: Opcode::Poison | Opcode::Null,
                ..
            }
            | InstData::StackAddr { .. }
            | InstData::GlobalAddr { .. }
            | InstData::FuncAddr { .. } => 0,
            | _ => return false,
        };
        cost += unit;
        if cost > budget {
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

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "This regression builds test IR with owned strings."
)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::{
        ir::{
            Profile,
            parse_module,
            verify_module,
        },
        util::bump::Bump,
    };
    #[test]
    fn ineligible_call_arms_use_bounded_time_and_scratch() {
        use std::fmt::Write as _;
        // Fail at the small size before a quadratic implementation can retain
        // gigabytes at the full review workload.
        for (count, calls) in [(1000, true), (10_000, true), (1000, false), (10_000, false)] {
            let mut text = "function @sink() external\nfunction @f(i1) external {\nblock0(v0: \
                            i1):\n"
                .to_owned();
            for index in 0..count {
                let arm = 2 * index + 1;
                let join = arm + 1;
                writeln!(
                    text,
                    " brif v0, block{arm}, block{join}\nblock{arm}:\n call @sink()\n jump \
                     block{join}\nblock{join}:"
                )
                .unwrap();
            }
            text.push_str(" return\n}\n");
            if !calls {
                text = text.replace(" call @sink()\n", "");
            }
            let arena = Bump::new();
            let module = parse_module(&arena, &text).unwrap();
            assert!(verify_module(&module, Profile::PreAbi, &Bump::new()).is_empty());
            let body = module
                .functions()
                .find(|(_, f)| f.name == "f")
                .unwrap()
                .1
                .body
                .as_ref()
                .unwrap();
            let scratch = Bump::new();
            let mut draft = Draft::lift(body, &scratch);
            let before = scratch.used();
            let start = Instant::now();
            let mut report = OptimizationReport::new(None);
            let mut analysis = Bump::new();
            assert!(!run_in(&mut draft, &mut report, &mut analysis));
            assert!(analysis.high_water() < count * 1024);
            assert!(
                scratch.high_water() - before < count * 1024,
                "{count} call triangles retained {} analysis bytes",
                scratch.high_water() - before
            );
            assert!(
                start.elapsed() < std::time::Duration::from_secs(2),
                "{count} call triangles took {:?}",
                start.elapsed()
            );
            eprintln!(
                "{count} triangles (calls={calls}): {} analysis bytes, {:?}",
                analysis.high_water() + scratch.high_water() - before,
                start.elapsed()
            );
        }
    }
}
