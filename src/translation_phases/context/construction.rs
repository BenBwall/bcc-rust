//! Creates translation-unit storage and initializes the reserved keyword
//! identities. Diagnostic text and slices borrow the same arena.
//!
//! C99: shared storage for phases 1-7, §5.1.1.2 paragraph 1, pp. 9-10;
//! PDF pp. 21-22. Diagnostic support: §5.1.1.3 paragraph 1, p. 11;
//! PDF p. 23. Language rules belong to the individual phases.

use rustc_hash::FxBuildHasher;

use super::{
    Context,
    FileSystem,
    expansion_sites::ExpansionSites,
};
use crate::{
    configuration::CompilerConfiguration,
    translation_phases::{
        preprocessing::KeywordTokenType,
        provenance::SourceVectorStack,
    },
    util::{
        bump::{
            ArenaMap,
            ArenaQueue,
            ArenaVec,
            Bump,
        },
        dedup_arena::DedupArena,
        region_vec::RegionVec,
        string_cache::StringCache,
    },
};

impl<'tu> Context<'tu> {
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
        for &spelling in KeywordTokenType::ALIASES {
            _ = string_cache.intern(spelling);
        }
        Self {
            tu,
            configuration,
            preprocessing_options: &[],
            preprocessed_input: false,
            phase_four_text: false,
            raw_source_bytes: false,
            include_entries: None,
            missing_headers_generated: false,
            generated_headers: ArenaVec::new_in(tu),
            library_builtins: crate::configuration::LibraryBuiltins::All,
            code_assumptions: crate::target::CodeAssumptions::default(),
            file_system: FileSystem::Host,
            preprocessed_tokens: 0,
            pack_pragmas: ArenaVec::new_in(tu),
            weak_pragmas: ArenaVec::new_in(tu),
            text_pragmas: ArenaVec::new_in(tu),
            source_vectors: SourceVectorStack(RegionVec::new()),
            parser_token_vectors: RegionVec::new(),
            retained_vectors: RegionVec::new(),
            string_cache,
            canonical_identifiers: ArenaMap::with_hasher_in(FxBuildHasher, tu),
            literal_values: DedupArena::new(tu),
            expansion_sites: std::array::from_fn(|_| ExpansionSites::new_in(tu)),
            expansion_starts: std::array::from_fn(|_| ExpansionSites::new_in(tu)),
            ignore_tokenizer_errors: false,
            pending_errors: ArenaQueue::new_in(tu),
            suppressed_errors: 0,
            drained_errors: 0,
            relocated_errors: 0,
            source_files: DedupArena::new(tu),
            presumed_files: ArenaMap::with_hasher_in(FxBuildHasher, tu),
            include_directories: &[],
            quote_include_count: 0,
            system_include_start: 0,
            system_headers: ArenaMap::with_hasher_in(FxBuildHasher, tu),
            system_regions: ArenaVec::new_in(tu),
            source_texts: ArenaVec::new_in(tu),
        }
    }

    #[cfg(test)]
    pub(crate) fn new(tu: &'tu Bump) -> Self {
        Self::with_configuration(tu, CompilerConfiguration::default())
    }

    /// The translation-unit arena, which holds everything that lives until
    /// the translation unit ends, the syntax tree included.
    pub(crate) fn tu_arena(&self) -> &'tu Bump {
        self.tu
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn diagnostic_text(&self, text: &str) -> &'tu str {
        self.tu.alloc_str(text)
    }

    /// Formats diagnostic text straight into the translation-unit arena.
    #[cold]
    #[inline(never)]
    pub(crate) fn diagnostic_format(&self, arguments: std::fmt::Arguments<'_>) -> &'tu str {
        crate::diagnostics::format_arguments_in(self.tu, arguments)
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn diagnostic_slice<T: Copy>(&self, values: &[T]) -> &'tu mut [T] {
        self.tu.alloc_slice_copy(values)
    }
}
