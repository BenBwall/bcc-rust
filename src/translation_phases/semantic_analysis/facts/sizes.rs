//! Shared size DAGs and explicit runtime-bound sites. Formula construction
//! visits each canonical type once, after nominal layouts are complete.
//! C99: §6.7.5.2p5, p. 117; PDF p. 129; §6.5.3.4p2,
//! p. 80; PDF p. 92; §6.9.1p10, p. 142; PDF p. 154.

use rustc_hash::FxBuildHasher;

use super::{
    super::{
        ArrayBound,
        BoundSiteFacts,
        TypeId,
        TypeKind,
        types::TypeInterner,
    },
    OccurrenceId,
};
use crate::{
    target::Scalar,
    translation_phases::parsing::{
        declaration_syntax::TypeName,
        syntax::Expression,
    },
    util::bump::{
        ArenaMap,
        ArenaVec,
        Bump,
    },
};

/// Runtime formulas contain semantic bound identities, never SSA values.
#[derive(Debug)]
pub(crate) struct SizeFacts<'tu> {
    pub(crate) formulas: &'tu [SizeFormula],
    types:               &'tu [Option<SizeFormulaId>],
    constants:           &'tu [Option<u64>],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SizeFormulaId(pub(crate) usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct BoundId(pub(crate) usize);

/// A product shares its inner formula with allocation, sizeof and strides.
/// Constant array extents use the same nodes as runtime extents.
/// C99: §6.5.3.4p2, p. 80; PDF p. 92; §6.5.6p8, p. 83; PDF p. 95.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SizeFormula {
    Constant(u64),
    Product {
        inner:  SizeFormulaId,
        extent: SizeExtent,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SizeExtent {
    Constant(u64),
    Bound(BoundId),
}

/// Exact evaluation mode of a sizeof occurrence. Constant queries execute
/// neither the operand nor its type-site actions.
/// C99: §6.5.3.4p2, p. 80; PDF p. 92.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SizeQueryFacts<'tu> {
    pub(crate) site:    BoundSiteFacts<'tu>,
    pub(crate) formula: Option<SizeFormulaId>,
    pub(crate) operand: Option<&'tu Expression<'tu>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TypeNameId(pub(crate) usize);

/// A resolved written type name and its type-site size/evaluation record.
/// Unmodeled/recovered operands have no checked size query.
/// C99: §6.7.6, p. 122; PDF p. 134; §6.5.3.4p2, p. 80; PDF p. 92.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TypeNameFacts<'tu> {
    pub(crate) id:         TypeNameId,
    pub(crate) syntax:     &'tu TypeName<'tu>,
    pub(crate) resolved:   TypeId,
    pub(crate) size_query: Option<SizeQueryFacts<'tu>>,
}

/// Origin remains explicit even when an unevaluated prototype bound has no
/// runtime action. Dependencies are saved extents read by its typed expression.
/// C99: §6.7.5.2p5, p. 117; PDF p. 129.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RuntimeBound<'tu> {
    pub(crate) expression:   &'tu Expression<'tu>,
    pub(crate) ty:           TypeId,
    pub(crate) conversion:   Scalar,
    pub(crate) site:         BoundEvaluationSite,
    pub(crate) dependencies: &'tu [BoundId],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoundEvaluationSite {
    Declaration(OccurrenceId),
    Parameter(usize),
    TypeName(usize),
    Unevaluated,
}

impl<'tu> SizeFacts<'tu> {
    pub(crate) fn formula(&self, ty: TypeId) -> Option<SizeFormulaId> {
        self.types.get(ty.index).copied().flatten()
    }

    pub(crate) fn constant_size(&self, ty: TypeId) -> Option<u64> {
        self.constants.get(ty.index).copied().flatten()
    }

    pub(in crate::translation_phases::semantic_analysis) fn retain(
        types: &TypeInterner<'tu, '_>,
        scratch: &Bump,
    ) -> Self {
        let mut formulas = ArenaVec::new_in(types.tu);
        let mut keys = ArenaMap::with_hasher_in(FxBuildHasher, scratch);
        let mut indices: ArenaVec<'_, Option<SizeFormulaId>> = ArenaVec::new_in(types.tu);
        let mut constants: ArenaVec<'_, Option<u64>> = ArenaVec::new_in(types.tu);
        for (index, &kind) in types.nodes.iter().enumerate() {
            let formula = match kind {
                | TypeKind::Array(element, bound) =>
                    indices[element.index].and_then(|inner| match (formulas[inner.0], bound) {
                        | (_, ArrayBound::Constant(count)) => Some(SizeFormula::Product {
                            inner,
                            extent: SizeExtent::Constant(count),
                        }),
                        | (_, ArrayBound::Variable(bound)) => Some(SizeFormula::Product {
                            inner,
                            extent: SizeExtent::Bound(BoundId(bound)),
                        }),
                        | _ => None,
                    }),
                | _ => types
                    .layout(TypeId {
                        index,
                        qualifiers: super::super::TypeQualifiers::empty(),
                        alignment: 0,
                    })
                    .map(|layout| SizeFormula::Constant(layout.size)),
            };
            let constant = match kind {
                | TypeKind::Array(element, ArrayBound::Constant(count)) =>
                    constants[element.index].and_then(|size| size.checked_mul(count)),
                | TypeKind::Array(..) => None,
                | _ =>
                    if let Some(SizeFormula::Constant(size)) = formula {
                        Some(size)
                    } else {
                        None
                    },
            };
            constants.push(constant);
            let id = formula.map(|formula| {
                *keys.entry(formula).or_insert_with(|| {
                    let id = SizeFormulaId(formulas.len());
                    formulas.push(formula);
                    id
                })
            });
            indices.push(id);
        }
        Self {
            formulas:  formulas.leak(),
            types:     indices.leak(),
            constants: constants.leak(),
        }
    }
}

/// Borrow the syntax node itself, including callers matching a borrowed
/// operand. Source vectors are provenance and cannot identify a type-name
/// occurrence.
pub(crate) fn type_name_identity(syntax: &TypeName<'_>) -> usize {
    std::ptr::from_ref(syntax).addr()
}
