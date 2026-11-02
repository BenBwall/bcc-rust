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

use std::path::Path;

use expansion_sites::ExpansionSites;
use rustc_hash::FxBuildHasher;
pub(crate) use source_files::FileSystem;
use source_text::SourceText;

#[cfg(test)]
use super::provenance::{
    SourcePosition,
    SourceVectors,
};
use super::{
    TranslationError,
    provenance::{
        SourceVector,
        SourceVectorStack,
    },
};
use crate::{
    configuration::CompilerConfiguration,
    translation_phases::preprocessing::LiteralUnit,
    util::{
        bump::{
            ArenaMap,
            ArenaQueue,
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

/// A pragma a text of phase 4 writes back where it was.
/// C99: §6.10.6 paragraph 1, p. 159; PDF p. 171.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TextPragma<'tu> {
    /// Its output-token ordinal ([`Context::preprocessed_tokens`] before
    /// the next token).
    pub(crate) position: usize,
    /// The source file being read and the line the reader was on.
    pub(crate) file:     u32,
    pub(crate) line:     u32,
    /// `#pragma` and its tokens as spelled, each run of white space as one
    /// space.
    pub(crate) text:     &'tu str,
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "Each flag is an independent fact about the translation unit's input."
)]
pub(crate) struct Context<'tu> {
    tu: &'tu Bump,
    pub(crate) configuration: CompilerConfiguration,
    pub(crate) preprocessing_options: &'tu [crate::configuration::PreprocessingOption<'tu>],
    /// The library functions code generation may treat as builtins.
    pub(crate) library_builtins: crate::configuration::LibraryBuiltins<'tu>,
    /// What code generation may assume where C leaves the behaviour of
    /// pointers undefined.
    pub(crate) code_assumptions: crate::target::CodeAssumptions,
    /// The source is already preprocessed (GCC's `-fpreprocessed`): no
    /// macro is replaced, and only line markers, `#pragma`, `#ident` and
    /// `#sccs` take effect; `#define` and `#undef` are read but, with
    /// replacement off, change nothing.
    pub(crate) preprocessed_input: bool,
    /// Phase 4 runs for a text of its tokens, as `-E` prints it; set before
    /// the preprocessor is made. Adjacent string literals stay apart, which
    /// phase 6 would concatenate, and phases 5 and 7 neither decode nor
    /// check a token: each keeps its spelling, so a character no C token
    /// starts with, a number that is no constant or an escape C does not
    /// define passes through unreported, and a `'` or `"` that closes on no
    /// line keeps the rest of its line with a warning, as in GCC's and
    /// Clang's `-E`.
    /// C99: §5.1.1.2 paragraph 1 items 3-7, p. 10; PDF p. 22.
    pub(crate) phase_four_text: bool,
    /// A source file held malformed UTF-8, whose bytes its text carries as
    /// [`crate::util::raw_bytes`] stand-ins; only then are they bytes.
    pub(crate) raw_source_bytes: bool,
    /// Each `#include` that entered a header, in order, with its nesting
    /// depth (1 for a header the main file includes), when recorded for
    /// GCC's `-H` or `cl /showIncludes`. An inclusion whose include guard
    /// is already defined, which GCC and Clang do not read again, is left
    /// out.
    pub(crate) include_entries: Option<ArenaVec<'tu, IncludeEntry>>,
    /// The header read before the main file and the `-include` files, as
    /// GCC on a glibc target reads `<stdc-predef.h>` (its
    /// `TARGET_C_PREINCLUDE`) when hosted and searching the standard
    /// directories: it defines `__STDC_IEC_559__` and `__STDC_ISO_10646__`,
    /// which the C library knows to be true. `None` when there is none.
    pub(crate) preinclude: Option<&'tu str>,
    /// Whether a header that cannot be found is taken to be generated
    /// later, as GCC's `-MG` takes it: it is no error, and the dependency
    /// rules list it as written.
    pub(crate) missing_headers_generated: bool,
    /// The headers [`Self::missing_headers_generated`] took to be
    /// generated, each interned as written, with whether its name was a
    /// `<...>` one.
    pub(crate) generated_headers: ArenaVec<'tu, (u32, bool)>,
    /// Where included files and `#embed` resources are found and read:
    /// the host's file system for the command line, only in-memory files
    /// for the explorer.
    pub(crate) file_system: FileSystem<'tu>,
    /// Output-token ordinals preserve directive order across includes/macros.
    pub(crate) preprocessed_tokens: usize,
    pub(crate) pack_pragmas: ArenaVec<'tu, super::pragma_pack::PackPragma<'tu>>,
    /// The names `#pragma weak` makes weak, which semantic analysis keeps
    /// out of the values Clang's evaluator folds, each with its output-token
    /// ordinal ([`Self::preprocessed_tokens`] before the next token).
    pub(crate) weak_pragmas: ArenaVec<'tu, (usize, StringCacheId)>,
    /// A pragma the identity's compiler hands to its parser was read,
    /// which keeps a translation unit that declares nothing from being
    /// empty.
    pub(crate) pragma_content: bool,
    /// The pragmas a text of phase 4 ([`Self::phase_four_text`]) writes back
    /// where they were.
    pub(crate) text_pragmas: ArenaVec<'tu, TextPragma<'tu>>,
    /// GCC's `#pragma scalar_storage_order` directives, in order, under the
    /// GCC identity only.
    pub(crate) storage_order_pragmas:
        ArenaVec<'tu, super::pragma_storage_order::StorageOrderPragma>,
    pub(crate) source_vectors: SourceVectorStack,
    parser_token_vectors: RegionVec<SourceVector>,
    retained_vectors: RegionVec<SourceVector>,
    pub(crate) string_cache: StringCache<'tu>,
    pub(crate) canonical_identifiers: ArenaMap<'tu, StringCacheId, StringCacheId>,
    literal_values: DedupArena<'tu, &'tu [LiteralUnit], FxBuildHasher>,
    /// Sparse endpoints follow their source arena's lifetime.
    expansion_sites: [ExpansionSites<'tu>; 3],
    /// Where the outermost macro invocation that produced a token begins,
    /// keyed by the token's first vector. Unlike the endpoints, these last
    /// for the whole translation unit, so semantic diagnostics can use them.
    expansion_starts: [ExpansionSites<'tu>; 3],
    ignore_tokenizer_errors: bool,
    /// Phase 4 is skipping a conditional group, whose lines it reads only
    /// for conditional directives (C99 §6.10.1p6).
    pub(crate) skipping_group: bool,
    pub(super) pending_errors: ArenaQueue<'tu, TranslationError<'tu>>,
    suppressed_errors: usize,
    /// Error-severity diagnostics already taken from `pending_errors`.
    drained_errors: usize,
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
    /// The regions GNU line markers with flag 3 make system headers, as
    /// in preprocessed input, in the order they were opened.
    system_regions: ArenaVec<'tu, SystemRegion>,
    /// Original text of each source file, indexed like `source_files`, kept
    /// so diagnostics can quote the lines they point at.
    source_texts: ArenaVec<'tu, Option<SourceText<'tu>>>,
}

/// One `#include` that entered a header: the header's source file index,
/// its nesting depth, and whether it was found beside an includer in the
/// working directory or one reached from it that way, whose path Clang
/// begins with `./`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IncludeEntry {
    pub(crate) file:   u32,
    pub(crate) depth:  u32,
    pub(crate) here:   bool,
    /// Reached from a command-line include, which `-H` leaves out.
    pub(crate) forced: bool,
}

/// Bytes `start..end` of physical file `physical` that a line marker with
/// flag 3 made a system header; `end` is `u32::MAX` until a later marker in
/// the file ends the region.
/// GNU extension: GCC preprocessor manual, "Preprocessor Output".
#[derive(Clone, Copy, Debug)]
pub(crate) struct SystemRegion {
    physical: u32,
    start:    u32,
    end:      u32,
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
