//! Final entity and symbol policy. Completion joins retained occurrences,
//! definitions and ABI identities once; consumers keep point-of-use types.
//! GNU assembler labels are decoded here, with capability refusals retained
//! for lowering at the original provenance and emission owner.
//! C99: §6.2.2p2, p. 30; PDF p. 42; §6.9, pp. 140-143; PDF pp. 152-155.

use rustc_hash::FxBuildHasher;

use super::{
    super::{
        Analyzer,
        Binding,
        BindingKind,
        Definition,
        DefinitionKind,
        Inlining,
        Linkage,
        SemanticTranslationUnit,
        TypeId,
        TypeKind,
    },
    EntityId,
    OccurrenceId,
};
use crate::{
    translation_phases::{
        Context,
        SourceVectors,
        parsing::declaration_syntax::{
            Declarator,
            DirectDeclarator,
            Initializer,
        },
        preprocessing::{
            StringTokenType,
            TokenType,
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

/// Finalized entities, independent of emitted-symbol collisions. Distinct
/// entities can request the same assembler name; lowering checks their IR
/// definitions and still keys private inline bodies by semantic identity.
#[derive(Debug)]
pub(crate) struct EntityFacts<'tu> {
    records:             ArenaMap<'tu, EntityId, Entity<'tu>>,
    functions:           ArenaMap<'tu, usize, FunctionPolicy>,
    pub(crate) refusals: &'tu [SymbolRefusal],
}

impl<'tu> EntityFacts<'tu> {
    pub(crate) fn entity(&self, id: EntityId) -> Option<&Entity<'tu>> {
        self.records.get(&id)
    }

    pub(crate) fn function(&self, binding: usize) -> Option<&FunctionPolicy> {
        self.functions.get(&binding)
    }

    #[cfg(test)]
    pub(crate) fn remove_entity(&mut self, id: EntityId) {
        _ = self.records.remove(&id);
    }

    #[cfg(test)]
    pub(crate) fn remove_function(&mut self, binding: usize) {
        _ = self.functions.remove(&binding);
    }

    #[cfg(test)]
    pub(crate) fn clear_initializer(&mut self, id: EntityId) {
        self.records.get_mut(&id).unwrap().initializer = None;
    }

    pub(in crate::translation_phases::semantic_analysis) fn empty(tu: &'tu Bump) -> Self {
        Self {
            records:   ArenaMap::with_hasher_in(FxBuildHasher, tu),
            functions: ArenaMap::with_hasher_in(FxBuildHasher, tu),
            refusals:  &[],
        }
    }

    /// Completes policy after inline/DLL attributes, ABI names and definitions.
    /// No IR capabilities or reachability decisions are made here.
    pub(in crate::translation_phases::semantic_analysis) fn retain(
        context: &Context<'tu>,
        sema: &SemanticTranslationUnit<'tu>,
        scratch: &Bump,
        tu: &'tu Bump,
        requests: &[PendingSymbolRequest<'tu>],
    ) -> Self {
        // Automatic objects and parameters need no symbol/entity completion.
        // Reserve once: bump arenas cannot reclaim discarded hash tables.
        let count = sema
            .bindings
            .iter()
            .filter(|b| b.needs_entity_policy())
            .count();
        let mut records = ArenaMap::with_capacity_and_hasher_in(count, FxBuildHasher, tu);
        let mut functions =
            ArenaMap::with_capacity_and_hasher_in(sema.functions.len(), FxBuildHasher, tu);
        let mut definitions =
            ArenaMap::with_capacity_and_hasher_in(sema.definitions.len(), FxBuildHasher, scratch);
        let mut bindings = ArenaMap::with_capacity_and_hasher_in(count, FxBuildHasher, scratch);
        let mut occurrences = ArenaMap::with_capacity_and_hasher_in(count, FxBuildHasher, scratch);
        for (index, &binding) in sema.bindings.iter().enumerate() {
            if !binding.needs_entity_policy() {
                continue;
            }
            let id = sema.correspondence.entity_of(index, binding);
            let linked = bindings
                .entry(id)
                .or_insert_with(|| ArenaVec::new_in(scratch));
            linked.push(index);
            let extension = sema.function_extensions.function(index);
            let identity = extension.map_or(binding.name.name, |e| e.identity);
            let abi_name = extension.map_or(binding.name.name, |e| e.abi_name);
            let entity = records.entry(id).or_insert(Entity {
                identity,
                abi_name,
                bindings: &[],
                occurrences: &[],
                definition: None,
                initializer: None,
                ty: binding.ty,
                symbol: None,
                function: None,
                body: BodyEmission::Declaration,
                end: FunctionEnd::Indeterminate,
                returns_twice: false,
            });
            // The last occurrence carries the final composite entity type;
            // binding.ty remains the type visible at that occurrence.
            entity.ty = binding.ty;
        }
        for fact in sema.correspondence.declarations {
            if let Some(id) = fact.entity
                && records.contains_key(&id)
            {
                occurrences
                    .entry(id)
                    .or_insert_with(|| ArenaVec::new_in(scratch))
                    .push(fact.id);
            }
        }
        for definition in sema.definitions {
            _ = definitions.insert(definition.binding, definition.kind);
            let id = sema
                .correspondence
                .entity_of(definition.binding, sema.bindings[definition.binding]);
            let Some(entity) = records.get_mut(&id) else {
                continue;
            };
            entity.definition = Some(*definition);
            entity.ty = sema.bindings[definition.binding].ty;
        }
        for fact in sema.correspondence.declarations {
            if let Some(id) = fact.entity
                && let Some(entity) = records.get_mut(&id)
                && entity
                    .definition
                    .is_some_and(|d| Some(d.binding) == fact.binding)
            {
                entity.initializer = fact.initializer;
            }
        }
        let mut refusals = ArenaVec::new_in(tu);
        for request in requests {
            let binding = sema
                .correspondence
                .declaration(request.declarator)
                .and_then(|f| f.binding);
            let owner = request.owner;
            let Some(binding) = binding else {
                refusals.push(SymbolRefusal {
                    owner,
                    source: request.declarator.source_vectors,
                    missing_occurrence: true,
                });
                continue;
            };
            let data = sema.bindings[binding];
            let name = request.name.filter(|_| {
                matches!(data.kind, BindingKind::Object | BindingKind::Function)
                    && (data.linkage != Linkage::None || data.duration.is_static_or_thread())
            });
            let entity = records.get_mut(&sema.correspondence.entity_of(binding, data));
            if let Some(name) = name
                && let Some(entity) = entity
                && entity.symbol.is_none_or(|previous| previous.name == name)
            {
                if entity.symbol.is_none() {
                    entity.symbol = Some(SymbolRequest {
                        name,
                        source: request.source,
                    });
                }
            } else {
                refusals.push(SymbolRefusal {
                    owner,
                    source: request.source,
                    missing_occurrence: false,
                });
            }
        }
        for function in sema.functions {
            let Some(binding) = function.binding else {
                continue;
            };
            let entity = records
                .get_mut(
                    &sema
                        .correspondence
                        .entity_of(binding, sema.bindings[binding]),
                )
                .expect("functions have entities");
            entity.function = Some(binding);
            entity.body = if definitions.get(&binding) == Some(&DefinitionKind::Inline) {
                if !function.assembly
                    && sema
                        .entity_attributes(binding)
                        .is_some_and(|a| a.inlining == Inlining::Always)
                {
                    BodyEmission::PrivateInline
                } else {
                    BodyEmission::InlineOnly
                }
            } else {
                BodyEmission::Definition
            };
            // Preserve the previous C-identifier-only rule even with an asm
            // label or private inline body. Hosted/mode refinements are a
            // separate change (C99 §5.1.2.2.3p1, p. 13; PDF p. 25).
            entity.end = if context.string_cache.at(sema.bindings[binding].name.name) == "main" {
                FunctionEnd::MainZero
            } else {
                FunctionEnd::Indeterminate
            };
            _ = functions.insert(
                binding,
                FunctionPolicy {
                    body: entity.body,
                    end:  entity.end,
                },
            );
        }
        for (id, entity) in &mut records {
            if let Some(definition) = entity.definition
                && let Some(policy) = functions.get(&definition.binding)
            {
                entity.function = Some(definition.binding);
                entity.body = policy.body;
                entity.end = policy.end;
            }
            entity.returns_twice = bindings[id].iter().any(|&binding| {
                sema.entity_attributes(binding)
                    .is_some_and(|a| a.returns_twice)
            }) || entity
                .symbol
                .is_some_and(|r| crate::builtins::returns_twice(r.name))
                && bindings[id].iter().any(|&binding| {
                    sema.bindings[binding].kind == BindingKind::Function
                        && sema.bindings[binding].linkage != Linkage::None
                });
            entity.bindings = tu.alloc_slice_copy(&bindings[id]);
            entity.occurrences = occurrences.get(id).map_or(&[], |v| tu.alloc_slice_copy(v));
        }
        Self {
            records,
            functions,
            refusals: refusals.leak(),
        }
    }
}

impl Binding {
    /// Entities that require symbol completion or static object association.
    /// C99: §6.2.2p2, p. 30; PDF p. 42; §6.2.4p3, p. 32; PDF p. 44.
    pub(crate) fn needs_entity_policy(self) -> bool {
        self.kind == BindingKind::Function
            || self.kind == BindingKind::Object
                && (self.linkage != Linkage::None || self.duration.is_static_or_thread())
    }
}

/// Policy of a particular body. GNU inline replacement bodies share an
/// entity but retain different emission decisions.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FunctionPolicy {
    pub(crate) body: BodyEmission,
    pub(crate) end:  FunctionEnd,
}

/// One C entity's completed state. Initializer placement is supplied by M8;
/// this record only retains the association with its defining occurrence.
#[derive(Debug)]
pub(crate) struct Entity<'tu> {
    pub(crate) identity:      StringCacheId,
    pub(crate) abi_name:      StringCacheId,
    pub(crate) bindings:      &'tu [usize],
    pub(crate) occurrences:   &'tu [OccurrenceId],
    pub(crate) definition:    Option<Definition>,
    pub(crate) initializer:   Option<&'tu Initializer<'tu>>,
    pub(crate) ty:            TypeId,
    pub(crate) symbol:        Option<SymbolRequest<'tu>>,
    /// A known definition, including inline bodies that supply no external
    /// definition.
    pub(crate) function:      Option<usize>,
    pub(crate) body:          BodyEmission,
    pub(crate) end:           FunctionEnd,
    pub(crate) returns_twice: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SymbolRequest<'tu> {
    pub(crate) name:   &'tu str,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Valid requests retain provenance for downstream symbol diagnostics."
        )
    )]
    pub(crate) source: SourceVectors,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SymbolRefusal {
    pub(crate) owner:              Option<usize>,
    pub(crate) source:             SourceVectors,
    pub(crate) missing_occurrence: bool,
}

/// Language permission to supply a body; module reachability remains
/// downstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BodyEmission {
    Declaration,
    Definition,
    InlineOnly,
    PrivateInline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FunctionEnd {
    MainZero,
    Indeterminate,
}

impl<'tu> SemanticTranslationUnit<'tu> {
    pub(crate) fn entity(&self, binding: usize) -> Option<&Entity<'tu>> {
        let data = *self.bindings.get(binding)?;
        self.entities
            .entity(self.correspondence.entity_of(binding, data))
    }

    pub(crate) fn function_policy(&self, binding: usize) -> Option<&FunctionPolicy> {
        self.entities.function(binding)
    }

    /// Open calls use the occurrence's type, never the final composite type.
    /// C99: §6.5.2.2p6, pp. 71-72; PDF pp. 83-84.
    pub(crate) fn unprototyped_external(&self, binding: usize) -> Option<bool> {
        Some(
            matches!(
                self.types.kind(self.bindings.get(binding)?.ty),
                TypeKind::Function {
                    prototype: false,
                    ..
                }
            ) && self.entity(binding)?.function.is_none(),
        )
    }
}

/// Traversal-time request before its declarator has a bound occurrence.
/// The decoded name lives in the TU arena; this continuation lives in scratch.
#[derive(Clone, Copy)]
pub(in crate::translation_phases::semantic_analysis) struct PendingSymbolRequest<'tu> {
    pub(in crate::translation_phases::semantic_analysis) owner: Option<usize>,
    declarator: Declarator<'tu>,
    name: Option<&'tu str>,
    source: SourceVectors,
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Decode at the existing type traversal, then associate the completed
    /// binding at TU finalization. Invalid requests stay capability refusals.
    /// GNU extension: assembler labels extend C99 §6.2.2p2, p. 30; PDF p. 42.
    pub(in crate::translation_phases::semantic_analysis) fn record_symbol_requests(
        &mut self,
        declarator: Declarator<'tu>,
    ) {
        for direct in declarator.kind {
            let DirectDeclarator::AsmLabel(label) = *direct else {
                continue;
            };
            let name = if !label.recovered
                && let Some(TokenType::String(StringTokenType::String(id))) =
                    label.template.map(|t| t.kind)
            {
                self.context
                    .literal_text_in(self.scratch, id, false)
                    .filter(|n| !n.is_empty() && !n.contains('\0') && !n.starts_with('\u{1}'))
                    .map(|n| &*self.types.tu.alloc_str(n))
            } else {
                None
            };
            self.symbol_requests.push(PendingSymbolRequest {
                owner: self.functions.reference_function,
                declarator,
                name,
                source: label.source_vectors,
            });
        }
    }
}
