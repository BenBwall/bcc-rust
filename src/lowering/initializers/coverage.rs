//! Indexes initializer leaf ranges for subobject reset coverage. Sorted starts
//! and a tree of maximum stops prune disjoint subtrees; an explicit search
//! stack returns overlapping leaves in source order. Storage is linear in
//! leaf count, and a query visits only overlapping leaves and tree paths.
//!
//! C99: §6.7.8 paragraph 19, p. 127; PDF p. 139 (explicit subobjects keep
//! their bytes until their initializers run; omitted subobjects are zeroed).

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
        ArenaVec,
        Bump,
    },
};

/// Bit ranges of leaves, indexed by start and subtree maximum stop.
pub(super) struct CoverageIndex<'s> {
    ranges: ArenaVec<'s, Coverage>,
    stops:  ArenaVec<'s, u64>,
    base:   usize,
}

impl<'s> CoverageIndex<'s> {
    /// Collects overlapping bit ranges, sorting them back into source order
    /// so enclosing braces and later union selections still delimit coverage.
    pub(super) fn overlapping(
        &self,
        start: u64,
        stop: u64,
        candidates: &mut ArenaVec<'s, Coverage>,
        search: &mut ArenaVec<'s, (usize, usize, usize)>,
    ) {
        candidates.clear();
        search.clear();
        search.push((1, 0, self.base));
        while let Some((node, low, high)) = search.pop() {
            if low >= self.ranges.len()
                || self.ranges[low].start >= stop
                || self.stops[node] <= start
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
        candidates.sort_unstable_by_key(|range| range.index);
    }

    /// Computes each leaf's layout once, then builds the maximum-stop tree
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
        ranges.sort_unstable_by_key(|range| range.start);
        let base = ranges.len().next_power_of_two();
        let mut stops = ArenaVec::with_capacity_in(2 * base, arena);
        stops.resize(2 * base, 0);
        for (index, range) in ranges.iter().enumerate() {
            stops[base + index] = range.stop;
        }
        for node in (1..base).rev() {
            stops[node] = stops[2 * node].max(stops[2 * node + 1]);
        }
        Ok(Self {
            ranges,
            stops,
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
