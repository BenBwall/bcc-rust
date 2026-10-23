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

/// Current writers indexed by bit position. Uniform nodes answer an overlap
/// query directly; partial updates split only the affected path, at most 64
/// levels for u64 offsets. Copies query a snapshot before writing destinations.
pub(crate) struct Writers<'a> {
    nodes: ArenaVec<'a, WriterNode>,
    end:   u64,
}

impl<'a> Writers<'a> {
    pub(crate) fn new(end: u64, arena: &'a Bump) -> Self {
        let mut nodes = ArenaVec::new_in(arena);
        nodes.push(WriterNode {
            writer:   None,
            children: None,
        });
        Self { nodes, end }
    }

    pub(crate) fn write(&mut self, start: u64, end: u64, writer: Option<usize>) {
        if start < end {
            self.update(0, 0, self.end, start, end, writer);
        }
    }

    fn update(
        &mut self,
        node: usize,
        low: u64,
        high: u64,
        start: u64,
        end: u64,
        writer: Option<usize>,
    ) {
        if start >= high || end <= low {
            return;
        }
        if self.nodes[node].children.is_none() && self.nodes[node].writer == writer {
            return;
        }
        if start <= low && high <= end {
            self.nodes[node] = WriterNode {
                writer,
                children: None,
            };
            return;
        }
        let middle = low + (high - low) / 2;
        let children = if let Some(children) = self.nodes[node].children {
            children
        } else {
            let first = self.nodes.len();
            let uniform = self.nodes[node];
            self.nodes.push(uniform);
            self.nodes.push(uniform);
            let children = [first, first + 1];
            self.nodes[node].children = Some(children);
            children
        };
        self.update(children[0], low, middle, start, end, writer);
        self.update(children[1], middle, high, start, end, writer);
    }

    pub(crate) fn source<'s>(
        &self,
        start: u64,
        end: u64,
        arena: &'s Bump,
    ) -> ArenaVec<'s, (u64, u64, Option<usize>)> {
        let mut result: ArenaVec<'_, (u64, u64, Option<usize>)> = ArenaVec::new_in(arena);
        let mut work = ArenaVec::new_in(arena);
        work.push((0, 0, self.end));
        while let Some((node, low, high)) = work.pop() {
            if start >= high || end <= low {
                continue;
            }
            if let Some([left, right]) = self.nodes[node].children {
                let middle = low + (high - low) / 2;
                work.push((right, middle, high));
                work.push((left, low, middle));
            } else {
                let low = low.max(start);
                let high = high.min(end);
                let writer = self.nodes[node].writer;
                if let Some(last) = result.last_mut()
                    && last.1 == low
                    && last.2 == writer
                {
                    last.1 = high;
                } else {
                    result.push((low, high, writer));
                }
            }
        }
        result
    }
}

#[derive(Clone, Copy)]
struct WriterNode {
    writer:   Option<usize>,
    children: Option<[usize; 2]>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_writers_match_overwrites_and_copy_snapshots() {
        let arena = Bump::new();
        let mut writers = Writers::new(100, &arena);
        let mut expected = [None; 100];
        for index in 0..200 {
            let start = index * 37 % 100;
            let end = (start + index * 13 % 30).min(100);
            let value = (index % 3 != 0).then_some(index);
            writers.write(start as u64, end as u64, value);
            expected[start..end].fill(value);
            let runs = writers.source(0, 100, &arena);
            for (at, expected) in expected.iter().enumerate() {
                let actual = runs
                    .iter()
                    .find(|&&(low, high, _)| low <= at as u64 && (at as u64) < high)
                    .unwrap()
                    .2;
                assert_eq!(actual, *expected, "update {index}, bit {at}");
            }
        }
        let snapshot = writers.source(0, 10, &arena);
        writers.write(0, 10, Some(999));
        for (low, high, value) in snapshot {
            writers.write(low + 50, high + 50, value);
        }
        let copied = writers.source(50, 60, &arena);
        for (at, expected) in expected[..10].iter().enumerate() {
            let actual = copied
                .iter()
                .find(|&&(low, high, _)| low <= at as u64 + 50 && at as u64 + 50 < high)
                .unwrap()
                .2;
            assert_eq!(actual, *expected, "copied bit {at}");
        }
    }

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
