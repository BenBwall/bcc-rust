//! An arena vector grows by adding segments without relocating elements.
//! Discarding a prefix advances its start; clearing keeps segments for reuse.
//! The context uses it for sparse macro-location endpoints.
//!
//! C99: provenance support for phases 1-7, §5.1.1.2 paragraph 1,
//! pp. 9-10; PDF pp. 21-22. This storage implements no language constraint.

use super::{
    ArenaVec,
    Bump,
};

/// A vector in an arena that grows by adding segments, each twice as long as
/// the one before, so it never moves or copies its elements and leaves no
/// outgrown buffers in the arena. Removing from the front only advances its
/// start, and clearing keeps the segments for reuse, as a `Vec` keeps its
/// capacity.
pub(super) struct SegmentedVec<'a, T> {
    pub(super) arena:    &'a Bump,
    pub(super) segments: ArenaVec<'a, ArenaVec<'a, T>>,
    /// The position of the first element.
    pub(super) start:    usize,
    /// The position after the last element.
    pub(super) end:      usize,
    /// The segment that holds the last element, or 0 when empty.
    pub(super) tail:     usize,
}

impl<'a, T: Clone> SegmentedVec<'a, T> {
    pub(super) fn push(&mut self, value: T) {
        // The tail segment takes the element unless it is full.
        let segment = match self.segments.get(self.tail) {
            | Some(tail) if self.end == 0 || tail.len() < Self::FIRST_SEGMENT << self.tail =>
                self.tail,
            | Some(_) => self.tail + 1,
            | None => 0,
        };
        if segment == self.segments.len() {
            self.segments.push(ArenaVec::with_capacity_in(
                Self::FIRST_SEGMENT << segment,
                self.arena,
            ));
        }
        debug_assert_eq!(
            Self::locate(self.end),
            (segment, self.segments[segment].len()),
            "segments fill in order"
        );
        self.segments[segment].push(value);
        self.tail = segment;
        self.end += 1;
    }

    /// Inserts `value` at `index`, shifting the elements after it.
    pub(super) fn insert(&mut self, index: usize, value: T) {
        let Some(last) = self.last().cloned() else {
            self.push(value);
            return;
        };
        self.push(last);
        for position in (index + 1..self.len() - 1).rev() {
            let previous = self[position - 1].clone();
            self[position] = previous;
        }
        self[index] = value;
    }

    /// Removes the first `count` elements.
    pub(super) fn discard_front(&mut self, count: usize) {
        self.start += count.min(self.len());
        if self.is_empty() {
            self.clear();
        }
    }

    /// The number of leading elements for which `predicate` holds, which must
    /// hold for a prefix.
    pub(super) fn partition_point(&self, mut predicate: impl FnMut(&T) -> bool) -> usize {
        let mut count = 0;
        for slice in self.slices() {
            match slice.last() {
                | Some(last) if predicate(last) => count += slice.len(),
                | _ => return count + slice.partition_point(&mut predicate),
            }
        }
        count
    }

    /// The elements in order, one slice per segment.
    pub(super) fn slices(&self) -> impl Iterator<Item = &[T]> {
        let (first, offset) = Self::locate(self.start);
        let segments = if self.is_empty() {
            &[][..]
        } else {
            &self.segments[first..=self.tail]
        };
        segments.iter().enumerate().map(move |(index, segment)| {
            if index == 0 {
                &segment[offset..]
            } else {
                &segment[..]
            }
        })
    }

    pub(super) fn for_each_mut(&mut self, mut visit: impl FnMut(&mut T)) {
        if self.is_empty() {
            return;
        }
        let (first, offset) = Self::locate(self.start);
        for (index, segment) in self.segments[first..=self.tail].iter_mut().enumerate() {
            let skip = if index == 0 { offset } else { 0 };
            segment[skip..].iter_mut().for_each(&mut visit);
        }
    }

    /// The segment holding `position`, and the offset there.
    pub(super) fn locate(position: usize) -> (usize, usize) {
        let blocks = position / Self::FIRST_SEGMENT + 1;
        let segment = blocks.ilog2() as usize;
        (
            segment,
            position - Self::FIRST_SEGMENT * ((1 << segment) - 1),
        )
    }

    pub(super) fn len(&self) -> usize {
        self.end - self.start
    }

    pub(super) fn last(&self) -> Option<&T> {
        if self.is_empty() {
            None
        } else {
            self.segments[self.tail].last()
        }
    }

    pub(super) fn last_mut(&mut self) -> Option<&mut T> {
        if self.is_empty() {
            None
        } else {
            self.segments[self.tail].last_mut()
        }
    }

    pub(super) fn clear(&mut self) {
        if self.end == 0 {
            return;
        }
        for segment in &mut self.segments[..=self.tail] {
            segment.clear();
        }
        self.start = 0;
        self.end = 0;
        self.tail = 0;
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &T> {
        self.slices().flatten()
    }

    pub(super) fn new_in(arena: &'a Bump) -> Self {
        Self {
            arena,
            segments: ArenaVec::new_in(arena),
            start: 0,
            end: 0,
            tail: 0,
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.start == self.end
    }

    pub(super) fn get(&self, index: usize) -> Option<&T> {
        (index < self.len()).then(|| {
            let (segment, offset) = Self::locate(self.start + index);
            &self.segments[segment][offset]
        })
    }

    pub(super) fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        (index < self.len()).then(|| {
            let (segment, offset) = Self::locate(self.start + index);
            &mut self.segments[segment][offset]
        })
    }
}

impl<T: Clone> SegmentedVec<'_, T> {
    /// The length of the first segment.
    pub(super) const FIRST_SEGMENT: usize = 16;
}

impl<T: Clone> std::ops::Index<usize> for SegmentedVec<'_, T> {
    type Output = T;

    fn index(&self, index: usize) -> &T {
        self.get(index)
            .expect("segmented vector index out of bounds")
    }
}

impl<T: Clone> std::ops::IndexMut<usize> for SegmentedVec<'_, T> {
    fn index_mut(&mut self, index: usize) -> &mut T {
        self.get_mut(index)
            .expect("segmented vector index out of bounds")
    }
}
