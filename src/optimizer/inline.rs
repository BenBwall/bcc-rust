//! Bottom-up direct-call expansion. Read `run`, then `expand`, then the
//! explicit-stack Tarjan walk in `CallGraph::compute`. Each call site owns
//! fresh slots, blocks, parameters, instructions and constant bits; only
//! module-level symbol and signature ids are shared.

use super::{
    OptimizationReport,
    draft::{
        Draft,
        Edge,
        Term,
    },
};
use crate::{
    ir::{
        Block,
        Body,
        Entity,
        FuncId,
        Inst,
        InstData,
        Module,
        Opcode,
        StackSlot,
        Value,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// Expands the calls present at the start of this pass. Copied calls wait
/// for a subsequent explicitly requested pass: the default pipeline visits
/// each callee first and runs this pass only once per function.
pub(super) fn run(
    draft: &mut Draft<'_, '_>,
    module: &Module<'_>,
    graph: &CallGraph<'_>,
    report: &mut OptimizationReport,
) -> bool {
    let original_size = live_size(draft);
    let cap = original_size
        .saturating_add(original_size.max(256))
        .min(MAX_CALLER_SIZE);
    let mut size = original_size;
    let mut sites = ArenaVec::new_in(draft.scratch);
    for block in draft.alive_blocks() {
        for &inst in &draft.block(block).insts {
            if !draft.is_removed(inst)
                && let InstData::Call { func, .. } = *draft.inst(inst)
            {
                sites.push((inst, func));
            }
        }
    }
    let mut changed = false;
    for (call, func) in sites {
        if graph.recursive[func.index()] || graph.unsafe_frame[func.index()] {
            continue;
        }
        let function = module.function(func);
        let Some(body) = &function.body else { continue };
        // A copy of a body whose block addresses are taken would need its
        // own labels, and its indirect branches the copies' addresses; LLVM
        // refuses to inline such functions too.
        if module.function_signature(func).variadic || body.has_indirect_control() {
            continue;
        }
        let leaf = body.blocks().all(|block| {
            body.block_insts(block).iter().all(|&inst| {
                !matches!(
                    body.inst(inst),
                    InstData::Call { .. } | InstData::CallIndirect { .. }
                )
            })
        });
        let small_leaf = leaf && body.inst_count() <= SMALL_LEAF_SIZE;
        // Splitting adds a jump, and each callee return becomes a jump.
        let grown = size.saturating_add(body.inst_count());
        if (!small_leaf && body.inst_count() > CALLEE_SIZE) || grown > cap {
            continue;
        }
        let location = draft.alive_blocks().find_map(|block| {
            draft
                .block(block)
                .insts
                .iter()
                .position(|&inst| inst == call)
                .map(|position| (block, position))
        });
        let Some((block, position)) = location else {
            continue;
        };
        if !report.allow() {
            break;
        }
        expand(draft, body, block, position, call);
        size = grown;
        changed = true;
    }
    changed
}

// Small leaves are always worthwhile within the caller budget. Larger
// leaves and non-leaves use the same bounded instruction threshold; these
// counts include terminators, not estimated machine-code cost units.
const SMALL_LEAF_SIZE: usize = 30;
const CALLEE_SIZE: usize = 100;
// At most double the caller (or add 256 instructions for a small caller),
// never exceeding 2,000 instructions. This bounds repeated call-site growth.
const MAX_CALLER_SIZE: usize = 2_000;

fn live_size(draft: &Draft<'_, '_>) -> usize {
    draft
        .alive_blocks()
        .map(|block| {
            1 + draft
                .block(block)
                .insts
                .iter()
                .filter(|&&inst| !draft.is_removed(inst))
                .count()
        })
        .sum()
}

/// Splits the call's block, reserves all copied definitions before remapping
/// operands (layout need not be dominance order), and joins every return.
fn expand(draft: &mut Draft<'_, '_>, body: &Body<'_>, block: Block, position: usize, call: Inst) {
    let scratch = draft.scratch;
    let InstData::Call { args, .. } = *draft.inst(call) else {
        unreachable!()
    };
    let arguments = scratch.alloc_slice_fill_iter(
        draft
            .call_operands(call, args)
            .iter()
            .map(|&arg| draft.resolve(arg)),
    );
    let continuation = draft.add_block();
    if let Some(result) = draft.inst_result(call) {
        let param = draft.add_block_param(continuation, draft.value_type(result));
        draft.replace(result, param);
    }
    let mut suffix = ArenaVec::new_in(scratch);
    suffix.extend_from_slice(&draft.block(block).insts[position + 1..]);
    draft.block_mut(continuation).insts = suffix;
    draft.block_mut(continuation).term =
        std::mem::replace(&mut draft.block_mut(block).term, Term::Unreachable);
    draft.block_mut(block).insts.truncate(position);
    draft.remove_inst(call);

    let mut slots = ArenaVec::new_in(scratch);
    for (_, data) in body.stack_slots() {
        slots.push(draft.add_stack_slot(data));
    }
    let mut blocks = ArenaVec::new_in(scratch);
    for _ in body.blocks() {
        blocks.push(draft.add_block());
    }
    let mut values = ArenaVec::new_in(scratch);
    values.resize(body.value_count(), None);
    let entry = body.entry_block().expect("verified callee has an entry");
    for (&param, &arg) in body.block_params(entry).iter().zip(arguments.iter()) {
        values[param.index()] = Some(arg);
    }
    let mut instructions = ArenaVec::new_in(scratch);
    instructions.resize(body.inst_count(), None);
    for old_block in body.blocks() {
        let new_block = blocks[old_block.index()];
        if old_block != entry {
            for &param in body.block_params(old_block) {
                values[param.index()] =
                    Some(draft.add_block_param(new_block, body.value_type(param)));
            }
        }
        for &old in body.block_insts(old_block) {
            if body.inst(old).is_terminator() {
                continue;
            }
            let result_type = body.inst_result(old).map(|value| body.value_type(value));
            let index = draft.block(new_block).insts.len();
            let result = draft.add_inst(new_block, index, *body.inst(old), result_type);
            let new_inst = draft.block(new_block).insts[index];
            instructions[old.index()] = Some(new_inst);
            if let Some(value) = body.inst_result(old) {
                values[value.index()] = result;
            }
        }
    }
    let map = |value: Value| values[value.index()].expect("every callee definition was reserved");
    for old_block in body.blocks() {
        for &old in body.block_insts(old_block) {
            if let Some(new_inst) = instructions[old.index()] {
                let data = remap_inst(*body.inst(old), &map, &slots);
                draft.replace_inst(new_inst, data);
                if let Some(bits) = body.const_bits(body.inst(old)) {
                    draft.set_const_bits(new_inst, bits);
                }
                if let InstData::Call { args, .. } | InstData::CallIndirect { args, .. } = data {
                    let mapped = scratch
                        .alloc_slice_fill_iter(body.value_list(args).iter().map(|&v| map(v)));
                    draft.set_call_args(new_inst, mapped);
                }
            }
        }
        let old_term = body
            .terminator(old_block)
            .expect("verified callee has terminators");
        draft.block_mut(blocks[old_block.index()]).term =
            remap_term(body, old_term, &blocks, &map, continuation, scratch);
    }
    draft.block_mut(block).term = Term::Jump(Edge {
        target: blocks[entry.index()],
        args:   ArenaVec::new_in(scratch),
    });
}

fn remap_inst(mut data: InstData, map: &impl Fn(Value) -> Value, slots: &[StackSlot]) -> InstData {
    match &mut data {
        | InstData::Binary { args, .. }
        | InstData::IntCompare { args, .. }
        | InstData::FloatCompare { args, .. }
        | InstData::Store { args, .. } => *args = args.map(map),
        | InstData::Select { args, .. } | InstData::MemoryRange { args, .. } =>
            *args = args.map(map),
        | InstData::Unary { arg, .. }
        | InstData::VaArgAggregate { list: arg, .. }
        | InstData::DynAlloca { size: arg, .. } => *arg = map(*arg),
        | InstData::Load { addr, .. } => *addr = map(*addr),
        | InstData::StackAddr { slot } => *slot = slots[slot.index()],
        // Constants and calls get owned pool data in expand; symbol ids are module-wide.
        | _ => {},
    }
    data
}

fn remap_term<'s>(
    body: &Body<'_>,
    inst: Inst,
    blocks: &[Block],
    map: &impl Fn(Value) -> Value,
    continuation: Block,
    scratch: &'s Bump,
) -> Term<'s> {
    let edge = |call| {
        let (target, args) = body.block_call(call);
        let mut mapped = ArenaVec::new_in(scratch);
        mapped.extend(args.iter().map(|&v| map(v)));
        Edge {
            target: blocks[target.index()],
            args:   mapped,
        }
    };
    match *body.inst(inst) {
        | InstData::Jump { dest } => Term::Jump(edge(dest)),
        | InstData::Brif { cond, dests } => Term::Brif {
            cond:      map(cond),
            then:      edge(dests[0]),
            otherwise: edge(dests[1]),
        },
        | InstData::Switch {
            value,
            default,
            cases,
        } => {
            let mut mapped = ArenaVec::new_in(scratch);
            mapped.extend(
                body.switch_cases(cases)
                    .map(|(bits, call)| (bits, edge(call))),
            );
            Term::Switch {
                value:   map(value),
                default: edge(default),
                cases:   mapped,
            }
        },
        | InstData::Return { value } => {
            let mut args = ArenaVec::new_in(scratch);
            args.extend(value.map(map));
            Term::Jump(Edge {
                target: continuation,
                args,
            })
        },
        | InstData::Unreachable => Term::Unreachable,
        | _ => unreachable!("verified block ends in a terminator"),
    }
}

/// Original module call graph: SCC membership stays valid while calls are
/// expanded. Removing calls cannot introduce a new recursive component.
pub(super) struct CallGraph<'s> {
    pub(super) order: ArenaVec<'s, FuncId>,
    recursive:        ArenaVec<'s, bool>,
    unsafe_frame:     ArenaVec<'s, bool>,
}

impl<'s> CallGraph<'s> {
    /// Iterative Tarjan traversal: a DFS frame records the next outgoing
    /// edge, while a separate component stack holds unresolved SCC members.
    /// SCCs leave the stack in callee-before-caller order, including forward
    /// declarations and disconnected functions. No IR traversal recurses.
    pub(super) fn compute(module: &Module<'_>, scratch: &'s Bump) -> Self {
        let count = module.functions().count();
        let mut edges = ArenaVec::new_in(scratch);
        let mut nodes = ArenaVec::new_in(scratch);
        let mut unsafe_frame = ArenaVec::new_in(scratch);
        for (_, function) in module.functions() {
            let mut calls = ArenaVec::new_in(scratch);
            let mut unsafe_body = returns_twice(function.name);
            if let Some(body) = &function.body {
                for block in body.blocks() {
                    for &inst in body.block_insts(block) {
                        match *body.inst(inst) {
                            | InstData::Call { func, .. } => calls.push(func),
                            | InstData::FuncAddr { func } =>
                                unsafe_body |= returns_twice(module.function(func).name),
                            | _ => {},
                        }
                        unsafe_body |= matches!(
                            body.inst(inst).opcode(),
                            Opcode::VaStart
                                | Opcode::DynAlloca
                                | Opcode::StackSave
                                | Opcode::StackRestore
                        );
                    }
                }
            }
            edges.push(calls);
            nodes.push(Node {
                index:    None,
                low:      0,
                on_stack: false,
            });
            unsafe_frame.push(unsafe_body);
        }
        let mut recursive = ArenaVec::new_in(scratch);
        recursive.resize(count, false);
        let mut order = ArenaVec::new_in(scratch);
        let mut dfs: ArenaVec<'s, Frame> = ArenaVec::new_in(scratch);
        let mut component = ArenaVec::new_in(scratch);
        let mut members = ArenaVec::new_in(scratch);
        let mut next_index = 0;
        for (root, _) in module.functions() {
            if nodes[root.index()].index.is_some() {
                continue;
            }
            enter(root, &mut nodes, &mut component, &mut dfs, &mut next_index);
            while let Some(frame) = dfs.last_mut() {
                let func = frame.func;
                if let Some(&callee) = edges[func.index()].get(frame.next_edge) {
                    frame.next_edge += 1;
                    if let Some(index) = nodes[callee.index()].index {
                        if nodes[callee.index()].on_stack {
                            nodes[func.index()].low = nodes[func.index()].low.min(index);
                        }
                    } else {
                        enter(
                            callee,
                            &mut nodes,
                            &mut component,
                            &mut dfs,
                            &mut next_index,
                        );
                    }
                    continue;
                }
                _ = dfs.pop();
                let node = nodes[func.index()];
                if node.index == Some(node.low) {
                    members.clear();
                    loop {
                        let member = component.pop().expect("the SCC root is on the stack");
                        nodes[member.index()].on_stack = false;
                        members.push(member);
                        if member == func {
                            break;
                        }
                    }
                    let cyclic = members.len() > 1 || edges[func.index()].contains(&func);
                    let unsafe_scc = members.iter().any(|f| {
                        unsafe_frame[f.index()]
                            || edges[f.index()]
                                .iter()
                                .any(|callee| unsafe_frame[callee.index()])
                    });
                    for &member in &members {
                        recursive[member.index()] = cyclic;
                        unsafe_frame[member.index()] = unsafe_scc;
                        if module.function(member).body.is_some() {
                            order.push(member);
                        }
                    }
                }
                if let Some(parent) = dfs.last() {
                    nodes[parent.func.index()].low = nodes[parent.func.index()].low.min(node.low);
                }
            }
        }
        Self {
            order,
            recursive,
            unsafe_frame,
        }
    }
}

#[derive(Clone, Copy)]
struct Node {
    index:    Option<usize>,
    low:      usize,
    on_stack: bool,
}
struct Frame {
    func:      FuncId,
    next_edge: usize,
}

fn enter<'s>(
    func: FuncId,
    nodes: &mut [Node],
    component: &mut ArenaVec<'s, FuncId>,
    dfs: &mut ArenaVec<'s, Frame>,
    next_index: &mut usize,
) {
    nodes[func.index()] = Node {
        index:    Some(*next_index),
        low:      *next_index,
        on_stack: true,
    };
    *next_index += 1;
    component.push(func);
    dfs.push(Frame { func, next_edge: 0 });
}

/// The IR currently has no returns-twice attribute; lowering rejects these
/// builtins. Guard textual IR too, including wrappers and address constants.
fn returns_twice(name: &str) -> bool {
    matches!(
        name,
        "setjmp" | "_setjmp" | "sigsetjmp" | "__sigsetjmp" | "vfork"
    )
}
