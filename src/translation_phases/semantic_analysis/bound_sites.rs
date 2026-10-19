//! Type-site evaluation facts produced during semantic type construction.
//! GNU extension: GCC Typeof evaluates operands of variably modified type.
//! <https://gcc.gnu.org/onlinedocs/gcc/Typeof.html>
//! C99: §6.5.3.4p2, p. 80; PDF p. 92 (runtime size operands).

use super::{
    Analyzer,
    ArrayBound,
    ExtendedType,
    SyntaxOperand,
    TypeId,
    TypeKind,
    TypeSpecifiers,
};
use crate::{
    translation_phases::parsing::syntax::{
        Expression,
        ExpressionType,
    },
    util::bump::ArenaVec,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    pub(super) fn record_bound_origins(
        &mut self,
        bounds: &[usize],
        site: super::BoundEvaluationSite,
    ) {
        for &bound in bounds {
            _ = self.bound_origins.entry(bound).or_insert(site);
        }
    }

    pub(super) fn retain_expression_actions(&mut self, expression: &'tu Expression<'tu>) {
        // Normalize the written type site once. Both effect classification and
        // value/effects lowering consume this same generic prelude record.
        if let ExpressionType::Cast { target_type, .. } = expression.kind
            && let Some(&site) = self
                .type_name_sites
                .get(&super::type_name_identity(target_type))
        {
            _ = self
                .expression_actions
                .insert(std::ptr::from_ref(expression).addr(), site);
        }
    }

    /// Newly written bounds; a typedef's remembered extent is not an action.
    /// C99: §6.7.5.2p5, p. 117; PDF p. 129.
    pub(super) fn bound_actions(&self, mut ty: TypeId, start: usize) -> &'tu [usize] {
        if start == self.types.runtime_bounds.len() || !self.types.variably_modified(ty) {
            return &[];
        }
        let mut bounds = ArenaVec::new_in(self.scratch);
        loop {
            match self.types.nodes[ty.index] {
                | TypeKind::Array(element, bound) => {
                    if let ArrayBound::Variable(index) = bound
                        && index >= start
                    {
                        bounds.push(index);
                    }
                    ty = element;
                },
                | TypeKind::Pointer(element) => ty = element,
                | _ => break,
            }
        }
        self.types.tu.alloc_slice_copy(&bounds)
    }

    pub(super) fn site_bounds(
        &self,
        ty: TypeId,
        start: usize,
        spec: TypeSpecifiers<'tu>,
    ) -> &'tu [usize] {
        let new = self.bound_actions(ty, start);
        let inherited = self.spec_bounds(spec);
        if inherited.is_empty() {
            return new;
        }
        if new.is_empty() {
            return inherited;
        }
        self.types
            .tu
            .alloc_slice_fill_iter(inherited.iter().chain(new).copied())
    }

    pub(super) fn spec_bounds(&self, spec: TypeSpecifiers<'tu>) -> &'tu [usize] {
        if let TypeSpecifiers::Extended(syntax @ ExtendedType::Typeof { .. }) = spec {
            self.typeof_sites
                .get(&std::ptr::from_ref(syntax).addr())
                .map_or(&[], |site| site.bounds)
        } else {
            &[]
        }
    }

    /// Keep execution at the written type site: typedef reuse does not execute
    /// the original typeof operand again. Nested type operands copy their
    /// retained actions; expression operands execute only when their type is
    /// VM. GNU extension: GCC Typeof; C99: §6.7.5.2p5, p. 117;
    /// PDF p. 129 (remembered array extents).
    pub(super) fn finish_typeof(&mut self, syntax: &'tu ExtendedType<'tu>) {
        let ExtendedType::Typeof {
            operand,
            unqualified,
        } = *syntax
        else {
            return;
        };
        let ty = self.operand_type(operand);
        let mut bounds = &[][..];
        let evaluations = if self.types.variably_modified(ty) {
            match operand {
                | SyntaxOperand::Expression(expression) =>
                    self.types.tu.alloc_slice_copy(&[expression]),
                | SyntaxOperand::Type(name) => {
                    let Some(site) = self.type_name_sites.get(&super::type_name_identity(name))
                    else {
                        self.values.push(self.types.unknown());
                        return;
                    };
                    bounds = site.bounds;
                    site.operand_evaluations
                },
            }
        } else {
            &[]
        };
        _ = self.typeof_sites.insert(
            std::ptr::from_ref(syntax).addr(),
            super::BoundSiteFacts {
                ty,
                bounds,
                operand_evaluations: evaluations,
            },
        );
        let ty = if unqualified {
            self.types.unqualified_array(self.types.non_atomic(ty))
        } else {
            ty
        };
        self.values.push(ty);
    }

    /// The producer consumes an immediate specifier identity, never provenance.
    pub(super) fn spec_evaluations(
        &self,
        spec: TypeSpecifiers<'tu>,
    ) -> Option<&'tu [&'tu Expression<'tu>]> {
        if let TypeSpecifiers::Extended(syntax @ ExtendedType::Typeof { .. }) = spec {
            self.typeof_sites
                .get(&std::ptr::from_ref(syntax).addr())
                .map(|site| site.operand_evaluations)
        } else {
            Some(&[])
        }
    }
}
