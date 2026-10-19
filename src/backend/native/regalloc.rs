//! Register allocation: linear scan over whole live intervals with
//! fixed-register ranges (Poletto and Sarkar, "Linear Scan Register
//! Allocation", TOPLAS 21(5), 1999).
//!
//! Instruction `i` (numbered in block layout order) reads at position `2i`
//! and writes at `2i+1`, so a value that dies at an instruction may share a
//! register with one it defines. Liveness is backward dataflow over the
//! blocks; each virtual register gets one interval from its first to its
//! last live position, holes included. Each physical register gets the
//! ranges where a precolored value lives in it (an argument between its
//! `mov` and the `call`, `%rax` between a `call` and the copy of its
//! result) and a point at each clobber. An interval may take a register
//! only if it overlaps none of that register's fixed ranges, so a value
//! live across a call, which overlaps every caller-saved clobber, lands in
//! a callee-saved register or on the stack without a special case.
//!
//! The scan visits intervals by start, expires those that ended, and gives
//! the current one its hint (the register a copy connects it to) or the
//! first free register in the convention's order. With none free it spills
//! whichever of the current interval and the active ones that could yield
//! a compatible register ends last. A spilled value lives in its own stack
//! slot for its whole life; `rewrite.rs` loads it into a scratch register
//! before each use and stores it after each definition. In debug builds,
//! `checker.rs` then proves the rewritten function against the
//! virtual-register program.
//!
//! Read [`allocate`], then `liveness.rs` for intervals and fixed ranges,
//! then `rewrite.rs` and `checker.rs`.

// Analysis
mod liveness;

// Rewriting and its proof
mod checker;
mod rewrite;

pub(crate) use rewrite::rewrite;

use self::liveness::Liveness;
#[cfg(test)]
pub(crate) use self::{
    checker::{
        CheckError,
        ErrorKind,
    },
    rewrite::rewrite_checked,
};
use super::{
    conv::CallConv,
    mir::{
        MFunction,
        PReg,
        PRegSet,
        Reg,
        VReg,
    },
};
use crate::{
    ir::Entity,
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// Allocates every virtual register of `func`, after deleting the pure
/// instructions whose results are dead.
pub(crate) fn allocate<'a>(
    func: &mut MFunction<'_>,
    conv: CallConv,
    arena: &'a Bump,
) -> Allocation<'a> {
    let liveness = Liveness::compute(func, arena);
    let mut intervals = liveness.intervals(func, arena);
    let fixed = liveness.fixed_ranges(func, arena);
    intervals.sort_unstable_by_key(|interval| (interval.start, interval.vreg));
    let mut locations = ArenaVec::with_capacity_in(func.vreg_count(), arena);
    locations.resize(func.vreg_count(), Location::None);
    let mut allocation = Allocation {
        locations,
        spill_slots: 0,
    };
    let mut active: ArenaVec<'_, Active> = ArenaVec::new_in(arena);
    for &interval in &intervals {
        active.retain(|active| active.end >= interval.start);
        let class = func.classes[interval.vreg.index()];
        // With a frame pointer, `%rbp` addresses the frame.
        let order = conv.order(class);
        let allowed = |register: &PReg| !(func.dynamic && *register == PReg::Rbp);
        let occupied = active
            .iter()
            .fold(PRegSet::EMPTY, |set, active| set.with(active.reg));
        let free = |register: PReg| {
            !occupied.contains(register) && !fixed.overlaps(register, interval.start, interval.end)
        };
        let hint = match func.hints[interval.vreg.index()] {
            | Some(Reg::Phys(register)) => Some(register),
            | Some(Reg::Virt(other)) => match allocation.locations[other.index()] {
                | Location::Reg(register) => Some(register),
                | _ => None,
            },
            | None => None,
        };
        let choice = hint
            .filter(|register| order.contains(register) && allowed(register) && free(*register))
            .or_else(|| {
                order
                    .iter()
                    .filter(|register| allowed(register))
                    .copied()
                    .find(|&register| free(register))
            });
        if let Some(register) = choice {
            allocation.locations[interval.vreg.index()] = Location::Reg(register);
            active.push(Active {
                end:  interval.end,
                vreg: interval.vreg,
                reg:  register,
            });
            continue;
        }
        let victim = active
            .iter()
            .enumerate()
            .filter(|(_, active)| {
                active.reg.class() == class
                    && !fixed.overlaps(active.reg, interval.start, interval.end)
            })
            .max_by_key(|(_, active)| active.end)
            .map(|(index, &active)| (index, active));
        match victim {
            | Some((index, victim)) if victim.end > interval.end => {
                allocation.locations[victim.vreg.index()] = Location::Spill(allocation.spill());
                allocation.locations[interval.vreg.index()] = Location::Reg(victim.reg);
                active[index] = Active {
                    end:  interval.end,
                    vreg: interval.vreg,
                    reg:  victim.reg,
                };
            },
            | _ =>
                allocation.locations[interval.vreg.index()] = Location::Spill(allocation.spill()),
        }
    }
    if cfg!(debug_assertions) {
        check(&intervals, &allocation, &fixed, arena);
    }
    allocation
}

/// Where each virtual register lives, and what the frame must provide.
pub(crate) struct Allocation<'a> {
    pub(crate) locations:   ArenaVec<'a, Location>,
    /// Spill slots used, 8 bytes each.
    pub(crate) spill_slots: u32,
}

impl Allocation<'_> {
    /// A new spill slot.
    fn spill(&mut self) -> u32 {
        let slot = self.spill_slots;
        self.spill_slots += 1;
        slot
    }
}

/// A virtual register's home.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[expect(
    variant_size_differences,
    reason = "A spill slot is one word; locations are a dense per-register table."
)]
pub(crate) enum Location {
    /// The register never occurs.
    None,
    Reg(PReg),
    Spill(u32),
}

/// A virtual register's live interval, positions inclusive.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Interval {
    pub(crate) vreg:  VReg,
    pub(crate) start: u32,
    pub(crate) end:   u32,
}

/// An interval holding a register during the scan.
#[derive(Clone, Copy, Debug)]
struct Active {
    end:  u32,
    vreg: VReg,
    reg:  PReg,
}

/// Disjoint, sorted ranges per physical register where a precolored value
/// lives or the register is clobbered.
pub(crate) struct FixedRanges<'a> {
    ranges: [ArenaVec<'a, (u32, u32)>; PReg::COUNT],
}

impl FixedRanges<'_> {
    /// Whether `[start, end]` meets a fixed range of `register`.
    pub(crate) fn overlaps(&self, register: PReg, start: u32, end: u32) -> bool {
        let ranges = &self.ranges[register.index()];
        let first = ranges.partition_point(|&(_, range_end)| range_end < start);
        ranges
            .get(first)
            .is_some_and(|&(range_start, _)| range_start <= end)
    }
}

/// Checks the allocation in debug builds: no two intervals in one register
/// overlap, and none overlaps a fixed range of its register.
fn check(
    intervals: &[Interval],
    allocation: &Allocation<'_>,
    fixed: &FixedRanges<'_>,
    arena: &Bump,
) {
    let mut last_end: ArenaVec<'_, Option<(u32, VReg)>> = ArenaVec::new_in(arena);
    last_end.resize(PReg::COUNT, None);
    for interval in intervals {
        let Location::Reg(register) = allocation.locations[interval.vreg.index()] else {
            continue;
        };
        assert!(
            !fixed.overlaps(register, interval.start, interval.end),
            "{:?} in {register:?} overlaps a fixed range",
            interval.vreg
        );
        if let Some((end, other)) = last_end[register.index()] {
            assert!(
                end < interval.start,
                "{:?} and {other:?} share {register:?} while both live",
                interval.vreg
            );
        }
        let end = last_end[register.index()].map_or(interval.end, |(end, _)| end.max(interval.end));
        last_end[register.index()] = Some((end, interval.vreg));
    }
}
