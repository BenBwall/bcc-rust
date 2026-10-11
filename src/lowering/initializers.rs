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
//! designation moves the frames to the subobject it names; GNU range
//! designators are not supported.
//!
//! C99: §6.7.8 paragraphs 12-22, pp. 126-127; PDF pp. 138-139.

use super::{
    Construct,
    FunctionLowerer,
    Local,
    LoweringError,
    Task,
    module_items::{
        at,
        peel,
    },
    ssa::Op,
    types::{
        self,
        Repr,
        repr,
    },
    work::{
        Item,
        Object,
        Operand,
        Place,
    },
};
use crate::{
    ir::{
        Align,
        InstData,
        InstFlags,
        Opcode,
        StackSlot,
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
            syntax::{
                Expression,
                ExpressionType,
            },
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
        ArenaVec,
        Bump,
    },
};

/// One thing an initializer stores, at a byte offset from the object's
/// start.
#[derive(Clone, Copy, Debug)]
pub(super) enum Leaf<'tu> {
    /// A scalar subobject and the expression whose converted value it
    /// gets.
    Scalar {
        offset:     u64,
        ty:         TypeId,
        expression: &'tu Expression<'tu>,
    },
    /// A character array of `size` bytes initialized by a string literal
    /// (§6.7.8p14).
    String {
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

/// The leaves of `initializer` for an object of type `ty`, in source order.
pub(super) fn leaves<'tu, 's>(
    sema: &SemanticTranslationUnit<'tu>,
    ty: TypeId,
    initializer: &'tu Initializer<'tu>,
    arena: &'s Bump,
) -> Result<ArenaVec<'s, Leaf<'tu>>, LoweringError> {
    let walker = Walker { sema };
    let mut leaves = ArenaVec::new_in(arena);
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
        });
        return Ok(leaves);
    }
    let mut frames = ArenaVec::new_in(arena);
    frames.push(Frame {
        ty,
        offset: 0,
        next: 0,
        list: Some((list.elements.as_slice(), 0)),
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
            frames.truncate(owner);
            continue;
        };
        if let Some(designation) = element.designation
            && designated != Some((owner, position))
        {
            walker.designate(&mut frames, owner, designation)?;
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
        let offset = frame.offset + subobject.offset;
        let consume = |frames: &mut ArenaVec<'_, Frame<'tu>>| {
            frames[top].next = subobject.next;
            if let Some((_, position)) = &mut frames[owner].list {
                *position += 1;
            }
        };
        match element.initializer.kind {
            | InitializerType::InitializerList(inner) => {
                if subobject.bit_field {
                    return Err(LoweringError::unsupported(
                        Construct::BitField,
                        element.source_vectors,
                    ));
                }
                consume(&mut frames);
                if walker.scalar(subobject.ty, element.source_vectors)? {
                    let expression = braced_scalar(element.initializer)?;
                    leaves.push(Leaf::Scalar {
                        offset,
                        ty: subobject.ty,
                        expression,
                    });
                } else {
                    frames.push(Frame {
                        ty: subobject.ty,
                        offset,
                        next: 0,
                        list: Some((inner.elements.as_slice(), 0)),
                    });
                }
            },
            | InitializerType::AssignmentExpression(expression) =>
                match walker.leaf(subobject.ty, offset, expression)? {
                    | Some(leaf) => {
                        if subobject.bit_field {
                            return Err(LoweringError::unsupported(
                                Construct::BitField,
                                element.source_vectors,
                            ));
                        }
                        consume(&mut frames);
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
}

/// The subobject an element initializes.
#[derive(Clone, Copy)]
struct Subobject {
    ty:        TypeId,
    /// Its offset from the start of the aggregate.
    offset:    u64,
    bit_field: bool,
    /// The aggregate's next position after it.
    next:      usize,
}

struct Walker<'a, 'tu> {
    sema: &'a SemanticTranslationUnit<'tu>,
}

impl<'tu> Walker<'_, 'tu> {
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
            }));
        }
        let types = &self.sema.types;
        if let TypeKind::Array(element, _) = types.kind(ty)
            && matches!(peel(expression).kind, ExpressionType::StringLiteral(_))
            && types.layout(element).is_some_and(|layout| layout.size == 1)
        {
            let size = types
                .layout(ty)
                .ok_or_else(|| LoweringError::missing("an array has no size", source))?
                .size;
            return Ok(Some(Leaf::String {
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
        if matches!(types.kind(ty), TypeKind::Tag(_))
            && converted.is_some_and(|from| from.index == ty.index)
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
                    ty:        element,
                    offset:    next as u64 * size,
                    bit_field: false,
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
                if let TypeKind::Array(_, ArrayBound::Incomplete) = types.kind(member.ty) {
                    return Err(LoweringError::unsupported(Construct::Initializer, source));
                }
                Ok(Some(Subobject {
                    ty:        member.ty,
                    offset:    member.offset,
                    bit_field: member.width.is_some(),
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
        &self,
        frames: &mut ArenaVec<'_, Frame<'tu>>,
        owner: usize,
        designation: &'tu Designation<'tu>,
    ) -> Result<(), LoweringError> {
        frames.truncate(owner + 1);
        let designators = designation.designators.as_slice();
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
                            self.open_next(frames, source)?;
                        }
                    }
                },
                | DesignatorType::Range(_) | DesignatorType::Error => return Err(unsupported()),
            }
            if index + 1 < designators.len() {
                self.open_next(frames, source)?;
            }
        }
        Ok(())
    }

    /// Opens a frame for the top frame's next subobject, which the
    /// following elements initialize.
    fn open_next(
        &self,
        frames: &mut ArenaVec<'_, Frame<'tu>>,
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        let top = frames.len() - 1;
        let frame = frames[top];
        let subobject = self
            .subobject(frame.ty, frame.next, source)?
            .ok_or_else(|| LoweringError::unsupported(Construct::Designator, source))?;
        frames[top].next = subobject.next;
        frames.push(Frame {
            ty:     subobject.ty,
            offset: frame.offset + subobject.offset,
            next:   0,
            list:   None,
        });
        Ok(())
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

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    /// Schedules the leaves of a braced or aggregate initializer: each
    /// scalar is evaluated and stored, a string literal copied, and the
    /// rest of the object was already zeroed (§6.7.8p21).
    pub(super) fn initialize_aggregate(
        &mut self,
        object: Object,
        ty: TypeId,
        initializer: &'tu Initializer<'tu>,
    ) -> Result<(), LoweringError> {
        let leaves = leaves(self.unit.sema, ty, initializer, self.scratch)?;
        let mut tasks = ArenaVec::with_capacity_in(2 * leaves.len(), self.scratch);
        for leaf in leaves {
            match leaf {
                | Leaf::Scalar {
                    offset,
                    ty,
                    expression,
                }
                | Leaf::Copy {
                    offset,
                    ty,
                    expression,
                } => {
                    tasks.push(Task::Evaluate(expression));
                    tasks.push(Task::Initialize { object, offset, ty });
                },
                | Leaf::String {
                    offset,
                    size,
                    literal,
                } => tasks.push(Task::InitializeString {
                    object,
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

    /// Stores the value on top of the stack into an object being
    /// initialized, at `offset` bytes from its start.
    pub(super) fn initialize(
        &mut self,
        object: Object,
        offset: u64,
        ty: TypeId,
    ) -> Result<(), LoweringError> {
        let item = self.pop();
        let (slot, source) = match object {
            | Object::Local(binding) => {
                let source = self.unit.sema.bindings[binding].name.source_vectors;
                match self.local(binding, source)? {
                    | Local::Variable(var) => {
                        let value = self.value(item, source)?;
                        self.draft.def_var(var, value);
                        return Ok(());
                    },
                    | Local::Slot(slot) => (slot, source),
                }
            },
            | Object::Slot(slot) => (slot, SourceVectors::empty()),
        };
        let base = self.inst(InstData::StackAddr { slot }, Type::Ptr);
        let address = self.offset_address(base, offset);
        let place = Item {
            operand: Operand::Place(Place::Memory(address)),
            ty:      ty.unqualified(),
        };
        _ = self.write(place, item, source)?;
        Ok(())
    }

    /// Copies a string literal's bytes into a character array; a literal
    /// one longer than an exactly sized array loses its null character.
    /// C99: §6.7.8 paragraph 14, p. 126; PDF p. 138.
    pub(super) fn initialize_string(
        &mut self,
        object: Object,
        offset: u64,
        size: u64,
        literal: &'tu Expression<'tu>,
    ) -> Result<(), LoweringError> {
        let source = literal.source_vectors;
        let global = self.unit.string(peel(literal))?;
        let length = self.unit.module.global(global).size.min(size);
        let slot = match object {
            | Object::Slot(slot) => slot,
            | Object::Local(binding) => match self.local(binding, source)? {
                | Local::Slot(slot) => slot,
                | Local::Variable(_) =>
                    return Err(LoweringError::missing(
                        "a character array is not in memory",
                        source,
                    )),
            },
        };
        let base = self.inst(InstData::StackAddr { slot }, Type::Ptr);
        let destination = self.offset_address(base, offset);
        let string = self.inst(InstData::GlobalAddr { global }, Type::Ptr);
        let length = self.iconst(Type::I64, i128::from(length));
        _ = self.draft.push(
            Op::Inst(InstData::MemoryRange {
                opcode: Opcode::Copy,
                flags:  InstFlags::empty(),
                align:  Align::BYTE,
                args:   [destination, string, length],
            }),
            None,
        );
        Ok(())
    }

    /// Sets every byte of an automatic object of type `ty` to zero, as the
    /// members an initializer leaves out are initialized (§6.7.8p21).
    pub(super) fn zero_fill(
        &mut self,
        slot: StackSlot,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        let layout = types::layout(&self.unit.sema.types, ty).map_err(|kind| at(kind, source))?;
        let address = self.inst(InstData::StackAddr { slot }, Type::Ptr);
        let zero = self.iconst(Type::I8, 0);
        let size = self.iconst(Type::I64, i128::from(layout.size));
        let flags = if types::is_volatile(ty) {
            InstFlags::VOLATILE
        } else {
            InstFlags::empty()
        };
        _ = self.draft.push(
            Op::Inst(InstData::MemoryRange {
                opcode: Opcode::Fill,
                flags,
                align: types::align(layout),
                args: [address, zero, size],
            }),
            None,
        );
        Ok(())
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
        let slot = self.draft.create_slot(size, types::align(layout));
        if self.repr(ty, source)? == Repr::Aggregate {
            self.zero_fill(slot, ty, source)?;
        }
        self.tasks.push(Task::PushSlot { slot, ty });
        self.initialize_aggregate(Object::Slot(slot), ty, initializer)
    }
}
