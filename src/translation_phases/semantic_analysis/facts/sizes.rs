//! Shared size DAGs and explicit runtime-bound sites. Formula construction
//! visits each canonical type once, after nominal layouts are complete, on
//! demand with an explicit stack: an array needs its element's formula, and
//! GNU's record with a member of variable size those of the members whose
//! sizes start its segments, which may be interned after it.
//! C99: §6.7.5.2p5, p. 117; PDF p. 129; §6.5.3.4p2,
//! p. 80; PDF p. 92; §6.9.1p10, p. 142; PDF p. 154.

use rustc_hash::FxBuildHasher;

use super::{
    super::{
        ArrayBound,
        BoundSiteFacts,
        SegmentBase,
        TagKind,
        TypeId,
        TypeKind,
        VariableRecord,
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
    /// The base of each segment after the first of each record of variable
    /// size, by tag and segment (GNU).
    segments:            ArenaMap<'tu, (usize, u32), SizeFormulaId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SizeFormulaId(pub(crate) usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct BoundId(pub(crate) usize);

/// A product shares its inner formula with allocation, sizeof and strides.
/// Constant array extents use the same nodes as runtime extents.
/// C99: §6.5.3.4p2, p. 80; PDF p. 92; §6.5.6p8, p. 83; PDF p. 95.
///
/// GNU's records with a member of variable size add sums, run-time
/// alignment and, for a union, the greater of two sizes.
/// GNU extension: GCC manual, "Variable Length".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SizeFormula {
    Constant(u64),
    Product {
        inner:  SizeFormulaId,
        extent: SizeExtent,
    },
    Sum(SizeFormulaId, SizeFormulaId),
    /// The formula rounded up to a multiple of the power of two.
    AlignUp(SizeFormulaId, u64),
    Max(SizeFormulaId, SizeFormulaId),
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
    /// The scope the bound is written in, which tells the function that
    /// evaluates it and from which a nested function reads it (GNU).
    pub(crate) scope:        usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoundEvaluationSite {
    Declaration(OccurrenceId),
    /// The specifiers of a declaration, which define a record with a member
    /// of variable size (GNU).
    Specifiers,
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

    /// The base of segment `segment` of the record `tag` (GNU), whose
    /// first segment's base is 0.
    pub(crate) fn segment_base(&self, tag: usize, segment: u32) -> Option<SizeFormulaId> {
        self.segments.get(&(tag, segment)).copied()
    }

    /// The nodes a formula node reads.
    pub(crate) fn operands(&self, formula: SizeFormulaId) -> [Option<SizeFormulaId>; 2] {
        match self.formulas[formula.0] {
            | SizeFormula::Constant(_) => [None, None],
            | SizeFormula::Product { inner, .. } | SizeFormula::AlignUp(inner, _) =>
                [Some(inner), None],
            | SizeFormula::Sum(left, right) | SizeFormula::Max(left, right) =>
                [Some(left), Some(right)],
        }
    }

    pub(in crate::translation_phases::semantic_analysis) fn retain(
        types: &TypeInterner<'tu, '_>,
        scratch: &Bump,
    ) -> Self {
        let count = types.nodes.len();
        let mut builder = Builder {
            types,
            formulas: ArenaVec::new_in(types.tu),
            keys: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            segments: ArenaMap::with_hasher_in(FxBuildHasher, types.tu),
            indices: ArenaVec::with_capacity_in(count, scratch),
            built: ArenaVec::with_capacity_in(count, scratch),
        };
        builder.indices.resize(count, None);
        builder.built.resize(count, false);
        let mut stack: ArenaVec<'_, usize> = ArenaVec::new_in(scratch);
        for index in 0..count {
            stack.push(index);
            while let Some(&top) = stack.last() {
                if builder.built[top] {
                    _ = stack.pop();
                    continue;
                }
                let before = stack.len();
                builder.missing(top, &mut stack);
                if stack.len() == before {
                    _ = stack.pop();
                    let formula = builder.build(top);
                    builder.indices[top] = formula;
                    builder.built[top] = true;
                }
            }
        }
        let mut indices = ArenaVec::with_capacity_in(count, types.tu);
        let mut constants: ArenaVec<'_, Option<u64>> = ArenaVec::with_capacity_in(count, types.tu);
        for (index, &kind) in types.nodes.iter().enumerate() {
            let formula = builder.indices[index];
            let constant = match kind {
                | TypeKind::Array(element, ArrayBound::Constant(n)) =>
                    constants[element.index].and_then(|size: u64| size.checked_mul(n)),
                | TypeKind::Array(..) => None,
                | _ => match formula.map(|formula| builder.formulas[formula.0]) {
                    | Some(SizeFormula::Constant(size)) => Some(size),
                    | _ => None,
                },
            };
            constants.push(constant);
            indices.push(formula);
        }
        Self {
            formulas:  builder.formulas.leak(),
            types:     indices.leak(),
            constants: constants.leak(),
            segments:  builder.segments,
        }
    }
}

/// The state of [`SizeFacts::retain`]: the formulas so far, interned, and
/// each type's, `None` until it is built.
struct Builder<'b, 'tu, 's> {
    types:    &'b TypeInterner<'tu, 's>,
    formulas: ArenaVec<'tu, SizeFormula>,
    keys:     ArenaMap<'b, SizeFormula, SizeFormulaId>,
    segments: ArenaMap<'tu, (usize, u32), SizeFormulaId>,
    /// Each type's formula, once [`Self::built`] says it is built.
    indices:  ArenaVec<'b, Option<SizeFormulaId>>,
    built:    ArenaVec<'b, bool>,
}

impl Builder<'_, '_, '_> {
    /// Pushes the types whose formulas the one of type `index` needs and
    /// which are not built yet.
    fn missing(&self, index: usize, stack: &mut ArenaVec<'_, usize>) {
        let mut need = |ty: TypeId| {
            if !self.built[ty.index] {
                stack.push(ty.index);
            }
        };
        match self.types.nodes[index] {
            | TypeKind::Array(element, _) => need(element),
            | TypeKind::Tag(id) =>
                if let Some(record) = self.types.tags[id].variable.get() {
                    for segment in record.segments {
                        if let SegmentBase::After { ty, .. } = segment.base {
                            need(ty);
                        }
                    }
                },
            | _ => {},
        }
    }

    /// The formula of type `index`, whose operands' are built.
    fn build(&mut self, index: usize) -> Option<SizeFormulaId> {
        match self.types.nodes[index] {
            | TypeKind::Array(element, bound) => {
                let inner = self.indices[element.index]?;
                let extent = match bound {
                    | ArrayBound::Constant(count) => SizeExtent::Constant(count),
                    | ArrayBound::Variable(bound) => SizeExtent::Bound(BoundId(bound)),
                    | _ => return None,
                };
                Some(self.intern(SizeFormula::Product { inner, extent }))
            },
            | TypeKind::Tag(id) if let Some(record) = self.types.tags[id].variable.get() =>
                self.record(id, record),
            | _ => self
                .types
                .layout(TypeId {
                    index,
                    qualifiers: super::super::TypeQualifiers::empty(),
                    alignment: 0,
                })
                .map(|layout| self.intern(SizeFormula::Constant(layout.size))),
        }
    }

    /// The size of the record `tag` of variable size, retaining the bases
    /// of its segments.
    fn record(&mut self, tag: usize, record: &VariableRecord<'_>) -> Option<SizeFormulaId> {
        let end = self.intern(SizeFormula::Constant(record.end));
        if self.types.tags[tag].kind == TagKind::Union {
            let mut size = end;
            for segment in record.segments {
                let SegmentBase::After { ty, .. } = segment.base else {
                    return None;
                };
                let member = self.indices[ty.index]?;
                size = self.intern(SizeFormula::Max(size, member));
            }
            return Some(self.align_up(size, record.align));
        }
        let mut bases = ArenaVec::with_capacity_in(record.segments.len() + 1, self.types.tu);
        bases.push(self.intern(SizeFormula::Constant(0)));
        for (index, segment) in record.segments.iter().enumerate() {
            let base = match segment.base {
                | SegmentBase::Aligned {
                    previous,
                    before,
                    align,
                } => {
                    let before = self.intern(SizeFormula::Constant(before));
                    let sum = self.sum(*bases.get(previous as usize)?, before);
                    self.align_up(sum, align)
                },
                | SegmentBase::After { previous, ty } => {
                    let size = self.indices[ty.index]?;
                    self.sum(*bases.get(previous as usize)?, size)
                },
            };
            let segment = u32::try_from(index + 1).ok()?;
            _ = self.segments.insert((tag, segment), base);
            bases.push(base);
        }
        let last = *bases.last()?;
        let size = self.sum(last, end);
        // The last base may be known aligned enough already.
        let known = record.known_alignment(u32::try_from(record.segments.len()).ok()?);
        if record.align <= known && record.end.is_multiple_of(record.align) {
            return Some(size);
        }
        Some(self.align_up(size, record.align))
    }

    /// `left + right`, folded when both are constants or one is 0.
    fn sum(&mut self, left: SizeFormulaId, right: SizeFormulaId) -> SizeFormulaId {
        match (self.formulas[left.0], self.formulas[right.0]) {
            | (SizeFormula::Constant(0), _) => right,
            | (_, SizeFormula::Constant(0)) => left,
            | (SizeFormula::Constant(a), SizeFormula::Constant(b))
                if let Some(sum) = a.checked_add(b) =>
                self.intern(SizeFormula::Constant(sum)),
            | _ => self.intern(SizeFormula::Sum(left, right)),
        }
    }

    /// `formula` rounded up to `align`, folded for a constant or 1.
    fn align_up(&mut self, formula: SizeFormulaId, align: u64) -> SizeFormulaId {
        match self.formulas[formula.0] {
            | _ if align <= 1 => formula,
            | SizeFormula::Constant(size)
                if let Some(size) = super::super::types::align_up(size, align) =>
                self.intern(SizeFormula::Constant(size)),
            | _ => self.intern(SizeFormula::AlignUp(formula, align)),
        }
    }

    fn intern(&mut self, formula: SizeFormula) -> SizeFormulaId {
        *self.keys.entry(formula).or_insert_with(|| {
            let id = SizeFormulaId(self.formulas.len());
            self.formulas.push(formula);
            id
        })
    }
}

/// Borrow the syntax node itself, including callers matching a borrowed
/// operand. Source vectors are provenance and cannot identify a type-name
/// occurrence.
pub(crate) fn type_name_identity(syntax: &TypeName<'_>) -> usize {
    std::ptr::from_ref(syntax).addr()
}
