//! Bits already replaced by later initializer operations. A fixed-depth radix
//! tree unions intervals without scanning earlier writes. Queries intersect a
//! periodic run arithmetically, so a scalar GNU range stays one operation.
//!
//! C99: §6.7.8p19, p. 127; PDF p. 139 (later initializers replace earlier
//! ones).

use super::Work;
use crate::util::bump::{
    ArenaVec,
    Bump,
};

/// A run of bits, repeated without expanding its elements.
#[derive(Clone, Copy)]
pub(super) struct Pattern {
    pub(super) start:  u64,
    pub(super) end:    u64,
    pub(super) count:  u64,
    pub(super) stride: u64,
}

impl Pattern {
    fn intersects(self, low: u128, high: u128) -> bool {
        let start = u128::from(self.start);
        let end = u128::from(self.end);
        if start == end || self.count == 0 {
            return false;
        }
        let index = if low >= end {
            if self.stride == 0 {
                return false;
            }
            (low - end) / u128::from(self.stride) + 1
        } else {
            0
        };
        index < u128::from(self.count) && start + index * u128::from(self.stride) < high
    }
}

/// Initially uncovered bits; a node is either uniform or has two children.
/// Address bits bound recursion to 64, independently of source nesting.
pub(super) struct Coverage<'a>(ArenaVec<'a, Node>);

impl<'a> Coverage<'a> {
    pub(super) fn new(arena: &'a Bump) -> Self {
        let mut nodes = ArenaVec::new_in(arena);
        nodes.push(Node::EMPTY);
        Self(nodes)
    }

    pub(super) fn cover(&mut self, start: u64, end: u64, work: &mut Work) -> Option<()> {
        self.insert(0, 0, 1_u128 << 64, u128::from(start), u128::from(end), work)
    }

    pub(super) fn any(&self, pattern: Pattern, covered: bool, work: &mut Work) -> Option<bool> {
        self.query(0, 0, 1_u128 << 64, pattern, covered, work)
    }

    fn insert(
        &mut self,
        node: usize,
        low: u128,
        high: u128,
        start: u128,
        end: u128,
        work: &mut Work,
    ) -> Option<()> {
        work.charge(1)?;
        if self.0[node].covered || end <= low || start >= high {
            return Some(());
        }
        if start <= low && end >= high {
            self.0[node].covered = true;
            return Some(());
        }
        let children = if let Some(children) = self.0[node].children {
            children
        } else {
            let children = self.0.len();
            self.0.push(Node::EMPTY);
            self.0.push(Node::EMPTY);
            self.0[node].children = Some(children);
            children
        };
        let middle = low + (high - low) / 2;
        self.insert(children, low, middle, start, end, work)?;
        self.insert(children + 1, middle, high, start, end, work)?;
        self.0[node].covered = self.0[children].covered && self.0[children + 1].covered;
        Some(())
    }

    fn query(
        &self,
        node: usize,
        low: u128,
        high: u128,
        pattern: Pattern,
        covered: bool,
        work: &mut Work,
    ) -> Option<bool> {
        work.charge(1)?;
        if !pattern.intersects(low, high) {
            return Some(false);
        }
        if self.0[node].covered {
            return Some(covered);
        }
        let Some(children) = self.0[node].children else {
            return Some(!covered);
        };
        let middle = low + (high - low) / 2;
        if self.query(children, low, middle, pattern, covered, work)? {
            return Some(true);
        }
        self.query(children + 1, middle, high, pattern, covered, work)
    }
}

#[derive(Clone, Copy)]
struct Node {
    covered:  bool,
    children: Option<usize>,
}

impl Node {
    const EMPTY: Self = Self {
        covered:  false,
        children: None,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periodic_coverage_matches_individual_bits() {
        let arena = Bump::new();
        let mut coverage = Coverage::new(&arena);
        let mut work = Work::new(100_000);
        let mut bits = [false; 128];
        for (start, end) in [(0, 3), (5, 8), (16, 32), (30, 47), (63, 65), (95, 96)] {
            coverage.cover(start, end, &mut work).unwrap();
            bits[start as usize..end as usize].fill(true);
        }
        for start in 0..20 {
            for stride in 1..16 {
                for width in 1..=stride {
                    for count in 0..7 {
                        let pattern = Pattern {
                            start,
                            end: start + width,
                            stride,
                            count,
                        };
                        for covered in [false, true] {
                            let expected = (0..count).any(|index| {
                                bits[(start + index * stride) as usize
                                    ..(start + index * stride + width) as usize]
                                    .iter()
                                    .any(|&bit| bit == covered)
                            });
                            assert_eq!(coverage.any(pattern, covered, &mut work), Some(expected));
                        }
                    }
                }
            }
        }
    }
}
