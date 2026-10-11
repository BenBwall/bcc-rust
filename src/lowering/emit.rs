//! Copying a finished draft into the module.
//!
//! Emission keeps only the blocks reachable from the entry and lays them out
//! in reverse post-order, so every value is defined before the text uses it.
//! Before that it removes the block parameters SSA construction made
//! redundant: a parameter whose incoming arguments, ignoring itself, are all
//! one value is that value (Braun et al., section 3.1). The check repeats
//! until nothing changes, so parameters made redundant by an earlier removal
//! go too. Edges then pass only the arguments of the parameters that remain.

use super::{
    LoweringError,
    ssa::{
        Def,
        Draft,
        Edge,
        Op,
        Terminator,
    },
};
use crate::{
    ir::{
        Block,
        Entity,
        FuncId,
        FunctionBuilder,
        InstData,
        Module,
        Value,
    },
    util::bump::ArenaVec,
};

/// Installs `draft` as the body of `func`.
pub(super) fn emit<'s>(
    mut draft: Draft<'s>,
    module: &mut Module<'_>,
    func: FuncId,
) -> Result<(), LoweringError> {
    let scratch = draft.scratch();
    let order = reverse_post_order(&draft);
    let mut reachable = ArenaVec::with_capacity_in(draft.blocks.len(), scratch);
    reachable.resize(draft.blocks.len(), false);
    for &block in &order {
        reachable[block.index()] = true;
    }
    remove_redundant_params(&mut draft, &order, &reachable);

    let mut builder = FunctionBuilder::new(module, func);
    for &slot in &draft.slots {
        _ = builder.create_stack_slot(slot.size, slot.align);
    }
    let mut blocks = ArenaVec::with_capacity_in(draft.blocks.len(), scratch);
    blocks.resize(draft.blocks.len(), None);
    for (position, &block) in order.iter().enumerate() {
        blocks[block.index()] = Some(if position == 0 {
            builder.create_entry_block()
        } else {
            builder.create_block()
        });
    }
    let mut values: ArenaVec<'_, Option<Value>> =
        ArenaVec::with_capacity_in(draft.values.len(), scratch);
    values.resize(draft.values.len(), None);
    let missing = || LoweringError {
        kind:   super::LoweringErrorKind::MissingFact("a value is used where it is not defined"),
        source: None,
    };
    for (position, &block) in order.iter().enumerate() {
        let target = blocks[block.index()].expect("reachable blocks are created");
        builder.switch_to_block(target);
        let data = &draft.blocks[block.index()];
        if position == 0 {
            let params = builder.body().block_params(target);
            for (&param, &value) in data.params.iter().zip(params) {
                values[param.index()] = Some(value);
            }
            for (index, &used) in used_poison(&draft, &order).iter().enumerate() {
                if used {
                    values[index] = Some(builder.poison(draft.values[index].ty));
                }
            }
        } else {
            for &param in &data.params {
                if draft.values[param.index()].alias.is_none() {
                    let ty = draft.values[param.index()].ty;
                    values[param.index()] = Some(builder.append_block_param(target, ty));
                }
            }
        }
        let map = |value: Value, values: &ArenaVec<'_, Option<Value>>| {
            values[draft.resolve(value).index()].ok_or_else(missing)
        };
        for &(op, result) in &data.insts {
            let emitted = match op {
                | Op::Inst(inst) => {
                    let inst = map_operands(inst, |value| map(value, &values))?;
                    let inst = builder.insert(inst);
                    builder.inst_result(inst)
                },
                | Op::Iconst(ty, constant) => Some(builder.iconst(ty, constant)),
                | Op::Call(callee, args) => {
                    let args = map_list(args, &values, &map, scratch)?;
                    let inst = builder.call(callee, &args);
                    builder.inst_result(inst)
                },
                | Op::CallIndirect(sig, callee, args) => {
                    let callee = map(callee, &values)?;
                    let args = map_list(args, &values, &map, scratch)?;
                    let inst = builder.call_indirect(sig, callee, &args);
                    builder.inst_result(inst)
                },
            };
            if let Some(result) = result {
                values[result.index()] = emitted;
            }
        }
        let edge_args = |edge: &Edge<'s>| -> Result<ArenaVec<'s, Value>, LoweringError> {
            let params = draft.block_params(edge.target);
            let mut args = ArenaVec::with_capacity_in(edge.args.len(), scratch);
            for (&param, &arg) in params.iter().zip(&edge.args) {
                if draft.values[param.index()].alias.is_none() {
                    args.push(map(arg, &values)?);
                }
            }
            Ok(args)
        };
        let target_of =
            |edge: &Edge<'_>| blocks[edge.target.index()].expect("successors are reachable");
        match data.terminator.as_ref() {
            | Some(Terminator::Jump(edge)) => {
                let args = edge_args(edge)?;
                builder.jump(target_of(edge), &args);
            },
            | Some(Terminator::Brif(cond, [then, otherwise])) => {
                let cond = map(*cond, &values)?;
                let then_args = edge_args(then)?;
                let otherwise_args = edge_args(otherwise)?;
                builder.brif(
                    cond,
                    (target_of(then), &then_args),
                    (target_of(otherwise), &otherwise_args),
                );
            },
            | Some(Terminator::Switch {
                value,
                default,
                cases,
            }) => {
                let value = map(*value, &values)?;
                let default_args = edge_args(default)?;
                let mut case_args = ArenaVec::with_capacity_in(cases.len(), scratch);
                for (_, edge) in cases {
                    case_args.push(edge_args(edge)?);
                }
                let mut list = ArenaVec::with_capacity_in(cases.len(), scratch);
                for ((case, edge), args) in cases.iter().zip(&case_args) {
                    list.push((*case, target_of(edge), args.as_slice()));
                }
                builder.switch(value, (target_of(default), &default_args), &list);
            },
            | Some(Terminator::Return(value)) => {
                let value = value.map(|value| map(value, &values)).transpose()?;
                builder.ret(value);
            },
            | None => builder.unreachable(),
        }
    }
    builder.finish();
    Ok(())
}

/// Which values are `poison` used by a reachable block, after redundant
/// parameters are resolved.
fn used_poison<'s>(draft: &Draft<'s>, order: &[Block]) -> ArenaVec<'s, bool> {
    let mut used = ArenaVec::with_capacity_in(draft.values.len(), draft.scratch());
    used.resize(draft.values.len(), false);
    let mut mark = |value: Value| {
        let value = draft.resolve(value);
        if draft.values[value.index()].def == Def::Poison {
            used[value.index()] = true;
        }
    };
    for &block in order {
        let data = &draft.blocks[block.index()];
        for &(op, _) in &data.insts {
            match op {
                | Op::Inst(inst) => {
                    _ = map_operands(inst, |value| {
                        mark(value);
                        Ok(value)
                    });
                },
                | Op::Iconst(..) => {},
                | Op::Call(_, args) =>
                    for &arg in args {
                        mark(arg);
                    },
                | Op::CallIndirect(_, callee, args) => {
                    mark(callee);
                    for &arg in args {
                        mark(arg);
                    }
                },
            }
        }
        let Some(terminator) = &data.terminator else {
            continue;
        };
        match *terminator {
            | Terminator::Brif(value, _)
            | Terminator::Switch { value, .. }
            | Terminator::Return(Some(value)) => mark(value),
            | _ => {},
        }
        for edge in terminator.edges() {
            let params = draft.block_params(edge.target);
            for (&param, &arg) in params.iter().zip(&edge.args) {
                if draft.values[param.index()].alias.is_none() {
                    mark(arg);
                }
            }
        }
    }
    used
}

/// The blocks reachable from the entry, in reverse post-order, by an
/// iterative depth-first search.
fn reverse_post_order<'s>(draft: &Draft<'s>) -> ArenaVec<'s, Block> {
    let scratch = draft.scratch();
    let count = draft.blocks.len();
    let mut visited = ArenaVec::with_capacity_in(count, scratch);
    visited.resize(count, false);
    let mut post_order = ArenaVec::with_capacity_in(count, scratch);
    let mut stack: ArenaVec<'_, (Block, usize)> = ArenaVec::new_in(scratch);
    let entry = Draft::entry();
    visited[entry.index()] = true;
    stack.push((entry, 0));
    // Successors are visited last to first, so the first of them comes
    // first in the reverse post-order: a `brif`'s true side before its
    // false side, a loop body before its exit.
    while let Some(&mut (block, ref mut visited_edges)) = stack.last_mut() {
        let successor = draft.blocks[block.index()]
            .terminator
            .as_ref()
            .and_then(|terminator| {
                let count = terminator.edges().count();
                let index = count.checked_sub(*visited_edges + 1)?;
                terminator.edges().nth(index)
            })
            .map(|edge| edge.target);
        *visited_edges += 1;
        match successor {
            | Some(successor) =>
                if !visited[successor.index()] {
                    visited[successor.index()] = true;
                    stack.push((successor, 0));
                },
            | None => {
                post_order.push(block);
                _ = stack.pop();
            },
        }
    }
    post_order.reverse();
    post_order
}

/// Aliases each parameter whose reachable incoming arguments are, apart
/// from itself, a single value, repeating until none is found.
fn remove_redundant_params(draft: &mut Draft<'_>, order: &[Block], reachable: &[bool]) {
    let entry = Draft::entry();
    loop {
        let mut changed = false;
        for &block in order {
            if block == entry {
                continue;
            }
            for index in 0..draft.blocks[block.index()].params.len() {
                let param = draft.blocks[block.index()].params[index];
                if draft.values[param.index()].alias.is_some() {
                    continue;
                }
                let mut same = None;
                let mut redundant = true;
                for &(pred, edge) in &draft.blocks[block.index()].preds {
                    if !reachable[pred.index()] {
                        continue;
                    }
                    let terminator = draft.blocks[pred.index()]
                        .terminator
                        .as_ref()
                        .expect("a predecessor ends in a branch");
                    let arg = terminator
                        .edges()
                        .nth(edge as usize)
                        .expect("the edge exists")
                        .args[index];
                    let arg = draft.resolve(arg);
                    if arg == param || Some(arg) == same {
                        continue;
                    }
                    if same.is_some() {
                        redundant = false;
                        break;
                    }
                    same = Some(arg);
                }
                if redundant {
                    let ty = draft.values[param.index()].ty;
                    let to = same.unwrap_or_else(|| draft.poison(ty));
                    draft.alias(param, to);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
}

fn map_list<'s>(
    args: &[Value],
    values: &ArenaVec<'_, Option<Value>>,
    map: &impl Fn(Value, &ArenaVec<'_, Option<Value>>) -> Result<Value, LoweringError>,
    scratch: &'s crate::util::bump::Bump,
) -> Result<ArenaVec<'s, Value>, LoweringError> {
    let mut list = ArenaVec::with_capacity_in(args.len(), scratch);
    for &arg in args {
        list.push(map(arg, values)?);
    }
    Ok(list)
}

/// `inst` with each operand replaced; only formats without pool operands
/// reach here.
fn map_operands(
    inst: InstData,
    mut map: impl FnMut(Value) -> Result<Value, LoweringError>,
) -> Result<InstData, LoweringError> {
    Ok(match inst {
        | InstData::Binary {
            opcode,
            ty,
            flags,
            args: [a, b],
        } => InstData::Binary {
            opcode,
            ty,
            flags,
            args: [map(a)?, map(b)?],
        },
        | InstData::Unary { opcode, ty, arg } => InstData::Unary {
            opcode,
            ty,
            arg: map(arg)?,
        },
        | InstData::IntCompare {
            cond,
            ty,
            args: [a, b],
        } => InstData::IntCompare {
            cond,
            ty,
            args: [map(a)?, map(b)?],
        },
        | InstData::FloatCompare {
            cond,
            ty,
            args: [a, b],
        } => InstData::FloatCompare {
            cond,
            ty,
            args: [map(a)?, map(b)?],
        },
        | InstData::Select {
            ty,
            args: [c, a, b],
        } => InstData::Select {
            ty,
            args: [map(c)?, map(a)?, map(b)?],
        },
        | InstData::Load {
            ty,
            flags,
            align,
            tag,
            addr,
        } => InstData::Load {
            ty,
            flags,
            align,
            tag,
            addr: map(addr)?,
        },
        | InstData::Store {
            ty,
            flags,
            align,
            tag,
            args: [value, addr],
        } => InstData::Store {
            ty,
            flags,
            align,
            tag,
            args: [map(value)?, map(addr)?],
        },
        | InstData::MemoryRange {
            opcode,
            flags,
            align,
            args: [a, b, c],
        } => InstData::MemoryRange {
            opcode,
            flags,
            align,
            args: [map(a)?, map(b)?, map(c)?],
        },
        | other => other,
    })
}
