//! Rewrites a function with its allocation: virtual registers become their
//! physical registers, copies between one register and itself disappear,
//! and spilled values move through the scratch registers.
//!
//! A spilled value used by an instruction is loaded into a scratch register
//! just before it, and one the instruction defines is stored from the
//! scratch register just after it. A copy to or from a spilled value
//! becomes a single store or load instead. No instruction names more than
//! two virtual registers, so the two scratch registers suffice; the loads
//! and stores are `mov`s, which leave the flags alone.

use super::{
    Allocation,
    Location,
};
use crate::{
    backend::native::{
        conv::SCRATCH,
        mir::{
            Base,
            FrameObject,
            MFunction,
            MInst,
            Mem,
            Operand,
            PReg,
            Reg,
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
    for block in &mut func.blocks {
        let mut out = ArenaVec::with_capacity_in(block.insts.len(), arena);
        for &inst in &block.insts {
            rewrite_inst(inst, allocation, &mut out);
        }
        block.insts.clear();
        block.insts.extend_from_slice(&out);
    }
}

/// Appends the rewritten form of one instruction to `out`.
fn rewrite_inst(inst: MInst, allocation: &Allocation<'_>, out: &mut ArenaVec<'_, MInst>) {
    let spill = |vreg: VReg| match allocation.locations[vreg.index()] {
        | Location::Spill(slot) => Some(spill_slot(slot)),
        | _ => None,
    };
    // Copies touching a spilled value become one load or store.
    if let MInst::Mov {
        width: Width::B64,
        dst,
        src: Operand::Reg(src),
    } = inst
    {
        let spilled = |reg: Reg| match reg {
            | Reg::Virt(vreg) => spill(vreg),
            | Reg::Phys(_) => None,
        };
        match (spilled(dst), spilled(src)) {
            | (Some(slot), None) => {
                out.push(MInst::Store {
                    width: Width::B64,
                    dst:   slot,
                    src:   Operand::Reg(Reg::Phys(physical(src, allocation))),
                });
                return;
            },
            | (None, Some(slot)) => {
                out.push(MInst::Mov {
                    width: Width::B64,
                    dst:   Reg::Phys(physical(dst, allocation)),
                    src:   Operand::Mem(slot),
                });
                return;
            },
            | _ => {},
        }
    }
    let mut inst = inst;
    let mut scratch: [(Option<VReg>, bool, bool); 2] = [(None, false, false); 2];
    inst.visit_registers(|reg, role| {
        let Reg::Virt(vreg) = *reg else { return };
        match allocation.locations[vreg.index()] {
            | Location::Reg(register) => *reg = Reg::Phys(register),
            | Location::Spill(_) => {
                let index = scratch
                    .iter()
                    .position(|&(owner, ..)| owner == Some(vreg))
                    .or_else(|| scratch.iter().position(|&(owner, ..)| owner.is_none()))
                    .expect("an instruction names at most two virtual registers");
                let entry = &mut scratch[index];
                entry.0 = Some(vreg);
                entry.1 |= role.reads();
                entry.2 |= role.writes();
                *reg = Reg::Phys(SCRATCH[index]);
            },
            | Location::None => unreachable!("{vreg:?} occurs, so it has an interval"),
        }
    });
    for (index, &(owner, reads, _)) in scratch.iter().enumerate() {
        if let Some(vreg) = owner
            && reads
        {
            out.push(MInst::Mov {
                width: Width::B64,
                dst:   Reg::Phys(SCRATCH[index]),
                src:   Operand::Mem(spill(vreg).expect("a scratch owner is spilled")),
            });
        }
    }
    let identity = matches!(
        inst,
        MInst::Mov {
            width: Width::B64,
            dst,
            src: Operand::Reg(src),
        } if dst == src
    );
    if !identity {
        out.push(inst);
    }
    for (index, &(owner, _, writes)) in scratch.iter().enumerate() {
        if let Some(vreg) = owner
            && writes
        {
            out.push(MInst::Store {
                width: Width::B64,
                dst:   spill(vreg).expect("a scratch owner is spilled"),
                src:   Operand::Reg(Reg::Phys(SCRATCH[index])),
            });
        }
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
