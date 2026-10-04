//! The parser driver loop and helpers shared by every grammar frame.

#[cfg(test)]
use std::fmt::Debug;

#[cfg(test)]
use super::machine::FrameTraceEvent;
use super::{
    ParsedTranslationUnit,
    Parser,
    ParserLimits,
    declaration_syntax::{
        Declarator,
        DirectDeclarator,
        TypeSpecifiers,
    },
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
    recovery::{
        DelimiterDepth,
        ExpressionTerminator,
        RecoveryState,
        SynchronizationKind,
        SynchronizationSet,
    },
    scope::ScopeStack,
    statement::is_statement_keyword,
    syntax::{
        DeclarationIndex,
        Expression,
        ExpressionIndex,
        ExpressionType,
        ExternalDeclaration,
        Identifier,
        StatementIndex,
        SyntaxList,
    },
    syntax_store::{
        SyntaxStore,
        SyntaxStoreCheckpoint,
        SyntaxTree,
    },
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
    util::string_cache::StringCacheId,
};

impl Parser {
    /// Preprocesses the whole translation unit, then creates an idle parser
    /// over the result. Every preprocessing diagnostic is pending in
    /// `context` before any parser diagnostic.
    ///
    /// C99: the input is the translation unit produced after phase 7 under
    /// §5.1.1.1-§5.1.1.2, pp. 9-10; PDF pp. 21-22.
    pub(crate) fn new(preprocessor: Preprocessor, context: &mut Context) -> Self {
        Self::new_with_config(preprocessor, context, ParserLimits::default())
    }

    fn new_with_config(
        mut preprocessor: Preprocessor,
        context: &mut Context,
        limits: ParserLimits,
    ) -> Self {
        preprocessor.prepare_for_parsing();
        let mut parser = Self::with_upstream(Upstream::preprocess_all(
            preprocessor,
            context,
            limits.source_segments,
        ));
        parser.limits = limits;
        parser
    }

    fn with_upstream(upstream: Upstream) -> Self {
        Self {
            cursor: TokenCursor::new(upstream),
            frames: Vec::new(),
            pools: FramePools::default(),
            retained_frame_nodes: 0,
            returned: None,
            syntax: SyntaxStore::default(),
            syntax_nodes: 0,
            emitted_roots: Vec::new(),
            scopes: ScopeStack::default(),
            label_scopes: Vec::new(),
            func_name: None,
            switch_scopes: Vec::new(),
            recovery: RecoveryState::default(),
            hard_error_count: 0,
            active_frame: ParseFrameKind::ExternalDeclaration,
            has_external_declaration: false,
            reported_empty_translation_unit: false,
            external_declaration_count: 0,
            limits: ParserLimits::default(),
            resource_limit_reported: false,
            #[cfg(test)]
            trace: Vec::new(),
            #[cfg(test)]
            action_budget: None,
        }
    }

    #[cfg(test)]
    pub(super) fn new_with_limits(
        preprocessor: Preprocessor,
        context: &mut Context,
        limits: ParserLimits,
    ) -> Self {
        Self::new_with_config(preprocessor, context, limits)
    }

    #[cfg(test)]
    pub(super) fn with_action_budget(mut self, action_budget: usize) -> Self {
        self.action_budget = Some(action_budget);
        self
    }

    /// Returns the complete arena-backed syntax store for diagnostic output.
    ///
    /// External declarations contain compact handles, so the CLI prints this
    /// view after the item stream to make those handles manually inspectable
    /// without exposing parser storage as part of the parser interface.
    #[cfg(test)]
    pub(crate) fn syntax_debug(&self) -> impl Debug + '_ {
        &self.syntax
    }

    /// Parses the complete phase-7 input into one source-ordered translation
    /// unit and transfers the validated syntax arenas to the result.
    ///
    /// Roots already observed through
    /// [`TranslationPhase::next_item`](crate::translation_phases::TranslationPhase::next_item)
    /// remain part of the aggregate result so mixing the streaming adapter
    /// with the owning seam cannot silently produce a suffix-only
    /// translation unit.
    pub(crate) fn parse_translation_unit(mut self, context: &mut Context) -> ParsedTranslationUnit {
        let mut roots = std::mem::take(&mut self.emitted_roots);
        while let Some(root) = self.drive(context) {
            roots.push(root);
        }
        let roots = roots.into_boxed_slice();
        let syntax = SyntaxTree::new(self.syntax, &roots);
        ParsedTranslationUnit { roots, syntax }
    }

    /// Runs owned frame actions until one external declaration reduces or EOF
    /// is observed between declarations.
    ///
    /// C99: translation-unit is a nonempty sequence of external-declaration
    /// values under §6.9, p. 140; PDF p. 152.
    pub(super) fn drive(&mut self, context: &mut Context) -> Option<ExternalDeclaration> {
        // A resource failure is terminal: the remaining input is neither
        // fetched nor parsed, so it cannot grow the exhausted storage.
        if self.resource_limit_reported {
            return None;
        }
        if let Some(token) = self.cursor.upstream.preprocessing_limit_token.take() {
            return self.resource_failure_at(
                context,
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
                self.external_declaration_count += 1;
                context.discard_completed_macro_locations(
                    self.cursor.previous.map(|token| token.source_vectors),
                );
                return Some(external);
            }
            debug_assert!(
                self.returned.is_none() || !self.frames.is_empty(),
                "a child value must have a parent frame"
            );

            if self.frames.is_empty() {
                if self.cursor.current(context).is_none() {
                    if !self.has_external_declaration && !self.reported_empty_translation_unit {
                        self.reported_empty_translation_unit = true;
                        self.report(context, ParserErrorType::EmptyTranslationUnit, None);
                    }
                    return None;
                }
                if self.external_declaration_count >= self.limits.external_declarations {
                    return self.resource_failure(
                        context,
                        ParserResource::ExternalDeclarations,
                        self.limits.external_declarations,
                    );
                }
                self.push_frame(ParseFrame::ExternalDeclaration(
                    ExternalDeclarationFrame::new(
                        self.hard_error_count,
                        context.pending_errors.len(),
                    ),
                ));
            }

            if self.frames.len() > self.limits.frame_depth {
                return self.resource_failure(
                    context,
                    ParserResource::FrameDepth,
                    self.limits.frame_depth,
                );
            }
            // The syntax-node limit was checked after the previous step. Since
            // then only a consumed token, a pushed frame (new frames retain no
            // nodes), or a reduction (which releases retained nodes) occurred.
            debug_assert!(
                self.syntax_nodes.saturating_add(self.retained_frame_nodes)
                    <= self.limits.syntax_nodes,
                "syntax-node limit holds between steps"
            );

            let token = self.cursor.current(context);
            let returned = self.returned.take();
            let syntax_checkpoint = self.syntax.checkpoint();
            // Step the active frame in place. Frames never inspect the control
            // stack, so it is detached while the frame borrows the parser.
            let mut frames = std::mem::take(&mut self.frames);
            let frame = frames.last_mut().expect("parser frame stack is nonempty");
            let frame_kind = frame.kind();
            self.active_frame = frame_kind;
            let retained_before = frame.retained_node_count();
            let action = frame.step(self, context, token, returned);
            let retained_after = frame.retained_node_count();
            self.frames = frames;
            self.retained_frame_nodes = self
                .retained_frame_nodes
                .checked_sub(retained_before)
                .expect("pending syntax count matches the frame stack")
                .saturating_add(retained_after);
            if context.source_segment_count() > self.limits.source_segments {
                self.restore_syntax(syntax_checkpoint);
                return self.resource_failure(
                    context,
                    ParserResource::SourceSegments,
                    self.limits.source_segments,
                );
            }

            #[cfg(test)]
            self.trace.push(FrameTraceEvent {
                frame:  frame_kind.label(),
                action: action.name(),
                token:  token.map(|token| token.kind),
                depth:  self.frames.len(),
            });

            debug_assert_eq!(
                self.syntax_nodes,
                self.syntax.node_count(),
                "the running syntax-node total matches the arenas"
            );
            if self.syntax_nodes.saturating_add(self.retained_frame_nodes)
                > self.limits.syntax_nodes
            {
                self.restore_syntax(syntax_checkpoint);
                return self.resource_failure(
                    context,
                    ParserResource::SyntaxNodes,
                    self.limits.syntax_nodes,
                );
            }

            match action {
                | ParseAction::Consume =>
                    if token.is_some() {
                        self.cursor.consume();
                    } else {
                        self.report(
                            context,
                            ParserErrorType::ParserFrameConsumedAtEndOfInput(frame_kind),
                            None,
                        );
                    },
                | ParseAction::Push(child) => {
                    self.push_frame(child);
                },
                | ParseAction::Reduce(value) => {
                    let mut frame = self.pop_frame();
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
                            self.recover(context, set, depth)
                        }
                        #[cfg(not(test))]
                        {
                            self.recover(context, set)
                        }
                    };
                    self.frames
                        .last_mut()
                        .expect("the recovering frame remains active")
                        .merge_recovered_sources(context, recovered_source_vectors);
                },
            }
        }
    }

    /// Returns a zero-width location for syntax the input left out: the start
    /// of the current token, or the end of input when no token remains.
    ///
    /// Unlike the upstream position, this does not depend on how far the
    /// preprocessor has read ahead, so every preprocessing strategy places
    /// recovered and missing nodes identically.
    pub(super) fn missing_syntax_source(&mut self, context: &mut Context) -> SourceVectors {
        if let Some(token) = self.cursor.current(context)
            && let Some(first) = context.get_source_vectors(token.source_vectors).first()
        {
            let (position, source_file_index) = (first.position(context), first.source_file_index);
            return context.create_retained_source_vectors(position, source_file_index, 0);
        }
        context.create_retained_source_vectors(self.position(context), self.source_file_index(), 0)
    }

    /// Adds one node to the syntax arena, counts it toward the node limit,
    /// and returns its raw handle.
    pub(super) fn push_syntax<T: 'static>(&mut self, node: T) -> u32 {
        self.syntax_nodes += 1;
        self.syntax.push(node)
    }

    /// Moves frame-retained nodes into one syntax list and counts them.
    pub(super) fn append_syntax<T: 'static>(&mut self, nodes: &mut Vec<T>) -> SyntaxList<T> {
        self.syntax_nodes += nodes.len();
        self.syntax.append(nodes)
    }

    /// Truncates the arenas to `checkpoint` and resynchronizes the total.
    fn restore_syntax(&mut self, checkpoint: SyntaxStoreCheckpoint) {
        self.syntax.restore(checkpoint);
        self.syntax_nodes = self.syntax.node_count();
    }

    fn push_frame(&mut self, mut frame: ParseFrame) {
        frame.lend_pooled(&mut self.pools);
        self.retained_frame_nodes = self
            .retained_frame_nodes
            .saturating_add(frame.retained_node_count());
        self.frames.push(frame);
    }

    fn pop_frame(&mut self) -> ParseFrame {
        let frame = self.frames.pop().expect("parser frame stack is nonempty");
        self.retained_frame_nodes = self
            .retained_frame_nodes
            .checked_sub(frame.retained_node_count())
            .expect("pending syntax count matches the frame stack");
        frame
    }

    fn resource_failure(
        &mut self,
        context: &mut Context,
        resource: ParserResource,
        limit: usize,
    ) -> Option<ExternalDeclaration> {
        self.resource_failure_at(context, resource, limit, None)
    }

    fn resource_failure_at(
        &mut self,
        context: &mut Context,
        resource: ParserResource,
        limit: usize,
        token_override: Option<Token>,
    ) -> Option<ExternalDeclaration> {
        if self.resource_limit_reported {
            return None;
        }
        self.resource_limit_reported = true;
        self.has_external_declaration = true;
        let token = token_override.or_else(|| self.cursor.current(context));
        let source_vectors = token.map_or_else(
            || {
                context.create_retained_source_vectors(
                    self.position(context),
                    self.source_file_index(),
                    0,
                )
            },
            |token| token.source_vectors,
        );
        self.report(
            context,
            ParserErrorType::ResourceLimitExceeded { resource, limit },
            token,
        );
        self.cursor.abandon();
        self.frames.clear();
        self.retained_frame_nodes = 0;
        self.returned = None;
        self.recovery = RecoveryState::default();
        self.scopes.restore_depth(0);
        self.label_scopes.clear();
        self.switch_scopes.clear();
        Some(ExternalDeclaration::Error(source_vectors))
    }

    /// Consumes malformed input until the active synchronization policy says
    /// its owning frame can safely resume.
    ///
    /// C99: continued translation after a required diagnostic is permitted by
    /// §5.1.1.3 paragraph 1 and footnote 8, p. 11; PDF p. 23. The exact
    /// synchronization algorithm is implementation-defined.
    fn recover(
        &mut self,
        context: &mut Context,
        set: SynchronizationSet,
        #[cfg(test)] depth: usize,
    ) -> Option<SourceVectors> {
        let mut source_vectors = None;
        let mut consumed_tokens = 0_usize;
        self.recovery.begin(set);

        while let Some(token) = self.cursor.current(context) {
            let state = self.recovery.active();
            let recovery_set = state.set;
            let at_top_level = state.parentheses == 0 && state.brackets == 0 && state.braces == 0;
            let delimiter_depth = DelimiterDepth {
                parentheses: state.parentheses,
                brackets:    state.brackets,
                braces:      state.braces,
            };
            let colon_matches_conditional = token.kind
                == TokenType::Operator(OperatorTokenType::Colon)
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
                && !(token.kind == TokenType::Identifier
                    && matches!(
                        state.last_token,
                        Some(TokenType::Operator(
                            OperatorTokenType::Period | OperatorTokenType::Arrow
                        ))
                    ));
            let at_next_k_and_r_identifier =
                matches!(recovery_set.kind, SynchronizationKind::KAndRParameter)
                    && at_top_level
                    && token.kind == TokenType::Identifier
                    && !self.scopes.is_typedef(token.contents);
            let at_next_enumerator =
                matches!(recovery_set.kind, SynchronizationKind::EnumeratorValue)
                    && at_top_level
                    && recovery_set.target == ParseFrameKind::EnumSpecifier
                    && token.kind == TokenType::Identifier;
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
                    && token.kind == TokenType::Identifier
                    && is_operator(self.cursor.following(context), OperatorTokenType::Colon);
            let at_statement_body_brace = (matches!(
                recovery_set.kind,
                SynchronizationKind::StatementExpression(
                    ExpressionTerminator::ClosingParenthesis | ExpressionTerminator::Semicolon
                )
            ) || matches!(
                recovery_set.kind,
                SynchronizationKind::BlockDeclaration | SynchronizationKind::ForInitializer
            )) && at_top_level
                && token.kind == TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
                && (state.last_token
                    != Some(TokenType::Operator(OperatorTokenType::ClosingParenthesis))
                    || !state.last_closed_parenthesis_was_type_name
                    || state.last_closed_parenthesis_was_sizeof_type_name
                        && self.cursor.following(context).is_some_and(|following| {
                            is_statement_keyword(following.kind)
                                || following.kind != TokenType::Identifier
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

            let opens_type_name = token.kind
                == TokenType::Operator(OperatorTokenType::OpeningParenthesis)
                && self
                    .cursor
                    .following(context)
                    .is_some_and(|following| self.declaration_starter(following));
            self.recovery.consume(token.kind, opens_type_name);

            #[cfg(test)]
            self.trace.push(FrameTraceEvent {
                frame: set.target.label(),
                action: "recover-consume",
                token: Some(token.kind),
                depth,
            });
            self.merge_source(context, &mut source_vectors, token);
            self.cursor.consume();
            consumed_tokens += 1;
        }
        let stopped_token = self.cursor.current(context);
        let stopped_at = stopped_token.map(|token| token.kind);
        if let Some(TranslationError::Parsing(error)) = context
            .pending_errors
            .iter_mut()
            .rev()
            .find(|error| matches!(error, TranslationError::Parsing(error) if error.recovery.is_none()))
        {
            if let Some(discarded) = source_vectors {
                error.ranges = vec![discarded].into_boxed_slice();
            }
            if let Some(stopped_token) = stopped_token {
                error.related = vec![RelatedParserDiagnostic {
                    message: "parsing resumes here",
                    source_vectors: stopped_token.source_vectors,
                }]
                .into_boxed_slice();
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
    pub(super) fn report(
        &mut self,
        context: &mut Context,
        error_type: ParserErrorType,
        token: Option<Token>,
    ) {
        let warning_group = error_type.warning_group();
        if warning_group == Some(ParserWarningGroup::RepeatedSpecifiers)
            && !context.configuration.repeated_specifier_warnings()
        {
            return;
        }
        if error_type.severity() == ErrorSeverity::Error && !error_type.leaves_syntax_intact() {
            self.hard_error_count += 1;
        }
        let found_spelling = token.map(|token| -> Box<str> {
            match token.kind {
                | TokenType::String(StringTokenType::String(contents)) =>
                    context.literal_spelling(contents, false).into(),
                | TokenType::String(StringTokenType::WideString(contents)) =>
                    context.literal_spelling(contents, true).into(),
                | _ => context
                    .string_cache
                    .at(token.contents)
                    .trim_end_matches('\0')
                    .into(),
            }
        });
        let source_vectors = match token {
            | Some(token) => token.source_vectors,
            | None => self.missing_syntax_source(context),
        };
        let insertion_point = if error_type.expects_terminating_semicolon() {
            self.semicolon_insertion_point(context, token)
        } else {
            None
        };
        context.parser_error(ParserError {
            code: error_type.code(),
            severity: error_type.severity(),
            warning_group,
            frame: self.active_frame,
            expected: error_type.expected_syntax(),
            found: token.map(|token| token.kind),
            found_spelling,
            insertion_point,
            error_type,
            source_vectors,
            ranges: Box::new([]),
            related: Box::new([]),
            recovery: None,
            consumed_tokens: self.cursor.consumed,
            ordering_location: context
                .user_source_end(source_vectors)
                .map(|location| (location.source_file_index, location.index)),
        });
    }

    /// Attaches a "missing `;`" suggestion after `source` to the diagnostic
    /// just reported, explaining why the following input was misread.
    pub(super) fn suggest_semicolon_after(&self, context: &mut Context, source: SourceVectors) {
        _ = self;
        let Some(last) = context.user_source_end(source) else {
            return;
        };
        let column = last.column + last.length;
        let insertion_point = context.create_retained_source_vectors(
            SourcePosition {
                index: last.end(),
                line: last.line,
                column,
            },
            last.source_file_index,
            0,
        );
        if let Some(TranslationError::Parsing(error)) = context.pending_errors.back_mut() {
            error.insertion_point = Some(insertion_point);
            error.related = vec![RelatedParserDiagnostic {
                message:        "not a function, so later declarations were read as its parameters",
                source_vectors: source,
            }]
            .into_boxed_slice();
        }
    }

    /// Returns an empty range just after the previous token when `found`
    /// starts a later line of the same file: the likely place of a missing
    /// `;`.
    fn semicolon_insertion_point(
        &self,
        context: &mut Context,
        found: Option<Token>,
    ) -> Option<SourceVectors> {
        let previous = self.cursor.previous?;
        let previous = context.user_source_end(previous.source_vectors)?;
        let next = context
            .get_source_vectors(found?.source_vectors)
            .first()?
            .clone();
        if previous.source_file_index != next.source_file_index || previous.line >= next.line {
            return None;
        }
        let column = previous.column + previous.length;
        Some(context.create_retained_source_vectors(
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
    #[expect(
        clippy::unused_self,
        reason = "Source accumulation is a parser-machine operation used by every frame."
    )]
    pub(super) fn merge_source(
        &self,
        context: &mut Context,
        existing: &mut Option<SourceVectors>,
        token: Token,
    ) {
        *existing = Some(existing.map_or(token.source_vectors, |source_vectors| {
            context.merge_vectors(source_vectors, token.source_vectors)
        }));
    }

    /// Reports whether `token` can begin declaration specifiers in the current
    /// typedef environment.
    ///
    /// C99: declaration-specifiers are §6.7, p. 97; PDF p. 109; typedef-name
    /// is a type-specifier under §6.7.2, p. 99; PDF p. 111.
    pub(super) fn declaration_starter(&self, token: Token) -> bool {
        match token.kind {
            | TokenType::Keyword(
                KeywordTokenType::Auto
                | KeywordTokenType::Char
                | KeywordTokenType::Complex
                | KeywordTokenType::Const
                | KeywordTokenType::Double
                | KeywordTokenType::Enum
                | KeywordTokenType::Extern
                | KeywordTokenType::Float
                | KeywordTokenType::Imaginary
                | KeywordTokenType::Inline
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
            | _ => false,
        }
    }

    pub(super) fn type_name_starter(&self, token: Token) -> bool {
        match token.kind {
            | TokenType::Keyword(
                KeywordTokenType::Char
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
            | _ => false,
        }
    }

    pub(super) fn declaration_recovery_starts_here(
        &mut self,
        context: &mut Context,
        token: Token,
    ) -> bool {
        self.declaration_starter(token)
            && (token.kind != TokenType::Identifier
                || self.typedef_name_continues_specifiers(context))
    }

    /// Resolves the declaration-specifier/declarator ambiguity after a visible
    /// typedef name using buffered lookahead.
    ///
    /// C99: typedef-name is §6.7.7, pp. 123-124; PDF pp. 135-136, and its
    /// declarator ambiguity is constrained by §6.7.5.3 paragraph 11,
    /// p. 119; PDF p. 131.
    /// Returns whether the declaration starting at the current token
    /// declares one of `names`, judged from its first identifiers that are
    /// neither typedef names nor tags. Recovery uses this to tell an
    /// old-style parameter declaration from an unrelated declaration that
    /// follows a head missing its `;`. When the scan runs out of lookahead it
    /// answers yes, keeping the definition reading.
    pub(super) fn next_declaration_declares_one_of(
        &mut self,
        context: &mut Context,
        names: &[StringCacheId],
    ) -> bool {
        const LOOKAHEAD: usize = 32;
        let mut after_tag_keyword = false;
        let mut depth = 0_usize;
        for index in 0..LOOKAHEAD {
            let token = if index == 0 {
                self.cursor.current(context)
            } else {
                self.cursor.lookahead(context, index - 1)
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
                        return names.contains(&token.contents);
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

    pub(super) fn typedef_name_continues_specifiers(&mut self, context: &mut Context) -> bool {
        let Some(following) = self.cursor.following(context) else {
            return false;
        };
        following.kind == TokenType::Identifier
            || is_operator(Some(following), OperatorTokenType::Asterisk)
            || self.parenthesized_declarator_follows_typedef(context)
            || self.declaration_starter(following)
    }

    /// Detects the parenthesized-pointer shape that forces a typedef spelling
    /// to remain a specifier rather than become the declarator name.
    ///
    /// C99: parenthesized direct-declarator and pointer are §6.7.5,
    /// p. 114; PDF p. 126; typedef-name is §6.7.7, pp. 123-124;
    /// PDF pp. 135-136.
    fn parenthesized_declarator_follows_typedef(&mut self, context: &mut Context) -> bool {
        let mut index = 0;
        while is_operator(
            self.cursor.lookahead(context, index),
            OperatorTokenType::OpeningParenthesis,
        ) {
            index += 1;
        }
        is_operator(
            self.cursor.lookahead(context, index),
            OperatorTokenType::Asterisk,
        )
    }

    /// Finds the identifier declared by nested parenthesized direct
    /// declarators.
    ///
    /// C99: declarator binding is specified by §6.7.5 paragraph 4,
    /// p. 114; PDF p. 126.
    pub(super) fn declarator_identifier(&self, declarator: Declarator) -> Option<Identifier> {
        let mut declarator = declarator;
        loop {
            let mut nested = None;
            for direct in &self.syntax[declarator.kind] {
                match *direct {
                    | DirectDeclarator::Identifier(identifier) => return Some(identifier),
                    | DirectDeclarator::Parenthesized(index) =>
                        nested = Some(self.syntax[index].declarator),
                    | _ => {},
                }
            }
            declarator = nested?;
        }
    }

    pub(super) fn declaration_head_declarator(
        &self,
        declaration: DeclarationIndex,
    ) -> Option<Declarator> {
        let [init] = &self.syntax[self.syntax[declaration].init_declarators] else {
            return None;
        };
        init.initializer.is_none().then_some(init.declarator)
    }

    pub(super) fn declaration_is_definition_head(&self, declaration: DeclarationIndex) -> bool {
        self.syntax[declaration].is_function_definition_head
            && self.declaration_head_declarator(declaration).is_some()
    }

    pub(super) fn declaration_is_meaningful(&self, declaration: DeclarationIndex) -> bool {
        let declaration = &self.syntax[declaration];
        let specifiers = declaration.declaration_specifiers;
        declaration.init_declarators.length() > 0
            || specifiers.storage_class.is_some()
            || !specifiers.type_qualifiers.is_empty()
            || specifiers.type_specifiers != TypeSpecifiers::Empty
            || specifiers.function_specifiers.is_inline
    }

    pub(super) fn function_suffix(&self, mut declarator: Declarator) -> Option<DirectDeclarator> {
        let mut suffix = None;
        loop {
            let direct = &self.syntax[declarator.kind];
            if let Some(candidate) = direct.get(1).copied()
                && matches!(
                    candidate,
                    DirectDeclarator::Function { .. } | DirectDeclarator::KAndRStyleFunction { .. }
                )
            {
                suffix = Some(candidate);
            }
            let Some(DirectDeclarator::Parenthesized(index)) = direct.first() else {
                return suffix;
            };
            declarator = self.syntax[*index].declarator;
        }
    }

    pub(super) fn collect_type_specifier_bindings(
        &self,
        type_specifiers: TypeSpecifiers,
        names: &mut Vec<StringCacheId>,
    ) {
        let mut pending = vec![type_specifiers];
        while let Some(type_specifiers) = pending.pop() {
            match type_specifiers {
                | TypeSpecifiers::Enum(index) => {
                    if let Some(enumeration_list) = self.syntax[index].enumeration_list {
                        names.extend(
                            self.syntax[enumeration_list]
                                .iter()
                                .map(|enumerator| enumerator.name.name),
                        );
                    }
                },
                | TypeSpecifiers::StructOrUnion(index) => {
                    if let Some(declarations) = self.syntax[index].struct_declaration_list {
                        pending.extend(
                            self.syntax[declarations]
                                .iter()
                                .map(|declaration| declaration.type_specifiers),
                        );
                    }
                },
                | _ => {},
            }
        }
    }

    pub(super) fn statement_source(&self, index: StatementIndex) -> SourceVectors {
        self.syntax[index].source_vectors
    }

    pub(super) fn store_expression(
        &mut self,
        kind: ExpressionType,
        source_vectors: SourceVectors,
        operator_source_vectors: Option<SourceVectors>,
        recovered: bool,
    ) -> ExpressionIndex {
        let recovered = recovered || self.expression_children_recovered(&kind);
        ExpressionIndex(self.push_syntax(Expression {
            kind,
            source_vectors,
            operator_source_vectors,
            recovered,
        }))
    }

    fn expression_children_recovered(&self, kind: &ExpressionType) -> bool {
        let expression_recovered = |index: ExpressionIndex| self.syntax[index].recovered;
        match kind {
            | ExpressionType::Parenthesized { expression }
            | ExpressionType::Unary {
                operand_expression: expression,
                ..
            }
            | ExpressionType::SizeofExpr(expression) => expression_recovered(*expression),
            | ExpressionType::Conditional {
                condition_expression,
                then_expression,
                else_expression,
            } =>
                expression_recovered(*condition_expression)
                    || expression_recovered(*then_expression)
                    || expression_recovered(*else_expression),
            | ExpressionType::Binary {
                left_expression,
                right_expression,
                ..
            } => expression_recovered(*left_expression) || expression_recovered(*right_expression),
            | ExpressionType::Call {
                function_expression,
                arguments,
            } =>
                expression_recovered(*function_expression)
                    || self.syntax[*arguments]
                        .iter()
                        .copied()
                        .any(expression_recovered),
            | ExpressionType::DirectMember {
                base_expression, ..
            }
            | ExpressionType::IndirectMember {
                base_expression, ..
            } => expression_recovered(*base_expression),
            | ExpressionType::CompoundLiteral {
                type_name,
                initializer,
            } => self.syntax[*type_name].recovered || self.syntax[*initializer].recovered,
            | ExpressionType::SizeofType(type_name) => self.syntax[*type_name].recovered,
            | ExpressionType::Cast {
                target_type,
                operand_expression,
            } => self.syntax[*target_type].recovered || expression_recovered(*operand_expression),
            | ExpressionType::Error => true,
            | ExpressionType::Identifier(..)
            | ExpressionType::Constant(..)
            | ExpressionType::StringLiteral(..) => false,
        }
    }
}
