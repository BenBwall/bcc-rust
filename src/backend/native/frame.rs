//! Frame layout: where the callee-saved registers, IR stack slots, spill
//! slots and outgoing arguments live, as offsets from `%rsp` after the
//! prologue (or from `%rbp`, which then holds that same address).
//!
//! ```text
//!  incoming stack arguments   CFA + home area (Win64: 32 bytes)
//!  Win64 home area            CFA .. CFA + 32
//!  return address             CFA - 8
//!  callee-saved pushes        8 bytes each (`%rbp` among them with a frame pointer)
//!  stack slots                each at its alignment (at most 16)
//!  register save area         System V variadic functions: 176 bytes
//!  Win64 SSE saves            16 bytes each for `%xmm6`-`%xmm15`
//!  spill slots                8 bytes each
//!  outgoing arguments         from 0(%rsp), after Win64's home area
//! ```
//!
//! A function whose `%rsp` moves at run time (`dyn_alloca`) keeps a frame
//! pointer: `%rbp` is set to `%rsp` right after the prologue's allocation, so
//! every fixed object keeps its offset from `%rbp`, while outgoing arguments
//! stay at the bottom of the current `%rsp`. When the function calls or has
//! a 16-byte-aligned object, the size subtracted after the pushes makes
//! `%rsp` a multiple of 16: at entry `%rsp + 8` is one (psABI §3.2.2;
//! Microsoft Learn, "x64 stack usage").

use super::{
    conv::CallConv,
    mir::{
        FrameObject,
        MFunction,
        MInst,
        PReg,
        PRegSet,
        Reg,
        RegClass,
    },
    regalloc::Allocation,
};
use crate::util::bump::{
    ArenaVec,
    Bump,
};

/// The bytes of the System V register save area: six general registers and
/// eight 16-byte SSE registers (psABI §3.5.7, Figure 3.33).
pub(crate) const REGISTER_SAVE_BYTES: u32 = 6 * 8 + 8 * 16;

/// A laid-out frame.
pub(crate) struct Frame<'a> {
    /// The callee-saved general registers the prologue pushes, in push
    /// order.
    pub(crate) saved:         ArenaVec<'a, PReg>,
    /// The callee-saved SSE registers (Win64) and their offsets.
    pub(crate) saved_xmm:     ArenaVec<'a, (PReg, u32)>,
    /// Bytes the prologue subtracts from `%rsp` after the pushes.
    pub(crate) size:          u32,
    /// Whether `%rbp` addresses the fixed frame.
    pub(crate) frame_pointer: bool,
    slots:                    ArenaVec<'a, u32>,
    spills:                   u32,
    outgoing_end:             u32,
    register_save:            u32,
    incoming:                 u32,
    home:                     u32,
}

impl<'a> Frame<'a> {
    /// Lays out the frame of an allocated and rewritten function.
    pub(crate) fn layout(
        func: &MFunction<'_>,
        allocation: &Allocation<'_>,
        conv: CallConv,
        arena: &'a Bump,
    ) -> Self {
        let written = written(func);
        let frame_pointer = func.dynamic;
        let mut saved = ArenaVec::new_in(arena);
        let mut xmm = ArenaVec::new_in(arena);
        for register in PReg::ALL {
            let save = conv.callee_saved.contains(register)
                && (written.contains(register) || (frame_pointer && register == PReg::Rbp));
            match (save, register.class()) {
                | (true, RegClass::Gpr) => saved.push(register),
                | (true, RegClass::Xmm) => xmm.push(register),
                | (false, _) => {},
            }
        }
        let mut aligned = func.calls || !xmm.is_empty() || func.register_save;
        // Dynamic allocations start right above the outgoing arguments and
        // keep 16-byte alignment.
        let outgoing_end = func.outgoing.next_multiple_of(16);
        let spills = outgoing_end;
        let mut end = spills + 8 * allocation.spill_slots;
        let mut saved_xmm = ArenaVec::with_capacity_in(xmm.len(), arena);
        for register in xmm {
            end = end.next_multiple_of(16);
            saved_xmm.push((register, end));
            end += 16;
        }
        let mut register_save = 0;
        if func.register_save {
            end = end.next_multiple_of(16);
            register_save = end;
            end += REGISTER_SAVE_BYTES;
        }
        let mut slots = ArenaVec::with_capacity_in(func.slots.len(), arena);
        for slot in &func.slots {
            let align = slot.align.max(1);
            aligned |= align >= 16;
            end = end.next_multiple_of(align);
            slots.push(end);
            end += slot.size;
        }
        let pushes = 8 * u32::try_from(saved.len()).expect("at most 16 registers");
        let mut size = end.next_multiple_of(8);
        if aligned && !(8 + pushes + size).is_multiple_of(16) {
            size += 8;
        }
        let incoming = size + pushes + 8 + conv.home_area;
        Self {
            saved,
            saved_xmm,
            size,
            frame_pointer,
            slots,
            spills,
            outgoing_end,
            register_save,
            incoming,
            home: incoming - conv.home_area,
        }
    }

    /// The register and offset that address a frame object: `%rsp` after
    /// the prologue, or `%rbp` for the fixed frame of a function whose stack
    /// moves.
    pub(crate) fn address(&self, object: FrameObject) -> (PReg, i64) {
        let offset = match object {
            | FrameObject::Slot(slot) => self.slots[slot as usize],
            | FrameObject::Incoming(offset) => self.incoming + offset,
            | FrameObject::Home(offset) => self.home + offset,
            | FrameObject::Outgoing(offset) => return (PReg::Rsp, i64::from(offset)),
            | FrameObject::OutgoingEnd => return (PReg::Rsp, i64::from(self.outgoing_end)),
            | FrameObject::Spill(slot) => self.spills + 8 * slot,
            | FrameObject::RegisterSave(offset) => self.register_save + offset,
        };
        let base = if self.frame_pointer {
            PReg::Rbp
        } else {
            PReg::Rsp
        };
        (base, i64::from(offset))
    }

    /// The distance from `%rsp` after the prologue to the canonical frame
    /// address (the caller's `%rsp` before the call).
    pub(crate) fn cfa_offset(&self) -> u32 {
        self.size + 8 * u32::try_from(self.saved.len()).expect("at most 16 registers") + 8
    }
}

/// Every physical register an instruction other than a call writes, after
/// rewriting: allocated registers, fixed ones and spill scratch registers.
fn written(func: &MFunction<'_>) -> PRegSet {
    let mut written = PRegSet::EMPTY;
    for block in &func.blocks {
        for inst in &block.insts {
            if matches!(inst, MInst::Call { .. }) {
                continue;
            }
            let mut inst = *inst;
            inst.visit_registers(|reg, role| {
                if let Reg::Phys(register) = *reg
                    && role.writes()
                {
                    written = written.with(register);
                }
            });
        }
    }
    written
}
