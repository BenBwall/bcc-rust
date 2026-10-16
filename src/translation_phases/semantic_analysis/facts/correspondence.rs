//! Declaration correspondence retained while bindings are produced. Syntax
//! list identity survives copied declarators; provenance is never a join key.
//! C99: §6.7p3-4, p. 97; PDF p. 109; §6.9.1p10, p. 142; PDF p. 154.

use rustc_hash::FxBuildHasher;

use super::super::{
    Binding,
    BoundSiteFacts,
    Linkage,
    TypeId,
};
use crate::{
    translation_phases::parsing::declaration_syntax::{
        Declarator,
        DirectDeclarator,
        Initializer,
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

/// One record per written declaration, including rejected occurrences.
/// C99: §6.7p3-4, p. 97; PDF p. 109.
#[derive(Debug)]
pub(crate) struct CorrespondenceFacts<'tu> {
    pub(crate) declarations: &'tu [DeclarationFacts<'tu>],
    indices: ArenaMap<'tu, usize, OccurrenceId>,
    /// Linked names to their entity: every declaration of an identifier
    /// with linkage denotes one entity. C99: §6.2.2p2, p. 30; PDF p. 42.
    entities: ArenaMap<'tu, StringCacheId, EntityId>,
    identities: ArenaMap<'tu, usize, StringCacheId>,
    pub(crate) parameter_minimums: ArenaMap<'tu, usize, super::super::ArrayBound>,
    #[cfg(test)]
    pub(crate) work: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct OccurrenceId(pub(crate) usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct EntityId(pub(crate) usize);

/// The initializer association precedes the placement plan supplied by M8.
/// A rejected occurrence has no accepted binding/entity; it remains
/// inspectable. C99: §6.7p3-4, p. 97; PDF p. 109; §6.7.8, pp. 125-130; PDF pp.
/// 137-142.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DeclarationFacts<'tu> {
    pub(crate) id:              OccurrenceId,
    pub(crate) binding:         Option<usize>,
    pub(crate) entity:          Option<EntityId>,
    pub(crate) initializer:     Option<&'tu Initializer<'tu>>,
    /// Prototype declarations retain their parameter correspondence too.
    /// Definitions consume their completed `FunctionRecord` instead.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Prototype-only correspondence is queryable but has no executable body."
        )
    )]
    pub(crate) parameter_facts: &'tu [ParameterFacts<'tu>],
    pub(crate) site:            BoundSiteFacts<'tu>,
}

/// Ordered incoming parameter correspondence. Invalid/unnamed parameters keep
/// their ordinal with absent bindings; no source-position join is necessary.
/// C99: §6.7.5.3p7-8, p. 119; PDF p. 131; §6.9.1p7,10,
/// p. 142; PDF p. 154.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ParameterFacts<'tu> {
    pub(crate) prototype_binding: Option<usize>,
    pub(crate) body_binding:      Option<usize>,
    pub(crate) declared:          TypeId,
    pub(crate) promoted:          TypeId,
    pub(crate) original:          TypeId,
    /// Written entry sites may be shared by K&R declarators in one declaration.
    pub(crate) site:              Option<BoundSiteFacts<'tu>>,
}

impl<'tu> CorrespondenceFacts<'tu> {
    /// The entity a binding denotes: its own occurrence without linkage,
    /// else the entity its linked name was first declared with.
    pub(crate) fn entity_of(&self, index: usize, binding: Binding) -> EntityId {
        if binding.linkage == Linkage::None {
            EntityId(index)
        } else {
            self.entities
                .get(
                    &self
                        .identities
                        .get(&index)
                        .copied()
                        .unwrap_or(binding.name.name),
                )
                .copied()
                .unwrap_or(EntityId(index))
        }
    }

    pub(crate) fn declaration(
        &self,
        declarator: Declarator<'tu>,
    ) -> Option<&DeclarationFacts<'tu>> {
        let id = self.indices.get(&identity(declarator))?;
        let fact = self.declarations.get(id.0)?;
        debug_assert_eq!(fact.id, *id, "occurrence indices select their own records");
        debug_assert_eq!(
            fact.binding.is_some(),
            fact.entity.is_some(),
            "accepted occurrences retain an entity"
        );
        Some(fact)
    }
}

pub(in crate::translation_phases::semantic_analysis) struct CorrespondenceBuilder<'tu, 's> {
    declarations: ArenaVec<'tu, DeclarationFacts<'tu>>,
    indices: ArenaMap<'tu, usize, OccurrenceId>,
    entities: ArenaMap<'tu, StringCacheId, EntityId>,
    identities: ArenaMap<'tu, usize, StringCacheId>,
    _scratch: std::marker::PhantomData<&'s ()>,
    pub(in crate::translation_phases::semantic_analysis) parameter_minimums:
        ArenaMap<'tu, usize, super::super::ArrayBound>,
    #[cfg(test)]
    work: usize,
}

impl<'tu, 's> CorrespondenceBuilder<'tu, 's> {
    pub(in crate::translation_phases::semantic_analysis) fn identify(
        &mut self,
        binding: usize,
        name: StringCacheId,
    ) {
        _ = self.identities.insert(binding, name);
    }

    pub(in crate::translation_phases::semantic_analysis) fn new(
        tu: &'tu Bump,
        _scratch: &'s Bump,
    ) -> Self {
        Self {
            declarations:       ArenaVec::new_in(tu),
            indices:            ArenaMap::with_hasher_in(FxBuildHasher, tu),
            entities:           ArenaMap::with_hasher_in(FxBuildHasher, tu),
            identities:         ArenaMap::with_hasher_in(FxBuildHasher, tu),
            _scratch:           std::marker::PhantomData,
            parameter_minimums: ArenaMap::with_hasher_in(FxBuildHasher, tu),
            #[cfg(test)]
            work:               0,
        }
    }

    pub(in crate::translation_phases::semantic_analysis) fn entity(
        &mut self,
        index: usize,
        binding: Binding,
    ) -> EntityId {
        if binding.linkage == Linkage::None {
            EntityId(index)
        } else {
            *self
                .entities
                .entry(
                    self.identities
                        .get(&index)
                        .copied()
                        .unwrap_or(binding.name.name),
                )
                .or_insert(EntityId(index))
        }
    }

    pub(in crate::translation_phases::semantic_analysis) fn declaration(
        &mut self,
        mut syntax: Declarator<'tu>,
        binding: Option<(usize, Binding)>,
        site: BoundSiteFacts<'tu>,
        parameter_facts: &'tu [ParameterFacts<'tu>],
    ) -> OccurrenceId {
        let id = OccurrenceId(self.declarations.len());
        let entity = binding.map(|(index, binding)| self.entity(index, binding));
        self.declarations.push(DeclarationFacts {
            id,
            binding: binding.map(|(index, _)| index),
            entity,
            initializer: None,
            parameter_facts,
            site,
        });
        loop {
            #[cfg(test)]
            {
                self.work += 1;
            }
            // Named suffix lists are nonempty arena blocks. Empty abstract
            // declarators have no declaration occurrence to index.
            if !syntax.kind.is_empty() {
                _ = self.indices.insert(identity(syntax), id);
            }
            let child = syntax.kind.iter().find_map(|direct| {
                if let DirectDeclarator::Parenthesized(child) = direct {
                    Some(child.declarator)
                } else {
                    None
                }
            });
            let Some(child) = child else {
                break;
            };
            syntax = child;
        }
        id
    }

    pub(in crate::translation_phases::semantic_analysis) fn initializer(
        &mut self,
        syntax: Declarator<'tu>,
        initializer: &'tu Initializer<'tu>,
    ) {
        if let Some(id) = self.indices.get(&identity(syntax)) {
            self.declarations[id.0].initializer = Some(initializer);
        }
    }

    pub(in crate::translation_phases::semantic_analysis) fn retain(
        self,
    ) -> CorrespondenceFacts<'tu> {
        CorrespondenceFacts {
            declarations:       self.declarations.leak(),
            indices:            self.indices,
            entities:           self.entities,
            identities:         self.identities,
            parameter_minimums: self.parameter_minimums,
            #[cfg(test)]
            work:               self.work,
        }
    }
}

fn identity(declarator: Declarator<'_>) -> usize {
    declarator.kind.as_slice().as_ptr().addr()
}
