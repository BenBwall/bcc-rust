//! Indexes initializer leaves in source order for subobject reset coverage.
//! Queries restrict the source interval before collecting leaves. An offset
//! index handles scattered designators; if that search exceeds the source
//! interval's budget, a source index prunes with minimum starts and maximum
//! stops instead. A reverse scan records the
//! next reset of each union range, so repeated member switches never collect
//! superseded leaves. Storage is linear; queries visit at most their source
//! interval and tree paths, using an explicit stack.
//!
//! C99: §6.7.8 paragraph 19, p. 127; PDF p. 139 (explicit subobjects keep
//! their bytes until their initializers run; omitted subobjects are zeroed).

use std::ops::Range;

use super::{
    Leaf,
    LoweringError,
    at,
    types,
};
use crate::{
    translation_phases::{
        SourceVectors,
        semantic_analysis::{
            SemanticTranslationUnit,
            TypeId,
        },
    },
    util::bump::{
        ArenaMap,
        ArenaVec,
        Bump,
    },
};

/// Source-ordered bit ranges with subtree bounds and following union resets.
pub(super) struct CoverageIndex<'s> {
    ranges:      ArenaVec<'s, Coverage>,
    bounds:      ArenaVec<'s, (u64, u64)>,
    spatial:     ArenaVec<'s, Coverage>,
    stops:       ArenaVec<'s, u64>,
    next_resets: ArenaVec<'s, usize>,
    base:        usize,
}

impl<'s> CoverageIndex<'s> {
    /// Collects spatial overlaps only inside the given source interval, in
    /// source order. The offset search is bounded by that interval's length,
    /// so superseded overlapping selections cannot make it quadratic.
    pub(super) fn overlapping(
        &self,
        start: u64,
        stop: u64,
        source: Range<usize>,
        candidates: &mut ArenaVec<'s, Coverage>,
        search: &mut ArenaVec<'s, (usize, usize, usize)>,
    ) {
        if self.overlapping_by_offset(start, stop, &source, candidates, search) {
            candidates.sort_unstable_by_key(|range| range.index);
            return;
        }
        candidates.clear();
        search.clear();
        search.push((1, 0, self.base));
        while let Some((node, low, high)) = search.pop() {
            if high <= source.start
                || low >= source.end
                || self.bounds[node].0 >= stop
                || self.bounds[node].1 <= start
            {
                continue;
            }
            if high - low == 1 {
                candidates.push(self.ranges[low]);
            } else {
                let middle = low + (high - low) / 2;
                search.push((2 * node + 1, middle, high));
                search.push((2 * node, low, middle));
            }
        }
    }

    /// The next selection of the same union byte range, or the list's end.
    pub(super) fn next_reset(&self, index: usize) -> usize {
        self.next_resets[index]
    }

    /// Preserves spatial pruning even when source order jumps between distant
    /// array elements. Abort before doing more work than the source search;
    /// 64 additional node visits cover the boundary paths of a u32 leaf count.
    fn overlapping_by_offset(
        &self,
        start: u64,
        stop: u64,
        source: &Range<usize>,
        candidates: &mut ArenaVec<'s, Coverage>,
        search: &mut ArenaVec<'s, (usize, usize, usize)>,
    ) -> bool {
        candidates.clear();
        search.clear();
        search.push((1, 0, self.base));
        let mut budget = source.len().saturating_add(64);
        while let Some((node, low, high)) = search.pop() {
            if budget == 0 {
                return false;
            }
            budget -= 1;
            if low >= self.spatial.len()
                || self.spatial[low].start >= stop
                || self.stops[node] <= start
            {
                continue;
            }
            if high - low == 1 {
                let range = self.spatial[low];
                if source.contains(&range.index) {
                    candidates.push(range);
                }
            } else {
                let middle = low + (high - low) / 2;
                search.push((2 * node + 1, middle, high));
                search.push((2 * node, low, middle));
            }
        }
        true
    }

    /// Computes each leaf's layout once, then builds the source-interval tree
    /// bottom-up without recursing over the initializer or its types.
    pub(super) fn new(
        leaves: &[Leaf<'_>],
        sema: &SemanticTranslationUnit<'_>,
        source: SourceVectors,
        arena: &'s Bump,
    ) -> Result<Self, LoweringError> {
        let mut ranges = ArenaVec::with_capacity_in(leaves.len(), arena);
        for (index, &leaf) in leaves.iter().enumerate() {
            let layout = |ty| types::layout(&sema.types, ty).map_err(|kind| at(kind, source));
            let (start, stop, ty) = match leaf {
                | Leaf::Zero {
                    offset, size, ty, ..
                }
                | Leaf::String {
                    offset, size, ty, ..
                } => (8 * offset, 8 * (offset + size), ty),
                | Leaf::Scalar {
                    offset,
                    ty,
                    bit_field: Some(bits),
                    ..
                } => {
                    let start = 8 * offset + u64::from(bits.bit_offset);
                    (start, start + u64::from(bits.width), ty)
                },
                | Leaf::Scalar { offset, ty, .. } | Leaf::Copy { offset, ty, .. } =>
                    (8 * offset, 8 * (offset + layout(ty)?.size), ty),
            };
            ranges.push(Coverage {
                index,
                start,
                stop,
                ty,
            });
        }
        let base = ranges.len().next_power_of_two();
        let mut bounds = ArenaVec::with_capacity_in(2 * base, arena);
        bounds.resize(2 * base, (u64::MAX, 0));
        for (index, range) in ranges.iter().enumerate() {
            bounds[base + index] = (range.start, range.stop);
        }
        for node in (1..base).rev() {
            bounds[node] = (
                bounds[2 * node].0.min(bounds[2 * node + 1].0),
                bounds[2 * node].1.max(bounds[2 * node + 1].1),
            );
        }
        let mut spatial = ArenaVec::with_capacity_in(ranges.len(), arena);
        spatial.extend(ranges.iter().copied());
        spatial.sort_unstable_by_key(|range| range.start);
        let mut stops = ArenaVec::with_capacity_in(2 * base, arena);
        stops.resize(2 * base, 0);
        for (index, range) in spatial.iter().enumerate() {
            stops[base + index] = range.stop;
        }
        for node in (1..base).rev() {
            stops[node] = stops[2 * node].max(stops[2 * node + 1]);
        }
        let mut next_resets = ArenaVec::with_capacity_in(leaves.len(), arena);
        next_resets.resize(leaves.len(), leaves.len());
        let mut resets = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena);
        for (index, &leaf) in leaves.iter().enumerate().rev() {
            if let Leaf::Zero {
                offset,
                size,
                end: usize::MAX,
                ..
            } = leaf
                && let Some(next) = resets.insert((offset, size), index)
            {
                next_resets[index] = next;
            }
        }
        Ok(Self {
            ranges,
            bounds,
            spatial,
            stops,
            next_resets,
            base,
        })
    }
}

/// One leaf's bit coverage, with its source position and destination type.
#[derive(Clone, Copy)]
pub(super) struct Coverage {
    pub(super) index: usize,
    pub(super) start: u64,
    pub(super) stop:  u64,
    pub(super) ty:    TypeId,
}
