//! Translation context shared by every phase: configuration, provenance
//! arenas, interned strings, source files, and pending diagnostics.

use std::{
    ffi::OsStr,
    path::Path,
};

use rustc_hash::FxBuildHasher;

use super::{
    TranslationError,
    provenance::{
        SourceArena,
        SourcePosition,
        SourceVector,
        SourceVectorStack,
        SourceVectors,
    },
};
use crate::{
    configuration::CompilerConfiguration,
    translation_phases::{
        initial_processing::InitialProcessorError,
        parsing::ParserError,
        preprocessing::{
            KeywordTokenType,
            LiteralId,
            LiteralUnit,
            PreprocessorError,
        },
        preprocessor_tokenizer::PreprocessorTokenizerError,
    },
    util::{
        bump::{
            ArenaMap,
            ArenaQueue,
            ArenaString,
            ArenaVec,
            Bump,
        },
        dedup_arena::DedupArena,
        region_vec::RegionVec,
        string_cache::{
            StringCache,
            StringCacheId,
        },
        vector_slice::UsizeExt,
    },
};

/// A vector in an arena that grows by adding segments, each twice as long as
/// the one before, so it never moves or copies its elements and leaves no
/// outgrown buffers in the arena. Removing from the front only advances its
/// start, and clearing keeps the segments for reuse, as a `Vec` keeps its
/// capacity.
struct SegmentedVec<'a, T> {
    arena:    &'a Bump,
    segments: ArenaVec<'a, ArenaVec<'a, T>>,
    /// The position of the first element.
    start:    usize,
    /// The position after the last element.
    end:      usize,
    /// The segment that holds the last element, or 0 when empty.
    tail:     usize,
}

impl<'a, T: Clone> SegmentedVec<'a, T> {
    /// The length of the first segment.
    const FIRST_SEGMENT: usize = 16;

    fn new_in(arena: &'a Bump) -> Self {
        Self {
            arena,
            segments: ArenaVec::new_in(arena),
            start: 0,
            end: 0,
            tail: 0,
        }
    }

    /// The segment holding `position`, and the offset there.
    fn locate(position: usize) -> (usize, usize) {
        let blocks = position / Self::FIRST_SEGMENT + 1;
        let segment = blocks.ilog2() as usize;
        (
            segment,
            position - Self::FIRST_SEGMENT * ((1 << segment) - 1),
        )
    }

    fn len(&self) -> usize {
        self.end - self.start
    }

    fn is_empty(&self) -> bool {
        self.start == self.end
    }

    fn get(&self, index: usize) -> Option<&T> {
        (index < self.len()).then(|| {
            let (segment, offset) = Self::locate(self.start + index);
            &self.segments[segment][offset]
        })
    }

    fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        (index < self.len()).then(|| {
            let (segment, offset) = Self::locate(self.start + index);
            &mut self.segments[segment][offset]
        })
    }

    fn last(&self) -> Option<&T> {
        if self.is_empty() {
            None
        } else {
            self.segments[self.tail].last()
        }
    }

    fn last_mut(&mut self) -> Option<&mut T> {
        if self.is_empty() {
            None
        } else {
            self.segments[self.tail].last_mut()
        }
    }

    fn push(&mut self, value: T) {
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
    fn insert(&mut self, index: usize, value: T) {
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

    /// The elements in order, one slice per segment.
    fn slices(&self) -> impl Iterator<Item = &[T]> {
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

    /// The number of leading elements for which `predicate` holds, which must
    /// hold for a prefix.
    fn partition_point(&self, mut predicate: impl FnMut(&T) -> bool) -> usize {
        let mut count = 0;
        for slice in self.slices() {
            match slice.last() {
                | Some(last) if predicate(last) => count += slice.len(),
                | _ => return count + slice.partition_point(&mut predicate),
            }
        }
        count
    }

    /// Removes the first `count` elements.
    fn discard_front(&mut self, count: usize) {
        self.start += count.min(self.len());
        if self.is_empty() {
            self.clear();
        }
    }

    fn clear(&mut self) {
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

    fn iter(&self) -> impl Iterator<Item = &T> {
        self.slices().flatten()
    }

    fn for_each_mut(&mut self, mut visit: impl FnMut(&mut T)) {
        if self.is_empty() {
            return;
        }
        let (first, offset) = Self::locate(self.start);
        for (index, segment) in self.segments[first..=self.tail].iter_mut().enumerate() {
            let skip = if index == 0 { offset } else { 0 };
            segment[skip..].iter_mut().for_each(&mut visit);
        }
    }
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

/// Endpoints are normally appended in source-arena order. Keep sparse keys
/// packed, and reuse end locations within each invocation. Unlike a global
/// interner, the transient pool is cleared during preprocessing compaction.
///
/// Both tables live in the translation-unit arena. They grow by segments, so
/// the parser-token table, which holds an entry for every macro-expanded
/// token until parsing reads past it, never copies itself into a larger
/// buffer and leaves the old one behind.
struct ExpansionSites<'tu> {
    entries: SegmentedVec<'tu, (u32, u32)>,
    ends:    SegmentedVec<'tu, SourceVector>,
}

impl<'tu> ExpansionSites<'tu> {
    fn new_in(tu: &'tu Bump) -> Self {
        Self {
            entries: SegmentedVec::new_in(tu),
            ends:    SegmentedVec::new_in(tu),
        }
    }

    fn get(&self, end: u32) -> Option<u32> {
        let &(last, id) = self.entries.last()?;
        if end >= last {
            return (end == last).then_some(id);
        }
        let index = self.entries.partition_point(|&(key, _)| key < end);
        self.entries
            .get(index)
            .filter(|&&(key, _)| key == end)
            .map(|&(_, id)| id)
    }

    fn insert(&mut self, end: u32, id: u32) {
        if let Some(last) = self.entries.last_mut() {
            if last.0 == end {
                last.1 = id;
                return;
            }
            if last.0 > end {
                let index = self.entries.partition_point(|&(key, _)| key < end);
                if self.entries[index].0 == end {
                    self.entries[index].1 = id;
                } else {
                    self.entries.insert(index, (end, id));
                }
                return;
            }
        }
        self.entries.push((end, id));
    }

    fn site_id(&mut self, site: SourceVector) -> u32 {
        if self.ends.last() == Some(&site) {
            return (self.ends.len() - 1).to_u32();
        }
        let id = self.ends.len().to_u32();
        self.ends.push(site);
        id
    }

    fn discard_before(&mut self, end: u32) {
        let count = self.entries.partition_point(|&(key, _)| key < end);
        // Compact geometrically for full-batch input; renumbering the whole
        // tail after every declaration would make parsing quadratic.
        if count == 0 || count < self.entries.len() / 2 {
            return;
        }
        self.entries.discard_front(count);
        let Some(first_id) = self.entries.iter().map(|&(_, id)| id).min() else {
            self.ends.clear();
            return;
        };
        self.ends.discard_front(first_id as usize);
        self.entries.for_each_mut(|(_, id)| *id -= first_id);
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.ends.clear();
    }
}
pub(crate) struct Context<'tu> {
    tu: &'tu Bump,
    pub(crate) configuration: CompilerConfiguration,
    pub(crate) source_vectors: SourceVectorStack,
    parser_token_vectors: RegionVec<SourceVector>,
    retained_vectors: RegionVec<SourceVector>,
    pub(crate) string_cache: StringCache<'tu>,
    pub(crate) canonical_identifiers: ArenaMap<'tu, StringCacheId, StringCacheId>,
    literal_values: DedupArena<'tu, &'tu [LiteralUnit], FxBuildHasher>,
    /// Sparse endpoints follow their source arena's lifetime.
    expansion_sites: [ExpansionSites<'tu>; 3],
    ignore_tokenizer_errors: bool,
    pub(super) pending_errors: ArenaQueue<'tu, TranslationError<'tu>>,
    /// How many leading pending errors no longer refer to the preprocessor
    /// arena, so compaction relocates each error's provenance only once.
    relocated_errors: usize,
    pub(crate) source_files: DedupArena<'tu, &'tu Path, FxBuildHasher>,
    quote_include_directories: &'tu [&'tu Path],
    system_include_directories: &'tu [&'tu Path],
    /// Original text of each source file, indexed like `source_files`, kept
    /// so diagnostics can quote the lines they point at.
    source_texts: ArenaVec<'tu, Option<&'tu str>>,
}

impl<'tu> Context<'tu> {
    /// The translation-unit arena, which holds everything that lives until
    /// the translation unit ends, the syntax tree included.
    pub(crate) fn tu_arena(&self) -> &'tu Bump {
        self.tu
    }

    pub(crate) fn diagnostic_text(&self, text: &str) -> &'tu str {
        self.tu.alloc_str(text)
    }

    /// Formats diagnostic text straight into the translation-unit arena.
    pub(crate) fn diagnostic_format(&self, arguments: std::fmt::Arguments<'_>) -> &'tu str {
        let mut text = ArenaString::new_in(self.tu);
        std::fmt::Write::write_fmt(&mut text, arguments).expect("arena formatting cannot fail");
        text.into_str()
    }

    pub(crate) fn diagnostic_slice<T: Copy>(&self, values: &[T]) -> &'tu mut [T] {
        self.tu.alloc_slice_copy(values)
    }

    pub(crate) fn new(tu: &'tu Bump) -> Self {
        Self::with_configuration(tu, CompilerConfiguration::default())
    }

    pub(crate) fn with_configuration(tu: &'tu Bump, configuration: CompilerConfiguration) -> Self {
        let mut string_cache = StringCache::new(tu);
        for &keyword in KeywordTokenType::ALL {
            let id = string_cache.intern(keyword.spelling());
            debug_assert_eq!(
                id,
                keyword.cache_id(),
                "keywords must occupy the reserved prefix"
            );
        }
        Self {
            tu,
            configuration,
            source_vectors: SourceVectorStack(RegionVec::new()),
            parser_token_vectors: RegionVec::new(),
            retained_vectors: RegionVec::new(),
            string_cache,
            canonical_identifiers: ArenaMap::with_hasher_in(FxBuildHasher, tu),
            literal_values: DedupArena::new(tu),
            expansion_sites: std::array::from_fn(|_| ExpansionSites::new_in(tu)),
            ignore_tokenizer_errors: false,
            pending_errors: ArenaQueue::new_in(tu),
            relocated_errors: 0,
            source_files: DedupArena::new(tu),
            quote_include_directories: &[],
            system_include_directories: &[],
            source_texts: ArenaVec::new_in(tu),
        }
    }

    pub(crate) fn record_expansion_end(&mut self, source: SourceVectors, site: SourceVector) {
        if source.length != 0 {
            let (arena, start) = SourceArena::decode(source);
            let sites = &mut self.expansion_sites[arena as usize];
            let id = sites.site_id(site);
            sites.insert(start + source.length - 1, id);
        }
    }

    /// Where a source range ends in the user's input, before macro expansion.
    pub(crate) fn user_source_end(&self, source: SourceVectors) -> Option<SourceVector> {
        if source.length == 0 {
            return None;
        }
        let (arena, start) = SourceArena::decode(source);
        self.expansion_sites[arena as usize]
            .get(start + source.length - 1)
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
            && previous.length != 0
        {
            let (arena, start) = SourceArena::decode(previous);
            if arena == SourceArena::ParserTokens {
                self.expansion_sites[arena as usize].discard_before(start + previous.length - 1);
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

    pub(crate) fn intern_literal(&mut self, units: &[LiteralUnit]) -> LiteralId {
        let tu = self.tu;
        LiteralId(
            self.literal_values
                .intern_by(units, || tu.alloc_slice_copy(units)),
        )
    }

    pub(crate) fn literal_units(&self, id: LiteralId) -> &[LiteralUnit] {
        self.literal_values[id.0]
    }

    /// The literal's characters spelled in `arena`, if they are text.
    /// Text-only consumers (filenames and tests) must reject non-UTF-8
    /// values.
    pub(crate) fn literal_text_in<'a>(
        &self,
        arena: &'a Bump,
        id: LiteralId,
        wide: bool,
    ) -> Option<&'a str> {
        let text = self.decode_literal_text(arena, id, wide)?.leak();
        // SAFETY: decoding checked that these bytes are UTF-8.
        Some(unsafe { std::str::from_utf8_unchecked(text) })
    }

    /// The literal's characters as UTF-8 in `arena`, if they are text.
    fn decode_literal_text<'a>(
        &self,
        arena: &'a Bump,
        id: LiteralId,
        wide: bool,
    ) -> Option<ArenaVec<'a, u8>> {
        let mut text = ArenaVec::new_in(arena);
        for unit in self.literal_units(id) {
            let character = match *unit {
                | LiteralUnit::Character(c) => c,
                | LiteralUnit::Numeric(code) if wide => char::from_u32(code)?,
                | LiteralUnit::Numeric(code) => {
                    text.push(u8::try_from(code).expect("narrow escape checked during decoding"));
                    continue;
                },
            };
            text.extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes());
        }
        std::str::from_utf8(&text).is_ok().then_some(text)
    }

    /// The literal as a C string literal in `arena`: quoted and escaped when
    /// its characters are text, and as numeric escapes otherwise. The text is
    /// decoded in `scratch` and taken back, unless something else is
    /// allocated there meanwhile.
    pub(crate) fn literal_spelling_in<'a>(
        &self,
        arena: &'a Bump,
        scratch: &Bump,
        id: LiteralId,
        wide: bool,
    ) -> &'a str {
        let mut spelling = ArenaString::new_in(arena);
        let text = self.decode_literal_text(scratch, id, wide);
        let text = text
            .as_deref()
            .and_then(|text| std::str::from_utf8(text).ok());
        self.write_literal_spelling(&mut spelling, text, id, wide)
            .expect("arena formatting cannot fail");
        spelling.into_str()
    }

    /// Writes the literal as a C string literal: quoted and escaped when
    /// `text` holds its characters, and as numeric escapes otherwise.
    fn write_literal_spelling(
        &self,
        out: &mut impl std::fmt::Write,
        text: Option<&str>,
        id: LiteralId,
        wide: bool,
    ) -> std::fmt::Result {
        if let Some(text) = text {
            return crate::diagnostics::write_c_quoted(out, if wide { "L" } else { "" }, '"', text);
        }
        out.write_str(if wide { "L\"" } else { "\"" })?;
        for unit in self.literal_units(id) {
            match *unit {
                | LiteralUnit::Character(c) if wide => write!(out, "\\x{:x}", u32::from(c))?,
                | LiteralUnit::Numeric(code) if wide => write!(out, "\\x{code:x}")?,
                | LiteralUnit::Character(c) =>
                    for byte in c.encode_utf8(&mut [0; 4]).bytes() {
                        write!(out, "\\{byte:03o}")?;
                    },
                | LiteralUnit::Numeric(code) => write!(
                    out,
                    "\\{:03o}",
                    u8::try_from(code).expect("narrow escape checked during decoding")
                )?,
            }
        }
        out.write_char('"')
    }

    pub(crate) fn push_source_vector(
        &mut self,
        start_position: SourcePosition,
        source_file_index: u32,
        length: usize,
    ) -> u32 {
        let index = self.source_vectors.0.len().to_u32();
        assert!(index < SourceArena::INDEX_MASK, "source arena overflow");
        self.source_vectors
            .0
            .push(SourceVector::new(start_position, source_file_index, length));
        index
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "We already checked that it is in range before casting"
    )]
    pub(crate) fn duplicate_source_vectors(
        self_source_vectors: &mut RegionVec<SourceVector>,
        source_vectors: SourceVectors,
    ) -> u32 {
        let end = u32::try_from(source_vectors.length as usize + self_source_vectors.len())
            .expect("overflow in duplicate_source_vectors");
        assert!(end <= SourceArena::INDEX_MASK, "source arena overflow");
        let start_index = self_source_vectors.len() as u32;
        let start = source_vectors.start_index();
        for i in start..start + source_vectors.length {
            self_source_vectors.push(self_source_vectors[i as usize].clone());
        }
        start_index
    }

    /// Copies owned provenance back into the preprocessor arena.
    pub(crate) fn push_source_vectors(&mut self, vectors: &[SourceVector]) -> SourceVectors {
        let start = self.source_vectors.0.len().to_u32();
        self.source_vectors.0.extend_from_slice(vectors);
        SourceArena::Preprocessor.encode(start, self.source_vectors.0.len().to_u32())
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
        if v1.length == 0 {
            return v2;
        }
        if v2.length == 0 {
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
        let end1 = start1 + v1.length;
        if arena1 == arena2 && end1 == start2 {
            return arena1.encode(start1, start2 + v2.length);
        }
        let target = Self::merge_target(arena1 == SourceArena::Preprocessor && arena2 == arena1);
        let start = if arena1 == target && end1 as usize == self.arena(target).len() {
            start1
        } else {
            let start = self.arena(target).len().to_u32();
            self.copy_into(target, v1);
            start
        };
        self.copy_into(target, v2);
        target.encode(start, self.arena(target).len().to_u32())
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
                .checked_add(source.length)
                .expect("source range overflow");
        }
        let Some((first_arena, first_start)) = first else {
            return SourceVectors::default();
        };
        if contiguous {
            return first_arena.encode(first_start, end);
        }
        let target = Self::merge_target(all_preprocessor);
        let start = self.arena(target).len().to_u32();
        let mut kept = anchors;
        for &source in sources {
            if kept.keeps(self, source) {
                self.copy_into(target, source);
            }
        }
        target.encode(start, self.arena(target).len().to_u32())
    }

    /// Whether `source` only marks where the parser found syntax missing: a
    /// zero-width location the parser created in the retained arena.
    fn is_parser_anchor(&self, source: SourceVectors) -> bool {
        source.length != 0
            && SourceArena::decode(source).0 == SourceArena::Retained
            && self
                .get_source_vectors(source)
                .iter()
                .all(|vector| vector.length == 0)
    }

    /// Whether the anchor `anchor` points exactly at the start of `source`.
    fn anchors_start_of(&self, anchor: SourceVectors, source: SourceVectors) -> bool {
        let anchor = self.first_source_vector(anchor);
        let start = self.first_source_vector(source);
        anchor.source_file_index == start.source_file_index && anchor.index == start.index
    }

    /// Copies a phase-6 output token's provenance into the parser-token
    /// provenance once, so provenance of consecutive tokens is adjacent and
    /// survives [`Self::compact_preprocessor_vectors`].
    pub(crate) fn retain_token_source(&mut self, source_vectors: SourceVectors) -> SourceVectors {
        if source_vectors.length == 0
            || SourceArena::decode(source_vectors).0 != SourceArena::Preprocessor
        {
            return source_vectors;
        }
        let start = self.parser_token_vectors.len().to_u32();
        self.copy_into(SourceArena::ParserTokens, source_vectors);
        SourceArena::ParserTokens.encode(start, self.parser_token_vectors.len().to_u32())
    }

    /// Creates a location in the retained arena, which preprocessor-arena
    /// compaction never discards. The parser makes its locations here.
    pub(crate) fn create_retained_source_vectors(
        &mut self,
        start_position: SourcePosition,
        source_file_index: u32,
        length: usize,
    ) -> SourceVectors {
        let start = self.retained_vectors.len().to_u32();
        self.retained_vectors
            .push(SourceVector::new(start_position, source_file_index, length));
        SourceArena::Retained.encode(start, self.retained_vectors.len().to_u32())
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
        for error in pending_errors.iter_mut().skip(self.relocated_errors) {
            error.for_each_source_vectors_mut(&mut |source_vectors| {
                *source_vectors = self.retain_preprocessor_range(*source_vectors);
            });
        }
        self.relocated_errors = pending_errors.len();
        self.pending_errors = pending_errors;
        self.source_vectors.0.clear();
        self.expansion_sites[SourceArena::Preprocessor as usize].clear();
    }

    /// Copies a preprocessor-arena range into the retained arena; other
    /// ranges are returned unchanged.
    fn retain_preprocessor_range(&mut self, source_vectors: SourceVectors) -> SourceVectors {
        if source_vectors.length == 0
            || SourceArena::decode(source_vectors).0 != SourceArena::Preprocessor
        {
            return source_vectors;
        }
        let start = self.retained_vectors.len().to_u32();
        self.copy_into(SourceArena::Retained, source_vectors);
        SourceArena::Retained.encode(start, self.retained_vectors.len().to_u32())
    }

    /// Total vectors retained by every provenance arena.
    pub(crate) fn source_segment_count(&self) -> usize {
        self.source_vectors.0.len() + self.parser_token_vectors.len() + self.retained_vectors.len()
    }

    fn merge_target(all_preprocessor: bool) -> SourceArena {
        if all_preprocessor {
            SourceArena::Preprocessor
        } else {
            SourceArena::Retained
        }
    }

    fn arena(&self, arena: SourceArena) -> &[SourceVector] {
        match arena {
            | SourceArena::Preprocessor => &self.source_vectors.0,
            | SourceArena::ParserTokens => &self.parser_token_vectors,
            | SourceArena::Retained => &self.retained_vectors,
        }
    }

    /// Appends the vectors of `source` to `target`.
    fn copy_into(&mut self, target: SourceArena, source: SourceVectors) {
        let (arena, start) = SourceArena::decode(source);
        let range = start as usize..(start + source.length) as usize;
        if source.length != 0
            && let Some(site) = self.expansion_sites[arena as usize].get(start + source.length - 1)
        {
            let end = self.arena(target).len().to_u32() + source.length - 1;
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

    pub(crate) fn ignore_tokenizer_errors(&self) -> bool {
        self.ignore_tokenizer_errors
    }

    pub(crate) fn set_ignore_tokenizer_errors(&mut self, value: bool) {
        self.ignore_tokenizer_errors = value;
    }

    #[inline(always)]
    pub(crate) fn missing_final_newline(&mut self, vector: SourceVector) {
        if !self.ignore_tokenizer_errors() {
            self.pending_errors
                .push_back(TranslationError::InitialProcessing(
                    InitialProcessorError::MissingFinalNewline(vector),
                ));
        }
    }

    pub(crate) fn escaped_final_newline(&mut self, vector: SourceVector) {
        if !self.ignore_tokenizer_errors() {
            self.pending_errors
                .push_back(TranslationError::InitialProcessing(
                    InitialProcessorError::EscapedFinalNewline(vector),
                ));
        }
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn preprocessor_tokenizer_error(&mut self, error: PreprocessorTokenizerError) {
        if !self.ignore_tokenizer_errors() {
            self.pending_errors
                .push_back(TranslationError::PreprocessorTokenizining(error));
        }
    }

    /// The quoted include extension treats the first quote after a backslash
    /// as the header delimiter, even though phase 3 lexed it as an escape.
    pub(crate) fn withdraw_quoted_header_lexer_error(&mut self, source: &SourceVector) {
        self.pending_errors.retain(|error| {
            !matches!(error, TranslationError::PreprocessorTokenizining(error) if error.is_unclosed_header_string_at(source))
        });
        // Retaining can remove a diagnostic from the relocated prefix.
        // Rechecking already-retained ranges during the next compaction is
        // safe.
        self.relocated_errors = 0;
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn preprocessor_error(&mut self, error: PreprocessorError<'tu>) {
        self.pending_errors
            .push_back(TranslationError::Preprocessing(error));
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn raw_preprocessor_error(
        self_pending_errors: &mut impl Extend<TranslationError<'tu>>,
        error: PreprocessorError<'tu>,
    ) {
        self_pending_errors.extend([TranslationError::Preprocessing(error)]);
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn parser_error(&mut self, error: ParserError<'tu>) {
        self.pending_errors
            .push_back(TranslationError::Parsing(error));
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn pop_pending_error(&mut self) -> Option<TranslationError<'tu>> {
        let error = self.pending_errors.pop_front();
        self.relocated_errors = self.relocated_errors.saturating_sub(1);
        error
    }

    pub(crate) fn pending_error_count(&self) -> usize {
        self.pending_errors.len()
    }

    #[cfg(test)]
    pub(crate) fn take_pending_errors(&mut self) -> Vec<TranslationError<'tu>> {
        self.relocated_errors = 0;
        std::iter::from_fn(|| self.pending_errors.pop_front()).collect()
    }

    /// Removes and yields the pending errors after the first `keep`, in
    /// order.
    pub(crate) fn split_off_pending_errors(
        &mut self,
        keep: usize,
    ) -> impl Iterator<Item = TranslationError<'tu>> + '_ {
        self.relocated_errors = self.relocated_errors.min(keep);
        self.pending_errors.split_off(keep)
    }

    pub(crate) fn append_pending_errors(
        &mut self,
        errors: impl IntoIterator<Item = TranslationError<'tu>>,
    ) {
        self.pending_errors.extend(errors);
    }

    pub(crate) fn get_source_vectors(&self, source_vectors: SourceVectors) -> &[SourceVector] {
        if source_vectors.length == 0 {
            return &[];
        }
        let (arena, start) = SourceArena::decode(source_vectors);
        let start = start as usize;
        &self.arena(arena)[start..start + source_vectors.length as usize]
    }

    pub(super) fn first_source_vector(&self, source_vectors: SourceVectors) -> &SourceVector {
        let (arena, start) = SourceArena::decode(source_vectors);
        &self.arena(arena)[start as usize]
    }

    pub(crate) fn intern_source_file(&mut self, path: &Path) -> u32 {
        let tu = self.tu;
        self.source_files
            .intern_by(path, || Self::alloc_path(tu, path))
    }

    /// Registers synthetic source text under a fresh identity, even when
    /// `path` names an earlier input, so diagnostics retained from each
    /// input keep quoting their own text.
    pub(crate) fn add_synthetic_source_file(&mut self, path: &Path, text: &'tu str) -> u32 {
        let index = self
            .source_files
            .push_unindexed(Self::alloc_path(self.tu, path));
        self.record_arena_source_text(index, text);
        index
    }

    pub(crate) fn get_source_file(&self, index: u32) -> &Path {
        self.source_files[index]
    }

    pub(crate) fn set_include_directories(&mut self, quote: &[&Path], system: &[&Path]) {
        self.quote_include_directories = self
            .tu
            .alloc_slice_fill_iter(quote.iter().map(|path| Self::alloc_path(self.tu, path)));
        self.system_include_directories = self
            .tu
            .alloc_slice_fill_iter(system.iter().map(|path| Self::alloc_path(self.tu, path)));
    }

    /// The directories searched for a header named in a file, in order: for
    /// a `"…"` name the including file's directory and the quote
    /// directories, then for both forms the system directories.
    pub(crate) fn include_search_directories(
        &self,
        including_file: u32,
        is_system_header: bool,
    ) -> impl Iterator<Item = &'tu Path> + Clone + use<'tu> {
        let including_file: &'tu Path = self.source_files[including_file];
        let quote_directories: &'tu [&'tu Path] = self.quote_include_directories;
        let system_directories: &'tu [&'tu Path] = self.system_include_directories;
        let quote = (!is_system_header).then(|| {
            let directory = including_file.parent().unwrap_or_else(|| Path::new(""));
            std::iter::once(directory).chain(quote_directories.iter().copied())
        });
        quote
            .into_iter()
            .flatten()
            .chain(system_directories.iter().copied())
    }

    fn alloc_path(tu: &'tu Bump, path: &Path) -> &'tu Path {
        let bytes = tu.alloc_slice_copy(path.as_os_str().as_encoded_bytes());
        // SAFETY: These are the complete encoded bytes of an OsStr from this
        // process and target, copied without splitting or changing them.
        Path::new(unsafe { OsStr::from_encoded_bytes_unchecked(bytes) })
    }

    /// Remembers the text a source file was translated from.
    pub(crate) fn record_source_text(&mut self, index: u32, text: &str) {
        let text = self.tu.alloc_str(text);
        self.record_arena_source_text(index, text);
    }

    pub(crate) fn record_arena_source_text(&mut self, index: u32, text: &'tu str) {
        let index = index as usize;
        if self.source_texts.len() <= index {
            self.source_texts.resize(index + 1, None);
        }
        self.source_texts[index] = Some(text);
    }

    /// Reads and retains an included file without a temporary heap string.
    pub(crate) fn read_source_file(&mut self, index: u32) -> std::io::Result<&'tu str> {
        let text = self.tu.read_to_str_lossy(self.get_source_file(index))?;
        self.record_arena_source_text(index, text);
        Ok(text)
    }

    /// Returns the text of a source file, if it was recorded.
    pub(crate) fn source_text(&self, index: u32) -> Option<&str> {
        self.source_texts
            .get(index as usize)?
            .as_ref()
            .map(|text| &**text)
    }

    /// Returns the exact source spelling covered by a single-segment range.
    pub(crate) fn source_spelling(&self, source_vectors: SourceVectors) -> Option<&str> {
        let [vector] = self.get_source_vectors(source_vectors) else {
            return None;
        };
        self.source_text(vector.source_file_index)?
            .get(vector.range())
    }
}

/// Which ranges [`Context::merge_vector_list`] keeps, decided in order
/// without collecting them: empty ranges are dropped, and parser anchors are
/// dropped after the first real range, or before it when they point at its
/// start.
#[derive(Clone, Copy)]
struct MergeAnchors {
    /// Whether any range is an anchor; otherwise every nonempty range stays.
    any:        bool,
    first_real: Option<SourceVectors>,
    real_seen:  bool,
}

impl MergeAnchors {
    fn new(context: &Context<'_>, sources: &[SourceVectors]) -> Self {
        let any = sources
            .iter()
            .any(|source| context.is_parser_anchor(*source));
        Self {
            any,
            first_real: any
                .then(|| {
                    sources
                        .iter()
                        .find(|source| source.length != 0 && !context.is_parser_anchor(**source))
                        .copied()
                })
                .flatten(),
            real_seen: false,
        }
    }

    /// Whether the next range in order, `source`, is kept.
    fn keeps(&mut self, context: &Context<'_>, source: SourceVectors) -> bool {
        if source.length == 0 {
            return false;
        }
        if !self.any {
            return true;
        }
        if context.is_parser_anchor(source) {
            !(self.real_seen
                || self
                    .first_real
                    .is_some_and(|first| context.anchors_start_of(source, first)))
        } else {
            self.real_seen = true;
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Context,
        SegmentedVec,
        SourceArena,
        SourceVector,
    };

    #[test]
    fn segmented_vectors_keep_order_across_segments_and_reuse_them() {
        let arena = crate::util::bump::Bump::new();
        let mut values = SegmentedVec::new_in(&arena);
        // 100 values span the first three segments (16, 32, and 64).
        for value in (0..200).step_by(2) {
            values.push(value);
        }
        values.insert(50, 99);
        let mut expected: Vec<i32> = (0..100).step_by(2).collect();
        expected.push(99);
        expected.extend((100..200).step_by(2));
        assert_eq!(values.iter().copied().collect::<Vec<_>>(), expected);
        assert_eq!(values.partition_point(|&value| value < 100), 51);
        assert_eq!(values.last(), Some(&198));

        values.discard_front(10);
        assert_eq!((values.len(), values[0]), (91, 20));
        assert_eq!(values.partition_point(|&value| value < 100), 41);
        assert_eq!(values.iter().count(), 91);
        values.for_each_mut(|value| *value += 1);
        assert_eq!(values[0], 21);

        // Clearing keeps the segments, so refilling takes no arena memory.
        let used = arena.used();
        values.clear();
        assert!(values.is_empty());
        for value in 0..100 {
            values.push(value);
        }
        assert_eq!(arena.used(), used);
        assert_eq!(
            values.iter().copied().collect::<Vec<_>>(),
            (0..100).collect::<Vec<_>>()
        );
    }

    #[test]
    fn repeated_literal_values_keep_one_identity_after_arena_growth() {
        use crate::translation_phases::preprocessing::LiteralUnit;

        let tu = crate::util::bump::Bump::new();
        let mut context = Context::new(&tu);
        let original = [LiteralUnit::Character('é'), LiteralUnit::Numeric(0)];
        let id = context.intern_literal(&original);
        for value in 0..2_000 {
            _ = context.intern_literal(&[LiteralUnit::Numeric(value)]);
        }
        assert_eq!(context.intern_literal(&original), id);
        assert_eq!(context.literal_units(id), original);
    }

    #[test]
    fn source_paths_keep_indices_and_synthetic_inputs_keep_distinct_text() {
        use std::path::PathBuf;

        let tu = crate::util::bump::Bump::new();
        let mut context = Context::new(&tu);
        let path = PathBuf::from("included/header.h");
        let first = context.intern_source_file(&path);
        for index in 0..2_000 {
            _ = context.intern_source_file(&PathBuf::from(format!("included/{index}.h")));
        }
        assert_eq!(context.intern_source_file(&path), first);
        assert_eq!(context.get_source_file(first), path);
        let synthetic = context.add_synthetic_source_file(&path, "second input");
        assert_ne!(synthetic, first);
        assert_eq!(context.source_text(synthetic), Some("second input"));
    }

    #[cfg(windows)]
    #[test]
    fn source_paths_preserve_unpaired_utf16_surrogates() {
        use std::{
            ffi::OsString,
            os::windows::ffi::OsStringExt,
            path::PathBuf,
        };

        let tu = crate::util::bump::Bump::new();
        let mut context = Context::new(&tu);
        let path = PathBuf::from(OsString::from_wide(&[u16::from(b'x'), 0xD800]));
        let id = context.intern_source_file(&path);
        assert_eq!(context.get_source_file(id), path);
        assert_eq!(context.intern_source_file(&path), id);
    }

    #[test]
    fn macro_locations_survive_compaction_without_retaining_temporary_metadata() {
        let tu = crate::util::bump::Bump::new();
        let mut context = Context::new(&tu);
        let mut saved = Vec::new();
        for index in 0..2_000 {
            let source = context.push_source_vectors(&[SourceVector {
                length: 1,
                ..SourceVector::default()
            }]);
            let site = SourceVector {
                index,
                line: index + 1,
                length: 1,
                ..SourceVector::default()
            };
            context.record_expansion_end(source, site.clone());
            let token = context.retain_token_source(source);
            let retained = context.retain_preprocessor_range(source);
            saved.push((token, retained, site));
            context.compact_preprocessor_vectors();
            let temporary = &context.expansion_sites[SourceArena::Preprocessor as usize];
            assert!(temporary.entries.is_empty());
            assert!(temporary.ends.is_empty());
        }
        for (token, retained, site) in saved {
            assert_eq!(context.user_source_end(token), Some(site.clone()));
            assert_eq!(context.user_source_end(retained), Some(site));
        }
    }
}
