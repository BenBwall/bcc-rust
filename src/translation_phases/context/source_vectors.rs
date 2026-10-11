//! Retains and merges source ranges across phase storage lifetimes.
//! Preprocessing ranges can be compacted after token ranges and diagnostic
//! ranges have moved to retained storage.
//!
//! C99: provenance for phases 1-7, §5.1.1.2 paragraph 1, pp. 9-10;
//! PDF pp. 21-22. Diagnostic locations: §5.1.1.3 paragraph 1, p. 11;
//! PDF p. 23. This module preserves locations rather than checking grammar.

use super::{
    ArenaQueue,
    Bump,
    Context,
    GetPosition,
    MergeAnchors,
    RegionVec,
    SourceArena,
    SourcePosition,
    SourceVector,
    SourceVectors,
};

impl Context<'_> {
    /// Copies a phase-6 output token's provenance into the parser-token
    /// provenance once, so provenance of consecutive tokens is adjacent and
    /// survives [`Self::compact_preprocessor_vectors`].
    pub(crate) fn retain_token_source(&mut self, source_vectors: SourceVectors) -> SourceVectors {
        if source_vectors.length() == 0
            || SourceArena::decode(source_vectors).0 != SourceArena::Preprocessor
        {
            return source_vectors;
        }
        let (start, end) = Self::checked_source_append(
            SourceArena::ParserTokens,
            self.parser_token_vectors.len(),
            self.parser_token_vectors.len(),
            source_vectors.length() as usize,
        );
        self.copy_into(SourceArena::ParserTokens, source_vectors);
        SourceArena::ParserTokens.encode(start, end)
    }

    /// Discards the preprocessor provenance arena once nothing but pending
    /// diagnostics can still refer to it. Those ranges move to the retained
    /// arena first, so diagnostics read the same vectors afterwards.
    pub(crate) fn compact_preprocessor_vectors(&mut self) {
        if self.source_vectors.0.is_empty() {
            return;
        }
        let mut pending_errors =
            std::mem::replace(&mut self.pending_errors, ArenaQueue::new_in(self.tu));
        for error in pending_errors.iter_mut_from(self.relocated_errors) {
            error.for_each_source_vectors_mut(&mut |source_vectors| {
                *source_vectors = self.retain_preprocessor_range(*source_vectors);
            });
        }
        self.relocated_errors = pending_errors.len();
        self.pending_errors = pending_errors;
        self.source_vectors.0.clear();
        self.expansion_sites[SourceArena::Preprocessor as usize].clear();
    }

    /// Joins two provenance ranges, preserving `v1`'s vectors followed by
    /// `v2`'s.
    ///
    /// Ranges adjacent in one arena merge without copying. Otherwise the
    /// result is copied into the preprocessor arena when both inputs live
    /// there, and into the parser merge arena when either is parser-owned; a
    /// left range already ending at that arena's tail is extended in place.
    ///
    /// A parser anchor (see [`Self::is_parser_anchor`]) after a nonempty
    /// range, or before one that starts where it points, adds nothing and is
    /// dropped. Keeping it would break adjacency, so every enclosing node of
    /// recovered syntax would copy its whole provenance again.
    pub(crate) fn merge_vectors(&mut self, v1: SourceVectors, v2: SourceVectors) -> SourceVectors {
        if v1.length() == 0 {
            return v2;
        }
        if v2.length() == 0 {
            return v1;
        }
        if self.is_parser_anchor(v2) && !self.is_parser_anchor(v1) {
            return v1;
        }
        if self.is_parser_anchor(v1) && self.anchors_start_of(v1, v2) {
            return v2;
        }
        let (arena1, start1) = SourceArena::decode(v1);
        let (arena2, start2) = SourceArena::decode(v2);
        let end1 = start1 + v1.length();
        if arena1 == arena2 && end1 == start2 {
            return arena1.encode(start1, start2 + v2.length());
        }
        let target = Self::merge_target(arena1 == SourceArena::Preprocessor && arena2 == arena1);
        let current_len = self.arena(target).len();
        let extend_left = arena1 == target && end1 as usize == current_len;
        let range_start = if extend_left {
            start1 as usize
        } else {
            current_len
        };
        let additional = if extend_left {
            v2.length() as usize
        } else {
            (v1.length() as usize)
                .checked_add(v2.length() as usize)
                .expect("merged source range length overflow")
        };
        let (start, end) =
            Self::checked_source_append(target, range_start, current_len, additional);
        if !extend_left {
            self.copy_into(target, v1);
        }
        self.copy_into(target, v2);
        target.encode(start, end)
    }

    /// Joins an ordered list of exact source segments in one allocation.
    /// List-owning parser frames defer this until reduction so a growing
    /// prefix is not copied once for every child.
    ///
    /// Parser anchors are dropped as in [`Self::merge_vectors`]: after the
    /// first nonempty range, or before the range they point at.
    pub(crate) fn merge_vector_list(&mut self, sources: &[SourceVectors]) -> SourceVectors {
        let anchors = MergeAnchors::new(self, sources);
        let mut kept = anchors;
        let mut first = None;
        let mut end = 0;
        let mut copied_length = 0usize;
        let mut contiguous = true;
        let mut all_preprocessor = true;
        for &source in sources {
            if !kept.keeps(self, source) {
                continue;
            }
            let (arena, start) = SourceArena::decode(source);
            all_preprocessor &= arena == SourceArena::Preprocessor;
            match first {
                | None => first = Some((arena, start)),
                | Some((first_arena, _)) if first_arena == arena && end == start => {},
                | Some(_) => contiguous = false,
            }
            end = start
                .checked_add(source.length())
                .expect("source range overflow");
            copied_length = copied_length
                .checked_add(source.length() as usize)
                .expect("merged source range length overflow");
        }
        let Some((first_arena, first_start)) = first else {
            return SourceVectors::default();
        };
        if contiguous {
            return first_arena.encode(first_start, end);
        }
        let target = Self::merge_target(all_preprocessor);
        let (start, end) = Self::checked_source_append(
            target,
            self.arena(target).len(),
            self.arena(target).len(),
            copied_length,
        );
        let mut kept = anchors;
        for &source in sources {
            if kept.keeps(self, source) {
                self.copy_into(target, source);
            }
        }
        target.encode(start, end)
    }

    /// Appends the vectors of `source` to `target`.
    pub(super) fn copy_into(&mut self, target: SourceArena, source: SourceVectors) {
        let (_, end) = Self::checked_source_append(
            target,
            self.arena(target).len(),
            self.arena(target).len(),
            source.length() as usize,
        );
        let (arena, start) = SourceArena::decode(source);
        let range = start as usize..(start + source.length()) as usize;
        if source.length() != 0
            && let Some(site) =
                self.expansion_sites[arena as usize].get(start + source.length() - 1)
        {
            let end = end - 1;
            let site = if arena == target {
                site
            } else {
                let location = self.expansion_sites[arena as usize].ends[site as usize].clone();
                self.expansion_sites[target as usize].site_id(location)
            };
            self.expansion_sites[target as usize].insert(end, site);
        }
        let (source, target) = match (arena, target) {
            | (SourceArena::Preprocessor, SourceArena::Preprocessor) => {
                self.source_vectors.0.extend_from_within(range);
                return;
            },
            | (SourceArena::Retained, SourceArena::Retained) => {
                self.retained_vectors.extend_from_within(range);
                return;
            },
            | (SourceArena::Preprocessor, SourceArena::ParserTokens) => (
                &self.source_vectors.0[range],
                &mut self.parser_token_vectors,
            ),
            | (SourceArena::Preprocessor, SourceArena::Retained) =>
                (&self.source_vectors.0[range], &mut self.retained_vectors),
            | (SourceArena::ParserTokens, SourceArena::Retained) => (
                &self.parser_token_vectors[range],
                &mut self.retained_vectors,
            ),
            | _ => unreachable!("parser provenance is never copied back into earlier arenas"),
        };
        target.extend_from_slice(source);
    }

    pub(crate) fn record_expansion_end(&mut self, source: SourceVectors, site: SourceVector) {
        if source.length() != 0 {
            let (arena, start) = SourceArena::decode(source);
            let sites = &mut self.expansion_sites[arena as usize];
            let id = sites.site_id(site);
            sites.insert(start + source.length() - 1, id);
        }
    }

    /// Where a source range ends in the user's input, before macro expansion.
    pub(crate) fn user_source_end(&self, source: SourceVectors) -> Option<SourceVector> {
        if source.length() == 0 {
            return None;
        }
        let (arena, start) = SourceArena::decode(source);
        self.expansion_sites[arena as usize]
            .get(start + source.length() - 1)
            .map(|id| self.expansion_sites[arena as usize].ends[id as usize].clone())
            .or_else(|| self.get_source_vectors(source).last().cloned())
    }

    /// Hint metadata is needed only by the active external declaration. Keep
    /// `previous` and prefetched tokens, but no endpoints from completed
    /// syntax.
    pub(crate) fn discard_completed_macro_locations(&mut self, previous: Option<SourceVectors>) {
        if self.expansion_sites[SourceArena::Retained as usize]
            .entries
            .is_empty()
            && self.expansion_sites[SourceArena::ParserTokens as usize]
                .entries
                .is_empty()
        {
            return;
        }
        self.expansion_sites[SourceArena::Retained as usize].clear();
        if let Some(previous) = previous
            && previous.length() != 0
        {
            let (arena, start) = SourceArena::decode(previous);
            if arena == SourceArena::ParserTokens {
                self.expansion_sites[arena as usize].discard_before(start + previous.length() - 1);
            }
        }
    }

    /// How many macro invocation hints are still recorded.
    #[cfg(test)]
    pub(crate) fn macro_hint_entries(&self) -> usize {
        self.expansion_sites
            .iter()
            .map(|sites| sites.entries.len())
            .sum()
    }

    pub(crate) fn push_source_vector(
        &mut self,
        start_position: SourcePosition,
        source_file_index: u32,
        length: usize,
    ) -> u32 {
        self.push_source_vector_value(SourceVector::new(start_position, source_file_index, length))
    }

    /// The caller read this span from a `LexedFile`, which checked its entire
    /// original source fits in `u32` before lexing. Its offsets and lengths
    /// therefore fit without repeated per-token conversion checks.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "LexedFile checked the source length and every span lies within that source"
    )]
    pub(crate) fn push_lexed_source_vector(
        &mut self,
        start_position: SourcePosition,
        source_file_index: u32,
        length: usize,
    ) -> u32 {
        self.push_source_vector_value(SourceVector {
            index: start_position.index as u32,
            column: start_position.column,
            line: start_position.line,
            source_file_index,
            length: length as u32,
        })
    }

    pub(super) fn push_source_vector_value(&mut self, vector: SourceVector) -> u32 {
        let (index, _) = Self::checked_source_append(
            SourceArena::Preprocessor,
            self.source_vectors.0.len(),
            self.source_vectors.0.len(),
            1,
        );
        self.source_vectors.0.push(vector);
        index
    }

    pub(crate) fn duplicate_source_vectors(
        self_source_vectors: &mut RegionVec<SourceVector>,
        source_vectors: SourceVectors,
    ) -> u32 {
        let (start_index, _) = Self::checked_source_append(
            SourceArena::Preprocessor,
            self_source_vectors.len(),
            self_source_vectors.len(),
            source_vectors.length() as usize,
        );
        let start = source_vectors.start_index();
        for i in start..start + source_vectors.length() {
            self_source_vectors.push(self_source_vectors[i as usize].clone());
        }
        start_index
    }

    /// Copies owned provenance back into the preprocessor arena.
    pub(crate) fn push_source_vectors(&mut self, vectors: &[SourceVector]) -> SourceVectors {
        let (start, end) = Self::checked_source_append(
            SourceArena::Preprocessor,
            self.source_vectors.0.len(),
            self.source_vectors.0.len(),
            vectors.len(),
        );
        self.source_vectors.0.extend_from_slice(vectors);
        SourceArena::Preprocessor.encode(start, end)
    }

    pub(crate) fn create_source_vectors(
        &mut self,
        start_position: SourcePosition,
        source_file_index: u32,
        length: usize,
    ) -> SourceVectors {
        let start_index = self.push_source_vector(start_position, source_file_index, length);
        SourceVectors::new(start_index, start_index + 1)
    }

    /// Appends `source` to the provenance accumulated in `existing`, which
    /// is `None` until something has been merged into it.
    pub(crate) fn merge_into(
        &mut self,
        existing: &mut Option<SourceVectors>,
        source: SourceVectors,
    ) {
        *existing = Some(existing.map_or(source, |existing| self.merge_vectors(existing, source)));
    }

    /// Whether the anchor `anchor` points exactly at the start of `source`.
    pub(super) fn anchors_start_of(&self, anchor: SourceVectors, source: SourceVectors) -> bool {
        let anchor = self.first_source_vector(anchor);
        let start = self.first_source_vector(source);
        anchor.source_file_index == start.source_file_index && anchor.index == start.index
    }

    /// Creates a location in the retained arena, which preprocessor-arena
    /// compaction never discards. The parser makes its locations here.
    pub(crate) fn create_retained_source_vectors(
        &mut self,
        start_position: SourcePosition,
        source_file_index: u32,
        length: usize,
    ) -> SourceVectors {
        let (start, end) = Self::checked_source_append(
            SourceArena::Retained,
            self.retained_vectors.len(),
            self.retained_vectors.len(),
            1,
        );
        self.retained_vectors
            .push(SourceVector::new(start_position, source_file_index, length));
        SourceArena::Retained.encode(start, end)
    }

    /// Retains a zero-width diagnostic anchor immediately after `vector`.
    /// C99: §5.1.1.3 paragraph 1 and footnote 8, p. 11; PDF p. 23.
    pub(crate) fn retain_source_end(&mut self, vector: &SourceVector) -> SourceVectors {
        let anchor = vector.end_anchor();
        self.create_retained_source_vectors(anchor.position(self), anchor.source_file_index, 0)
    }

    /// Discards the preprocessor provenance arena once preprocessing has
    /// ended and releases its region, which would otherwise stay reserved,
    /// empty, through parsing. Parsed tokens and parser locations use the
    /// other two arenas; a later preprocessor-arena range would reserve a new
    /// region.
    pub(crate) fn release_preprocessor_vectors(&mut self) {
        self.compact_preprocessor_vectors();
        self.source_vectors.0 = RegionVec::new();
    }

    /// Copies a preprocessor-arena range into the retained arena; other
    /// ranges are returned unchanged.
    pub(super) fn retain_preprocessor_range(
        &mut self,
        source_vectors: SourceVectors,
    ) -> SourceVectors {
        if source_vectors.length() == 0
            || SourceArena::decode(source_vectors).0 != SourceArena::Preprocessor
        {
            return source_vectors;
        }
        let (start, end) = Self::checked_source_append(
            SourceArena::Retained,
            self.retained_vectors.len(),
            self.retained_vectors.len(),
            source_vectors.length() as usize,
        );
        self.copy_into(SourceArena::Retained, source_vectors);
        SourceArena::Retained.encode(start, end)
    }

    /// Total vectors retained by every provenance arena.
    pub(crate) fn source_segment_count(&self) -> usize {
        self.source_vectors
            .0
            .len()
            .checked_add(self.parser_token_vectors.len())
            .and_then(|count| count.checked_add(self.retained_vectors.len()))
            .expect("source segment count overflow")
    }

    pub(super) fn merge_target(all_preprocessor: bool) -> SourceArena {
        if all_preprocessor {
            SourceArena::Preprocessor
        } else {
            SourceArena::Retained
        }
    }

    /// Checks representational limits before any vectors or expansion sites
    /// are appended. An encoded range cannot start at the final `u32` index.
    pub(super) fn checked_source_append(
        arena: SourceArena,
        range_start: usize,
        current_len: usize,
        additional: usize,
    ) -> (u32, u32) {
        let end = current_len
            .checked_add(additional)
            .unwrap_or_else(|| panic!("{arena:?} source arena length overflows usize"));
        let end = u32::try_from(end)
            .unwrap_or_else(|_| panic!("{arena:?} source arena exceeds u32::MAX vectors"));
        let range_start = u32::try_from(range_start)
            .unwrap_or_else(|_| panic!("{arena:?} source range start exceeds u32::MAX"));
        assert!(
            range_start < u32::MAX,
            "{arena:?} source range cannot start at index u32::MAX"
        );
        assert!(
            range_start as usize <= current_len,
            "{arena:?} source range starts beyond the arena end"
        );
        assert!(
            end - range_start <= SourceVectors::MAX_LENGTH,
            "{arena:?} source range exceeds the 30-bit length limit"
        );
        (range_start, end)
    }

    pub(super) fn arena(&self, arena: SourceArena) -> &[SourceVector] {
        match arena {
            | SourceArena::Preprocessor => &self.source_vectors.0,
            | SourceArena::ParserTokens => &self.parser_token_vectors,
            | SourceArena::Retained => &self.retained_vectors,
        }
    }

    /// Copies the first vector, or the default vector when no source is
    /// available. This fallback is for internal preprocessing state, not
    /// diagnostic positions.
    /// C99: provenance across §5.1.1.2, pp. 9-10; PDF pp. 21-22.
    pub(crate) fn first_source_vector_or_default(&self, source: SourceVectors) -> SourceVector {
        self.get_source_vectors(source)
            .first()
            .cloned()
            .unwrap_or_default()
    }

    /// Copies the ordered source vectors into the caller's arena so they
    /// survive changes to the context's provenance stores. Empty ranges
    /// remain empty.
    /// C99: provenance across §5.1.1.2, pp. 9-10; PDF pp. 21-22.
    pub(crate) fn copy_source_vectors_in<'a>(
        &self,
        source: SourceVectors,
        arena: &'a Bump,
    ) -> &'a [SourceVector] {
        arena.alloc_slice_fill_iter(self.get_source_vectors(source).iter().cloned())
    }

    /// Returns the primary diagnostic position from a non-empty source range.
    /// A zero-width vector is a valid location; a range with no vectors is not.
    /// Empty ranges assert in debug builds, so tests catch them; a release
    /// build falls back to the default position instead of panicking.
    /// C99: §5.1.1.3 paragraph 1 and footnote 8, p. 11; PDF p. 23.
    #[inline(always)]
    pub(crate) fn diagnostic_position(&self, source: SourceVectors) -> SourcePosition {
        debug_assert!(
            source.length() != 0,
            "diagnostic primary source range must be non-empty"
        );
        self.get_source_vectors(source)
            .first()
            .map_or_else(SourcePosition::default, |vector| vector.position(self))
    }

    pub(in crate::translation_phases) fn first_source_vector(
        &self,
        source_vectors: SourceVectors,
    ) -> &SourceVector {
        let (arena, start) = SourceArena::decode(source_vectors);
        &self.arena(arena)[start as usize]
    }

    /// Whether `source` only marks where the parser found syntax missing: a
    /// zero-width location the parser created in the retained arena.
    pub(super) fn is_parser_anchor(&self, source: SourceVectors) -> bool {
        source.length() != 0
            && SourceArena::decode(source).0 == SourceArena::Retained
            && self
                .get_source_vectors(source)
                .iter()
                .all(|vector| vector.length == 0)
    }

    pub(crate) fn get_source_vectors(&self, source_vectors: SourceVectors) -> &[SourceVector] {
        if source_vectors.length() == 0 {
            return &[];
        }
        let (arena, start) = SourceArena::decode(source_vectors);
        let start = start as usize;
        &self.arena(arena)[start..start + source_vectors.length() as usize]
    }
}
