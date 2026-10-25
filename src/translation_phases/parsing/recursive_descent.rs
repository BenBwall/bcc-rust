//! Opt-in phase-7 recursive descent parser.
//!
//! Each production family has a module of recursive `rd_*` functions; a
//! family that is not yet ported runs on the retained frames through
//! [`Parser::rd_child`]. Machine mode is the default, including when this
//! study feature is compiled. Recovery reproduces the frame grammar's
//! diagnostics, synchronization stops and provenance.
//!
//! C99: phase 7, §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22;
//! expressions §6.5, pp. 67-94; PDF pp. 79-106; statements §6.8,
//! pp. 131-139; PDF pp. 143-151. Semantic constraints remain in analysis.

mod declaration_specifiers;
mod declarations;
mod declarators;
mod enum_specifier;
mod expressions;
mod gnu;
mod initializers;
mod modern;
mod msvc;
mod parameter_lists;
mod statements;
mod struct_or_union;
mod type_names;

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
        ParseFrame,
        ParseFrameKind,
        ParseValue,
    },
    recovery::SynchronizationSet,
    syntax::ConstantExpression,
};
use crate::translation_phases::SourceVectors;

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

    /// Whether `child`'s production family has a recursive implementation
    /// that [`Self::study_child`] dispatches to. Add a family here and to
    /// `study_child` when porting it.
    pub(super) fn is_recursive_family(child: &ParseFrame<'tu, 'p>) -> bool {
        matches!(
            child,
            ParseFrame::Expression(_)
                | ParseFrame::Statement(_)
                | ParseFrame::CompoundStatement(_)
                | ParseFrame::DeclarationSpecifiers(_)
                | ParseFrame::StructOrUnionSpecifier(_)
                | ParseFrame::EnumSpecifier(_)
                | ParseFrame::Modern(_)
                | ParseFrame::Gnu(_)
                | ParseFrame::Msvc(_)
                | ParseFrame::Declaration(_)
                | ParseFrame::FunctionDefinition(_)
                | ParseFrame::Initializer(_)
                | ParseFrame::Declarator(_)
                | ParseFrame::ParameterList(_)
                | ParseFrame::TypeName(_)
        )
    }

    /// Runs a new child using recursive grammar or the retained frame grammar.
    pub(super) fn study_child(&mut self, child: ParseFrame<'tu, 'p>) -> Option<ParseValue<'tu>> {
        match child {
            | ParseFrame::Expression(frame) => {
                let (mode, boundary, recovery) = frame.study_parameters();
                let result = self.rd_expression(mode, boundary, recovery)?;
                Some(if mode == ExpressionMode::ConstantExpression {
                    ParseValue::ConstantExpression(ConstantExpressionResult {
                        expression: ConstantExpression(result.expression),
                        recovered:  result.recovered,
                    })
                } else {
                    ParseValue::Expression(result)
                })
            },
            | ParseFrame::Statement(frame) => {
                let (scope, block_item, leave_else) = frame.study_parameters();
                self.rd_statement(scope, block_item, leave_else)
                    .map(ParseValue::Statement)
            },
            | ParseFrame::CompoundStatement(frame) => {
                let function_body = frame.study_function_body();
                self.rd_compound(function_body)
                    .map(ParseValue::CompoundStatement)
            },
            | ParseFrame::DeclarationSpecifiers(frame) => {
                let (mode, implicit_name_allowed) = frame.study_parameters();
                self.rd_declaration_specifiers(mode, implicit_name_allowed)
                    .map(ParseValue::DeclarationSpecifiers)
            },
            | ParseFrame::StructOrUnionSpecifier(frame) => {
                self.pools.struct_or_union_specifiers.reclaim(frame);
                self.rd_struct_or_union_specifier()
                    .map(ParseValue::StructOrUnionSpecifier)
            },
            | ParseFrame::EnumSpecifier(_) =>
                self.rd_enum_specifier().map(ParseValue::EnumSpecifier),
            | ParseFrame::Modern(frame) => {
                let (kind, keyword, errors) = frame.study_parameters();
                self.pools.modern_frames.reclaim(frame);
                self.rd_modern(kind, keyword, errors)
                    .map(ParseValue::Modern)
            },
            | ParseFrame::Gnu(frame) => {
                let (kind, errors) = frame.study_parameters();
                self.pools.gnu_frames.reclaim(frame);
                self.rd_gnu(kind, errors).map(ParseValue::Gnu)
            },
            | ParseFrame::Msvc(frame) => {
                let (keyword, errors) = frame.study_parameters();
                self.pools.msvc_frames.reclaim(frame);
                self.rd_msvc(keyword, errors).map(ParseValue::Statement)
            },
            | ParseFrame::Declaration(frame) => {
                let (context, starting_error_count) = frame.study_parameters();
                self.rd_declaration(context, starting_error_count)
                    .map(ParseValue::Declaration)
            },
            | ParseFrame::FunctionDefinition(frame) => {
                let (head, starting_error_count) = frame.study_parameters();
                self.rd_function_definition(head, starting_error_count)
                    .map(ParseValue::FunctionDefinition)
            },
            | ParseFrame::Initializer(frame) => {
                let (starting_error_count, parenthesis_boundary, bracket_boundary) =
                    frame.study_parameters();
                self.rd_initializer(starting_error_count, parenthesis_boundary, bracket_boundary)
                    .map(ParseValue::Initializer)
            },
            | ParseFrame::Declarator(frame) => self
                .rd_declarator(frame.study_parameters())
                .map(ParseValue::Declarator),
            | ParseFrame::ParameterList(frame) => {
                let allow_k_and_r = frame.study_parameters();
                self.pools.parameter_lists.reclaim(frame);
                self.rd_parameter_list(allow_k_and_r)
                    .map(ParseValue::ParameterList)
            },
            | ParseFrame::TypeName(frame) => {
                let (starting_error_count, compound_literal) = frame.study_parameters();
                self.rd_type_name_syntax(starting_error_count, compound_literal)
                    .map(ParseValue::TypeName)
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

    /// Runs the synchronization scan `set` from recursive grammar code and
    /// returns the provenance of the tokens it discarded.
    pub(super) fn rd_recover(&mut self, set: SynchronizationSet) -> Option<SourceVectors> {
        #[cfg(test)]
        {
            let depth = self.frames.len();
            self.recover(set, depth)
        }
        #[cfg(not(test))]
        {
            self.recover(set)
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

    /// Enters a recursive production of `kind`: counts its depth, makes it
    /// the active frame kind for diagnostics, and stops with the frame-depth
    /// resource failure in limited mode. Pair with [`Self::study_leave`].
    #[inline(always)]
    pub(super) fn rd_enter(&mut self, kind: ParseFrameKind) -> Option<StudyEntry> {
        if let StudyMode::Limited { depth } = self.study.mode
            && self.study.depth >= depth
        {
            self.study.stats.depth_limit_hits += 1;
            self.study.failure = self.resource_failure(ParserResource::FrameDepth, depth);
            return None;
        }
        let marker = 0_u8;
        let address = if self.study.stack_samples {
            std::ptr::from_ref(&marker).addr()
        } else {
            0
        };
        Some(self.study_enter(kind, address))
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

    pub(super) fn study_leave(&mut self, entry: StudyEntry) {
        self.study.depth -= 1;
        self.active_frame = entry.active;
        self.study.base = entry.base;
        self.study.segment_low = entry.segment_low;
    }
}

#[derive(Clone, Copy)]
pub(super) struct StudyEntry {
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
    /// Porting oracle: `BCC_TEST_PARSER_MODE=recursive` runs every test parse
    /// through the recursive grammar so the machine-mode expectations check
    /// it. Temporary; removed once the frame machine is deleted.
    pub(super) fn oracle_default() -> Self {
        #[cfg(test)]
        #[expect(clippy::disallowed_methods, reason = "test-only porting switch")]
        if std::env::var_os("BCC_TEST_PARSER_MODE").is_some_and(|mode| mode == "recursive") {
            return Self {
                mode: StudyMode::Recursive,
                ..Self::default()
            };
        }
        Self::default()
    }

    pub(super) fn enabled(&self) -> bool {
        self.mode != StudyMode::Machine && self.suspended == 0
    }
}
