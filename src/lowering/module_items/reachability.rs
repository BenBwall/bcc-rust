//! Module emission uses the same entity reachability as semantic BSS checks,
//! and declares `always_inline` bodies in Clang's creation order.
//! C99: §6.9p3-5, p. 140; PDF p. 152.

use super::{
    ArenaMap,
    ArenaSet,
    ArenaVec,
    BindingKind,
    Definition,
    DefinitionKind,
    Linkage,
    Unit,
};
use crate::{
    lowering::initializers::{
        self,
        Leaf,
        Storage,
    },
    translation_phases::semantic_analysis::InitOperation,
    util::interval_cover::last_writes,
};

impl<'a> Unit<'a, '_, '_> {
    /// The bindings the module emits: semantic analysis's reachability,
    /// which also decides its BSS checks, with the plain inline-only bodies
    /// whose copies lowering emits following their references too.
    pub(super) fn reachable_items(&self) -> ArenaSet<'a, usize> {
        self.sema
            .reachable_bindings(self.context, self.tables, |binding| {
                self.plain_inline.contains(&binding)
            })
    }
}

impl<'a> Unit<'a, '_, '_> {
    /// For each binding, the first binding of the entity or linked symbol it
    /// names.
    fn canonical_bindings(&self) -> ArenaVec<'a, usize> {
        let mut entities = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, self.tables);
        let mut canonical = ArenaVec::new_in(self.tables);
        for (index, binding) in self.sema.bindings.iter().enumerate() {
            let key = if binding.linkage == Linkage::None {
                Entity::Local(index)
            } else {
                Entity::Linked(self.symbol_name(index))
            };
            canonical.push(*entities.entry(key).or_insert(index));
        }
        canonical
    }

    /// The order in which Clang creates the LLVM function or global of each
    /// entity, by canonical binding: on its first reference, or when its
    /// definition is emitted. Clang emits external definitions (and `used`
    /// ones) where they appear and every other definition (inline, internal,
    /// discardable) at the end of the unit, once referenced: depth first,
    /// each body before the bodies it newly references
    /// (`CodeGenModule::EmitDeferred`). An inline body that a later
    /// declaration makes external is created at that declaration and
    /// emitted with the deferred ones (`EmitGlobal`,
    /// `doesDeclarationForceExternallyVisibleDefinition`). LLVM's
    /// always-inliner visits functions in this module order, so it decides
    /// which recursive inline calls stay calls; lowering declares
    /// `always_inline` bodies in it. Live references are visited in source
    /// order, which is the order Clang emits the expressions that name
    /// them; those of a block-scope static object's initializer belong to
    /// its function, which emits it where it reaches the declaration
    /// (`EmitStaticVarDecl`). A dead reference
    /// ([`EntityReference::dead`](crate::translation_phases::semantic_analysis::EntityReference))
    /// creates nothing, as Clang emits no code for it. Indexed by binding.
    pub(super) fn clang_creation_order(&self) -> ArenaVec<'a, Option<usize>> {
        let canonical = self.canonical_bindings();
        let mut function_scopes = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, self.tables);
        for record in self.sema.functions {
            if let Some(binding) = record.binding {
                _ = function_scopes.insert(record.scope, binding);
            }
        }
        // Scopes are created after their parents. Resolve enclosing
        // functions once rather than climbing each static's ancestry for
        // every reference (quadratic in nested blocks).
        let mut scope_functions = ArenaVec::with_capacity_in(self.sema.scopes.len(), self.tables);
        for (index, scope) in self.sema.scopes.iter().enumerate() {
            let function = function_scopes
                .get(&index)
                .copied()
                .or_else(|| scope.parent.and_then(|parent| scope_functions[parent]));
            scope_functions.push(function);
        }
        let owner_of = |owner: usize| {
            let binding = self.sema.bindings[owner];
            if binding.kind == BindingKind::Object
                && binding.linkage == Linkage::None
                && binding.duration.is_static_or_thread()
            {
                if let Some(function) = scope_functions[binding.scope] {
                    return canonical[function];
                }
            }
            canonical[owner]
        };
        let mut initializer_values =
            ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, self.tables);
        for definition in self.sema.definitions {
            let binding = definition.binding;
            if owner_of(binding) != canonical[binding]
                && let Some(values) = self.materialized_initializer_values(binding)
            {
                drop(initializer_values.insert(binding, values));
            }
        }
        // References grouped by owner, in source order. An analyzed value
        // fully replaced by a later static initializer creates no symbol.
        let mut starts = ArenaVec::new_in(self.tables);
        starts.resize(canonical.len() + 1, 0usize);
        // Clang's code generation skips dead code, which creates nothing.
        let live = || {
            self.sema.references.iter().filter(|reference| {
                !reference.dead
                    && initializer_values
                        .get(&reference.owner)
                        .is_none_or(|values| {
                            reference
                                .initializer_value
                                .is_none_or(|value| values.contains(&value))
                        })
            })
        };
        for reference in live() {
            starts[owner_of(reference.owner) + 1] += 1;
        }
        for index in 1..starts.len() {
            starts[index] += starts[index - 1];
        }
        let mut next = starts.clone();
        let mut targets = ArenaVec::new_in(self.tables);
        targets.resize(starts[canonical.len()], 0usize);
        for reference in live() {
            let owner = owner_of(reference.owner);
            targets[next[owner]] = canonical[reference.binding];
            next[owner] += 1;
        }
        // The entities a later declaration makes external, from where.
        let mut forced = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, self.tables);
        for &binding in self.sema.external_redeclarations {
            _ = forced.insert(canonical[binding]);
        }
        let mut retained = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, self.tables);
        for init in self.sema.symbols.init_functions {
            _ = retained.insert(canonical[init.binding]);
        }
        for definition in self.sema.definitions {
            if self
                .sema
                .entity_attributes(definition.binding)
                .is_some_and(|attributes| attributes.used)
            {
                _ = retained.insert(canonical[definition.binding]);
            }
        }
        let eager = |definition: &Definition| {
            retained.contains(&canonical[definition.binding])
                || (self.eagerly_emitted(definition)
                    && !forced.contains(&canonical[definition.binding]))
        };
        // Plain C99 inline-only bodies are not emitted at O0, even when
        // `used` is written. Weak and always_inline bodies are emitted
        // once referenced; their special copies can affect this order.
        let creates_body = |definition: &Definition| {
            definition.kind != DefinitionKind::Inline
                || self.sema.entity_attributes(definition.binding).is_some_and(|attributes| {
                    attributes.weak || attributes.inlining == crate::translation_phases::semantic_analysis::Inlining::Always
                })
        };
        let mut deferred = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, self.tables);
        for definition in self.sema.definitions {
            if creates_body(definition) && !eager(definition) {
                _ = deferred.insert(canonical[definition.binding]);
            }
        }
        let mut order = Creation {
            ranks:   ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, self.tables),
            defined: ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, self.tables),
            queued:  ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, self.tables),
            queue:   ArenaVec::new_in(self.tables),
        };
        let mut emitted = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, self.tables);
        let mut redeclarations = self.sema.external_redeclarations.iter().peekable();
        for definition in self.sema.definitions {
            // A declaration before this definition that makes an earlier
            // body external creates it and makes it pending.
            while let Some(&binding) =
                redeclarations.next_if(|&&binding| binding < definition.binding)
            {
                order.touch(canonical[binding], &deferred);
            }
            if !creates_body(definition) {
                continue;
            }
            let entity = canonical[definition.binding];
            if eager(definition) {
                order.touch(entity, &deferred);
                if emitted.insert(entity) {
                    for &target in &targets[starts[entity]..starts[entity + 1]] {
                        order.touch(target, &deferred);
                    }
                }
            } else if order.defined.insert(entity)
                && order.ranks.contains_key(&entity)
                && order.queued.insert(entity)
            {
                order.queue.push(entity);
            }
        }
        for &binding in redeclarations {
            order.touch(canonical[binding], &deferred);
        }
        // `EmitDeferred`: each pending body, then at once (depth first) the
        // bodies it made pending.
        let mut stack = ArenaVec::new_in(self.tables);
        stack.push((
            core::mem::replace(&mut order.queue, ArenaVec::new_in(self.tables)),
            0,
        ));
        while let Some((list, index)) = stack.last_mut() {
            let Some(&entity) = list.get(*index) else {
                drop(stack.pop());
                continue;
            };
            *index += 1;
            if !emitted.insert(entity) {
                continue;
            }
            for &target in &targets[starts[entity]..starts[entity + 1]] {
                order.touch(target, &deferred);
            }
            if !order.queue.is_empty() {
                stack.push((
                    core::mem::replace(&mut order.queue, ArenaVec::new_in(self.tables)),
                    0,
                ));
            }
        }
        let mut ranks = ArenaVec::with_capacity_in(canonical.len(), self.tables);
        ranks.extend(
            canonical
                .iter()
                .map(|entity| order.ranks.get(entity).copied()),
        );
        ranks
    }

    /// Initializer values contributing to a static object's final bits.
    /// Reuse lowering's plan replay for brace and union resets. GNU ranges
    /// remain conservative here, without expanding their extent merely to
    /// determine creation order.
    /// C99: §6.7.8 paragraph 19, p. 127; PDF p. 139.
    fn materialized_initializer_values(&self, binding: usize) -> Option<ArenaSet<'a, usize>> {
        let initializer = self.sema.entity(binding)?.initializer?;
        let plan = self.sema.initializers.plan(initializer)?;
        if plan
            .operations
            .iter()
            .any(|operation| matches!(operation, InitOperation::Repeat { .. }))
        {
            return None;
        }
        let (leaves, _) = initializers::leaves(
            self.sema,
            plan.ty,
            initializer,
            Storage::Static,
            self.tables,
        )
        .ok()?;
        let mut ranges = ArenaVec::with_capacity_in(leaves.len(), self.tables);
        for &leaf in &leaves {
            ranges.push(leaf.bits(&self.sema.types).ok()?);
        }
        let mut values = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, self.tables);
        for (_, _, index) in last_writes(&ranges, self.tables) {
            if let Leaf::Scalar { expression, .. } | Leaf::Copy { expression, .. } = leaves[index] {
                _ = values.insert(std::ptr::from_ref(expression).addr());
            }
        }
        Some(values)
    }

    /// Whether Clang emits `definition` where it appears rather than once
    /// referenced: the reachability roots.
    fn eagerly_emitted(&self, definition: &Definition) -> bool {
        let binding = self.sema.bindings[definition.binding];
        (binding.linkage == Linkage::External
            && !matches!(
                definition.kind,
                DefinitionKind::Inline | DefinitionKind::DiscardableFunction
            ))
            || self
                .sema
                .entity_attributes(definition.binding)
                .is_some_and(|attributes| attributes.used)
    }
}

/// [`Unit::clang_creation_order`]'s state.
struct Creation<'a> {
    ranks:   ArenaMap<'a, usize, usize>,
    /// Deferred definitions seen so far.
    defined: ArenaSet<'a, usize>,
    queued:  ArenaSet<'a, usize>,
    /// Deferred definitions referenced since the last emission step.
    queue:   ArenaVec<'a, usize>,
}

impl Creation<'_> {
    /// A reference creates the entity's LLVM value, and makes a deferred
    /// definition already seen pending.
    fn touch(&mut self, entity: usize, deferred: &ArenaSet<'_, usize>) {
        let next = self.ranks.len();
        _ = self.ranks.entry(entity).or_insert(next);
        if deferred.contains(&entity)
            && self.defined.contains(&entity)
            && self.queued.insert(entity)
        {
            self.queue.push(entity);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Entity<'a> {
    Linked(&'a str),
    Local(usize),
}
