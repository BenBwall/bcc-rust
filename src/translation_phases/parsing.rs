//! The syntax parser builds an immutable tree from a preprocessed token array.
//! [`Parser::drive`] runs one explicit stack of grammar frames. Each frame owns
//! its delimiters, resumes after its child returns, and asks the loop to
//! consume input, push a child, reduce a value, or recover. Typedef
//! classification follows the current scope. Recovery keeps useful syntax and
//! its source provenance so semantic analysis can inspect repaired declarations
//! and function definitions. Type checking and the remaining language
//! constraints belong to that later phase.
//!
//! For `typedef int T; T f(T x) { return x + 1; }`, the loop first pushes an
//! external-declaration frame. Its declaration children recognize the typedef
//! and publish `T` before reducing the first root. The next external frame
//! recognizes `T` as a type, parses the function head, and pushes a
//! function-definition frame. The compound and statement frames then push an
//! expression frame for `x + 1`. Child values return up the same stack until
//! the second root reduces.
//!
//! Read these items first:
//!
//! 1. [`Parser::parse_translation_unit`], [`Parser::next_item`], then
//!    [`Parser::drive`] for the caller seams and action loop; [`Parser`] holds
//!    the state they run on.
//! 2. [`ParseFrame`], [`ParseAction`], [`ParseValue`], and [`ParseFrame::step`]
//!    in `parsing/machine.rs` for frame dispatch and typed child returns.
//! 3. [`frames::external_declaration::ExternalDeclarationFrame::step`], then
//!    [`frames::declaration::DeclarationFrame::step`] for the
//!    declaration/function split.
//! 4. [`frames::expression::ExpressionFrame::step`] for the Double-E reducer,
//!    and [`Parser::recover`] for synchronization after malformed input.
//! 5. [`ParsedTranslationUnit`] and [`ParsedTranslationUnit::inspect`] for the
//!    retained result and its iterative inspection view.
//!
//! Files are grouped by role under `parsing/`:
//!
//! - Machine core: `machine.rs` dispatches frames; `frame_pool.rs` recycles
//!   their storage; `token_cursor.rs` owns token access; `scope.rs` classifies
//!   names; `recovery.rs` scans to production-specific boundaries.
//! - Construction and shared operations: `construction.rs` creates parsers;
//!   `limits.rs` handles resource exhaustion; `allocation.rs` allocates syntax;
//!   `lookahead.rs` resolves declaration/type ambiguity; `errors.rs` reports
//!   diagnostics; `token_diagnostics.rs` tracks pending diagnostic occurrences.
//! - Grammar frames: `frames.rs` groups `frames/external_declaration.rs`,
//!   `declaration.rs`, `declaration_specifiers.rs`, `declarator.rs`,
//!   `parameter_list.rs`, `struct_or_union.rs`, `enum_specifier.rs`,
//!   `function_definition.rs`, `compound_statement.rs`, `statement.rs`,
//!   `expression.rs`, `expression_operators.rs`, `type_name.rs`, and
//!   `initializer.rs`. `frames/expression/lookahead.rs` scans expression
//!   recovery boundaries.
//! - Extensions: `extensions.rs` applies diagnostic policy and groups
//!   `extensions/modern.rs`, `gnu.rs`, and `msvc.rs`, whose children use the
//!   same machine as C99 grammar frames.
//! - Retained syntax and views: `syntax.rs` and `declaration_syntax.rs` define
//!   nodes; `translation_unit.rs` defines the input and result boundaries;
//!   `inspection.rs` and `inspection/labels.rs` render the tree;
//!   `syntax_log.rs` records allocations in test builds. `tests.rs` groups the
//!   suites in `tests/`.
//!
//! References use WG14/N1256, ISO/IEC 9899:TC3 (C99 with Technical Corrigenda
//! 1-3).
//!
//! C99: translation phase 7, §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22.
//!
//! C99: phrase-structure grammar, §6.5-§6.9, pp. 67-144; PDF pp. 79-156;
//! summarized in §A.2, pp. 409-416; PDF pp. 421-428.
//!
//! C99: translation-unit and external-declaration, §6.9 paragraph 1, p. 140;
//! PDF p. 152.
//!
//! C99: diagnostics, §5.1.1.3, p. 11; PDF p. 23.

// Machine core
mod frame_pool;
mod machine;
mod recovery;
mod scope;
mod token_cursor;

// Construction and shared operations
mod allocation;
mod construction;
mod errors;
mod limits;
mod lookahead;
mod token_diagnostics;

// Grammar and extension frames
mod extensions;
mod frames;

// Retained syntax and inspection
pub(crate) mod declaration_syntax;
mod inspection;
pub(crate) mod syntax;
mod syntax_log;
mod translation_unit;

#[cfg(test)]
pub(crate) use declaration_syntax::DirectDeclarator;
pub(crate) use declaration_syntax::TypeSpecifiers;
pub(crate) use errors::ParserError;
use errors::{
    ParserErrorType,
    ParserResource,
};
use extensions::{
    gnu,
    modern,
    msvc,
};
pub(crate) use gnu::{
    Builtin,
    OffsetMember,
};
pub(crate) use inspection::InspectionOptions;
use limits::ParserLimits;
#[cfg(test)]
use machine::FrameTrace;
#[cfg(test)]
use machine::FrameTraceEvent;
use machine::{
    ParseAction,
    ParseFrame,
    ParseFrameKind,
    ParseValue,
};
pub(crate) use modern::{
    AttributeSpecifier,
    ExtendedType,
    GenericSelection,
    SpecifierExtension,
    SpecifierExtensionKind,
    StaticAssertion,
    SyntaxOperand,
};
use recovery::RecoveryState;
use scope::{
    LabelScopes,
    ScopeStack,
    SwitchScope,
};
pub(crate) use syntax::ExternalDeclaration;
#[cfg(test)]
use syntax_log::SyntaxLog;
use token_cursor::TokenCursor;
pub(crate) use translation_unit::{
    ParsedTranslationUnit,
    PreprocessedTranslationUnit,
};

use self::frames::{
    expression_operators::is_operator,
    external_declaration::ExternalDeclarationFrame,
};
use crate::{
    translation_phases::{
        Context,
        preprocessing,
        preprocessing::OperatorTokenType,
    },
    util::{
        bump::{
            ArenaVec,
            Bump,
        },
        region_vec::RegionVec,
        string_cache::StringCacheId,
    },
};

impl<'tu> Parser<'_, 'tu, '_> {
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

    /// Streaming adapter: drives the machine until one external declaration
    /// reduces, and returns it.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Tests stream roots; the pipeline parses whole translation units."
        )
    )]
    pub(crate) fn next_item(&mut self) -> Option<ExternalDeclaration<'tu>> {
        let root = self.drive()?;
        self.emitted_roots.push(root);
        Some(root)
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
}

/// Owns parser input, control frames, the syntax node count, scopes, and
/// diagnostics.
///
/// Calling [`Parser::next_item`] drives the machine until one
/// external declaration reduces or the preprocessed token stream ends.
///
/// C99: translation units and external declarations are specified by §6.9,
/// p. 140; PDF p. 152: a translation unit “consists of a sequence of external
/// declarations.” The diagnostic obligation is §5.1.1.3, p. 11; PDF p. 23.
pub(crate) struct Parser<'c, 'tu, 'p> {
    /// The translation context, borrowed for the whole parse.
    pub(super) context: &'c mut Context<'tu>,
    /// The parse arena, which holds the parser's working memory until
    /// parsing ends.
    arena: &'p Bump,
    /// The translation-unit arena, which holds the syntax tree until the
    /// translation unit ends.
    tree: &'tu Bump,
    /// Buffered parser-facing token stream.
    cursor: TokenCursor,
    /// Grammar control stack in the parse arena; the final element is
    /// active.
    frames: ArenaVec<'p, ParseFrame<'tu, 'p>>,
    /// Spare storage lent to pushed frames and reclaimed when they pop.
    pools: frame_pool::FramePools<'tu, 'p>,
    /// Syntax nodes retained by the pending frames, updated on push/pop.
    retained_frame_nodes: usize,
    /// Completed child value waiting for its parent frame.
    returned: Option<ParseValue<'tu>>,
    /// Every syntax node this parser allocated, by kind, for tests.
    #[cfg(test)]
    syntax: SyntaxLog<'tu>,
    /// Running total of the syntax nodes allocated in the translation-unit
    /// arena, maintained by [`Self::alloc_syntax`] and
    /// [`Self::alloc_syntax_list`].
    syntax_nodes: usize,
    /// Roots parsed so far. They are output rather than working memory, so
    /// they grow in place in their own region instead of the parse arena.
    /// The finished vector becomes the parsed unit's, without a copy.
    emitted_roots: RegionVec<ExternalDeclaration<'tu>>,
    /// Parser-visible ordinary-name classification used for typedef ambiguity.
    ///
    /// C99: scopes are §6.2.1, pp. 29-30; PDF pp. 41-42; typedef-name is
    /// §6.7.7 paragraph 1, p. 123; PDF p. 135.
    scopes: ScopeStack<'p>,
    /// Function-local label namespaces, independent of ordinary identifiers.
    ///
    /// C99: labels have function scope (§6.2.1 paragraph 3, p. 29; PDF p. 41)
    /// and their own name space (§6.2.3 paragraph 1, p. 31; PDF p. 43).
    label_scopes: LabelScopes<'p>,
    /// Interned `__func__`, predeclared in every function body.
    ///
    /// C99: §6.4.2.2 paragraph 1, p. 52; PDF p. 64.
    func_name: Option<StringCacheId>,
    /// Active switch contexts used to associate `case` and `default` labels.
    ///
    /// C99: §6.8.4.2 paragraph 3, p. 134; PDF p. 146.
    switch_scopes: ArenaVec<'p, SwitchScope>,
    /// Scan storage for the nested specifiers whose enumeration constants a
    /// function definition's parameters declare, reused by every definition.
    binding_scan: ArenaVec<'p, TypeSpecifiers<'tu>>,
    /// Delimiter depth and ownership while a synchronization scan is active.
    recovery: RecoveryState<'p>,
    /// Number of hard parser diagnostics emitted so far.
    hard_error_count: usize,
    /// Open `__extension__` scopes; while any is open, extension diagnostics
    /// for the tokens read are suppressed.
    pedantic_suppression: usize,
    /// Token diagnostics indexed once by spelling, provenance and invocation.
    token_diagnostics: token_diagnostics::TokenDiagnostics<'tu, 'p>,
    /// The number of `switch_scopes` entries that belong to enclosing
    /// function bodies, so a nested function's `case` labels never attach
    /// to an outer `switch`.
    switch_floor: usize,
    /// Frame currently executing, captured into every parser diagnostic.
    active_frame: ParseFrameKind,
    /// Whether at least one external declaration has reduced successfully or
    /// through recovery.
    has_external_declaration: bool,
    /// Prevents repeated end-of-stream polling from diagnosing an empty
    /// translation unit more than once.
    reported_empty_translation_unit: bool,
    external_declaration_count: usize,
    limits: ParserLimits,
    resource_limit_reported: bool,
    #[cfg(test)]
    /// Driver actions retained only for machine and recovery regressions.
    trace: FrameTrace,
    #[cfg(test)]
    /// Optional hard stop used by malformed-input tests to turn nonprogress
    /// into a deterministic failure instead of an external test timeout.
    action_budget: Option<usize>,
}

// Tests
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod tests;
