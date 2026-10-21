//! Opt-in phase-7 recursive descent performance experiment.
//!
//! `expressions` implements precedence climbing, prefix and postfix descent;
//! `statements` implements recursive statement and compound parsing.
//! Declarations, type names, initializers and extension productions retain the
//! existing frames. A fallback runs on the same driver and retains its scopes
//! and provenance. Machine mode is the default, including when this study
//! feature is compiled.
//!
//! The experiment targets valid C99 syntax. Extension atoms/statements and
//! malformed operand recovery use frames. Recovery after an already consumed
//! recursive prefix can produce different repairs; corpus comparisons must
//! report those differences. It is not a replacement for the production parser.
//!
//! C99: phase 7, §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22;
//! expressions §6.5, pp. 67-94; PDF pp. 79-106; statements §6.8,
//! pp. 131-139; PDF pp. 143-151. Semantic constraints remain in analysis.

mod expressions;
mod statements;

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "test workloads and owned inspection snapshots use the global allocator"
)]
mod tests;

use super::{
    ExternalDeclaration,
    ParsedTranslationUnit,
    Parser,
    errors::ParserResource,
    frames::expression::ExpressionMode,
    machine::{
        ConstantExpressionResult,
        ExpressionResult,
        ParseFrame,
        ParseFrameKind,
        ParseValue,
    },
    syntax::ConstantExpression,
};

impl<'tu, 'p> Parser<'_, 'tu, 'p> {
    /// Selects the study algorithm. Call before reading the first root.
    pub(crate) fn with_study_mode(mut self, mode: StudyMode) -> Self {
        assert!(self.frames.is_empty(), "select study mode before parsing");
        self.study.mode = mode;
        self
    }

    /// Enables grammar-entry stack-address sampling. Disabled for timing runs.
    pub(crate) fn with_study_stack_samples(mut self, enabled: bool) -> Self {
        self.study.stack_samples = enabled;
        self
    }

    /// Parses all roots while preserving the final study counters.
    pub(crate) fn parse_study_translation_unit(
        mut self,
    ) -> (ParsedTranslationUnit<'tu>, StudyStats) {
        while let Some(root) = self.drive() {
            self.emitted_roots.push(root);
            self.root_ends.push(self.cursor.consumed);
        }
        let stats = self.study_stats();
        (
            ParsedTranslationUnit {
                roots: self.emitted_roots,
                ends:  self.root_ends.leak(),
            },
            stats,
        )
    }

    /// Returns counters accumulated so far.
    pub(crate) fn study_stats(&self) -> StudyStats {
        self.study.stats
    }

    /// Runs a new child using recursive grammar or the retained frame grammar.
    pub(super) fn study_child(&mut self, child: ParseFrame<'tu, 'p>) -> Option<ParseValue<'tu>> {
        match child {
            | ParseFrame::Expression(frame) => {
                let (mode, boundary) = frame.study_parameters();
                let expression = self.rd_expression(mode, boundary)?;
                let recovered = expression.recovered;
                Some(if mode == ExpressionMode::ConstantExpression {
                    ParseValue::ConstantExpression(ConstantExpressionResult {
                        expression: ConstantExpression(expression),
                        recovered,
                    })
                } else {
                    ParseValue::Expression(ExpressionResult {
                        expression,
                        recovered,
                    })
                })
            },
            | ParseFrame::Statement(frame) => {
                let (scope, block_item) = frame.study_parameters();
                self.rd_statement(scope, block_item)
                    .map(ParseValue::Statement)
            },
            | ParseFrame::CompoundStatement(frame) => {
                let function_body = frame.study_function_body();
                self.rd_compound(function_body)
                    .map(ParseValue::CompoundStatement)
            },
            | _ => unreachable!("only recursive grammar families are intercepted"),
        }
    }

    /// Executes a fallback subtree with the ordinary action loop. `suspend`
    /// makes the whole subtree iterative, including its expression children.
    fn study_frame(
        &mut self,
        child: ParseFrame<'tu, 'p>,
        suspend: bool,
    ) -> Option<ParseValue<'tu>> {
        let depth = self.frames.len();
        let active = self.active_frame;
        self.push_frame(child);
        self.study.suspended += usize::from(suspend);
        let value = self.drive_until(Some(depth));
        self.study.suspended -= usize::from(suspend);
        self.active_frame = active;
        if let Some(ParseValue::ExternalDeclaration(root)) = value {
            self.study.failure = Some(root);
            None
        } else {
            value
        }
    }

    fn study_fallback(&mut self) -> Option<bool> {
        match self.study.mode {
            | StudyMode::Hybrid { depth } if self.study.depth >= depth => {
                self.study.stats.fallback_count += 1;
                Some(true)
            },
            | StudyMode::Limited { depth } if self.study.depth >= depth => {
                self.study.stats.depth_limit_hits += 1;
                self.study.failure = self.resource_failure(ParserResource::FrameDepth, depth);
                None
            },
            | _ => Some(false),
        }
    }

    fn study_enter(&mut self, kind: ParseFrameKind, address: usize) -> StudyEntry {
        let entry = StudyEntry {
            active:      self.active_frame,
            base:        self.study.base,
            segment_low: self.study.segment_low,
        };
        self.study.depth += 1;
        self.study.stats.max_depth = self.study.stats.max_depth.max(self.study.depth);
        // A new stacker segment has a different address interval. Only compare
        // samples within one segment (stacker supplies its remaining size).
        if self.study.stack_samples {
            let segment_low =
                stacker::remaining_stack().map(|remaining| address.saturating_sub(remaining));
            if self.study.depth == 1
                || segment_low
                    .zip(self.study.segment_low)
                    .is_some_and(|(now, before)| now.abs_diff(before) > 4096)
            {
                self.study.base = address;
            }
            self.study.segment_low = segment_low;
            // Without a platform stack bound, raw/hybrid samples still share
            // one native stack, but growing segments cannot be compared.
            if self.study.mode != StudyMode::Growing || segment_low.is_some() {
                self.study.stats.native_stack_span = self
                    .study
                    .stats
                    .native_stack_span
                    .max(self.study.base.abs_diff(address));
            }
        }
        self.active_frame = kind;
        entry
    }

    fn study_leave(&mut self, entry: StudyEntry) {
        self.study.depth -= 1;
        self.active_frame = entry.active;
        self.study.base = entry.base;
        self.study.segment_low = entry.segment_low;
    }
}

#[derive(Clone, Copy)]
struct StudyEntry {
    active:      ParseFrameKind,
    base:        usize,
    segment_low: Option<usize>,
}

/// Study-only parser selection; no mode changes the default compiler path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum StudyMode {
    #[default]
    Machine,
    /// Unbounded native recursion; run adversarial inputs in a subprocess.
    Recursive,
    /// Native recursion with stacker growth at recursive grammar entries.
    Growing,
    /// Switch to the iterative grammar at this active recursive depth.
    Hybrid { depth: usize },
    /// Emit the ordinary terminal frame-depth resource diagnostic at this
    /// depth.
    Limited { depth: usize },
}

/// Samples are grammar-entry addresses, not a high-water mark of every native
/// call. The span excludes preprocessing and is a lower bound; growing stacks
/// are separate segments, not a contiguous stack-byte measurement.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct StudyStats {
    pub(crate) expression_entries: usize,
    pub(crate) statement_entries:  usize,
    pub(crate) compound_entries:   usize,
    pub(crate) max_depth:          usize,
    pub(crate) native_stack_span:  usize,
    pub(crate) fallback_count:     usize,
    pub(crate) depth_limit_hits:   usize,
}

#[derive(Default)]
pub(super) struct StudyState<'tu> {
    pub(super) mode:    StudyMode,
    pub(super) stats:   StudyStats,
    pub(super) failure: Option<ExternalDeclaration<'tu>>,
    depth:              usize,
    suspended:          usize,
    base:               usize,
    segment_low:        Option<usize>,
    stack_samples:      bool,
}

impl StudyState<'_> {
    pub(super) fn enabled(&self) -> bool {
        self.mode != StudyMode::Machine && self.suspended == 0
    }
}
