//! Frame layout: where the callee-saved registers, IR stack slots, spill
//! slots and outgoing arguments live, as offsets from `%rsp` after the
//! prologue.
//!
//! ```text
//!  incoming stack arguments   CFA + home area (Win64: 32 bytes)
//!  return address             CFA - 8
//!  callee-saved pushes        8 bytes each
//!  stack slots                each at its alignment (at most 16)
//!  spill slots                8 bytes each
//!  outgoing arguments         from 0(%rsp), after Win64's home area
//! ```
//!
//! The frame has no frame pointer and a fixed size. When the function calls
//! or has a 16-byte-aligned slot, the size subtracted after the pushes makes
//! `%rsp` a multiple of 16: at entry `%rsp + 8` is one (psABI §3.2.2;
//! Microsoft Learn, "x64 stack usage").

use super::{
    conv::CallConv,
    mir::{
        FrameObject,
        MFunction,
        PReg,
    },
    regalloc::Allocation,
};
use crate::util::bump::{
    ArenaVec,
    Bump,
};

/// A laid-out frame.
pub(crate) struct Frame<'a> {
    /// The callee-saved registers the prologue pushes, in push order.
    pub(crate) saved: ArenaVec<'a, PReg>,
    /// Bytes the prologue subtracts from `%rsp` after the pushes.
    pub(crate) size:  u32,
    slots:            ArenaVec<'a, u32>,
    spills:           u32,
    incoming:         u32,
}

impl<'a> Frame<'a> {
    /// Lays out the frame of an allocated function.
    pub(crate) fn layout(
        func: &MFunction<'_>,
        allocation: &Allocation<'_>,
        conv: CallConv,
        arena: &'a Bump,
    ) -> Self {
        let mut saved = ArenaVec::new_in(arena);
        for register in PReg::ALL {
            if conv.callee_saved.contains(register) && allocation.written.contains(register) {
                saved.push(register);
            }
        }
        let spills = func.outgoing.next_multiple_of(8);
        let mut end = spills + 8 * allocation.spill_slots;
        let mut slots = ArenaVec::with_capacity_in(func.slots.len(), arena);
        let mut aligned = func.calls;
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
        Self {
            saved,
            size,
            slots,
            spills,
            incoming: size + pushes + 8 + conv.home_area,
        }
    }

    /// The offset of a frame object from `%rsp` after the prologue.
    pub(crate) fn offset(&self, object: FrameObject) -> i64 {
        i64::from(match object {
            | FrameObject::Slot(slot) => self.slots[slot as usize],
            | FrameObject::Incoming(offset) => self.incoming + offset,
            | FrameObject::Outgoing(offset) => offset,
            | FrameObject::Spill(slot) => self.spills + 8 * slot,
        })
    }

    /// The distance from `%rsp` after the prologue to the canonical frame
    /// address (the caller's `%rsp` before the call).
    pub(crate) fn cfa_offset(&self) -> u32 {
        self.size + 8 * u32::try_from(self.saved.len()).expect("at most 16 registers") + 8
    }
}
