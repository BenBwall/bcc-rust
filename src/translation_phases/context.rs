//! [`Context`] holds the configuration, source files, interned strings,
//! literal values, source ranges, and pending diagnostics for one translation
//! unit. Its stores outlive the phases that use them. Token locations are
//! copied from temporary preprocessing storage before that storage is
//! compacted.
//!
//! For a macro that expands to `2`, the lexer records its definition's source
//! range. Preprocessing also records the invocation site. Retaining the output
//! token's range keeps both locations available to later diagnostics.
//!
//! Read [`Context`], [`Context::with_configuration`], and
//! [`Context::retain_token_source`]. Then follow [`Context::read_source_file`]
//! and [`Context::parser_error`] for source loading and diagnostic reporting.
//!
//! Files by role:
//! - Setup: `construction.rs`.
//! - Source storage: `source_files.rs`, `source_text.rs`.
//! - Provenance: `source_vectors.rs`, `expansion_sites.rs`, `merge_anchors.rs`.
//! - Segmented storage: `segmented_vec.rs`.
//! - Phase results: `diagnostics.rs`, `literals.rs`.
//! - Context fixtures: `tests.rs`.
//!
//! C99: shared storage for translation phases 1-7, §5.1.1.2 paragraph 1,
//! pp. 9-10; PDF pp. 21-22. Diagnostics: §5.1.1.3 paragraph 1, p. 11;
//! PDF p. 23. Each phase remains responsible for its language rules.

// Context construction
mod construction;

// Source files
mod source_files;
mod source_text;

// Provenance storage
mod expansion_sites;
mod merge_anchors;
mod segmented_vec;
mod source_vectors;

// Literals and diagnostics
mod diagnostics;
mod literals;

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

// Tests
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod tests;
