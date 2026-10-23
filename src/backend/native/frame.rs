//! Frame layout: where the callee-saved registers, IR stack slots, spill
//! slots and outgoing arguments live, as offsets from `%rsp` after the
//! prologue (or from `%rbp`, which then holds that same address).
//!
//! ```text
//!  incoming stack arguments   CFA + home area (Win64: 32 bytes)
//!  Win64 home area            CFA .. CFA + 32
//!  return address             CFA - 8
//!  callee-saved pushes        8 bytes each (`%rbp` among them with a frame pointer;
//!                             first, at CFA - 16, when the frame address is taken)
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
//! stay at the bottom of the current `%rsp`. When the function calls, has a
//! 16-byte-aligned object or allocates dynamically, the size subtracted
//! after the pushes makes `%rsp` a multiple of 16: at entry `%rsp + 8` is one
//! (psABI §3.2.2; Microsoft Learn, "x64 stack usage").
//!
//! A System V leaf function whose frame fits the 128-byte red zone below
//! `%rsp` keeps it there and never moves `%rsp` (psABI §3.2.2): the frame is
//! laid out as usual, but its objects sit `size` bytes lower relative to the
//! `%rsp` the function runs with, and the prologue and epilogue neither
//! subtract nor add. A function that calls, moves `%rsp`, reads or writes
//! `%rsp` itself, saves a register area for `va_start` or has a slot aligned
//! above 16 keeps the ordinary frame, as does every Win64 function.

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
    support::Construct,
};
use crate::util::bump::{
    ArenaVec,
    Bump,
};

/// The bytes of the System V register save area: six general registers and
/// eight 16-byte SSE registers (psABI §3.5.7, Figure 3.33).
pub(crate) const REGISTER_SAVE_BYTES: u32 = 6 * 8 + 8 * 16;

/// The largest frame offset an operand's signed 32-bit displacement holds.
const MAX_DISPLACEMENT: u64 = 0x7FFF_FFFF;

/// A laid-out frame.
pub(crate) struct Frame<'a> {
    /// The callee-saved general registers the prologue pushes, in push
    /// order.
    pub(crate) saved:     ArenaVec<'a, PReg>,
    /// The callee-saved SSE registers (Win64) and their offsets.
    pub(crate) saved_xmm: ArenaVec<'a, (PReg, u32)>,
    /// Bytes of the frame below the pushes: what the prologue subtracts from
    /// `%rsp`, unless the frame is in the red zone.
    pub(crate) size:      u32,
    /// Whether `%rbp` addresses the fixed frame.
    pub(crate) rbp_based: bool,
    /// Whether the frame lies in the System V red zone, so the prologue
    /// leaves `%rsp` where the pushes put it.
    pub(crate) red_zone:  bool,
    slots:                ArenaVec<'a, u32>,
    spills:               u32,
    outgoing_end:         u32,
    register_save:        u32,
    incoming:             u32,
    home:                 u32,
}

impl<'a> Frame<'a> {
    /// Lays out the frame of an allocated and rewritten function, or
    /// refuses it when an offset would not fit an operand's signed 32-bit
    /// displacement. The support check bounds the IR stack slots and each
    /// call's stack arguments separately; together with the spill slots and
    /// the incoming arguments above the frame they can still exceed it.
    pub(crate) fn layout(
        func: &MFunction<'_>,
        allocation: &Allocation<'_>,
        conv: CallConv,
        arena: &'a Bump,
    ) -> Result<Self, Construct> {
        let written = written(func);
        let frame_pointer = func.dynamic;
        let mut saved = ArenaVec::new_in(arena);
        let mut xmm = ArenaVec::new_in(arena);
        // The frame address is where a frame pointer saves the caller's
        // `%rbp`, so that register is pushed first.
        if let Some(register) = func.first_push {
            saved.push(register);
        }
        for register in PReg::ALL {
            let save = conv.callee_saved.contains(register)
                && func.first_push != Some(register)
                && (written.contains(register) || (frame_pointer && register == PReg::Rbp));
            match (save, register.class()) {
                | (true, RegClass::Gpr) => saved.push(register),
                | (true, RegClass::Xmm) => xmm.push(register),
                | (false, _) => {},
            }
        }
        // A dynamic block is 16-byte aligned only if `%rsp` is.
        let mut aligned = func.calls || !xmm.is_empty() || func.register_save || func.dynamic;
        // Dynamic allocations start right above the outgoing arguments and
        // keep 16-byte alignment.
        // Offsets are summed in 64 bits and narrowed once they are known to
        // fit.
        let outgoing_end = u64::from(func.outgoing).next_multiple_of(16);
        let spills = outgoing_end;
        let mut end = spills + 8 * u64::from(allocation.spill_slots);
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
            end += u64::from(REGISTER_SAVE_BYTES);
        }
        let mut slots = ArenaVec::with_capacity_in(func.slots.len(), arena);
        for slot in &func.slots {
            let align = slot.align.max(1);
            aligned |= align >= 16;
            end = end.next_multiple_of(u64::from(align));
            slots.push(end);
            end += u64::from(slot.size);
        }
        let pushes = 8 * u64::try_from(saved.len()).expect("at most 16 registers");
        let mut size = end.next_multiple_of(8);
        if aligned && !(8 + pushes + size).is_multiple_of(16) {
            size += 8;
        }
        let incoming = size + pushes + 8 + u64::from(conv.home_area);
        // The highest offset a frame operand names: the last incoming
        // stack argument, above every other object.
        let reach = incoming + u64::from(func.incoming);
        if reach > MAX_DISPLACEMENT {
            return Err(Construct::FrameSize(reach));
        }
        // A leaf frame in the red zone: no call can push into it, and no
        // instruction moves `%rsp` under it or takes `%rsp` as a value. A
        // function with a slot aligned above 16 keeps the ordinary frame, as
        // LLVM's do: they realign `%rsp` through a frame pointer.
        let red_zone = size > 0
            && size <= u64::from(conv.red_zone)
            && !func.calls
            && !frame_pointer
            && !func.register_save
            && func.slot_align <= 16
            && saved_xmm.is_empty()
            && leaf(func);
        let narrow = |offset: u64| u32::try_from(offset).expect("bounded by the reach");
        let mut saved_xmm_offsets = ArenaVec::with_capacity_in(saved_xmm.len(), arena);
        saved_xmm_offsets.extend(saved_xmm.into_iter().map(|(r, at)| (r, narrow(at))));
        let mut slot_offsets = ArenaVec::with_capacity_in(slots.len(), arena);
        slot_offsets.extend(slots.into_iter().map(narrow));
        let incoming = narrow(incoming);
        Ok(Self {
            saved,
            saved_xmm: saved_xmm_offsets,
            size: narrow(size),
            rbp_based: frame_pointer,
            red_zone,
            slots: slot_offsets,
            spills: narrow(spills),
            outgoing_end: narrow(outgoing_end),
            register_save: narrow(register_save),
            incoming,
            home: incoming - conv.home_area,
        })
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
            | FrameObject::ReturnAddress => self.home - 8,
            | FrameObject::FrameAddress => self.home - 16,
        };
        let base = if self.rbp_based { PReg::Rbp } else { PReg::Rsp };
        (base, i64::from(offset) - i64::from(self.allocated_below()))
    }

    /// The bytes of the frame below `%rsp` as the function runs: the frame's
    /// size in the red zone, otherwise none.
    fn allocated_below(&self) -> u32 {
        if self.red_zone { self.size } else { 0 }
    }

    /// The bytes the prologue subtracts from `%rsp` after the pushes, and
    /// the epilogue adds back.
    pub(crate) fn allocation(&self) -> u32 {
        self.size - self.allocated_below()
    }

    /// The distance from `%rsp` after the prologue to the canonical frame
    /// address (the caller's `%rsp` before the call).
    pub(crate) fn cfa_offset(&self) -> u32 {
        self.allocation() + 8 * u32::try_from(self.saved.len()).expect("at most 16 registers") + 8
    }
}

/// Whether `func` neither calls nor names `%rsp` in any instruction, so
/// nothing but its own frame accesses touch the stack below the pushes.
fn leaf(func: &MFunction<'_>) -> bool {
    func.blocks
        .iter()
        .flat_map(|block| &block.insts)
        .all(|inst| {
            if matches!(inst, MInst::Call { .. }) {
                return false;
            }
            let mut inst = *inst;
            let mut touches = false;
            inst.visit_registers(|reg, _| touches |= *reg == Reg::Phys(PReg::Rsp));
            !touches
        })
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
