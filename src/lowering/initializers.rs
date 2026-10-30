//! Storing an initializer into the object it initializes.
//!
//! Semantic analysis retains a placement plan for every initializer it
//! accepts: the subobject each element initializes, at its absolute offset,
//! with its completed qualified type, the brace pairs whose omitted bytes are
//! zero, the union members selected and the GNU ranges repeated
//! (`facts/initializer_plans.rs`). `plan.rs` replays the plan as source-ordered
//! leaves, ending a range whose elements all get the first's bytes with one
//! copy of them; this file schedules their automatic stores, evaluating each
//! value once and zeroing only the bytes no later leaf of its scope covers
//! (`coverage.rs`), and `module_items.rs` maps the same leaves to a static
//! object's bytes and relocations. Lowering never chooses a subobject itself.
//!
//! C99: §6.7.8 paragraphs 12-22, pp. 126-127; PDF pp. 138-139.

mod coverage;
mod plan;

use coverage::CoverageIndex;
pub(super) use plan::Storage;
#[cfg(test)]
pub(super) use plan::{
    MINIMUM as REPLICATION_MINIMUM,
    STEPS as REPLAY_STEPS,
};

use super::{
    Construct,
    FunctionLowerer,
    LoweringError,
    LoweringErrorKind,
    module_items::{
        at,
        peel,
    },
    types,
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
        Value,
    },
    translation_phases::{
        SourceVectors,
        parsing::{
            declaration_syntax::Initializer,
            syntax::Expression,
        },
        semantic_analysis::{
            InitValueId,
            InitializerPlan,
            SemanticTranslationUnit,
            TypeId,
            TypeKind,
            Types,
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
        /// The plan value `expression` evaluates.
        value:      InitValueId,
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
        /// The plan value `expression` evaluates.
        value:      InitValueId,
    },
    /// The later elements of a GNU range: the `size` bytes at `offset`,
    /// which the leaves before it wrote into the first element, copied
    /// `stride`, `2 * stride`, ... up to `count * stride` bytes further on.
    Replicate {
        offset: u64,
        size:   u64,
        stride: u64,
        count:  u64,
        /// The type of the first leaf it copies.
        ty:     TypeId,
        /// Every copied byte is zero.
        zero:   bool,
    },
}

impl Leaf<'_> {
    const fn ty(self) -> TypeId {
        match self {
            | Self::Zero { ty, .. }
            | Self::Scalar { ty, .. }
            | Self::String { ty, .. }
            | Self::Copy { ty, .. }
            | Self::Replicate { ty, .. } => ty,
        }
    }

    /// The bits the leaf writes, from the object's start. A replication
    /// whose copies are not adjacent reports none.
    pub(super) fn bits(self, table: &Types<'_>) -> Result<(u64, u64), LoweringErrorKind> {
        Ok(match self {
            | Self::Zero { offset, size, .. } | Self::String { offset, size, .. } =>
                (8 * offset, 8 * (offset + size)),
            | Self::Scalar {
                offset,
                bit_field: Some(bits),
                ..
            } => {
                let start = 8 * offset + u64::from(bits.bit_offset);
                (start, start + u64::from(bits.width))
            },
            | Self::Scalar { offset, ty, .. } | Self::Copy { offset, ty, .. } =>
                (8 * offset, 8 * (offset + types::layout(table, ty)?.size)),
            | Self::Replicate {
                offset,
                size,
                stride,
                count,
                ..
            } if size == stride => (8 * (offset + size), 8 * (offset + (count + 1) * size)),
            | Self::Replicate { offset, .. } => (8 * offset, 8 * offset),
        })
    }
}

/// The stores of `initializer` for an object of type `ty` and `storage`
/// duration, in source order, from the placement plan semantic analysis
/// retained for it, and that plan.
/// C99: §6.7.8 paragraphs 14-19, pp. 126-127; PDF pp. 138-139.
pub(super) fn leaves<'a, 'tu, 's>(
    sema: &'a SemanticTranslationUnit<'tu>,
    ty: TypeId,
    initializer: &'tu Initializer<'tu>,
    storage: Storage,
    arena: &'s Bump,
) -> Result<(ArenaVec<'s, Leaf<'tu>>, &'a InitializerPlan<'tu>), LoweringError> {
    let plan = sema.initializers.plan(initializer).ok_or_else(|| {
        LoweringError::missing(
            "an initializer has no retained plan",
            initializer.source_vectors,
        )
    })?;
    debug_assert_eq!(plan.ty, ty, "the plan completes the object's type");
    Ok((plan::plan_leaves(sema, plan, storage, arena)?, plan))
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
        let (leaves, plan) = leaves(
            self.unit.sema,
            ty,
            initializer,
            Storage::Automatic,
            self.scratch,
        )?;
        // An initializer of zeroes clears the whole object at once, as
        // Clang's does. A typed store would leave LLVM's SROA an `f80` view
        // of a union, through which it copies other members' bytes on the
        // x87 stack, which does not keep every bit pattern.
        if leaves
            .iter()
            .all(|&leaf| plan::zero_leaf(self.unit.sema, leaf))
        {
            let size = types::layout(&self.unit.sema.types, ty)
                .map_err(|kind| at(kind, initializer.source_vectors))?
                .size;
            self.tasks.push(Task::InitializeZero {
                object,
                ty,
                offset: 0,
                size,
                mask: u8::MAX,
            });
            return Ok(());
        }
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
        // Where each plan value a range repeats is remembered once evaluated.
        let mut remembered = ArenaVec::with_capacity_in(plan.values.len(), self.scratch);
        remembered.resize(plan.values.len(), usize::MAX);
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
                    value,
                    bit_field,
                } => {
                    self.initializer_expression(
                        &mut tasks,
                        &mut remembered,
                        plan.values[value.0 as usize].repeated.then_some(value),
                        expression,
                        ty,
                    );
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
                    value,
                } => {
                    self.initializer_expression(
                        &mut tasks,
                        &mut remembered,
                        plan.values[value.0 as usize].repeated.then_some(value),
                        expression,
                        ty,
                    );
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
                | Leaf::Replicate {
                    offset,
                    size,
                    stride,
                    count,
                    zero,
                    ..
                } => tasks.push(Task::InitializeCopies {
                    object,
                    offset,
                    size,
                    stride,
                    count,
                    zero,
                }),
            }
        }
        // The first leaf runs first.
        self.tasks.extend(tasks.iter().rev().copied());
        Ok(())
    }

    /// Evaluates the expression of a plan value. A GNU range stores one
    /// value into each element (`repeated`): its first store evaluates the
    /// expression once and remembers the converted value, including an
    /// aggregate's, which the later stores reuse.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Designated-Inits.html>
    fn initializer_expression(
        &mut self,
        tasks: &mut ArenaVec<'s, Task<'tu>>,
        remembered: &mut [usize],
        repeated: Option<InitValueId>,
        expression: &'tu Expression<'tu>,
        ty: TypeId,
    ) {
        let Some(value) = repeated else {
            tasks.push(Task::Evaluate(expression));
            return;
        };
        let slot = &mut remembered[value.0 as usize];
        if *slot == usize::MAX {
            let index = self.initializer_values.len();
            self.initializer_values.push(Item {
                operand: Operand::Void,
                ty,
            });
            *slot = index;
            tasks.push(Task::Evaluate(expression));
            tasks.push(Task::SnapshotArgument(expression));
            tasks.push(Task::RememberInitializer(index));
        } else {
            tasks.push(Task::ReuseInitializer(*slot));
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
        let base = self.object_address(object)?;
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

    /// Gives the later elements of a GNU range the first element's `size`
    /// bytes at `offset`: one zero fill when they are zero, otherwise
    /// copies that double the initialized elements each time, so at most
    /// 64. Copies that are not adjacent are made one at a time.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Designated-Inits.html>
    pub(super) fn initialize_copies(
        &mut self,
        object: Object,
        offset: u64,
        size: u64,
        stride: u64,
        count: u64,
        zero: bool,
    ) -> Result<(), LoweringError> {
        let base = self.object_address(object)?;
        let first = self.offset_address(base, offset);
        if size != stride {
            for copy in 1..=count {
                let destination = self.offset_address(first, copy * stride);
                let length = self.iconst(Type::I64, i128::from(size));
                self.memory_range(Opcode::Copy, [destination, first, length]);
            }
            return Ok(());
        }
        if zero {
            let destination = self.offset_address(first, stride);
            let byte = self.iconst(Type::I8, 0);
            let length = self.iconst(Type::I64, i128::from(count * stride));
            self.memory_range(Opcode::Fill, [destination, byte, length]);
            return Ok(());
        }
        let mut done = 1_u64;
        while done <= count {
            let doubled = done.min(count + 1 - done);
            let destination = self.offset_address(first, done * stride);
            let length = self.iconst(Type::I64, i128::from(doubled * stride));
            self.memory_range(Opcode::Copy, [destination, first, length]);
            done += doubled;
        }
        Ok(())
    }

    /// A `copy` or `fill` of bytes no volatile object holds.
    fn memory_range(&mut self, opcode: Opcode, args: [Value; 3]) {
        _ = self.draft.push(
            self.memory_op(InstData::MemoryRange {
                opcode,
                flags: InstFlags::empty(),
                align: Align::BYTE,
                args,
            }),
            None,
        );
    }

    /// The address of an aggregate being initialized.
    fn object_address(&mut self, object: Object) -> Result<Value, LoweringError> {
        Ok(match object {
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
        })
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
        let mut item = self.pop();
        // GNU: an array compound literal initializes an array whole, as a
        // copy of the literal's object.
        if let Operand::Place(Place::Memory(address)) = item.operand
            && matches!(self.unit.sema.types.kind(ty), TypeKind::Array(..))
        {
            item.operand = Operand::Aggregate(address);
        }
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
        let size = u32::try_from(layout.size)
            .map_err(|_| LoweringError::unsupported(Construct::LargeAutomaticObject, source))?;
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
