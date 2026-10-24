//! A switch whose controlling expression code generation folds:
//! `CodeGenFunction::EmitSwitchStmt` selects the matching case, else
//! `default`, and emits only the statements `CollectStatementsForCase`
//! gathers from that label up to the `break` that leaves the switch, where
//! its other case labels resume nothing. When the collection fails, Clang
//! emits the switch as usual.
//!
//! Semantic analysis walks the body in source order as for any switch, and
//! learns each case label's value just before the statement it labels.
//! Until a label is selected, the body is analyzed as unfolded; selecting
//! a label then marks every reference recorded since the body began as
//! skipped, which can only make more code skipped. From the selected label
//! on, later labels resume nothing, and the selected statements' `break`
//! ends them. A `default` is selected when it is met, tentatively: a later
//! matching case replaces it, so it is selected only when every later
//! case's collection succeeds. Without a match and a `default`, Clang emits
//! nothing of a body that holds no label.
//!
//! #104's whole-arm omission can mark the same regions: the references
//! before the selected label at selection ([`Analyzer::skip_folded_region`])
//! and the statements after its `break` as they are walked
//! ([`Phase::Ended`]). Lowering still dispatches to every case label, so
//! only statements that hold no label of the switch may be omitted.
//! C99: §6.8.4.2 paragraphs 4-5, p. 134; PDF p. 146; §6.8.6.3 paragraph 2,
//! p. 138; PDF p. 150; §6.8.1 paragraph 4, p. 132; PDF p. 144.

use super::super::{
    Analyzer,
    DefinitionKind,
    Integer,
    TypeId,
};
use crate::{
    translation_phases::parsing::syntax::{
        BlockItem,
        Expression,
        Statement,
        StatementType,
    },
    util::bump::ArenaVec,
};

/// How many case labels after a `default` [`Analyzer::later_cases_collect`]
/// collects from, bounding the work per `default` to that many walks of
/// the body.
const LATER_CASES: usize = 64;

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Starts folding switch `index` (statement `switch`, body `body`) when
    /// code generation folds its controlling `condition`, of promoted type
    /// `ty`. Nothing is analyzed ahead of its source position.
    pub(in crate::translation_phases::semantic_analysis) fn prepare_folded_switch(
        &mut self,
        index: usize,
        switch: &'tu Statement<'tu>,
        condition: Option<&'tu Expression<'tu>>,
        body: &'tu Statement<'tu>,
        ty: TypeId,
    ) {
        if self.tainted {
            return;
        }
        let Some((bits, signed)) = self.integer_type(ty) else {
            return;
        };
        let Some(value) = condition.and_then(|condition| self.folded_integer(condition)) else {
            if let Some(condition) = condition
                && self.switch_labels(switch).is_some()
                && !self.statement_contains_label(body, true)
            {
                self.limit_optional_locals_for_next(condition, None);
            }
            return;
        };
        if self.switch_labels(switch).is_none() {
            return;
        }
        let start = SwitchMark {
            references:  self.reference_count(),
            definitions: self.definitions.len(),
            statements:  self.control.statements.len(),
        };
        _ = self.functions.emission.folds.insert(
            index,
            FoldedSwitch {
                value: value.cast(bits, signed),
                switch,
                body,
                start,
                phase: Phase::Searching,
                end: None,
                tentative: false,
                selected: None,
                after: false,
            },
        );
    }

    /// The skipped state that case or default `label` of the innermost
    /// switch gives the statement it labels: the code around the switch,
    /// unless the switch is folded and a label is selected. Then labels
    /// inside the selected statements change nothing, and those after them
    /// stay skipped. A `default` met before a matching case is selected
    /// tentatively, unless the switch has a case range, which Clang does
    /// not fold without a match, or a later case's collection fails
    /// ([`Self::later_cases_collect`]).
    /// C99: §6.8.4.2 paragraph 4, p. 134; PDF p. 146.
    pub(in crate::translation_phases::semantic_analysis) fn switch_label_dead(
        &mut self,
        label: &'tu Statement<'tu>,
    ) -> bool {
        let around = self.functions.switch_dead;
        let Some(index) = self.statements.switch else {
            return around;
        };
        let Some(fold) = self.functions.emission.folds.get(&index).copied() else {
            return around;
        };
        match fold.phase {
            | Phase::Searching => {
                if matches!(label.kind, StatementType::Default(_))
                    && self.later_cases_collect(fold, label)
                {
                    self.select(index, label, true);
                }
                around
            },
            | Phase::Collecting => self.functions.dead,
            | Phase::Ended => true,
            | Phase::Unfolded => around,
        }
    }

    /// Whether `default` of folded switch `fold` may be selected
    /// tentatively: the switch has no case range, and Clang's collection
    /// succeeds from every case label after it. A matching later case
    /// replaces the `default` then, while one whose collection fails would
    /// leave the switch unfolded, which cannot undo what selecting the
    /// `default` skipped. Past [`LATER_CASES`] later labels, the `default`
    /// is not selected: the switch is analyzed as unfolded, which leaves
    /// more code emitted than Clang's, never less.
    fn later_cases_collect(
        &mut self,
        fold: FoldedSwitch<'tu>,
        default: &'tu Statement<'tu>,
    ) -> bool {
        let Some(labels) = self.switch_labels(fold.switch) else {
            return false;
        };
        let Some(position) = self.label_position(default) else {
            return false;
        };
        let later = labels.labels.get(position + 1..).unwrap_or_default();
        labels.last_range.is_none()
            && later.len() <= LATER_CASES
            && later
                .iter()
                .all(|&label| self.collect_case(fold.body, label).is_some())
    }

    /// Learns the value of case `label` of the innermost switch: a folded
    /// switch selects it when it matches, unless a case range follows it,
    /// which fails Clang's scan.
    pub(in crate::translation_phases::semantic_analysis) fn folded_case(
        &mut self,
        label: &'tu Statement<'tu>,
        value: Integer,
    ) {
        let Some(index) = self.statements.switch else {
            return;
        };
        let Some(fold) = self.functions.emission.folds.get(&index).copied() else {
            return;
        };
        let undecided = match fold.phase {
            | Phase::Searching => true,
            | Phase::Collecting | Phase::Ended => fold.tentative,
            | Phase::Unfolded => false,
        };
        if !undecided || value.order_key() != fold.value.order_key() {
            return;
        }
        let position = self.label_position(label);
        let range_after =
            self.switch_labels(fold.switch)
                .zip(position)
                .is_none_or(|(labels, position)| {
                    labels.last_range.is_some_and(|range| range > position)
                });
        // The label resumes the code around the switch either way.
        self.functions.dead = self.functions.switch_dead;
        self.functions.unvisited &= self.functions.switch_dead;
        if range_after {
            // The label walk records every label but those in a statement
            // expression of a type name, where a case label is an error.
            debug_assert!(
                !fold.tentative || position.is_none(),
                "a tentative default precedes no case range"
            );
            self.set_phase(index, Phase::Unfolded);
        } else {
            self.select(index, label, false);
        }
    }

    /// Selects `label` of folded switch `index` when Clang's collection from
    /// it succeeds, skipping what was analyzed since the body began; when
    /// it fails, a matching case leaves the switch unfolded, and a
    /// `default` leaves it searching.
    ///
    /// This is the one point where a selection changes: a matching case
    /// replaces a tentative `default` here, and [`Self::skip_folded_region`]
    /// then covers everything since the body began again, a superset of
    /// what the `default`'s selection covered, its selected statements and
    /// what followed their `break` included. A tentative selection is never
    /// undone: [`Self::later_cases_collect`] admits one only when every case
    /// after the `default` collects, so a failed collection cannot follow
    /// it. What #104 marks under a tentative `default`, retroactively or
    /// after its `break`, stays valid when a case replaces it.
    fn select(&mut self, index: usize, label: &'tu Statement<'tu>, tentative: bool) {
        let Some(fold) = self.functions.emission.folds.get(&index).copied() else {
            return;
        };
        let collected = self.collect_case(fold.body, label);
        debug_assert!(
            collected.is_some() || !fold.tentative,
            "a case after a tentative default collects"
        );
        match collected {
            | Some(end) => {
                self.skip_folded_region(fold, Some(label));
                if let Some(fold) = self.functions.emission.folds.get_mut(&index) {
                    fold.phase = Phase::Collecting;
                    fold.end = end;
                    fold.tentative = tentative;
                    fold.selected = Some(label);
                }
            },
            | None if !tentative => self.set_phase(index, Phase::Unfolded),
            | None => {},
        }
    }

    /// Omits the already analyzed region before the selected label, including
    /// a tentative default replaced by a later matching case, or the whole
    /// body when nothing is selected. Lowering jumps straight to the
    /// selection, so only the blocks and labels leading to it are lowered:
    /// every other statement of the region is omitted with its references,
    /// case labels included, as Clang's `CollectStatementsForCase` skips it.
    fn skip_folded_region(&mut self, fold: FoldedSwitch<'tu>, label: Option<&'tu Statement<'tu>>) {
        let start = fold.start;
        let path = selection_path(self.scratch, fold.body, label);
        self.elide_references(start.references..self.reference_count());
        for definition in &mut self.definitions[start.definitions..] {
            if definition.kind == DefinitionKind::Object {
                definition.elided = true;
            }
        }
        for index in start.statements..self.control.statements.len() {
            let syntax = self.control.statements[index].syntax;
            if !self.held(syntax).label && !path.contains(&std::ptr::from_ref(syntax).addr()) {
                self.control.statements[index].elided = true;
            }
        }
    }

    /// Clang collects nothing after the selected break. Only a tentative
    /// `default` keeps the case labels after it, since a later matching
    /// case still replaces it ([`Self::select`]).
    pub(in crate::translation_phases::semantic_analysis) fn folded_statement_elided(
        &self,
        statement: &'tu Statement<'tu>,
    ) -> bool {
        self.statements
            .switch
            .and_then(|index| self.functions.emission.folds.get(&index))
            .is_some_and(|fold| {
                let holds = self.held(statement);
                fold.phase == Phase::Ended && !holds.label && !(fold.tentative && holds.case)
            })
    }

    /// Declarations are block items rather than statements, but collection
    /// skips them too after the selected break. C99 §6.8.4.2p4-5, p. 134;
    /// PDF p. 146; emission follows Clang's `CollectStatementsForCase`.
    pub(in crate::translation_phases::semantic_analysis) fn folded_declaration_elided(
        &self,
    ) -> bool {
        self.statements
            .switch
            .and_then(|index| self.functions.emission.folds.get(&index))
            .is_some_and(|fold| fold.phase == Phase::Ended)
    }

    fn set_phase(&mut self, index: usize, phase: Phase) {
        if let Some(fold) = self.functions.emission.folds.get_mut(&index) {
            fold.phase = phase;
        }
    }

    /// Ends a folded switch's selected statements at the `break` that
    /// `CollectStatementsForCase` stops at; Clang emits nothing for it.
    /// C99: §6.8.6.3 paragraph 2, p. 138; PDF p. 150.
    pub(in crate::translation_phases::semantic_analysis) fn switch_break(
        &mut self,
        statement: &'tu Statement<'tu>,
    ) {
        let dead = self.functions.dead;
        if let Some(fold) = self
            .statements
            .switch
            .and_then(|index| self.functions.emission.folds.get_mut(&index))
            && fold.phase == Phase::Collecting
            && fold.end.is_some_and(|end| std::ptr::eq(end, statement))
        {
            fold.phase = Phase::Ended;
            fold.after = dead;
            // Clang does not visit what follows the selected statements.
            self.functions.unvisited = true;
        }
    }

    /// At the end of switch `index`'s body, passes the skipped state its
    /// selected statements ended in to the code after it
    /// ([`FunctionWork::SwitchEnd`](super::super::functions::FunctionWork)).
    /// Without a match or `default`, Clang emits nothing of a body without
    /// labels or case ranges; otherwise an unfolded switch leaves the code
    /// around it.
    pub(in crate::translation_phases::semantic_analysis) fn finish_folded_switch(
        &mut self,
        index: usize,
    ) {
        let fold = self.functions.emission.folds.remove(&index);
        if let Some(fold) = fold
            && matches!(fold.phase, Phase::Collecting | Phase::Ended)
        {
            self.retain_folded_dispatch(fold.switch, fold.selected);
        }
        self.functions.emission.switch_after = match fold.map(|fold| fold.phase) {
            | Some(Phase::Collecting) => Some(self.functions.dead),
            | Some(Phase::Ended) => fold.map(|fold| fold.after),
            | Some(Phase::Searching) => {
                let fold = fold.expect("a searching switch is folded");
                let elided = self.switch_labels(fold.switch).is_some_and(|labels| {
                    labels.last_range.is_none()
                        && !labels
                            .labels
                            .iter()
                            .any(|label| matches!(label.kind, StatementType::Default(_)))
                }) && !self.held(fold.body).label;
                if elided {
                    self.skip_folded_region(fold, None);
                    self.retain_folded_dispatch(fold.switch, None);
                }
                None
            },
            | _ => None,
        };
    }

    fn retain_folded_dispatch(
        &mut self,
        switch: &'tu Statement<'tu>,
        label: Option<&'tu Statement<'tu>>,
    ) {
        if let Some(id) = self.control.statement_id(switch) {
            self.control.statements[id.0].generated = super::super::GeneratedControl::Switch(label);
            if let StatementType::Switch {
                condition_expression,
                ..
            } = switch.kind
                && let Some(condition) =
                    super::super::statements::slot_expression(condition_expression)
            {
                self.elide_condition_references(condition);
            }
        }
    }

    /// Clang's `CollectStatementsForCase` for `body` and the selected
    /// `label`: the `break` the collection ends at, if any, or `None` when
    /// it fails. The statements it collects run from the label to that
    /// `break`, or to the end of the body. It fails on a label or a
    /// declaration it would skip, a `break` inside a collected statement
    /// other than a block, a block left without a `break` after a
    /// declaration, and a label it cannot reach through blocks and labels.
    #[expect(
        clippy::option_option,
        reason = "Failure, an end at a break and an end of the body are Clang's three outcomes."
    )]
    fn collect_case(
        &self,
        body: &'tu Statement<'tu>,
        label: &'tu Statement<'tu>,
    ) -> Option<Option<&'tu Statement<'tu>>> {
        let mut frames = ArenaVec::new_in(self.scratch);
        let mut found = false;
        let mut end = None;
        let mut visit = Some((body, true));
        let mut outcome = None;
        loop {
            // Visits one statement, while `searching` for the label: its
            // outcome, or none when it entered a block.
            if let Some((mut statement, mut searching)) = visit.take() {
                outcome = loop {
                    match statement.kind {
                        | StatementType::Case(_, child) | StatementType::Default(child) => {
                            if searching && std::ptr::eq(statement, label) {
                                found = true;
                                searching = false;
                            }
                            statement = child;
                        },
                        | StatementType::CaseRange(range) => {
                            if searching && std::ptr::eq(statement, label) {
                                found = true;
                                searching = false;
                            }
                            statement = range.statement;
                        },
                        | StatementType::Break if !searching => {
                            end = Some(statement);
                            break Some(Collected::Success);
                        },
                        | StatementType::Compound { items } => {
                            frames.push(CollectFrame {
                                block: statement,
                                items: items.as_slice(),
                                next: 0,
                                searching,
                                started_live: !searching,
                                declared: false,
                            });
                            break None;
                        },
                        | _ if searching =>
                            break Some(if self.held(statement).label {
                                Collected::Failure
                            } else {
                                Collected::Success
                            }),
                        | _ =>
                            break Some(if self.held(statement).escaping_break {
                                Collected::Failure
                            } else {
                                Collected::FallThrough
                            }),
                    }
                };
            }
            let Some(frame) = frames.last_mut() else {
                return match outcome {
                    | Some(Collected::Success | Collected::FallThrough) if found => Some(end),
                    | _ => None,
                };
            };
            // The outcome of the block's item just visited.
            if let Some(result) = outcome.take() {
                let finished = match (result, frame.searching) {
                    | (Collected::Failure, _) => Some(Collected::Failure),
                    // The item held the label and ended at its `break`.
                    | (Collected::Success, true) if found => Some(if frame.declared {
                        Collected::Failure
                    } else {
                        self.rest_unlabeled(frame)
                    }),
                    | (Collected::Success, true) | (Collected::FallThrough, false) => None,
                    | (Collected::FallThrough, true) => {
                        frame.searching = false;
                        frame.declared.then_some(Collected::Failure)
                    },
                    | (Collected::Success, false) => Some(self.rest_unlabeled(frame)),
                };
                if let Some(finished) = finished {
                    _ = frames.pop();
                    outcome = Some(finished);
                    continue;
                }
            }
            if let Some(&item) = frame.items.get(frame.next) {
                frame.next += 1;
                frame.declared |= might_add_declaration(self.scratch, item);
                match item {
                    | BlockItem::Statement(statement) => visit = Some((statement, frame.searching)),
                    | BlockItem::Declaration(declaration) => {
                        let holds = self.declaration_holds(declaration);
                        outcome = Some(if frame.searching {
                            if holds.label {
                                Collected::Failure
                            } else {
                                Collected::Success
                            }
                        } else if holds.escaping_break {
                            Collected::Failure
                        } else {
                            Collected::FallThrough
                        });
                    },
                    | BlockItem::FunctionDefinition(_) =>
                        outcome = Some(if frame.searching {
                            Collected::Success
                        } else {
                            Collected::FallThrough
                        }),
                }
            } else {
                // A block left without its `break` ends its declarations'
                // lifetimes: Clang then emits it whole, if it started in the
                // selected statements and holds no `break`.
                let frame = frames.pop().expect("a block is being collected");
                outcome = Some(if frame.searching {
                    Collected::Success
                } else if frame.declared
                    && (!frame.started_live || self.held(frame.block).escaping_break)
                {
                    Collected::Failure
                } else {
                    Collected::FallThrough
                });
            }
        }
    }

    /// Clang requires the items after a collection's `break` to be
    /// skippable: success unless one holds an ordinary label, in a
    /// statement expression of a declaration too.
    fn rest_unlabeled(&self, frame: &CollectFrame<'tu>) -> Collected {
        let labeled = frame.items[frame.next..].iter().any(|item| match *item {
            | BlockItem::Statement(statement) => self.held(statement).label,
            | BlockItem::Declaration(declaration) => self.declaration_holds(declaration).label,
            | BlockItem::FunctionDefinition(_) => false,
        });
        if labeled {
            Collected::Failure
        } else {
            Collected::Success
        }
    }
}

/// The statements from a folded switch's `body` down to its selected
/// `label`, by address. A successful collection reaches the label only
/// through blocks and case labels (`collect_case`), none of
/// which lowers an expression or skips a declaration before it.
fn selection_path<'a, 'tu>(
    scratch: &'a crate::util::bump::Bump,
    body: &'tu Statement<'tu>,
    label: Option<&'tu Statement<'tu>>,
) -> crate::util::bump::ArenaSet<'a, usize> {
    let mut path = crate::util::bump::ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, scratch);
    let Some(label) = label else {
        return path;
    };
    // Breadth-first through blocks and labels, each with its parent.
    let mut nodes: ArenaVec<'_, (&'tu Statement<'tu>, Option<usize>)> = ArenaVec::new_in(scratch);
    nodes.push((body, None));
    let mut next = 0;
    while let Some(&(statement, _)) = nodes.get(next) {
        if std::ptr::eq(statement, label) {
            let mut at = Some(next);
            while let Some(index) = at {
                let (statement, parent) = nodes[index];
                _ = path.insert(std::ptr::from_ref(statement).addr());
                at = parent;
            }
            break;
        }
        match statement.kind {
            | StatementType::Case(_, child) | StatementType::Default(child) =>
                nodes.push((child, Some(next))),
            | StatementType::CaseRange(range) => nodes.push((range.statement, Some(next))),
            | StatementType::Compound { items } =>
                for &item in items {
                    if let BlockItem::Statement(child) = item {
                        nodes.push((child, Some(next)));
                    }
                },
            | _ => {},
        }
        next += 1;
    }
    path
}

/// Clang's `mightAddDeclToScope`: whether block item `item` declares
/// something in the enclosing block, rather than in a block of its own.
/// C99: §6.2.1 paragraph 4, pp. 29-30; PDF pp. 41-42.
fn might_add_declaration(scratch: &crate::util::bump::Bump, item: BlockItem<'_>) -> bool {
    let BlockItem::Statement(statement) = item else {
        return true;
    };
    let mut work = ArenaVec::new_in(scratch);
    work.push(statement);
    while let Some(statement) = work.pop() {
        match statement.kind {
            | StatementType::Declaration(_) | StatementType::LocalLabels(_) => return true,
            | StatementType::Label(_, child)
            | StatementType::Case(_, child)
            | StatementType::Default(child) => work.push(child),
            | StatementType::CaseRange(range) => work.push(range.statement),
            | StatementType::Attributed(attributed) => work.push(attributed.statement),
            | _ => {},
        }
    }
    false
}

/// A switch whose controlling expression code generation folds, while its
/// body is analyzed.
#[derive(Clone, Copy)]
pub(in crate::translation_phases::semantic_analysis) struct FoldedSwitch<'tu> {
    /// The promoted controlling value.
    value:     Integer,
    switch:    &'tu Statement<'tu>,
    body:      &'tu Statement<'tu>,
    /// Emission facts recorded when the body began.
    start:     SwitchMark,
    phase:     Phase,
    /// The `break` the selected statements end at, if any.
    end:       Option<&'tu Statement<'tu>>,
    /// A `default` is selected tentatively: a matching case replaces it.
    tentative: bool,
    selected:  Option<&'tu Statement<'tu>>,
    /// The skipped state the selected statements ended in.
    after:     bool,
}

/// Append-only semantic stores at the start of a folded switch body.
#[derive(Clone, Copy)]
struct SwitchMark {
    references:  usize,
    definitions: usize,
    statements:  usize,
}

/// Where the analysis of a folded switch's body stands.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::translation_phases::semantic_analysis) enum Phase {
    /// No label is selected: the body is analyzed as unfolded.
    Searching,
    /// In the selected statements.
    Collecting,
    /// After the `break` the selected statements end at.
    Ended,
    /// Clang emits the switch without folding it.
    Unfolded,
}

/// What collecting one statement of a folded switch's body found.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Collected {
    /// The switch cannot be folded.
    Failure,
    /// The statement was collected; collection continues after it.
    FallThrough,
    /// The statement was skipped while searching for the label, or the
    /// collection ended at a `break` in it.
    Success,
}

/// A block of a folded switch's body being collected.
struct CollectFrame<'tu> {
    block:        &'tu Statement<'tu>,
    items:        &'tu [BlockItem<'tu>],
    next:         usize,
    /// Still searching for the selected label.
    searching:    bool,
    /// The block began inside the selected statements.
    started_live: bool,
    /// An item may declare something: one skipped while searching, which
    /// fails the collection once the label is found, then one collected.
    declared:     bool,
}
