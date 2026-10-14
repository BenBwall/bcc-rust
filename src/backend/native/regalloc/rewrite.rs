//! Rewrites a function with its allocation: virtual registers become their
//! physical registers, copies between one register and itself disappear,
//! and spilled values move through the scratch registers.
//!
//! A spilled value used by an instruction is loaded into a scratch register
//! of its class just before it, and one the instruction defines is stored
//! from the scratch register just after it. A copy to or from a spilled
//! value becomes a single store or load instead. No instruction names more
//! than two virtual registers of one class, so two scratch registers per
//! class suffice; the loads and stores are moves, which leave the flags
//! alone.

use super::{
    Allocation,
    Location,
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

/// Replaces every virtual register of `func` by its location.
pub(crate) fn rewrite(func: &mut MFunction<'_>, allocation: &Allocation<'_>, arena: &Bump) {
    let classes = &func.classes;
    for block in &mut func.blocks {
        let mut out = ArenaVec::with_capacity_in(block.insts.len(), arena);
        for &inst in &block.insts {
            rewrite_inst(inst, allocation, classes, &mut out);
        }
        block.insts.clear();
        block.insts.extend_from_slice(&out);
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

/// Appends the rewritten form of one instruction to `out`.
fn rewrite_inst(
    inst: MInst,
    allocation: &Allocation<'_>,
    classes: &[RegClass],
    out: &mut ArenaVec<'_, MInst>,
) {
    let spill = |vreg: VReg| match allocation.locations[vreg.index()] {
        | Location::Spill(slot) => Some(spill_slot(slot)),
        | _ => None,
    };
    if copy_through_memory(inst, allocation, out) {
        return;
    }
    let mut inst = inst;
    // General scratch registers first, then SSE ones.
    let mut scratch = [Scratch {
        owner:  None,
        reads:  false,
        writes: false,
    }; 4];
    inst.visit_registers(|reg, role| {
        let Reg::Virt(vreg) = *reg else { return };
        match allocation.locations[vreg.index()] {
            | Location::Reg(register) => *reg = Reg::Phys(register),
            | Location::Spill(_) => {
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
                *reg = Reg::Phys(scratch_register(index));
            },
            | Location::None => unreachable!("{vreg:?} occurs, so it has an interval"),
        }
    });
    for (index, entry) in scratch.iter().enumerate() {
        if let Some(vreg) = entry.owner
            && entry.reads
        {
            let slot = spill(vreg).expect("a scratch owner is spilled");
            out.push(load(scratch_register(index), slot));
        }
    }
    let identity = match inst {
        | MInst::Mov {
            width: Width::B64,
            dst,
            src: Operand::Reg(src),
        }
        | MInst::XMov {
            width: Width::B64,
            dst,
            src: Operand::Reg(src),
        } => dst == src,
        | _ => false,
    };
    if !identity {
        out.push(inst);
    }
    for (index, entry) in scratch.iter().enumerate() {
        if let Some(vreg) = entry.owner
            && entry.writes
        {
            let slot = spill(vreg).expect("a scratch owner is spilled");
            out.push(store(slot, scratch_register(index)));
        }
    }
}

/// Rewrites a copy that has exactly one spilled side as a single load or
/// store; whether it did.
fn copy_through_memory(
    inst: MInst,
    allocation: &Allocation<'_>,
    out: &mut ArenaVec<'_, MInst>,
) -> bool {
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
        return false;
    };
    let spilled = |reg: Reg| match reg {
        | Reg::Virt(vreg) => match allocation.locations[vreg.index()] {
            | Location::Spill(slot) => Some(spill_slot(slot)),
            | _ => None,
        },
        | Reg::Phys(_) => None,
    };
    match (spilled(dst), spilled(src)) {
        | (Some(slot), None) => out.push(store(slot, physical(src, allocation))),
        | (None, Some(slot)) => out.push(load(physical(dst, allocation), slot)),
        | _ => return false,
    }
    true
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

/// The physical register of a register that is not spilled.
fn physical(reg: Reg, allocation: &Allocation<'_>) -> PReg {
    match reg {
        | Reg::Phys(register) => register,
        | Reg::Virt(vreg) => match allocation.locations[vreg.index()] {
            | Location::Reg(register) => register,
            | location => unreachable!("{vreg:?} is in {location:?}"),
        },
    }
}

/// The memory operand of a spill slot.
const fn spill_slot(slot: u32) -> Mem {
    Mem::new(Base::Frame(FrameObject::Spill(slot)), 0)
}
