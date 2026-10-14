//! Phase-7 statement constraints, function labels and switch dispatch.
//! C99: §6.8.1-§6.8.6.4, pp. 131-139; PDF pp. 143-151. All walking and
//! jump validation are iterative; this does not construct a backend CFG.

use rustc_hash::FxBuildHasher;

use super::{
    super::parsing::syntax::ConstantExpressionSlot,
    Analyzer,
    BindingId,
    BindingKind,
    CaseFacts,
    CleanupKind,
    ControlPoint,
    ControlTarget,
    ExprId,
    FunctionId,
    LabelFacts,
    LabelId,
    LoopFacts,
    RegionId,
    RegionKind,
    ScopeKind,
    SemanticErrorKind,
    SwitchFacts,
    SwitchId,
    Transition,
    Work,
    collection::Collection,
    expressions::ConversionKind,
    scopes::Namespace,
    types::{
        Scalar,
        TypeId,
        TypeKind,
    },
};
use crate::{
    configuration::CStandard,
    translation_phases::{
        SourceVectors,
        parsing::{
            declaration_syntax::Declaration,
            syntax::{
                Expression,
                ExpressionSlot,
                Identifier,
                Statement,
                StorageClass,
            },
        },
    },
    util::{
        arena_list::ArenaList,
        bump::{
            ArenaMap,
            ArenaVec,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Completes statement scope, switch promotion, for declaration and return
    /// checks.
    /// C99: §6.8.4 paragraph 3, p. 133; PDF p. 145.
    /// C99: §6.8.4.2 paragraph 5, p. 134; PDF p. 146.
    /// C99: §6.8.5 paragraphs 3-5, p. 135; PDF p. 147.
    /// C99: §6.8.6.4 paragraphs 1-3, p. 139; PDF p. 151.
    pub(super) fn statement_work(&mut self, work: StatementWork<'tu>) {
        match work {
            | StatementWork::RetainTail(statement) => {
                self.statement_effects(statement);
                self.retain_statement_tail(statement);
            },
            | StatementWork::RestoreStatementExpression { region, previous } => {
                if let Some(region) = region {
                    self.control.regions[region.0].kind =
                        RegionKind::StatementExpression(ExprId(self.expressions.len()));
                }
                self.control.close_to(previous);
            },
            | StatementWork::NormalExit(statement, destination) => {
                self.control.normal_exit(statement, destination);
            },
            | StatementWork::FinishStatement(statement) => {
                if matches!(
                    statement.kind,
                    crate::translation_phases::parsing::syntax::StatementType::Compound { .. }
                ) && let Some(id) = self.control.statement_id(statement)
                {
                    self.control
                        .normal_exit(statement, self.control.statements[id.0].entry.region);
                }
            },
            | StatementWork::Substatement(statement) => {
                let outside = self.control.current;
                self.enter(ScopeKind::Block);
                self.work.push(Work::PopScope);
                if let Some(outside) = outside {
                    self.work
                        .push(Work::StatementWork(StatementWork::NormalExit(
                            statement, outside,
                        )));
                }
                self.work.push(Work::Statement(statement, false));
            },
            | StatementWork::LoopBody(owner, statement) => {
                // C99 §6.8.6.2p1, §6.8.6.3p1 (p. 138; PDF p. 150):
                // jump context belongs to the body. GNU "Statement Exprs"
                // sends header jumps to an enclosing body, if one exists.
                // <https://gcc.gnu.org/onlinedocs/gcc/Statement-Exprs.html>
                self.statements.loops += 1;
                if let Some(id) = self.control.statement_id(owner) {
                    let header = self.control.point().expect("loop has a region");
                    let exit = self.control.statements[id.0].entry;
                    let index = self.control.loops.len();
                    self.control.loop_body(statement, index);
                    self.control.loops.push(LoopFacts {
                        statement: id,
                        header,
                        body: header,
                        step: header,
                        exit,
                    });
                    self.control.targets.push((id, Some(index)));
                }
                self.work
                    .push(Work::StatementWork(StatementWork::LeaveLoop));
                self.work
                    .push(Work::StatementWork(StatementWork::Substatement(statement)));
            },
            | StatementWork::SwitchReady(owner, slot, body) => {
                let ty = if let Some(e) = slot_expression(slot) {
                    let info = self.expression_info(e);
                    let ty = self.converted(info);
                    if self.types.unanalyzed(ty) {
                        self.types.unknown()
                    } else {
                        self.promote(info, ty)
                    }
                } else {
                    self.types.unknown()
                };
                if let Some(e) = slot_expression(slot) {
                    self.convert(e, ty, ConversionKind::Arithmetic);
                }
                let index = self.statements.switches.len();
                self.statements.switches.push(Switch {
                    statement: owner,
                    point: self.control.point(),
                    ty,
                    default: None,
                    cases: self.scratch.alloc(Collection::new()),
                });
                self.work
                    .push(Work::StatementWork(StatementWork::SwitchFinish(
                        index,
                        self.statements.switch,
                    )));
                self.statements.switch = Some(index);
                if let Some(id) = self.control.statement_id(owner) {
                    let nearest_loop = self.control.targets.last().and_then(|t| t.1);
                    self.control.targets.push((id, nearest_loop));
                }
                self.work
                    .push(Work::StatementWork(StatementWork::Substatement(body)));
            },
            | StatementWork::SwitchFinish(index, previous) => {
                self.finish_switch(index);
                self.statements.switch = previous;
                _ = self.control.targets.pop();
            },
            | StatementWork::ForDeclarationDone(d) => {
                // C99 §6.8.5p3, p. 135; PDF p. 147. C23 relaxes the
                // restriction to object declarations in the for initializer.
                if d.init_declarators.is_empty() {
                    self.context.report_extension(
                        crate::configuration::Feature::ForNonVariableDeclarations,
                        "non-variable declaration in 'for' loop",
                        d.source_vectors,
                    );
                }
                if d.declaration_specifiers
                    .storage_class
                    .is_some_and(|s| !matches!(s, StorageClass::Auto | StorageClass::Register))
                {
                    return;
                }
                for init in d.init_declarators {
                    if let Some(name) = init.declarator.identifier()
                        && let Some(entry) = self.lookup(Namespace::Ordinary, name.name)
                        && self.bindings[entry.binding].kind != BindingKind::Object
                    {
                        self.error(
                            SemanticErrorKind::InvalidForDeclaration,
                            name.source_vectors,
                            Some(name.name),
                            None,
                        );
                    }
                }
            },
            | StatementWork::LeaveLoop => {
                self.statements.loops = self.statements.loops.saturating_sub(1);
                _ = self.control.targets.pop();
            },
            | StatementWork::CaseDone(statement, lower, upper, source) =>
                self.case_done(statement, lower, upper, source),
            | StatementWork::ReturnDone(slot, source) => self.return_done(slot, source),
        }
    }

    /// C99: §6.8.5p3, p. 135; PDF p. 147.
    pub(super) fn for_declaration(&mut self, d: &'tu Declaration<'tu>) {
        if d.recovered {
            return;
        }
        self.work
            .push(Work::StatementWork(StatementWork::ForDeclarationDone(d)));
        if d.declaration_specifiers
            .storage_class
            .is_some_and(|s| !matches!(s, StorageClass::Auto | StorageClass::Register))
        {
            self.error(
                SemanticErrorKind::InvalidForDeclaration,
                d.source_vectors,
                None,
                None,
            );
        }
    }

    /// C99: §6.8.6.4p1,p3, p. 139; PDF p. 151.
    fn return_done(&mut self, slot: Option<ExpressionSlot<'tu>>, source: SourceVectors) {
        let Some(function) = self.functions.current else {
            return;
        };
        let result = function.result;
        if self.types.unanalyzed(result) {
            return;
        }
        let void = matches!(
            self.types.nodes[result.index],
            TypeKind::Scalar(Scalar::Void)
        );
        if slot.is_none() && !void {
            let kind = if self.context.configuration.standard() < CStandard::C99
                || self.context.configuration.gnu_extensions()
            {
                SemanticErrorKind::MissingReturnValueWarning
            } else {
                SemanticErrorKind::MissingReturnValue
            };
            self.error(kind, source, None, None);
        } else if let Some(slot) = slot {
            let Some(e) = slot_expression(slot) else {
                return;
            };
            let info = self.expression_info(e);
            if void {
                // GNU C accepts returning a void expression as an extension.
                if matches!(
                    self.types.nodes[info.ty.index],
                    TypeKind::Scalar(Scalar::Void)
                ) && self
                    .context
                    .configuration
                    .accepts(crate::configuration::Feature::VoidExpressionReturn)
                {
                    self.context.report_extension(
                        crate::configuration::Feature::VoidExpressionReturn,
                        "return with a void expression",
                        source,
                    );
                } else if !self.types.unanalyzed(info.ty) {
                    self.error(SemanticErrorKind::VoidReturnValue, source, None, None);
                }
            } else if !self.assignment_compatible(result, info) {
                self.error(
                    SemanticErrorKind::InvalidReturnConversion,
                    e.source_vectors,
                    None,
                    None,
                );
            } else {
                self.convert(e, result.unqualified(), ConversionKind::Assignment);
            }
        }
    }

    /// Retains the expression ancestry after its lexical block closes, so
    /// forward gotos can be checked after their labels have been resolved.
    /// GNU extension: GCC "Statement Exprs" prohibits entering an expression.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Statement-Exprs.html>
    /// C99: §6.8.6.1p1, p. 137; PDF p. 149 (extended goto constraint).
    pub(super) fn enter_statement_expression(&mut self, source: SourceVectors) {
        self.open_block_effects();
        let previous = self.control.current;
        let region = self.control.open(
            self.scope,
            RegionKind::StatementExpression(ExprId(usize::MAX)),
            CleanupKind::None,
            source,
        );
        self.work.push(Work::StatementWork(
            StatementWork::RestoreStatementExpression { region, previous },
        ));
    }

    /// Records unique label definitions and goto uses in the current function.
    /// C99: §6.8.1 paragraph 3, p. 132; PDF p. 144.
    /// C99: §6.8.6.1 paragraph 1, p. 137; PDF p. 149.
    pub(super) fn label(
        &mut self,
        statement: &'tu Statement<'tu>,
        name: Identifier,
        definition: bool,
    ) {
        if self.tainted {
            return;
        }
        let Some(index) = self.label_index(name) else {
            return;
        };
        if definition {
            if let Some(previous) = self.statements.labels[index].definition {
                self.error(
                    SemanticErrorKind::DuplicateLabel,
                    name.source_vectors,
                    Some(name.name),
                    Some(previous.source_vectors),
                );
            } else {
                self.statements.labels[index].definition = Some(name);
                self.statements.labels[index].point = self.control.point();
                let id = self.statements.labels[index].id;
                self.control.labels[id.0].definition = Some(statement);
                self.control.labels[id.0].point = self.control.point();
                if let Some(statement_id) = self.control.statement_id(statement) {
                    self.control.statements[statement_id.0].label = Some(id);
                }
            }
        } else {
            _ = self.statements.labels[index].used.get_or_insert(name);
            self.statements.jumps.push(Jump {
                point: self.control.point(),
                function: self.functions.current.map_or(usize::MAX, |f| f.id),
                label: index,
                source: name,
                statement,
            });
            if let Some(point) = self.control.point() {
                self.control.jump(
                    statement,
                    ControlTarget::Label(self.statements.labels[index].id),
                    point,
                );
            }
        }
    }

    /// GNU local labels participate in lexical lookup; ordinary labels have
    /// function scope. Extensions are already reported by parser owners.
    /// C99: §6.2.1p3, p. 29; PDF p. 41; §6.8.1p3, p. 132; PDF p. 144.
    /// GNU extension: GCC manual, "Local Labels".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Local-Labels.html>
    pub(super) fn local_labels(&mut self, names: ArenaList<'tu, Identifier>) {
        if self.tainted || self.functions.current.is_none() {
            return;
        }
        for &name in names {
            if let Some(entry) = self.lookup(Namespace::Label, name.name)
                && entry.scope == self.scope
            {
                self.error(
                    SemanticErrorKind::DuplicateLocalLabel,
                    name.source_vectors,
                    Some(name.name),
                    Some(entry.name.source_vectors),
                );
                continue;
            }
            let index = self.statements.labels.len();
            let id = self.new_control_label(name, true);
            self.statements.labels.push(Label {
                id,
                point: None,
                local: true,
                function: self.functions.current.map_or(usize::MAX, |f| f.id),
                name,
                definition: None,
                used: None,
            });
            self.install(name, Namespace::Label, index);
        }
    }

    /// Resolves GNU local labels before function-scoped ordinary labels.
    /// GNU extension: GCC manual, "Local Labels".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Local-Labels.html>
    /// C99: §6.2.1 paragraph 3, p. 29; PDF p. 41.
    fn label_index(&mut self, name: Identifier) -> Option<usize> {
        if let Some(entry) = self.lookup(Namespace::Label, name.name) {
            return Some(entry.binding);
        }
        let function = self.functions.current?.id;
        if let Some(&index) = self.statements.label_names.get(&(function, name.name)) {
            return Some(index);
        }
        let index = self.statements.labels.len();
        let id = self.new_control_label(name, false);
        self.statements.labels.push(Label {
            id,
            point: None,
            local: false,
            function,
            name,
            definition: None,
            used: None,
        });
        _ = self
            .statements
            .label_names
            .insert((function, name.name), index);
        Some(index)
    }

    /// Records a GNU label-address use as requiring a label definition.
    /// GNU extension: GCC manual, "Labels as Values".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Labels-as-Values.html>
    pub(super) fn label_address(&mut self, name: Identifier) {
        if !self.tainted
            && let Some(index) = self.label_index(name)
        {
            _ = self.statements.labels[index].used.get_or_insert(name);
        }
    }

    /// Persistent paths include every variably modified identifier, including
    /// pointers and typedefs, not just VLA objects.
    /// C99: §6.8.6.1p1, p. 137; PDF p. 149; §6.8.4.2p2, p. 134; PDF p. 146.
    pub(super) fn record_vm(&mut self, binding: usize) {
        let b = self.bindings[binding];
        if self.scopes[b.scope].kind == ScopeKind::Prototype
            || self.functions.current.is_none()
            || !self.variably_modified(b.ty)
            || self.types.unanalyzed(b.ty)
        {
            return;
        }
        let cleanup = if b.kind == BindingKind::Object
            && b.duration == super::Duration::Automatic
            && matches!(self.types.nodes[b.ty.index], TypeKind::Array(..))
        {
            CleanupKind::AutomaticVla(BindingId(binding))
        } else {
            CleanupKind::None
        };
        _ = self.control.open(
            self.scope,
            RegionKind::VmDeclaration(BindingId(binding)),
            cleanup,
            b.name.source_vectors,
        );
    }

    fn new_control_label(&mut self, name: Identifier, local: bool) -> LabelId {
        let id = LabelId(self.control.labels.len());
        self.control.labels.push(LabelFacts {
            id,
            name: Some(name),
            local,
            function: FunctionId(self.functions.current.map_or(usize::MAX, |f| f.id)),
            definition: None,
            point: None,
        });
        id
    }

    pub(super) fn control_jump(&mut self, statement: &'tu Statement<'tu>, is_continue: bool) {
        if let Some(&(owner, nearest_loop)) = self.control.targets.last() {
            let (target, destination) = if is_continue {
                let Some(index) = nearest_loop else {
                    return;
                };
                let facts = self.control.loops[index];
                (ControlTarget::LoopContinue(facts.statement), facts.step)
            } else if let Some(index) = nearest_loop
                && self.control.loops[index].statement == owner
            {
                let facts = self.control.loops[index];
                (ControlTarget::LoopBreak(owner), facts.exit)
            } else {
                (
                    ControlTarget::SwitchBreak(owner),
                    self.control.statements[owner.0].entry,
                )
            };
            self.control.jump(statement, target, destination);
        }
    }

    /// Checks label definitions and prohibits jumps into variably modified
    /// scopes.
    /// GNU extension: GCC manual, "Local Labels".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Local-Labels.html>
    /// C99: §6.8.6.1 paragraph 1, p. 137; PDF p. 149.
    pub(super) fn finish_labels(&mut self) {
        for i in 0..self.statements.labels.len() {
            let label = self.statements.labels[i];
            if label.definition.is_none()
                && let Some(usage) = label.used.or_else(|| label.local.then_some(label.name))
            {
                self.error(
                    if label.local {
                        SemanticErrorKind::UndefinedLocalLabel
                    } else {
                        SemanticErrorKind::UndefinedLabel
                    },
                    usage.source_vectors,
                    Some(label.name.name),
                    None,
                );
            }
        }
        for i in 0..self.statements.jumps.len() {
            let jump = self.statements.jumps[i];
            let label = self.statements.labels[jump.label];
            #[cfg(test)]
            {
                self.statements
                    .ancestry_steps
                    .set(self.statements.ancestry_steps.get() + 1);
                self.review_step(0);
            }
            if label.definition.is_some()
                && label.function == jump.function
                && let Some(source) = jump.point.zip(label.point).and_then(|(from, to)| {
                    self.control.forbidden_expression(from.region, to.region)
                })
            {
                self.error(
                    SemanticErrorKind::JumpIntoStatementExpression,
                    jump.source.source_vectors,
                    Some(jump.source.name),
                    Some(source),
                );
            }
            if label.definition.is_some()
                && label.function == jump.function
                && let Some(binding) = self
                    .control
                    .statement_id(jump.statement)
                    .and_then(|id| self.control.statements[id.0].jump)
                    .and_then(|id| {
                        self.control
                            .ancestry
                            .as_ref()
                            .expect("control finalized")
                            .first_entered_vm(
                                &self.control.regions,
                                self.control.jumps[id.0].transition,
                            )
                    })
                    .and_then(|region| match self.control.regions[region.0].kind {
                        | RegionKind::VmDeclaration(binding) => Some(binding.0),
                        | _ => None,
                    })
            {
                self.error(
                    SemanticErrorKind::JumpIntoVariableScope,
                    jump.source.source_vectors,
                    Some(jump.source.name),
                    Some(self.bindings[binding].name.source_vectors),
                );
            }
        }
    }

    /// C99: §6.8.1p2, p. 131; PDF p. 143; §6.8.4.2p2-3,p5,
    /// p. 134; PDF p. 146. GNU ranges use inclusive converted intervals.
    /// GNU extension: GCC manual, "Case Ranges".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Case-Ranges.html>
    pub(super) fn case_label(
        &mut self,
        statement: &'tu Statement<'tu>,
        lower: ConstantExpressionSlot<'tu>,
        upper: Option<ConstantExpressionSlot<'tu>>,
        source: SourceVectors,
    ) {
        self.switch_label(statement, source);
        let parsed = |slot| match slot {
            | ConstantExpressionSlot::Parsed(e) => Some(e.expression()),
            | ConstantExpressionSlot::Missing(_) => None,
        };
        let lower = parsed(lower);
        let upper = upper.and_then(parsed);
        self.work.push(Work::StatementWork(StatementWork::CaseDone(
            statement, lower, upper, source,
        )));
        for expression in [upper, lower].into_iter().flatten() {
            self.work.push(Work::RequireConstant(
                expression,
                None,
                self.semantic_errors,
            ));
            self.work.push(Work::Eval(expression));
            self.work.push(Work::Expression(expression));
        }
    }

    /// Checks switch-label placement and entry into variably modified scopes.
    /// C99: §6.8.1 paragraph 2, p. 131; PDF p. 143.
    /// C99: §6.8.4.2 paragraph 2, p. 134; PDF p. 146.
    pub(super) fn switch_label(&mut self, statement: &'tu Statement<'tu>, source: SourceVectors) {
        if let Some(index) = self.statements.switch {
            let points = self.statements.switches[index]
                .point
                .zip(self.control.point());
            if let Some(expression) = points
                .and_then(|(from, to)| self.control.forbidden_expression(from.region, to.region))
            {
                self.error(
                    SemanticErrorKind::SwitchIntoStatementExpression,
                    source,
                    None,
                    Some(expression),
                );
            }
            if let Some((from, to)) = points
                && let Some(region) = self.control.forbidden_vm(from.region, to.region)
            {
                let RegionKind::VmDeclaration(binding) = self.control.regions[region.0].kind else {
                    unreachable!("nearest VM is a VM region")
                };
                let diagnostic = self.context.pending_errors.len();
                self.error(
                    SemanticErrorKind::SwitchIntoVariableScope,
                    source,
                    None,
                    Some(self.bindings[binding.0].name.source_vectors),
                );
                if self.context.pending_errors.len() > diagnostic {
                    self.control
                        .deferred_vm_labels
                        .push((diagnostic, from.region, to.region));
                }
            }
            if let Some(point) = self.control.point() {
                let id = LabelId(self.control.labels.len());
                self.control.labels.push(LabelFacts {
                    id,
                    name: None,
                    function: point.function,
                    local: false,
                    definition: Some(statement),
                    point: Some(point),
                });
                if let Some(statement_id) = self.control.statement_id(statement) {
                    self.control.statements[statement_id.0].label = Some(id);
                }
                if matches!(
                    statement.kind,
                    crate::translation_phases::parsing::syntax::StatementType::Default(_)
                ) {
                    self.statements.switches[index].default = Some(id);
                }
            }
        } else {
            self.error(SemanticErrorKind::CaseOutsideSwitch, source, None, None);
        }
    }

    /// Converts case constants to the promoted switch type and checks GNU
    /// ranges.
    /// GNU extension: GCC manual, "Case Ranges".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Case-Ranges.html>
    /// C99: §6.8.4.2 paragraph 5, p. 134; PDF p. 146.
    fn case_done(
        &mut self,
        statement: &'tu Statement<'tu>,
        lower: Option<&'tu Expression<'tu>>,
        upper: Option<&'tu Expression<'tu>>,
        source: SourceVectors,
    ) {
        if self.tainted {
            return;
        }
        let Some(index) = self.statements.switch else {
            return;
        };
        let switch = self.statements.switches[index];
        let Some((bits, signed)) = self.integer_type(switch.ty) else {
            return;
        };
        let value = |e| {
            let info = self.expression_info(e);
            info.ice
                .then_some(info.integer)
                .flatten()
                .map(|v| v.cast(bits, signed))
        };
        let Some(lower) = lower.and_then(value) else {
            return;
        };
        let range = upper.is_some();
        let Some(upper) = upper.map_or(Some(lower), value) else {
            return;
        };
        if lower.order_key() > upper.order_key() {
            self.error(SemanticErrorKind::EmptyCaseRange, source, None, None);
            // GNU Case Ranges: no dispatch edge, but retain the label for
            // fallthrough. <https://gcc.gnu.org/onlinedocs/gcc/Case-Ranges.html>
        }
        switch.cases.push(
            self.scratch,
            Case {
                lower,
                upper,
                source,
                ordinal: self.statements.case_count,
                statement,
                range,
                point: self.control.point(),
            },
        );
        self.statements.case_count += 1;
    }

    /// Sort once at switch exit rather than scanning preceding cases for each
    /// label. This keeps long lists O(n log n), including overlapping ranges.
    /// C99: §6.8.4.2p3, p. 134; PDF p. 146.
    fn finish_switch(&mut self, index: usize) {
        let list = self.statements.switches[index]
            .cases
            .finish(self.scratch, self.scratch);
        let mut cases = ArenaVec::new_in(self.scratch);
        cases.extend(
            list.iter()
                .copied()
                .filter(|case| case.lower.order_key() <= case.upper.order_key()),
        );
        cases.sort_unstable_by_key(|c| (c.lower.order_key(), c.ordinal));
        let mut previous: Option<Case<'tu>> = None;
        if let Some(statement) = self
            .control
            .statement_id(self.statements.switches[index].statement)
            && let Some(dispatch) = self.statements.switches[index].point
            && let Some(control) =
                slot_expression(match self.statements.switches[index].statement.kind {
                    | crate::translation_phases::parsing::syntax::StatementType::Switch {
                        condition_expression,
                        ..
                    } => condition_expression,
                    | _ => unreachable!("switch syntax"),
                })
        {
            let mut facts = ArenaVec::new_in(self.scratch);
            for case in list {
                if let Some(id) = self.control.statement_id(case.statement)
                    && let Some(label) = self.control.statements[id.0].label
                    && let Some(point) = case.point
                {
                    facts.push(CaseFacts {
                        label,
                        statement: case.statement,
                        point,
                        lower: case.lower,
                        upper: case.upper,
                        range: case.range,
                        ordinal: facts.len(),
                        dispatch: (case.lower.order_key() <= case.upper.order_key())
                            .then(|| Transition::empty(dispatch.region)),
                    });
                }
            }
            let mut sorted = ArenaVec::new_in(self.scratch);
            let mut ordinals = ArenaMap::with_hasher_in(FxBuildHasher, self.scratch);
            for (ordinal, fact) in facts.iter().enumerate() {
                _ = ordinals.insert(std::ptr::from_ref(fact.statement).addr(), ordinal);
            }
            sorted.extend(cases.iter().filter_map(|case| {
                ordinals
                    .get(&std::ptr::from_ref(case.statement).addr())
                    .copied()
            }));
            let id = SwitchId(self.control.switches.len());
            let start = self.control.cases.len();
            self.control.cases.extend_from_slice(&facts);
            self.control.case_runs.push(start..self.control.cases.len());
            let exit = self.control.statements[statement.0].entry;
            self.control.switches.push(SwitchFacts {
                statement,
                control,
                ty: self.statements.switches[index].ty,
                dispatch,
                exit,
                default: self.statements.switches[index].default,
                default_dispatch: None,
                no_match: Transition::empty(dispatch.region),
                cases: &[],
                sorted: self.types.tu.alloc_slice_copy(&sorted),
            });
            self.control.statements[statement.0].switch = Some(id);
        }
        for case in cases {
            if let Some(prior) = previous
                && case.lower.order_key() <= prior.upper.order_key()
            {
                let (later, earlier) = if case.ordinal > prior.ordinal {
                    (case, prior)
                } else {
                    (prior, case)
                };
                self.error(
                    SemanticErrorKind::DuplicateCase,
                    later.source,
                    None,
                    Some(earlier.source),
                );
            }
            if previous.is_none_or(|p| case.upper.order_key() > p.upper.order_key()) {
                previous = Some(case);
            }
        }
    }
}

/// Resumable statement scope, switch, loop and return checks.
/// C99: §6.8.4 paragraph 3, p. 133; PDF p. 145.
/// C99: §6.8.5 paragraphs 3-5, p. 135; PDF p. 147.
/// C99: §6.8.6.4 paragraphs 1-3, p. 139; PDF p. 151.
#[derive(Clone, Copy)]
pub(super) enum StatementWork<'tu> {
    RestoreStatementExpression {
        region:   Option<RegionId>,
        previous: Option<RegionId>,
    },
    /// C99: §6.8.4p3, p. 133; PDF p. 145; §6.8.5p5, p. 135;
    /// PDF p. 147. Associated substatements are blocks, including expressions.
    Substatement(&'tu Statement<'tu>),
    SwitchReady(
        &'tu Statement<'tu>,
        ExpressionSlot<'tu>,
        &'tu Statement<'tu>,
    ),
    SwitchFinish(usize, Option<usize>),
    LeaveLoop,
    LoopBody(&'tu Statement<'tu>, &'tu Statement<'tu>),
    NormalExit(&'tu Statement<'tu>, RegionId),
    FinishStatement(&'tu Statement<'tu>),
    RetainTail(&'tu Statement<'tu>),
    ForDeclarationDone(&'tu Declaration<'tu>),
    CaseDone(
        &'tu Statement<'tu>,
        Option<&'tu Expression<'tu>>,
        Option<&'tu Expression<'tu>>,
        SourceVectors,
    ),
    ReturnDone(Option<ExpressionSlot<'tu>>, SourceVectors),
}

/// A function-scoped label or a GNU local label and its definition.
/// GNU extension: GCC manual, "Local Labels".
/// <https://gcc.gnu.org/onlinedocs/gcc/Local-Labels.html>
/// C99: §6.2.1 paragraph 3, p. 29; PDF p. 41.
/// C99: §6.8.1 paragraph 3, p. 132; PDF p. 144.
#[derive(Clone, Copy)]
struct Label {
    id:         LabelId,
    point:      Option<ControlPoint>,
    local:      bool,
    function:   usize,
    name:       Identifier,
    definition: Option<Identifier>,
    used:       Option<Identifier>,
}

/// A goto target and the variably modified scope path at its use.
/// C99: §6.8.6.1 paragraph 1, p. 137; PDF p. 149.
#[derive(Clone, Copy)]
struct Jump<'tu> {
    point:     Option<ControlPoint>,
    statement: &'tu Statement<'tu>,
    function:  usize,
    label:     usize,
    source:    Identifier,
}

/// A converted switch case value or inclusive GNU case interval.
/// GNU extension: GCC manual, "Case Ranges".
/// <https://gcc.gnu.org/onlinedocs/gcc/Case-Ranges.html>
/// C99: §6.8.4.2 paragraph 3, p. 134; PDF p. 146.
/// C99: §6.8.4.2 paragraph 5, p. 134; PDF p. 146.
#[derive(Clone, Copy)]
struct Case<'tu> {
    statement: &'tu Statement<'tu>,
    point:     Option<ControlPoint>,
    range:     bool,
    lower:     super::Integer,
    upper:     super::Integer,
    source:    SourceVectors,
    ordinal:   usize,
}

/// Promoted controlling type and scope/label state for one switch.
/// C99: §6.8.4.2 paragraphs 2-5, p. 134; PDF p. 146.
#[derive(Clone, Copy)]
struct Switch<'tu, 's> {
    statement: &'tu Statement<'tu>,
    point:     Option<ControlPoint>,
    default:   Option<LabelId>,
    ty:        TypeId,
    cases:     &'s Collection<'s, Case<'tu>>,
}

pub(super) struct State<'tu, 's> {
    #[cfg(test)]
    pub(super) ancestry_steps: std::cell::Cell<usize>,
    pub(super) loops:          usize,
    pub(super) switch:         Option<usize>,
    labels:                    ArenaVec<'s, Label>,
    label_names:               ArenaMap<'s, (usize, StringCacheId), usize>,
    jumps:                     ArenaVec<'s, Jump<'tu>>,
    switches:                  ArenaVec<'s, Switch<'tu, 's>>,
    case_count:                usize,
}

impl<'s> State<'_, 's> {
    pub(super) fn new(scratch: &'s Bump) -> Self {
        Self {
            #[cfg(test)]
            ancestry_steps: std::cell::Cell::new(0),
            loops: 0,
            switch: None,
            labels: ArenaVec::new_in(scratch),
            label_names: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            jumps: ArenaVec::new_in(scratch),
            switches: ArenaVec::new_in(scratch),
            case_count: 0,
        }
    }
}

pub(super) fn slot_expression(mut slot: ExpressionSlot<'_>) -> Option<&Expression<'_>> {
    loop {
        match slot {
            | ExpressionSlot::Parsed(e) => return Some(e),
            | ExpressionSlot::Selection(header) => slot = header.expression?,
            | ExpressionSlot::Missing(_) => return None,
        }
    }
}
