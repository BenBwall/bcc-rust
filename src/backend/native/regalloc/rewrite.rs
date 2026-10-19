//! Rewrites a function with its allocation: virtual registers become the
//! locations their pieces have at each instruction, and the allocation's
//! moves go in between.
//!
//! The moves at one place form a parallel copy, sequentialized so that no
//! move overwrites a location another still reads; a cycle goes through a
//! scratch register. A piece that lives in its spill slot and still has
//! uses (the scan's fallback) is loaded into a scratch register of its
//! class just before the instruction that uses it and stored from it just
//! after one that defines it, and a copy to or from such a piece becomes a
//! single store or load. No instruction names more than two virtual
//! registers of one class, so two scratch registers per class suffice; all
//! these moves leave the flags alone.
//!
//! Copies between a register and itself stay until the checker has seen
//! them, then go; `edges.rs` then threads away the blocks left holding
//! only a jump.

use super::{
    Allocation,
    Edit,
    Location,
    checker::{
        self,
        CheckError,
        INSERTED,
    },
    edges,
};
use crate::{
    backend::native::{
        conv::{
            SCRATCH,
            SCRATCH_XMM,
        },
        mir::{
            Base,
            FrameObject,
            MFunction,
            MInst,
            Mem,
            Operand,
            PReg,
            Reg,
            RegClass,
            Role,
            VReg,
            Width,
        },
    },
    ir::Entity,
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// Replaces every virtual register of `func` by its location, inserts the
/// allocation's moves, deletes the copies between a register and itself
/// and threads the jumps left alone in their blocks. Debug builds first
/// prove the rewritten function with the symbolic checker (`checker.rs`).
pub(crate) fn rewrite<'a>(func: &mut MFunction<'a>, allocation: &Allocation<'_>, arena: &'a Bump) {
    if cfg!(debug_assertions) {
        if let Err(error) = rewrite_checked(func, allocation, arena) {
            panic!("the register allocation checker rejects the function: {error}");
        }
    } else {
        rewrite_blocks(func, allocation, None, arena);
    }
    remove_identity_copies(func);
    edges::thread_jumps(func, arena);
}

/// Rewrites `func` as [`rewrite`] does, before deleting identity copies,
/// and checks the result against the virtual-register program.
pub(crate) fn rewrite_checked<'a>(
    func: &mut MFunction<'_>,
    allocation: &Allocation<'_>,
    arena: &'a Bump,
) -> Result<(), &'a CheckError> {
    let mut original = ArenaVec::with_capacity_in(func.blocks.len(), arena);
    for block in &func.blocks {
        original.push(&*arena.alloc_slice_copy(&block.insts));
    }
    let mut origins = ArenaVec::with_capacity_in(func.blocks.len(), arena);
    rewrite_blocks(func, allocation, Some(&mut origins), arena);
    let mut views = ArenaVec::with_capacity_in(origins.len(), arena);
    views.extend(origins.iter().map(|origins| &origins[..]));
    checker::check(func, &original, &views, arena)
}

/// Rewrites every block; with `origins`, records for each block which
/// program instruction each rewritten one stands for.
fn rewrite_blocks<'a>(
    func: &mut MFunction<'_>,
    allocation: &Allocation<'_>,
    mut origins: Option<&mut ArenaVec<'a, ArenaVec<'a, u32>>>,
    arena: &'a Bump,
) {
    let classes = &func.classes;
    let mut edits = &allocation.edits[..];
    let mut moves = ArenaVec::new_in(arena);
    let mut position = 0_u32;
    for (block, data) in func.blocks.iter_mut().enumerate() {
        let mut out = ArenaVec::with_capacity_in(data.insts.len(), arena);
        let mut block_origins = ArenaVec::new_in(arena);
        for (index, &inst) in data.insts.iter().enumerate() {
            while let Some(edit) = edits.first()
                && (edit.block as usize, edit.index as usize) == (block, index)
            {
                let group = edits
                    .iter()
                    .take_while(|other| {
                        (other.block, other.index, other.phase)
                            == (edit.block, edit.index, edit.phase)
                    })
                    .count();
                moves.clear();
                for &Edit { vreg, src, dst, .. } in &edits[..group] {
                    moves.push(Move {
                        dst:   allocation.location(vreg, dst),
                        src:   allocation.location(vreg, src),
                        class: classes[vreg.index()],
                    });
                }
                edits = &edits[group..];
                let start = out.len();
                parallel_copy(&mut moves, &mut out);
                block_origins.extend((start..out.len()).map(|_| INSERTED));
            }
            let start = out.len();
            let program = rewrite_inst(inst, position, allocation, classes, &mut out);
            if origins.is_some() {
                let origin = u32::try_from(index).expect("instruction numbers fit u32");
                // The spill code around the program's instruction is the
                // allocator's.
                block_origins.extend(
                    (start..out.len()).map(|at| if at == program { origin } else { INSERTED }),
                );
            }
            position += 2;
        }
        data.insts.clear();
        data.insts.extend_from_slice(&out);
        if let Some(origins) = origins.as_deref_mut() {
            origins.push(block_origins);
        }
    }
    debug_assert!(edits.is_empty(), "every move has its place");
}

/// One move of a parallel copy.
#[derive(Clone, Copy)]
struct Move {
    dst:   Location,
    src:   Location,
    class: RegClass,
}

/// Emits the parallel copy `moves` as a sequence: a move goes once no
/// other still reads its destination; when only cycles remain, one source
/// is saved in the scratch register of its class and read from there.
/// Spill slots are never in a cycle, since a register's pieces share one
/// slot and a parallel copy moves each register once.
fn parallel_copy(moves: &mut ArenaVec<'_, Move>, out: &mut ArenaVec<'_, MInst>) {
    moves.retain(|copy| copy.dst != copy.src);
    while !moves.is_empty() {
        let ready =
            (0..moves.len()).find(|&index| moves.iter().all(|other| other.src != moves[index].dst));
        if let Some(index) = ready {
            let copy = moves.swap_remove(index);
            out.push(move_inst(copy.dst, copy.src));
            continue;
        }
        let saved = moves[0];
        let scratch = Location::Reg(match saved.class {
            | RegClass::Gpr => SCRATCH[0],
            | RegClass::Xmm => SCRATCH_XMM[0],
        });
        out.push(move_inst(scratch, saved.src));
        for copy in moves.iter_mut() {
            if copy.src == saved.src {
                copy.src = scratch;
            }
        }
    }
}

/// A whole-register move between two locations, one of them a register.
fn move_inst(dst: Location, src: Location) -> MInst {
    match (dst, src) {
        | (Location::Reg(dst), Location::Reg(src)) => match dst.class() {
            | RegClass::Gpr => MInst::Mov {
                width: Width::B64,
                dst:   Reg::Phys(dst),
                src:   Operand::Reg(Reg::Phys(src)),
            },
            | RegClass::Xmm => MInst::XMov {
                width: Width::B64,
                dst:   Reg::Phys(dst),
                src:   Operand::Reg(Reg::Phys(src)),
            },
        },
        | (Location::Reg(dst), Location::Spill(slot)) => load(dst, spill_slot(slot)),
        | (Location::Spill(slot), Location::Reg(src)) => store(spill_slot(slot), src),
        | _ => unreachable!("a move from {src:?} to {dst:?}"),
    }
}

/// A scratch register in use for one instruction: the spilled register it
/// stands for, and whether the instruction reads and writes it.
#[derive(Clone, Copy)]
struct Scratch {
    owner:  Option<VReg>,
    reads:  bool,
    writes: bool,
}

/// Appends the rewritten form of the instruction at `position` to `out`,
/// with its spill code; returns where the instruction itself went.
fn rewrite_inst(
    inst: MInst,
    position: u32,
    allocation: &Allocation<'_>,
    classes: &[RegClass],
    out: &mut ArenaVec<'_, MInst>,
) -> usize {
    let location = |vreg: VReg, role: Role| {
        let at = if role == Role::Def {
            position + 1
        } else {
            position
        };
        allocation.location(vreg, at)
    };
    if let Some(copy) = copy_through_memory(inst, &location) {
        out.push(copy);
        return out.len() - 1;
    }
    let mut inst = inst;
    // General scratch registers first, then SSE ones.
    let mut scratch = [Scratch {
        owner:  None,
        reads:  false,
        writes: false,
    }; 4];
    let mut slots = [0_u32; 4];
    inst.visit_registers(|reg, role| {
        let Reg::Virt(vreg) = *reg else { return };
        match location(vreg, role) {
            | Location::Reg(register) => *reg = Reg::Phys(register),
            | Location::Spill(slot) => {
                let base = match classes[vreg.index()] {
                    | RegClass::Gpr => 0,
                    | RegClass::Xmm => 2,
                };
                let entries = &scratch[base..base + 2];
                let index = base
                    + entries
                        .iter()
                        .position(|entry| entry.owner == Some(vreg))
                        .or_else(|| entries.iter().position(|entry| entry.owner.is_none()))
                        .expect("an instruction names at most two registers of a class");
                let entry = &mut scratch[index];
                entry.owner = Some(vreg);
                entry.reads |= role.reads();
                entry.writes |= role.writes();
                slots[index] = slot;
                *reg = Reg::Phys(scratch_register(index));
            },
            | Location::None => unreachable!("{vreg:?} occurs, so it is live"),
        }
    });
    for (index, entry) in scratch.iter().enumerate() {
        if entry.owner.is_some() && entry.reads {
            out.push(load(scratch_register(index), spill_slot(slots[index])));
        }
    }
    let program = out.len();
    out.push(inst);
    for (index, entry) in scratch.iter().enumerate() {
        if entry.owner.is_some() && entry.writes {
            out.push(store(spill_slot(slots[index]), scratch_register(index)));
        }
    }
    program
}

/// A copy that has exactly one side in a spill slot, as a single load or
/// store.
fn copy_through_memory(inst: MInst, location: &impl Fn(VReg, Role) -> Location) -> Option<MInst> {
    let (MInst::Mov {
        width: Width::B64,
        dst,
        src: Operand::Reg(src),
    }
    | MInst::XMov {
        width: Width::B64,
        dst,
        src: Operand::Reg(src),
    }) = inst
    else {
        return None;
    };
    let place = |reg: Reg, role: Role| match reg {
        | Reg::Virt(vreg) => location(vreg, role),
        | Reg::Phys(register) => Location::Reg(register),
    };
    let (dst, src) = (place(dst, Role::Def), place(src, Role::Use));
    match (dst, src) {
        | (Location::Spill(_), Location::Reg(_)) | (Location::Reg(_), Location::Spill(_)) =>
            Some(move_inst(dst, src)),
        | _ => None,
    }
}

/// Deletes the 64-bit copies between a register and itself.
fn remove_identity_copies(func: &mut MFunction<'_>) {
    for block in &mut func.blocks {
        block.insts.retain(|inst| {
            !matches!(
                *inst,
                MInst::Mov {
                    width: Width::B64,
                    dst,
                    src: Operand::Reg(src),
                }
                | MInst::XMov {
                    width: Width::B64,
                    dst,
                    src: Operand::Reg(src),
                } if dst == src
            )
        });
    }
}

/// The scratch register of a scratch entry.
const fn scratch_register(index: usize) -> PReg {
    if index < 2 {
        SCRATCH[index]
    } else {
        SCRATCH_XMM[index - 2]
    }
}

/// A load of a whole spill slot into `register`.
const fn load(register: PReg, slot: Mem) -> MInst {
    match register.class() {
        | RegClass::Gpr => MInst::Mov {
            width: Width::B64,
            dst:   Reg::Phys(register),
            src:   Operand::Mem(slot),
        },
        | RegClass::Xmm => MInst::XMov {
            width: Width::B64,
            dst:   Reg::Phys(register),
            src:   Operand::Mem(slot),
        },
    }
}

/// A store of `register` into a whole spill slot.
const fn store(slot: Mem, register: PReg) -> MInst {
    match register.class() {
        | RegClass::Gpr => MInst::Store {
            width: Width::B64,
            dst:   slot,
            src:   Operand::Reg(Reg::Phys(register)),
        },
        | RegClass::Xmm => MInst::XStore {
            width: Width::B64,
            dst:   slot,
            src:   Reg::Phys(register),
        },
    }
}

/// The memory operand of a spill slot.
const fn spill_slot(slot: u32) -> Mem {
    Mem::new(Base::Frame(FrameObject::Spill(slot)), 0)
}
