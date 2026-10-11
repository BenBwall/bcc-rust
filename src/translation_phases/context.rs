//! Translation context shared by every phase: configuration, provenance
//! arenas, interned strings, source files, and pending diagnostics.
//!
//! It connects translation phases 1-7 (§5.1.1.2, pp. 9-10; PDF pp. 21-22)
//! and holds the diagnostics they report (§5.1.1.3p1, p. 11; PDF p. 23).

mod construction;

mod diagnostics;

mod expansion_sites;

mod literals;

mod merge_anchors;

mod segmented_vec;

mod source_files;

mod source_text;

mod source_vectors;

use std::{
    cell::OnceCell,
    ffi::OsStr,
    path::Path,
};

use expansion_sites::ExpansionSites;
use merge_anchors::MergeAnchors;
use rustc_hash::FxBuildHasher;
use segmented_vec::SegmentedVec;
use source_text::SourceText;

use super::{
    ErrorSeverity,
    GetPosition,
    GetSeverity,
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
    headers::HeaderSearch,
    translation_phases::{
        initial_processing::InitialProcessorError,
        parsing::ParserError,
        preprocessing::{
            KeywordTokenType,
            LiteralUnit,
            PreprocessorError,
        },
        preprocessor_tokenizer::PreprocessorTokenizerError,
    },
    util::{
        bump::{
            ArenaMap,
            ArenaQueue,
            ArenaSet,
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
    },
};

pub(crate) struct Context<'tu> {
    tu: &'tu Bump,
    pub(crate) configuration: CompilerConfiguration,
    pub(crate) preprocessing_options: &'tu [crate::configuration::PreprocessingOption<'tu>],
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
    suppressed_errors: usize,
    /// How many leading pending errors no longer refer to the preprocessor
    /// arena, so compaction relocates each error's provenance only once.
    relocated_errors: usize,
    source_files: DedupArena<'tu, &'tu Path, FxBuildHasher>,
    /// Presumed filename identities introduced by `#line`, mapped to the
    /// physical source file whose bytes their vectors still index.
    presumed_files: ArenaMap<'tu, u32, u32>,
    /// Every configured header search entry in order, the resource
    /// directory included; the first `quote_include_count` are `-iquote`
    /// entries. An entry's index is its identity for `#include_next`.
    include_directories: &'tu [&'tu Path],
    quote_include_count: usize,
    /// The index of the first system entry in `include_directories`.
    system_include_start: usize,
    /// Each system header, with the byte offset from which it is one: 0
    /// for a header found through a system directory, or the position of
    /// its `#pragma GCC system_header`.
    system_headers: ArenaMap<'tu, u32, u32>,
    /// Original text of each source file, indexed like `source_files`, kept
    /// so diagnostics can quote the lines they point at.
    source_texts: ArenaVec<'tu, Option<SourceText<'tu>>>,
}

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod tests;
