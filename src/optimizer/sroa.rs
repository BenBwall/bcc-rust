//! Scalar replacement of aggregates: splits a stack slot that is accessed
//! only at constant offsets into one slot per field, so that `promote` can
//! turn every field into SSA values.
//!
//! A structure or array local, a by-value argument copy and a call's result
//! area all live in stack slots that the program reads and writes member by
//! member, and copies whole or by member with `copy`. `promote` keeps such a
//! slot in memory because its address is offset and copied. This pass
//! rewrites it into scalar slots that `promote` accepts:
//!
//! 1. **Uses** (`plan.rs`). Every pointer derived from a slot's `stack_addr`
//!    through `ptr_add`s with constant offsets must be the address of a
//!    non-volatile `load` or `store` inside the slot, an operand of a
//!    non-volatile `copy` of a constant byte range inside the slot, or the
//!    destination of such a `fill` with a constant byte. Any other use (a call
//!    or branch argument, a stored address, a comparison, a variable offset)
//!    lets the address escape and keeps the slot as it is.
//! 2. **Components.** A `copy` between two slots joins them (with union-find).
//!    A component splits only if none of its slots escapes.
//! 3. **Fields.** Each slot's loads and stores, by offset and type, are its
//!    first fields. A `copy` between two slots must move whole fields, so the
//!    fields inside its range spread to the other side, shifted, until no copy
//!    adds any. Two accesses at one offset must have one type, and fields must
//!    not overlap one another or straddle the edge of a copied or filled range;
//!    a union read through two members, or an array read at a variable index,
//!    keeps its component whole.
//! 4. **Rewrite** (`rewrite.rs`). Each slot gets one new slot per field. Loads
//!    and stores use the field's slot instead; a `copy` between two slots
//!    becomes a load per field followed by a store per field; a `copy` from or
//!    to other memory becomes one load and one store per field at the field's
//!    offset there; a `fill` stores each field's constant. The old
//!    `stack_addr`s, offsets and slots are removed.
//!
//! In
//!
//! ```text
//! block0(v0: ptr):                      block0(v0: ptr):
//!     v1 = stack_addr slot0                 v1 = stack_addr slot2
//!     copy v1, v0, v2, align 4              v2 = stack_addr slot1
//!     v3 = ptr_add inbounds v1, v4          v3 = load.i32 v0, align 4
//!     v5 = load.i32 v3, align 4             store.i32 v3, v2, align 4
//!                                           v4 = ptr_add inbounds v0, v5
//!                                           v6 = load.i32 v4, align 4
//!                                           store.i32 v6, v1, align 4
//!                                           v7 = load.i32 v1, align 4
//! ```
//!
//! the eight-byte `slot0` with fields at offsets 0 and 4 becomes `slot1` and
//! `slot2`, which `promote` then removes.
//!
//! **Bytes and poison.** The IR's memory holds bytes, each possibly poison,
//! and a load is poison if any byte it reads is. A field moved as a value is
//! poison as a whole when any of its bytes is, and bytes outside every field
//! are not moved at all. Neither difference is visible while the moved bytes
//! are observed only through loads, which read whole fields. A `copy` out to
//! other memory observes bytes too, so:
//!
//! - The bytes of a slot that can reach a copy out (through copies between
//!   slots) and that no access covers become integer fields as well, so that
//!   fills and copies still carry them.
//! - A slot on a path of copies from other memory to other memory is
//!   *retained*: its bytes may mix poison and defined bytes within a field. It
//!   keeps its memory and every write to it, so that the copy out still copies
//!   exact bytes, and its fields become *shadows* that only its loads read. A
//!   shadow is written wherever the slot is (a store, a fill, a copy in, as a
//!   load from the source at the copy), so a load of the shadow reads exactly
//!   what a load of the slot would. A copy into a retained slot from a split
//!   one stores the split slot's fields into its memory: a slot that reaches a
//!   retained one and is not retained itself has no bytes from other memory, so
//!   its fields hold its exact bytes.
//!
//! A field slot is as aligned as the field's offset in the original slot.
//! Each component split is one rewrite for bisecting. A slot with more than
//! [`MAX_FIELDS`] fields keeps its component whole, and a component is left
//! alone when splitting gains nothing: a lone slot accessed as one scalar of
//! its size (which `promote` takes as it is), or retained slots that nothing
//! loads.
//!
//! This is LLVM's `SROA` pass in miniature (`llvm/lib/Transforms/Scalar/
//! SROA.cpp`): partitions by constant offsets, memory intrinsics split along
//! them, without its splitting of overlapping or vector accesses, and with
//! retained slots where LLVM would move partly poisoned bytes as one value.
//!
//! C99: §6.2.5 paragraph 20, p. 35; PDF p. 47 (structures and arrays are
//! sequences of member objects); §6.2.6.1 paragraph 6, p. 37; PDF p. 49
//! (padding bytes take unspecified values, which the copy-out rule above
//! still carries); §5.1.2.3 paragraph 5, pp. 13-14; PDF pp. 25-26.

mod plan;
mod rewrite;

use super::{
    draft::Draft,
    report::OptimizationReport,
};
use crate::{
    ir::{
        Block,
        Inst,
        StackSlot,
        Type,
    },
    util::bump::ArenaVec,
};

/// The most fields one slot is split into; a copy expands to a load and a
/// store per field.
const MAX_FIELDS: usize = 32;

/// Splits every splittable component of stack slots of one function.
/// Returns whether it split any.
pub(super) fn run(draft: &mut Draft<'_, '_>, report: &mut OptimizationReport) -> bool {
    if draft.stack_slot_count() == 0 {
        return false;
    }
    let uses = plan::Uses::collect(draft);
    let plans = uses.plans(draft);
    let mut changed = false;
    for plan in &plans {
        if !report.allow() {
            break;
        }
        rewrite::rewrite(draft, &uses, plan);
        changed = true;
    }
    if changed {
        draft.sweep();
    }
    changed
}

/// One component to split.
struct Plan<'s> {
    /// Its slots, in slot order.
    slots: ArenaVec<'s, SlotPlan<'s>>,
}

/// How one slot of a component is split.
struct SlotPlan<'s> {
    slot:     StackSlot,
    /// Whether the slot keeps its memory, with its fields as shadows.
    retained: bool,
    /// Offsets and types, sorted by offset, not overlapping.
    fields:   ArenaVec<'s, (u32, Type)>,
}

/// A `copy` or `fill` that touches a slot of a component.
#[derive(Clone, Copy, Debug)]
struct Move {
    block: Block,
    inst:  Inst,
    size:  u32,
    kind:  MoveKind,
}

/// What a [`Move`] moves, with the slots and offsets of its range.
#[derive(Clone, Copy, Debug)]
enum MoveKind {
    /// A copy from one slot range to another (or the same) slot's range.
    Copy {
        to:   (StackSlot, u32),
        from: (StackSlot, u32),
    },
    /// A copy into a slot range from other memory.
    CopyIn { to: (StackSlot, u32) },
    /// A copy out of a slot range to other memory.
    CopyOut { from: (StackSlot, u32) },
    /// A fill of a slot range with a constant byte.
    Fill { to: (StackSlot, u32), byte: u8 },
}
