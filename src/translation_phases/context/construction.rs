use super::{
    ArenaMap,
    ArenaQueue,
    ArenaVec,
    Bump,
    CompilerConfiguration,
    Context,
    DedupArena,
    ExpansionSites,
    FxBuildHasher,
    KeywordTokenType,
    RegionVec,
    SourceVectorStack,
    StringCache,
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
            source_vectors: SourceVectorStack(RegionVec::new()),
            parser_token_vectors: RegionVec::new(),
            retained_vectors: RegionVec::new(),
            string_cache,
            canonical_identifiers: ArenaMap::with_hasher_in(FxBuildHasher, tu),
            literal_values: DedupArena::new(tu),
            expansion_sites: std::array::from_fn(|_| ExpansionSites::new_in(tu)),
            ignore_tokenizer_errors: false,
            pending_errors: ArenaQueue::new_in(tu),
            suppressed_errors: 0,
            relocated_errors: 0,
            source_files: DedupArena::new(tu),
            presumed_files: ArenaMap::with_hasher_in(FxBuildHasher, tu),
            include_directories: &[],
            quote_include_count: 0,
            system_include_start: 0,
            system_headers: ArenaMap::with_hasher_in(FxBuildHasher, tu),
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
