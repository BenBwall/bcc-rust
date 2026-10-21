//! Phase-7 function definitions and translation-unit completion.
//! C99: §6.9-§6.9.2, pp. 140-143; PDF pp. 152-155; inline definitions
//! §6.7.4p3, p. 112; PDF p. 124. This validates bodies without lowering them.

use rustc_hash::FxBuildHasher;

use super::{
    Analyzer,
    BindingKind,
    Duration,
    Linkage,
    ScopeKind,
    SemanticErrorKind,
    StatementId,
    Work,
    results::{
        Definition,
        DefinitionKind,
        FunctionRecord,
    },
    scopes::Namespace,
    types::{
        ArrayBound,
        Parameter,
        Scalar,
        TypeId,
        TypeKind,
    },
};
use crate::{
    translation_phases::{
        SourceVectors,
        parsing::{
            GenericSelection,
            declaration_syntax::{
                Declaration,
                DeclarationSpecifiers,
                Declarator,
                DirectDeclarator,
                TypeQualifiers,
                TypeSpecifiers,
            },
            syntax::{
                Expression,
                FunctionDefinition,
                Identifier,
                Statement,
                StatementType,
                StorageClass,
            },
        },
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

impl<'tu> Analyzer<'_, 'tu, '_> {
    pub(super) fn prepare_function(&mut self, f: &'tu FunctionDefinition<'tu>) {
        self.work.push(Work::RestoreReferenceFunction(
            self.functions.reference_function,
        ));
        self.functions.reference_function = Some(std::ptr::from_ref(f).addr());
        // Definition parameter expressions execute on entry. Index them while
        // resolving the signature, using the same root as the later body.
        let previous = self.control.current;
        let control =
            self.control
                .start_function(self.functions.next_function, self.scope, f.source_vectors);
        _ = self
            .function_controls
            .insert(std::ptr::from_ref(f).addr(), (control, previous));
        if let Some(name) = f.declarator.identifier()
            && let Some(entry) = self.lookup(Namespace::Ordinary, name.name)
        {
            _ = self.functions.previous.insert(
                std::ptr::from_ref(f).addr(),
                self.bindings[entry.binding].name.source_vectors,
            );
        }
    }

    /// C99: §6.9.1p2-7, pp. 141-142; PDF pp. 153-154.
    /// GNU extension: nested definitions have no linkage and reject
    /// extern/static.
    /// GNU extension: GCC manual, "Nested Functions".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Nested-Functions.html>
    pub(super) fn function_body(&mut self, f: &'tu FunctionDefinition<'tu>, declared_type: TypeId) {
        let nested = self.scopes[self.scope].kind != ScopeKind::File;
        let name = f.declarator.identifier();
        let prior = self.bindings.len();
        let errors_before = self.semantic_errors;
        if nested {
            // GNU nested functions denote lexical entities, not translation-
            // unit externals. Their parser owner reports the extension.
            if matches!(
                f.declaration_specifiers.storage_class,
                Some(StorageClass::Static | StorageClass::Extern)
            ) {
                self.error(
                    SemanticErrorKind::NestedFunctionStorage,
                    f.declaration_specifiers.source_vectors,
                    name.map(|n| n.name),
                    None,
                );
            }
            if let Some(name) = name {
                self.bind(
                    name,
                    declared_type,
                    BindingKind::Function,
                    Linkage::None,
                    Duration::None,
                    None,
                );
            }
        } else {
            self.bind_declaration(f.declarator, f.declaration_specifiers, declared_type, false);
        }
        let binding = name
            .and_then(|n| self.lookup(Namespace::Ordinary, n.name))
            .map(|e| e.binding);
        _ = self.correspondence.declaration(
            f.declarator,
            binding
                .filter(|&i| i >= prior && self.semantic_errors == errors_before && !self.tainted)
                .map(|i| (i, self.bindings[i])),
            super::BoundSiteFacts {
                ty:                  declared_type,
                bounds:              &[],
                operand_evaluations: &[],
            },
            &[],
        );
        if let Some(binding) = binding.filter(|&i| i >= prior) {
            let subject = super::attributes::Subject::Function {
                binding,
                definition: true,
            };
            self.declaration_attributes(
                subject,
                f.declaration_specifiers.extensions,
                Some(f.declarator),
            );
            self.declaration_dll(subject, f.declaration_specifiers, false);
        }
        let ty = binding.map_or_else(|| self.types.unknown(), |i| self.bindings[i].ty);
        let derivation = function_derivation(f.declarator, self.scratch);
        if derivation.is_none()
            || (!self.types.unanalyzed(ty)
                && !matches!(self.types.nodes[ty.index], TypeKind::Function { .. }))
        {
            self.error(
                SemanticErrorKind::InvalidFunctionDefinition,
                f.declarator.source_vectors,
                name.map(|n| n.name),
                None,
            );
        }
        if !nested && f.declaration_specifiers.storage_class == Some(StorageClass::Typedef) {
            self.error(
                SemanticErrorKind::FunctionDefinitionStorage,
                f.declaration_specifiers.source_vectors,
                name.map(|n| n.name),
                None,
            );
        }
        let mut result = match self.types.nodes[ty.index] {
            | TypeKind::Function { result, .. } => result,
            | _ => self.types.unknown(),
        };
        if !self.types.unanalyzed(result)
            && !matches!(
                self.types.nodes[result.index],
                TypeKind::Scalar(Scalar::Void)
            )
            && !self.complete_object(result)
        {
            self.error(
                SemanticErrorKind::IncompleteFunctionReturn,
                f.declarator.source_vectors,
                name.map(|n| n.name),
                None,
            );
            result = self.types.unknown();
        }
        if derivation.is_none()
            || matches!(
                self.types.nodes[result.index],
                TypeKind::Array(..) | TypeKind::Function { .. }
            )
        {
            result = self.types.unknown();
        }
        let (parameters, prototype_scope) = derivation
            .and_then(|d| self.parameters.get(&std::ptr::from_ref(d).addr()).copied())
            .unwrap_or((&[], 0));
        let old_style = derivation
            .is_some_and(|d| matches!(d, DirectDeclarator::KAndRStyleFunction { .. }))
            || (parameters.is_empty() && !f.declaration_list.is_empty());
        if !old_style && !f.declaration_list.is_empty() {
            self.error(
                SemanticErrorKind::InvalidDefinitionParameterList,
                f.source_vectors,
                name.map(|n| n.name),
                None,
            );
        }
        if let Some(index) = binding {
            self.record_function_definition(index, f);
        }
        self.work.push(Work::FunctionWork(FunctionWork::RestoreDead(
            self.functions.dead,
        )));
        self.work
            .push(Work::FunctionWork(FunctionWork::RestoreUnvisited(
                self.functions.unvisited,
            )));
        self.work
            .push(Work::FunctionWork(FunctionWork::RestoreOrphan(
                self.functions.emission.orphan,
            )));
        self.work
            .push(Work::FunctionWork(FunctionWork::RestoreWeakHorizon(
                self.functions.emission.weak_horizon,
            )));
        self.functions.dead = false;
        self.functions.unvisited = false;
        self.functions.emission.orphan = None;
        // Clang emits this body where it is defined, or after the unit.
        self.functions.emission.weak_horizon =
            if binding.is_some_and(|index| self.emitted_where_defined(index, f)) {
                self.functions.emission.root_end
            } else {
                usize::MAX
            };
        self.work.push(Work::FunctionWork(FunctionWork::Finish(
            self.functions.current,
            self.statements.loops,
            self.statements.switch,
            self.functions.sizeof,
            self.control.targets.len(),
        )));
        self.enter(ScopeKind::Function);
        self.functions.sizeof = None;
        self.statements.loops = 0;
        self.statements.switch = None;
        let id = self.functions.next_function;
        self.functions.next_function += 1;
        debug_assert_eq!(
            id,
            self.function_records.len(),
            "records follow function ids"
        );
        let (control, previous) = self.function_controls[&std::ptr::from_ref(f).addr()];
        debug_assert_eq!(
            control.id.0, id,
            "signature and body share their function root"
        );
        self.control.regions[control.root.0].lexical_scope = super::ScopeId(self.scope);
        self.control.scope_regions[self.scope] = previous;
        self.function_records.push(FunctionRecord {
            control,
            parameter_sites: &[],
            old_style,
            parameter_facts: &[],
            syntax: f,
            binding,
            result,
            scope: self.scope,
            parameter_aliases: &[],
            assembly: false,
        });
        self.functions.current = Some(FunctionContext {
            id,
            syntax: f,
            binding,
            result,
            parameters,
            scope: self.scope,
            name: None,
            pretty: None,
        });
        self.function_name = name;
        if old_style {
            for parameter in parameters {
                if let Some(name) = parameter.name {
                    _ = self
                        .functions
                        .old_names
                        .insert((id, name.name), name.source_vectors);
                }
            }
        }
        let mut parameter_aliases = ArenaVec::new_in(self.scratch);
        if prototype_scope != 0 {
            let entries = self.scope_entries[prototype_scope].finish(self.scratch, self.scratch);
            for &index in entries {
                let entry = self.entries[index];
                if entry.namespace == Namespace::Tag {
                    self.install(entry.name, entry.namespace, entry.binding);
                } else if entry.namespace == Namespace::Ordinary
                    && self.bindings[entry.binding].kind == BindingKind::Enumerator
                {
                    let b = self.bindings[entry.binding];
                    self.bind(b.name, b.ty, b.kind, b.linkage, b.duration, b.value);
                }
            }
        }
        if !old_style {
            let void = matches!(parameters, [Parameter { name: None, ty, .. }] if matches!(self.types.nodes[ty.index], TypeKind::Scalar(Scalar::Void)));
            let mut parameter_bindings = ArenaVec::new_in(self.scratch);
            for (parameter_index, parameter) in parameters.iter().enumerate() {
                if void {
                    continue;
                }
                parameter_bindings.push(None);
                let syntax = match derivation {
                    | Some(DirectDeclarator::Function { parameter_list, .. }) =>
                        parameter_list.as_slice().get(parameter_index),
                    | _ => None,
                };
                // C99 §6.7.5.3p4: completeness after adjustment applies even
                // when C23 permits omitting the parameter's name. Void
                // parameters were already rejected during prototype
                // construction.
                let valid = !matches!(
                    self.types.nodes[parameter.ty.index],
                    TypeKind::Scalar(Scalar::Void)
                ) && self.validate_definition_parameter(
                    parameter.ty,
                    parameter.name.map_or_else(
                        || syntax.map_or(f.declarator.source_vectors, |p| p.source_vectors),
                        |name| name.source_vectors,
                    ),
                    parameter.name.map(|name| name.name),
                );
                if let Some(name) = parameter.name {
                    // Prototype construction already diagnosed duplicate
                    // parameter/enumerator names. Replay each binding once.
                    if self
                        .lookup(Namespace::Ordinary, name.name)
                        .is_some_and(|e| e.scope == self.scope)
                    {
                        continue;
                    }
                    // Failed parameter types must not generate dependent
                    // errors in the body.
                    let parameter_type = if valid {
                        parameter.ty
                    } else {
                        self.types.unknown()
                    };
                    self.bind(
                        name,
                        parameter_type,
                        BindingKind::Parameter,
                        Linkage::None,
                        Duration::Automatic,
                        None,
                    );
                    if let Some(prototype) = parameter.binding {
                        if let Some(&alignment) = self.binding_alignments.get(&prototype) {
                            _ = self
                                .binding_alignments
                                .insert(self.bindings.len() - 1, alignment);
                        }
                        parameter_aliases.push((prototype, self.bindings.len() - 1));
                        if let Some(&typed) = self.functions.emission.noreturn.get(&prototype) {
                            _ = self
                                .functions
                                .emission
                                .noreturn
                                .insert(self.bindings.len() - 1, typed);
                        }
                        if let Some(&written) =
                            self.function_extensions.written_types.get(&prototype)
                        {
                            _ = self
                                .function_extensions
                                .written_types
                                .insert(self.bindings.len() - 1, written);
                        }
                        if let Some(&size) = self.extension_parameters.get(&prototype) {
                            _ = self
                                .extension_parameters
                                .insert(self.bindings.len() - 1, size);
                        }
                    }
                    if let Some(minimum) = parameter.array_minimum {
                        _ = self
                            .correspondence
                            .parameter_minimums
                            .insert(self.bindings.len() - 1, minimum);
                    }
                    self.record_vm(self.bindings.len() - 1);
                    if let Some(slot) = parameter_bindings.last_mut() {
                        *slot = Some(self.bindings.len() - 1);
                    }
                    // The body binding is distinct from its prototype binding.
                    // C99 §6.5.3.2p1, p. 78; PDF p. 90.
                    if matches!(derivation, Some(DirectDeclarator::Function { parameter_list, .. })
                        if parameter_list.as_slice().get(parameter_index).is_some_and(|p|
                            p.declaration_specifiers.storage_class == Some(StorageClass::Register)))
                    {
                        _ = self.register_bindings.insert(self.bindings.len() - 1, true);
                    }
                } else if !self
                    .context
                    .configuration
                    .is_native(crate::configuration::Feature::UnnamedDefinitionParameters)
                {
                    let parser_exempt = syntax.is_some_and(|p| {
                        parameters.len() == 1
                            && p.declarator.is_none()
                            && matches!(
                                p.declaration_specifiers.type_specifiers,
                                TypeSpecifiers::Void | TypeSpecifiers::TypedefName(_)
                            )
                    });
                    // The parser already reports the C23 spelling under a
                    // pedantic policy. Do not emit the same requirement twice.
                    if parser_exempt
                        || self.context.configuration.extension_policy()
                            == crate::configuration::ExtensionPolicy::Allow
                    {
                        self.extension_error(
                            crate::configuration::Feature::UnnamedDefinitionParameters,
                            crate::translation_phases::DiagnosticPolicy::Warning,
                            SemanticErrorKind::UnnamedDefinitionParameter,
                            syntax.map_or(f.declarator.source_vectors, |p| p.source_vectors),
                            None,
                        );
                    }
                }
            }
            self.validate_definition_stars(derivation);
            self.function_records[id].parameter_aliases =
                self.types.tu.alloc_slice_copy(&parameter_aliases);
            self.retain_parameter_facts(id, parameters, &parameter_bindings);
        }
        self.define_func_name(name);
        if !old_style {
            self.check_main(f, ty);
        }
        self.work.push(Work::PopScope);
        self.work.push(Work::Statement(f.body, false));
        self.work
            .push(Work::RestoreParameterMode(self.old_parameter_mode));
        if old_style {
            self.work.push(Work::OldSignature(f));
        }
        self.old_parameter_mode = old_style;
        for &d in f.declaration_list.iter().rev() {
            self.work.push(Work::Declaration(d));
        }
    }

    pub(super) fn function_work(&mut self, work: FunctionWork<'tu>) {
        match work {
            | FunctionWork::Finish(previous, loops, switch, sizeof, targets) => {
                self.functions.current = previous;
                self.functions.sizeof = sizeof;
                self.function_name = previous.and_then(|f| f.syntax.declarator.identifier());
                self.statements.loops = loops;
                self.statements.switch = switch;
                self.control.targets.truncate(targets);
            },
            | FunctionWork::RestoreSizeof(previous) => self.functions.sizeof = previous,
            | FunctionWork::RestoreReferenceContext(previous) =>
                self.functions.reference_context = previous,
            | FunctionWork::RestoreDead(dead) => self.functions.dead = dead,
            | FunctionWork::RestoreUnvisited(unvisited) => self.functions.unvisited = unvisited,
            | FunctionWork::RestoreOrphan(orphan) => self.functions.emission.orphan = orphan,
            | FunctionWork::RestoreWeakHorizon(horizon) =>
                self.functions.emission.weak_horizon = horizon,
            | FunctionWork::RestoreUnjoinedOrphan(orphan) =>
                self.functions.emission.orphan = self.functions.emission.orphan.and(orphan),
            | FunctionWork::RestoreSwitchDead(dead) => self.functions.switch_dead = dead,
            | FunctionWork::ThenEnd => self.functions.arm_ends.push(self.functions.dead),
            | FunctionWork::JoinArms => {
                let then_end = self.functions.arm_ends.pop().expect("then arm ended");
                self.functions.dead &= then_end;
                self.functions.emission.orphan = None;
            },
            | FunctionWork::SwitchEnd(outer) => {
                let after = self.functions.emission.switch_after.take();
                self.functions.dead = outer || after.unwrap_or(false);
                self.functions.emission.orphan = None;
            },
            | FunctionWork::DeadStatementArm(condition, live, outer, statement) => {
                let skipped = self.folded_condition(condition) == Some(!live)
                    && statement
                        .is_none_or(|statement| !self.statement_contains_label(statement, false));
                self.functions.dead = outer.dead || skipped;
                self.functions.unvisited = outer.unvisited || skipped;
            },
            | FunctionWork::DeadArm(condition, live, outer, arm) => {
                // Clang emits a folded-away operand that holds a label.
                let skipped = self.folded_condition(condition) == Some(!live)
                    && !self.expression_contains_label(arm);
                self.functions.dead = outer.dead || skipped;
                self.functions.unvisited = outer.unvisited || skipped;
            },
            | FunctionWork::ConditionalMark(expression, index, outer) =>
                self.mark_conditional_arm(expression, index, outer),
            | FunctionWork::GenericMark(generic) => {
                let at = self.functions.uses.len();
                let scratch = self.scratch;
                let marks = self
                    .functions
                    .generic_marks
                    .entry(std::ptr::from_ref(generic).addr())
                    .or_insert_with(|| ArenaVec::new_in(scratch));
                // The controlling operand is not emitted, and only the
                // selected association is: each starts from the block
                // around the selection ([`super::emission`]).
                if let Some(&(_, around)) = marks.first()
                    && (marks.len() == 1 || marks.len() % 2 == 0)
                {
                    self.functions.emission.orphan = around;
                }
                marks.push((at, self.functions.emission.orphan));
            },
            | FunctionWork::ExpressionStatementEnd {
                expression,
                serial,
                value,
            } => self.end_expression_statement(expression, serial, value),
            | FunctionWork::StatementExpressionEnd(outer) => {
                // `StmtExprEvaluation` ensures an insertion point: a body that
                // ends skipped leaves a new block without predecessors. Only
                // an emitted statement expression does: not one in code or an
                // operand that is skipped, nor one in an unevaluated operand.
                if self.functions.dead
                    && !outer
                    && self.functions.sizeof.is_none()
                    && self.functions.reference_context.is_none()
                {
                    self.functions.emission.orphan = Some(self.functions.emission.serial);
                }
                self.functions.dead = outer;
            },
            | FunctionWork::LoopEnd(statement, outer) => {
                let infinite = match statement.kind {
                    | StatementType::While {
                        condition_expression,
                        ..
                    } =>
                        super::statements::slot_expression(condition_expression)
                            .and_then(|condition| self.generated_condition(condition))
                            == Some(true),
                    // Clang emits a condition exit for `for (;1;)`, but
                    // creates none when the condition is absent.
                    | StatementType::For(for_loop) => for_loop.condition_expression.is_none(),
                    | _ => false,
                };
                let exits = self
                    .control
                    .statement_id(statement)
                    .is_some_and(|id| self.has_generated_break(id));
                self.functions.dead = outer || (infinite && !exits);
                self.functions.emission.orphan = None;
            },
        }
    }

    /// Translation-unit completion is deterministic in declaration order.
    /// C99: §6.9p3,p5, p. 140; PDF p. 152; §6.9.2p2-3, p. 143;
    /// PDF p. 155; inline definitions §6.7.4p3,p6, p. 112; PDF p. 124.
    pub(super) fn finish_translation_unit(&mut self) {
        self.control.finalize(self.scratch, self.types.tu);
        for i in 0..self.control.deferred_vm_labels.len() {
            let (diagnostic, from, to) = self.control.deferred_vm_labels[i];
            let ancestry = self.control.ancestry.as_ref().expect("control finalized");
            let transition = ancestry.transition(&self.control.regions, from, to);
            if let Some(region) = ancestry.first_entered_vm(&self.control.regions, transition)
                && let Some(crate::translation_phases::TranslationError::Semantic(error)) =
                    self.context.pending_errors.iter_mut_from(diagnostic).next()
            {
                error.previous = Some(self.control.regions[region.0].source);
            }
        }
        self.finish_labels();
        let mut entities = ArenaVec::new_in(self.scratch);
        entities.extend(self.functions.entities.values().copied());
        entities.sort_unstable_by_key(|entity| entity.latest);
        for mut entity in entities {
            let index = entity.latest;
            let b = self.bindings[index];
            if entity.definition.is_none()
                && entity.tentative.is_some()
                && !matches!(self.types.nodes[b.ty.index], TypeKind::Scalar(Scalar::Void))
                && !self.declares_alias(b.name.name)
            {
                let mut ty = b.ty;
                if let TypeKind::Array(element, ArrayBound::Incomplete) = self.types.nodes[ty.index]
                    && b.linkage == Linkage::External
                    && !self.types.unanalyzed(ty)
                {
                    self.error(
                        SemanticErrorKind::TentativeArrayAssumedOne,
                        b.name.source_vectors,
                        Some(b.name.name),
                        None,
                    );
                    ty = self
                        .types
                        .intern(TypeKind::Array(element, ArrayBound::Constant(1)))
                        .qualified(ty.qualifiers);
                    self.bindings[index].ty = ty;
                }
                if !self.types.unanalyzed(ty) && !self.complete_object(ty) {
                    if b.linkage != Linkage::Internal {
                        self.error(
                            SemanticErrorKind::IncompleteTentativeDefinition,
                            b.name.source_vectors,
                            Some(b.name.name),
                            None,
                        );
                    }
                } else {
                    entity.definition = Some(b.name.source_vectors);
                    self.definitions.push(Definition {
                        binding: index,
                        kind:    DefinitionKind::Tentative,
                    });
                }
                _ = self
                    .functions
                    .entities
                    .insert(self.function_identity(index), entity);
            }
        }
        let mut ignored = ArenaVec::new_in(self.scratch);
        for (index, context) in self.functions.sizeof_contexts.iter().enumerate() {
            ignored
                .push(self.sizeof_unevaluated(index) || context.parent.is_some_and(|p| ignored[p]));
        }
        let mut used = ArenaMap::with_hasher_in(FxBuildHasher, self.scratch);
        let mut ignored_references = ArenaVec::new_in(self.scratch);
        for (index, context) in self.functions.reference_contexts.iter().enumerate() {
            ignored_references.push(
                self.reference_context_unevaluated(index)
                    || context.parent.is_some_and(|p| ignored_references[p]),
            );
        }
        let mut owners = ArenaMap::with_hasher_in(FxBuildHasher, self.scratch);
        for function in &self.function_records {
            if let Some(binding) = function.binding {
                _ = owners.insert(std::ptr::from_ref(function.syntax).addr(), binding);
            }
        }
        for owner in self
            .attributes
            .refusals
            .iter_mut()
            .map(|refusal| &mut refusal.owner)
            .chain(
                self.attributes
                    .conventions
                    .iter_mut()
                    .map(|convention| &mut convention.owner),
            )
            .chain(
                self.symbol_requests
                    .iter_mut()
                    .map(|request| &mut request.owner),
            )
        {
            *owner = owner.and_then(|syntax| owners.get(&syntax).copied());
        }
        let mut evaluated = ArenaVec::new_in(self.scratch);
        for i in 0..self.functions.uses.len() {
            let usage = self.functions.uses[i];
            let b = self.bindings[usage.binding];
            let owner = match usage.owner {
                | Some(ReferenceOwner::Function(syntax)) => owners.get(&syntax).copied(),
                | Some(ReferenceOwner::Object(binding)) => Some(binding),
                | None => None,
            };
            if !usage.sizeof.is_some_and(|s| ignored[s])
                && !usage
                    .reference_context
                    .is_some_and(|s| ignored_references[s])
            {
                if let Some(owner) = owner {
                    self.references.push(super::EntityReference {
                        owner,
                        binding: usage.binding,
                        dead: usage.dead,
                    });
                }
                if let Some(import) = usage.dll_import {
                    evaluated.push((owner, usage.binding, import, usage.dead));
                }
            }
            if let Some(function) = usage.inline_function
                && b.linkage == Linkage::Internal
                && self.is_inline_definition(function)
            {
                self.error(
                    SemanticErrorKind::InlineInternalReference,
                    usage.name.source_vectors,
                    Some(usage.name.name),
                    Some(b.name.source_vectors),
                );
            }
            if b.linkage == Linkage::Internal
                && !usage.sizeof.is_some_and(|s| ignored[s])
                && !usage
                    .reference_context
                    .is_some_and(|s| ignored_references[s])
            {
                _ = used
                    .entry(self.function_identity(usage.binding))
                    .or_insert(usage.name);
            }
        }
        for i in 0..self.functions.inline_objects.len() {
            let object = self.functions.inline_objects[i];
            if self.is_inline_definition(object.function) {
                let b = self.bindings[object.binding];
                self.error(
                    SemanticErrorKind::InlineStaticObject,
                    b.name.source_vectors,
                    Some(b.name.name),
                    None,
                );
            }
        }
        let mut entities = ArenaVec::new_in(self.scratch);
        entities.extend(self.functions.entities.values().copied());
        entities.sort_unstable_by_key(|entity| entity.latest);
        for entity in entities {
            let b = self.bindings[entity.latest];
            if b.linkage == Linkage::Internal
                && entity.definition.is_none()
                && !self.types.unanalyzed(b.ty)
            {
                if let Some(&usage) = used.get(&self.function_identity(entity.latest)) {
                    self.error(
                        SemanticErrorKind::UndefinedInternal,
                        usage.source_vectors,
                        Some(b.name.name),
                        Some(b.name.source_vectors),
                    );
                } else if b.kind == BindingKind::Function {
                    self.group_diagnostic(
                        crate::configuration::DiagnosticGroup::UnusedFunction,
                        SemanticErrorKind::UnusedStaticFunction,
                        b.name.source_vectors,
                        Some(b.name.name),
                        None,
                    );
                }
            }
        }
        for i in 0..self.definitions.len() {
            let definition = self.definitions[i];
            if matches!(
                definition.kind,
                DefinitionKind::Function | DefinitionKind::Inline
            ) && self.bindings[definition.binding].linkage == Linkage::External
            {
                self.definitions[i].kind = self.external_function_kind(definition);
            }
        }
        self.functions.evaluated_imports = evaluated.leak();
        self.definitions.sort_unstable_by_key(|d| d.binding);
    }

    /// The final kind of an external function definition, after every
    /// declaration. Microsoft inline bodies remain callable external
    /// definitions; their retention follows source-order emission,
    /// independently of C99 inline-only declarations, and they repeat in
    /// each unit with COMDAT linkage, as in Clang's
    /// `basicGVALinkageForFunction`. A GNU extern inline body that a
    /// plain `inline` declaration makes visible is external. An imported
    /// inline body supplies no definition, nor a C99 inline-only one except
    /// a MinGW export. The resulting definitions are the alias targets.
    /// C99 §6.7.4p6, pp. 112-113; PDF pp. 124-125; Microsoft ABI and GNU
    /// extensions under §4p6, p. 7; PDF p. 19.
    fn external_function_kind(&mut self, definition: Definition) -> DefinitionKind {
        let binding = definition.binding;
        let identity = self.function_identity(binding);
        let Some(entity) = self.functions.entities.get(&identity).copied() else {
            return definition.kind;
        };
        let source = self.bindings[binding].name.source_vectors;
        let final_body = matches!(
            entity.function_body,
            Some(FunctionBody::Replaceable(body) | FunctionBody::Unique(body)) if body == source
        );
        let exported = self.dll_storage(binding) == Some(super::DllStorage::Export);
        let microsoft = !self.context.configuration.target().c99_inline_definitions();
        let mut kind = definition.kind;
        if kind == DefinitionKind::Inline && final_body && entity.gnu_plain_inline {
            kind = DefinitionKind::Function;
        } else if kind == DefinitionKind::Function {
            if !microsoft {
                if self.inline_only_declarations(binding) && !exported {
                    kind = DefinitionKind::Inline;
                }
            } else if entity.msvc_inline.emission == InlineEmission::Discardable {
                kind = DefinitionKind::DiscardableFunction;
            }
        }
        let entity_id = self.entity_of(binding);
        if self.ends_imported(entity_id) {
            // An imported body is no definition, nor an alias target, even
            // where the body recorded one.
            kind = DefinitionKind::Inline;
            if let Some(entity) = self.functions.entities.get_mut(&identity) {
                entity.definition = None;
            }
        }
        if matches!(
            kind,
            DefinitionKind::Function | DefinitionKind::DiscardableFunction
        ) {
            if let Some(entity) = self.functions.entities.get_mut(&identity) {
                entity.definition = Some(source);
            }
            if entity.msvc_inline.inline_body && !entity.gnu_inline && (microsoft || exported) {
                self.set_inline_odr(
                    binding,
                    if entity.msvc_inline.explicit_extern || exported {
                        super::InlineOdr::Strong
                    } else {
                        super::InlineOdr::Discardable
                    },
                );
            }
        }
        kind
    }

    /// The latest binding of the linked entity `name` and whether the
    /// translation unit defines it, including completed tentative objects.
    /// C99: §6.9p5, p. 140; PDF p. 152; §6.9.2p2, p. 143; PDF p. 155.
    pub(super) fn linked_entity(&self, name: StringCacheId) -> Option<(usize, bool)> {
        self.functions
            .entities
            .get(&name)
            .map(|entity| (entity.latest, entity.definition.is_some()))
    }

    /// Whether a declaration of `binding`'s function, up to this one, says
    /// `inline`, as Clang's `FunctionDecl::isInlined` inherits it.
    /// C99: §6.7.4p6, pp. 112-113; PDF pp. 124-125.
    pub(super) fn inline_declared(&self, binding: usize) -> bool {
        self.functions
            .entities
            .get(&self.function_identity(binding))
            .is_some_and(|entity| entity.msvc_inline.inline_declared)
    }

    /// An applicable `dllexport` before the body keeps a Microsoft inline
    /// definition; one first written after the body is ignored.
    /// Microsoft ABI extension under C99: §4p6, p. 7; PDF p. 19.
    pub(super) fn note_dll_export(&mut self, binding: usize) {
        let identity = self.function_identity(binding);
        if let Some(entity) = self.functions.entities.get_mut(&identity)
            && entity.function_body.is_none()
        {
            entity.msvc_inline.dllexport = true;
        }
    }

    /// Where an earlier declaration defines the linked entity that `binding`
    /// redeclares: its function body or its initialized object. A tentative
    /// definition completes only at the end of the translation unit, so it
    /// does not count yet.
    /// C99: §6.9p5, p. 140; PDF p. 152; §6.9.2p2, p. 143; PDF p. 155.
    pub(super) fn earlier_definition(&self, binding: usize) -> Option<SourceVectors> {
        let b = self.bindings[binding];
        if b.linkage == Linkage::None {
            return None;
        }
        let entity = self
            .functions
            .entities
            .get(&self.function_identity(binding))?;
        let definition = match entity.function_body {
            | Some(FunctionBody::Replaceable(source) | FunctionBody::Unique(source)) =>
                Some(source),
            | None => entity.definition,
        }?;
        (definition != b.name.source_vectors).then_some(definition)
    }

    /// Definition parameters have complete adjusted object types.
    /// C99: §6.7.5.3p4, p. 118; PDF p. 130; §6.9.1p7, p. 142;
    /// PDF p. 154.
    fn validate_definition_parameter(
        &mut self,
        ty: TypeId,
        source: SourceVectors,
        name: Option<StringCacheId>,
    ) -> bool {
        if !self.types.unanalyzed(ty) && !self.complete_object(ty) {
            self.error(
                SemanticErrorKind::IncompleteDefinitionParameter,
                source,
                name,
                None,
            );
            return false;
        }
        true
    }

    /// `[*]` belongs to prototype scope, not a definition's parameter scope.
    /// C99: §6.7.5.2p4, pp. 116-117; PDF pp. 128-129; §6.9.1p7, p. 142; PDF p.
    /// 154.
    fn validate_definition_stars(&mut self, derivation: Option<&'tu DirectDeclarator<'tu>>) {
        let Some(DirectDeclarator::Function { parameter_list, .. }) = derivation else {
            return;
        };
        let mut work = ArenaVec::new_in(self.scratch);
        for parameter in *parameter_list {
            if let Some(d) = parameter.declarator {
                work.push(d);
            }
        }
        while let Some(d) = work.pop() {
            for direct in d.kind {
                match direct {
                    | DirectDeclarator::Parenthesized(p) => work.push(p.declarator),
                    | DirectDeclarator::Array {
                        is_pointer: true, ..
                    } => self.error(
                        SemanticErrorKind::DefinitionStarArray,
                        d.source_vectors,
                        d.identifier().map(|n| n.name),
                        None,
                    ),
                    // Nested function prototypes retain their own prototype scope.
                    | _ => {},
                }
            }
        }
    }

    /// Rejects empty declarations in an old-style definition parameter list.
    /// C99: §6.9.1 paragraph 6, p. 141; PDF p. 153.
    pub(super) fn validate_declaration_list_item(&mut self, d: &'tu Declaration<'tu>) {
        if self.old_parameter_mode && d.init_declarators.is_empty() && d.assertion.is_none() {
            self.error(
                SemanticErrorKind::InvalidDefinitionParameterList,
                d.source_vectors,
                None,
                None,
            );
        }
    }

    /// C99: §6.9.1p6, p. 141; PDF p. 153.
    pub(super) fn validate_old_parameter(
        &mut self,
        name: Identifier,
        _spec: DeclarationSpecifiers<'tu>,
        ty: TypeId,
        initialized: bool,
    ) -> bool {
        let Some(function) = self.functions.current else {
            return true;
        };
        let mut valid = true;
        if !self
            .functions
            .old_names
            .contains_key(&(function.id, name.name))
            || initialized
        {
            valid = false;
            self.error(
                SemanticErrorKind::InvalidDefinitionParameterList,
                name.source_vectors,
                Some(name.name),
                None,
            );
        }
        if let Some(entry) = self.lookup(Namespace::Ordinary, name.name)
            && self.bindings[entry.binding].kind == BindingKind::Typedef
        {
            valid = false;
            self.error(
                SemanticErrorKind::InvalidDefinitionParameterList,
                name.source_vectors,
                Some(name.name),
                Some(entry.name.source_vectors),
            );
        }
        let adjusted = match self.types.nodes[ty.index] {
            | TypeKind::Array(element, _) => self.types.intern(TypeKind::Pointer(element)),
            | TypeKind::Function { .. } => self.types.intern(TypeKind::Pointer(ty)),
            | _ => ty,
        };
        self.validate_definition_parameter(adjusted, name.source_vectors, Some(name.name)) && valid
    }

    /// Missing K&R declarations use C89 implicit int through shared policy.
    /// Promoted signatures use the identical defining suffix as body bindings.
    /// C99: §6.7.5.3p15, pp. 119-120; PDF pp. 131-132; §6.9.1p6,
    /// p. 141; PDF p. 153. Implicit int is a pre-C99 extension.
    pub(super) fn old_signature(&mut self, function: &'tu FunctionDefinition<'tu>) {
        let Some(current) = self.functions.current else {
            return;
        };
        let mut seen = ArenaMap::with_hasher_in(FxBuildHasher, self.scratch);
        let mut parameters = ArenaVec::new_in(self.scratch);
        let mut parameter_bindings = ArenaVec::new_in(self.scratch);
        let mut original_types = ArenaVec::new_in(self.scratch);
        for parameter in current.parameters {
            parameter_bindings.push(None);
            let Some(name) = parameter.name else {
                continue;
            };
            if let Some(previous) = seen.insert(name.name, name.source_vectors) {
                self.error(
                    SemanticErrorKind::DuplicateDeclaration,
                    name.source_vectors,
                    Some(name.name),
                    Some(previous),
                );
                continue;
            }
            let entry = self
                .lookup(Namespace::Ordinary, name.name)
                .filter(|e| e.scope == self.scope);
            let declared = if let Some(entry) = entry {
                if let Some(slot) = parameter_bindings.last_mut() {
                    *slot = Some(entry.binding);
                }
                self.bindings[entry.binding].ty
            } else {
                let int = self.types.scalar(Scalar::Int);
                self.bind(
                    name,
                    int,
                    BindingKind::Parameter,
                    Linkage::None,
                    Duration::Automatic,
                    None,
                );
                if let Some(slot) = parameter_bindings.last_mut() {
                    *slot = Some(self.bindings.len() - 1);
                }
                if !self
                    .context
                    .configuration
                    .is_native(crate::configuration::Feature::ImplicitInt)
                    && !self.context.configuration.gnu_extensions()
                {
                    self.extension_error(
                        crate::configuration::Feature::ImplicitInt,
                        crate::translation_phases::DiagnosticPolicy::Error,
                        SemanticErrorKind::InvalidDefinitionParameterList,
                        name.source_vectors,
                        Some(name.name),
                    );
                } else {
                    self.context.report_extension(
                        crate::configuration::Feature::ImplicitInt,
                        "implicit int parameter",
                        name.source_vectors,
                    );
                }
                int
            };
            let promoted = match self.types.nodes[declared.index] {
                | TypeKind::Scalar(
                    Scalar::Bool
                    | Scalar::Char
                    | Scalar::SignedChar
                    | Scalar::UnsignedChar
                    | Scalar::Short
                    | Scalar::UnsignedShort,
                ) => self.types.scalar(Scalar::Int),
                | TypeKind::Scalar(Scalar::Float) => self.types.scalar(Scalar::Double),
                | _ => declared.unqualified(),
            };
            original_types.push(
                self.functions
                    .old_types
                    .get(&(current.id, name.name))
                    .copied()
                    .unwrap_or(declared),
            );
            parameters.push(promoted);
        }
        let mut sites = ArenaVec::new_in(self.scratch);
        for &declaration in function.declaration_list {
            if let Some(operand_evaluations) =
                self.spec_evaluations(declaration.declaration_specifiers.type_specifiers)
            {
                sites.push(super::BoundSiteFacts {
                    ty: self.types.unknown(),
                    operand_evaluations,
                    bounds: self.spec_bounds(declaration.declaration_specifiers.type_specifiers),
                });
            }
            for init in declaration.init_declarators {
                if let Some(name) = init.declarator.identifier()
                    && let Some(&ty) = self.functions.old_types.get(&(current.id, name.name))
                    && let Some(entry) = self.lookup(Namespace::Ordinary, name.name)
                    && let Some(&bounds) = self.binding_bounds.get(&entry.binding)
                {
                    sites.push(super::BoundSiteFacts {
                        ty,
                        operand_evaluations: &[],
                        bounds,
                    });
                }
            }
        }
        self.function_records[current.id].parameter_sites = self.types.tu.alloc_slice_copy(&sites);
        self.function_records[current.id].parameter_facts =
            self.types
                .tu
                .alloc_slice_fill_iter(parameter_bindings.iter().enumerate().map(
                    |(ordinal, &binding)| {
                        let declared = binding.map_or_else(
                            || self.types.unknown(),
                            |binding| self.bindings[binding].ty,
                        );
                        let original = original_types.get(ordinal).copied().unwrap_or(declared);
                        super::ParameterFacts {
                            prototype_binding: None,
                            body_binding: binding,
                            declared,
                            promoted: parameters.get(ordinal).copied().unwrap_or(declared),
                            original,
                            site: Some(super::BoundSiteFacts {
                                ty:                  original,
                                bounds:              binding
                                    .and_then(|binding| self.binding_bounds.get(&binding).copied())
                                    .unwrap_or(&[]),
                                operand_evaluations: &[],
                            }),
                        }
                    },
                ));
        let Some(index) = current.binding else {
            return;
        };
        let binding = self.bindings[index];
        let TypeKind::Function {
            result,
            parameters: prior_parameters,
            prototype,
            ..
        } = self.types.nodes[binding.ty.index]
        else {
            return;
        };
        let parameters = self.types.tu.alloc_slice_copy(&parameters);
        let defined = self.types.intern(TypeKind::Function {
            result,
            parameters,
            prototype: false,
            variadic: false,
        });
        // C99 §6.7.5.3p15: a definition's identifier list, including an empty
        // one, must match a preceding prototype's parameter count.
        if (!prototype || prior_parameters.len() == parameters.len())
            && let Some(composite) = self.types.composite(binding.ty, defined)
        {
            self.bindings[index].ty = composite;
            self.check_main(function, composite);
        } else {
            let previous = self
                .functions
                .previous
                .get(&std::ptr::from_ref(function).addr())
                .copied();
            self.error(
                SemanticErrorKind::IncompatibleDeclaration,
                binding.name.source_vectors,
                Some(binding.name.name),
                previous,
            );
        }
    }

    /// The predefined object exists at function entry, including before its
    /// first use and when an outer object has the same spelling.
    /// C99: §6.4.2.2p1, p. 52; PDF p. 64.
    fn define_func_name(&mut self, function: Option<Identifier>) {
        let Some(function) = function else {
            return;
        };
        let name = self.context.string_cache.intern("__func__");
        let element = self
            .types
            .scalar(Scalar::Char)
            .qualified(TypeQualifiers::CONST);
        let count = self.context.string_cache.at(function.name).len() as u64 + 1;
        let ty = self
            .types
            .intern(TypeKind::Array(element, ArrayBound::Constant(count)));
        self.bind(
            Identifier {
                name,
                source_vectors: function.source_vectors,
            },
            ty,
            BindingKind::Object,
            Linkage::None,
            Duration::Static,
            None,
        );
        self.definitions.push(Definition {
            binding: self.bindings.len() - 1,
            kind:    DefinitionKind::FunctionName(function.name),
        });
        if let Some(current) = &mut self.functions.current {
            current.name = Some(self.bindings.len() - 1);
        }
    }

    /// Hosted `main` signatures are implementation-defined outside the two
    /// portable forms; warn without rejecting them. A freestanding startup
    /// function's name and type are implementation-defined, so nothing is
    /// checked there.
    /// C99: §5.1.2.2.1p1, p. 12; PDF p. 24; §5.1.2.1p1, p. 11; PDF p. 23.
    fn check_main(&mut self, f: &'tu FunctionDefinition<'tu>, ty: TypeId) {
        if !self.context.configuration.hosted() {
            return;
        }
        let Some(name) = f.declarator.identifier() else {
            return;
        };
        if self.context.string_cache.at(name.name) != "main"
            || self.scopes[self.scope]
                .parent
                .is_some_and(|s| self.scopes[s].kind != ScopeKind::File)
        {
            return;
        }
        let TypeKind::Function {
            result,
            parameters,
            variadic,
            ..
        } = self.types.nodes[ty.index]
        else {
            return;
        };
        if self.types.unanalyzed(ty) {
            return;
        }
        let int = self.types.scalar(Scalar::Int);
        let char = self.types.scalar(Scalar::Char);
        let pointer = self.types.intern(TypeKind::Pointer(char));
        let argv = self.types.intern(TypeKind::Pointer(pointer));
        if f.declaration_specifiers.storage_class == Some(StorageClass::Static)
            || result != int
            || variadic
            || !(parameters.is_empty() || parameters == [int, argv])
        {
            self.error(
                SemanticErrorKind::MainSignature,
                name.source_vectors,
                Some(name.name),
                None,
            );
        }
    }

    /// C99: §6.9.2p1-3, p. 143; PDF p. 155; §6.7.4p3, p. 112;
    /// PDF p. 124. Definition state is separate from declaration occurrences.
    /// GNU inline attribute applicability extends §6.7.4p6, pp. 112-113;
    /// PDF pp. 124-125.
    /// Microsoft ABI extension under C99: §4p6, p. 7; PDF p. 19:
    /// Clang's doesDeclarationForceExternallyVisibleDefinition retains a
    /// deferred inline body only on a forcing file-scope redeclaration.
    pub(super) fn record_declaration(
        &mut self,
        index: usize,
        spec: DeclarationSpecifiers<'tu>,
        declarator: Declarator<'tu>,
        initialized: bool,
    ) {
        if self.tainted {
            return;
        }
        let binding = self.bindings[index];
        self.record_vm(index);
        if binding.kind == BindingKind::Object && (initialized || binding.linkage == Linkage::None)
        {
            self.definitions.push(Definition {
                binding: index,
                kind:    DefinitionKind::Object,
            });
        }
        // C11 §6.7.4p3 extends this to thread storage duration.
        if binding.kind == BindingKind::Object
            && binding.duration.is_static_or_thread()
            && (initialized || binding.linkage == Linkage::None)
            && !self.types.unanalyzed(binding.ty)
            && self.inline_object_modifiable(binding.ty)
            && let Some(function) = self.functions.current
            && function
                .syntax
                .declaration_specifiers
                .function_specifiers
                .is_inline
            && let Some(function) = function.binding
            && self.bindings[function].linkage == Linkage::External
        {
            self.functions.inline_objects.push(InlineObject {
                function,
                binding: index,
            });
        }
        let previous = (binding.linkage != Linkage::None)
            .then(|| {
                self.functions
                    .entities
                    .get(&self.function_identity(index))
                    .copied()
            })
            .flatten();
        self.record_noreturn(
            index,
            spec,
            declarator,
            previous.map(|entity| entity.latest),
        );
        if binding.linkage == Linkage::None {
            return;
        }
        let mut entity = previous.unwrap_or_default();
        entity.latest = index;
        // A file-scope declaration may make an inline body seen before it
        // an external definition, which Clang then creates and emits.
        let forcible = binding.kind == BindingKind::Function
            && self.scopes[binding.scope].kind == ScopeKind::File
            && entity.function_body.is_some()
            && !self.body_external(&entity);
        if binding.kind == BindingKind::Function {
            let explicit_extern = spec.storage_class == Some(StorageClass::Extern);
            if self.scopes[binding.scope].kind == ScopeKind::File
                && entity.msvc_inline.emission == InlineEmission::Discardable
                && ((self.context.configuration.msvc_compatibility()
                    && explicit_extern
                    && !entity.msvc_inline.explicit_extern)
                    || (!self.context.configuration.gnu89_inline()
                        && !entity.non_inline
                        && (!spec.function_specifiers.is_inline || explicit_extern)))
            {
                entity.msvc_inline.emission = InlineEmission::Required;
            }
            // Inline propagates forward through linked declarations, even
            // from block scope. A block extern can require a later body,
            // but does not itself materialize an earlier deferred body.
            entity.msvc_inline.inline_declared |= spec.function_specifiers.is_inline;
            entity.msvc_inline.explicit_extern |= explicit_extern;
        }
        if self.scopes[binding.scope].kind == ScopeKind::File {
            if binding.kind == BindingKind::Function {
                // GNU attribute applicability extends C99 §6.7.4p6:
                // Clang ignores it without inline or after a unique body.
                entity.gnu_inline |= spec.function_specifiers.is_inline
                    && !matches!(entity.function_body, Some(FunctionBody::Unique(_)))
                    && self.has_function_attribute(spec, declarator, "gnu_inline");
                entity.can_redefine = spec.function_specifiers.is_inline
                    && spec.storage_class == Some(StorageClass::Extern)
                    && (entity.gnu_inline || self.context.configuration.gnu89_inline());
                entity.non_inline |= !spec.function_specifiers.is_inline
                    || spec.storage_class == Some(StorageClass::Extern);
                // Clang's isInlineDefinitionExternallyVisible: under GNU
                // inline semantics, any file-scope declaration that says
                // `inline` without `extern` makes the body external.
                entity.gnu_plain_inline |= spec.function_specifiers.is_inline
                    && spec.storage_class != Some(StorageClass::Extern);
            } else if binding.kind == BindingKind::Object {
                if initialized {
                    self.define_entity(index, &mut entity);
                } else if spec.storage_class.is_none()
                    || spec.storage_class == Some(StorageClass::Static)
                {
                    entity.tentative = Some(index);
                    if binding.linkage == Linkage::Internal
                        && !self.types.unanalyzed(binding.ty)
                        && !self.complete_object(binding.ty)
                    {
                        self.error(
                            SemanticErrorKind::IncompleteInternalTentative,
                            binding.name.source_vectors,
                            Some(binding.name.name),
                            None,
                        );
                    }
                }
            }
        }
        if forcible && self.body_external(&entity) {
            self.functions.external_redeclarations.push(index);
        }
        _ = self
            .functions
            .entities
            .insert(self.function_identity(index), entity);
    }

    /// Whether the body of function `entity` is an external definition by
    /// the declarations so far: Clang's `isInlineDefinitionExternallyVisible`
    /// for C99 and GNU inline semantics, and its Microsoft inline retention
    /// (`doesDeclarationForceExternallyVisibleDefinition`) elsewhere.
    /// C99: §6.7.4 paragraph 6, pp. 112-113; PDF pp. 124-125. GNU extension:
    /// GCC manual, "Inline"; Microsoft ABI extension under C99: §4p6, p. 7;
    /// PDF p. 19.
    fn body_external(&self, entity: &Entity) -> bool {
        let gnu = entity.gnu_inline || self.context.configuration.gnu89_inline();
        if !gnu && !self.context.configuration.target().c99_inline_definitions() {
            entity.msvc_inline.emission == InlineEmission::Required
        } else if gnu {
            !matches!(entity.function_body, Some(FunctionBody::Replaceable(_)))
                || entity.gnu_plain_inline
        } else {
            entity.non_inline
        }
    }

    /// Inline object restrictions concern the object's own qualification,
    /// not aggregate assignment: a const member does not protect other members.
    /// C99: §6.7.4p3, p. 112; PDF p. 124; array qualification §6.7.3p8,
    /// p. 109; PDF p. 121.
    fn inline_object_modifiable(&self, mut ty: TypeId) -> bool {
        loop {
            if ty.qualifiers.contains(TypeQualifiers::CONST) {
                return false;
            }
            if let TypeKind::Array(element, _) = self.types.nodes[ty.index] {
                ty = element;
            } else {
                return true;
            }
        }
    }

    /// A Microsoft `selectany` declaration without an initializer defines
    /// its object, zero-initialized, even when it is `extern`, as Clang
    /// treats it: a later initialized declaration is a redefinition, and
    /// the type must be complete.
    /// MSVC extension: Microsoft Learn, "selectany"; C99: §6.9p3, p. 140;
    /// PDF p. 152; §6.7.8p10, p. 126; PDF p. 138 (zero initialization).
    pub(super) fn select_any_definition(&mut self, index: usize) {
        let b = self.bindings[index];
        let mut entity = self
            .functions
            .entities
            .get(&b.name.name)
            .copied()
            .unwrap_or_default();
        if entity.definition == Some(b.name.source_vectors) {
            return;
        }
        if !self.types.unanalyzed(b.ty) && !self.complete_object(b.ty) {
            self.error(
                SemanticErrorKind::IncompleteTentativeDefinition,
                b.name.source_vectors,
                Some(b.name.name),
                None,
            );
            return;
        }
        let defined = entity.definition.is_some();
        self.define_entity(index, &mut entity);
        _ = self.functions.entities.insert(b.name.name, entity);
        if !defined {
            self.definitions.push(Definition {
                binding: index,
                kind:    DefinitionKind::Tentative,
            });
        }
    }

    /// Rejects multiple definitions of an identifier with linkage.
    /// C99: §6.9 paragraph 3, p. 140; PDF p. 152.
    /// C99: §6.9 paragraph 5, p. 140; PDF p. 152.
    fn define_entity(&mut self, index: usize, entity: &mut Entity) {
        let b = self.bindings[index];
        if let Some(previous) = entity.definition {
            self.error(
                SemanticErrorKind::DuplicateDefinition,
                b.name.source_vectors,
                Some(b.name.name),
                Some(previous),
            );
        } else {
            entity.definition = Some(b.name.source_vectors);
        }
    }

    /// C99: §6.7.4p6, pp. 112-113; PDF pp. 124-125; §6.9p3, p. 140;
    /// PDF p. 152. Internal bodies satisfy the definition requirement even
    /// when GNU inline semantics allow replacing them.
    /// Microsoft ABI extension under C99: §4p6, p. 7; PDF p. 19:
    /// Clang's basicGVALinkageForFunction defers an inlined definition
    /// without an explicit extern or applicable `dllexport`, unless
    /// `gnu_inline` applies. Export retention does not implement DLL codegen.
    fn record_function_definition(&mut self, index: usize, f: &'tu FunctionDefinition<'tu>) {
        if self.tainted {
            return;
        }
        let b = self.bindings[index];
        if b.linkage == Linkage::None {
            self.definitions.push(Definition {
                binding: index,
                kind:    DefinitionKind::Function,
            });
            return;
        }
        let mut entity = self
            .functions
            .entities
            .get(&self.function_identity(index))
            .copied()
            .unwrap_or_default();
        entity.latest = index;
        if let Some(FunctionBody::Unique(previous)) = entity.function_body {
            self.error(
                SemanticErrorKind::DuplicateDefinition,
                b.name.source_vectors,
                Some(b.name.name),
                Some(previous),
            );
        } else {
            entity.function_body = Some(if entity.can_redefine {
                FunctionBody::Replaceable(b.name.source_vectors)
            } else {
                FunctionBody::Unique(b.name.source_vectors)
            });
        }
        // Clang's FunctionDecl::isInlined for the definition: inline
        // written on it or on an earlier declaration.
        entity.msvc_inline.inline_body = entity.msvc_inline.inline_declared;
        entity.msvc_inline.emission =
            if !self.context.configuration.target().c99_inline_definitions()
                && !entity.gnu_inline
                && entity.msvc_inline.inline_declared
                && !entity.msvc_inline.explicit_extern
                && !entity.msvc_inline.dllexport
            {
                InlineEmission::Discardable
            } else {
                InlineEmission::Required
            };
        // C99 §6.7.4p6 and §6.9p3: an internal inline body supplies the
        // required definition even when GNU semantics permit its replacement.
        if (!entity.can_redefine || b.linkage == Linkage::Internal)
            && (b.linkage == Linkage::Internal
                || !f.declaration_specifiers.function_specifiers.is_inline
                || f.declaration_specifiers.storage_class == Some(StorageClass::Extern)
                || entity.gnu_inline
                || self.context.configuration.gnu89_inline())
        {
            entity.definition = entity.function_body.map(|body| match body {
                | FunctionBody::Replaceable(source) | FunctionBody::Unique(source) => source,
            });
            entity.non_inline = true;
        }
        _ = self
            .functions
            .entities
            .insert(self.function_identity(index), entity);
        // GNU extension: GCC manual, "Inline": extern inline supplies no
        // external definition, even after an ordinary declaration. C99
        // §6.7.4p6-7 instead considers all file-scope declarations.
        // Microsoft retention is finalized separately from the C99/GNU
        // definition constraints; explicit gnu_inline keeps GNU omission.
        self.definitions.push(Definition {
            binding: index,
            kind:    if entity.can_redefine
                && b.linkage == Linkage::External
                && (entity.gnu_inline
                    || self.context.configuration.target().c99_inline_definitions())
            {
                DefinitionKind::Inline
            } else {
                DefinitionKind::Function
            },
        });
    }

    /// Retains uses needed for internal-definition and inline-reference
    /// constraints.
    /// C99: §6.9 paragraph 3, p. 140; PDF p. 152.
    /// C99: §6.7.4 paragraph 3, p. 112; PDF p. 124.
    pub(super) fn record_binding_use(&mut self, binding: usize, name: Identifier) {
        if self.tainted {
            return;
        }
        let dll_import = self.dll_reference(binding);
        let inline_function = self
            .functions
            .current
            .filter(|f| {
                f.syntax
                    .declaration_specifiers
                    .function_specifiers
                    .is_inline
            })
            .and_then(|f| f.binding)
            .filter(|&i| self.bindings[i].linkage == Linkage::External);
        // Clang emits a block-scope static object's declaration wherever it
        // visits the declaration, even after a jump (`EmitSimpleStmt`), so
        // its initializer is skipped only where the code is not visited.
        let static_local = self.functions.initializer.is_some_and(|binding| {
            let data = self.bindings[binding];
            data.linkage == Linkage::None && data.duration.is_static_or_thread()
        });
        self.functions.uses.push(Use {
            reference_context: self.functions.reference_context,
            owner: self
                .functions
                .initializer
                // A thread-local object's initializer is constant data too.
                .filter(|&binding| self.bindings[binding].duration.is_static_or_thread())
                .map(ReferenceOwner::Object)
                .or(self
                    .functions
                    .reference_function
                    .map(ReferenceOwner::Function)),
            binding,
            name,
            sizeof: self.functions.sizeof,
            inline_function,
            dll_import,
            dead: self.functions.dead && (self.functions.unvisited || !static_local),
        });
    }

    /// The number of references recorded so far, which marks where the
    /// references of the code analyzed next begin.
    pub(super) fn reference_count(&self) -> usize {
        self.functions.uses.len()
    }

    /// Marks the references recorded in `range` as skipped by code
    /// generation, except those of nested function bodies, which are
    /// emitted on their own.
    pub(super) fn skip_references(&mut self, range: std::ops::Range<usize>) {
        let function = self.functions.reference_function;
        for usage in &mut self.functions.uses[range] {
            if !matches!(usage.owner, Some(ReferenceOwner::Function(owner)) if Some(owner) != function)
            {
                usage.dead = true;
            }
        }
    }

    /// Marks the references of a generic selection's controlling operand
    /// and of its unselected associations as skipped: Clang's code
    /// generation emits only the selected association. C11: §6.5.1.1p3,
    /// p. 79; PDF p. 97.
    pub(super) fn skip_unselected_generic(
        &mut self,
        generic: &'tu GenericSelection<'tu>,
        selected: Option<&'tu Expression<'tu>>,
    ) {
        let Some(marks) = self
            .functions
            .generic_marks
            .remove(&std::ptr::from_ref(generic).addr())
        else {
            return;
        };
        if marks.len() != 2 * generic.associations.len() + 2 {
            return;
        }
        let mut ranges = ArenaVec::new_in(self.scratch);
        ranges.push(marks[0].0..marks[1].0);
        for (index, association) in generic.associations.iter().enumerate() {
            if selected.is_none_or(|selected| !std::ptr::eq(selected, association.expression)) {
                ranges.push(marks[2 + 2 * index].0..marks[3 + 2 * index].0);
            } else {
                // Code generation continues in the block the selected
                // association ends in.
                self.functions.emission.orphan = marks[3 + 2 * index].1;
            }
        }
        for range in ranges {
            for usage in &mut self.functions.uses[range] {
                usage.dead = true;
            }
        }
    }

    /// Pushes the work that enters operand `arm` of `condition`, taken when
    /// it is `live`: code generation skips it when the condition folds to
    /// the other value (`CodeGenFunction::ConstantFoldsToSimpleInteger`),
    /// unless it holds a label (`CodeGenFunction::ContainsLabel`).
    pub(super) fn dead_arm(
        &mut self,
        condition: &'tu Expression<'tu>,
        live: bool,
        arm: &'tu Expression<'tu>,
    ) {
        self.work.push(Work::FunctionWork(FunctionWork::DeadArm(
            condition,
            live,
            self.skipped_state(),
            arm,
        )));
    }

    /// The skipped state of the code being analyzed, which an operand or
    /// arm starts from.
    pub(super) fn skipped_state(&self) -> Skipped {
        Skipped {
            dead:      self.functions.dead,
            unvisited: self.functions.unvisited,
        }
    }

    /// Loop code generation folds arithmetic truth, including floating
    /// constants, before deciding whether to create a condition exit.
    /// C99: §6.8.5p4, p. 135; PDF p. 147.
    fn generated_condition(&self, condition: &'tu Expression<'tu>) -> Option<bool> {
        let info = self.expression_info(condition);
        (info.constant == super::ConstantClass::Arithmetic)
            .then(|| Self::constant_truth(info))
            .flatten()
    }

    /// A generated `break` gives its innermost switch or loop an exit edge.
    /// A nested target's break cannot make an enclosing infinite loop exit.
    /// C99: §6.8.6.3p2, p. 138; PDF p. 150.
    pub(super) fn generated_break(&mut self) {
        if !self.functions.dead
            && let Some(&(target, _)) = self.control.targets.last()
        {
            self.functions.generated_breaks.push(GeneratedBreak {
                target,
                sizeof: self.functions.sizeof,
                reference_context: self.functions.reference_context,
            });
        }
    }

    fn has_generated_break(&self, target: StatementId) -> bool {
        self.functions.generated_breaks.iter().any(|jump| {
            jump.target == target && self.context_evaluated(jump.sizeof, jump.reference_context)
        })
    }

    /// Whether code in sizeof context `sizeof` and reference context
    /// `context`, both analyzed, is evaluated: neither they nor any context
    /// around them leaves its operand unevaluated.
    pub(super) fn context_evaluated(&self, sizeof: Option<usize>, context: Option<usize>) -> bool {
        let mut sizeof = sizeof;
        while let Some(index) = sizeof {
            if self.sizeof_unevaluated(index) {
                return false;
            }
            sizeof = self.functions.sizeof_contexts[index].parent;
        }
        let mut context = context;
        while let Some(index) = context {
            if self.reference_context_unevaluated(index) {
                return false;
            }
            context = self.functions.reference_contexts[index].parent;
        }
        true
    }

    /// Whether sizeof context `index` leaves its operand unevaluated: its
    /// result is constant, unless the operand has a VM type.
    /// C99: §6.5.3.4p2, p. 80; PDF p. 92.
    fn sizeof_unevaluated(&self, index: usize) -> bool {
        let info = self.expression_info(self.functions.sizeof_contexts[index].expression);
        info.ice || self.types.unanalyzed(info.ty)
    }

    /// Whether reference context `index` leaves its operand unevaluated:
    /// alignof always does, GNU typeof unless its type is VM.
    fn reference_context_unevaluated(&self, index: usize) -> bool {
        match self.functions.reference_contexts[index].kind {
            | ReferenceContextKind::Unevaluated => true,
            | ReferenceContextKind::Typeof(syntax) => self
                .typeof_sites
                .get(&std::ptr::from_ref(syntax).addr())
                .is_none_or(|site| !self.types.variably_modified(site.ty)),
        }
    }

    /// Whether a reference made so far to `entity` is evaluated, as
    /// Clang's `isUsed` records it; an operand still being analyzed counts
    /// as unevaluated. C99: §6.5.3.4p2, p. 80; PDF p. 92.
    pub(super) fn evaluated_reference(&mut self, entity: super::facts::EntityId) -> bool {
        for i in 0..self.functions.uses.len() {
            let usage = self.functions.uses[i];
            if usage.dll_import.is_none() || self.entity_of(usage.binding) != entity {
                continue;
            }
            let mut sizeof = usage.sizeof;
            let mut evaluated = true;
            while let Some(index) = sizeof {
                evaluated &= !self.sizeof_unevaluated(index);
                sizeof = self.functions.sizeof_contexts[index].parent;
            }
            let mut context = usage.reference_context;
            while let Some(index) = context {
                evaluated &= !self.reference_context_unevaluated(index);
                context = self.functions.reference_contexts[index].parent;
            }
            if evaluated {
                return true;
            }
        }
        false
    }

    /// Records, for each entity, whether the unit's last evaluated
    /// reference in emitted code named an imported declaration. Code
    /// generation skips dead code ([`State::dead`]) and bodies it does not
    /// emit: a C99 inline-only (unless weak or `always_inline`) or imported
    /// body, and an internal or
    /// Microsoft discardable inline body that neither GNU `used` nor a
    /// reference from emitted code keeps, as Clang defers such definitions
    /// until their first use. An entity with no such reference, which
    /// Clang never references, stays imported when any evaluated reference
    /// in a body lowering emits named an import, dead or not: lowering
    /// still emits some code Clang skips, after a call that does not return
    /// and in a folded switch's unselected statements, and such a reference
    /// must name the import it was written with to link. Lowering emits a
    /// deferred body that any reference reaches.
    pub(super) fn finish_dll_references(&mut self, symbols: &super::facts::SymbolFacts<'tu>) {
        let evaluated = self.functions.evaluated_imports;
        let mut kinds = ArenaMap::with_hasher_in(FxBuildHasher, self.scratch);
        for i in 0..self.definitions.len() {
            let definition = self.definitions[i];
            let entity = self.entity_of(definition.binding);
            _ = kinds.insert(entity, definition.kind);
        }
        let mut emitted = ArenaMap::with_hasher_in(FxBuildHasher, self.scratch);
        let mut lowered = ArenaMap::with_hasher_in(FxBuildHasher, self.scratch);
        for binding in symbols
            .aliases
            .iter()
            .flat_map(|alias| [alias.binding, alias.target])
            .chain(symbols.init_functions.iter().map(|init| init.binding))
        {
            let entity = self.entity_of(binding);
            _ = emitted.insert(entity, ());
            _ = lowered.insert(entity, ());
        }
        for (reached, live_only) in [(&mut emitted, true), (&mut lowered, false)] {
            let mut changed = true;
            while changed {
                changed = false;
                for &(owner, binding, _, dead) in evaluated {
                    if !(dead && live_only) && self.owner_emitted(owner, &kinds, &*reached, symbols)
                    {
                        let entity = self.entity_of(binding);
                        changed |= reached.insert(entity, ()).is_none();
                    }
                }
            }
        }
        // Any reference lowering emits decides when none in emitted code
        // does, an import over a plain one.
        for &(owner, binding, import, _) in evaluated {
            if self.owner_emitted(owner, &kinds, &lowered, symbols) {
                self.dll_lowered(binding, import);
            }
        }
        for &(owner, binding, import, dead) in evaluated {
            if !dead && self.owner_emitted(owner, &kinds, &emitted, symbols) {
                self.dll_referenced(binding, import);
            }
        }
    }

    /// Whether the body or initializer `owner` is emitted: see
    /// [`Self::finish_dll_references`].
    fn owner_emitted(
        &mut self,
        owner: Option<usize>,
        kinds: &ArenaMap<'_, super::facts::EntityId, DefinitionKind>,
        emitted: &ArenaMap<'_, super::facts::EntityId, ()>,
        symbols: &super::facts::SymbolFacts<'tu>,
    ) -> bool {
        let Some(owner) = owner else { return true };
        let entity = self.entity_of(owner);
        let deferred = match kinds.get(&entity) {
            // Clang emits a weak inline definition, and an `always_inline`
            // one as `available_externally`, once referenced, so their
            // references count; it emits other inline-only bodies only
            // when optimizing, which this unit's facts do not depend on.
            | Some(DefinitionKind::Inline)
                if symbols.entity(entity).is_some_and(|attributes| {
                    attributes.weak || attributes.inlining == super::Inlining::Always
                }) =>
                true,
            | Some(DefinitionKind::Inline) => return false,
            | Some(DefinitionKind::DiscardableFunction) => true,
            | _ => self.bindings[owner].linkage == Linkage::Internal,
        };
        !deferred || emitted.contains_key(&entity) || self.retained_by_used(entity)
    }

    /// Constant sizeof operands do not require an internal definition.
    /// C99: §6.9p3, p. 140; PDF p. 152.
    pub(super) fn enter_sizeof(&mut self, expression: &'tu Expression<'tu>) {
        self.work
            .push(Work::FunctionWork(FunctionWork::RestoreSizeof(
                self.functions.sizeof,
            )));
        let index = self.functions.sizeof_contexts.len();
        self.functions.sizeof_contexts.push(SizeofContext {
            expression,
            parent: self.functions.sizeof,
        });
        self.functions.sizeof = Some(index);
    }

    /// Retains evaluation contexts for emission and internal-definition use
    /// tracking. GNU typeof executes an expression only for VM types;
    /// alignof never evaluates its operand. C99: §6.7.5.2p5, p. 117;
    /// PDF p. 129 (VM types); GCC manual, "Typeof".
    pub(super) fn enter_reference_context(
        &mut self,
        syntax: Option<&'tu super::ExtendedType<'tu>>,
    ) {
        self.work
            .push(Work::FunctionWork(FunctionWork::RestoreReferenceContext(
                self.functions.reference_context,
            )));
        let index = self.functions.reference_contexts.len();
        self.functions.reference_contexts.push(ReferenceContext {
            kind:   syntax.map_or(
                ReferenceContextKind::Unevaluated,
                ReferenceContextKind::Typeof,
            ),
            parent: self.functions.reference_context,
        });
        self.functions.reference_context = Some(index);
    }

    /// C99: §6.7.4p6, pp. 112-113; PDF pp. 124-125, unless the target's ABI
    /// emits inline functions as external definitions or the function is
    /// exported, which makes a MinGW inline body an external `weak_odr`
    /// one, as `external_function_kind` keeps it.
    fn is_inline_definition(&mut self, binding: usize) -> bool {
        self.context.configuration.target().c99_inline_definitions()
            && self.inline_only_declarations(binding)
            && self.dll_storage(binding) != Some(super::DllStorage::Export)
    }

    /// Whether Clang's code generation emits definition `f` of `binding`
    /// where it appears rather than after the unit: a retained definition,
    /// or a non-discardable external definition by the declarations seen
    /// so far (`CodeGenModule::EmitGlobal`, `MustBeEmitted`).
    /// C99: §6.7.4 paragraph 6, pp. 112-113; PDF pp. 124-125.
    fn emitted_where_defined(&mut self, binding: usize, _f: &FunctionDefinition<'tu>) -> bool {
        let entity = self.entity_of(binding);
        self.retained_by_used(entity)
            || self.attributes.init_function(entity)
            || (self.bindings[binding].linkage == Linkage::External
                && self
                    .functions
                    .entities
                    .get(&self.function_identity(binding))
                    .is_some_and(|entity| self.body_external(entity)))
    }

    /// Whether every file-scope declaration selects a C99 inline-only
    /// body, independently of the target's external-inline ABI rule.
    /// C99: §6.7.4p6, pp. 112-113; PDF pp. 124-125.
    fn inline_only_declarations(&self, binding: usize) -> bool {
        self.functions
            .entities
            .get(&self.function_identity(binding))
            .is_some_and(|e| {
                !e.non_inline && !e.gnu_inline && !self.context.configuration.gnu89_inline()
            })
    }

    /// Clang permits replacing a GNU extern inline declaration.
    /// GNU extension: GCC manual, "Inline".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Inline.html>
    pub(super) fn gnu_function_redefinable(&self, name: StringCacheId) -> bool {
        self.functions
            .entities
            .get(&name)
            .is_some_and(|e| e.can_redefine)
    }

    /// Reads recognized entity attribute names for GNU inline semantics and
    /// Microsoft export retention. Names in arguments and unknown namespaces
    /// do not count. GNU/MSVC extensions under C99: §4p6, p. 7; PDF p. 19.
    /// <https://clang.llvm.org/docs/AttributeReference.html#gnu-inline>
    /// <https://learn.microsoft.com/en-us/cpp/cpp/dllexport-dllimport>
    fn has_function_attribute(
        &self,
        spec: DeclarationSpecifiers<'tu>,
        declarator: Declarator<'tu>,
        name: &str,
    ) -> bool {
        // The catalog's namespace-aware reader also recognizes GNU-scoped
        // standard attributes and the Microsoft declspec spelling.
        let attribute_has = |attribute: &super::AttributeSpecifier<'tu>| {
            !attribute.recovered
                && super::attributes::known_names(attribute, self.context)
                    .any(|(_, attribute_name)| attribute_name == name)
        };
        let mut chain = spec.extensions;
        while let Some(item) = chain {
            if let super::SpecifierExtensionKind::Attributes(attribute) = item.kind
                && attribute_has(attribute)
            {
                return true;
            }
            chain = item.next;
        }
        declarator
            .kind
            .iter()
            .any(|direct| matches!(direct, DirectDeclarator::Attributes(a) if attribute_has(a)))
    }
}

/// The active function definition, result type and adjusted parameters.
/// C99: §6.9.1 paragraphs 2-7, pp. 141-142; PDF pp. 153-154.
#[derive(Clone, Copy)]
pub(super) struct FunctionContext<'tu> {
    pub(super) id:         usize,
    pub(super) syntax:     &'tu FunctionDefinition<'tu>,
    pub(super) binding:    Option<usize>,
    pub(super) result:     TypeId,
    pub(super) parameters: &'tu [Parameter<'tu>],
    /// The function scope, which holds the predefined names.
    pub(super) scope:      usize,
    /// The `__func__` object, shared by `__FUNCTION__` as Clang shares them.
    pub(super) name:       Option<usize>,
    /// The `__PRETTY_FUNCTION__` object, made at its first use.
    pub(super) pretty:     Option<usize>,
}

#[derive(Clone, Copy)]
pub(super) enum FunctionWork<'tu> {
    Finish(
        Option<FunctionContext<'tu>>,
        usize,
        Option<usize>,
        Option<usize>,
        usize,
    ),
    RestoreSizeof(Option<usize>),
    RestoreReferenceContext(Option<usize>),
    /// Sets whether code generation skips the code that follows.
    RestoreDead(bool),
    /// Sets whether code generation visits the code that follows
    /// ([`State::unvisited`]).
    RestoreUnvisited(bool),
    /// Restores whether the code being analyzed is emitted into a block
    /// without predecessors ([`super::emission::State::orphan`]).
    RestoreOrphan(Option<usize>),
    /// Restores which weak names apply to the code being analyzed
    /// ([`super::emission::State::weak_horizon`]).
    RestoreWeakHorizon(usize),
    /// Restores that state after operands code generation may not emit,
    /// unless they joined paths, which leaves a block with predecessors.
    RestoreUnjoinedOrphan(Option<usize>),
    /// Restores the skipped state around the innermost switch.
    RestoreSwitchDead(bool),
    /// After a switch: code generation resumes with the code around it,
    /// unless a folded switch's selected statements ended skipped.
    SwitchEnd(bool),
    /// Saves the skipped state at the end of an `if` statement's first arm.
    ThenEnd,
    /// Skips the code after an `if` statement when both arms end skipped.
    JoinArms,
    /// Enters operand `arm` of a condition, taken when it is `live`, given
    /// the skipped state of the code around the condition; an operand that
    /// holds a label is not skipped.
    DeadArm(&'tu Expression<'tu>, bool, Skipped, &'tu Expression<'tu>),
    /// The reference boundaries and insertion states of a conditional's
    /// arms, before its result type settles whether either is skipped.
    ConditionalMark(&'tu Expression<'tu>, usize, Skipped),
    /// Enters an `if` statement's arm, as [`Self::DeadArm`] does, unless
    /// the arm holds a label, which keeps Clang from skipping it.
    DeadStatementArm(
        &'tu Expression<'tu>,
        bool,
        Skipped,
        Option<&'tu Statement<'tu>>,
    ),
    /// Clang omits an infinite loop's exit block unless a generated break
    /// targets it. The loop's body is emitted even for a false condition.
    LoopEnd(&'tu Statement<'tu>, bool),
    /// Records where the references of a generic selection's next operand
    /// begin or end ([`State::generic_marks`]).
    GenericMark(&'tu GenericSelection<'tu>),
    /// After an expression statement: Clang's code generation erases the
    /// block without predecessors that a call that does not return left,
    /// and skips what follows. `serial` numbers the statement; `value` is
    /// the skipped state and reference count before a statement
    /// expression's value statement ([`super::emission`]).
    ExpressionStatementEnd {
        expression: &'tu Expression<'tu>,
        serial:     usize,
        value:      Option<(bool, usize)>,
    },
    /// After a statement expression, given the skipped state around it.
    StatementExpressionEnd(bool),
}

/// Definition and tentative-definition state for an identifier with linkage.
/// GNU extension: GCC manual, "Inline".
/// <https://gcc.gnu.org/onlinedocs/gcc/Inline.html>
/// C99: §6.9 paragraphs 3-5, p. 140; PDF p. 152.
/// C99: §6.9.2 paragraph 2, p. 143; PDF p. 155.
#[derive(Clone, Copy, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Each flag is an independent declaration fact of the entity."
)]
pub(super) struct Entity {
    latest:           usize,
    definition:       Option<SourceVectors>,
    tentative:        Option<usize>,
    non_inline:       bool,
    function_body:    Option<FunctionBody>,
    msvc_inline:      MsvcInlineState,
    /// Clang's canRedefineFunction: GNU extern inline declarations permit
    /// a later static declaration and a replacement function body.
    can_redefine:     bool,
    gnu_inline:       bool,
    /// A file-scope declaration says `inline` without `extern`, which
    /// makes a GNU extern inline body an external definition.
    gnu_plain_inline: bool,
}

/// Source facts and the current emission decision for Microsoft inline
/// definitions. Inline inherits forward; explicit extern includes linked
/// block-scope declarations; `dllexport` applies only at or before the body
/// (`attributes/dll.rs`). `inline_body` says the body itself is inline.
/// Microsoft ABI extension under C99: §4p6, p. 7; PDF p. 19.
#[derive(Clone, Copy, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Each flag is an independent source fact of the inline function."
)]
struct MsvcInlineState {
    inline_declared: bool,
    inline_body:     bool,
    explicit_extern: bool,
    dllexport:       bool,
    emission:        InlineEmission,
}

/// Whether an external Microsoft inline body is an emission root or waits
/// for a reachable reference. This is independent of C99 definition state.
/// Microsoft ABI extension under C99: §4p6, p. 7; PDF p. 19.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum InlineEmission {
    #[default]
    Required,
    Discardable,
}

/// GNU extern inline bodies may be replaced; ordinary definitions may not.
/// Extension: Clang's canRedefineFunction; C99: §6.9p5, p. 140; PDF p. 152.
#[derive(Clone, Copy)]
enum FunctionBody {
    Replaceable(SourceVectors),
    Unique(SourceVectors),
}

/// Whether code generation skips the code being analyzed, and whether it
/// does not even visit it ([`State::dead`], [`State::unvisited`]).
#[derive(Clone, Copy)]
pub(super) struct Skipped {
    pub(super) dead:      bool,
    pub(super) unvisited: bool,
}

#[derive(Clone, Copy)]
struct Use {
    reference_context: Option<usize>,
    owner:             Option<ReferenceOwner>,
    binding:           usize,
    name:              Identifier,
    sizeof:            Option<usize>,
    inline_function:   Option<usize>,
    /// Whether the reference named an imported declaration, for an entity
    /// with linkage.
    dll_import:        Option<bool>,
    /// Code generation skips the reference ([`State::dead`]).
    dead:              bool,
}

/// A break's evaluation context settles after an enclosing sizeof/typeof
/// operand is typed, just as a generated reference's context does.
/// C99: §6.5.3.4p2, p. 80; PDF p. 92.
#[derive(Clone, Copy)]
struct GeneratedBreak {
    target:            StatementId,
    sizeof:            Option<usize>,
    reference_context: Option<usize>,
}

#[derive(Clone, Copy)]
struct SizeofContext<'tu> {
    expression: &'tu Expression<'tu>,
    parent:     Option<usize>,
}

#[derive(Clone, Copy)]
struct ReferenceContext<'tu> {
    kind:   ReferenceContextKind<'tu>,
    parent: Option<usize>,
}

#[derive(Clone, Copy)]
enum ReferenceContextKind<'tu> {
    Typeof(&'tu super::ExtendedType<'tu>),
    Unevaluated,
}

/// Parameter type sites execute as part of the definition, before its binding
/// is installed. Resolve their syntactic owner to that binding at completion.
/// C99: §6.9.1p10, p. 142; PDF p. 154.
#[derive(Clone, Copy)]
enum ReferenceOwner {
    Function(usize),
    Object(usize),
}

/// A static object whose declaration is checked against inline restrictions.
/// C99: §6.7.4 paragraph 3, p. 112; PDF p. 124.
#[derive(Clone, Copy)]
struct InlineObject {
    function: usize,
    binding:  usize,
}

pub(super) struct State<'tu, 's> {
    pub(super) reference_context: Option<usize>,
    reference_contexts: ArenaVec<'s, ReferenceContext<'tu>>,
    pub(super) reference_function: Option<usize>,
    pub(super) initializer: Option<usize>,
    pub(super) current: Option<FunctionContext<'tu>>,
    next_function: usize,
    old_names: ArenaMap<'s, (usize, StringCacheId), SourceVectors>,
    pub(super) old_types: ArenaMap<'s, (usize, StringCacheId), TypeId>,
    entities: ArenaMap<'s, StringCacheId, Entity>,
    previous: ArenaMap<'s, usize, SourceVectors>,
    uses: ArenaVec<'s, Use>,
    /// The file-scope declarations that made an inline body seen before
    /// them an external definition, in source order.
    pub(super) external_redeclarations: ArenaVec<'s, usize>,
    /// Evaluated DLL references retained until aliases and init roots resolve.
    evaluated_imports: &'s [(Option<usize>, usize, bool, bool)],
    sizeof_contexts: ArenaVec<'s, SizeofContext<'tu>>,
    pub(super) sizeof: Option<usize>,
    /// Code generation skips the code being analyzed, as Clang's does at
    /// `-O0`: the arm a constant condition does not take, the statements a
    /// folded switch does not select, and statements after a jump, a loop
    /// without an exit or an expression statement that ends in a call that
    /// does not return, until a label (`CodeGenFunction::EmitStmt`;
    /// [`super::emission`]).
    pub(super) dead: bool,
    /// Code generation does not even visit the code being analyzed: a
    /// statement it skips whole after a jump, the arm or operand a folded
    /// condition skips, and what follows a folded switch's selected
    /// statements. Where it visits skipped code, it still emits the
    /// declarations of block-scope static objects (`EmitSimpleStmt`).
    pub(super) unvisited: bool,
    /// The skipped state around the innermost switch, which its case
    /// labels resume.
    pub(super) switch_dead: bool,
    /// The emission model's state ([`super::emission`]).
    pub(super) emission: super::emission::State<'tu, 's>,
    /// The skipped state at the end of each enclosing `if` statement's
    /// first arm.
    arm_ends: ArenaVec<'s, bool>,
    /// Target and evaluation contexts of emitted breaks. Semantic jumps
    /// and references remain independent of this code-generation fact.
    generated_breaks: ArenaVec<'s, GeneratedBreak>,
    inline_objects: ArenaVec<'s, InlineObject>,
    /// The `__func__` and `__PRETTY_FUNCTION__` objects of file scope.
    pub(super) outside: [Option<usize>; 2],
    /// For each generic selection being analyzed, by address, the number
    /// of references recorded at the start and end of its controlling
    /// operand and of each association's expression, with the emission
    /// model's block state there.
    generic_marks: ArenaMap<'s, usize, ArenaVec<'s, (usize, Option<usize>)>>,
}

impl<'s> State<'_, 's> {
    pub(super) fn new(scratch: &'s Bump) -> Self {
        Self {
            reference_context:       None,
            reference_contexts:      ArenaVec::new_in(scratch),
            reference_function:      None,
            initializer:             None,
            current:                 None,
            next_function:           0,
            old_names:               ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            old_types:               ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            entities:                ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            previous:                ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            uses:                    ArenaVec::new_in(scratch),
            external_redeclarations: ArenaVec::new_in(scratch),
            evaluated_imports:       &[],
            sizeof_contexts:         ArenaVec::new_in(scratch),
            sizeof:                  None,
            dead:                    false,
            unvisited:               false,
            switch_dead:             false,
            emission:                super::emission::State::new(scratch),
            arm_ends:                ArenaVec::new_in(scratch),
            generated_breaks:        ArenaVec::new_in(scratch),
            inline_objects:          ArenaVec::new_in(scratch),
            outside:                 [None; 2],
            generic_marks:           ArenaMap::with_hasher_in(FxBuildHasher, scratch),
        }
    }
}

/// Finds the first derivation outward from the identifier, including grouping.
/// A function typedef supplies no such derivation. Identity is the immutable
/// suffix node, not the source span of its containing declarator.
/// C99: §6.7.5p4-6, p. 115; PDF p. 127; §6.9.1p2, p. 141;
/// PDF p. 153.
pub(super) fn function_derivation<'tu>(
    declarator: Declarator<'tu>,
    scratch: &Bump,
) -> Option<&'tu DirectDeclarator<'tu>> {
    enum Step<'tu> {
        Declarator(Declarator<'tu>),
        Direct(&'tu DirectDeclarator<'tu>),
        Pointer,
    }
    let mut work = ArenaVec::new_in(scratch);
    work.push(Step::Declarator(declarator));
    while let Some(step) = work.pop() {
        match step {
            | Step::Declarator(d) => {
                if !d.pointer.levels.is_empty() {
                    work.push(Step::Pointer);
                }
                for direct in d.kind.as_slice().iter().rev() {
                    work.push(Step::Direct(direct));
                }
            },
            | Step::Direct(direct) => match direct {
                | DirectDeclarator::Parenthesized(p) => work.push(Step::Declarator(p.declarator)),
                | DirectDeclarator::Function { .. }
                | DirectDeclarator::KAndRStyleFunction { .. } => return Some(direct),
                | DirectDeclarator::Array { .. } => return None,
                | _ => {},
            },
            | Step::Pointer => return None,
        }
    }
    None
}
