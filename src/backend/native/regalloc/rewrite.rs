//! Rewrites a function with its allocation: virtual registers become the
//! locations their pieces have at each instruction, and the allocation's
//! moves go in between.
//!
//! The moves at one place form a parallel copy, sequentialized so that no
//! move overwrites a location another still reads; a cycle goes through a
//! scratch register. A value in its spill slot that an instruction can read
//! as a memory source operand is read there, and a copy to or from it
//! becomes a single store or load; a copy between two values that coalescing
//! joined, made while their slot holds them, becomes nothing. Other uses of a
//! spilled piece (the scan's fallback) load it into a scratch register of its
//! class just before the instruction and store it from there just after. An
//! instruction reads all its operands before it writes any, so a value it only
//! defines shares a scratch register with one it only reads. No instruction
//! reads more than two virtual registers of one class except one with an
//! indexed memory operand and a third general register, such as an indexed
//! store, whose base, index and value may all be spilled (values live into the
//! target of an indirect jump are never split); its address then goes into
//! the first scratch register with a `lea`, which the instruction reads
//! instead, so two scratch registers per class always suffice. All these
//! moves and the `lea` leave the flags alone.
//!
//! Copies between a location and itself stay until the checker has seen
//! them, then go (one within a spill slot is a placeholder `Store` from the
//! slot to itself, which x86 cannot encode); `edges.rs` then threads away the
//! blocks left holding only a jump.

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
    peephole,
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
            Index,
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
/// allocation's moves, deletes the copies between a register and itself,
/// combines copies and additions into `lea`s, threads the jumps left alone
/// in their blocks and copies short tests over the jumps to them. Debug
/// builds first prove the rewritten function with the symbolic checker
/// (`checker.rs`).
pub(crate) fn rewrite<'a>(func: &mut MFunction<'a>, allocation: &Allocation<'_>, arena: &'a Bump) {
    if cfg!(debug_assertions) {
        if let Err(error) = rewrite_checked(func, allocation, arena) {
            panic!("the register allocation checker rejects the function: {error}");
        }
    } else {
        rewrite_blocks(func, allocation, None, arena);
    }
    remove_identity_copies(func);
    peephole::combine(func);
    edges::thread_jumps(func, arena);
    edges::duplicate_tests(func, arena);
    // A test every jump to which was copied is left unreachable.
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

/// A spilled register an instruction names, the scratch register that
/// stands for it, and whether the instruction reads and writes it.
#[derive(Clone, Copy)]
struct Scratch {
    vreg:     VReg,
    class:    RegClass,
    slot:     u32,
    register: Option<PReg>,
    reads:    bool,
    writes:   bool,
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
    // A spilled value the instruction can read from memory stays there.
    if let Some(vreg) = inst.memory_source_vreg()
        && let Location::Spill(slot) = location(vreg, Role::Use)
        && let Some(operand) = inst.memory_source()
    {
        *operand = Operand::Mem(spill_slot(slot));
    }
    // The spilled registers the instruction names, each once.
    let mut spilled = [Scratch {
        vreg:     VReg::new(0),
        class:    RegClass::Gpr,
        slot:     0,
        register: None,
        reads:    false,
        writes:   false,
    }; MAX_SPILLED];
    let mut count = 0;
    inst.visit_registers(|reg, role| {
        let Reg::Virt(vreg) = *reg else { return };
        match location(vreg, role) {
            | Location::Reg(_) => {},
            | Location::Spill(slot) => {
                let index = spilled[..count]
                    .iter()
                    .position(|entry| entry.vreg == vreg)
                    .unwrap_or_else(|| {
                        count += 1;
                        count - 1
                    });
                let entry = &mut spilled[index];
                entry.vreg = vreg;
                entry.class = classes[vreg.index()];
                entry.slot = slot;
                entry.reads |= role.reads();
                entry.writes |= role.writes();
            },
            | Location::None => unreachable!("{vreg:?} occurs, so it is live"),
        }
    });
    let count = fold_address(&mut inst, &mut spilled[..count], out);
    let spilled = &mut spilled[..count];
    for (class, scratch) in [(RegClass::Gpr, SCRATCH), (RegClass::Xmm, SCRATCH_XMM)] {
        assign(spilled, class, scratch);
    }
    inst.visit_registers(|reg, role| {
        let Reg::Virt(vreg) = *reg else { return };
        *reg = Reg::Phys(match location(vreg, role) {
            | Location::Reg(register) => register,
            | _ => spilled
                .iter()
                .find(|entry| entry.vreg == vreg)
                .and_then(|entry| entry.register)
                .expect("every spilled register has a scratch register"),
        });
    });
    for entry in spilled.iter() {
        if entry.reads {
            let register = entry.register.expect("assigned above");
            out.push(load(register, spill_slot(entry.slot)));
        }
    }
    let program = out.len();
    out.push(inst);
    for entry in spilled.iter() {
        if entry.writes {
            let register = entry.register.expect("assigned above");
            out.push(store(spill_slot(entry.slot), register));
        }
    }
    program
}

/// More distinct virtual registers than any instruction names: an indexed
/// store names three.
const MAX_SPILLED: usize = 4;

/// Makes room when `inst` reads three spilled general registers, the most
/// any instruction names: the base and index of an indexed address and one
/// more (the value an indexed store stores). The address goes into the first
/// scratch register with a `lea`, which leaves the flags alone, and the
/// instruction reads its memory through that register, which leaves the second
/// scratch register for the third value. Returns how many of `spilled` the
/// instruction still names: the base's entry stays, holding the address and
/// loaded already, and the index's goes.
fn fold_address(inst: &mut MInst, spilled: &mut [Scratch], out: &mut ArenaVec<'_, MInst>) -> usize {
    let reads = spilled
        .iter()
        .filter(|entry| entry.class == RegClass::Gpr && entry.reads)
        .count();
    if reads <= 2 {
        return spilled.len();
    }
    let mem = inst
        .memory_operand()
        .expect("only an indexed address adds a third general register");
    let (
        Base::Reg(Reg::Virt(base)),
        Some(Index {
            reg: Reg::Virt(index),
            scale,
        }),
    ) = (mem.base, mem.index)
    else {
        unreachable!("three spilled reads include an address's base and index: {mem:?}");
    };
    let [first, second] = SCRATCH;
    let find = |vreg: VReg| {
        spilled
            .iter()
            .position(|entry| entry.vreg == vreg)
            .expect("a spilled read")
    };
    let (base_at, index_at) = (find(base), find(index));
    out.push(load(first, spill_slot(spilled[base_at].slot)));
    out.push(load(second, spill_slot(spilled[index_at].slot)));
    out.push(MInst::Lea {
        dst:  Reg::Phys(first),
        addr: Mem {
            base:  Base::Reg(Reg::Phys(first)),
            index: Some(Index {
                reg: Reg::Phys(second),
                scale,
            }),
            disp:  0,
        },
    });
    *mem = Mem::new(Base::Reg(Reg::Phys(first)), mem.disp);
    spilled[base_at].register = Some(first);
    spilled[base_at].reads = false;
    let last = spilled.len() - 1;
    spilled.swap(index_at, last);
    last
}

/// Gives each spilled register of `class` one of the class's two scratch
/// registers: the ones the instruction reads first, each its own, then the
/// ones it only writes, each sharing a register with a read-only one or
/// taking a free one. An instruction reads all its operands before it writes
/// any, so a value it defines may land where one it only read was.
fn assign(spilled: &mut [Scratch], class: RegClass, scratch: [PReg; 2]) {
    for reading in [true, false] {
        for index in 0..spilled.len() {
            let entry = spilled[index];
            if entry.register.is_some() || entry.reads != reading || entry.class != class {
                continue;
            }
            let taken = |register: PReg| {
                spilled
                    .iter()
                    .any(|other| other.register == Some(register) && (reading || other.writes))
            };
            let register = scratch
                .into_iter()
                .find(|&register| !taken(register))
                .expect("an instruction reads at most two spilled registers of a class");
            spilled[index].register = Some(register);
        }
    }
}

/// A copy that has exactly one side in a spill slot, as a single load or
/// store, or both in one slot, as the placeholder [`identity_copy`].
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
        | (Location::Spill(dst), Location::Spill(src)) if dst == src =>
            Some(identity_copy(spill_slot(dst))),
        | _ => None,
    }
}

/// Deletes the 64-bit copies between a register and itself and the
/// placeholders of copies within a spill slot.
fn remove_identity_copies(func: &mut MFunction<'_>) {
    for block in &mut func.blocks {
        block.insts.retain(|inst| match *inst {
            | MInst::Mov {
                width: Width::B64,
                dst,
                src: Operand::Reg(src),
            }
            | MInst::XMov {
                width: Width::B64,
                dst,
                src: Operand::Reg(src),
            } => dst != src,
            | MInst::Store {
                width: Width::B64,
                dst,
                src: Operand::Mem(src),
            } => dst != src,
            | _ => true,
        });
    }
}

/// The placeholder of a copy between two values in one spill slot: a store
/// of the slot to itself, which the checker reads as the copy and which is
/// deleted with the identity copies, before anything emits it.
const fn identity_copy(slot: Mem) -> MInst {
    MInst::Store {
        width: Width::B64,
        dst:   slot,
        src:   Operand::Mem(slot),
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
