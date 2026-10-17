//! Semantic overload identities are assigned at declaration time. ABI names
//! instead number anonymous and repeated local tags at definition signatures
//! and emitted uses; unused declarations and discarded bodies do not consume
//! names. Alias targets are rooted before declaration-only fallback naming.
//! C99: §6.2.2p2, p. 30; PDF p. 42; §6.9p3-5, p. 140; PDF p. 152.
//! <https://clang.llvm.org/docs/AttributeReference.html#overloadable>

use rustc_hash::FxBuildHasher;

use super::super::{
    Analyzer,
    BindingKind,
    DefinitionKind,
    FunctionExtension,
    Inlining,
    Linkage,
    PredicateKind,
    SymbolFacts,
};
use crate::{
    target::Target,
    util::{
        bump::{
            ArenaMap,
            ArenaSet,
            ArenaVec,
        },
        string_cache::StringCacheId,
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    pub(in crate::translation_phases::semantic_analysis) fn finish_function_abi(
        &mut self,
        symbols: &SymbolFacts<'tu>,
    ) {
        if !self
            .function_extensions
            .functions
            .values()
            .any(|extension| {
                extension.overloadable
                    || extension
                        .predicates
                        .iter()
                        .any(|p| p.kind == PredicateKind::Enable)
            })
        {
            return;
        }
        self.function_extensions.anonymous_names.clear();
        self.function_extensions.local_names.clear();
        let mut definitions = ArenaMap::with_hasher_in(FxBuildHasher, self.scratch);
        let mut roots = ArenaVec::new_in(self.scratch);
        let mut root_entities = ArenaSet::with_hasher_in(FxBuildHasher, self.scratch);
        for definition in &self.definitions {
            let binding = definition.binding;
            let entity = self.abi_entity(binding);
            _ = definitions.insert(entity, (binding, definition.kind));
            let data = self.bindings[binding];
            let attributes = symbols.entity(self.correspondence.entity(binding, data));
            let root = (data.linkage == Linkage::External
                && !matches!(
                    definition.kind,
                    DefinitionKind::Inline | DefinitionKind::DiscardableFunction
                ))
                || attributes.is_some_and(|a| a.used);
            if root {
                _ = root_entities.insert(entity);
            }
            // Clang names a definition's signature in source order even if
            // its internal body is deferred and never emitted.
            roots.push(AbiWork::Binding(binding, root));
        }
        for init in symbols.init_functions {
            if root_entities.insert(self.abi_entity(init.binding)) {
                roots.push(AbiWork::Binding(init.binding, true));
            }
        }
        for binding in self.alias_bindings() {
            if root_entities.insert(self.abi_entity(binding)) {
                roots.push(AbiWork::AliasDeclaration(binding));
            }
        }
        roots.sort_unstable_by_key(|work| work.binding());
        let mut heads = ArenaMap::with_hasher_in(FxBuildHasher, self.scratch);
        let mut edges = ArenaVec::new_in(self.scratch);
        for reference in self.references.iter().rev() {
            let owner = self.abi_entity(reference.owner);
            let next = heads.insert(owner, edges.len());
            edges.push((reference.binding, next));
        }
        let mut work = roots;
        let mut queued = root_entities;
        let mut position = 0;
        let eager_end = work.len();
        let mut alias_targets = ArenaMap::with_hasher_in(FxBuildHasher, self.scratch);
        while position < work.len() {
            if position == eager_end && self.alias_bindings().next().is_some() {
                // Every definition signature is known before deferred bodies.
                // Alias events keep their source position in the same queue
                // as ordinary references, without renumbering their targets.
                alias_targets.extend(self.alias_targets());
            }
            let (binding, traverse) = match work[position] {
                | AbiWork::Binding(binding, traverse) => (binding, traverse),
                | AbiWork::AliasDeclaration(binding) => {
                    self.assign_function_abi(binding);
                    work.push(AbiWork::AliasTarget(binding));
                    position += 1;
                    continue;
                },
                | AbiWork::AliasTarget(alias) => {
                    let Some(&target) = alias_targets.get(&alias) else {
                        position += 1;
                        continue;
                    };
                    (target, true)
                },
            };
            let deferred = position >= eager_end;
            position += 1;
            let owner = self.abi_entity(binding);
            // Deduplicate on consumption so an earlier alias event can win
            // over a later ordinary reference to the same deferred body.
            if deferred && !queued.insert(owner) {
                continue;
            }
            self.assign_function_abi(binding);
            if !traverse {
                continue;
            }
            if let Some(&(definition, DefinitionKind::Inline)) = definitions.get(&owner) {
                let data = self.bindings[definition];
                if !symbols
                    .entity(self.correspondence.entity(definition, data))
                    .is_some_and(|a| a.inlining == Inlining::Always)
                {
                    continue;
                }
            }
            let mut edge = heads.get(&owner).copied();
            while let Some(index) = edge {
                let (referenced, next) = edges[index];
                self.assign_function_abi(referenced);
                if let Some(&(definition, _)) = definitions.get(&self.abi_entity(referenced)) {
                    work.push(AbiWork::Binding(definition, true));
                }
                edge = next;
            }
        }
        // Declaration-only APIs still expose a deterministic name to semantic
        // consumers; assigning them last cannot consume an emitted ordinal.
        for binding in 0..self.bindings.len() {
            self.assign_function_abi(binding);
        }
        let mut names = ArenaMap::with_hasher_in(FxBuildHasher, self.scratch);
        let mut collisions = ArenaSet::with_hasher_in(FxBuildHasher, self.scratch);
        for (binding, data) in self.bindings.iter().enumerate() {
            if data.kind != BindingKind::Function {
                continue;
            }
            let identity = self.function_identity(binding);
            let name = self
                .function_extensions
                .abi_names
                .get(&identity)
                .copied()
                .unwrap_or(data.name.name);
            if names
                .get(&name)
                .is_some_and(|&previous| previous != identity)
            {
                _ = collisions.insert(name);
            } else {
                _ = names.insert(name, identity);
            }
        }
        for extension in self.function_extensions.functions.values_mut() {
            let abi_name = self
                .function_extensions
                .abi_names
                .get(&extension.identity)
                .copied()
                .unwrap_or(extension.abi_name);
            // Clang's Microsoft ABI omits enable_if predicates, and unmarked
            // C functions use C names. Preserve distinct bodies only where
            // distinct semantic entities actually collide in that ABI.
            let abi_name = if collisions.contains(&abi_name) {
                extension.identity
            } else {
                abi_name
            };
            *extension = self.types.tu.alloc(FunctionExtension {
                abi_name,
                ..**extension
            });
        }
        for (&identity, name) in &mut self.function_extensions.abi_names {
            if collisions.contains(name) {
                *name = identity;
            }
        }
    }

    fn assign_function_abi(&mut self, binding: usize) {
        let Some(&extension) = self.function_extensions.function(binding) else {
            return;
        };
        if self
            .function_extensions
            .abi_names
            .contains_key(&extension.identity)
        {
            return;
        }
        let data = self.bindings[binding];
        let name = if !extension.overloadable || extension.identity == data.name.name {
            // A failed semantic mangling already reported the unsupported
            // predicate. Do not run it again while assigning emitted names.
            data.name.name
        } else if self.context.configuration.target() == Target::WindowsMsvc {
            self.mangle_msvc_function(
                data.name.name,
                data.ty,
                extension,
                data.linkage == Linkage::Internal,
            )
        } else {
            self.mangle_function(
                data.name.name,
                data.ty,
                extension,
                data.linkage == Linkage::Internal,
            )
        };
        _ = self
            .function_extensions
            .abi_names
            .insert(extension.identity, name);
    }

    fn abi_entity(&self, binding: usize) -> AbiEntity {
        let data = self.bindings[binding];
        if data.linkage == Linkage::None {
            AbiEntity::Local(binding)
        } else {
            AbiEntity::Linked(self.function_identity(binding))
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum AbiEntity {
    Local(usize),
    Linked(StringCacheId),
}

#[derive(Clone, Copy)]
enum AbiWork {
    Binding(usize, bool),
    AliasDeclaration(usize),
    AliasTarget(usize),
}

impl AbiWork {
    fn binding(self) -> usize {
        match self {
            | Self::Binding(binding, _)
            | Self::AliasDeclaration(binding)
            | Self::AliasTarget(binding) => binding,
        }
    }
}
