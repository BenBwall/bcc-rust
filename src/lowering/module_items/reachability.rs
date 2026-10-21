//! Module emission uses the same entity reachability as semantic BSS checks,
//! and declares `always_inline` bodies in Clang's creation order.
//! C99: §6.9p3-5, p. 140; PDF p. 152.

use super::{
    ArenaMap,
    ArenaSet,
    ArenaVec,
    Definition,
    DefinitionKind,
    Linkage,
    Unit,
};

impl<'a> Unit<'a, '_, '_> {
    /// The bindings the module emits: semantic analysis's reachability,
    /// which also decides its BSS checks, with the plain inline-only bodies
    /// whose copies lowering emits following their references too.
    pub(super) fn reachable_items(&self) -> ArenaSet<'a, usize> {
        self.sema.reachable_bindings_with_locals(
            self.context,
            self.tables,
            |binding| self.plain_inline.contains(&binding),
            self.retain_local_statics,
        )
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
    /// (`CodeGenModule::EmitDeferred`). LLVM's always-inliner visits
    /// functions in this module order, so it decides which recursive inline
    /// calls stay calls; lowering declares `always_inline` bodies in it.
    /// Live references are visited in source order, which is the order
    /// Clang emits the expressions that name them; a dead reference
    /// ([`EntityReference::dead`](crate::translation_phases::semantic_analysis::EntityReference))
    /// creates nothing, as Clang emits no code for it. Indexed by binding.
    pub(super) fn clang_creation_order(&self) -> ArenaVec<'a, Option<usize>> {
        let canonical = self.canonical_bindings();
        // References grouped by owner, in source order.
        let mut starts = ArenaVec::new_in(self.tables);
        starts.resize(canonical.len() + 1, 0usize);
        // Clang's code generation skips dead code, which creates nothing.
        let live = || {
            self.sema
                .references
                .iter()
                .filter(|reference| !reference.dead)
        };
        for reference in live() {
            starts[canonical[reference.owner] + 1] += 1;
        }
        for index in 1..starts.len() {
            starts[index] += starts[index - 1];
        }
        let mut next = starts.clone();
        let mut targets = ArenaVec::new_in(self.tables);
        targets.resize(starts[canonical.len()], 0usize);
        for reference in live() {
            let owner = canonical[reference.owner];
            targets[next[owner]] = canonical[reference.binding];
            next[owner] += 1;
        }
        let mut deferred = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, self.tables);
        for definition in self.sema.definitions {
            if !self.eagerly_emitted(definition) {
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
        for definition in self.sema.definitions {
            let entity = canonical[definition.binding];
            if self.eagerly_emitted(definition) {
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
