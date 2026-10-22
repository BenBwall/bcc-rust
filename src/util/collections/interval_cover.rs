//! Last-write coverage of source-ordered intervals. A sweep visits sorted
//! endpoints, maintaining the greatest active source index in a segment tree.
//! Construction costs O(n log n) time and O(n) storage regardless of write
//! order; the result is sorted, disjoint and coalesces adjacent equal writers.

use crate::util::bump::{
    ArenaVec,
    Bump,
};

/// The final writer of every covered interval in `ranges`, in spatial order.
/// Empty ranges write nothing. The third result component indexes `ranges`.
pub(crate) fn last_writes<'a>(
    ranges: &[(u64, u64)],
    arena: &'a Bump,
) -> ArenaVec<'a, (u64, u64, usize)> {
    let mut events = ArenaVec::with_capacity_in(2 * ranges.len(), arena);
    for (index, &(start, end)) in ranges.iter().enumerate() {
        if start < end {
            events.push((start, index, true));
            events.push((end, index, false));
        }
    }
    events.sort_unstable();
    let base = ranges.len().next_power_of_two();
    let mut active = ArenaVec::with_capacity_in(2 * base, arena);
    active.resize(2 * base, 0usize);
    let mut cover: ArenaVec<'_, (u64, u64, usize)> = ArenaVec::new_in(arena);
    let mut previous = 0;
    for (at, index, begins) in events {
        if previous < at && active[1] != 0 {
            let writer = active[1] - 1;
            if let Some(last) = cover.last_mut()
                && last.1 == previous
                && last.2 == writer
            {
                last.1 = at;
            } else {
                cover.push((previous, at, writer));
            }
        }
        let mut node = base + index;
        active[node] = if begins { index + 1 } else { 0 };
        while node > 1 {
            node /= 2;
            active[node] = active[2 * node].max(active[2 * node + 1]);
        }
        previous = at;
    }
    cover
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_matches_last_store_for_overlapping_and_empty_ranges() {
        let arena = Bump::new();
        let ranges = [
            (0, 16),
            (12, 16),
            (8, 12),
            (4, 8),
            (0, 4),
            (2, 14),
            (4, 4),
            (5, 9),
        ];
        let cover = last_writes(&ranges, &arena);
        for at in 0..20 {
            let expected = ranges
                .iter()
                .rposition(|&(start, end)| start <= at && at < end);
            let actual = cover
                .iter()
                .find(|&&(start, end, _)| start <= at && at < end)
                .map(|&(_, _, writer)| writer);
            assert_eq!(actual, expected, "at {at}");
        }
        assert!(last_writes(&[], &arena).is_empty());
    }
}
