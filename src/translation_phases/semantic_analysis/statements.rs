//! Phase-7 statement constraints, function labels and switch dispatch.
//! C99: §6.8.1-§6.8.6.4, pp. 131-139; PDF pp. 143-151. All walking and
//! jump validation are iterative; this does not construct a backend CFG.

use super::{
    super::parsing::syntax::ConstantExpressionSlot,
    Analyzer,
    ArenaList,
    ArenaMap,
    ArenaVec,
    BindingKind,
    Bump,
    CStandard,
    Collection,
    ConversionKind,
    Declaration,
    Expression,
    ExpressionSlot,
    FxBuildHasher,
    Identifier,
    Namespace,
    Scalar,
    ScopeKind,
    SemanticErrorKind,
    SourceVectors,
    Statement,
    StorageClass,
    StringCacheId,
    TypeId,
    TypeKind,
    Work,
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
            | StatementWork::Substatement(statement) => {
                self.enter(ScopeKind::Block);
                self.work.push(Work::PopScope);
                self.work.push(Work::Statement(statement, false));
            },
            | StatementWork::SwitchReady(slot, body) => {
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
                    ty,
                    vm: self.statements.vm,
                    cases: self.scratch.alloc(Collection::new()),
                });
                self.work
                    .push(Work::StatementWork(StatementWork::SwitchFinish(
                        index,
                        self.statements.switch,
                    )));
                self.statements.switch = Some(index);
                self.work
                    .push(Work::StatementWork(StatementWork::Substatement(body)));
            },
            | StatementWork::SwitchFinish(index, previous) => {
                self.finish_switch(index);
                self.statements.switch = previous;
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
            | StatementWork::LeaveLoop =>
                self.statements.loops = self.statements.loops.saturating_sub(1),
            | StatementWork::CaseDone(lower, upper, source) => self.case_done(lower, upper, source),
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

    /// Records unique label definitions and goto uses in the current function.
    /// C99: §6.8.1 paragraph 3, p. 132; PDF p. 144.
    /// C99: §6.8.6.1 paragraph 1, p. 137; PDF p. 149.
    pub(super) fn label(&mut self, name: Identifier, definition: bool) {
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
                self.statements.labels[index].vm = self.statements.vm;
            }
        } else {
            _ = self.statements.labels[index].used.get_or_insert(name);
            self.statements.jumps.push(Jump {
                function: self.functions.current.map_or(usize::MAX, |f| f.id),
                label:    index,
                source:   name,
                vm:       self.statements.vm,
            });
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
            self.statements.labels.push(Label {
                local: true,
                function: self.functions.current.map_or(usize::MAX, |f| f.id),
                name,
                definition: None,
                vm: None,
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
        self.statements.labels.push(Label {
            local: false,
            function,
            name,
            definition: None,
            vm: None,
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
        let index = self.statements.vm_scopes.len();
        let parent = self.statements.vm;
        let depth = parent.map_or(1, |p| self.statements.vm_scopes[p].depth + 1);
        self.statements.vm_scopes.push(VmScope {
            binding,
            parent,
            depth,
        });
        self.statements.vm = Some(index);
    }

    /// Cache immutable path pairs, including successful scope exits.
    /// C99: §6.8.6.1p1, p. 137; PDF p. 149.
    fn entered_vm(&mut self, source: Option<usize>, target: Option<usize>) -> Option<usize> {
        if target.is_none() || source == target {
            return None;
        }
        if let Some(&result) = self.statements.vm_queries.get(&(source, target)) {
            return result;
        }
        let result = self.find_entered_vm(source, target);
        _ = self.statements.vm_queries.insert((source, target), result);
        result
    }

    /// Finds a variably modified identifier whose scope a jump would enter.
    /// C99: §6.8.6.1 paragraph 1, p. 137; PDF p. 149.
    fn find_entered_vm(&self, source: Option<usize>, mut target: Option<usize>) -> Option<usize> {
        let mut source = source;
        let depth = |path: Option<usize>| path.map_or(0, |p| self.statements.vm_scopes[p].depth);
        while depth(source) > depth(target) {
            #[cfg(test)]
            self.review_step(0);
            source = source.and_then(|p| self.statements.vm_scopes[p].parent);
        }
        let mut entered = None;
        while source != target {
            #[cfg(test)]
            self.review_step(0);
            if depth(target) >= depth(source) {
                let p = target?;
                entered = Some(self.statements.vm_scopes[p].binding);
                target = self.statements.vm_scopes[p].parent;
            } else {
                source = source.and_then(|p| self.statements.vm_scopes[p].parent);
            }
        }
        entered
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
            if label.definition.is_some()
                && label.function == jump.function
                && let Some(binding) = self.entered_vm(jump.vm, label.vm)
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
        lower: ConstantExpressionSlot<'tu>,
        upper: Option<ConstantExpressionSlot<'tu>>,
        source: SourceVectors,
    ) {
        self.switch_label(source);
        let parsed = |slot| match slot {
            | ConstantExpressionSlot::Parsed(e) => Some(e.expression()),
            | ConstantExpressionSlot::Missing(_) => None,
        };
        let lower = parsed(lower);
        let upper = upper.and_then(parsed);
        self.work.push(Work::StatementWork(StatementWork::CaseDone(
            lower, upper, source,
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
    pub(super) fn switch_label(&mut self, source: SourceVectors) {
        if let Some(index) = self.statements.switch {
            if let Some(binding) =
                self.entered_vm(self.statements.switches[index].vm, self.statements.vm)
            {
                self.error(
                    SemanticErrorKind::SwitchIntoVariableScope,
                    source,
                    None,
                    Some(self.bindings[binding].name.source_vectors),
                );
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
                .map(|v| v.cast(bits, signed).order_key())
        };
        let Some(lower) = lower.and_then(value) else {
            return;
        };
        let Some(upper) = upper.map_or(Some(lower), value) else {
            return;
        };
        if lower > upper {
            self.error(SemanticErrorKind::EmptyCaseRange, source, None, None);
            return;
        }
        switch.cases.push(
            self.scratch,
            Case {
                lower,
                upper,
                source,
                ordinal: self.statements.case_count,
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
        cases.extend_from_slice(list);
        cases.sort_unstable_by_key(|c| (c.lower, c.ordinal));
        let mut previous: Option<Case> = None;
        for case in cases {
            if let Some(prior) = previous
                && case.lower <= prior.upper
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
            if previous.is_none_or(|p| case.upper > p.upper) {
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
    /// C99: §6.8.4p3, p. 133; PDF p. 145; §6.8.5p5, p. 135;
    /// PDF p. 147. Associated substatements are blocks, including expressions.
    Substatement(&'tu Statement<'tu>),
    SwitchReady(ExpressionSlot<'tu>, &'tu Statement<'tu>),
    SwitchFinish(usize, Option<usize>),
    LeaveLoop,
    ForDeclarationDone(&'tu Declaration<'tu>),
    CaseDone(
        Option<&'tu Expression<'tu>>,
        Option<&'tu Expression<'tu>>,
        SourceVectors,
    ),
    ReturnDone(Option<ExpressionSlot<'tu>>, SourceVectors),
}

/// A persistent declaration-scope path; forward labels can retain it after
/// the lexical scope has closed. C99: §6.8.6.1p1, p. 137; PDF p. 149.
#[derive(Clone, Copy)]
struct VmScope {
    binding: usize,
    parent:  Option<usize>,
    depth:   usize,
}

/// A function-scoped label or a GNU local label and its definition.
/// GNU extension: GCC manual, "Local Labels".
/// <https://gcc.gnu.org/onlinedocs/gcc/Local-Labels.html>
/// C99: §6.2.1 paragraph 3, p. 29; PDF p. 41.
/// C99: §6.8.1 paragraph 3, p. 132; PDF p. 144.
#[derive(Clone, Copy)]
struct Label {
    local:      bool,
    function:   usize,
    name:       Identifier,
    definition: Option<Identifier>,
    vm:         Option<usize>,
    used:       Option<Identifier>,
}

/// A goto target and the variably modified scope path at its use.
/// C99: §6.8.6.1 paragraph 1, p. 137; PDF p. 149.
#[derive(Clone, Copy)]
struct Jump {
    function: usize,
    label:    usize,
    source:   Identifier,
    vm:       Option<usize>,
}

/// A converted switch case value or inclusive GNU case interval.
/// GNU extension: GCC manual, "Case Ranges".
/// <https://gcc.gnu.org/onlinedocs/gcc/Case-Ranges.html>
/// C99: §6.8.4.2 paragraph 3, p. 134; PDF p. 146.
/// C99: §6.8.4.2 paragraph 5, p. 134; PDF p. 146.
#[derive(Clone, Copy)]
struct Case {
    lower:   i128,
    upper:   i128,
    source:  SourceVectors,
    ordinal: usize,
}

/// Promoted controlling type and scope/label state for one switch.
/// C99: §6.8.4.2 paragraphs 2-5, p. 134; PDF p. 146.
#[derive(Clone, Copy)]
struct Switch<'s> {
    ty:    TypeId,
    vm:    Option<usize>,
    cases: &'s Collection<'s, Case>,
}

pub(super) struct State<'s> {
    pub(super) loops:    usize,
    pub(super) switch:   Option<usize>,
    pub(super) vm:       Option<usize>,
    pub(super) scope_vm: ArenaVec<'s, Option<usize>>,
    vm_scopes:           ArenaVec<'s, VmScope>,
    labels:              ArenaVec<'s, Label>,
    label_names:         ArenaMap<'s, (usize, StringCacheId), usize>,
    jumps:               ArenaVec<'s, Jump>,
    switches:            ArenaVec<'s, Switch<'s>>,
    case_count:          usize,
    vm_queries:          ArenaMap<'s, (Option<usize>, Option<usize>), Option<usize>>,
}

impl<'s> State<'s> {
    pub(super) fn new(scratch: &'s Bump) -> Self {
        let mut scope_vm = ArenaVec::new_in(scratch);
        scope_vm.push(None);
        Self {
            loops: 0,
            switch: None,
            vm: None,
            scope_vm,
            vm_scopes: ArenaVec::new_in(scratch),
            labels: ArenaVec::new_in(scratch),
            label_names: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            jumps: ArenaVec::new_in(scratch),
            switches: ArenaVec::new_in(scratch),
            case_count: 0,
            vm_queries: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
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
