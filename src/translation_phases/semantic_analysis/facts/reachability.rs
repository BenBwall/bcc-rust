//! Module emission from semantic entity references, with one explicit worklist.
//! No function signatures or bodies are lowered until this plan is complete.
//! C99: §6.7.4p6-7, pp. 112-113; PDF pp. 124-125; §6.9p3-5,
//! p. 140; PDF p. 152; §6.2.4p3, p. 32; PDF p. 44. Omitting unused
//! internal definitions and retaining locals at O0 follow Clang.

use super::super::{
    BindingKind,
    BodyEmission,
    DefinitionKind,
    Linkage,
    ScopeKind,
    SemanticTranslationUnit,
};
use crate::{
    translation_phases::Context,
    util::bump::{
        ArenaMap,
        ArenaSet,
        ArenaVec,
        Bump,
    },
};

impl SemanticTranslationUnit<'_> {
    /// Nondiscardable external definitions are roots, and so are constructors,
    /// destructors, aliases and their targets, and `used` entities. Internal
    /// objects, functions and discardable Microsoft inline bodies are reached
    /// through references from emitted definitions and initializers.
    /// Inline-only bodies supply no external definition, even when referenced:
    /// at O0 calls and addresses continue to name the external declaration.
    /// A private `always_inline` body is emitted when reached, so its own
    /// references, such as another private body or a static helper, are,
    /// and so are those of the inline-only bodies `copied` names, whose
    /// copies lowering emits.
    /// Without optimization, emitted bodies also retain all their explicit
    /// static and thread-local objects. With optimization, only referenced
    /// or `used` locals survive. A `used` local never roots an omitted body.
    /// Initializer references then retain the objects and functions they name.
    /// This emission policy follows Clang; static duration itself is C99:
    /// §6.2.4p3, p. 32; PDF p. 44 (thread duration is a C11 extension).
    /// C99: §6.7.4p6-7, pp. 112-113; PDF pp. 124-125; §6.9p3-5,
    /// p. 140; PDF p. 152. GNU extern inline follows the same emission policy.
    /// Microsoft discardable bodies retain their callable external definition
    /// when reached; this is an ABI extension under §4p6, p. 7; PDF p. 19.
    pub(crate) fn reachable_bindings<'a>(
        &self,
        context: &Context<'_>,
        arena: &'a Bump,
        copied: impl Fn(usize) -> bool,
    ) -> ArenaSet<'a, usize> {
        let mut entities = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena);
        let mut canonical = ArenaVec::new_in(arena);
        for (index, binding) in self.bindings.iter().enumerate() {
            let key = if binding.linkage == Linkage::None {
                Entity::Local(index)
            } else {
                Entity::Linked(self.entity(index).map_or_else(
                    || context.string_cache.at(binding.name.name),
                    |entity| {
                        entity.symbol.map_or_else(
                            || context.string_cache.at(entity.abi_name),
                            |request| request.name,
                        )
                    },
                ))
            };
            canonical.push(*entities.entry(key).or_insert(index));
        }
        let mut heads = ArenaVec::new_in(arena);
        heads.resize(canonical.len(), None);
        let mut edges = ArenaVec::new_in(arena);
        let mut elided = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, arena);
        for definition in self.definitions {
            if definition.elided {
                _ = elided.insert(canonical[definition.binding]);
            }
        }
        let body_emitted = |binding| {
            !self
                .function_policy(binding)
                .is_some_and(|policy| policy.body == BodyEmission::InlineOnly)
                || copied(binding)
        };
        for reference in self.references {
            if reference.dead
                || !body_emitted(reference.owner)
                || elided.contains(&canonical[reference.binding])
            {
                continue;
            }
            let owner = canonical[reference.owner];
            edges.push((canonical[reference.binding], heads[owner]));
            heads[owner] = Some(edges.len() - 1);
        }
        // Scope parents precede their children. Resolve each scope's nearest
        // function once, including nested blocks, without walking ancestors
        // for every local. C99 §6.2.1p4, pp. 29-30; PDF pp. 41-42.
        let mut owners = ArenaVec::new_in(arena);
        owners.resize(self.scopes.len(), None);
        for function in self.functions {
            owners[function.scope] = function.binding;
        }
        for (index, scope) in self.scopes.iter().enumerate() {
            if scope.kind != ScopeKind::Function {
                owners[index] = scope.parent.and_then(|parent| owners[parent]);
            }
        }
        let local_owner = |index: usize| {
            let binding = self.bindings[index];
            (binding.kind == BindingKind::Object
                && binding.linkage == Linkage::None
                && binding.duration.is_static_or_thread())
            .then(|| owners[binding.scope])
            .flatten()
        };
        let used = |binding| {
            self.entity_attributes(binding)
                .is_some_and(|attributes| attributes.used)
        };
        for definition in self.definitions {
            if !matches!(
                definition.kind,
                DefinitionKind::Object | DefinitionKind::Tentative
            ) || definition.elided
                || (context.configuration.optimization().0 && !used(definition.binding))
            {
                continue;
            }
            if let Some(owner) = local_owner(definition.binding).filter(|&owner| {
                body_emitted(owner)
                    || (context.configuration.optimization().0 && used(definition.binding))
            }) {
                let owner = canonical[owner];
                edges.push((canonical[definition.binding], heads[owner]));
                heads[owner] = Some(edges.len() - 1);
            }
        }
        let mut work = ArenaVec::new_in(arena);
        for definition in self.definitions {
            let binding = self.bindings[definition.binding];
            if matches!(
                definition.kind,
                DefinitionKind::Inline | DefinitionKind::DiscardableFunction
            ) {
                continue;
            }
            if binding.linkage == Linkage::External {
                work.push(canonical[definition.binding]);
            }
        }
        // GNU symbol attributes keep definitions without references, as
        // Clang emits them: constructors and destructors run at startup and
        // exit, every alias is emitted and names its target, and `used`
        // retains its entity. GNU extension: GCC manual, "Common Function
        // Attributes" (alias, constructor, destructor, used).
        for init in self.symbols.init_functions {
            work.push(canonical[init.binding]);
        }
        for alias in self.symbols.aliases {
            work.extend([canonical[alias.binding], canonical[alias.target]]);
        }
        for binding in 0..canonical.len() {
            if used(binding) && local_owner(binding).is_none() {
                work.push(canonical[binding]);
            }
        }
        let mut reached = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, arena);
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
        let mut bindings = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, arena);
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
