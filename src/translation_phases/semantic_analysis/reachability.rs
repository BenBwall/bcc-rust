//! Whether Clang's control-flow graph reaches an expression, for the warnings
//! that ask: `DiagIfReachable`, which keeps a warning about a discarded value
//! until the end of its function, and `DiagRuntimeBehavior`, which goes through
//! it. C does not ask this question, and neither does GCC, which warns in code
//! that does not run: each warning here is one C99 §5.1.1.3p1 permits.
//!
//! This is not the `dead` flag of [`super::emission`], which answers another
//! question: whether code generation has an insertion point. The two disagree
//! in both directions. A loop that never runs is emitted, and an unreferenced
//! label starts a block that code generation emits, but neither is reachable;
//! `__builtin_trap` leaves an insertion point, but does not return.
//!
//! The answer needs each condition's constant value, so a warning waits until
//! the function has been analyzed, as Clang's waits for its CFG. A function
//! with no such warning costs nothing. Then [`Analyzer::flow_statement`] and
//! [`Analyzer::flow_expression`] follow control through the function in
//! execution order, carrying whether the point they are at is reachable, and
//! `known_conditions` decides which edges Clang prunes
//! (`PruneTriviallyFalseEdges`). A jump backward can reach what the walk has
//! already passed, so it repeats until nothing new is reached.
//!
//! A warning is reported only if the walk reached its expression, so what the
//! walk does not read stays silent: named `break` and `continue`, the body of a
//! SEH statement, an `asm` operand, a type name's bounds in a cast.
//! C99: §5.1.1.3p1, p. 11; PDF p. 23; §6.8, pp. 131-139; PDF pp. 143-151.

mod known_conditions;

use rustc_hash::FxBuildHasher;

use super::{
    Analyzer,
    ExpressionType as E,
    SemanticErrorKind,
    types::{
        TagKind,
        TypeKind,
    },
};
use crate::{
    configuration::{
        DiagnosticGroup,
        RuleSet,
    },
    translation_phases::{
        SourceVectors,
        parsing::{
            Builtin,
            GenericSelection,
            OffsetMember,
            SyntaxOperand,
            declaration_syntax::{
                Declaration,
                Declarator,
                DesignatorType,
                DirectDeclarator,
                Initializer,
                InitializerType,
            },
            syntax::{
                BinaryOperator as B,
                BlockItem,
                ConstantExpressionSlot,
                Expression,
                ExpressionSlot,
                ForInitializer,
                Identifier,
                Statement,
                StatementType as S,
            },
        },
        preprocessing::KeywordTokenType,
    },
    util::{
        bump::{
            ArenaMap,
            ArenaVec,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

/// A warning waiting for the end of its function.
#[derive(Clone, Copy)]
struct DeferredWarning {
    function: usize,
    /// The identity of the expression whose reachability decides.
    anchor:   usize,
    group:    DiagnosticGroup,
    kind:     SemanticErrorKind,
    site:     SourceVectors,
    name:     Option<StringCacheId>,
}

/// What the walk needs that analysis alone knows.
pub(super) struct ReachabilityState<'s> {
    deferred:         ArenaVec<'s, DeferredWarning>,
    /// Each enumeration's values, by tag, which tell whether a switch covers
    /// them.
    enumerators:      ArenaMap<'s, usize, ArenaVec<'s, i128>>,
    /// The enumeration each enumerator belongs to, by binding.
    enumerator_tags:  ArenaMap<'s, usize, usize>,
    /// The association each generic selection selects, by identity.
    selected_generic: ArenaMap<'s, usize, usize>,
}

impl<'s> ReachabilityState<'s> {
    pub(super) fn new(scratch: &'s Bump) -> Self {
        Self {
            deferred:         ArenaVec::new_in(scratch),
            enumerators:      ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            enumerator_tags:  ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            selected_generic: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
        }
    }
}

/// One walk's knowledge, which only grows from pass to pass.
struct Flow<'s> {
    /// Each waiting warning's expression, and whether it is reached.
    anchors:       ArenaMap<'s, usize, bool>,
    /// Labels a reachable jump names: the local label's declaration, or 0 for
    /// a function's own label, and the name.
    jumped_to:     ArenaMap<'s, (usize, StringCacheId), ()>,
    /// Labels whose address is taken, anywhere.
    address_taken: ArenaMap<'s, (usize, StringCacheId), ()>,
    /// A computed goto is reachable, so each of those labels is.
    computed_goto: bool,
    /// Loops whose head a back edge reaches, by identity.
    repeating:     ArenaMap<'s, usize, ()>,
    /// Conditions already decided, by identity.
    known:         ArenaMap<'s, usize, Option<bool>>,
    /// The loops and switches around the current point, innermost last.
    targets:       ArenaVec<'s, Target>,
    /// The switches around the current point, innermost last.
    switches:      ArenaVec<'s, SwitchFlow>,
    /// The local labels in scope, innermost last.
    local_labels:  ArenaVec<'s, (StringCacheId, usize)>,
    /// This pass reached something it may already have passed.
    changed:       bool,
}

/// A statement `break` or `continue` jumps out of.
#[derive(Clone, Copy)]
struct Target {
    is_loop:   bool,
    breaks:    bool,
    continues: bool,
}

/// Which of a switch's labels its condition reaches.
#[derive(Clone, Copy)]
struct SwitchFlow {
    /// The condition's end is reachable.
    entered: bool,
    /// The constant condition's value.
    value:   Option<super::Integer>,
    /// Nothing reaches `default` or the statement's end directly: a case
    /// matches the constant condition, or the labels cover its enumeration.
    covered: bool,
}

impl<'tu, 's> Analyzer<'_, 'tu, 's> {
    /// Reports the warning at the end of the function if its expression
    /// `anchor` is reachable then, or at once outside a function, where Clang
    /// has no CFG to ask.
    /// C99: §5.1.1.3p1, p. 11; PDF p. 23.
    pub(super) fn warn_if_reachable(
        &mut self,
        anchor: &'tu Expression<'tu>,
        group: DiagnosticGroup,
        kind: SemanticErrorKind,
        site: SourceVectors,
    ) {
        if self.tainted {
            return;
        }
        let Some(function) = self.functions.current else {
            self.group_diagnostic(group, kind, site, None, None);
            return;
        };
        self.reachability.deferred.push(DeferredWarning {
            function: function.id,
            anchor: std::ptr::from_ref(anchor).addr(),
            group,
            kind,
            site,
            name: None,
        });
    }

    /// Reports the current function's waiting warnings that are reachable,
    /// when its analysis is done.
    pub(super) fn report_reachable_warnings(&mut self) {
        let Some(function) = self.functions.current else {
            return;
        };
        let scratch = self.scratch;
        let deferred =
            std::mem::replace(&mut self.reachability.deferred, ArenaVec::new_in(scratch));
        if !deferred.iter().any(|w| w.function == function.id) {
            self.reachability.deferred = deferred;
            return;
        }
        let mut flow = Flow {
            anchors:       ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            jumped_to:     ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            address_taken: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            computed_goto: false,
            repeating:     ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            known:         ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            targets:       ArenaVec::new_in(scratch),
            switches:      ArenaVec::new_in(scratch),
            local_labels:  ArenaVec::new_in(scratch),
            changed:       false,
        };
        for warning in deferred.iter().filter(|w| w.function == function.id) {
            _ = flow.anchors.insert(warning.anchor, false);
        }
        loop {
            flow.changed = false;
            _ = self.flow_statement(&mut flow, function.syntax.body, true);
            if !flow.changed {
                break;
            }
        }
        // Each warning was taken only where nothing recovered around it.
        let tainted = std::mem::replace(&mut self.tainted, false);
        for warning in &deferred {
            if warning.function != function.id {
                self.reachability.deferred.push(*warning);
            } else if flow.anchors.get(&warning.anchor).copied().unwrap_or(false) {
                self.group_diagnostic(
                    warning.group,
                    warning.kind,
                    warning.site,
                    warning.name,
                    None,
                );
            }
        }
        self.tainted = tainted;
    }

    /// Records an enumerator, for Clang's reading of a switch over its type.
    /// C99: §6.7.2.2 paragraph 3, p. 105; PDF p. 117.
    pub(super) fn note_enumerator(
        &mut self,
        tag: usize,
        name: Identifier,
        value: Option<super::Integer>,
    ) {
        if self.context.configuration.compiler_identity().rules() != RuleSet::Clang {
            return;
        }
        let Some(value) = value.and_then(super::Integer::to_i128) else {
            return;
        };
        let scratch = self.scratch;
        self.reachability
            .enumerators
            .entry(tag)
            .or_insert_with(|| ArenaVec::new_in(scratch))
            .push(value);
        if let Some(entry) = self.lookup(super::scopes::Namespace::Ordinary, name.name) {
            _ = self.reachability.enumerator_tags.insert(entry.binding, tag);
        }
    }

    /// Records which association a generic selection selects.
    /// C11: §6.5.1.1 paragraphs 2-3, p. 79; PDF p. 97.
    pub(super) fn note_selected_association(
        &mut self,
        generic: &'tu GenericSelection<'tu>,
        selected: Option<&'tu Expression<'tu>>,
    ) {
        if let Some(selected) = selected {
            _ = self.reachability.selected_generic.insert(
                std::ptr::from_ref(generic).addr(),
                std::ptr::from_ref(selected).addr(),
            );
        }
    }

    /// Whether the end of `statement` is reachable, when its start is if
    /// `live`.
    /// C99: §6.8, pp. 131-139; PDF pp. 143-151.
    fn flow_statement(
        &mut self,
        flow: &mut Flow<'s>,
        statement: &'tu Statement<'tu>,
        live: bool,
    ) -> bool {
        match statement.kind {
            | S::Compound { items } => {
                let scope = flow.local_labels.len();
                let mut live = live;
                for item in items {
                    live = match *item {
                        | BlockItem::Statement(child) => self.flow_statement(flow, child, live),
                        | BlockItem::Declaration(declaration) =>
                            self.flow_declaration(flow, declaration, live),
                        | BlockItem::FunctionDefinition(_) => live,
                    };
                }
                flow.local_labels.truncate(scope);
                live
            },
            | S::Declaration(declaration) => self.flow_declaration(flow, declaration, live),
            | S::Attributed(attributed) => self.flow_statement(flow, attributed.statement, live),
            | S::LocalLabels(names) => {
                for name in names {
                    flow.local_labels
                        .push((name.name, std::ptr::from_ref(statement).addr()));
                }
                live
            },
            | S::Label(name, child) => {
                let label = Self::label_key(flow, name.name);
                let reached = live
                    || flow.jumped_to.contains_key(&label)
                    || (flow.computed_goto && flow.address_taken.contains_key(&label));
                self.flow_statement(flow, child, reached)
            },
            | S::Goto(name) => {
                if live {
                    let label = Self::label_key(flow, name.name);
                    flow.changed |= flow.jumped_to.insert(label, ()).is_none();
                }
                false
            },
            | S::ComputedGoto(slot) => {
                if self.flow_slot(flow, slot, live) {
                    flow.changed |= !flow.computed_goto;
                    flow.computed_goto = true;
                }
                false
            },
            | S::Default(child) => {
                let reached = live
                    || flow
                        .switches
                        .last()
                        .is_some_and(|switch| switch.entered && !switch.covered);
                self.flow_statement(flow, child, reached)
            },
            | S::Case(bound, child) => {
                let reached = live || self.case_reached(flow, bound, bound);
                self.flow_statement(flow, child, reached)
            },
            | S::CaseRange(range) => {
                let reached = live || self.case_reached(flow, range.lower, range.upper);
                self.flow_statement(flow, range.statement, reached)
            },
            | S::If {
                condition_expression,
                then_statement,
                else_statement,
            } => {
                let (yes, no) = self.flow_condition(flow, condition_expression, live);
                let then_end = self.flow_statement(flow, then_statement, yes);
                let else_end =
                    else_statement.map_or(no, |child| self.flow_statement(flow, child, no));
                then_end || else_end
            },
            | S::Switch {
                condition_expression,
                body_statement,
            } => self.flow_switch(flow, statement, condition_expression, body_statement, live),
            | S::While {
                condition_expression,
                body_statement,
            } => {
                let head = live || Self::repeats(flow, statement);
                let (yes, no) = self.flow_condition(flow, condition_expression, head);
                let (end, target) = self.flow_loop_body(flow, body_statement, yes);
                if end || target.continues {
                    Self::repeat(flow, statement);
                }
                no || target.breaks
            },
            | S::DoWhile {
                condition_expression,
                body_statement,
            } => {
                let entry = live || Self::repeats(flow, statement);
                let (end, target) = self.flow_loop_body(flow, body_statement, entry);
                let (yes, no) =
                    self.flow_condition(flow, condition_expression, end || target.continues);
                if yes {
                    Self::repeat(flow, statement);
                }
                no || target.breaks
            },
            | S::For(for_loop) => {
                let entry = match for_loop.initializer {
                    | Some(ForInitializer::Declaration(declaration)) =>
                        self.flow_declaration(flow, declaration, live),
                    | Some(ForInitializer::Expression(slot)) => self.flow_slot(flow, slot, live),
                    | None => live,
                };
                let head = entry || Self::repeats(flow, statement);
                let (yes, no) = for_loop
                    .condition_expression
                    .map_or((head, false), |slot| self.flow_condition(flow, slot, head));
                let (end, target) = self.flow_loop_body(flow, for_loop.body_statement, yes);
                let step = end || target.continues;
                let again = for_loop
                    .iteration_expression
                    .map_or(step, |slot| self.flow_slot(flow, slot, step));
                if again {
                    Self::repeat(flow, statement);
                }
                no || target.breaks
            },
            | S::Return(slot) => {
                if let Some(slot) = slot {
                    _ = self.flow_slot(flow, slot, live);
                }
                false
            },
            | S::Break => {
                if live && let Some(target) = flow.targets.last_mut() {
                    target.breaks = true;
                }
                false
            },
            | S::Continue => {
                if live && let Some(target) = flow.targets.iter_mut().rev().find(|t| t.is_loop) {
                    target.continues = true;
                }
                false
            },
            | S::NamedBreak(_) | S::NamedContinue(_) => false,
            | S::Expression(slot) => self.flow_slot(flow, slot, live),
            | S::Asm(asm) => {
                if live && asm.qualifiers.goto {
                    for name in asm.labels {
                        let label = Self::label_key(flow, name.name);
                        flow.changed |= flow.jumped_to.insert(label, ()).is_none();
                    }
                }
                live
            },
            | _ => live,
        }
    }

    /// The body of a loop, whose start is reachable if `live`: whether its
    /// end is, and what jumps out of it.
    fn flow_loop_body(
        &mut self,
        flow: &mut Flow<'s>,
        body: &'tu Statement<'tu>,
        live: bool,
    ) -> (bool, Target) {
        flow.targets.push(Target {
            is_loop:   true,
            breaks:    false,
            continues: false,
        });
        let end = self.flow_statement(flow, body, live);
        let target = flow.targets.pop().expect("this loop's target");
        (end, target)
    }

    fn repeats(flow: &Flow<'s>, statement: &'tu Statement<'tu>) -> bool {
        flow.repeating
            .contains_key(&std::ptr::from_ref(statement).addr())
    }

    fn repeat(flow: &mut Flow<'s>, statement: &'tu Statement<'tu>) {
        flow.changed |= flow
            .repeating
            .insert(std::ptr::from_ref(statement).addr(), ())
            .is_none();
    }

    /// A label's name in the scope of the local labels around it.
    /// GNU extension: GCC manual, "Local Labels".
    fn label_key(flow: &Flow<'s>, name: StringCacheId) -> (usize, StringCacheId) {
        let scope = flow
            .local_labels
            .iter()
            .rev()
            .find(|&&(local, _)| local == name)
            .map_or(0, |&(_, scope)| scope);
        (scope, name)
    }

    /// A switch statement: its condition decides which labels are reached,
    /// with Clang's `shouldAddCase` for a constant, and its body starts
    /// unreachable.
    /// C99: §6.8.4.2 paragraph 4, p. 134; PDF p. 146.
    fn flow_switch(
        &mut self,
        flow: &mut Flow<'s>,
        statement: &'tu Statement<'tu>,
        condition: ExpressionSlot<'tu>,
        body: &'tu Statement<'tu>,
        live: bool,
    ) -> bool {
        let entered = self.flow_slot(flow, condition, live);
        let parsed = super::statements::slot_expression(condition);
        let value = parsed.and_then(|e| match self.evaluate_ignoring_side_effects(e) {
            | Some(super::emission::Value::Integer(value)) => Some(value),
            | _ => None,
        });
        let labels = self.switch_labels(statement);
        let bounds = |analyzer: &Self, label: &'tu Statement<'tu>| match label.kind {
            | S::Case(bound, _) => analyzer.case_value_range(bound, bound),
            | S::CaseRange(range) => analyzer.case_value_range(range.lower, range.upper),
            | _ => None,
        };
        let mut has_default = false;
        let mut covered = false;
        if let Some(labels) = labels {
            let mut ranges: ArenaVec<'_, (super::Integer, super::Integer)> =
                ArenaVec::new_in(self.scratch);
            for &label in labels.labels {
                has_default |= matches!(label.kind, S::Default(_));
                ranges.extend(bounds(self, label));
            }
            covered = value.map_or_else(
                || self.covers_enumeration(parsed, &ranges),
                |value| {
                    ranges
                        .iter()
                        .any(|&(low, high)| Self::in_case_range(value, low, high))
                        || self.covers_enumeration(parsed, &ranges)
                },
            );
        }
        flow.switches.push(SwitchFlow {
            entered,
            value,
            covered,
        });
        flow.targets.push(Target {
            is_loop:   false,
            breaks:    false,
            continues: false,
        });
        let end = self.flow_statement(flow, body, false);
        let target = flow.targets.pop().expect("this switch's target");
        _ = flow.switches.pop();
        end || target.breaks || (entered && !has_default && !covered)
    }

    /// Whether the innermost switch's condition reaches a case label.
    fn case_reached(
        &self,
        flow: &Flow<'s>,
        low: ConstantExpressionSlot<'tu>,
        high: ConstantExpressionSlot<'tu>,
    ) -> bool {
        let Some(switch) = flow.switches.last() else {
            return false;
        };
        if !switch.entered {
            return false;
        }
        let Some(value) = switch.value else {
            return true;
        };
        self.case_value_range(low, high)
            .is_none_or(|(low, high)| Self::in_case_range(value, low, high))
    }

    fn case_value_range(
        &self,
        low: ConstantExpressionSlot<'tu>,
        high: ConstantExpressionSlot<'tu>,
    ) -> Option<(super::Integer, super::Integer)> {
        let value = |slot| match slot {
            | ConstantExpressionSlot::Parsed(bound) =>
                self.expression_info(bound.expression()).integer,
            | ConstantExpressionSlot::Missing(_) => None,
        };
        Some((value(low)?, value(high)?))
    }

    /// Whether a condition's `value` is in a case's range, the bounds being
    /// converted to its type. C99: §6.8.4.2 paragraph 5, p. 134; PDF p. 146.
    fn in_case_range(value: super::Integer, low: super::Integer, high: super::Integer) -> bool {
        let converted = |bound: super::Integer| bound.cast(value.bits, value.signed).to_i128();
        match (value.to_i128(), converted(low), converted(high)) {
            | (Some(value), Some(low), Some(high)) => low <= value && value <= high,
            | _ => true,
        }
    }

    /// Clang's `isAllEnumCasesCovered`: `condition` has an enumeration type,
    /// through parentheses and a comma, and the labels hold each of its
    /// values, which leaves `default` and the end of the statement no edge.
    fn covers_enumeration(
        &self,
        condition: Option<&'tu Expression<'tu>>,
        ranges: &[(super::Integer, super::Integer)],
    ) -> bool {
        let Some(condition) = condition else {
            return false;
        };
        // The usual arithmetic conversions leave a conditional's result an
        // integer, and bcc keeps the enumeration.
        if matches!(
            Self::without_parentheses(condition).kind,
            E::Conditional(_) | E::OmittedConditional(_)
        ) {
            return false;
        }
        let ty = self.types.non_atomic(self.expression_info(condition).ty);
        let TypeKind::Tag(tag) = self.types.nodes[ty.index] else {
            return false;
        };
        if self.types.tags[tag].kind != TagKind::Enum {
            return false;
        }
        self.reachability
            .enumerators
            .get(&tag)
            .is_some_and(|values| {
                values.iter().all(|&value| {
                    ranges.iter().any(|&(low, high)| {
                        low.to_i128()
                            .zip(high.to_i128())
                            .is_some_and(|(low, high)| low <= value && value <= high)
                    })
                })
            })
    }

    /// Whether the end of a declaration is reachable: its bounds and
    /// initializers run in order, those of a `static` object included, which
    /// can only hold unevaluated operands.
    /// C99: §6.7.5.2 paragraph 5, p. 117; PDF p. 129; §6.7.8, pp. 125-130;
    /// PDF pp. 137-142.
    fn flow_declaration(
        &mut self,
        flow: &mut Flow<'s>,
        declaration: &'tu Declaration<'tu>,
        live: bool,
    ) -> bool {
        let mut live = live;
        for init in declaration.init_declarators {
            live = self.flow_declarator(flow, init.declarator, live);
            if let Some(initializer) = init.initializer {
                live = self.flow_initializer(flow, initializer, live);
            }
        }
        live
    }

    fn flow_declarator(
        &mut self,
        flow: &mut Flow<'s>,
        declarator: Declarator<'tu>,
        live: bool,
    ) -> bool {
        let mut live = live;
        for direct in declarator.kind {
            match *direct {
                | DirectDeclarator::Array {
                    assignment_expression: Some(bound),
                    ..
                } => live = self.flow_expression(flow, bound, live),
                | DirectDeclarator::Parenthesized(inner) =>
                    live = self.flow_declarator(flow, inner.declarator, live),
                | _ => {},
            }
        }
        live
    }

    fn flow_initializer(
        &mut self,
        flow: &mut Flow<'s>,
        initializer: &'tu Initializer<'tu>,
        live: bool,
    ) -> bool {
        match initializer.kind {
            | InitializerType::AssignmentExpression(e) => self.flow_expression(flow, e, live),
            | InitializerType::InitializerList(list) => {
                let mut live = live;
                for element in list.elements {
                    for designator in element.designation.iter().flat_map(|d| d.designators) {
                        match designator.kind {
                            | DesignatorType::Array(index) =>
                                live = self.flow_expression(flow, index.expression(), live),
                            | DesignatorType::Range(range) => {
                                live = self.flow_expression(flow, range.lower.expression(), live);
                                live = self.flow_expression(flow, range.upper.expression(), live);
                            },
                            | _ => {},
                        }
                    }
                    live = self.flow_initializer(flow, element.initializer, live);
                }
                live
            },
        }
    }

    fn flow_slot(&mut self, flow: &mut Flow<'s>, slot: ExpressionSlot<'tu>, live: bool) -> bool {
        match slot {
            | ExpressionSlot::Parsed(e) => self.flow_expression(flow, e, live),
            | ExpressionSlot::Selection(header) => {
                let live = self.flow_declaration(flow, header.declaration, live);
                header
                    .expression
                    .map_or(live, |slot| self.flow_slot(flow, slot, live))
            },
            | ExpressionSlot::Missing(_) => live,
        }
    }

    /// A condition in a slot: whether its true and its false edge are
    /// reachable.
    fn flow_condition(
        &mut self,
        flow: &mut Flow<'s>,
        slot: ExpressionSlot<'tu>,
        live: bool,
    ) -> (bool, bool) {
        match slot {
            | ExpressionSlot::Parsed(e) => self.flow_branch(flow, e, live),
            | ExpressionSlot::Selection(header) => {
                let live = self.flow_declaration(flow, header.declaration, live);
                header
                    .expression
                    .map_or((live, live), |slot| self.flow_condition(flow, slot, live))
            },
            | ExpressionSlot::Missing(_) => (live, live),
        }
    }

    /// Whether `e`'s true and false edges are reachable, as Clang's
    /// `VisitLogicalOperator` links `&&` and `||` to the branch they decide.
    /// C99: §6.5.13p4, p. 89; PDF p. 101; §6.5.14p4, p. 89; PDF p. 101.
    fn flow_branch(
        &mut self,
        flow: &mut Flow<'s>,
        e: &'tu Expression<'tu>,
        live: bool,
    ) -> (bool, bool) {
        let known = self.known_condition(flow, e);
        let (yes, no) = match Self::without_parentheses(e).kind {
            | E::Binary {
                operator: B::LogicalAnd,
                left_expression,
                right_expression,
            } => {
                let (left_yes, left_no) =
                    self.flow_branch(flow, Self::without_parentheses(left_expression), live);
                let (right_yes, right_no) =
                    self.flow_branch(flow, Self::without_parentheses(right_expression), left_yes);
                (right_yes, left_no || right_no)
            },
            | E::Binary {
                operator: B::LogicalOr,
                left_expression,
                right_expression,
            } => {
                let (left_yes, left_no) =
                    self.flow_branch(flow, Self::without_parentheses(left_expression), live);
                let (right_yes, right_no) =
                    self.flow_branch(flow, Self::without_parentheses(right_expression), left_no);
                (left_yes || right_yes, right_no)
            },
            | _ => {
                let end = self.flow_expression(flow, e, live);
                (end, end)
            },
        };
        (yes && known != Some(false), no && known != Some(true))
    }

    /// Whether the end of `e` is reachable, when its start is if `live`.
    /// A waiting warning's expression counts as reached at its end, where the
    /// CFG holds it: after a call in it that does not return.
    /// C99: §5.1.2.3p2, p. 13; PDF p. 25; §6.5, pp. 67-94; PDF pp. 79-106.
    fn flow_expression(
        &mut self,
        flow: &mut Flow<'s>,
        e: &'tu Expression<'tu>,
        live: bool,
    ) -> bool {
        let end = match e.kind {
            | E::Parenthesized { expression }
            | E::Unary {
                operand_expression: expression,
                ..
            }
            | E::Cast {
                operand_expression: expression,
                ..
            }
            | E::DirectMember {
                base_expression: expression,
                ..
            }
            | E::IndirectMember {
                base_expression: expression,
                ..
            } => self.flow_expression(flow, expression, live),
            | E::StatementExpression(statement) => self.flow_statement(flow, statement, live),
            | E::Binary {
                operator: B::LogicalAnd | B::LogicalOr,
                ..
            } => {
                let (yes, no) = self.flow_branch(flow, e, live);
                yes || no
            },
            | E::Binary {
                left_expression,
                right_expression,
                ..
            } => {
                let live = self.flow_expression(flow, left_expression, live);
                self.flow_expression(flow, right_expression, live)
            },
            | E::Conditional(c) => {
                let (yes, no) = self.flow_branch(flow, c.condition_expression, live);
                let then_end = self.flow_expression(flow, c.then_expression, yes);
                let else_end = self.flow_expression(flow, c.else_expression, no);
                then_end || else_end
            },
            | E::OmittedConditional(c) => {
                let (yes, no) = self.flow_branch(flow, c.condition_expression, live);
                let else_end = self.flow_expression(flow, c.else_expression, no);
                yes || else_end
            },
            | E::Call {
                function_expression,
                arguments,
            } => {
                let mut end = self.flow_expression(flow, function_expression, live);
                for &argument in arguments {
                    end = self.flow_expression(flow, argument, end);
                }
                end && !self.cfg_call_does_not_return(
                    flow,
                    function_expression,
                    arguments.as_slice(),
                )
            },
            | E::CompoundLiteral { initializer, .. } =>
                self.flow_initializer(flow, initializer, live),
            | E::LabelAddress(name) => {
                let label = Self::label_key(flow, name.name);
                flow.changed |=
                    flow.address_taken.insert(label, ()).is_none() && flow.computed_goto;
                live
            },
            | E::Generic(generic) => {
                let selected = self
                    .reachability
                    .selected_generic
                    .get(&std::ptr::from_ref(generic).addr())
                    .copied();
                generic
                    .associations
                    .iter()
                    .find(|association| {
                        Some(std::ptr::from_ref(association.expression).addr()) == selected
                    })
                    .map_or(live, |association| {
                        self.flow_expression(flow, association.expression, live)
                    })
            },
            | E::Builtin(builtin) => self.flow_builtin(flow, builtin, live),
            // A `sizeof` of a variable length type evaluates its bounds; an
            // operand is not evaluated for its own sake.
            // C99: §6.5.3.4p2, p. 80; PDF p. 92.
            | E::SizeofType(name) => name.declarator.map_or(live, |declarator| {
                self.flow_declarator(flow, declarator, live)
            }),
            | _ => live,
        };
        if let Some(reached) = flow.anchors.get_mut(&std::ptr::from_ref(e).addr()) {
            *reached |= end;
        }
        end
    }

    /// A builtin with operands: `__builtin_choose_expr` evaluates the operand
    /// its condition selects, and `offsetof` its indices.
    /// GNU extension: GCC manual, "Other Builtins".
    fn flow_builtin(
        &mut self,
        flow: &mut Flow<'s>,
        builtin: &'tu Builtin<'tu>,
        live: bool,
    ) -> bool {
        if builtin.keyword == KeywordTokenType::BuiltinChooseExpr {
            let [
                SyntaxOperand::Expression(condition),
                SyntaxOperand::Expression(yes),
                SyntaxOperand::Expression(no),
            ] = *builtin.operands.as_slice()
            else {
                return live;
            };
            return match Self::constant_truth(self.expression_info(condition)) {
                | Some(true) => self.flow_expression(flow, yes, live),
                | Some(false) => self.flow_expression(flow, no, live),
                | None => live,
            };
        }
        let mut live = live;
        for operand in builtin.operands {
            if let SyntaxOperand::Expression(operand) = *operand {
                live = self.flow_expression(flow, operand, live);
            }
        }
        for member in builtin.members {
            if let OffsetMember::Index(index) = *member {
                live = self.flow_expression(flow, index, live);
            }
        }
        live
    }

    /// Clang's `FunctionDecl::isNoReturn` and the CFG's `__builtin_assume` of
    /// a false condition. A call through a function that is `noreturn`
    /// differs from [`Self::call_does_not_return`] in the builtins whose call
    /// Clang emits without ending the block, as `__builtin_trap`.
    /// C11: §6.7.4 paragraph 8, p. 126; PDF p. 144.
    fn cfg_call_does_not_return(
        &mut self,
        flow: &mut Flow<'s>,
        callee: &'tu Expression<'tu>,
        arguments: &'tu [&'tu Expression<'tu>],
    ) -> bool {
        if self.call_does_not_return(callee) {
            return true;
        }
        let designator = Self::without_parentheses(callee);
        let E::Identifier(name) = designator.kind else {
            return false;
        };
        let intrinsic = self
            .expression_info(designator)
            .binding
            .is_some_and(|binding| {
                !matches!(
                    self.bindings[binding].origin,
                    super::BindingOrigin::User | super::BindingOrigin::ImplicitOrdinary
                )
            });
        if !intrinsic {
            return false;
        }
        match self.context.string_cache.at(name.name) {
            | "__builtin_trap" | "__builtin_verbose_trap" | "__builtin_eh_return" => true,
            | "__builtin_assume" => arguments
                .first()
                .is_some_and(|&condition| self.known_condition(flow, condition) == Some(false)),
            | _ => false,
        }
    }
}
