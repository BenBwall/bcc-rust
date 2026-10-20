//! Building the retained placement plan while the current-object checker
//! runs. The checker reports each placement decision as it makes it; this
//! builder records it as an [`InitOperation`] at the subobject's absolute
//! offset, wraps the operations of a GNU range element in a
//! [`InitOperation::Repeat`] stored at the range's first element, and closes
//! each brace pair's zero scope when its list ends. Nothing here chooses a
//! subobject: the checker's cursors do.
//!
//! C99: §6.7.8 paragraphs 17-22, pp. 126-127; PDF pp. 138-139.
//! GNU extension: GCC manual, "Designated Initializers" (ranges).
//! <https://gcc.gnu.org/onlinedocs/gcc/Designated-Inits.html>

use super::super::{
    Analyzer,
    BitRange,
    InitOperation,
    InitValue,
    InitValueId,
    InitializerPlan,
    Subobject,
    types::{
        TagKind,
        TypeId,
        TypeKind,
    },
};
use crate::{
    translation_phases::{
        SourceVectors,
        parsing::{
            declaration_syntax::TypeQualifiers,
            syntax::Expression,
        },
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// The operations of one initializer, before it is accepted.
pub(super) struct PlanBuilder<'tu, 's> {
    operations:       ArenaVec<'s, InitOperation<'tu>>,
    values:           ArenaVec<'s, InitValue<'tu>>,
    /// Open range repeats: the operation, its offset adjustment and whether
    /// it has more than one element.
    repeats:          ArenaVec<'s, (u32, u64, bool)>,
    /// Bytes from the subobjects the checker's cursors are at, the last
    /// element of each open range, to the first, where operations are stored.
    shift:            u64,
    /// Open repeats with more than one element.
    repeating:        u32,
    /// Operations whose destination is the root array of unknown size, or the
    /// flexible array member the object's own initializer fills; their types
    /// are completed when the checker has seen every element.
    root_patches:     ArenaVec<'s, u32>,
    flexible_patches: ArenaVec<'s, u32>,
    valid:            bool,
}

/// Where an initializer of an array of unknown size stores its elements:
/// into the root object, into the flexible array member of the object's own
/// structure, or nowhere, for an empty list of another flexible array member.
/// GNU extension: GCC manual, "Arrays of Length Zero".
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Completion {
    Complete,
    Root,
    Flexible,
    Skip,
}

impl<'tu, 's> PlanBuilder<'tu, 's> {
    /// The scope index of an operation outside any brace pair: a braced
    /// scalar's.
    pub(super) const NO_SCOPE: u32 = u32::MAX;

    pub(super) fn new_in(scratch: &'s Bump) -> Self {
        Self {
            operations:       ArenaVec::new_in(scratch),
            values:           ArenaVec::new_in(scratch),
            repeats:          ArenaVec::new_in(scratch),
            shift:            0,
            repeating:        0,
            root_patches:     ArenaVec::new_in(scratch),
            flexible_patches: ArenaVec::new_in(scratch),
            valid:            true,
        }
    }

    /// Forgets the previous initializer's operations, keeping the storage's
    /// capacity for the next.
    pub(super) fn reset(&mut self) {
        self.operations.clear();
        self.values.clear();
        self.repeats.clear();
        self.shift = 0;
        self.repeating = 0;
        self.root_patches.clear();
        self.flexible_patches.clear();
        self.valid = true;
    }

    /// A brace pair initializes the aggregate at `destination`; returns its
    /// scope, or [`Self::NO_SCOPE`] for an empty list that stores nothing.
    pub(super) fn zero(
        &mut self,
        destination: Subobject,
        completion: Completion,
        source: SourceVectors,
    ) -> u32 {
        if completion == Completion::Skip {
            return Self::NO_SCOPE;
        }
        let destination = self.at(destination);
        self.push_completed(
            InitOperation::Zero {
                destination,
                end: u32::MAX,
                source,
            },
            completion,
        )
    }

    /// The brace pair whose scope is `zero` has no more elements.
    pub(super) fn close_zero(&mut self, zero: u32) {
        let length = self.length();
        if let Some(InitOperation::Zero { end, .. }) = self.operations.get_mut(zero as usize) {
            *end = length;
        }
    }

    pub(super) fn select(
        &mut self,
        destination: Subobject,
        member: u64,
        scope: u32,
        source: SourceVectors,
    ) {
        let destination = self.at(destination);
        let Ok(member) = u32::try_from(member) else {
            self.valid = false;
            return;
        };
        _ = self.push(InitOperation::Select {
            destination,
            member,
            scope,
            source,
        });
    }

    pub(super) fn store(
        &mut self,
        expression: &'tu Expression<'tu>,
        destination: Subobject,
        bit_field: Option<BitRange>,
        source: SourceVectors,
    ) {
        let value = self.value(expression);
        let destination = self.at(destination);
        _ = self.push(InitOperation::Store {
            value,
            destination,
            bit_field,
            source,
        });
    }

    pub(super) fn copy(
        &mut self,
        expression: &'tu Expression<'tu>,
        destination: Subobject,
        enclosing_unions: &'tu [TypeId],
    ) {
        let value = self.value(expression);
        let destination = self.at(destination);
        _ = self.push(InitOperation::Copy {
            value,
            destination,
            enclosing_unions,
        });
    }

    pub(super) fn string(
        &mut self,
        literal: &'tu Expression<'tu>,
        destination: Subobject,
        completion: Completion,
    ) {
        if completion == Completion::Skip {
            return;
        }
        let destination = self.at(destination);
        _ = self.push_completed(
            InitOperation::String {
                literal,
                destination,
            },
            completion,
        );
    }

    pub(super) fn empty_scalar(&mut self, destination: Subobject, source: SourceVectors) {
        let destination = self.at(destination);
        _ = self.push(InitOperation::EmptyScalar {
            destination,
            source,
        });
    }

    pub(super) fn unmodeled(&mut self, destination: Subobject, source: SourceVectors) {
        let destination = self.at(destination);
        _ = self.push(InitOperation::Unmodeled {
            destination,
            source,
        });
    }

    /// A range designator selects elements `lower..=upper` of an array whose
    /// elements are `stride` bytes; the checker's cursor is at `upper`. The
    /// element's operations follow until [`Self::close_repeats`].
    pub(super) fn open_repeat(&mut self, lower: u64, upper: u64, stride: u64) {
        let Some(delta) = (upper - lower).checked_mul(stride) else {
            self.valid = false;
            return;
        };
        let count = upper - lower + 1;
        let index = self.push(InitOperation::Repeat {
            count,
            stride,
            length: 0,
        });
        self.shift += delta;
        self.repeating += u32::from(count > 1);
        self.repeats.push((index, delta, count > 1));
    }

    /// Ends the innermost `count` repeats: the element that opened them has
    /// been checked.
    pub(super) fn close_repeats(&mut self, count: u32) {
        for _ in 0..count {
            let Some((index, delta, repeated)) = self.repeats.pop() else {
                return;
            };
            let length = self.length() - index - 1;
            if let InitOperation::Repeat { length: run, .. } = &mut self.operations[index as usize]
            {
                *run = length;
            }
            self.shift -= delta;
            self.repeating -= u32::from(repeated);
        }
    }

    /// Analysis cannot describe this initializer's placement; no plan is
    /// retained, and the object is not lowered.
    pub(super) fn invalidate(&mut self) {
        self.valid = false;
    }

    /// Completes the operations' types and copies the plan into `tu`: an
    /// array of unknown size gets `root`, the object's completed type; the
    /// flexible array member gets `flexible`, room for every element any of
    /// its initializers writes, qualified as the object.
    pub(super) fn finish(
        &mut self,
        tu: &'tu Bump,
        root: TypeId,
        flexible: Option<TypeId>,
    ) -> Option<InitializerPlan<'tu>> {
        if !self.valid {
            return None;
        }
        let operations = &mut self.operations;
        let completions = [
            (&self.root_patches, Some(root)),
            (
                &self.flexible_patches,
                flexible.map(|array| array.qualified(root.qualifiers)),
            ),
        ];
        for (patches, ty) in completions {
            for &index in patches {
                let ty = ty?;
                if let InitOperation::Zero { destination, .. }
                | InitOperation::String { destination, .. } = &mut operations[index as usize]
                {
                    destination.ty = ty;
                }
            }
        }
        let mut retained = ArenaVec::with_capacity_in(operations.len(), tu);
        retained.extend(operations.iter().copied());
        let mut values = ArenaVec::with_capacity_in(self.values.len(), tu);
        values.extend(self.values.iter().copied());
        Some(InitializerPlan {
            ty:         root,
            operations: retained.leak(),
            values:     values.leak(),
        })
    }

    fn value(&mut self, expression: &'tu Expression<'tu>) -> InitValueId {
        let id = InitValueId(u32::try_from(self.values.len()).unwrap_or_else(|_| {
            self.valid = false;
            0
        }));
        self.values.push(InitValue {
            expression,
            repeated: self.repeating > 0,
        });
        id
    }

    /// `destination` at the first element of every open range.
    fn at(&mut self, destination: Subobject) -> Subobject {
        match destination.offset.checked_sub(self.shift) {
            | Some(offset) => Subobject {
                offset,
                ..destination
            },
            | None => {
                self.valid = false;
                destination
            },
        }
    }

    fn push_completed(&mut self, operation: InitOperation<'tu>, completion: Completion) -> u32 {
        let index = self.push(operation);
        match completion {
            | Completion::Root => self.root_patches.push(index),
            | Completion::Flexible => self.flexible_patches.push(index),
            | Completion::Complete | Completion::Skip => {},
        }
        index
    }

    fn push(&mut self, operation: InitOperation<'tu>) -> u32 {
        let index = self.length();
        if index == u32::MAX {
            self.valid = false;
        }
        self.operations.push(operation);
        index
    }

    fn length(&self) -> u32 {
        u32::try_from(self.operations.len()).unwrap_or(u32::MAX)
    }
}

/// Where the checker's current subobject or list object is in the plan.
/// C99: §6.7.3 paragraph 8, p. 109; PDF p. 121 (a subobject has its
/// containers' qualifiers).
#[derive(Clone, Copy)]
pub(super) struct Site<'s> {
    /// Absolute byte offset, at the checker's position: the last element of
    /// an open range.
    pub(super) offset:        u64,
    /// The qualifiers of the subobject's containers.
    pub(super) qualifiers:    TypeQualifiers,
    pub(super) bits:          Option<BitRange>,
    /// The unions containing the subobject.
    pub(super) unions:        Option<&'s UnionLink<'s>>,
    /// The element that initializes the subobject.
    pub(super) source:        SourceVectors,
    /// Inside a braced scalar, the outer brace pair's element.
    pub(super) scalar_source: Option<SourceVectors>,
    /// The subobject is the flexible array member the object's own
    /// initializer fills.
    pub(super) own_flexible:  bool,
}

impl Site<'_> {
    pub(super) fn root(source: SourceVectors) -> Self {
        Self {
            offset: 0,
            qualifiers: TypeQualifiers::empty(),
            bits: None,
            unions: None,
            source,
            scalar_source: None,
            own_flexible: false,
        }
    }

    pub(super) fn destination(&self, ty: TypeId) -> Subobject {
        Subobject {
            offset: self.offset,
            ty:     ty.qualified(self.qualifiers),
        }
    }
}

/// A container of the current subobject that is a union, innermost first.
#[derive(Clone, Copy)]
pub(super) struct UnionLink<'s> {
    pub(super) ty:   TypeId,
    pub(super) next: Option<&'s UnionLink<'s>>,
}

/// A container's position in the plan: its absolute offset, its qualifiers
/// with its containers', and the unions among it and its containers.
#[derive(Clone, Copy)]
pub(super) struct Container<'s> {
    pub(super) offset:     u64,
    pub(super) qualifiers: TypeQualifiers,
    pub(super) unions:     Option<&'s UnionLink<'s>>,
}

impl<'tu, 's> Analyzer<'_, 'tu, 's> {
    /// The plan position of an aggregate of type `ty` at `site`, as the
    /// container of its own subobjects.
    pub(super) fn plan_container(&self, site: &Site<'s>, ty: TypeId) -> Container<'s> {
        let qualifiers = ty.qualifiers | site.qualifiers;
        Container {
            offset: site.offset,
            qualifiers,
            unions: self.union_link(ty.qualified(site.qualifiers), site.unions),
        }
    }

    /// The plan position of subobject `index` of `container`, whose own type
    /// is `child`, as a container in turn; `None` if its offset overflows.
    pub(super) fn plan_child(
        &self,
        container: TypeId,
        at: &Container<'s>,
        index: u64,
        child: TypeId,
    ) -> Option<Container<'s>> {
        let (offset, _) = self.subobject_position(container, index)?;
        Some(Container {
            offset:     at.offset.checked_add(offset)?,
            qualifiers: child.qualifiers | at.qualifiers,
            unions:     self.union_link(child.qualified(at.qualifiers), at.unions),
        })
    }

    /// The site of subobject `index` of `container` at `at`.
    pub(super) fn plan_site(
        &self,
        container: TypeId,
        at: &Container<'s>,
        index: u64,
        source: SourceVectors,
    ) -> Option<Site<'s>> {
        let (offset, bits) = self.subobject_position(container, index)?;
        Some(Site {
            offset: at.offset.checked_add(offset)?,
            qualifiers: at.qualifiers,
            bits,
            unions: at.unions,
            source,
            scalar_source: None,
            own_flexible: false,
        })
    }

    /// The union types containing a subobject, outermost last, for a copy
    /// into it to keep their selections.
    pub(super) fn enclosing_unions(&self, site: &Site<'s>) -> &'tu [TypeId] {
        let mut unions = ArenaVec::new_in(self.types.tu);
        let mut link = site.unions;
        while let Some(union) = link {
            unions.push(union.ty);
            link = union.next;
        }
        unions.leak()
    }

    /// Whether `ty` is a union type.
    pub(super) fn union_type(&self, ty: TypeId) -> bool {
        matches!(
            self.types.nodes[ty.index],
            TypeKind::Tag(id) if self.types.tags[id].kind == TagKind::Union
        )
    }

    /// The byte offset of subobject `index` in `container` and, for a
    /// bit-field member, its bits: an element's index times the element
    /// size, a member's offset, or zero for a braced scalar.
    /// C99: §6.7.2.1 paragraphs 10-13, pp. 102-103; PDF pp. 114-115.
    fn subobject_position(&self, container: TypeId, index: u64) -> Option<(u64, Option<BitRange>)> {
        match self.types.nodes[container.index] {
            | TypeKind::Array(element, _) | TypeKind::Vector { element, .. } =>
                Some((index.checked_mul(self.types.layout(element)?.size)?, None)),
            | TypeKind::Tag(id) if self.types.tags[id].kind != TagKind::Enum => {
                let member = self.types.tags[id]
                    .members
                    .get()
                    .get(usize::try_from(index).ok()?)?;
                Some((
                    member.offset,
                    member.width.map(|width| BitRange {
                        bit_offset: member.bit_offset,
                        width,
                    }),
                ))
            },
            | _ => Some((0, None)),
        }
    }

    fn union_link(&self, ty: TypeId, next: Option<&'s UnionLink<'s>>) -> Option<&'s UnionLink<'s>> {
        if self.union_type(ty) {
            Some(self.scratch.alloc(UnionLink { ty, next }))
        } else {
            next
        }
    }
}
