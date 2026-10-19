//! Control-flow surgery around allocation: critical edges are split before
//! it, so every edge has a place for its moves, and blocks that only jump
//! are threaded away after it.
//!
//! An edge is critical when its source has several successors and its
//! target several predecessors; a move on it could go neither at the
//! source's end nor at the target's start. The selector already split the
//! edges that pass block arguments; this splits the rest of the
//! conditional ones, placing each new block right after its source, the
//! fall-through target first. An indirect jump's edges cannot be split,
//! since an address names the target; the allocator keeps every value live
//! across them in one place instead.
//!
//! After the rewrite, most of these blocks (and the selector's edge blocks
//! whose copies coalescing removed) hold nothing but a `jmp`. Branches to
//! them go straight to their target, and blocks no branch reaches any more
//! are dropped, keeping the layout of the rest. Then a `jmp` to a short
//! block that ends in a conditional branch, typically a loop's back edge
//! to its test, takes a copy of that block instead.

use crate::{
    backend::native::mir::{
        MBlock,
        MBlockData,
        MFunction,
        MInst,
    },
    ir::Entity,
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// Splits the conditional branches' critical edges of `func`.
pub(super) fn split_critical_edges<'a>(func: &mut MFunction<'a>, arena: &'a Bump) {
    let blocks = func.blocks.len();
    let mut predecessors = ArenaVec::with_capacity_in(blocks, arena);
    predecessors.resize(blocks, 0_u32);
    // The entry is also entered from the caller.
    predecessors[0] = 1;
    for block in 0..blocks {
        for successor in func.successors(MBlock::new(block)) {
            predecessors[successor.index()] += 1;
        }
    }
    let mut inserted = ArenaVec::new_in(arena);
    for block in 0..blocks {
        let Some(&MInst::Jcc {
            cond,
            taken,
            not_taken,
        }) = func.blocks[block].insts.last()
        else {
            continue;
        };
        let mut split = |target: MBlock, func: &mut MFunction<'a>| {
            if predecessors[target.index()] < 2 {
                return target;
            }
            let edge = func.block(arena);
            func.blocks[edge.index()].insts.push(MInst::Jmp { target });
            inserted.push((block, edge));
            edge
        };
        let not_taken = split(not_taken, func);
        let taken = split(taken, func);
        *func.blocks[block]
            .insts
            .last_mut()
            .expect("the block ends in the branch") = MInst::Jcc {
            cond,
            taken,
            not_taken,
        };
    }
    if inserted.is_empty() {
        return;
    }
    let mut order = ArenaVec::with_capacity_in(func.blocks.len(), arena);
    let mut next = 0;
    for block in 0..blocks {
        order.push(block);
        while next < inserted.len() && inserted[next].0 == block {
            order.push(inserted[next].1.index());
            next += 1;
        }
    }
    relayout(func, &order, arena);
}

/// Sends branches to blocks that hold only a `jmp` straight to its target
/// and drops the blocks nothing reaches any more. Blocks whose address is
/// taken stay.
pub(super) fn thread_jumps<'a>(func: &mut MFunction<'a>, arena: &'a Bump) {
    let blocks = func.blocks.len();
    let mut addressed = ArenaVec::with_capacity_in(blocks, arena);
    addressed.resize(blocks, false);
    addressed[0] = true;
    for &(_, block) in &func.labels {
        addressed[block.index()] = true;
    }
    for targets in &func.indirect {
        for &block in targets {
            addressed[block.index()] = true;
        }
    }
    let mut forward = ArenaVec::with_capacity_in(blocks, arena);
    forward.extend((0..blocks).map(MBlock::new));
    for block in 0..blocks {
        if let [MInst::Jmp { target }] = func.blocks[block].insts[..]
            && !addressed[block]
        {
            forward[block] = target;
        }
    }
    // Follow chains; a cycle of such blocks is an endless loop and stays.
    let destination = |block: MBlock| {
        let mut current = block;
        for _ in 0..blocks {
            let next = forward[current.index()];
            if next == current {
                return current;
            }
            current = next;
        }
        block
    };
    for data in &mut func.blocks {
        let Some(last) = data.insts.last_mut() else {
            continue;
        };
        match *last {
            | MInst::Jmp { target } =>
                *last = MInst::Jmp {
                    target: destination(target),
                },
            | MInst::Jcc {
                cond,
                taken,
                not_taken,
            } => {
                let (taken, not_taken) = (destination(taken), destination(not_taken));
                *last = if taken == not_taken {
                    MInst::Jmp { target: taken }
                } else {
                    MInst::Jcc {
                        cond,
                        taken,
                        not_taken,
                    }
                };
            },
            | _ => {},
        }
    }
    let mut reached = addressed;
    let mut work = ArenaVec::new_in(arena);
    work.extend((0..blocks).filter(|&block| reached[block]));
    while let Some(block) = work.pop() {
        for successor in func.successors(MBlock::new(block)) {
            if !reached[successor.index()] {
                reached[successor.index()] = true;
                work.push(successor.index());
            }
        }
    }
    if reached.iter().all(|&reached| reached) {
        return;
    }
    let mut order = ArenaVec::with_capacity_in(blocks, arena);
    order.extend((0..blocks).filter(|&block| reached[block]));
    relayout(func, &order, arena);
}

/// The most instructions a block may have to be copied over a `jmp` to it.
const DUPLICATED: usize = 8;

/// Replaces each `jmp` to a short block that ends in a conditional branch
/// by a copy of that block, unless the jump falls through to it anyway. A
/// loop whose test is at the top then branches back from its bottom on the
/// copied test instead of jumping to the top and testing there (loop
/// rotation), and an `if` that joins before another test branches on it
/// directly. The copied instructions use allocated registers only, so the
/// copy computes what the jump's target did; blocks with calls stay.
pub(super) fn duplicate_tests<'a>(func: &mut MFunction<'a>, arena: &'a Bump) {
    let mut copy = ArenaVec::new_in(arena);
    for block in 0..func.blocks.len() {
        let Some(&MInst::Jmp { target }) = func.blocks[block].insts.last() else {
            continue;
        };
        let insts = &func.blocks[target.index()].insts;
        if target.index() == block + 1
            || target.index() == block
            || insts.len() > DUPLICATED
            || !matches!(insts.last(), Some(MInst::Jcc { .. }))
            || insts.iter().any(|inst| matches!(inst, MInst::Call { .. }))
        {
            continue;
        }
        copy.clear();
        copy.extend_from_slice(insts);
        let data = &mut func.blocks[block].insts;
        _ = data.pop();
        data.extend_from_slice(&copy);
    }
}

/// Puts the blocks in `order` (old indices; blocks left out must be
/// unreachable) and renumbers every reference to them.
fn relayout<'a>(func: &mut MFunction<'a>, order: &[usize], arena: &'a Bump) {
    let mut renumber = ArenaVec::with_capacity_in(func.blocks.len(), arena);
    renumber.resize(func.blocks.len(), None);
    for (new, &old) in order.iter().enumerate() {
        renumber[old] = Some(MBlock::new(new));
    }
    let map = |block: MBlock| renumber[block.index()].expect("a kept block names only kept blocks");
    let mut blocks = ArenaVec::with_capacity_in(order.len(), arena);
    for &old in order {
        let insts = std::mem::replace(&mut func.blocks[old].insts, ArenaVec::new_in(arena));
        blocks.push(MBlockData { insts });
    }
    for data in &mut blocks {
        if let Some(last) = data.insts.last_mut() {
            match last {
                | MInst::Jmp { target } => *target = map(*target),
                | MInst::Jcc {
                    taken, not_taken, ..
                } => {
                    *taken = map(*taken);
                    *not_taken = map(*not_taken);
                },
                | _ => {},
            }
        }
    }
    func.blocks = blocks;
    for targets in &mut func.indirect {
        for block in targets.iter_mut() {
            *block = map(*block);
        }
    }
    for (_, block) in &mut func.labels {
        *block = map(*block);
    }
}
