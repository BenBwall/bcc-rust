//! Microsoft's structured exception handling: `__try` with its `__except`
//! filter and handler, or its `__finally` block, and `__leave`. Each block is
//! analyzed as any other code of the function, so that what it names counts
//! as named, inside a control region of its own ([`SehBlock`]): a jump may
//! leave such a block but not enter it, and `__leave` belongs in a `__try`
//! block. Lowering does not translate the statement.
//! MSVC extension: Microsoft Learn, "try-except statement", "try-finally
//! statement", "Compiler Error C2703", "Compiler Error C2705".
//! <https://learn.microsoft.com/en-us/cpp/cpp/try-except-statement>
//! Clang: `JumpScopeChecker`, `Sema::ActOnSEHLeaveStmt`.
//! C99: §4p6, p. 7; PDF p. 19 (extensions); §6.8.6.1p1, p. 137; PDF p. 149
//! and §6.8.4.2p2, p. 134; PDF p. 146 (the corresponding constraints for
//! variably modified identifiers).

use super::super::{
    Analyzer,
    ClErrorCode,
    ControlPoint,
    RegionId,
    RegionKind,
    RelatedNotes,
    SehBlock,
    SemanticErrorKind,
    Work,
    functions::FunctionWork,
};
use crate::{
    diagnostics::format_in,
    translation_phases::{
        Context,
        SourceVectors,
        parsing::syntax::{
            ExpressionSlot,
            Identifier,
            Statement,
            StatementType,
        },
    },
};

/// A statement that may leave a `__finally` block, as `cl` names it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::translation_phases::semantic_analysis) enum FinallyExit {
    Break,
    Continue,
    Return,
}

/// A jump that may enter a structured exception handling block.
#[derive(Clone, Copy)]
pub(in crate::translation_phases::semantic_analysis) enum SehEntry {
    /// A goto at `site` to the label defined at `label`.
    Goto {
        site:  SourceVectors,
        label: Identifier,
    },
    /// Switch dispatch to the case or default label at `site`.
    Case { site: SourceVectors },
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// `__try` with its `__except` filter and handler, or its `__finally`
    /// block, each analyzed as any other code of the function in a region of
    /// its own. An exception may leave the guarded block anywhere, so the
    /// filter and the handler run where it begins, and the code after the
    /// statement as well.
    pub(in crate::translation_phases::semantic_analysis) fn structured_exception_handler(
        &mut self,
        statement: &'tu Statement<'tu>,
    ) {
        use super::StatementWork as W;
        let StatementType::Seh(seh) = statement.kind else {
            return;
        };
        if let Some(function) = self.functions.current
            && !self.statements.seh_functions.contains(&function.id)
        {
            self.statements.seh_functions.push(function.id);
        }
        let previous = self.control.current;
        let dead = self.functions.dead;
        let handler = match seh.filter {
            | Some(_) => SehBlock::Except,
            | None => {
                self.note_seh_finally();
                SehBlock::Finally
            },
        };
        let handler_site = seh
            .handler_keyword
            .map_or(statement.source_vectors, |keyword| keyword.source_vectors);
        self.work
            .push(Work::FunctionWork(FunctionWork::RestoreDead(dead)));
        self.work
            .push(Work::FunctionWork(FunctionWork::RestoreUnvisited(
                self.functions.unvisited,
            )));
        self.work
            .push(Work::StatementWork(W::CloseSehBlock(previous)));
        self.work
            .push(Work::StatementWork(W::Substatement(seh.handler)));
        self.work
            .push(Work::FunctionWork(FunctionWork::RestoreDead(dead)));
        // The filter belongs to the `__except` block, whose keyword comes
        // first.
        if let Some(filter) = seh.filter {
            self.work.push(Work::StatementWork(W::SehFilter(
                self.statements.seh_filter,
            )));
            self.expression_slot(ExpressionSlot::Parsed(filter));
            self.work.push(Work::StatementWork(W::SehFilter(true)));
        }
        self.work.push(Work::StatementWork(W::OpenSehBlock(
            handler,
            Context::first_token_vector(handler_site),
        )));
        self.work
            .push(Work::FunctionWork(FunctionWork::RestoreDead(dead)));
        self.work
            .push(Work::StatementWork(W::CloseSehBlock(previous)));
        self.work
            .push(Work::StatementWork(W::Substatement(seh.body)));
        self.work.push(Work::StatementWork(W::OpenSehBlock(
            SehBlock::Try,
            Context::first_token_vector(statement.source_vectors),
        )));
    }

    /// Whether the code being analyzed is in an `__except` filter or
    /// handler. Clang's control-flow graph does not reach the handler
    /// (`CFGBuilder::VisitSEHTryStmt` adds no exception edges) and does not
    /// hold the filter, so Clang's warnings about code that runs
    /// (`DiagRuntimeBehavior`) wait there for the reachability walk, which
    /// also knows whether the function has a graph at all.
    pub(in crate::translation_phases::semantic_analysis) fn in_seh_handler(&self) -> bool {
        let Some(point) = self.control.point() else {
            return false;
        };
        let regions = &self.control.regions;
        let mut block = regions[point.region.0].nearest_seh;
        while let Some(region) = block {
            if regions[region.0].kind == RegionKind::Seh(SehBlock::Except) {
                return true;
            }
            block = regions[region.0]
                .parent
                .and_then(|parent| regions[parent.0].nearest_seh);
        }
        false
    }

    /// Whether the code being analyzed is an `__except` filter, which Clang
    /// does not finish as a full expression (`Parser::ParseSEHExceptBlock`
    /// makes no `ActOnFinishFullExpr`), so checks neither its conversions
    /// nor its overflow (`Sema::CheckCompletedExpr`).
    pub(in crate::translation_phases::semantic_analysis) const fn in_seh_filter(&self) -> bool {
        self.statements.seh_filter
    }

    /// Checks the gotos of the current function as its analysis ends, if
    /// it has a SEH statement ([`Self::check_seh_jumps`]).
    pub(in crate::translation_phases::semantic_analysis) fn finish_seh_function(&mut self) {
        if let Some(function) = self.functions.current
            && self.statements.seh_functions.contains(&function.id)
        {
            self.check_seh_jumps(function.id);
        }
    }

    /// Whether control going from `from` to `to`, or out of the function
    /// when `to` is `None`, leaves a `__finally` block.
    pub(in crate::translation_phases::semantic_analysis) fn leaves_finally(
        &self,
        from: RegionId,
        to: Option<RegionId>,
    ) -> bool {
        let regions = &self.control.regions;
        let mut block = regions[from.0].nearest_seh;
        while let Some(region) = block {
            if to.is_some_and(|to| super::super::facts::ancestry::ancestor(regions, region, to)) {
                return false;
            }
            if regions[region.0].kind == RegionKind::Seh(SehBlock::Finally) {
                return true;
            }
            block = regions[region.0]
                .parent
                .and_then(|parent| regions[parent.0].nearest_seh);
        }
        false
    }

    /// Whether a goto from `from` to `to` draws Clang's `-Wjump-seh-finally`
    /// or `cl`'s C4532: it leaves a `__finally` block, which for `cl` is its
    /// innermost SEH block. `cl` says nothing of a goto from a `__try` or
    /// `__except` block inside the `__finally` block, though it does of a
    /// `break` or `continue` there. Microsoft Learn: "Compiler Warning
    /// (level 1) C4532".
    pub(in crate::translation_phases::semantic_analysis) fn goto_leaves_finally(
        &self,
        from: RegionId,
        to: RegionId,
    ) -> bool {
        let regions = &self.control.regions;
        self.leaves_finally(from, Some(to))
            && (self.context.configuration.compiler_identity()
                != crate::configuration::CompilerIdentity::Msvc
                || regions[from.0].nearest_seh.is_some_and(|block| {
                    regions[block.0].kind == RegionKind::Seh(SehBlock::Finally)
                }))
    }

    /// Clang's `-Wjump-seh-finally` and `cl`'s C4532 for a `break`,
    /// `continue` or `return` that leaves a `__finally` block: Clang warns
    /// of each at its keyword (`CheckJumpOutOfSEHFinally`), and `cl` of
    /// `break` and `continue` only, naming the keyword. Microsoft Learn:
    /// "try-finally statement", "Compiler Warning (level 1) C4532".
    pub(in crate::translation_phases::semantic_analysis) fn check_finally_exit(
        &mut self,
        statement: &'tu Statement<'tu>,
        exit: FinallyExit,
    ) {
        if let Some(point) = self.control.point() {
            let cl = self.context.configuration.compiler_identity()
                == crate::configuration::CompilerIdentity::Msvc;
            let target = match exit {
                | FinallyExit::Return if cl => return,
                | FinallyExit::Return => None,
                | FinallyExit::Break | FinallyExit::Continue => {
                    let Some(&(owner, nearest_loop)) = self.control.targets.last() else {
                        return;
                    };
                    let owner = match (exit, nearest_loop) {
                        | (FinallyExit::Continue, Some(index)) =>
                            self.control.loops[index].statement,
                        | (FinallyExit::Continue, None) => return,
                        | _ => owner,
                    };
                    Some(self.control.statements[owner.0].entry.region)
                },
            };
            if self.leaves_finally(point.region, target) {
                // Only `cl`'s words name the statement.
                let name = self.context.configuration.msvc_wording().then(|| {
                    self.context.string_cache.intern(match exit {
                        | FinallyExit::Break => "break",
                        | FinallyExit::Continue => "continue",
                        | FinallyExit::Return => "return",
                    })
                });
                self.group_diagnostic(
                    crate::configuration::DiagnosticGroup::JumpSehFinally,
                    SemanticErrorKind::JumpOutOfFinally,
                    Context::first_token_vector(statement.source_vectors),
                    name,
                    None,
                );
            }
        }
    }

    /// Opens the region of a `__try`, `__except` or `__finally` block whose
    /// keyword is at `keyword`.
    pub(super) fn open_seh_block(&mut self, block: SehBlock, keyword: SourceVectors) {
        _ = self.control.open(
            self.scope,
            RegionKind::Seh(block),
            super::super::CleanupKind::None,
            keyword,
        );
    }

    /// `__leave` jumps to the end of the innermost `__try` block, so one
    /// outside every `__try` block of its function is an error, in Clang's
    /// words or as `cl`'s C2703. A `__try` block counts around its own
    /// `__except` and `__finally` blocks only if it encloses the whole
    /// statement. Microsoft Learn: "try-except statement", "Compiler Error
    /// C2703".
    pub(in crate::translation_phases::semantic_analysis) fn check_leave(
        &mut self,
        statement: &'tu Statement<'tu>,
    ) {
        let Some(point) = self.control.point() else {
            return;
        };
        let regions = &self.control.regions;
        let mut block = regions[point.region.0].nearest_seh;
        while let Some(region) = block {
            if regions[region.0].kind == RegionKind::Seh(SehBlock::Try) {
                return;
            }
            block = regions[region.0]
                .parent
                .and_then(|parent| regions[parent.0].nearest_seh);
        }
        let site = Context::first_token_vector(statement.source_vectors);
        if self.context.configuration.msvc_wording() {
            self.cl_error(
                ClErrorCode::IllegalLeave,
                site,
                "illegal __leave statement",
                None,
            );
        } else {
            self.error(SemanticErrorKind::LeaveOutsideTry, site, None, None);
        }
    }

    /// Reports a goto or switch dispatch from `from` to `to` that enters a
    /// `__try`, `__except` or `__finally` block, and says whether it did:
    /// at the jump in Clang's words, with a note at each block it enters,
    /// innermost first; or at the label as `cl`'s C2705, which names the
    /// innermost block and calls a case label `'null'`, once per label.
    /// `cl` compares how deep the jump and the label are in SEH blocks
    /// (`cl_entered_seh_block`), so a jump to a shallower label enters
    /// nothing ([`Self::goto_leaves_finally`] may still warn of it).
    /// Microsoft Learn: "Compiler Error C2705". Clang: `JumpScopeChecker`.
    pub(in crate::translation_phases::semantic_analysis) fn check_seh_entry(
        &mut self,
        from: ControlPoint,
        to: ControlPoint,
        entry: SehEntry,
        cl_reported: bool,
    ) -> bool {
        let entered = if self.context.configuration.compiler_identity()
            == crate::configuration::CompilerIdentity::Msvc
        {
            [
                self.control.cl_entered_seh_block(from.region, to.region),
                None,
                None,
            ]
        } else {
            self.control.entered_seh_blocks::<3>(from.region, to.region)
        };
        let Some(innermost) = entered[0] else {
            return false;
        };
        if self.context.configuration.msvc_wording() {
            if !cl_reported {
                let RegionKind::Seh(block) = self.control.regions[innermost.0].kind else {
                    unreachable!("an entered SEH region is one")
                };
                let (site, name) = match entry {
                    | SehEntry::Goto { label, .. } => (
                        label.source_vectors,
                        self.context.string_cache.at(label.name),
                    ),
                    | SehEntry::Case { site } => (site, "null"),
                };
                let words = format_in!(
                    self.scratch,
                    "'{name}': illegal jump into '{}' scope",
                    block.keyword()
                );
                self.cl_error(ClErrorCode::IllegalJumpIntoScope, site, words, None);
            }
            return true;
        }
        let mut related = RelatedNotes::default();
        for (note, region) in related.notes.iter_mut().zip(entered) {
            *note = region.map(|region| self.seh_note(region, "jump bypasses initialization of"));
        }
        let (kind, site) = match entry {
            | SehEntry::Goto { site, .. } => (SemanticErrorKind::JumpIntoSehBlock, site),
            | SehEntry::Case { site } => (SemanticErrorKind::SwitchIntoSehBlock, site),
        };
        self.with_related_notes(related, |analyzer| {
            analyzer.error_with_notes(kind, Context::first_token_vector(site), None);
        });
        true
    }

    /// Reports a computed goto at `site` that may reach the label `target`
    /// at `label` across a `__try`, `__except` or `__finally` block, entering
    /// it (`exits` false) or leaving it, which Clang rejects since the
    /// goto's targets are unknown. Clang: `JumpScopeChecker`.
    /// GNU extension: GCC manual, "Labels as Values".
    pub(in crate::translation_phases::semantic_analysis) fn report_indirect_seh_jump(
        &mut self,
        site: SourceVectors,
        label: SourceVectors,
        block: RegionId,
        exits: bool,
    ) {
        let mut related = RelatedNotes::default();
        related.notes[0] = Some((
            self.context.file_location(label),
            self.context
                .string_cache
                .intern("possible target of indirect goto statement"),
        ));
        related.notes[1] = Some(self.seh_note(
            block,
            if exits {
                "jump exits"
            } else {
                "jump bypasses initialization of"
            },
        ));
        self.with_related_notes(related, |analyzer| {
            analyzer.error_with_notes(SemanticErrorKind::IndirectJumpSehBlock, site, None);
        });
    }

    /// Clang's note at the keyword of the block `region`: `words` and the
    /// block's name, such as "jump exits __try block".
    fn seh_note(
        &mut self,
        region: RegionId,
        words: &str,
    ) -> (SourceVectors, crate::util::string_cache::StringCacheId) {
        let data = self.control.regions[region.0];
        let RegionKind::Seh(block) = data.kind else {
            unreachable!("an SEH block's region is one")
        };
        let text = format_in!(self.scratch, "{words} {} block", block.keyword());
        (
            self.context.file_location(data.source),
            self.context.string_cache.intern(text),
        )
    }
}
