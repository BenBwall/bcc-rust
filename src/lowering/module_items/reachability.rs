//! Module emission from semantic entity references, with one explicit worklist.
//! No function signatures or bodies are lowered until this plan is complete.
//! C99: §6.7.4p6-7, pp. 112-113; PDF pp. 124-125; §6.9p3-5,
//! p. 140; PDF p. 152. Omitting unused internal definitions follows Clang.

use super::{
    ArenaMap,
    ArenaSet,
    ArenaVec,
    DefinitionKind,
    Linkage,
    Unit,
};

impl<'a> Unit<'a, '_, '_> {
    /// External definitions are roots. Internal objects and functions are
    /// reached through references from emitted definitions and initializers.
    /// Inline-only bodies supply no external definition, even when referenced:
    /// at O0 calls and addresses continue to name the external declaration.
    /// C99: §6.7.4p6-7, pp. 112-113; PDF pp. 124-125; §6.9p3-5,
    /// p. 140; PDF p. 152. GNU extern inline follows the same emission policy.
    pub(super) fn reachable_items(&self) -> ArenaSet<'a, usize> {
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
        let mut heads = ArenaVec::new_in(self.tables);
        heads.resize(canonical.len(), None);
        let mut edges = ArenaVec::new_in(self.tables);
        for reference in self.sema.references {
            if self.definitions.get(&reference.owner) == Some(&DefinitionKind::Inline) {
                continue;
            }
            let owner = canonical[reference.owner];
            edges.push((canonical[reference.binding], heads[owner]));
            heads[owner] = Some(edges.len() - 1);
        }
        let mut work = ArenaVec::new_in(self.tables);
        for definition in self.sema.definitions {
            let binding = self.sema.bindings[definition.binding];
            if definition.kind == DefinitionKind::Inline {
                continue;
            }
            if binding.linkage == Linkage::External {
                work.push(canonical[definition.binding]);
            }
        }
        let mut reached = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, self.tables);
        while let Some(owner) = work.pop() {
            if !reached.insert(owner) {
                continue;
            }
            let mut edge = heads[owner];
            while let Some(index) = edge {
                let (binding, next) = edges[index];
                work.push(binding);
                edge = next;
            }
        }
        let mut bindings = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, self.tables);
        for (binding, entity) in canonical.iter().enumerate() {
            if reached.contains(entity) {
                _ = bindings.insert(binding);
            }
        }
        bindings
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Entity<'a> {
    Linked(&'a str),
    Local(usize),
}
