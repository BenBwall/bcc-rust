//! Parameter facts produced from the checked parameter, never a provenance
//! join. C99: §6.7.5.3p7-8, p. 119; PDF p. 131; §6.9.1p7,10,
//! p. 142; PDF p. 154.

use super::super::{
    Analyzer,
    Parameter,
    ParameterFacts,
    TypeKind,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    pub(in crate::translation_phases::semantic_analysis) fn prototype_parameter_facts(
        &self,
        declarator: crate::translation_phases::parsing::declaration_syntax::Declarator<'tu>,
    ) -> &'tu [ParameterFacts<'tu>] {
        let Some(derivation) =
            super::super::functions::function_derivation(declarator, self.scratch)
        else {
            return &[];
        };
        let Some(&(parameters, _)) = self.parameters.get(&std::ptr::from_ref(derivation).addr())
        else {
            return &[];
        };
        if matches!(parameters, [Parameter { name: None, ty, .. }] if matches!(self.types.nodes[ty.index], TypeKind::Scalar(super::super::Scalar::Void)))
        {
            return &[];
        }
        self.types
            .tu
            .alloc_slice_fill_iter(parameters.iter().map(|parameter| ParameterFacts {
                prototype_binding: parameter.binding,
                body_binding:      None,
                declared:          parameter.ty,
                promoted:          parameter.ty.unqualified(),
                original:          parameter.original,
                site:              parameter.site,
            }))
    }

    /// Retain incoming/body correspondence in parameter order, including holes.
    /// C99: §6.9.1p7,10, p. 142; PDF p. 154.
    pub(in crate::translation_phases::semantic_analysis) fn retain_parameter_facts(
        &mut self,
        id: usize,
        parameters: &[Parameter<'tu>],
        bindings: &[Option<usize>],
    ) {
        let facts = self
            .types
            .tu
            .alloc_slice_fill_iter(bindings.iter().enumerate().map(|(index, &body)| {
                let parameter = parameters[index];
                let site = parameter.site;
                ParameterFacts {
                    prototype_binding: parameter.binding,
                    body_binding: body,
                    declared: body.map_or(parameter.ty, |binding| self.bindings[binding].ty),
                    promoted: parameter.ty.unqualified(),
                    original: parameter.original,
                    site,
                }
            }));
        self.function_records[id].parameter_facts = facts;
    }
}
