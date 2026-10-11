use rustc_hash::FxBuildHasher;

#[cfg(test)]
use super::FrameTrace;
use super::{
    LabelScopes,
    ParseFrameKind,
    Parser,
    ParserLimits,
    PreprocessedTranslationUnit,
    RecoveryState,
    ScopeStack,
    TokenCursor,
    frame_pool::FramePools,
    token_cursor::Upstream,
    token_diagnostics::{
        TokenDiagnostic,
        TokenDiagnostics,
    },
};
use crate::{
    translation_phases::{
        Context,
        GetPosition,
        GetSourceFileIndex,
        SourcePosition,
        TranslationError,
        preprocessing::Preprocessor,
    },
    util::{
        bump::{
            ArenaMap,
            ArenaQueue,
            ArenaVec,
            Bump,
        },
        region_vec::RegionVec,
    },
};

impl<'c, 'tu, 'p> Parser<'c, 'tu, 'p> {
    pub(crate) fn preprocess(
        preprocessor: Preprocessor<'tu, '_>,
        context: &mut Context<'tu>,
    ) -> PreprocessedTranslationUnit {
        Self::preprocess_with_limit(
            preprocessor,
            context,
            ParserLimits::default().source_segments,
        )
    }

    pub(super) fn preprocess_with_limit(
        mut preprocessor: Preprocessor<'tu, '_>,
        context: &mut Context<'tu>,
        source_segment_limit: usize,
    ) -> PreprocessedTranslationUnit {
        preprocessor.prepare_for_parsing();
        let upstream = Upstream::preprocess_all(preprocessor, context, source_segment_limit);
        context.release_preprocessor_vectors();
        PreprocessedTranslationUnit { upstream }
    }

    pub(super) fn with_upstream(
        upstream: Upstream,
        context: &'c mut Context<'tu>,
        arena: &'p Bump,
    ) -> Self {
        let mut token_diagnostics: TokenDiagnostics<'tu, 'p> =
            ArenaMap::with_hasher_in(FxBuildHasher, arena);
        for (index, error) in context.pending_errors.iter().enumerate() {
            let (spelling, source, occurrence) = match error {
                | TranslationError::Extension(error) => (
                    error.spelling(),
                    error.source_vectors,
                    TokenDiagnostic::Extension(error.suppressed),
                ),
                | TranslationError::Preprocessing(error) => {
                    let Some(spelling) = super::msvc::constant_diagnostic(&error.error_type) else {
                        continue;
                    };
                    (
                        spelling,
                        error.source_vectors,
                        TokenDiagnostic::Constant(index),
                    )
                },
                | _ => continue,
            };
            let vectors = context.copy_source_vectors_in(source, arena);
            token_diagnostics
                .entry((spelling, vectors, context.user_source_end(source)))
                .or_insert_with(|| ArenaQueue::new_in(arena))
                .push_back(occurrence);
        }
        // Reserved resource-header type is present even without <stdarg.h>.
        // C99: implementation extension supporting §7.15p3, p. 249; PDF p. 261.
        let mut scopes = ScopeStack::new_in(arena);
        scopes.publish(
            context.string_cache.intern("__builtin_va_list"),
            super::scope::NameClass::Typedef,
        );
        // GNU reserved builtin typedefs (C99 §4p6 extension).
        for name in ["__int128_t", "__uint128_t"] {
            scopes.publish(
                context.string_cache.intern(name),
                super::scope::NameClass::Typedef,
            );
        }
        Self {
            cursor: TokenCursor::new(upstream),
            arena,
            tree: context.tu_arena(),
            context,
            frames: ArenaVec::new_in(arena),
            pools: FramePools::new_in(arena),
            retained_frame_nodes: 0,
            returned: None,
            #[cfg(test)]
            syntax: super::SyntaxLog::default(),
            syntax_nodes: 0,
            emitted_roots: RegionVec::new(),
            scopes,
            label_scopes: LabelScopes::new_in(arena),
            func_name: None,
            switch_scopes: ArenaVec::new_in(arena),
            binding_scan: ArenaVec::new_in(arena),
            recovery: RecoveryState::new_in(arena),
            hard_error_count: 0,
            pedantic_suppression: 0,
            token_diagnostics,
            switch_floor: 0,
            active_frame: ParseFrameKind::ExternalDeclaration,
            has_external_declaration: false,
            reported_empty_translation_unit: false,
            external_declaration_count: 0,
            limits: ParserLimits::default(),
            resource_limit_reported: false,
            #[cfg(test)]
            trace: FrameTrace::new(),
            #[cfg(test)]
            action_budget: None,
        }
    }

    /// Creates an idle parser over `preprocessed` that borrows `context` for
    /// the whole parse. Its syntax tree goes to the context's
    /// translation-unit arena and its working memory comes from the parse
    /// arena.
    pub(crate) fn from_preprocessed(
        preprocessed: PreprocessedTranslationUnit,
        context: &'c mut Context<'tu>,
        arena: &'p Bump,
    ) -> Self {
        Self::with_upstream(preprocessed.upstream, context, arena)
    }

    /// Preprocesses the whole translation unit, then creates an idle parser
    /// over the result. Every preprocessing diagnostic is pending in
    /// `context` before any parser diagnostic.
    ///
    /// C99: the input is the translation unit (§5.1.1.1, p. 9; PDF p. 21)
    /// left by phases 1-6, whose preprocessing tokens phase 7 converts to
    /// tokens (§5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22).
    #[cfg(test)]
    pub(crate) fn new(
        preprocessor: Preprocessor<'tu, '_>,
        context: &'c mut Context<'tu>,
        arena: &'p Bump,
    ) -> Self {
        Self::new_with_config(preprocessor, context, ParserLimits::default(), arena)
    }

    #[cfg(test)]
    pub(super) fn new_with_config(
        preprocessor: Preprocessor<'tu, '_>,
        context: &'c mut Context<'tu>,
        limits: ParserLimits,
        arena: &'p Bump,
    ) -> Self {
        let preprocessed =
            Self::preprocess_with_limit(preprocessor, context, limits.source_segments);
        let mut parser = Self::from_preprocessed(preprocessed, context, arena);
        parser.limits = limits;
        parser
    }

    #[cfg(test)]
    pub(super) fn new_with_limits(
        preprocessor: Preprocessor<'tu, '_>,
        context: &'c mut Context<'tu>,
        limits: ParserLimits,
        arena: &'p Bump,
    ) -> Self {
        Self::new_with_config(preprocessor, context, limits, arena)
    }

    #[cfg(test)]
    pub(super) fn with_action_budget(mut self, action_budget: usize) -> Self {
        self.action_budget = Some(action_budget);
        self
    }

    /// Where the preprocessor stopped reading, for end-of-input locations.
    pub(super) fn position(&self) -> SourcePosition {
        self.cursor.upstream.position(self.context)
    }

    /// The translation context this parser borrows, for tests outside the
    /// parser.
    #[cfg(test)]
    pub(crate) fn context(&mut self) -> &mut Context<'tu> {
        self.context
    }
}

impl GetSourceFileIndex for Parser<'_, '_, '_> {
    fn source_file_index(&self) -> u32 {
        self.cursor.upstream.source_file_index()
    }
}
