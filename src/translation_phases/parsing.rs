//! Non-recursive C language parser and its arena-backed syntax model.
//!
//! This is the syntax-analysis half of translation phase 7 (§5.1.1.2
//! paragraph 1, p. 10; PDF p. 22): the converted tokens of one translation
//! unit are parsed against the phrase-structure grammar of §6.5-§6.9,
//! pp. 67-144; PDF pp. 79-156, summarized in §A.2, pp. 409-416;
//! PDF pp. 421-428. Semantic analysis, the other half of phase 7, is not
//! implemented; constraints are checked here only where the grammar or a
//! frame needs them.
//!
//! [`Parser`] is a stack machine: [`ParseFrame`] values own resumable grammar
//! productions, return typed [`ParseValue`] children, and ask the driver to
//! consume, push, reduce, reprocess, or recover through
//! [`ParseAction`](machine::ParseAction). Each
//! delimiter belongs to one frame. Child frames begin on unconsumed lookahead,
//! and malformed input is synchronized by production-specific sets.
//!
//! Hard syntax diagnostics do not discard useful syntax. If a declaration can
//! be repaired, the parser yields [`ExternalDeclaration::RecoveredDeclaration`]
//! with its syntax so later semantic analysis can continue. The distinct
//! status prevents repaired syntax from being mistaken for fully valid input.
//!
//! Phase 05 closes declarations, function definitions, compound blocks, every
//! C99 statement family, expressions, type names, initializers, and the scope
//! transitions needed for typedef-sensitive grammar decisions into a complete
//! translation-unit interface. All productions use frames held in the parse
//! arena and retain recovered syntax at their owning grammar boundaries.
//!
//! Standard references in this module cite WG14/N1256, ISO/IEC 9899:TC3
//! (C99 with Technical Corrigenda 1, 2, and 3). Each reference gives the
//! normative clause, the standard's printed page, and the one-based page in
//! the repository's `standards/c99-n1256.pdf`.

mod compound_statement;
mod declaration;
mod declaration_specifiers;
mod declaration_syntax;
mod declarator;
mod driver;
mod enum_specifier;
mod errors;
mod expression;
mod expression_operators;
mod external_declaration;
mod frame_pool;
mod function_definition;
mod gnu;
mod initializer;
mod inspection;
mod machine;
mod modern;
mod msvc;
mod parameter_list;
mod recovery;
mod scope;
mod statement;
mod struct_or_union;
mod syntax;
mod syntax_log;
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod tests;
mod token_cursor;
mod type_name;

use std::fmt::Debug;

#[cfg(test)]
pub(crate) use declaration_syntax::DirectDeclarator;
pub(crate) use declaration_syntax::TypeSpecifiers;
pub(crate) use errors::ParserError;
pub(crate) use inspection::InspectionOptions;
#[cfg(test)]
use machine::FrameTraceEvent;
use machine::{
    ParseFrame,
    ParseFrameKind,
    ParseValue,
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

use crate::{
    translation_phases::{
        Context,
        GetPosition,
        GetSourceFileIndex,
        SourcePosition,
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

/// Driver actions, recorded only by test builds.
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "A test-only trace, compiled only under `cfg(test)`."
)]
type FrameTrace = Vec<FrameTraceEvent>;

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
    token_diagnostics: driver::TokenDiagnostics<'tu, 'p>,
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

/// Fully preprocessed parser input. The preprocessor can be dropped before
/// parser working memory is created.
///
/// C99: the output of translation phases 1-6 for one translation unit
/// (§5.1.1.1, p. 9; PDF p. 21; §5.1.1.2 paragraph 1, pp. 9-10;
/// PDF pp. 21-22).
pub(crate) struct PreprocessedTranslationUnit {
    upstream: token_cursor::Upstream,
}

/// Catchable ceilings on parser resources.
///
/// C99: §5.2.4.1, pp. 20-21; PDF pp. 32-33 sets only minimums, and its
/// footnote 13, p. 20; PDF p. 32 asks implementations to avoid fixed
/// translation limits. The defaults are therefore representation bounds,
/// not grammar limits.
#[derive(Debug, Clone, Copy)]
struct ParserLimits {
    external_declarations: usize,
    syntax_nodes:          usize,
    frame_depth:           usize,
    source_segments:       usize,
}

impl Default for ParserLimits {
    fn default() -> Self {
        Self {
            // These counters and their backing collections use `usize`.
            // There is no smaller grammar or representation limit.
            external_declarations: usize::MAX,
            syntax_nodes:          usize::MAX,
            // Scope bindings store their nesting depth in a `u32`.
            frame_depth:           u32::MAX as usize,
            // Each source arena checks its own `u32` index space. Their
            // combined count has no smaller representation limit than usize.
            source_segments:       usize::MAX,
        }
    }
}

/// One completely parsed translation unit: its source-ordered roots, which
/// borrow the syntax tree from the translation-unit arena.
///
/// The roots themselves stay in the region the parser collected them in,
/// which the unit owns and releases when it is dropped.
///
/// This is the shared boundary for callers, inspection, tests, and the future
/// semantic-analysis phase. Parser-machine state is deliberately not exposed.
///
/// C99: `translation-unit`, §6.9 paragraph 1, p. 140; PDF p. 152.
#[derive(Debug)]
pub(crate) struct ParsedTranslationUnit<'tu> {
    roots: RegionVec<ExternalDeclaration<'tu>>,
}

impl<'tu> ParsedTranslationUnit<'tu> {
    #[cfg_attr(
        not(any(test, feature = "benchmarking-internals")),
        expect(
            dead_code,
            reason = "The CLI reads roots through inspection; tests and benchmarks read them here."
        )
    )]
    pub(crate) fn external_declarations(&self) -> &[ExternalDeclaration<'tu>] {
        &self.roots
    }

    /// The whole tree as Rust debug output, for storage debugging. Each root
    /// prints on one line in compact form even under `{:#?}`: indenting a
    /// deeply nested tree would make the output grow with the square of its
    /// depth.
    pub(crate) fn raw_debug(&self) -> impl Debug + Send + '_ {
        RawRoots(&self.roots)
    }
}

/// [`ParsedTranslationUnit::raw_debug`]'s view.
struct RawRoots<'a, 'tu>(&'a [ExternalDeclaration<'tu>]);

/// One root in compact debug form, whatever the formatter's flags.
struct CompactRoot<'a, 'tu>(&'a ExternalDeclaration<'tu>);

impl Debug for RawRoots<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParsedTranslationUnit")
            .field("roots", &RawList(self.0))
            .finish()
    }
}

/// The roots as a list of compact entries.
struct RawList<'a, 'tu>(&'a [ExternalDeclaration<'tu>]);

impl Debug for RawList<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.0.iter().map(CompactRoot))
            .finish()
    }
}

impl Debug for CompactRoot<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl Parser<'_, '_, '_> {
    /// Where the preprocessor stopped reading, for end-of-input locations.
    fn position(&self) -> SourcePosition {
        self.cursor.upstream.position(self.context)
    }
}

impl GetSourceFileIndex for Parser<'_, '_, '_> {
    fn source_file_index(&self) -> u32 {
        self.cursor.upstream.source_file_index()
    }
}

impl<'tu> Parser<'_, 'tu, '_> {
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

    /// The translation context this parser borrows, for tests outside the
    /// parser.
    #[cfg(test)]
    pub(crate) fn context(&mut self) -> &mut Context<'tu> {
        self.context
    }
}
