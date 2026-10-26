//! Retained declaration capability gates and symbol queries. Semantic
//! analysis has decoded and propagated assembler requests and ABI names;
//! lowering only refuses unsupported meanings in emitted owners.
//! C99: §6.2.2p2, p. 30; PDF p. 42; §6.9p3-5, p. 140; PDF p. 152.
use super::{
    Construct,
    ENTITY_FACTS,
    LoweringError,
    module_items::Unit,
};
use crate::{
    translation_phases::semantic_analysis::SymbolRequest,
    util::{
        bump::ArenaVec,
        string_cache::StringCacheId,
    },
};

impl<'a, 'tu> Unit<'a, 'tu, '_> {
    /// Refuses the attribute meanings code generation lacks, except those
    /// written in bodies the module does not emit: retained ownership keeps
    /// them from refusing the module, while file declarations still
    /// constrain emitted uses. C99: §6.9 paragraphs 3-5, p. 140; PDF p. 152.
    pub(super) fn check_extensions(&self, errors: &mut ArenaVec<'_, LoweringError>) {
        for refusal in self.sema.symbols.refusals {
            if refusal
                .owner
                .is_some_and(|owner| !self.emits_function(owner))
            {
                continue;
            }
            errors.push(LoweringError::unsupported(
                Construct::Attribute(refusal.capability),
                refusal.source,
            ));
        }
        for refusal in self.sema.entities.refusals {
            if refusal
                .owner
                .is_some_and(|owner| !self.emits_function(owner))
            {
                continue;
            }
            errors.push(if refusal.missing_occurrence {
                LoweringError::missing(
                    "assembler-label declaration occurrence is not retained",
                    refusal.source,
                )
            } else {
                LoweringError::unsupported(Construct::AssemblerLabel, refusal.source)
            });
        }
    }

    pub(super) fn binding_identity(&self, binding: usize) -> StringCacheId {
        self.sema.entity(binding).expect(ENTITY_FACTS).identity
    }

    pub(super) fn symbol_request(&self, binding: usize) -> Option<SymbolRequest<'tu>> {
        self.sema.entity(binding).expect(ENTITY_FACTS).symbol
    }

    pub(super) fn symbol_name(&self, binding: usize) -> &'a str {
        self.sema
            .symbol_name(binding, self.context)
            .expect(ENTITY_FACTS)
    }
}
