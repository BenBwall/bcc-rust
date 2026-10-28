//! Live intervals: where a virtual register's value is needed, as sorted,
//! disjoint, half-open ranges of positions with lifetime holes between
//! them, and the positions where instructions read or write it.
//!
//! Instruction `k` (numbered in block layout order) reads at position `2k`
//! and writes at `2k+1`. A value read last by instruction `k` is live up to
//! `2k+1`, exclusive, and one it defines from `2k+1`, so the two may share a
//! register. A move the allocator inserts "before instruction `k`" happens
//! between positions `2k-1` and `2k`, which is why intervals are only ever
//! split at even positions.

use crate::{
    backend::native::mir::{
        PReg,
        RegClass,
        VReg,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// A half-open range of positions.
pub(super) type Range = (u32, u32);

/// One piece of a virtual register's lifetime and where it lives.
pub(super) struct Interval<'a> {
    /// The register, after coalescing its representative.
    pub(super) vreg:         VReg,
    pub(super) class:        RegClass,
    /// Sorted, disjoint and never adjacent.
    pub(super) ranges:       ArenaVec<'a, Range>,
    /// The positions at which instructions read or write the register and
    /// need it in a register, ascending: all but reads of a memory source
    /// operand, which can take the spill slot instead.
    pub(super) uses:         ArenaVec<'a, u32>,
    /// The positions of reads through a memory source operand, which can
    /// take the spill slot but are cheaper from a register, ascending.
    pub(super) weak:         ArenaVec<'a, u32>,
    /// The position this piece starts holding the value from: 0 for a
    /// whole interval, the split position for a piece split off.
    pub(super) from:         u32,
    pub(super) loc:          Loc,
    /// The register a copy or the piece before this one suggests.
    pub(super) hint:         Option<PReg>,
    /// Whether the value is live on both sides of `from` inside a block,
    /// so a move must carry it over from the piece before.
    pub(super) carried:      bool,
    /// Whether the interval must keep one location: it is live into a
    /// block an indirect jump reaches, whose edges cannot get moves.
    pub(super) unsplittable: bool,
}

/// Where an interval lives.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Loc {
    /// Not yet decided.
    None,
    Reg(PReg),
    /// The register's spill slot. A spilled piece with uses has them
    /// rewritten through the scratch registers.
    Spill,
}

impl<'a> Interval<'a> {
    pub(super) fn new(vreg: VReg, class: RegClass, arena: &'a Bump) -> Self {
        Self {
            vreg,
            class,
            ranges: ArenaVec::new_in(arena),
            uses: ArenaVec::new_in(arena),
            weak: ArenaVec::new_in(arena),
            from: 0,
            loc: Loc::None,
            hint: None,
            carried: false,
            unsplittable: false,
        }
    }

    /// The first live position.
    pub(super) fn start(&self) -> u32 {
        self.ranges[0].0
    }

    /// The position after the last live one.
    pub(super) fn end(&self) -> u32 {
        self.ranges[self.ranges.len() - 1].1
    }

    pub(super) fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// Whether the value is live at `position`.
    pub(super) fn covers(&self, position: u32) -> bool {
        let index = self.ranges.partition_point(|&(_, end)| end <= position);
        self.ranges
            .get(index)
            .is_some_and(|&(start, _)| start <= position)
    }

    /// The first use at or after `position`.
    pub(super) fn next_use(&self, position: u32) -> Option<u32> {
        let index = self.uses.partition_point(|&at| at < position);
        self.uses.get(index).copied()
    }

    /// The first use of either kind at or after `position`.
    pub(super) fn next_any_use(&self, position: u32) -> Option<u32> {
        let index = self.weak.partition_point(|&at| at < position);
        match (self.next_use(position), self.weak.get(index).copied()) {
            | (Some(strong), Some(weak)) => Some(strong.min(weak)),
            | (strong, weak) => strong.or(weak),
        }
    }

    /// The first position at or after `from` where both are live.
    pub(super) fn next_intersection(&self, other: &[Range], from: u32) -> Option<u32> {
        next_intersection(&self.ranges, other, from)
    }

    /// Splits the interval at the even position `at`, strictly inside it:
    /// this piece keeps everything before `at`, the returned one the rest.
    pub(super) fn split(&mut self, at: u32, arena: &'a Bump) -> Self {
        debug_assert!(
            at.is_multiple_of(2) && self.start() < at && at < self.end(),
            "splits fall between instructions, inside the interval"
        );
        let carried = self.covers(at - 1) && self.covers(at);
        let index = self.ranges.partition_point(|&(_, end)| end <= at);
        let mut ranges = ArenaVec::with_capacity_in(self.ranges.len() - index, arena);
        let mut rest = index;
        if self.ranges[index].0 < at {
            ranges.push((at, self.ranges[index].1));
            self.ranges[index].1 = at;
            rest += 1;
        }
        ranges.extend_from_slice(&self.ranges[rest..]);
        self.ranges.truncate(rest);
        let first_use = self.uses.partition_point(|&position| position < at);
        let mut uses = ArenaVec::with_capacity_in(self.uses.len() - first_use, arena);
        uses.extend_from_slice(&self.uses[first_use..]);
        self.uses.truncate(first_use);
        let first_weak = self.weak.partition_point(|&position| position < at);
        let mut weak = ArenaVec::with_capacity_in(self.weak.len() - first_weak, arena);
        weak.extend_from_slice(&self.weak[first_weak..]);
        self.weak.truncate(first_weak);
        Self {
            vreg: self.vreg,
            class: self.class,
            ranges,
            uses,
            weak,
            from: at,
            loc: Loc::None,
            hint: match self.loc {
                | Loc::Reg(register) => Some(register),
                | _ => self.hint,
            },
            carried,
            unsplittable: false,
        }
    }

    /// Adds `other`'s ranges and uses, which must not overlap these. The
    /// lists come out as if all of both were merged, ranges that touch
    /// joined and positions deduplicated; once these lists are (`merged`,
    /// after the first join), only their part from just before `other`'s
    /// first element is rebuilt, so joining a short interval to the end of
    /// a long one, as a chain of block-argument copies does, costs the
    /// short one.
    pub(super) fn absorb(&mut self, other: &Self, merged: bool, arena: &'a Bump) {
        merge(
            &mut self.ranges,
            &other.ranges,
            merged,
            |&(start, _)| start,
            push_range,
            arena,
        );
        let push_new = |list: &mut ArenaVec<'_, u32>, position| {
            if list.last() != Some(&position) {
                list.push(position);
            }
        };
        merge(
            &mut self.uses,
            &other.uses,
            merged,
            |&at| at,
            push_new,
            arena,
        );
        merge(
            &mut self.weak,
            &other.weak,
            merged,
            |&at| at,
            push_new,
            arena,
        );
        if self.hint.is_none() {
            self.hint = other.hint;
        }
    }
}

/// Merges the sorted `other` into the sorted `list` by `key`, adding each
/// element with `push`. Unless `merged` says `list` already went through
/// `push`, all of it is rebuilt; otherwise only from the element before
/// the first that sorts after `other`'s first, which `push` would leave as
/// they are.
fn merge<'a, T: Copy>(
    list: &mut ArenaVec<'a, T>,
    other: &[T],
    merged: bool,
    key: impl Fn(&T) -> u32,
    push: impl Fn(&mut ArenaVec<'a, T>, T),
    arena: &'a Bump,
) {
    let start = match other.first() {
        | Some(first) if merged => list
            .partition_point(|item| key(item) < key(first))
            .saturating_sub(1),
        | None if merged => return,
        | _ => 0,
    };
    let mut tail = ArenaVec::with_capacity_in(list.len() - start + other.len(), arena);
    let (mut left, mut right) = (start, 0);
    while left < list.len() || right < other.len() {
        let take_left =
            right == other.len() || (left < list.len() && key(&list[left]) < key(&other[right]));
        let item = if take_left {
            left += 1;
            list[left - 1]
        } else {
            right += 1;
            other[right - 1]
        };
        push(&mut tail, item);
    }
    list.truncate(start);
    list.extend_from_slice(&tail);
}

/// Appends `range` to sorted `ranges`, joining it to the last one when they
/// touch.
pub(super) fn push_range(ranges: &mut ArenaVec<'_, Range>, range: Range) {
    if let Some(last) = ranges.last_mut()
        && range.0 <= last.1
    {
        last.1 = last.1.max(range.1);
        return;
    }
    ranges.push(range);
}

/// The first position at or after `from` in both sorted range lists.
pub(super) fn next_intersection(left: &[Range], right: &[Range], from: u32) -> Option<u32> {
    let mut i = left.partition_point(|&(_, end)| end <= from);
    let mut j = right.partition_point(|&(_, end)| end <= from);
    while i < left.len() && j < right.len() {
        let start = left[i].0.max(right[j].0).max(from);
        if start < left[i].1 && start < right[j].1 {
            return Some(start);
        }
        if left[i].1 <= right[j].1 {
            i += 1;
        } else {
            j += 1;
        }
    }
    None
}

/// Disjoint, sorted ranges per physical register where a precolored value
/// lives or the register is clobbered.
pub(super) struct FixedRanges<'a> {
    pub(super) ranges: [ArenaVec<'a, Range>; PReg::COUNT],
}

impl FixedRanges<'_> {
    /// The first position at or after `from` where `register` is fixed
    /// and `ranges` live.
    pub(super) fn next_intersection(
        &self,
        register: PReg,
        ranges: &[Range],
        from: u32,
    ) -> Option<u32> {
        next_intersection(&self.ranges[register.index()], ranges, from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Entity;

    /// An interval of `vreg` over `ranges`, with no uses.
    fn interval<'a>(vreg: usize, ranges: &[Range], arena: &'a Bump) -> Interval<'a> {
        let mut interval = Interval::new(VReg::new(vreg), RegClass::Gpr, arena);
        interval.ranges.extend_from_slice(ranges);
        interval
    }

    #[test]
    fn absorbing_joins_ranges_that_touch() {
        let arena = Bump::new();
        for merged in [false, true] {
            // The joined interval starts where the representative's last
            // range ends, and ends where its next one would start.
            let mut rep = interval(0, &[(0, 4), (10, 20)], &arena);
            rep.absorb(&interval(1, &[(20, 30)], &arena), merged, &arena);
            assert_eq!(&rep.ranges[..], &[(0, 4), (10, 30)], "merged: {merged}");
            rep.absorb(&interval(2, &[(6, 10)], &arena), true, &arena);
            assert_eq!(&rep.ranges[..], &[(0, 4), (6, 30)], "merged: {merged}");
        }
    }
}
