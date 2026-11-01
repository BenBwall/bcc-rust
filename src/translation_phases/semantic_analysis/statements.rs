//! Phase-7 statement constraints, function labels and switch dispatch, and
//! GNU label addresses and computed gotos.
//! C99: §6.8.1-§6.8.6.4, pp. 131-139; PDF pp. 143-151. All walking and
//! jump validation are iterative; this does not construct a backend CFG.
//! GNU extension: GCC "Labels as Values".

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
    JumpId,
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
    translation_phases::{
        SourceVectors,
        parsing::{
            declaration_syntax::{
                Declaration,
                TypeQualifiers,
            },
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
            | StatementWork::AssemblyDone(statement, asm, jumps) =>
                self.finish_assembly(statement, asm, jumps),
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
                self.prepare_folded_switch(index, owner, slot_expression(slot), body, ty);
            },
            | StatementWork::SwitchFinish(index, previous) => {
                self.finish_folded_switch(index);
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
            | StatementWork::ComputedGotoDone(slot) => self.computed_goto_done(slot),
        }
    }

    /// Records a computed goto as a jump to any address-taken label of the
    /// function, to be checked against them once every label is known.
    /// GNU extension: GCC "Labels as Values".
    /// C99: §6.8.6.1p1, p. 137; PDF p. 149 (extended to every target).
    pub(super) fn computed_goto(
        &mut self,
        statement: &'tu Statement<'tu>,
        slot: ExpressionSlot<'tu>,
    ) {
        self.work
            .push(Work::StatementWork(StatementWork::ComputedGotoDone(slot)));
        self.expression_slot(slot);
        // Recovered syntax records no label addresses, so, as for `goto`,
        // it records no jump to check against them.
        if !self.tainted
            && let Some(point) = self.control.point()
        {
            _ = self.control.jump(
                statement,
                ControlTarget::IndirectGoto(point.function),
                point,
            );
            self.statements.indirect_jumps.push(IndirectJump {
                point,
                source: statement.source_vectors,
            });
        }
    }

    /// Converts a computed goto's operand to `const void *`, as Clang does,
    /// with the constraints of simple assignment.
    /// GNU extension: GCC "Labels as Values".
    /// C99: §6.5.16.1p1, p. 92; PDF p. 104.
    fn computed_goto_done(&mut self, slot: ExpressionSlot<'tu>) {
        let Some(e) = slot_expression(slot) else {
            return;
        };
        let info = self.expression_info(e);
        let void = self
            .types
            .scalar(Scalar::Void)
            .qualified(TypeQualifiers::CONST);
        let target = self.types.intern(TypeKind::Pointer(void));
        if self.assignment_compatible(target, info) {
            self.convert(e, target, ConversionKind::Assignment);
        } else if !self.types.unanalyzed(info.ty) {
            self.error(
                SemanticErrorKind::InvalidComputedGoto,
                e.source_vectors,
                None,
                None,
            );
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

    /// A `return` that disagrees with the result type is reported under
    /// `-Wreturn-mismatch`, whose severity follows the identity and the
    /// revision; the function goes on returning as if it agreed.
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
        let name = function.syntax.declarator.identifier().map(|name| name.name);
        if slot.is_none() && !void {
            self.group_diagnostic(
                crate::configuration::DiagnosticGroup::ReturnMismatch,
                SemanticErrorKind::ReturnMismatch,
                source,
                name,
                None,
            );
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
                    // GCC points at the value, Clang and TCC at `return`.
                    let site = if self.context.configuration.diagnostic_format()
                        == crate::configuration::DiagnosticFormat::Gcc
                    {
                        e.source_vectors
                    } else {
                        source
                    };
                    self.group_diagnostic(
                        crate::configuration::DiagnosticGroup::VoidReturnMismatch,
                        SemanticErrorKind::VoidReturnMismatch,
                        site,
                        name,
                        None,
                    );
                }
            } else if !self.assignment_conversion_for(
                result,
                info,
                super::compiler_wording::ConversionSite::Return,
            ) {
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

    /// Records unique label definitions and goto uses in the current function,
    /// returning a use's control jump.
    /// C99: §6.8.1 paragraph 3, p. 132; PDF p. 144.
    /// C99: §6.8.6.1 paragraph 1, p. 137; PDF p. 149.
    pub(super) fn label(
        &mut self,
        statement: &'tu Statement<'tu>,
        name: Identifier,
        definition: bool,
    ) -> Option<JumpId> {
        if self.tainted {
            return None;
        }
        let index = self.label_index(name)?;
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
            self.record_initializing_jump(statement);
            let control = self.control.point().and_then(|point| {
                self.control.jump(
                    statement,
                    ControlTarget::Label(self.statements.labels[index].id),
                    point,
                )
            });
            self.statements.jumps.push(Jump {
                point: self.control.point(),
                control,
                function: self.functions.current.map_or(usize::MAX, |f| f.id),
                label: index,
                source: name,
                statement,
            });
            return control;
        }
        None
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

    /// Records a GNU label-address use as requiring a label definition, and
    /// the label as a possible computed-goto target. Outside a function body
    /// there is no label to name.
    /// GNU extension: GCC manual, "Labels as Values".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Labels-as-Values.html>
    pub(super) fn label_address(&mut self, expression: &'tu Expression<'tu>, name: Identifier) {
        if self.tainted {
            return;
        }
        if self.functions.current.is_none() {
            self.error(
                SemanticErrorKind::LabelAddressOutsideFunction,
                expression.source_vectors,
                Some(name.name),
                None,
            );
            return;
        }
        if let Some(index) = self.label_index(name) {
            _ = self.statements.labels[index].used.get_or_insert(name);
            let id = self.statements.labels[index].id;
            self.control.take_address(expression, id, self.scratch);
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
            || (self.types.unanalyzed(b.ty) && !self.variable_size(b.ty))
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
            address: None,
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
            _ = self.control.jump(statement, target, destination);
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
                && let Some(binding) = jump
                    .control
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
            } else if label.definition.is_some() && label.function == jump.function {
                self.check_cleanup_entry(
                    jump.statement,
                    jump.control,
                    jump.source.source_vectors,
                    jump.point.zip(label.point),
                );
            }
        }
        self.finish_indirect_jumps();
    }

    /// Checks every computed goto against every address-taken label of its
    /// function, as Clang does: a function without any is an error, and so
    /// is entering a statement expression, the scope of a variably modified
    /// identifier or a GNU cleanup variable's scope, reported once per goto
    /// for its first such target; leaving a cleanup variable's scope is an
    /// error too, since an indirect jump runs no cleanup. Each function's
    /// labels inside such a region, and the preorder span of all of its
    /// labels, are collected once, so the work is O(gotos + labels) plus one
    /// check per goto and protected label; plain dispatch tables need no
    /// pairwise checks.
    /// GNU extension: GCC "Labels as Values", "Statement Exprs" and "Common
    /// Variable Attributes", cleanup.
    /// C99: §6.8.6.1p1, p. 137; PDF p. 149.
    fn finish_indirect_jumps(&mut self) {
        // Each function's protected labels: a run of `protected`, found once,
        // with its first and last address-taken label in region preorder.
        let mut runs = ArenaMap::with_hasher_in(FxBuildHasher, self.scratch);
        let mut protected = ArenaVec::new_in(self.scratch);
        for i in 0..self.statements.indirect_jumps.len() {
            let jump = self.statements.indirect_jumps[i];
            let function = jump.point.function;
            let (run, span) = runs
                .entry(function)
                .or_insert_with(|| {
                    let start = protected.len();
                    let mut span: Option<(IndirectTarget, IndirectTarget)> = None;
                    for &label in self.control.taken(function) {
                        let facts = self.control.labels[label.0];
                        if let (Some(point), Some(name)) = (facts.point, facts.name) {
                            let region = self.control.regions[point.region.0];
                            let target = IndirectTarget {
                                region: point.region,
                                preorder: region.preorder,
                                name,
                                source: facts
                                    .definition
                                    .map_or(name.source_vectors, |s| s.source_vectors),
                            };
                            span = Some(match span {
                                | None => (target, target),
                                | Some((first, last)) => (
                                    if target.preorder < first.preorder {
                                        target
                                    } else {
                                        first
                                    },
                                    if target.preorder > last.preorder {
                                        target
                                    } else {
                                        last
                                    },
                                ),
                            });
                            if region.nearest_vm.is_some()
                                || region.nearest_statement_expression.is_some()
                                || region.nearest_cleanup_declaration.is_some()
                            {
                                protected.push(target);
                            }
                        }
                    }
                    (start..protected.len(), span)
                })
                .clone();
            if self.control.taken(function).is_empty() {
                self.error(
                    SemanticErrorKind::IndirectGotoWithoutLabels,
                    jump.source,
                    None,
                    None,
                );
                continue;
            }
            if let Some(target) = self.cleanup_left(jump.point.region, span) {
                self.error(
                    SemanticErrorKind::IndirectJumpCleanupScope,
                    jump.source,
                    None,
                    Some(target.source),
                );
                continue;
            }
            for index in run {
                let IndirectTarget {
                    region: target,
                    name,
                    source: target_source,
                    ..
                } = protected[index];
                #[cfg(test)]
                self.statements
                    .indirect_checks
                    .set(self.statements.indirect_checks.get() + 1);
                if let Some(expression) =
                    self.control.forbidden_expression(jump.point.region, target)
                {
                    self.error(
                        SemanticErrorKind::IndirectJumpIntoStatementExpression,
                        jump.source,
                        Some(name.name),
                        Some(expression),
                    );
                    break;
                }
                if let Some(vm) = self.control.forbidden_vm(jump.point.region, target) {
                    let RegionKind::VmDeclaration(binding) = self.control.regions[vm.0].kind else {
                        unreachable!("nearest VM is a VM region")
                    };
                    self.error(
                        SemanticErrorKind::IndirectJumpIntoVariableScope,
                        jump.source,
                        Some(name.name),
                        Some(self.bindings[binding.0].name.source_vectors),
                    );
                    break;
                }
                if self.cleanup_entered(jump.point.region, target).is_some() {
                    self.error(
                        SemanticErrorKind::IndirectJumpCleanupScope,
                        jump.source,
                        None,
                        Some(target_source),
                    );
                    break;
                }
            }
        }
    }

    /// The first or last address-taken label of `span` when it lies outside
    /// the innermost cleanup scope around `from`: an indirect goto there
    /// would leave the scope without its cleanup. A scope is a contiguous
    /// run of region preorder, so the span's ends decide.
    fn cleanup_left(
        &self,
        from: RegionId,
        span: Option<(IndirectTarget, IndirectTarget)>,
    ) -> Option<IndirectTarget> {
        let cleanup = self.control.regions[from.0].nearest_cleanup_declaration?;
        let (first, last) = span?;
        let scope = self.control.regions[cleanup.0];
        if first.preorder < scope.preorder {
            Some(first)
        } else {
            (last.preorder >= scope.subtree_end).then_some(last)
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
            if let Some((from, to)) = points {
                self.check_cleanup_dispatch(from, to, source);
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
                    address: None,
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
        let Some(lower) = lower.and_then(|e| self.case_value(e, bits, signed)) else {
            return;
        };
        let range = upper.is_some();
        let Some(upper) = upper.map_or(Some(lower), |e| self.case_value(e, bits, signed)) else {
            return;
        };
        if !range {
            self.folded_case(statement, lower);
        }
        if self.wide.compare(lower, upper).is_gt() {
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

    /// The value of case label expression `e`, converted to the promoted
    /// controlling type of `bits` width and `signed`ness. `RequireConstant`
    /// checks the expression. This retains the same additional arithmetic
    /// forms accepted by evaluate; keeping only strict ICEs would silently
    /// drop the switch dispatch edge.
    /// C99: §6.8.4.2 paragraphs 3 and 5, p. 134; PDF p. 146; §6.6
    /// paragraph 10, p. 96; PDF p. 108.
    pub(super) fn case_value(
        &self,
        e: &'tu Expression<'tu>,
        bits: u32,
        signed: bool,
    ) -> Option<super::Integer> {
        let info = self.expression_info(e);
        (info.ice
            || (!info.atomic_cast()
                && info.constant == super::ConstantClass::Arithmetic
                && (self.context.configuration.gnu_extensions() || info.folded_address())))
        .then_some(info.integer)
        .flatten()
        .map(|v| self.wide.convert(v, bits, signed))
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
                .filter(|case| self.wide.compare(case.lower, case.upper).is_le()),
        );
        cases.sort_unstable_by(|a, b| {
            self.wide
                .compare(a.lower, b.lower)
                .then(a.ordinal.cmp(&b.ordinal))
        });
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
                        dispatch: self
                            .wide
                            .compare(case.lower, case.upper)
                            .is_le()
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
                && self.wide.compare(case.lower, prior.upper).is_le()
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
            if previous.is_none_or(|p| self.wide.compare(case.upper, p.upper).is_gt()) {
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
    ComputedGotoDone(ExpressionSlot<'tu>),
    /// A GNU `asm` statement whose operand expressions are typed.
    AssemblyDone(
        &'tu Statement<'tu>,
        &'tu crate::translation_phases::parsing::Asm<'tu>,
        &'tu [JumpId],
    ),
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
    /// This use's control jump, whose transition the scope checks read: an
    /// `asm goto` statement has one per label.
    control:   Option<JumpId>,
    statement: &'tu Statement<'tu>,
    function:  usize,
    label:     usize,
    source:    Identifier,
}

/// A computed goto and where it is, checked against every address-taken
/// label of its function. GNU extension: GCC "Labels as Values".
#[derive(Clone, Copy)]
struct IndirectJump {
    point:  ControlPoint,
    source: SourceVectors,
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
    /// Computed-goto checks against protected labels.
    #[cfg(test)]
    pub(super) indirect_checks: std::cell::Cell<usize>,
    pub(super) loops: usize,
    pub(super) switch: Option<usize>,
    labels: ArenaVec<'s, Label>,
    label_names: ArenaMap<'s, (usize, StringCacheId), usize>,
    jumps: ArenaVec<'s, Jump<'tu>>,
    indirect_jumps: ArenaVec<'s, IndirectJump>,
    switches: ArenaVec<'s, Switch<'tu, 's>>,
    case_count: usize,
}

impl<'s> State<'_, 's> {
    pub(super) fn new(scratch: &'s Bump) -> Self {
        Self {
            #[cfg(test)]
            ancestry_steps: std::cell::Cell::new(0),
            #[cfg(test)]
            indirect_checks: std::cell::Cell::new(0),
            loops: 0,
            switch: None,
            labels: ArenaVec::new_in(scratch),
            label_names: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            jumps: ArenaVec::new_in(scratch),
            indirect_jumps: ArenaVec::new_in(scratch),
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

/// An address-taken label as a computed goto may reach it.
#[derive(Clone, Copy)]
struct IndirectTarget {
    region:   RegionId,
    preorder: usize,
    name:     Identifier,
    /// The label's definition, for the diagnostic's second location.
    source:   SourceVectors,
}
