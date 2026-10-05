//! Non-recursive C language parser and its arena-backed syntax model.
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
//! the repository's `c-spec.pdf`.

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
mod initializer;
mod inspection;
mod machine;
mod parameter_list;
mod recovery;
mod scope;
mod statement;
mod struct_or_union;
mod syntax;
mod syntax_log;
#[cfg(test)]
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
        SetPosition,
        SetSourceFileIndex,
        SourcePosition,
        TranslationPhase,
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

/// Owns parser input, control frames, the syntax node count, scopes, and
/// diagnostics.
///
/// Calling [`TranslationPhase::next_item`] drives the machine until one
/// external declaration reduces or the preprocessed token stream ends.
///
/// C99: translation units and external declarations are specified by §6.9,
/// p. 140; PDF p. 152: a translation unit “consists of a sequence of external
/// declarations.” The diagnostic obligation is §5.1.1.3, p. 11; PDF p. 23.
pub(crate) struct Parser<'tu, 'p> {
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
    /// they grow in place in their own region instead of the parse arena,
    /// and the finished list is copied once into the translation-unit arena.
    emitted_roots: RegionVec<ExternalDeclaration<'tu>>,
    /// Parser-visible ordinary-name classification used for typedef ambiguity.
    scopes: ScopeStack<'p>,
    /// Function-local label namespaces, independent of ordinary identifiers.
    label_scopes: LabelScopes<'p>,
    /// Interned `__func__`, predeclared in every function body.
    func_name: Option<StringCacheId>,
    /// Active switch contexts used to associate `case` and `default` labels.
    switch_scopes: ArenaVec<'p, SwitchScope>,
    /// Scan storage for the nested specifiers whose enumeration constants a
    /// function definition's parameters declare, reused by every definition.
    binding_scan: ArenaVec<'p, TypeSpecifiers<'tu>>,
    /// Delimiter depth and ownership while a synchronization scan is active.
    recovery: RecoveryState<'p>,
    /// Number of hard parser diagnostics emitted so far.
    hard_error_count: usize,
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
    trace: Vec<FrameTraceEvent>,
    #[cfg(test)]
    /// Optional hard stop used by malformed-input tests to turn nonprogress
    /// into a deterministic failure instead of an external test timeout.
    action_budget: Option<usize>,
}

/// Fully preprocessed parser input. The preprocessor can be dropped before
/// parser working memory is created.
pub(crate) struct PreprocessedTranslationUnit {
    upstream: token_cursor::Upstream,
}

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
            external_declarations: 1_000_000,
            syntax_nodes:          8_000_000,
            frame_depth:           1_000_000,
            source_segments:       40_000_000,
        }
    }
}

/// One completely parsed translation unit: its source-ordered roots, which
/// borrow the syntax tree from the translation-unit arena.
///
/// This is the shared boundary for callers, inspection, tests, and the future
/// semantic-analysis phase. Parser-machine state is deliberately not exposed.
#[derive(Debug)]
pub(crate) struct ParsedTranslationUnit<'tu> {
    roots: &'tu [ExternalDeclaration<'tu>],
}

impl<'tu> ParsedTranslationUnit<'tu> {
    #[cfg_attr(
        not(any(test, feature = "benchmarking-internals")),
        expect(
            dead_code,
            reason = "The CLI reads roots through inspection; tests and benchmarks read them here."
        )
    )]
    pub(crate) fn external_declarations(&self) -> &'tu [ExternalDeclaration<'tu>] {
        self.roots
    }

    /// The whole tree as Rust debug output, for storage debugging. Each root
    /// prints on one line in compact form even under `{:#?}`: indenting a
    /// deeply nested tree would make the output grow with the square of its
    /// depth.
    pub(crate) fn raw_debug(&self) -> impl Debug + '_ {
        RawRoots(self.roots)
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

impl GetPosition for Parser<'_, '_> {
    fn position(&self, context: &Context<'_>) -> SourcePosition {
        self.cursor.upstream.position(context)
    }
}

impl SetPosition for Parser<'_, '_> {
    fn set_position(&mut self, context: &mut Context<'_>, position: SourcePosition) {
        self.cursor.upstream.set_position(context, position);
    }
}

impl GetSourceFileIndex for Parser<'_, '_> {
    fn source_file_index(&self) -> u32 {
        self.cursor.upstream.source_file_index()
    }
}

impl SetSourceFileIndex for Parser<'_, '_> {
    fn set_source_file_index(&mut self, context: &mut Context<'_>, source_file_index: u32) {
        self.cursor
            .upstream
            .set_source_file_index(context, source_file_index);
    }
}

impl<'tu> TranslationPhase<'_> for Parser<'tu, '_> {
    type Item = ExternalDeclaration<'tu>;

    fn next_item(&mut self, context: &mut Context<'_>) -> Option<Self::Item> {
        let root = self.drive(context)?;
        self.emitted_roots.push(root);
        Some(root)
    }
}
