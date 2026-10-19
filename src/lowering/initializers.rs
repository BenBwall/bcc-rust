//! Flattening an initializer into the scalars it stores.
//!
//! Semantic analysis checked every initializer but kept no placement plan,
//! so lowering walks the braces again with the current-object rules of
//! §6.7.8 paragraphs 17-20: each element initializes the next subobject of
//! the innermost open brace pair, and an element that is not itself braced
//! descends into an aggregate subobject until it reaches a scalar, a
//! character array it is a string literal for, or a structure of its own
//! type (brace elision). An explicit stack of frames replaces recursion;
//! a frame either owns a braced list or was opened by elision or a
//! designator and takes its elements from the nearest list below it. A
//! designation moves the frames to the subobject it names. GNU ranges use
//! inclusive cursors; repeated expression leaves share one evaluated value.
//!
//! C99: §6.7.8 paragraphs 12-22, pp. 126-127; PDF pp. 138-139.

mod coverage;

use coverage::CoverageIndex;

use super::{
    Construct,
    FunctionLowerer,
    LoweringError,
    module_items::{
        at,
        peel,
    },
    types::{
        self,
        Repr,
        repr,
    },
    work::{
        BitFieldBits,
        Item,
        Local,
        Object,
        Operand,
        Place,
        Task,
    },
};
use crate::{
    ir::{
        Align,
        InstData,
        InstFlags,
        Opcode,
        Type,
    },
    translation_phases::{
        SourceVectors,
        parsing::{
            declaration_syntax::{
                Designation,
                DesignatorType,
                Initializer,
                InitializerElement,
                InitializerType,
            },
            syntax::Expression,
        },
        semantic_analysis::{
            ArrayBound,
            Integer,
            SemanticTranslationUnit,
            TagKind,
            TypeId,
            TypeKind,
        },
    },
    util::bump::{
        ArenaMap,
        ArenaVec,
        Bump,
    },
};

/// One thing an initializer stores, at a byte offset from the object's
/// start.
#[derive(Clone, Copy, Debug)]
pub(super) enum Leaf<'tu> {
    /// Implicit zero initialization of a complete braced subobject.
    Zero {
        offset: u64,
        size:   u64,
        ty:     TypeId,
        /// Exclusive end of a braced scope; `usize::MAX` for a union switch,
        /// whose selection ends at the next switch or enclosing brace.
        end:    usize,
        /// Zero leaf of the enclosing brace pair (self for a braced scope).
        scope:  usize,
    },
    /// A scalar subobject and the expression whose converted value it
    /// gets.
    Scalar {
        offset:     u64,
        ty:         TypeId,
        expression: &'tu Expression<'tu>,
        /// For a bit-field, where its bits are from `offset`.
        bit_field:  Option<BitFieldBits>,
    },
    /// A character array of `size` bytes initialized by a string literal
    /// (§6.7.8p14-15).
    String {
        ty:      TypeId,
        offset:  u64,
        size:    u64,
        literal: &'tu Expression<'tu>,
    },
    /// A structure or union initialized by an expression of its type
    /// (§6.7.8p13).
    Copy {
        offset:     u64,
        ty:         TypeId,
        expression: &'tu Expression<'tu>,
    },
}

/// The stores of `initializer` for an object of type `ty`, in source order.
/// `flexible` is the array type semantic analysis gave the flexible array
/// member of a static structure whose initializer fills it (GNU), with room
/// for every element the initializer writes, including those a later
/// initializer of the member replaces; the caller cuts the replaced tail
/// off the object.
/// C99: §6.7.8 paragraphs 14-19, pp. 126-127; PDF pp. 138-139.
pub(super) fn leaves<'tu, 's>(
    sema: &SemanticTranslationUnit<'tu>,
    ty: TypeId,
    initializer: &'tu Initializer<'tu>,
    flexible: Option<TypeId>,
    arena: &'s Bump,
) -> Result<ArenaVec<'s, Leaf<'tu>>, LoweringError> {
    let mut walker = Walker {
        flexible: flexible.map(|flexible| (ty, flexible)),
        sema,
        selected_unions: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena),
        ranges: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena),
        arena,
    };
    let mut leaves = ArenaVec::new_in(arena);
    if let Some(leaf) = walker.braced_string(ty, 0, initializer)? {
        leaves.push(leaf);
        return Ok(leaves);
    }
    let list = match initializer.kind {
        | InitializerType::AssignmentExpression(expression) => {
            let leaf = walker.leaf(ty, 0, expression)?.ok_or_else(|| {
                LoweringError::unsupported(Construct::Initializer, expression.source_vectors)
            })?;
            leaves.push(leaf);
            return Ok(leaves);
        },
        | InitializerType::InitializerList(list) => list,
    };
    if walker.scalar(ty, initializer.source_vectors)? {
        let expression = braced_scalar(initializer)?;
        leaves.push(Leaf::Scalar {
            offset: 0,
            ty,
            expression,
            bit_field: None,
        });
        return Ok(leaves);
    }
    let mut frames = ArenaVec::new_in(arena);
    let size = types::layout(&sema.types, ty)
        .map_err(|kind| at(kind, initializer.source_vectors))?
        .size;
    leaves.push(Leaf::Zero {
        offset: 0,
        size,
        ty,
        end: usize::MAX,
        scope: 0,
    });
    frames.push(Frame {
        ty,
        offset: 0,
        next: 0,
        list: Some((list.elements.as_slice(), 0)),
        zero: 0,
    });
    // The element whose designation has moved the frames, so that brace
    // elision retrying the element does not move them again.
    let mut designated = None;
    while let Some(top) = frames.len().checked_sub(1) {
        let owner = frames
            .iter()
            .rposition(|frame| frame.list.is_some())
            .expect("the outermost frame owns a list");
        let (elements, position) = frames[owner].list.expect("the owner has a list");
        let Some(element) = elements.get(position) else {
            let length = leaves.len();
            if let Leaf::Zero { end, .. } = &mut leaves[frames[owner].zero] {
                *end = length;
            }
            frames.truncate(owner);
            continue;
        };
        if let Some(designation) = element.designation
            && designated != Some((owner, position))
        {
            walker.designate(&mut frames, owner, position, designation, &mut leaves)?;
            designated = Some((owner, position));
            continue;
        }
        let frame = frames[top];
        let Some(subobject) = walker.subobject(frame.ty, frame.next, element.source_vectors)?
        else {
            if top == owner {
                // Semantic analysis rejects excess initializers (§6.7.8p2).
                return Err(LoweringError::missing(
                    "an initializer has more elements than its object",
                    element.source_vectors,
                ));
            }
            _ = frames.pop();
            continue;
        };
        if walker.incomplete(subobject.ty) {
            // GNU: an empty list leaves a flexible array member without
            // elements and stores nothing; semantic analysis completes the
            // member's type for every other initializer it accepts.
            if !matches!(
                element.initializer.kind,
                InitializerType::InitializerList(inner) if inner.elements.as_slice().is_empty()
            ) {
                return Err(LoweringError::missing(
                    "a flexible array member is initialized without its extent",
                    element.source_vectors,
                ));
            }
            walker.consume(&mut frames, top, owner, subobject.next);
            designated = None;
            continue;
        }
        walker.select_union(frame, element.source_vectors, &mut leaves)?;
        let offset = frame.offset + subobject.offset;
        match element.initializer.kind {
            | InitializerType::InitializerList(inner) => {
                walker.consume(&mut frames, top, owner, subobject.next);
                designated = None;
                if let Some(leaf) =
                    walker.braced_string(subobject.ty, offset, element.initializer)?
                {
                    leaves.push(leaf);
                } else if walker.scalar(subobject.ty, element.source_vectors)? {
                    let expression = braced_scalar(element.initializer)?;
                    leaves.push(Leaf::Scalar {
                        offset,
                        ty: subobject.ty,
                        expression,
                        bit_field: subobject.bit_field,
                    });
                } else {
                    // C99 §6.7.8p19: a later complete initializer replaces
                    // implicit members as well as explicitly written ones.
                    let size = types::layout(&sema.types, subobject.ty)
                        .map_err(|kind| at(kind, element.source_vectors))?
                        .size;
                    leaves.push(Leaf::Zero {
                        offset,
                        size,
                        ty: subobject.ty,
                        end: usize::MAX,
                        scope: leaves.len(),
                    });
                    frames.push(Frame {
                        ty: subobject.ty,
                        offset,
                        next: 0,
                        list: Some((inner.elements.as_slice(), 0)),
                        zero: leaves.len() - 1,
                    });
                }
            },
            | InitializerType::AssignmentExpression(expression) =>
                match walker.leaf(subobject.ty, offset, expression)? {
                    | Some(mut leaf) => {
                        if let Leaf::Copy { ty, .. } = leaf {
                            let size = types::layout(&sema.types, ty)
                                .map_err(|kind| at(kind, element.source_vectors))?
                                .size;
                            walker.forget_subobjects(&frames, offset, size);
                        }
                        if let Leaf::Scalar { bit_field, .. } = &mut leaf {
                            *bit_field = subobject.bit_field;
                        }
                        walker.consume(&mut frames, top, owner, subobject.next);
                        designated = None;
                        leaves.push(leaf);
                    },
                    | None => {
                        // The element initializes the subobject's first
                        // member; the subobject is this aggregate's current
                        // one either way.
                        frames[top].next = subobject.next;
                        frames.push(Frame {
                            ty: subobject.ty,
                            offset,
                            next: 0,
                            list: None,
                            zero: frame.zero,
                        });
                    },
                },
        }
    }
    Ok(leaves)
}

/// The expression inside a braced scalar initializer such as `{ 1 }`
/// (§6.7.8p11).
fn braced_scalar<'tu>(
    mut initializer: &'tu Initializer<'tu>,
) -> Result<&'tu Expression<'tu>, LoweringError> {
    loop {
        match initializer.kind {
            | InitializerType::AssignmentExpression(expression) => return Ok(expression),
            | InitializerType::InitializerList(list) => {
                let element = list.elements.as_slice().first().ok_or_else(|| {
                    LoweringError::unsupported(Construct::Initializer, initializer.source_vectors)
                })?;
                if element.designation.is_some() {
                    return Err(LoweringError::unsupported(
                        Construct::Designator,
                        element.source_vectors,
                    ));
                }
                initializer = element.initializer;
            },
        }
    }
}

/// An aggregate being initialized and the position of its next subobject.
#[derive(Clone, Copy)]
struct Frame<'tu> {
    ty:     TypeId,
    offset: u64,
    /// The next element index, or the next member index.
    next:   usize,
    /// The braced list this frame takes elements from, and the position of
    /// its next element; `None` for a frame opened by brace elision.
    list:   Option<(&'tu [InitializerElement<'tu>], usize)>,
    /// Zero leaf of the innermost enclosing brace pair, also for elision
    /// and designator frames, so member resets never search for their scope.
    zero:   usize,
}

/// The subobject an element initializes.
#[derive(Clone, Copy)]
struct Subobject {
    ty:        TypeId,
    /// Its offset from the start of the aggregate.
    offset:    u64,
    /// Where a bit-field member's bits are from `offset`.
    bit_field: Option<BitFieldBits>,
    /// The aggregate's next position after it.
    next:      usize,
}

struct Walker<'a, 'tu, 's> {
    /// The object's structure and the written type of its flexible array
    /// member, which only that structure's own frame reaches.
    flexible:        Option<(TypeId, TypeId)>,
    arena:           &'s Bump,
    sema:            &'a SemanticTranslationUnit<'tu>,
    /// Selection survives frames closed and reopened by designators. The
    /// type distinguishes nested unions starting at the same byte offset.
    selected_unions: ArenaMap<'s, (u64, usize), usize>,
    /// Inclusive range cursors for the currently consumed source element.
    ranges:          ArenaMap<'s, (usize, usize), &'s mut [RangeCursor]>,
}

/// Inclusive GNU range bounds and the current element in their product.
type RangeCursor = (u64, u64, u64);

impl<'tu> Walker<'_, 'tu, '_> {
    /// GNU Designated Initializers: advance a Cartesian range in source order,
    /// then resume at the subobject following its inclusive upper endpoint.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Designated-Inits.html>
    fn consume(
        &mut self,
        frames: &mut ArenaVec<'_, Frame<'tu>>,
        top: usize,
        owner: usize,
        next: usize,
    ) {
        frames[top].next = next;
        let position = frames[owner]
            .list
            .expect("a brace frame owns the element")
            .1;
        if let Some(cursors) = self.ranges.get_mut(&(owner, position)) {
            for (lower, current, upper) in cursors.iter_mut().rev() {
                if *current < *upper {
                    *current += 1;
                    return;
                }
                *current = *lower;
            }
            _ = self.ranges.remove(&(owner, position));
        }
        frames[owner]
            .list
            .as_mut()
            .expect("a brace frame owns the element")
            .1 += 1;
    }

    /// Uses semantic analysis's whole-array classification before descending
    /// into an explicitly braced array's first character.
    /// C99: §6.7.8p14-15, p. 126; PDF p. 138.
    fn braced_string(
        &self,
        ty: TypeId,
        offset: u64,
        initializer: &'tu Initializer<'tu>,
    ) -> Result<Option<Leaf<'tu>>, LoweringError> {
        if let InitializerType::InitializerList(list) = initializer.kind
            && let [element] = list.elements.as_slice()
            && element.designation.is_none()
            && let InitializerType::AssignmentExpression(expression) = element.initializer.kind
            && self.sema.string_initializes(expression, ty)
        {
            return self.leaf(ty, offset, expression);
        }
        Ok(None)
    }

    /// What a non-braced initializer `expression` stores into a subobject
    /// of type `ty`, or `None` if it initializes the subobject's first
    /// member instead.
    fn leaf(
        &self,
        ty: TypeId,
        offset: u64,
        expression: &'tu Expression<'tu>,
    ) -> Result<Option<Leaf<'tu>>, LoweringError> {
        let source = expression.source_vectors;
        if self.scalar(ty, source)? {
            return Ok(Some(Leaf::Scalar {
                offset,
                ty,
                expression,
                bit_field: None,
            }));
        }
        let types = &self.sema.types;
        // C99 §6.7.8p14-15: use the retained narrow/wide string classification.
        if self.sema.string_initializes(expression, ty) {
            let size = types
                .layout(ty)
                .ok_or_else(|| LoweringError::missing("an array has no size", source))?
                .size;
            return Ok(Some(Leaf::String {
                ty,
                offset,
                size,
                literal: expression,
            }));
        }
        let converted = self
            .sema
            .expression_conversions(expression)
            .last()
            .map(|conversion| conversion.ty)
            .or_else(|| self.sema.expression_info(expression).map(|info| info.ty));
        let value_ty = types.non_atomic(ty);
        if matches!(types.kind(value_ty), TypeKind::Tag(_))
            && converted.is_some_and(|from| types.non_atomic(from).index == value_ty.index)
        {
            return Ok(Some(Leaf::Copy {
                offset,
                ty,
                expression,
            }));
        }
        Ok(None)
    }

    /// The subobject at position `next` of an aggregate: an array element,
    /// the next named or anonymous member of a structure, or the first of
    /// a union (§6.7.8p17); `None` when the aggregate is full.
    /// C99: §6.5.2.3 paragraph 3, p. 73; PDF p. 85; §6.7.3 paragraphs
    /// 6 and 8, p. 109; PDF p. 121 (subobjects retain destination qualifiers).
    fn subobject(
        &self,
        ty: TypeId,
        next: usize,
        source: SourceVectors,
    ) -> Result<Option<Subobject>, LoweringError> {
        let types = &self.sema.types;
        match types.kind(ty) {
            | TypeKind::Array(element, ArrayBound::Constant(count)) => {
                if next as u64 >= count {
                    return Ok(None);
                }
                let size = types
                    .layout(element)
                    .ok_or_else(|| LoweringError::missing("an array element has no size", source))?
                    .size;
                Ok(Some(Subobject {
                    ty:        element.qualified(ty.qualifiers),
                    offset:    next as u64 * size,
                    bit_field: None,
                    next:      next + 1,
                }))
            },
            | TypeKind::Tag(_) => {
                let tag = types.tag(ty).expect("a tag type has a tag");
                let members = tag.members.get();
                let Some((index, member)) = members
                    .iter()
                    .enumerate()
                    .skip(next)
                    .find(|(_, member)| member.initializable())
                else {
                    return Ok(None);
                };
                // The flexible array member of the object's own structure has
                // the extent semantic analysis recorded; a structure cannot
                // contain itself, so its type alone identifies that frame.
                // Any other flexible array member keeps its incomplete type,
                // which only an empty list initializes.
                let member_ty = match self.flexible {
                    | Some((root, flexible))
                        if root.index == ty.index
                            && matches!(
                                types.kind(member.ty),
                                TypeKind::Array(_, ArrayBound::Incomplete)
                            ) =>
                        flexible,
                    | _ => member.ty,
                };
                Ok(Some(Subobject {
                    ty:        member_ty.qualified(ty.qualifiers),
                    offset:    member.offset,
                    bit_field: member.width.map(|width| BitFieldBits {
                        bit_offset: member.bit_offset,
                        width,
                    }),
                    next:      if tag.kind == TagKind::Union {
                        members.len()
                    } else {
                        index + 1
                    },
                }))
            },
            | _ => Ok(None),
        }
    }

    /// Moves the frames to the subobject a designation names: the frames
    /// above the brace pair that owns the element close, each designator
    /// but the last opens a frame for the subobject it names, and the last
    /// makes its subobject the next one (§6.7.8p17-18). A member reached
    /// through anonymous members opens a frame for each.
    fn designate(
        &mut self,
        frames: &mut ArenaVec<'_, Frame<'tu>>,
        owner: usize,
        position: usize,
        designation: &'tu Designation<'tu>,
        leaves: &mut ArenaVec<'_, Leaf<'tu>>,
    ) -> Result<(), LoweringError> {
        frames.truncate(owner + 1);
        let designators = designation.designators.as_slice();
        if !self.ranges.contains_key(&(owner, position)) {
            let mut cursors = ArenaVec::new_in(self.arena);
            for designator in designators {
                if let DesignatorType::Range(range) = designator.kind {
                    let value = |e| {
                        self.sema
                            .expression_info(e)
                            .and_then(|info| info.integer)
                            .and_then(Integer::to_u64)
                    };
                    let lower = value(range.lower.expression()).ok_or_else(|| {
                        LoweringError::unsupported(Construct::Designator, designator.source_vectors)
                    })?;
                    let upper = value(range.upper.expression()).ok_or_else(|| {
                        LoweringError::unsupported(Construct::Designator, designator.source_vectors)
                    })?;
                    cursors.push((lower, lower, upper));
                }
            }
            if !cursors.is_empty() {
                _ = self.ranges.insert((owner, position), cursors.leak());
            }
        }
        let mut range_index = 0;
        for (index, designator) in designators.iter().enumerate() {
            let source = designator.source_vectors;
            let unsupported = || LoweringError::unsupported(Construct::Designator, source);
            let top = frames.len() - 1;
            let ty = frames[top].ty;
            match designator.kind {
                | DesignatorType::Array(constant) => {
                    let position = self
                        .sema
                        .expression_info(constant.expression())
                        .and_then(|info| info.integer)
                        .and_then(Integer::to_u64)
                        .and_then(|value| usize::try_from(value).ok())
                        .ok_or_else(unsupported)?;
                    frames[top].next = position;
                },
                | DesignatorType::Field(name) | DesignatorType::GnuField(name) => {
                    let tag = self.sema.types.tag(ty).ok_or_else(unsupported)?;
                    let field = tag
                        .fields
                        .get()
                        .iter()
                        .find(|field| field.name.name == name.name)
                        .ok_or_else(unsupported)?;
                    let steps = field.path.len();
                    for (step, member) in field.path.iter().enumerate() {
                        let top = frames.len() - 1;
                        frames[top].next = member;
                        if step + 1 < steps {
                            self.open_next(frames, source, leaves)?;
                        }
                    }
                },
                | DesignatorType::Range(_) => {
                    let current = self
                        .ranges
                        .get(&(owner, position))
                        .and_then(|ranges| ranges.get(range_index))
                        .ok_or_else(unsupported)?
                        .1;
                    frames[top].next = usize::try_from(current).map_err(|_| unsupported())?;
                    range_index += 1;
                },
                | DesignatorType::Error => return Err(unsupported()),
            }
            if index + 1 < designators.len() {
                self.open_next(frames, source, leaves)?;
            }
        }
        Ok(())
    }

    /// Opens a frame for the top frame's next subobject, which the
    /// following elements initialize.
    fn open_next(
        &mut self,
        frames: &mut ArenaVec<'_, Frame<'tu>>,
        source: SourceVectors,
        leaves: &mut ArenaVec<'_, Leaf<'tu>>,
    ) -> Result<(), LoweringError> {
        let top = frames.len() - 1;
        let frame = frames[top];
        let subobject = self
            .subobject(frame.ty, frame.next, source)?
            .ok_or_else(|| LoweringError::unsupported(Construct::Designator, source))?;
        if self.incomplete(subobject.ty) {
            return Err(LoweringError::missing(
                "a flexible array member is initialized without its extent",
                source,
            ));
        }
        self.select_union(frame, source, leaves)?;
        frames[top].next = subobject.next;
        frames.push(Frame {
            ty:     subobject.ty,
            offset: frame.offset + subobject.offset,
            next:   0,
            list:   None,
            zero:   frame.zero,
        });
        Ok(())
    }

    /// Whether a subobject is a flexible array member semantic analysis did
    /// not complete. C99: §6.7.2.1 paragraph 16, p. 103; PDF p. 115.
    fn incomplete(&self, ty: TypeId) -> bool {
        matches!(
            self.sema.types.kind(ty),
            TypeKind::Array(_, ArrayBound::Incomplete)
        )
    }

    /// A newly selected union member replaces the previous member, including
    /// its implicitly initialized subobjects. Designators within the same
    /// member keep accumulating writes.
    /// C99: §6.7.8 paragraphs 17-19, pp. 126-127; PDF pp. 138-139.
    fn select_union(
        &mut self,
        frame: Frame<'tu>,
        source: SourceVectors,
        leaves: &mut ArenaVec<'_, Leaf<'tu>>,
    ) -> Result<(), LoweringError> {
        let Some(tag) = self.sema.types.tag(frame.ty) else {
            return Ok(());
        };
        if tag.kind != TagKind::Union {
            return Ok(());
        }
        let member = tag
            .members
            .get()
            .iter()
            .enumerate()
            .skip(frame.next)
            .find(|(_, member)| member.initializable())
            .map(|(index, _)| index)
            .ok_or_else(|| LoweringError::unsupported(Construct::Designator, source))?;
        let previous = self
            .selected_unions
            .insert((frame.offset, frame.ty.index), member);
        if previous.is_some_and(|previous| previous != member) {
            let size = types::layout(&self.sema.types, frame.ty)
                .map_err(|kind| at(kind, source))?
                .size;
            leaves.push(Leaf::Zero {
                offset: frame.offset,
                size,
                ty: frame.ty,
                end: usize::MAX,
                scope: frame.zero,
            });
        }
        Ok(())
    }

    /// A whole-subobject expression copy has no known selected union member
    /// below it. The containing frames keep their selected members.
    /// C99: §6.7.8 paragraph 19, p. 127; PDF p. 139.
    fn forget_subobjects(&mut self, frames: &[Frame<'tu>], offset: u64, size: u64) {
        self.selected_unions.retain(|&(at, ty), _| {
            at < offset
                || at >= offset + size
                || frames
                    .iter()
                    .any(|frame| frame.offset == at && frame.ty.index == ty)
        });
    }

    fn scalar(&self, ty: TypeId, source: SourceVectors) -> Result<bool, LoweringError> {
        Ok(
            match repr(&self.sema.types, ty).map_err(|kind| at(kind, source))? {
                | Repr::Aggregate => false,
                | representation => representation.is_scalar(),
            },
        )
    }
}

// Storing into automatic objects

impl<'tu, 's> FunctionLowerer<'_, '_, 'tu, '_, 's> {
    /// Schedules the leaves of a braced or aggregate initializer: each
    /// scalar is evaluated before its bytes are stored. Only omitted ranges
    /// are zeroed, including resets of later complete subobject initializers.
    /// C99: §6.2.4p5, p. 32; PDF p. 44; §6.7.8 paragraphs 19 and 21,
    /// p. 127; PDF p. 139.
    pub(super) fn initialize_aggregate(
        &mut self,
        object: Object,
        ty: TypeId,
        initializer: &'tu Initializer<'tu>,
    ) -> Result<(), LoweringError> {
        let leaves = leaves(self.unit.sema, ty, initializer, None, self.scratch)?;
        let coverage = CoverageIndex::new(
            &leaves,
            self.unit.sema,
            initializer.source_vectors,
            self.scratch,
        )?;
        let mut candidates = ArenaVec::new_in(self.scratch);
        let mut covered = ArenaVec::new_in(self.scratch);
        let mut search = ArenaVec::new_in(self.scratch);
        let mut tasks = ArenaVec::with_capacity_in(2 * leaves.len(), self.scratch);
        let mut values = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, self.scratch);
        let mut counts = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, self.scratch);
        for leaf in &leaves {
            if let Leaf::Scalar { expression, .. } | Leaf::Copy { expression, .. } = leaf {
                *counts
                    .entry(std::ptr::from_ref(*expression).addr())
                    .or_insert(0_usize) += 1;
            }
        }
        for (index, &leaf) in leaves.iter().enumerate() {
            match leaf {
                | Leaf::Zero {
                    offset,
                    size,
                    ty,
                    end,
                    scope,
                } => {
                    let union_reset = end == usize::MAX;
                    let end = if union_reset {
                        #[cfg(debug_assertions)]
                        debug_assert!(
                            matches!(leaves[scope], Leaf::Zero { .. }),
                            "a brace scope starts with a zero leaf"
                        );
                        if let Leaf::Zero { end, .. } = leaves[scope] {
                            end
                        } else {
                            leaves.len()
                        }
                    } else {
                        end
                    };
                    // C99 §6.7.8p19: only this subobject's following
                    // initializers can preserve its explicit bytes. Search
                    // only after this reset and before the next selection of
                    // the same union or the enclosing brace's end. Nested
                    // braces and enclosing union resets still delimit below.
                    let end = if union_reset {
                        end.min(coverage.next_reset(index))
                    } else {
                        end
                    };
                    coverage.overlapping(
                        8 * offset,
                        8 * (offset + size),
                        index + 1..end,
                        &mut candidates,
                        &mut search,
                    );
                    covered.clear();
                    let mut skip_until = index + 1;
                    for &candidate in &candidates {
                        let next = candidate.index;
                        if next < skip_until {
                            continue;
                        }
                        if next >= end {
                            break;
                        }
                        if let Leaf::Zero {
                            offset: at,
                            size: length,
                            end: scope,
                            ..
                        } = leaves[next]
                        {
                            // A subsequent reset of this union ends this
                            // selection's coverage; nested braces own their
                            // own omissions and can be skipped as one range.
                            if union_reset
                                && scope == usize::MAX
                                && at <= offset
                                && at + length >= offset + size
                            {
                                break;
                            }
                            if scope != usize::MAX {
                                skip_until = scope;
                            }
                        }
                        let start = candidate.start.max(8 * offset);
                        let stop = candidate.stop.min(8 * (offset + size));
                        if start < stop {
                            covered.push((start, stop, candidate.ty));
                        }
                    }
                    covered.sort_unstable_by_key(|&(start, stop, _)| (start, stop));
                    let mut cursor = 8 * offset;
                    let mut cursor_ty = ty;
                    for &(start, stop, covered_ty) in &covered {
                        if cursor < start {
                            zero_gap(
                                &mut tasks,
                                object,
                                ty,
                                cursor,
                                start,
                                (cursor_ty, covered_ty),
                            );
                        }
                        if stop > cursor {
                            cursor = stop;
                            cursor_ty = covered_ty;
                        }
                    }
                    if cursor < 8 * (offset + size) {
                        zero_gap(
                            &mut tasks,
                            object,
                            ty,
                            cursor,
                            8 * (offset + size),
                            (cursor_ty, ty),
                        );
                    }
                },
                | Leaf::Scalar {
                    offset,
                    ty,
                    expression,
                    bit_field,
                } => {
                    self.initializer_expression(&mut tasks, &mut values, &counts, expression, ty);
                    tasks.push(Task::Initialize {
                        object,
                        offset,
                        ty,
                        bit_field,
                    });
                },
                | Leaf::Copy {
                    offset,
                    ty,
                    expression,
                } => {
                    self.initializer_expression(&mut tasks, &mut values, &counts, expression, ty);
                    tasks.push(Task::Initialize {
                        object,
                        offset,
                        ty,
                        bit_field: None,
                    });
                },
                | Leaf::String {
                    offset,
                    size,
                    literal,
                    ty,
                } => tasks.push(Task::InitializeString {
                    object,
                    ty,
                    offset,
                    size,
                    literal,
                }),
            }
        }
        // The first leaf runs first.
        self.tasks.extend(tasks.iter().rev().copied());
        Ok(())
    }

    /// A GNU range initializer evaluates each expression once, then reuses
    /// its converted value for every selected subobject, including aggregates.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Designated-Inits.html>
    fn initializer_expression(
        &mut self,
        tasks: &mut ArenaVec<'s, Task<'tu>>,
        values: &mut ArenaMap<'s, usize, usize>,
        counts: &ArenaMap<'s, usize, usize>,
        expression: &'tu Expression<'tu>,
        ty: TypeId,
    ) {
        let key = std::ptr::from_ref(expression).addr();
        if counts.get(&key).copied().unwrap_or(0) <= 1 {
            tasks.push(Task::Evaluate(expression));
        } else if let Some(&index) = values.get(&key) {
            tasks.push(Task::ReuseInitializer(index));
        } else {
            let index = self.initializer_values.len();
            self.initializer_values.push(Item {
                operand: Operand::Void,
                ty,
            });
            _ = values.insert(key, index);
            tasks.push(Task::Evaluate(expression));
            tasks.push(Task::SnapshotArgument(expression));
            tasks.push(Task::RememberInitializer(index));
        }
    }

    /// Replaces the implicit members of a braced subobject with zeroes.
    /// C99: §6.7.8 paragraph 19, p. 127; PDF p. 139.
    /// C99: §6.7.3 paragraph 6, p. 109; PDF p. 121.
    pub(super) fn initialize_zero(
        &mut self,
        object: Object,
        offset: u64,
        size: u64,
        ty: TypeId,
        mask: u8,
    ) -> Result<(), LoweringError> {
        let base = match object {
            | Object::Slot(slot) => self.inst(InstData::StackAddr { slot }, Type::Ptr),
            | Object::Local(binding) => match self.local(
                binding,
                self.unit.sema.bindings[binding].name.source_vectors,
            )? {
                | Local::Slot(slot) => self.inst(InstData::StackAddr { slot }, Type::Ptr),
                | Local::Address(address) => address,
                | Local::Variable(_) =>
                    return Err(LoweringError::missing(
                        "an aggregate has no address",
                        SourceVectors::empty(),
                    )),
            },
        };
        let address = self.offset_address(base, offset);
        if mask != u8::MAX {
            let flags = self.initialization_flags(ty);
            let old = self.inst(
                InstData::Load {
                    ty: Type::I8,
                    flags,
                    align: Align::BYTE,
                    tag: None,
                    addr: address,
                },
                Type::I8,
            );
            let old = self.convert(Opcode::Freeze, Type::I8, old);
            let keep = self.iconst(Type::I8, i128::from(!mask));
            let value = self.binary_inst(Opcode::And, InstFlags::empty(), old, keep);
            _ = self.draft.push(
                self.memory_op(InstData::Store {
                    ty: Type::I8,
                    flags,
                    align: Align::BYTE,
                    tag: None,
                    args: [value, address],
                }),
                None,
            );
            return Ok(());
        }
        let zero = self.iconst(Type::I8, 0);
        let size = self.iconst(Type::I64, i128::from(size));
        _ = self.draft.push(
            self.memory_op(InstData::MemoryRange {
                opcode: Opcode::Fill,
                flags:  self.initialization_flags(ty),
                align:  Align::BYTE,
                args:   [address, zero, size],
            }),
            None,
        );
        Ok(())
    }

    /// Stores the value on top of the stack into an object being
    /// initialized, at `offset` bytes from its start.
    /// C99: §6.7.3 paragraph 6, p. 109; PDF p. 121.
    pub(super) fn initialize(
        &mut self,
        object: Object,
        offset: u64,
        ty: TypeId,
        bit_field: Option<BitFieldBits>,
    ) -> Result<(), LoweringError> {
        let item = self.pop();
        let (base, source) = match object {
            | Object::Local(binding) => {
                let source = self.unit.sema.bindings[binding].name.source_vectors;
                match self.local(binding, source)? {
                    | Local::Variable(var) => {
                        let value = self.value(item, source)?;
                        self.draft.def_var(var, value);
                        return Ok(());
                    },
                    | Local::Slot(slot) =>
                        (self.inst(InstData::StackAddr { slot }, Type::Ptr), source),
                    | Local::Address(address) => (address, source),
                }
            },
            | Object::Slot(slot) => (
                self.inst(InstData::StackAddr { slot }, Type::Ptr),
                SourceVectors::empty(),
            ),
        };
        let place = if let Some(bits) = bit_field {
            let bit_field = self.bit_field(ty, offset, bits, source)?;
            Place::BitField(self.offset_address(base, offset), bit_field)
        } else {
            Place::Memory(self.offset_address(base, offset))
        };
        let place = Item {
            operand: Operand::Place(place),
            ty,
        };
        _ = self.write(place, item, source)?;
        Ok(())
    }

    /// Copies a string literal's bytes into a character array; a literal
    /// one longer than an exactly sized array loses its null character.
    /// C99: §6.7.8 paragraphs 14-15, p. 126; PDF p. 138.
    /// C99: §6.7.8 paragraphs 19 and 21, p. 127; PDF p. 139 (replacement
    /// includes the implicitly zero-initialized remainder of the array).
    /// C99: §6.7.3 paragraph 6, p. 109; PDF p. 121.
    pub(super) fn initialize_string(
        &mut self,
        object: Object,
        offset: u64,
        size: u64,
        literal: &'tu Expression<'tu>,
        ty: TypeId,
    ) -> Result<(), LoweringError> {
        let source = literal.source_vectors;
        let global = self.unit.string(peel(literal))?;
        let length = self.unit.module.global(global).size.min(size);
        let slot = match object {
            | Object::Slot(slot) => slot,
            | Object::Local(binding) => match self.local(binding, source)? {
                | Local::Slot(slot) => slot,
                | Local::Variable(_) | Local::Address(_) =>
                    return Err(LoweringError::missing(
                        "a character array is not in a stack slot",
                        source,
                    )),
            },
        };
        let base = self.inst(InstData::StackAddr { slot }, Type::Ptr);
        let destination = self.offset_address(base, offset);
        let string = self.inst(InstData::GlobalAddr { global }, Type::Ptr);
        let copied_length = self.iconst(Type::I64, i128::from(length));
        let flags = self.initialization_flags(ty);
        _ = self.draft.push(
            self.memory_op(InstData::MemoryRange {
                opcode: Opcode::Copy,
                flags,
                align: Align::BYTE,
                args: [destination, string, copied_length],
            }),
            None,
        );
        if length < size {
            let tail = self.offset_address(destination, length);
            let zero = self.iconst(Type::I8, 0);
            let remaining = self.iconst(Type::I64, i128::from(size - length));
            _ = self.draft.push(
                self.memory_op(InstData::MemoryRange {
                    opcode: Opcode::Fill,
                    flags,
                    align: Align::BYTE,
                    args: [tail, zero, remaining],
                }),
                None,
            );
        }
        Ok(())
    }

    /// Range writes retain qualifiers on the object or its array elements.
    /// C99: §6.7.3 paragraphs 6 and 8, p. 109; PDF p. 121.
    fn initialization_flags(&self, mut ty: TypeId) -> InstFlags {
        loop {
            if types::is_volatile(ty) {
                return InstFlags::VOLATILE;
            }
            match self.unit.sema.types.kind(ty) {
                | TypeKind::Array(element, _) => ty = element,
                | _ => return InstFlags::empty(),
            }
        }
    }

    /// A compound literal in a function: an unnamed automatic object,
    /// initialized where the literal is evaluated, whose place is the
    /// literal's value.
    /// C99: §6.5.2.5 paragraphs 4-6, pp. 75-76; PDF pp. 87-88.
    pub(super) fn compound_literal(
        &mut self,
        ty: TypeId,
        initializer: &'tu Initializer<'tu>,
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        let layout = types::layout(&self.unit.sema.types, ty).map_err(|kind| at(kind, source))?;
        let size = u32::try_from(layout.size).map_err(|_| {
            LoweringError::missing("a compound literal is larger than 4 GiB", source)
        })?;
        let slot = self.object_slot(
            size,
            super::alignment::storage_alignment(&self.unit.sema.types, ty)
                .map_err(|kind| at(kind, source))?,
        );
        self.tasks.push(Task::PushSlot { slot, ty });
        self.initialize_aggregate(Object::Slot(slot), ty, initializer)
    }
}

/// Emits a fill for the whole bytes of an omitted bit range, with at most
/// two masked writes for bytes shared with explicit bit-field initializers.
/// C99: §6.7.8 paragraphs 19 and 21, p. 127; PDF p. 139.
fn zero_gap(
    tasks: &mut ArenaVec<'_, Task<'_>>,
    object: Object,
    ty: TypeId,
    mut start: u64,
    end: u64,
    boundaries: (TypeId, TypeId),
) {
    let shared_ty = ty.qualified(boundaries.0.qualifiers | boundaries.1.qualifiers);
    if !start.is_multiple_of(8) {
        let stop = end.min(start.next_multiple_of(8));
        let mask = (u8::MAX >> (8 - (stop - start))) << (start % 8);
        tasks.push(Task::InitializeZero {
            object,
            ty: shared_ty,
            offset: start / 8,
            size: 1,
            mask,
        });
        start = stop;
    }
    let whole_end = end / 8 * 8;
    if start < whole_end {
        tasks.push(Task::InitializeZero {
            object,
            ty,
            offset: start / 8,
            size: (whole_end - start) / 8,
            mask: u8::MAX,
        });
        start = whole_end;
    }
    if start < end {
        tasks.push(Task::InitializeZero {
            object,
            ty: shared_ty,
            offset: start / 8,
            size: 1,
            mask: u8::MAX >> (8 - (end - start)),
        });
    }
}
