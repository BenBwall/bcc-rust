#[cfg(feature = "benchmarking-internals")]
use std::path::PathBuf;
use std::{
    collections::VecDeque,
    convert::Infallible,
    fmt::{
        Debug,
        Display,
        Formatter,
        Result as FmtResult,
    },
    hash::Hash,
    path::Path,
};

use crate::{
    configuration::CompilerConfiguration,
    diagnostics::{
        Diagnostic,
        ToDiagnostic,
    },
    util::{
        dedup_arena::DedupArena,
        shared::SharedString,
        string_cache::StringCache,
        vector_slice::{
            UsizeExt,
            VectorSlice,
        },
    },
};

#[derive(Error, Debug)]
pub(crate) enum TranslationError {
    #[error(transparent)]
    InitialProcessing(InitialProcessorError),
    #[error(transparent)]
    PreprocessorTokenizining(PreprocessorTokenizerError),
    #[error(transparent)]
    Preprocessing(PreprocessorError),
    #[error(transparent)]
    Parsing(ParserError),
}

impl GetSeverity for TranslationError {
    fn severity(&self) -> ErrorSeverity {
        match self {
            | Self::InitialProcessing(error) => error.severity(),
            | Self::PreprocessorTokenizining(error) => error.severity(),
            | Self::Preprocessing(error) => error.severity(),
            | Self::Parsing(error) => error.severity(),
        }
    }
}

impl ToDiagnostic for TranslationError {
    fn to_diagnostic(&self, context: &Context, source: SourceVectors) -> Diagnostic {
        match self {
            | Self::InitialProcessing(error) => error.to_diagnostic(context, source),
            | Self::PreprocessorTokenizining(error) => error.to_diagnostic(context, source),
            | Self::Preprocessing(error) => error.to_diagnostic(context, source),
            | Self::Parsing(error) => error.to_diagnostic(context, source),
        }
    }
}

impl GetPosition for TranslationError {
    fn position(&self, context: &Context) -> SourcePosition {
        match self {
            | Self::InitialProcessing(error) => error.position(context),
            | Self::PreprocessorTokenizining(error) => error.position(context),
            | Self::Preprocessing(error) => error.position(context),
            | Self::Parsing(error) => error.position(context),
        }
    }
}

impl GetSourceVectors for TranslationError {
    fn source_vectors(&self, context: &mut Context) -> SourceVectors {
        match self {
            | Self::InitialProcessing(error) => error.source_vectors(context),
            | Self::PreprocessorTokenizining(error) => error.source_vectors(context),
            | Self::Preprocessing(error) => error.source_vectors(context),
            | Self::Parsing(error) => error.source_vectors(context),
        }
    }
}

use rustc_hash::FxBuildHasher;
use smallstr::SmallString;
use thiserror::Error;

use self::{
    initial_processing::InitialProcessorError,
    parsing::ParserError,
    preprocessing::PreprocessorError,
    preprocessor_tokenizer::PreprocessorTokenizerError,
};

pub(crate) mod initial_processing;
pub(crate) mod parsing;
pub(crate) mod preprocessing;
pub(crate) mod preprocessor_tokenizer;

pub(crate) type TokenString = SmallString<[u8; 1024]>;

trait StrExt {
    /// Returns the character at the given index,
    /// or `None` if the index is out of bounds or is in the middle of a
    /// character.
    fn char_at(&self, index: usize) -> Option<char>;

    /// Returns the character at the given index but in lowercase,
    /// See [`StrExt::char_at`] for more information.
    #[expect(
        dead_code,
        reason = "We're not currently using this method, it's here for potential future use."
    )]
    fn char_at_case_insensitive(&self, index: usize) -> Option<char> {
        self.char_at(index).map(|c| c.to_ascii_lowercase())
    }

    /// Returns the string cloned into a [`TokenString`].
    fn to_token_string(&self) -> TokenString
    where
        for<'a> &'a Self: Into<TokenString>,
    {
        self.into()
    }
}

impl StrExt for str {
    fn char_at(&self, index: usize) -> Option<char> {
        self.get(index..)?.chars().next()
    }
}

#[doc(hidden)]
#[macro_export]
macro_rules! bail {
    ($e:expr $(,)?) => {
        match $e {
            | Ok(v) => v,
            | Err(e) => return Some(Err(e.into())),
        }
    };
}

#[expect(dead_code, reason = "We aren't using the Note variant yet")]
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum ErrorSeverity {
    Warning,
    Error,
    Note,
}

impl Display for ErrorSeverity {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            | Self::Warning => f.write_str("warning"),
            | Self::Error => f.write_str("error"),
            | Self::Note => f.write_str("note"),
        }
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Copy, Hash)]
pub(crate) struct SourcePosition {
    pub(crate) index:  usize,
    pub(crate) line:   u32,
    pub(crate) column: u32,
}

impl Default for SourcePosition {
    fn default() -> Self {
        Self {
            index:  0,
            line:   1,
            column: 1,
        }
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Hash)]
pub(crate) struct SourceVector {
    pub(crate) index:             usize,
    pub(crate) column:            u32,
    pub(crate) line:              u32,
    pub(crate) source_file_index: u32,
    pub(crate) length:            usize,
}

impl Default for SourceVector {
    fn default() -> Self {
        Self {
            index:             0,
            column:            1,
            line:              1,
            source_file_index: 0,
            length:            0,
        }
    }
}

impl GetPosition for SourceVector {
    #[inline(always)]
    fn position(&self, _context: &Context) -> SourcePosition {
        SourcePosition {
            index:  self.index,
            line:   self.line,
            column: self.column,
        }
    }
}
pub(crate) type SourceVectors = VectorSlice<SourceVector>;

impl GetPosition for SourceVectors {
    #[inline(always)]
    fn position(&self, context: &Context) -> SourcePosition {
        let start = context.first_source_vector(*self);
        SourcePosition {
            index:  start.index,
            line:   start.line,
            column: start.column,
        }
    }
}

impl GetSourceVectors for SourceVectors {
    fn source_vectors(&self, _context: &mut Context) -> SourceVectors {
        *self
    }
}

pub(crate) trait GetSeverity {
    fn severity(&self) -> ErrorSeverity;
}

impl GetSeverity for ErrorSeverity {
    fn severity(&self) -> ErrorSeverity {
        *self
    }
}

impl GetSeverity for Infallible {
    fn severity(&self) -> ErrorSeverity {
        match *self {}
    }
}

pub(crate) trait GetSourceFileIndex {
    fn source_file_index(&self) -> u32;
}

pub(crate) trait GetPosition {
    fn position(&self, context: &Context) -> SourcePosition;
    #[inline(always)]
    fn index(&self, context: &Context) -> usize {
        self.position(context).index
    }
    #[inline(always)]
    fn column(&self, context: &Context) -> u32 {
        self.position(context).column
    }
    #[inline(always)]
    fn line(&self, context: &Context) -> u32 {
        self.position(context).line
    }
}

pub(crate) trait GetSourceVectors {
    fn source_vectors(&self, context: &mut Context) -> SourceVectors;
}

pub(crate) trait SetPosition: GetPosition {
    fn set_position(&mut self, context: &mut Context, position: SourcePosition);
    #[expect(
        dead_code,
        reason = "Position setters are retained for translation-phase implementations."
    )]
    #[inline(always)]
    fn set_index(&mut self, context: &mut Context, index: usize) {
        self.set_position(
            context,
            SourcePosition {
                index,
                line: self.line(context),
                column: self.column(context),
            },
        );
    }
    #[expect(
        dead_code,
        reason = "Position setters are retained for translation-phase implementations."
    )]
    #[inline(always)]
    fn set_column(&mut self, context: &mut Context, column: u32) {
        self.set_position(
            context,
            SourcePosition {
                index: self.index(context),
                line: self.line(context),
                column,
            },
        );
    }
    #[inline(always)]
    fn set_line(&mut self, context: &mut Context, line: u32) {
        self.set_position(
            context,
            SourcePosition {
                index: self.index(context),
                line,
                column: self.column(context),
            },
        );
    }
}

pub(crate) trait SetSourceFileIndex {
    fn set_source_file_index(&mut self, context: &mut Context, source_file_index: u32);
}

impl GetPosition for SourcePosition {
    #[inline(always)]
    fn position(&self, _context: &Context) -> SourcePosition {
        *self
    }
}

impl GetPosition for Infallible {
    #[inline(always)]
    fn position(&self, _context: &Context) -> SourcePosition {
        match *self {}
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
#[expect(
    clippy::struct_field_names,
    reason = "I think using source and source_file_index as member names is fine here."
)]
pub(crate) struct SourceFile {
    pub(crate) source_file_index: u32,
    pub(crate) source:            SharedString,
    pub(crate) line:              u32,
    pub(crate) column:            u32,
    pub(crate) index:             usize,
}

impl Default for SourceFile {
    fn default() -> Self {
        Self {
            source_file_index: 0,
            source:            SharedString::default(),
            line:              1,
            column:            1,
            index:             0,
        }
    }
}

impl SourceFile {
    pub(crate) fn new(source_file_index: u32, source: SharedString) -> Self {
        Self {
            source_file_index,
            source,
            line: 1,
            column: 1,
            index: 0,
        }
    }
}

impl GetPosition for SourceFile {
    #[inline(always)]
    fn position(&self, _context: &Context) -> SourcePosition {
        SourcePosition {
            index:  self.index,
            line:   self.line,
            column: self.column,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct SourceVectorStack(pub(crate) Vec<SourceVector>);

impl Display for SourceVectorStack {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        for (i, vector) in self.0.iter().enumerate() {
            writeln!(
                f,
                "SourceVector {}: index: {}, line: {}, column: {}, length: {}",
                i, vector.index, vector.line, vector.column, vector.length
            )?;
        }
        Ok(())
    }
}

/// Storage that a [`SourceVectors`] range indexes, encoded in the two high bits
/// of its `start_index`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SourceArena {
    /// Vectors produced by initial processing, tokenization, and
    /// preprocessing.
    Preprocessor,
    /// One copy of each parser-fetched token's vectors, in fetch order, so
    /// provenance of consecutive tokens is adjacent and merges in O(1).
    ParserTokens,
    /// Parser merges whose operands are not adjacent in one arena.
    ParserMerges,
}

impl SourceArena {
    const INDEX_BITS: u32 = 30;
    const INDEX_MASK: u32 = (1 << Self::INDEX_BITS) - 1;

    #[inline(always)]
    fn decode(source_vectors: SourceVectors) -> (Self, u32) {
        let arena = match source_vectors.start_index >> Self::INDEX_BITS {
            | 0 => Self::Preprocessor,
            | 1 => Self::ParserTokens,
            | 2 => Self::ParserMerges,
            | _ => unreachable!("empty source ranges carry no arena"),
        };
        (arena, source_vectors.start_index & Self::INDEX_MASK)
    }

    #[inline(always)]
    fn encode(self, start: u32, end: u32) -> SourceVectors {
        assert!(end <= Self::INDEX_MASK, "source arena overflow");
        let tag = (self as u32) << Self::INDEX_BITS;
        SourceVectors::new(tag | start, tag | end)
    }
}

pub(crate) struct Context {
    pub(crate) configuration:     CompilerConfiguration,
    pub(crate) source_vectors:    SourceVectorStack,
    parser_token_vectors:         Vec<SourceVector>,
    parser_merge_vectors:         Vec<SourceVector>,
    pub(crate) string_cache:      StringCache,
    is_tokenizing_include_string: bool,
    ignore_tokenizer_errors:      bool,
    pending_errors:               VecDeque<TranslationError>,
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
    #[expect(
        dead_code,
        reason = "We aren't using this yet, but we will be when the parser is implemented."
    )]
    pub(crate) fn raw_parser_error(
        self_pending_errors: &mut impl Extend<TranslationError>,
        error: ParserError,
    ) {
        self_pending_errors.extend([TranslationError::Parsing(error)]);
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn pop_pending_error(&mut self) -> Option<TranslationError> {
        self.pending_errors.pop_front()
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

    fn first_source_vector(&self, source_vectors: SourceVectors) -> &SourceVector {
        let (arena, start) = SourceArena::decode(source_vectors);
        &self.arena(arena)[start as usize]
    }

    pub(crate) fn intern_source_file(&mut self, path: Box<Path>) -> u32 {
        self.source_files.intern(path)
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

pub(crate) trait TranslationPhase:
    GetPosition + SetPosition + GetSourceFileIndex + SetSourceFileIndex
{
    type Item;
    fn next_item(&mut self, context: &mut Context) -> Option<Self::Item>;
}

#[cfg(feature = "benchmarking-internals")]
pub(crate) fn box_path_from_str(s: &str) -> Box<Path> {
    PathBuf::from(s).into_boxed_path()
}
