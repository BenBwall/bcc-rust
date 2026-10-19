//! The linear scan over split intervals: Wimmer and Mössenböck, "Optimized
//! Interval Splitting in a Linear Scan Register Allocator" (VEE 2005), the
//! successor of Traub, Holloway and Smith's second-chance binpacking.
//!
//! Intervals are visited by start position. Those holding a register are
//! *active* where they are live at the current position and *inactive* in
//! a lifetime hole. The current interval takes a register free for its
//! whole life if one is (its hint first, then the convention's order), or
//! else the register free longest, splitting itself where that register is
//! next needed; the rest returns to the queue. When no register is free at
//! all, the one whose occupants are next needed latest is taken from them:
//! they are split before the current position, their tails live in their
//! spill slots until shortly before their next use, and from there return
//! to the queue for a register. If the current interval itself is needed
//! later than every occupant, it is the one spilled up to its first use.
//!
//! Every use therefore normally has a register. A spilled piece that still
//! has uses (an interval that must keep one location, or a position where
//! no register can be taken) keeps them, and the rewrite moves them through
//! the scratch registers, which no interval gets; that fallback never
//! fails, so allocation always succeeds.

use super::intervals::{
    FixedRanges,
    Interval,
    Loc,
};
use crate::{
    backend::native::{
        conv::CallConv,
        mir::{
            PReg,
            VReg,
        },
    },
    ir::Entity,
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// Allocates every interval of `scan`, appending the pieces splitting
/// creates.
pub(super) fn run(scan: &mut Scan<'_, '_>) {
    for id in 0..scan.intervals.len() {
        if !scan.intervals[id].is_empty() {
            scan.enqueue(id);
        }
    }
    while let Some(id) = scan.dequeue() {
        let position = scan.intervals[id].start();
        scan.advance(position);
        if !scan.try_free(id) {
            scan.blocked(id);
        }
        if let Loc::Reg(_) = scan.intervals[id].loc {
            scan.active.push(id);
        }
    }
}

/// The scan's state.
pub(super) struct Scan<'s, 'a> {
    pub(super) intervals: ArenaVec<'a, Interval<'a>>,
    fixed:                &'s FixedRanges<'a>,
    conv:                 CallConv,
    /// Whether `%rbp` addresses the frame and is not allocatable.
    frame_pointer:        bool,
    /// For each representative register, a register it is copied to or
    /// from; the hint is that register's latest location.
    copy_hints:           &'s [Option<VReg>],
    /// Each representative's latest register.
    last_register:        ArenaVec<'a, Option<PReg>>,
    /// The loop depth of each block and the position each block starts
    /// at, then the end of the function.
    depth:                &'s [u32],
    block_starts:         &'s [u32],
    /// A binary min-heap of `(start, id)`.
    queue:                ArenaVec<'a, (u32, u32)>,
    active:               ArenaVec<'a, usize>,
    inactive:             ArenaVec<'a, usize>,
    arena:                &'a Bump,
    /// Intervals whose uses go through the scratch registers.
    pub(super) fallbacks: u32,
}

impl<'s, 'a> Scan<'s, 'a> {
    #[expect(
        clippy::too_many_arguments,
        reason = "The scan's inputs are distinct analyses."
    )]
    pub(super) fn new(
        intervals: ArenaVec<'a, Interval<'a>>,
        fixed: &'s FixedRanges<'a>,
        conv: CallConv,
        frame_pointer: bool,
        copy_hints: &'s [Option<VReg>],
        depth: &'s [u32],
        block_starts: &'s [u32],
        arena: &'a Bump,
    ) -> Self {
        let mut last_register = ArenaVec::with_capacity_in(intervals.len(), arena);
        last_register.resize(intervals.len(), None);
        Self {
            intervals,
            fixed,
            conv,
            frame_pointer,
            copy_hints,
            last_register,
            depth,
            block_starts,
            queue: ArenaVec::new_in(arena),
            active: ArenaVec::new_in(arena),
            inactive: ArenaVec::new_in(arena),
            arena,
            fallbacks: 0,
        }
    }

    /// Moves intervals that ended to the handled set (by dropping them)
    /// and those that entered or left a lifetime hole between the active
    /// and inactive lists.
    fn advance(&mut self, position: u32) {
        let intervals = &self.intervals;
        let mut index = 0;
        while index < self.active.len() {
            let id = self.active[index];
            if intervals[id].end() <= position {
                _ = self.active.swap_remove(index);
            } else if !intervals[id].covers(position) {
                _ = self.active.swap_remove(index);
                self.inactive.push(id);
            } else {
                index += 1;
            }
        }
        let mut index = 0;
        while index < self.inactive.len() {
            let id = self.inactive[index];
            if intervals[id].end() <= position {
                _ = self.inactive.swap_remove(index);
            } else if intervals[id].covers(position) {
                _ = self.inactive.swap_remove(index);
                self.active.push(id);
            } else {
                index += 1;
            }
        }
    }

    /// The registers `id` may take, in preference order.
    fn order(&self, id: usize) -> impl Iterator<Item = PReg> + use<'_> {
        let frame_pointer = self.frame_pointer;
        self.conv
            .order(self.intervals[id].class)
            .iter()
            .copied()
            .filter(move |&register| !(frame_pointer && register == PReg::Rbp))
    }

    /// The register a copy or an earlier piece suggests for `id`.
    fn hint(&self, id: usize) -> Option<PReg> {
        let interval = &self.intervals[id];
        interval.hint.or_else(|| {
            let other = self.copy_hints[interval.vreg.index()]?;
            self.last_register[other.index()]
        })
    }

    /// Gives `id` a register that is free at its start, for all of its
    /// life or up to a split; whether it did.
    fn try_free(&mut self, id: usize) -> bool {
        let position = self.intervals[id].start();
        let mut free_until = [u32::MAX; PReg::COUNT];
        for &other in &self.active {
            if let Loc::Reg(register) = self.intervals[other].loc {
                free_until[register.index()] = 0;
            }
        }
        for &other in &self.inactive {
            let other = &self.intervals[other];
            if let Loc::Reg(register) = other.loc
                && let Some(at) = other.next_intersection(&self.intervals[id].ranges, position)
            {
                free_until[register.index()] = free_until[register.index()].min(at);
            }
        }
        for register in self.order(id) {
            if let Some(at) =
                self.fixed
                    .next_intersection(register, &self.intervals[id].ranges, position)
            {
                free_until[register.index()] = free_until[register.index()].min(at);
            }
        }
        let end = self.intervals[id].end();
        let hint = self
            .hint(id)
            .filter(|&register| self.order(id).any(|allowed| allowed == register));
        let whole = hint
            .filter(|register| free_until[register.index()] >= end)
            .or_else(|| {
                self.order(id)
                    .find(|register| free_until[register.index()] >= end)
            });
        if let Some(register) = whole {
            self.assign(id, register);
            return true;
        }
        if self.intervals[id].unsplittable {
            return false;
        }
        let mut best = None;
        for register in hint.into_iter().chain(self.order(id)) {
            if best.is_none_or(|best: PReg| free_until[register.index()] > free_until[best.index()])
            {
                best = Some(register);
            }
        }
        let Some(register) = best else { return false };
        let Some(at) = self.split_position(position, free_until[register.index()]) else {
            return false;
        };
        let child = self.intervals[id].split(at, self.arena);
        self.push(child);
        self.assign(id, register);
        true
    }

    /// Gives `id` a register by spilling the intervals that hold it, or
    /// spills `id` itself up to its first use.
    fn blocked(&mut self, id: usize) {
        if self.intervals[id].unsplittable {
            self.fall_back(id);
            return;
        }
        let position = self.intervals[id].start();
        // Occupants leave their register before the instruction at or
        // after which the current interval starts.
        let evict_at = position & !1;
        let mut next_use = [u32::MAX; PReg::COUNT];
        let mut block = [u32::MAX; PReg::COUNT];
        let need = |interval: &Interval<'_>| {
            if interval.unsplittable {
                0
            } else {
                interval.next_use(evict_at).unwrap_or(u32::MAX)
            }
        };
        for &other in &self.active {
            if let Loc::Reg(register) = self.intervals[other].loc {
                let slot = &mut next_use[register.index()];
                *slot = (*slot).min(need(&self.intervals[other]));
            }
        }
        for &other in &self.inactive {
            let interval = &self.intervals[other];
            if let Loc::Reg(register) = interval.loc
                && interval
                    .next_intersection(&self.intervals[id].ranges, position)
                    .is_some()
            {
                let slot = &mut next_use[register.index()];
                *slot = (*slot).min(need(interval));
            }
        }
        let mut best: Option<PReg> = None;
        for register in self.order(id) {
            if let Some(at) =
                self.fixed
                    .next_intersection(register, &self.intervals[id].ranges, position)
            {
                block[register.index()] = at;
                next_use[register.index()] = next_use[register.index()].min(at);
            }
            if best.is_none_or(|best| next_use[register.index()] > next_use[best.index()]) {
                best = Some(register);
            }
        }
        let Some(register) = best else {
            self.fall_back(id);
            return;
        };
        let first_use = self.intervals[id].next_use(position);
        let Some(first_use) = first_use else {
            // Never read or written again: the slot will do.
            self.intervals[id].loc = Loc::Spill;
            return;
        };
        if next_use[register.index()] < first_use
            && let Some(at) = self.reload_position(position, first_use)
        {
            // Everyone else needs a register sooner.
            let child = self.intervals[id].split(at, self.arena);
            self.push(child);
            self.intervals[id].loc = Loc::Spill;
            return;
        }
        if next_use[register.index()] < evict_at + 2 {
            self.fall_back(id);
            return;
        }
        if block[register.index()] < self.intervals[id].end() {
            let at = self
                .split_position(position, block[register.index()])
                .expect("the fixed range starts after the occupants' next use");
            let child = self.intervals[id].split(at, self.arena);
            self.push(child);
        }
        self.assign(id, register);
        let mut index = 0;
        while index < self.active.len() {
            let other = self.active[index];
            if self.intervals[other].loc == Loc::Reg(register) {
                _ = self.active.swap_remove(index);
                self.evict(other, evict_at);
            } else {
                index += 1;
            }
        }
        let mut index = 0;
        while index < self.inactive.len() {
            let other = self.inactive[index];
            if self.intervals[other].loc == Loc::Reg(register)
                && self.intervals[other]
                    .next_intersection(&self.intervals[id].ranges, position)
                    .is_some()
            {
                _ = self.inactive.swap_remove(index);
                self.evict(other, evict_at);
            } else {
                index += 1;
            }
        }
    }

    /// Takes `id`'s register from `at` on: the part from `at` lives in the
    /// spill slot until shortly before its next use, and from there waits
    /// in the queue for a register.
    fn evict(&mut self, id: usize, at: u32) {
        let spilled = if self.intervals[id].start() < at {
            let child = self.intervals[id].split(at, self.arena);
            self.intervals.push(child);
            self.intervals.len() - 1
        } else {
            self.intervals[id].loc = Loc::None;
            id
        };
        let start = self.intervals[spilled].start();
        match self.intervals[spilled].next_use(start) {
            | None => self.intervals[spilled].loc = Loc::Spill,
            | Some(next) => match self.reload_position(start, next) {
                | Some(reload) => {
                    let child = self.intervals[spilled].split(reload, self.arena);
                    self.push(child);
                    self.intervals[spilled].loc = Loc::Spill;
                },
                // Live again only where it is used: it waits whole.
                | None => {
                    self.intervals[spilled].loc = Loc::None;
                    self.enqueue(spilled);
                },
            },
        }
    }

    /// Leaves `id` in its spill slot for the rest of its life, its uses
    /// going through the scratch registers.
    fn fall_back(&mut self, id: usize) {
        self.intervals[id].loc = Loc::Spill;
        self.fallbacks += 1;
    }

    fn assign(&mut self, id: usize, register: PReg) {
        self.intervals[id].loc = Loc::Reg(register);
        let vreg = self.intervals[id].vreg;
        self.last_register[vreg.index()] = Some(register);
    }

    /// Where to split an interval that starts at `start` and may keep its
    /// register up to `limit`: an even position in `(start, limit]`,
    /// preferably a block boundary of a shallower loop than the latest
    /// candidate's.
    fn split_position(&self, start: u32, limit: u32) -> Option<u32> {
        let latest = limit & !1;
        (latest > start).then(|| self.shallowest(start, latest))
    }

    /// Where to reload a value spilled from `start` before its use at
    /// `next`: an even position in `(start, next]`, hoisted to a shallower
    /// block boundary when there is one.
    fn reload_position(&self, start: u32, next: u32) -> Option<u32> {
        self.split_position(start, next)
    }

    /// The latest block start in `(start, latest]` whose loop is shallower
    /// than `latest`'s, or `latest`.
    fn shallowest(&self, start: u32, latest: u32) -> u32 {
        let block_of = |position: u32| self.block_starts.partition_point(|&at| at <= position) - 1;
        let mut best = latest;
        let mut best_depth = self.depth[block_of(latest)];
        let first = self.block_starts.partition_point(|&at| at <= start);
        let last = self.block_starts.partition_point(|&at| at <= latest);
        for block in (first..last).rev() {
            if self.depth[block] < best_depth {
                best = self.block_starts[block];
                best_depth = self.depth[block];
            }
        }
        best
    }

    fn push(&mut self, interval: Interval<'a>) {
        self.intervals.push(interval);
        self.enqueue(self.intervals.len() - 1);
    }

    fn enqueue(&mut self, id: usize) {
        let key = (
            self.intervals[id].start(),
            u32::try_from(id).expect("interval numbers fit u32"),
        );
        let mut index = self.queue.len();
        self.queue.push(key);
        while index > 0 {
            let parent = (index - 1) / 2;
            if self.queue[parent] <= self.queue[index] {
                break;
            }
            self.queue.swap(parent, index);
            index = parent;
        }
    }

    fn dequeue(&mut self) -> Option<usize> {
        let last = self.queue.pop()?;
        let top = if self.queue.is_empty() {
            last
        } else {
            let top = std::mem::replace(&mut self.queue[0], last);
            let mut index = 0;
            loop {
                let mut smallest = index;
                for child in [2 * index + 1, 2 * index + 2] {
                    if child < self.queue.len() && self.queue[child] < self.queue[smallest] {
                        smallest = child;
                    }
                }
                if smallest == index {
                    break;
                }
                self.queue.swap(index, smallest);
                index = smallest;
            }
            top
        };
        Some(top.1 as usize)
    }
}
