//! The parser driver loop and helpers shared by every grammar frame.
//!
//! Translation phase 7 (§5.1.1.2 paragraph 1, p. 10; PDF p. 22): the driver
//! takes the converted tokens of one translation unit (§5.1.1.1, p. 9;
//! PDF p. 21) and steps the frame machine until each `external-declaration`
//! of `translation-unit` reduces (§6.9 paragraph 1, p. 140; PDF p. 152).
//! It emits the diagnostics §5.1.1.3, p. 11; PDF p. 23 requires and
//! resynchronizes after them. Grammar nesting lives on an explicit frame
//! stack, so no nesting minimum of §5.2.4.1, p. 20; PDF p. 32 becomes a
//! recursion limit; footnote 13 there asks implementations to avoid fixed
//! translation limits, and the remaining resource ceilings are
//! representation bounds only.

use std::cell::Cell;

use rustc_hash::FxBuildHasher;

#[cfg(test)]
use super::{
    FrameTrace,
    machine::FrameTraceEvent,
};
use super::{
    ParsedTranslationUnit,
    Parser,
    ParserLimits,
    PreprocessedTranslationUnit,
    errors::{
        ParserError,
        ParserErrorType,
        ParserResource,
        ParserWarningGroup,
        RecoverySummary,
        RelatedParserDiagnostic,
    },
    expression_operators::is_operator,
    external_declaration::ExternalDeclarationFrame,
    frame_pool::FramePools,
    machine::{
        ParseAction,
        ParseFrame,
        ParseFrameKind,
        ParseValue,
    },
    modern::{
        ModernFrame,
        ModernKind,
    },
    recovery::{
        DelimiterDepth,
        ExpressionTerminator,
        RecoveryState,
        SynchronizationKind,
        SynchronizationSet,
    },
    scope::{
        LabelScopes,
        ScopeStack,
    },
    statement::is_statement_keyword,
    syntax::{
        ConditionalExpression,
        Expression,
        ExpressionType,
        ExternalDeclaration,
        Identifier,
    },
    syntax_log::TreeNode,
    token_cursor::{
        TokenCursor,
        Upstream,
    },
};
use crate::{
    translation_phases::{
        Context,
        ErrorSeverity,
        GetPosition,
        GetSeverity,
        GetSourceFileIndex,
        SourcePosition,
        SourceVector,
        SourceVectors,
        TranslationError,
        preprocessing::{
            KeywordTokenType,
            OperatorTokenType,
            Preprocessor,
            StringTokenType,
            Token,
            TokenType,
        },
    },
    util::{
        arena_list::ArenaList,
        bump::{
            ArenaMap,
            ArenaQueue,
            ArenaVec,
            Bump,
        },
        region_vec::RegionVec,
    },
};

/// Each spelling/provenance pair owns its FIFO of diagnostic occurrences.
/// Macro replacements can share provenance, so occurrences must not be merged.
/// C99: §5.1.1.3, p. 11; PDF p. 23. GNU extension markers suppress only
/// diagnostics in their owning parser frame, without reordering the queue.
pub(super) type TokenDiagnostics<'tu, 'p> = ArenaMap<
    'p,
    (&'tu str, &'p [SourceVector], Option<SourceVector>),
    ArenaQueue<'p, TokenDiagnostic<'tu>>,
>;

/// One pending diagnostic that the frame consuming its token may withdraw.
#[derive(Debug, Clone, Copy)]
pub(super) enum TokenDiagnostic<'tu> {
    /// An extension diagnostic, suppressed through its marker.
    Extension(&'tu Cell<bool>),
    /// A phase-7 constant-conversion error at its pending-queue position,
    /// which MSVC assembly withdraws (see
    /// [`super::msvc::constant_diagnostic`]).
    Constant(usize),
}

/// A borrowed lookup keeps temporary context borrows out of stored key
/// lifetimes.
#[derive(Hash)]
struct DiagnosticLookup<'a> {
    spelling: &'a str,
    vectors:  &'a [SourceVector],
    user_end: Option<SourceVector>,
}
impl hashbrown::Equivalent<(&str, &[SourceVector], Option<SourceVector>)> for DiagnosticLookup<'_> {
    fn equivalent(&self, key: &(&str, &[SourceVector], Option<SourceVector>)) -> bool {
        self.spelling == key.0 && self.vectors == key.1 && self.user_end == key.2
    }
}

impl<'c, 'tu, 'p> Parser<'c, 'tu, 'p> {
    /// Reuses the parser arena's pooled storage for a modern syntax frame.
    pub(super) fn pooled_modern_frame(&mut self, kind: ModernKind) -> ParseFrame<'tu, 'p> {
        ParseFrame::Modern(self.pools.modern(ModernFrame::new(
            self.arena,
            kind,
            self.hard_error_count,
        )))
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

    fn preprocess_with_limit(
        mut preprocessor: Preprocessor<'tu, '_>,
        context: &mut Context<'tu>,
        source_segment_limit: usize,
    ) -> PreprocessedTranslationUnit {
        preprocessor.prepare_for_parsing();
        let upstream = Upstream::preprocess_all(preprocessor, context, source_segment_limit);
        context.release_preprocessor_vectors();
        PreprocessedTranslationUnit { upstream }
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

    #[cfg(test)]
    fn new_with_config(
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

    fn with_upstream(upstream: Upstream, context: &'c mut Context<'tu>, arena: &'p Bump) -> Self {
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
            let vectors =
                arena.alloc_slice_fill_iter(context.get_source_vectors(source).iter().cloned());
            token_diagnostics
                .entry((spelling, &*vectors, context.user_source_end(source)))
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

    /// Parses the complete phase-7 input into one source-ordered translation
    /// unit whose roots and syntax live in the translation-unit arena.
    ///
    /// Roots already observed through [`Self::next_item`] remain part of the
    /// aggregate result so mixing the streaming adapter with the owning seam
    /// cannot silently produce a suffix-only translation unit.
    pub(crate) fn parse_translation_unit(mut self) -> ParsedTranslationUnit<'tu> {
        while let Some(root) = self.drive() {
            self.emitted_roots.push(root);
        }
        ParsedTranslationUnit {
            roots: self.emitted_roots,
        }
    }

    /// Runs owned frame actions until one external declaration reduces or EOF
    /// is observed between declarations.
    ///
    /// C99: translation-unit is a nonempty sequence of external-declaration
    /// values under §6.9, p. 140; PDF p. 152.
    pub(super) fn drive(&mut self) -> Option<ExternalDeclaration<'tu>> {
        // A resource failure is terminal: the remaining input is neither
        // fetched nor parsed, so it cannot grow the exhausted storage.
        if self.resource_limit_reported {
            return None;
        }
        if let Some(token) = self.cursor.upstream.preprocessing_limit_token.take() {
            return self.resource_failure_at(
                ParserResource::SourceSegments,
                self.limits.source_segments,
                Some(token),
            );
        }
        loop {
            #[cfg(test)]
            assert!(
                self.action_budget
                    .is_none_or(|budget| self.trace.len() < budget),
                "parser exhausted its test action budget without terminating"
            );
            if matches!(self.returned, Some(ParseValue::ExternalDeclaration(_))) {
                let Some(ParseValue::ExternalDeclaration(external)) = self.returned.take() else {
                    unreachable!("the returned value was just checked")
                };
                self.has_external_declaration = true;
                self.external_declaration_count = self
                    .external_declaration_count
                    .checked_add(1)
                    .expect("external declaration count overflows usize");
                self.scopes.clear_retained_bindings();
                self.context.discard_completed_macro_locations(
                    self.cursor.previous.map(|token| token.source_vectors),
                );
                return Some(external);
            }
            debug_assert!(
                self.returned.is_none() || !self.frames.is_empty(),
                "a child value must have a parent frame"
            );

            if self.frames.is_empty() {
                // C99 §6.9p1: `translation-unit` needs at least one
                // `external-declaration`.
                if self.cursor.current().is_none() {
                    if !self.has_external_declaration && !self.reported_empty_translation_unit {
                        self.reported_empty_translation_unit = true;
                        self.report(ParserErrorType::EmptyTranslationUnit, None);
                    }
                    return None;
                }
                // C99 §6.9p1 has no empty external declaration. GNU
                // extension: a `;` between external declarations, such as one
                // an empty macro leaves behind, declares nothing. As in Clang,
                // it keeps the translation unit from being empty.
                if let Some(token) = self.cursor.current()
                    && is_operator(Some(token), OperatorTokenType::Semicolon)
                {
                    self.extension(
                        crate::configuration::Feature::ExtraSemicolons,
                        "extra semicolon outside a function",
                        token,
                    );
                    self.has_external_declaration = true;
                    self.cursor.consume();
                    continue;
                }
                if self.external_declaration_count >= self.limits.external_declarations {
                    return self.resource_failure(
                        ParserResource::ExternalDeclarations,
                        self.limits.external_declarations,
                    );
                }
                self.push_frame(ParseFrame::ExternalDeclaration(
                    ExternalDeclarationFrame::new(
                        self.hard_error_count,
                        self.context.pending_errors.len(),
                    ),
                ));
            }

            if self.frames.len() > self.limits.frame_depth {
                return self.resource_failure(ParserResource::FrameDepth, self.limits.frame_depth);
            }
            // The syntax-node limit was checked after the previous step. Since
            // then only a consumed token, a pushed frame (new frames retain no
            // nodes), or a reduction (which releases retained nodes) occurred.
            debug_assert!(
                self.syntax_nodes
                    .checked_add(self.retained_frame_nodes)
                    .expect("total syntax node count overflows usize")
                    <= self.limits.syntax_nodes,
                "syntax-node limit holds between steps"
            );

            let token = self.cursor.current();
            let returned = self.returned.take();
            // Step the active frame in place. Frames never inspect the control
            // stack, so it is detached while the frame borrows the parser.
            let mut frames = std::mem::replace(&mut self.frames, ArenaVec::new_in(self.arena));
            let frame = frames.last_mut().expect("parser frame stack is nonempty");
            let frame_kind = frame.kind();
            self.active_frame = frame_kind;
            let retained_before = frame.retained_node_count();
            let action = frame.step(self, token, returned);
            let retained_after = frame.retained_node_count();
            self.frames = frames;
            self.retained_frame_nodes = self
                .retained_frame_nodes
                .checked_sub(retained_before)
                .expect("pending syntax count matches the frame stack")
                .checked_add(retained_after)
                .expect("retained syntax node count overflows usize");
            if self.context.source_segment_count() > self.limits.source_segments {
                return self
                    .resource_failure(ParserResource::SourceSegments, self.limits.source_segments);
            }

            #[cfg(test)]
            self.trace.push(FrameTraceEvent {
                frame:  frame_kind.label(),
                action: action.name(),
                token:  token.map(|token| token.kind),
                depth:  self.frames.len(),
            });

            #[cfg(test)]
            assert_eq!(
                self.syntax_nodes,
                self.syntax.node_count(),
                "the running syntax-node total matches the log"
            );
            // A step that crosses a limit keeps the nodes it allocated: the
            // translation-unit arena cannot take memory back while
            // references into it may exist. Parsing stops here, so those
            // nodes are freed with the arena.
            if self
                .syntax_nodes
                .checked_add(self.retained_frame_nodes)
                .expect("total syntax node count overflows usize")
                > self.limits.syntax_nodes
            {
                return self
                    .resource_failure(ParserResource::SyntaxNodes, self.limits.syntax_nodes);
            }

            match action {
                | ParseAction::Consume =>
                    if let Some(token) = token {
                        self.suppress_token_diagnostic(token);
                        self.cursor.consume();
                    } else {
                        self.report(
                            ParserErrorType::ParserFrameConsumedAtEndOfInput(frame_kind),
                            None,
                        );
                    },
                | ParseAction::Push(child) => {
                    self.push_frame(child);
                },
                | ParseAction::Reduce(value) => {
                    let frame = self.pop_frame();
                    frame.reclaim_pooled(&mut self.pools);
                    self.returned = Some(value);
                },
                | ParseAction::Reprocess => {},
                | ParseAction::Continue => {
                    unreachable!("frame dispatch resolves continued transitions")
                },
                | ParseAction::Recover(set) => {
                    debug_assert!(
                        set.target == frame_kind,
                        "a recovery set must target the frame that requested it"
                    );

                    let recovered_source_vectors = {
                        #[cfg(test)]
                        {
                            let depth = self.frames.len();
                            self.recover(set, depth)
                        }
                        #[cfg(not(test))]
                        {
                            self.recover(set)
                        }
                    };
                    self.frames
                        .last_mut()
                        .expect("the recovering frame remains active")
                        .merge_recovered_sources(self.context, recovered_source_vectors);
                },
            }
        }
    }

    /// Advances the diagnostic occurrence cursor even on recovery skips.
    fn suppress_token_diagnostic(&mut self, token: Token) {
        if self.token_diagnostics.is_empty() {
            return;
        }
        match token.kind {
            | TokenType::Keyword(_) => {
                self.suppress_diagnostic_occurrence(token, None);
            },
            | TokenType::Integer(_) | TokenType::Float(_) => {
                for spelling in [
                    "imaginary constant",
                    "long long integer constant",
                    "hexadecimal floating constant",
                    "binary integer constant",
                ]
                .into_iter()
                .chain(super::msvc::CONSTANT_DIAGNOSTICS)
                {
                    self.suppress_diagnostic_occurrence(token, Some(spelling));
                }
            },
            | _ => {},
        }
    }

    fn suppress_diagnostic_occurrence(&mut self, token: Token, spelling: Option<&str>) {
        let spelling = spelling.unwrap_or_else(|| self.context.string_cache.at(token.contents));
        let vectors = self.context.get_source_vectors(token.source_vectors);
        let user_end = self.context.user_source_end(token.source_vectors);
        // A diagnostic about a macro argument can lack the invocation that
        // the token carries; the argument's spelling already places it.
        let occurrence = [user_end, vectors.last().cloned()]
            .into_iter()
            .find_map(|user_end| {
                self.token_diagnostics
                    .get_mut(&DiagnosticLookup {
                        spelling,
                        vectors,
                        user_end,
                    })
                    .and_then(ArenaQueue::pop_front)
            });
        match occurrence {
            | Some(TokenDiagnostic::Extension(suppressed))
                if self.pedantic_suppression != 0
                    || matches!(token.kind, TokenType::Keyword(KeywordTokenType::Extension)) =>
                self.context.suppress_extension(suppressed),
            | Some(TokenDiagnostic::Constant(index)) if self.active_frame == ParseFrameKind::Msvc =>
                self.context.withdraw_pending_error(index),
            | _ => {},
        }
    }

    /// Returns a zero-width location for syntax the input left out: the start
    /// of the current token, or the end of input when no token remains.
    ///
    /// Unlike the upstream position, this does not depend on how far the
    /// preprocessor has read ahead, so every preprocessing strategy places
    /// recovered and missing nodes identically.
    pub(super) fn missing_syntax_source(&mut self) -> SourceVectors {
        if let Some(token) = self.cursor.current()
            && let Some(first) = self
                .context
                .get_source_vectors(token.source_vectors)
                .first()
        {
            let (position, source_file_index) =
                (first.position(self.context), first.source_file_index);
            return self
                .context
                .create_retained_source_vectors(position, source_file_index, 0);
        }
        self.context
            .create_retained_source_vectors(self.position(), self.source_file_index(), 0)
    }

    /// Allocates one node in the translation-unit arena and counts it toward
    /// the node limit.
    pub(super) fn alloc_syntax<T: TreeNode<'tu>>(&mut self, node: T) -> &'tu T {
        let node = &*self.tree.alloc(node);
        self.syntax_nodes = self
            .syntax_nodes
            .checked_add(1)
            .expect("syntax node count overflows usize");
        #[cfg(test)]
        self.syntax.record(node);
        node
    }

    /// Stores the out-of-line part of a node, such as a conditional's three
    /// operands, in the translation-unit arena. It belongs to its node, so
    /// it is not counted as a node of its own.
    pub(super) fn alloc_syntax_part<T: Copy>(&self, part: T) -> &'tu T {
        self.tree.alloc(part)
    }

    /// Copies frame-retained nodes into one length-prefixed list in the
    /// translation-unit arena, counts them, and empties `nodes` for reuse.
    /// An empty list allocates nothing.
    pub(super) fn alloc_syntax_list<T: TreeNode<'tu> + Copy>(
        &mut self,
        nodes: &mut ArenaVec<'_, T>,
    ) -> ArenaList<'tu, T> {
        let list = ArenaList::copy_from_slice(self.tree, nodes);
        nodes.clear();
        self.syntax_nodes = self
            .syntax_nodes
            .checked_add(list.len())
            .expect("syntax node count overflows usize");
        #[cfg(test)]
        for node in list {
            self.syntax.record(node);
        }
        list
    }

    fn push_frame(&mut self, mut frame: ParseFrame<'tu, 'p>) {
        frame.lend_pooled(&mut self.pools);
        self.retained_frame_nodes = self
            .retained_frame_nodes
            .checked_add(frame.retained_node_count())
            .expect("retained syntax node count overflows usize");
        self.frames.push(frame);
    }

    fn pop_frame(&mut self) -> ParseFrame<'tu, 'p> {
        let frame = self.frames.pop().expect("parser frame stack is nonempty");
        self.retained_frame_nodes = self
            .retained_frame_nodes
            .checked_sub(frame.retained_node_count())
            .expect("pending syntax count matches the frame stack");
        frame
    }

    fn resource_failure(
        &mut self,
        resource: ParserResource,
        limit: usize,
    ) -> Option<ExternalDeclaration<'tu>> {
        self.resource_failure_at(resource, limit, None)
    }

    fn resource_failure_at(
        &mut self,
        resource: ParserResource,
        limit: usize,
        token_override: Option<Token>,
    ) -> Option<ExternalDeclaration<'tu>> {
        if self.resource_limit_reported {
            return None;
        }
        self.resource_limit_reported = true;
        self.has_external_declaration = true;
        let token = token_override.or_else(|| self.cursor.current());
        let source_vectors = token.map_or_else(
            || {
                self.context.create_retained_source_vectors(
                    self.position(),
                    self.source_file_index(),
                    0,
                )
            },
            |token| token.source_vectors,
        );
        self.report(
            ParserErrorType::ResourceLimitExceeded { resource, limit },
            token,
        );
        self.cursor.abandon();
        while let Some(frame) = self.frames.pop() {
            frame.reclaim_pooled(&mut self.pools);
        }
        self.retained_frame_nodes = 0;
        self.returned = None;
        self.recovery.abandon();
        self.scopes.restore_depth(0);
        self.scopes.clear_retained_bindings();
        self.label_scopes.exit_all();
        self.switch_scopes.clear();
        self.switch_floor = 0;
        self.pedantic_suppression = 0;
        Some(ExternalDeclaration::Error(source_vectors))
    }

    /// Consumes malformed input until the active synchronization policy says
    /// its owning frame can safely resume.
    ///
    /// C99: continued translation after a required diagnostic is permitted by
    /// §5.1.1.3 paragraph 1 and footnote 8, p. 11; PDF p. 23. C99 does not
    /// specify how an implementation resynchronizes.
    fn recover(
        &mut self,
        set: SynchronizationSet,
        #[cfg(test)] depth: usize,
    ) -> Option<SourceVectors> {
        let mut source_vectors = None;
        let mut consumed_tokens = 0_usize;
        self.recovery.begin(set);

        while let Some(token) = self.cursor.current() {
            let state = self.recovery.active();
            let recovery_set = state.set;
            let at_top_level = state.parentheses == 0 && state.brackets == 0 && state.braces == 0;
            let delimiter_depth = DelimiterDepth {
                parentheses: state.parentheses,
                brackets:    state.brackets,
                braces:      state.braces,
            };
            let colon_matches_conditional =
                matches!(token.kind, TokenType::Operator(OperatorTokenType::Colon))
                    && state
                        .questions
                        .iter()
                        .rev()
                        .any(|question| *question == delimiter_depth);
            let at_unambiguous_owning_delimiter = !colon_matches_conditional
                && recovery_set.kind.stops_before_despite_unbalanced_child(
                    token.kind,
                    state.parentheses,
                    state.brackets,
                    state.braces,
                );
            let stops_at_initial_declaration = matches!(
                (recovery_set.kind, recovery_set.target),
                |(
                    SynchronizationKind::Declaration
                    | SynchronizationKind::BlockDeclaration
                    | SynchronizationKind::OldStyleParameter
                    | SynchronizationKind::Parameter,
                    _,
                )| (
                    SynchronizationKind::StructMember,
                    ParseFrameKind::StructOrUnionSpecifier
                ) | (
                    SynchronizationKind::EnumeratorValue,
                    ParseFrameKind::EnumSpecifier
                )
            );
            let stops_at_declaration_after_malformed_prefix = matches!(
                recovery_set.kind,
                SynchronizationKind::ArrayBound
                    | SynchronizationKind::VariadicParameterList
                    | SynchronizationKind::StructMember
                    | SynchronizationKind::EnumeratorValue
                    | SynchronizationKind::StatementExpression(_)
            );
            let at_next_declaration = (stops_at_initial_declaration
                || consumed_tokens > 0 && stops_at_declaration_after_malformed_prefix)
                && at_top_level
                && self.declaration_starter(token)
                && !(matches!(token.kind, TokenType::Identifier)
                    && matches!(
                        state.last_token,
                        Some(TokenType::Operator(
                            OperatorTokenType::Period | OperatorTokenType::Arrow
                        ))
                    ));
            let at_next_k_and_r_identifier =
                matches!(recovery_set.kind, SynchronizationKind::KAndRParameter)
                    && at_top_level
                    && matches!(token.kind, TokenType::Identifier)
                    && !self.scopes.is_typedef(token.contents);
            let at_next_enumerator =
                matches!(recovery_set.kind, SynchronizationKind::EnumeratorValue)
                    && at_top_level
                    && recovery_set.target == ParseFrameKind::EnumSpecifier
                    && matches!(token.kind, TokenType::Identifier);
            let has_pending_conditional_at_depth = state.questions.contains(&delimiter_depth);
            let at_next_identifier_label =
                (matches!(
                    recovery_set.kind,
                    SynchronizationKind::StatementExpression(ExpressionTerminator::Semicolon)
                ) || matches!(recovery_set.kind, SynchronizationKind::BlockDeclaration)
                    || matches!(recovery_set.kind, SynchronizationKind::Statement)
                        && consumed_tokens > 0)
                    && at_top_level
                    && !has_pending_conditional_at_depth
                    && matches!(token.kind, TokenType::Identifier)
                    && is_operator(self.cursor.following(), OperatorTokenType::Colon);
            let at_statement_body_brace = (matches!(
                recovery_set.kind,
                SynchronizationKind::StatementExpression(
                    ExpressionTerminator::ClosingParenthesis | ExpressionTerminator::Semicolon
                )
            ) || matches!(
                recovery_set.kind,
                SynchronizationKind::BlockDeclaration | SynchronizationKind::ForInitializer
            )) && at_top_level
                && matches!(
                    token.kind,
                    TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
                )
                && (!matches!(
                    state.last_token,
                    Some(TokenType::Operator(OperatorTokenType::ClosingParenthesis))
                ) || !state.last_closed_parenthesis_was_type_name
                    || state.last_closed_parenthesis_was_sizeof_type_name
                        && self.cursor.following().is_some_and(|following| {
                            is_statement_keyword(following.kind)
                                || !matches!(following.kind, TokenType::Identifier)
                                    && self.declaration_starter(following)
                                || matches!(
                                    following.kind,
                                    TokenType::Operator(
                                        OperatorTokenType::Semicolon
                                            | OperatorTokenType::ClosingCurlyBrace
                                    )
                                )
                        }));
            let at_next_statement_keyword = matches!(
                recovery_set.kind,
                SynchronizationKind::Statement | SynchronizationKind::ForInitializer
            ) && consumed_tokens > 0
                && at_top_level
                && is_statement_keyword(token.kind);
            if at_unambiguous_owning_delimiter
                || at_next_declaration
                || at_next_k_and_r_identifier
                || at_next_enumerator
                || at_next_identifier_label
                || at_statement_body_brace
                || at_next_statement_keyword
                || at_top_level
                    && !colon_matches_conditional
                    && recovery_set.kind.stops_before(token.kind)
            {
                break;
            }

            let opens_type_name = matches!(
                token.kind,
                TokenType::Operator(OperatorTokenType::OpeningParenthesis)
            ) && self
                .cursor
                .following()
                .is_some_and(|following| self.declaration_starter(following));
            self.recovery.consume(token.kind, opens_type_name);

            #[cfg(test)]
            self.trace.push(FrameTraceEvent {
                frame: set.target.label(),
                action: "recover-consume",
                token: Some(token.kind),
                depth,
            });
            self.merge_source(&mut source_vectors, token);
            self.suppress_token_diagnostic(token);
            self.cursor.consume();
            consumed_tokens += 1;
        }
        let stopped_token = self.cursor.current();
        let stopped_at = stopped_token.map(|token| token.kind);
        let ranges = source_vectors.map(|discarded| self.context.diagnostic_slice(&[discarded]));
        let related = stopped_token.map(|token| {
            self.context.diagnostic_slice(&[RelatedParserDiagnostic {
                message:        "parsing resumes here",
                source_vectors: token.source_vectors,
            }])
        });
        if let Some(TranslationError::Parsing(error)) = self.context
            .pending_errors
            .iter_mut()
            .rev()
            .find(|error| matches!(error, TranslationError::Parsing(error) if error.recovery.is_none()))
        {
            if let Some(ranges) = ranges {
                error.ranges = ranges;
            }
            if let Some(related) = related {
                error.related = related;
            }
            error.recovery = Some(RecoverySummary {
                owner: set.target,
                discarded: source_vectors,
                discarded_tokens: consumed_tokens,
                stopped_at,
            });
        }
        self.recovery.finish();
        source_vectors
    }

    /// Emits one structured diagnostic and records whether it was a hard error.
    ///
    /// C99: syntax-rule and constraint violations require at least one
    /// diagnostic under §5.1.1.3, p. 11; PDF p. 23: implementations must
    /// “produce at least one diagnostic message”.
    pub(super) fn report(&mut self, error_type: ParserErrorType<'tu>, token: Option<Token>) {
        let warning_group = error_type.warning_group();
        if warning_group == Some(ParserWarningGroup::RepeatedSpecifiers)
            && !self.context.configuration.repeated_specifier_warnings()
        {
            return;
        }
        if error_type.severity() == ErrorSeverity::Error && !error_type.leaves_syntax_intact() {
            self.hard_error_count += 1;
        }
        let found_spelling = token.map(|token| -> &str {
            match token.kind {
                | TokenType::String(StringTokenType::String(contents)) => self
                    .context
                    .literal_spelling_in(self.context.tu_arena(), self.arena, contents, ""),
                | TokenType::String(StringTokenType::WideString(contents)) => self
                    .context
                    .literal_spelling_in(self.context.tu_arena(), self.arena, contents, "L"),
                | _ => self.context.diagnostic_text(
                    self.context
                        .string_cache
                        .at(token.contents)
                        .trim_end_matches('\0'),
                ),
            }
        });
        let source_vectors = match token {
            | Some(token) => token.source_vectors,
            | None => self.missing_syntax_source(),
        };
        // Clang's missing-declarations warning remains suppressible in system
        // headers even when -pedantic-errors promotes it to an error.
        if matches!(error_type, ParserErrorType::MemberDeclaresNothing)
            && self
                .context
                .withholds(ErrorSeverity::Warning, false, source_vectors)
        {
            return;
        }
        let insertion_point = if error_type.expects_terminating_semicolon() {
            self.semicolon_insertion_point(token)
        } else {
            None
        };
        self.context.parser_error(ParserError {
            code: error_type.code(),
            severity: if matches!(error_type, ParserErrorType::MemberDeclaresNothing)
                && self.context.configuration.extension_policy()
                    == crate::configuration::ExtensionPolicy::Deny
            {
                ErrorSeverity::Error
            } else {
                error_type.severity()
            },
            warning_group,
            frame: self.active_frame,
            expected: error_type.expected_syntax(),
            found: token.map(|token| token.kind),
            found_spelling,
            insertion_point,
            error_type,
            source_vectors,
            ranges: &mut [],
            related: &mut [],
            recovery: None,
            consumed_tokens: self.cursor.consumed,
            ordering_location: self
                .context
                .user_source_end(source_vectors)
                .map(|location| (location.source_file_index, location.index)),
        });
    }

    /// Attaches a "missing `;`" suggestion after `source` to the diagnostic
    /// just reported, explaining why the following input was misread.
    pub(super) fn suggest_semicolon_after(&mut self, source: SourceVectors) {
        let Some(last) = self.context.user_source_end(source) else {
            return;
        };
        let column = last.column + last.length;
        let insertion_point = self.context.create_retained_source_vectors(
            SourcePosition {
                index: last.end(),
                line: last.line,
                column,
            },
            last.source_file_index,
            0,
        );
        let related = self.context.diagnostic_slice(&[RelatedParserDiagnostic {
            message:        "not a function, so later declarations were read as its parameters",
            source_vectors: source,
        }]);
        if let Some(TranslationError::Parsing(error)) = self.context.pending_errors.back_mut() {
            error.insertion_point = Some(insertion_point);
            error.related = related;
        }
    }

    /// Returns an empty range just after the previous token when `found`
    /// starts a later line of the same file: the likely place of a missing
    /// `;`.
    fn semicolon_insertion_point(&mut self, found: Option<Token>) -> Option<SourceVectors> {
        let previous = self.cursor.previous?;
        let previous = self.context.user_source_end(previous.source_vectors)?;
        let next = self
            .context
            .get_source_vectors(found?.source_vectors)
            .first()?
            .clone();
        if previous.source_file_index != next.source_file_index || previous.line >= next.line {
            return None;
        }
        let column = previous.column + previous.length;
        Some(self.context.create_retained_source_vectors(
            SourcePosition {
                index: previous.end(),
                line: previous.line,
                column,
            },
            previous.source_file_index,
            0,
        ))
    }

    /// Adds a token's provenance to an optional accumulated source range.
    ///
    /// C99: diagnostics should identify the violation where possible under
    /// §5.1.1.3 and footnote 8, p. 11; PDF p. 23. `SourceVectors` is the
    /// implementation's macro/include provenance mechanism.
    pub(super) fn merge_source(&mut self, existing: &mut Option<SourceVectors>, token: Token) {
        *existing = Some(existing.map_or(token.source_vectors, |source_vectors| {
            self.context
                .merge_vectors(source_vectors, token.source_vectors)
        }));
    }

    /// Reports a syntax feature through the shared mode policy.
    /// C99: §5.1.1.3, p. 11; PDF p. 23. Later ISO syntax is an extension.
    pub(super) fn extension(
        &mut self,
        feature: crate::configuration::Feature,
        spelling: &'static str,
        token: Token,
    ) {
        self.extension_source(feature, spelling, token.source_vectors);
    }

    pub(super) fn extension_source(
        &mut self,
        feature: crate::configuration::Feature,
        spelling: &'static str,
        source: SourceVectors,
    ) {
        if self.pedantic_suppression == 0 {
            self.context.report_extension(feature, spelling, source);
        }
    }

    /// Reports whether the token after any leading GNU `__extension__`
    /// markers starts a declaration, so `__extension__ x` stays an
    /// expression.
    pub(super) fn extension_precedes_declaration(&self) -> bool {
        let mut offset = 0;
        let mut token = self.cursor.current();
        while token
            .is_some_and(|x| matches!(x.kind, TokenType::Keyword(KeywordTokenType::Extension)))
        {
            token = self.cursor.lookahead(offset);
            offset += 1;
        }
        token.is_some_and(|x| self.declaration_starter(x))
    }

    /// Reports unambiguous C99 grammar absent from the grouped feature table.
    /// C99: array declarators §6.7.5 paragraph 1, p. 114; PDF p. 126.
    pub(super) fn c99_syntax_extension(&mut self, spelling: &'static str, token: Token) {
        if self.pedantic_suppression == 0 {
            self.context.report_extension_since(
                spelling,
                crate::configuration::FeatureOrigin::Standard(crate::configuration::CStandard::C99),
                token.source_vectors,
            );
        }
    }

    /// Reports whether a declaration, not a statement, follows the attribute
    /// specifiers (and `__extension__` markers) starting at the current token.
    pub(super) fn attributes_precede_declaration(&self) -> bool {
        let mut offset = 0;
        let mut token = self.cursor.current();
        loop {
            let parenthesized = token.is_some_and(|x| {
                matches!(
                    x.kind,
                    TokenType::Keyword(KeywordTokenType::Attribute | KeywordTokenType::Declspec)
                )
            });
            let mut depth = 0usize;
            let mut opened = false;
            while let Some(current) = token {
                match current.kind {
                    | TokenType::Operator(OperatorTokenType::OpeningParenthesis) if parenthesized =>
                    {
                        depth += 1;
                        opened = true;
                    },
                    | TokenType::Operator(OperatorTokenType::ClosingParenthesis) if parenthesized =>
                        depth = depth.saturating_sub(1),
                    | TokenType::Operator(OperatorTokenType::OpeningSquareBracket)
                        if !parenthesized =>
                    {
                        depth += 1;
                        opened = true;
                    },
                    | TokenType::Operator(OperatorTokenType::ClosingSquareBracket)
                        if !parenthesized =>
                        depth = depth.saturating_sub(1),
                    | TokenType::Operator(
                        OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace,
                    ) => return false,
                    | _ => {},
                }
                token = self.cursor.lookahead(offset);
                offset += 1;
                if opened && depth == 0 {
                    break;
                }
            }
            let Some(mut current) = token else {
                return false;
            };
            while matches!(
                current.kind,
                TokenType::Keyword(KeywordTokenType::Extension)
            ) {
                token = self.cursor.lookahead(offset);
                offset += 1;
                let Some(next) = token else { return false };
                current = next;
            }
            if matches!(
                current.kind,
                TokenType::Keyword(KeywordTokenType::Attribute | KeywordTokenType::Declspec)
            ) || matches!(
                current.kind,
                TokenType::Operator(OperatorTokenType::OpeningSquareBracket)
            ) && self.cursor.lookahead(offset).is_some_and(|x| {
                matches!(
                    x.kind,
                    TokenType::Operator(OperatorTokenType::OpeningSquareBracket)
                )
            }) {
                continue;
            }
            return self.declaration_starter(current);
        }
    }

    /// Reports whether `token`, the current token, begins an attribute
    /// specifier: `__attribute__`, `__declspec`, or `[[`.
    ///
    /// C23 (N3220): attribute-specifier is §6.7.13.2 paragraph 1,
    /// pp. 142-143; PDF pp. 155-156. `__attribute__` and `__declspec` are GNU
    /// and MSVC extensions.
    pub(super) fn attribute_starter(&self, token: Option<Token>) -> bool {
        token.is_some_and(|token| Self::attribute_starter_before(token, self.cursor.following()))
    }

    /// Reports whether `token`, followed by `next`, begins an attribute
    /// specifier. A `[` starts one only when another `[` follows it.
    fn attribute_starter_before(token: Token, next: Option<Token>) -> bool {
        match token.kind {
            | TokenType::Keyword(KeywordTokenType::Attribute | KeywordTokenType::Declspec) => true,
            | TokenType::Operator(OperatorTokenType::OpeningSquareBracket) =>
                next.is_some_and(|next| {
                    matches!(
                        next.kind,
                        TokenType::Operator(OperatorTokenType::OpeningSquareBracket)
                    )
                }),
            | _ => false,
        }
    }

    /// [`Self::attribute_starter_before`] for a token the caller may have
    /// read by lookahead. The token after a `[` is known only when the `[` is
    /// the current token or the one following it; at any later position the
    /// `[` is not taken as an attribute start.
    fn attribute_starter_in_lookahead(&self, token: Token) -> bool {
        if !matches!(
            token.kind,
            TokenType::Operator(OperatorTokenType::OpeningSquareBracket)
        ) {
            return Self::attribute_starter_before(token, None);
        }
        let next = if Some(token) == self.cursor.current() {
            self.cursor.following()
        } else if Some(token) == self.cursor.following() {
            self.cursor.lookahead(1)
        } else {
            None
        };
        Self::attribute_starter_before(token, next)
    }

    /// Reports whether `token` can begin declaration specifiers in the current
    /// typedef environment.
    ///
    /// C99: declaration-specifiers are §6.7, p. 97; PDF p. 109, built from
    /// storage-class specifiers (§6.7.1, p. 98; PDF p. 110), type specifiers
    /// (§6.7.2, p. 99; PDF p. 111), type qualifiers (§6.7.3, p. 108;
    /// PDF p. 120), and `inline` (§6.7.4, p. 112; PDF p. 124); typedef-name
    /// is a type-specifier under §6.7.2, p. 99; PDF p. 111. `_Imaginary` is
    /// accepted here so the specifier frame can diagnose it. Later-standard
    /// specifiers, `_Static_assert`, attribute specifiers, and the GNU and
    /// MSVC specifier keywords also start one; a `[` counts only as described
    /// on [`Self::attribute_starter_in_lookahead`].
    pub(super) fn declaration_starter(&self, token: Token) -> bool {
        match token.kind {
            | TokenType::Keyword(
                KeywordTokenType::Alignas
                | KeywordTokenType::Atomic
                | KeywordTokenType::Noreturn
                | KeywordTokenType::ThreadLocal
                | KeywordTokenType::BitInt
                | KeywordTokenType::Decimal32
                | KeywordTokenType::Decimal64
                | KeywordTokenType::Decimal128
                | KeywordTokenType::Constexpr
                | KeywordTokenType::Int128
                | KeywordTokenType::Float128
                | KeywordTokenType::Int8
                | KeywordTokenType::Int16
                | KeywordTokenType::Int32
                | KeywordTokenType::Int64
                | KeywordTokenType::Ptr32
                | KeywordTokenType::Ptr64
                | KeywordTokenType::Unaligned
                | KeywordTokenType::W64
                | KeywordTokenType::Sptr
                | KeywordTokenType::Uptr
                | KeywordTokenType::AutoType
                | KeywordTokenType::Extension
                | KeywordTokenType::Typeof
                | KeywordTokenType::TypeofUnqual
                | KeywordTokenType::StaticAssert
                | KeywordTokenType::Auto
                | KeywordTokenType::Char
                | KeywordTokenType::Complex
                | KeywordTokenType::Const
                | KeywordTokenType::Double
                | KeywordTokenType::Enum
                | KeywordTokenType::Extern
                | KeywordTokenType::Float
                | KeywordTokenType::Imaginary
                | KeywordTokenType::Inline
                | KeywordTokenType::Forceinline
                | KeywordTokenType::Cdecl
                | KeywordTokenType::Stdcall
                | KeywordTokenType::Fastcall
                | KeywordTokenType::Vectorcall
                | KeywordTokenType::Thiscall
                | KeywordTokenType::Int
                | KeywordTokenType::Long
                | KeywordTokenType::Register
                | KeywordTokenType::Restrict
                | KeywordTokenType::Short
                | KeywordTokenType::Signed
                | KeywordTokenType::Static
                | KeywordTokenType::Struct
                | KeywordTokenType::Typedef
                | KeywordTokenType::Union
                | KeywordTokenType::Unsigned
                | KeywordTokenType::Void
                | KeywordTokenType::Volatile
                | KeywordTokenType::Bool,
            ) => true,
            | TokenType::Identifier => self.scopes.is_typedef(token.contents),
            | _ => self.attribute_starter_in_lookahead(token),
        }
    }

    /// Reports whether `token` can begin a `type-name`: a type specifier or
    /// qualifier, or a visible typedef name.
    ///
    /// C99: a `type-name` begins with a `specifier-qualifier-list` (§6.7.6
    /// paragraph 1, p. 122; PDF p. 134; §6.7.2.1 paragraph 1, p. 101;
    /// PDF p. 113).
    pub(super) fn type_name_starter(&self, token: Token) -> bool {
        match token.kind {
            | TokenType::Keyword(
                KeywordTokenType::Alignas
                | KeywordTokenType::Atomic
                | KeywordTokenType::BitInt
                | KeywordTokenType::Decimal32
                | KeywordTokenType::Decimal64
                | KeywordTokenType::Decimal128
                | KeywordTokenType::Int128
                | KeywordTokenType::Float128
                | KeywordTokenType::Int8
                | KeywordTokenType::Int16
                | KeywordTokenType::Int32
                | KeywordTokenType::Int64
                | KeywordTokenType::Ptr32
                | KeywordTokenType::Ptr64
                | KeywordTokenType::Unaligned
                | KeywordTokenType::W64
                | KeywordTokenType::Sptr
                | KeywordTokenType::Uptr
                | KeywordTokenType::AutoType
                | KeywordTokenType::Typeof
                | KeywordTokenType::TypeofUnqual
                | KeywordTokenType::Char
                | KeywordTokenType::Complex
                | KeywordTokenType::Const
                | KeywordTokenType::Double
                | KeywordTokenType::Enum
                | KeywordTokenType::Float
                | KeywordTokenType::Imaginary
                | KeywordTokenType::Int
                | KeywordTokenType::Long
                | KeywordTokenType::Restrict
                | KeywordTokenType::Short
                | KeywordTokenType::Signed
                | KeywordTokenType::Struct
                | KeywordTokenType::Union
                | KeywordTokenType::Unsigned
                | KeywordTokenType::Void
                | KeywordTokenType::Volatile
                | KeywordTokenType::Bool,
            ) => true,
            | TokenType::Identifier => self.scopes.is_typedef(token.contents),
            | _ => self.attribute_starter_in_lookahead(token),
        }
    }

    pub(super) fn declaration_recovery_starts_here(&mut self, token: Token) -> bool {
        if matches!(token.kind, TokenType::Keyword(KeywordTokenType::Extension)) {
            return self.extension_precedes_declaration();
        }
        self.declaration_starter(token)
            && (!matches!(token.kind, TokenType::Identifier)
                || self.typedef_name_continues_specifiers())
    }

    /// Returns whether the declaration starting at the current token
    /// declares one of `parameters`, judged from its first identifiers that are
    /// neither typedef names nor tags. Recovery uses this to tell an
    /// old-style parameter declaration from an unrelated declaration that
    /// follows a head missing its `;`. When the scan runs out of lookahead it
    /// answers yes, keeping the definition reading.
    pub(super) fn next_declaration_declares_one_of(&mut self, parameters: &[Identifier]) -> bool {
        const LOOKAHEAD: usize = 32;
        let mut after_tag_keyword = false;
        let mut depth = 0_usize;
        for index in 0..LOOKAHEAD {
            let token = if index == 0 {
                self.cursor.current()
            } else {
                self.cursor.lookahead(index - 1)
            };
            let Some(token) = token else {
                return false;
            };
            match token.kind {
                | TokenType::Keyword(
                    KeywordTokenType::Struct | KeywordTokenType::Union | KeywordTokenType::Enum,
                ) => after_tag_keyword = true,
                | TokenType::Identifier => {
                    let tag = std::mem::take(&mut after_tag_keyword);
                    if depth == 0 && !tag && !self.scopes.is_typedef(token.contents) {
                        return parameters
                            .iter()
                            .any(|parameter| parameter.name == token.contents);
                    }
                },
                | TokenType::Operator(OperatorTokenType::OpeningCurlyBrace) => {
                    after_tag_keyword = false;
                    depth += 1;
                },
                | TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) => {
                    depth = depth.saturating_sub(1);
                },
                | TokenType::Operator(
                    OperatorTokenType::Semicolon
                    | OperatorTokenType::Comma
                    | OperatorTokenType::Equals,
                ) if depth == 0 => return false,
                | _ => after_tag_keyword = false,
            }
        }
        true
    }

    /// Resolves the declaration-specifier/declarator ambiguity after a visible
    /// typedef name using buffered lookahead.
    ///
    /// C99: typedef-name is §6.7.7, pp. 123-124; PDF pp. 135-136, and its
    /// declarator ambiguity is constrained by §6.7.5.3 paragraph 11,
    /// p. 119; PDF p. 131.
    pub(super) fn typedef_name_continues_specifiers(&mut self) -> bool {
        let Some(following) = self.cursor.following() else {
            return false;
        };
        matches!(following.kind, TokenType::Identifier)
            || is_operator(Some(following), OperatorTokenType::Asterisk)
            || self.parenthesized_declarator_follows_typedef()
            || self.declaration_starter(following)
    }

    /// Detects the parenthesized-pointer shape that forces a typedef spelling
    /// to remain a specifier rather than become the declarator name.
    ///
    /// C99: parenthesized direct-declarator and pointer are §6.7.5,
    /// p. 114; PDF p. 126; typedef-name is §6.7.7, pp. 123-124;
    /// PDF pp. 135-136.
    fn parenthesized_declarator_follows_typedef(&mut self) -> bool {
        let mut index = 0;
        while is_operator(
            self.cursor.lookahead(index),
            OperatorTokenType::OpeningParenthesis,
        ) {
            index += 1;
        }
        is_operator(self.cursor.lookahead(index), OperatorTokenType::Asterisk)
    }

    pub(super) fn store_expression(
        &mut self,
        kind: ExpressionType<'tu>,
        source_vectors: SourceVectors,
        operator_source_vectors: Option<SourceVectors>,
        recovered: bool,
    ) -> &'tu Expression<'tu> {
        let recovered = recovered || Self::expression_children_recovered(&kind);
        self.alloc_syntax(Expression {
            kind,
            source_vectors,
            operator_source_vectors,
            recovered,
        })
    }

    /// Marks `expression` as recovered. Tree nodes never change, so a copy
    /// that says so takes its place; the copy is not a new node.
    pub(super) fn mark_expression_recovered(
        &mut self,
        expression: &'tu Expression<'tu>,
    ) -> &'tu Expression<'tu> {
        if expression.recovered {
            return expression;
        }
        let marked = &*self.tree.alloc(Expression {
            recovered: true,
            ..*expression
        });
        #[cfg(test)]
        self.syntax.replace_expression(expression, marked);
        marked
    }

    fn expression_children_recovered(kind: &ExpressionType<'tu>) -> bool {
        let expression_recovered = |expression: &Expression<'_>| expression.recovered;
        match kind {
            | ExpressionType::StatementExpression(x) => x.recovered,
            | ExpressionType::Builtin(x) => x.recovered,
            | ExpressionType::OmittedConditional(x) =>
                x.condition_expression.recovered || x.else_expression.recovered,
            | ExpressionType::Parenthesized { expression }
            | ExpressionType::Unary {
                operand_expression: expression,
                ..
            }
            | ExpressionType::AlignofExpr(expression)
            | ExpressionType::SizeofExpr(expression) => expression_recovered(expression),
            | ExpressionType::Conditional(ConditionalExpression {
                condition_expression,
                then_expression,
                else_expression,
            }) =>
                expression_recovered(condition_expression)
                    || expression_recovered(then_expression)
                    || expression_recovered(else_expression),
            | ExpressionType::Binary {
                left_expression,
                right_expression,
                ..
            } => expression_recovered(left_expression) || expression_recovered(right_expression),
            | ExpressionType::Call {
                function_expression,
                arguments,
            } =>
                expression_recovered(function_expression)
                    || arguments
                        .iter()
                        .any(|argument| expression_recovered(argument)),
            | ExpressionType::DirectMember {
                base_expression, ..
            }
            | ExpressionType::IndirectMember {
                base_expression, ..
            } => expression_recovered(base_expression),
            | ExpressionType::CompoundLiteral {
                type_name,
                initializer,
            } => type_name.recovered || initializer.recovered,
            | ExpressionType::SizeofType(type_name) | ExpressionType::AlignofType(type_name) =>
                type_name.recovered,
            | ExpressionType::Countof(operand) => match operand {
                | super::modern::SyntaxOperand::Type(x) => x.recovered,
                | super::modern::SyntaxOperand::Expression(x) => x.recovered,
            },
            | ExpressionType::Generic(selection) =>
                (match selection.controlling {
                    | super::modern::SyntaxOperand::Type(x) => x.recovered,
                    | super::modern::SyntaxOperand::Expression(x) => x.recovered,
                }) || selection
                    .associations
                    .iter()
                    .any(|x| x.expression.recovered || x.type_name.is_some_and(|x| x.recovered)),
            | ExpressionType::Cast {
                target_type,
                operand_expression,
            } => target_type.recovered || expression_recovered(operand_expression),
            | ExpressionType::Error => true,
            | ExpressionType::LabelAddress(_)
            | ExpressionType::Boolean(_)
            | ExpressionType::Nullptr
            | ExpressionType::Identifier(..)
            | ExpressionType::Constant(..)
            | ExpressionType::StringLiteral(..) => false,
        }
    }
}
