//! Clang's warnings about reading a local variable before anything stores
//! to it: `-Wuninitialized`, `-Wsometimes-uninitialized`,
//! `-Wuninitialized-const-pointer` and `-Wconditional-uninitialized`. C99
//! gives such a variable an indeterminate value (§6.2.4p5, §6.7.8p10), and
//! reading it is undefined where the value may be a trap representation
//! (§6.2.6.1p5); each warning is one §5.1.1.3p1 permits.
//!
//! Clang runs a dataflow analysis over its control-flow graph when it
//! finishes a function (`runUninitializedVariablesAnalysis` in
//! `UninitializedValues.cpp`), and reports it as `DiagnoseUninitializedUse`
//! in `AnalysisBasedWarnings.cpp` does. The warnings name blocks of that
//! graph, so `cfg` builds it as Clang's builder does; `values` runs the
//! analysis to its fixed point and finds each use of a variable that may be
//! uninitialized, with the branches after which it is; `report` words them.
//!
//! GCC's warnings of these names come from its optimizer and depend on the
//! optimization level, so GCC's rules report none of them.
//! C99: §6.2.4p5, p. 32; PDF p. 44; §6.7.8p10, p. 126; PDF p. 138;
//! §6.2.6.1p5, p. 38; PDF p. 50; §5.1.1.3p1, p. 11; PDF p. 23.

mod cfg;
mod report;
mod values;

use rustc_hash::FxBuildHasher;

use super::{
    Analyzer,
    ReportOrder,
};
use crate::{
    configuration::{
        DiagnosticGroup as G,
        RuleSet,
    },
    translation_phases::{
        Context,
        parsing::syntax::StatementType,
    },
    util::{
        bump::{
            ArenaMap,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

/// What the analysis keeps between functions.
pub(super) struct UninitializedState<'s> {
    /// The graph and values of the function being analyzed, emptied after
    /// each one.
    arena:                  Bump,
    /// Functions defined with an empty body, by name, whose pointer-to-
    /// `const` arguments Clang takes as neither read nor written
    /// (`FunctionDecl::hasTrivialBody`).
    pub(super) trivial:     ArenaMap<'s, StringCacheId, ()>,
    /// The current function holds something the graph cannot follow: an
    /// object with `cleanup`, whose destruction Clang's graph adds, or an
    /// initializer whose excess elements Clang drops unevaluated.
    pub(super) unsupported: bool,
}

impl<'s> UninitializedState<'s> {
    pub(super) fn new(scratch: &'s Bump) -> Self {
        Self {
            arena:       Bump::new(),
            trivial:     ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            unsupported: false,
        }
    }
}

/// The groups whose warnings the analysis gives, any of which makes Clang
/// run it.
const GROUPS: [G; 4] = [
    G::Uninitialized,
    G::SometimesUninitialized,
    G::ConditionalUninitialized,
    G::UninitializedConstPointer,
];

impl Analyzer<'_, '_, '_> {
    /// Analyzes the current function, when its analysis is done, and
    /// reports the uses of uninitialized variables Clang reports.
    /// C99: §6.2.4p5, p. 32; PDF p. 44.
    pub(super) fn report_uninitialized_uses(&mut self) {
        let Some(function) = self.functions.current else {
            return;
        };
        let unsupported = std::mem::take(&mut self.uninitialized.unsupported);
        let body = function.syntax.body;
        if let StatementType::Compound { items } = body.kind
            && items.is_empty()
            && let Some(name) = function.syntax.declarator.identifier()
        {
            _ = self.uninitialized.trivial.insert(name.name, ());
        }
        // Clang's analysis stops at the unit's first error, and runs only
        // when one of its warnings is on where the function begins.
        if unsupported
            || self.tainted
            || self.context.configuration.compiler_identity().rules() != RuleSet::Clang
            || self.context.error_count() != 0
            || !GROUPS.iter().any(|&group| {
                self.context
                    .group_severity(group, function.syntax.source_vectors)
                    .is_some()
            })
        {
            return;
        }
        let mut arena = std::mem::replace(&mut self.uninitialized.arena, Bump::new());
        self.analyze_uninitialized(&arena);
        arena.reset();
        self.uninitialized.arena = arena;
    }

    /// Builds the graph, runs the analysis and reports what it finds, all in
    /// `arena`.
    fn analyze_uninitialized(&mut self, arena: &Bump) {
        let Some(function) = self.functions.current else {
            return;
        };
        let builder = cfg::Builder::new(arena);
        let Some(cfg) = self.build_uninitialized_cfg(builder, function.syntax.body) else {
            return;
        };
        if cfg.vars.is_empty() {
            return;
        }
        let values = values::solve(&cfg, arena);
        let uses = values::find_uses(&cfg, &values, arena);
        if uses.order.is_empty() {
            return;
        }
        // Clang reports them as it finishes the function, after what it
        // reported in the body (`AnalysisBasedWarnings::IssueWarnings`).
        let order = ReportOrder::After(self.context.file_location(Context::last_source_vector(
            function.syntax.body.source_vectors,
        )));
        self.with_report_order(order, |analyzer| {
            analyzer.report_uninitialized(&cfg, &uses);
        });
    }
}
