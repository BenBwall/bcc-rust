//! Translation context shared by every phase: configuration, provenance
//! arenas, interned strings, source files, and pending diagnostics.

use std::{
    collections::VecDeque,
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
        preprocessing::PreprocessorError,
        preprocessor_tokenizer::PreprocessorTokenizerError,
    },
    util::{
        dedup_arena::DedupArena,
        shared::SharedString,
        string_cache::StringCache,
        vector_slice::UsizeExt,
    },
};

pub(crate) struct Context {
    pub(crate) configuration:     CompilerConfiguration,
    pub(crate) source_vectors:    SourceVectorStack,
    parser_token_vectors:         Vec<SourceVector>,
    parser_merge_vectors:         Vec<SourceVector>,
    pub(crate) string_cache:      StringCache,
    is_tokenizing_include_string: bool,
    ignore_tokenizer_errors:      bool,
    pub(super) pending_errors:    VecDeque<TranslationError>,
    pub(crate) source_files:      DedupArena<Box<Path>, FxBuildHasher>,
    /// Original text of each source file, indexed like `source_files`, kept
    /// so diagnostics can quote the lines they point at.
    source_texts:                 Vec<Option<SharedString>>,
}

impl Context {
    pub(crate) fn new() -> Self {
        Self::with_configuration(CompilerConfiguration::default())
    }

    pub(crate) fn with_configuration(configuration: CompilerConfiguration) -> Self {
        Self {
            configuration,
            source_vectors: SourceVectorStack(Vec::new()),
            parser_token_vectors: Vec::new(),
            parser_merge_vectors: Vec::new(),
            string_cache: StringCache::new(),
            is_tokenizing_include_string: false,
            ignore_tokenizer_errors: false,
            pending_errors: VecDeque::new(),
            source_files: DedupArena::new(),
            source_texts: Vec::new(),
        }
    }

    pub(crate) fn push_source_vector(
        &mut self,
        start_position: SourcePosition,
        source_file_index: u32,
        length: usize,
    ) -> u32 {
        let index = self.source_vectors.0.len().to_u32();
        assert!(index < SourceArena::INDEX_MASK, "source arena overflow");
        self.source_vectors.0.push(SourceVector {
            index: start_position.index,
            column: start_position.column,
            line: start_position.line,
            source_file_index,
            length,
        });
        index
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "We already checked that it is in range before casting"
    )]
    pub(crate) fn duplicate_source_vectors(
        self_source_vectors: &mut Vec<SourceVector>,
        source_vectors: SourceVectors,
    ) -> u32 {
        let end = u32::try_from(source_vectors.length as usize + self_source_vectors.len())
            .expect("overflow in duplicate_source_vectors");
        assert!(end <= SourceArena::INDEX_MASK, "source arena overflow");
        let start_index = self_source_vectors.len() as u32;
        for i in source_vectors.start_index..source_vectors.start_index + source_vectors.length {
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
    pub(crate) fn merge_vectors(&mut self, v1: SourceVectors, v2: SourceVectors) -> SourceVectors {
        if v1.length == 0 {
            return v2;
        }
        if v2.length == 0 {
            return v1;
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
    pub(crate) fn merge_vector_list(&mut self, sources: &[SourceVectors]) -> SourceVectors {
        let mut first = None;
        let mut end = 0;
        let mut contiguous = true;
        let mut all_preprocessor = true;
        for source in sources.iter().filter(|source| source.length != 0) {
            let (arena, start) = SourceArena::decode(*source);
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
        for source in sources.iter().filter(|source| source.length != 0) {
            self.copy_into(target, *source);
        }
        target.encode(start, self.arena(target).len().to_u32())
    }

    /// Copies a parser-fetched token's provenance into the parser token arena
    /// once, so provenance of consecutively fetched tokens is adjacent.
    pub(crate) fn record_parser_token_source(
        &mut self,
        source_vectors: SourceVectors,
    ) -> SourceVectors {
        if source_vectors.length == 0
            || SourceArena::decode(source_vectors).0 != SourceArena::Preprocessor
        {
            return source_vectors;
        }
        let start = self.parser_token_vectors.len().to_u32();
        self.copy_into(SourceArena::ParserTokens, source_vectors);
        SourceArena::ParserTokens.encode(start, self.parser_token_vectors.len().to_u32())
    }

    /// Total vectors retained by every provenance arena.
    pub(crate) fn source_segment_count(&self) -> usize {
        self.source_vectors.0.len()
            + self.parser_token_vectors.len()
            + self.parser_merge_vectors.len()
    }

    fn merge_target(all_preprocessor: bool) -> SourceArena {
        if all_preprocessor {
            SourceArena::Preprocessor
        } else {
            SourceArena::ParserMerges
        }
    }

    fn arena(&self, arena: SourceArena) -> &Vec<SourceVector> {
        match arena {
            | SourceArena::Preprocessor => &self.source_vectors.0,
            | SourceArena::ParserTokens => &self.parser_token_vectors,
            | SourceArena::ParserMerges => &self.parser_merge_vectors,
        }
    }

    /// Appends the vectors of `source` to `target`.
    fn copy_into(&mut self, target: SourceArena, source: SourceVectors) {
        let (arena, start) = SourceArena::decode(source);
        let range = start as usize..(start + source.length) as usize;
        let (source, target) = match (arena, target) {
            | (SourceArena::Preprocessor, SourceArena::Preprocessor) => {
                self.source_vectors.0.extend_from_within(range);
                return;
            },
            | (SourceArena::ParserMerges, SourceArena::ParserMerges) => {
                self.parser_merge_vectors.extend_from_within(range);
                return;
            },
            | (SourceArena::Preprocessor, SourceArena::ParserTokens) => (
                &self.source_vectors.0[range],
                &mut self.parser_token_vectors,
            ),
            | (SourceArena::Preprocessor, SourceArena::ParserMerges) => (
                &self.source_vectors.0[range],
                &mut self.parser_merge_vectors,
            ),
            | (SourceArena::ParserTokens, SourceArena::ParserMerges) => (
                &self.parser_token_vectors[range],
                &mut self.parser_merge_vectors,
            ),
            | _ => unreachable!("parser provenance is never copied back into earlier arenas"),
        };
        target.extend_from_slice(source);
    }

    pub(crate) fn is_tokenizing_include_string(&self) -> bool {
        self.is_tokenizing_include_string
    }

    pub(crate) fn set_is_tokenizing_include_string(&mut self, value: bool) {
        self.is_tokenizing_include_string = value;
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

    #[cold]
    #[inline(never)]
    pub(crate) fn preprocessor_tokenizer_error(&mut self, error: PreprocessorTokenizerError) {
        if !self.ignore_tokenizer_errors() {
            self.pending_errors
                .push_back(TranslationError::PreprocessorTokenizining(error));
        }
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn preprocessor_error(&mut self, error: PreprocessorError) {
        self.pending_errors
            .push_back(TranslationError::Preprocessing(error));
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn raw_preprocessor_error(
        self_pending_errors: &mut impl Extend<TranslationError>,
        error: PreprocessorError,
    ) {
        self_pending_errors.extend([TranslationError::Preprocessing(error)]);
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn parser_error(&mut self, error: ParserError) {
        self.pending_errors
            .push_back(TranslationError::Parsing(error));
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn pop_pending_error(&mut self) -> Option<TranslationError> {
        self.pending_errors.pop_front()
    }

    pub(crate) fn has_pending_errors(&self) -> bool {
        !self.pending_errors.is_empty()
    }

    pub(crate) fn take_pending_errors(&mut self) -> Vec<TranslationError> {
        std::mem::take(&mut self.pending_errors).into()
    }

    pub(crate) fn append_pending_errors(&mut self, errors: Vec<TranslationError>) {
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

    pub(crate) fn intern_source_file(&mut self, path: Box<Path>) -> u32 {
        self.source_files.intern(path)
    }

    /// Registers synthetic source text under a fresh identity, even when
    /// `path` names an earlier input, so diagnostics retained from each
    /// input keep quoting their own text.
    pub(crate) fn add_synthetic_source_file(&mut self, path: Box<Path>, text: SharedString) -> u32 {
        let index = self.source_files.push_unindexed(path);
        self.record_source_text(index, text);
        index
    }

    pub(crate) fn get_source_file(&self, index: u32) -> &Path {
        &self.source_files[index]
    }

    /// Remembers the text a source file was translated from.
    pub(crate) fn record_source_text(&mut self, index: u32, text: SharedString) {
        let index = index as usize;
        if self.source_texts.len() <= index {
            self.source_texts.resize(index + 1, None);
        }
        self.source_texts[index] = Some(text);
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
            .get(vector.index..vector.index + vector.length)
    }
}
