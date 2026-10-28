//! Lvalues: where an identifier's object is, reading a place and writing
//! one, and the small instruction helpers the rest of lowering builds on.
//!
//! An automatic scalar whose address is never taken is an SSA variable; any
//! other automatic object has a stack slot; a static-duration object is a
//! global. A read of a variable is a [`Draft::use_var`](super::ssa::Draft)
//! and a write a `def_var`; memory is read and written with `load` and
//! `store` carrying the place's guaranteed alignment and `volatile`, and a
//! structure is copied with `copy`.
//! C99: §6.3.2.1 paragraphs 1-2, p. 46; PDF p. 58; §6.5.16.1 paragraph 2,
//! p. 92; PDF p. 104.

use super::{
    Construct,
    FunctionLowerer,
    LoweringError,
    module_items::at,
    ssa::Op,
    types::{
        self,
        Repr,
        repr,
    },
    work::{
        Item,
        Local,
        Operand,
        Place,
    },
};
use crate::{
    ir::{
        FloatCC,
        InstData,
        InstFlags,
        IntCC,
        MemFlags,
        Opcode,
        Type,
        Value,
    },
    translation_phases::{
        SourceVectors,
        parsing::syntax::Expression,
        semantic_analysis::{
            BindingKind,
            Conversion,
            Duration,
            ExpressionInfo,
            TypeId,
            TypeKind,
        },
    },
};

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    /// The place or value an identifier designates.
    /// C99: §6.5.1 paragraph 2, p. 69; PDF p. 81.
    pub(super) fn identifier(
        &mut self,
        info: &ExpressionInfo<'tu>,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let binding = self
            .unit
            .sema
            .function_extensions
            .value(info.expression)
            .or(info.binding)
            .ok_or_else(|| LoweringError::missing("an identifier has no binding", source))?;
        let data = self.unit.sema.bindings[binding];
        let place = match data.kind {
            | BindingKind::Function => Place::Function(
                self.unit
                    .function(binding)
                    .map_err(|kind| at(kind, source))?,
            ),
            | BindingKind::Object | BindingKind::Parameter => match data.duration {
                // A thread-local global's address is the evaluating
                // thread's copy (C11 §6.2.4p4).
                | Duration::Static | Duration::Thread => {
                    let global = self.unit.object(binding).map_err(|kind| at(kind, source))?;
                    Place::Memory(self.inst(InstData::GlobalAddr { global }, Type::Ptr))
                },
                | _ => match self.locals.get(&binding).copied() {
                    | Some(Local::Variable(var)) => Place::Variable(var),
                    | Some(Local::Slot(slot)) =>
                        Place::Memory(self.inst(InstData::StackAddr { slot }, Type::Ptr)),
                    | Some(Local::Address(address)) => Place::Memory(address),
                    | None =>
                        return Err(LoweringError::missing(
                            "an automatic object is used outside its function",
                            source,
                        )),
                },
            },
            | BindingKind::Enumerator | BindingKind::Typedef =>
                return Err(LoweringError::missing("an identifier has no value", source)),
        };
        Ok(Item {
            operand: Operand::Place(place),
            ty:      info.ty,
        })
    }

    /// The storage of an automatic object, allocated on first use.
    pub(super) fn local(
        &mut self,
        binding: usize,
        source: SourceVectors,
    ) -> Result<Local, LoweringError> {
        if let Some(&local) = self.locals.get(&binding) {
            return Ok(local);
        }
        let ty = self.unit.sema.bindings[binding].ty;
        if self.unit.sema.types.unanalyzed(ty) {
            return Err(LoweringError::unanalyzed(&self.unit.sema.types, ty, source));
        }
        let representation = self.repr(ty, source)?;
        let local = match representation.value_type() {
            | Some(value_type)
                if !types::is_volatile(ty)
                    && !matches!(self.unit.sema.types.kind(ty), TypeKind::Atomic(_))
                    && !self.unit.address_taken.contains(&binding) =>
                Local::Variable(self.draft.declare_var(value_type)),
            | _ if self.unit.sema.types.layout(ty).is_none()
                && matches!(self.unit.sema.types.kind(ty), TypeKind::Array(..)) =>
            {
                let size = self.runtime_size(ty, source)?;
                self.enter_vla(binding, source)?;
                let align = super::alignment::variable_alignment(
                    self.unit.sema,
                    self.unit.target,
                    binding,
                    ty,
                )
                .map_err(|kind| at(kind, source))?;
                Local::Address(self.dynamic_object(size, Some(align)))
            },
            | _ => {
                let layout =
                    types::layout(&self.unit.sema.types, ty).map_err(|kind| at(kind, source))?;
                let size = u32::try_from(layout.size).map_err(|_| {
                    LoweringError::unsupported(Construct::LargeAutomaticObject, source)
                })?;
                Local::Slot(
                    self.object_slot(
                        size,
                        super::alignment::variable_alignment(
                            self.unit.sema,
                            self.unit.target,
                            binding,
                            ty,
                        )
                        .map_err(|kind| at(kind, source))?,
                    ),
                )
            },
        };
        _ = self.locals.insert(binding, local);
        Ok(local)
    }

    /// The value of the object at `place`, whose lvalue type is `ty`,
    /// converted to type `to` by lvalue conversion. A structure is not
    /// loaded: its value is its address. A volatile aggregate is copied
    /// at conversion, before the value loses its lvalue qualifiers.
    /// C99: §6.3.2.1 paragraph 2, p. 46; PDF p. 58; §6.7.3 paragraph 6,
    /// p. 109; PDF p. 121.
    pub(super) fn read(
        &mut self,
        place: Place,
        ty: TypeId,
        to: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        if matches!(self.unit.sema.types.kind(ty), TypeKind::Atomic(_))
            && let Place::Memory(address) = place
        {
            return self.atomic_read(address, ty, to, source);
        }
        let operand = match (place, self.repr(ty, source)?) {
            | (Place::BitField(address, field), _) => {
                let result = self.value_type(ty, source)?;
                Operand::Value(self.load_bit_field(address, field, types::is_volatile(ty), result))
            },
            | (Place::Memory(address), Repr::Complex { .. }) =>
                self.load_complex(address, ty, source)?,
            | (Place::Memory(address), Repr::Aggregate) => {
                let layout =
                    types::layout(&self.unit.sema.types, ty).map_err(|kind| at(kind, source))?;
                let guaranteed = self.place_alignment(address, types::align(layout));
                self.retain_alignment(address, guaranteed);
                let address = if types::is_volatile(ty) {
                    let align = guaranteed;
                    let copy = self.fresh_object(layout.size, align, source)?;
                    let size = self.iconst(Type::I64, i128::from(layout.size));
                    _ = self.draft.push(
                        self.memory_op(InstData::MemoryRange {
                            opcode: Opcode::Copy,
                            flags: InstFlags::VOLATILE,
                            align,
                            args: [copy, address, size],
                        }),
                        None,
                    );
                    copy
                } else {
                    address
                };
                Operand::Aggregate(address)
            },
            | (Place::Variable(var), _) => Operand::Value(self.draft.use_var(var)),
            // `*pv` with `pv` a `void *` designates no object, so its value
            // is a void expression and nothing is accessed (DR 106).
            // C99 §6.3.2.2p1, p. 47; PDF p. 59.
            | (Place::Memory(_), Repr::Void) => Operand::Void,
            | (Place::Memory(address), representation) => {
                let value_type = representation
                    .value_type()
                    .ok_or_else(|| LoweringError::missing("a read has no value type", source))?;
                let flags = self.mem_flags(ty, source)?;
                let loaded = self.inst(
                    InstData::Load {
                        ty:    value_type,
                        flags: volatile(flags),
                        align: flags.align,
                        tag:   None,
                        addr:  address,
                    },
                    value_type,
                );
                // The padding bits of a narrow bit-precise object need not
                // extend its value: GCC and Clang store them extended but
                // read none of them, and Clang passes them unextended.
                Operand::Value(self.normalize(loaded, representation))
            },
            | (Place::Function(_), _) =>
                return Err(LoweringError::missing(
                    "a function is read as a value",
                    source,
                )),
        };
        Ok(Item { operand, ty: to })
    }

    /// Stores `value` into the lvalue `place` and returns the stored value,
    /// which is the value of an assignment expression (§6.5.16p3).
    pub(super) fn write(
        &mut self,
        place: Item,
        value: Item,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let value = self.settle(value);
        let ty = place.ty;
        let Operand::Place(target) = place.operand else {
            return Err(LoweringError::missing(
                "an assignment target is not an lvalue",
                source,
            ));
        };
        if matches!(self.unit.sema.types.kind(ty), TypeKind::Atomic(_))
            && let Place::Memory(address) = target
        {
            self.atomic_write(address, ty, value, source)?;
            return Ok(Item {
                operand: value.operand,
                ty:      self.unit.sema.types.non_atomic(ty).unqualified(),
            });
        }
        let stored = match (target, value.operand) {
            | (Place::Memory(address), Operand::Complex(_)) => {
                self.store_complex(address, Item { ty, ..value }, source)?;
                value.operand
            },
            | (Place::Memory(address), Operand::Aggregate(from)) => {
                // C99 §6.5.16.1p3: the objects may overlap only exactly.
                let layout =
                    types::layout(&self.unit.sema.types, ty).map_err(|kind| at(kind, source))?;
                let guaranteed = self.place_alignment(address, types::align(layout));
                self.retain_alignment(address, guaranteed);
                let size = self.iconst(Type::I64, i128::from(layout.size));
                let flags = if types::is_volatile(ty) {
                    InstFlags::MAY_OVERLAP | InstFlags::VOLATILE
                } else {
                    InstFlags::MAY_OVERLAP
                };
                _ = self.draft.push(
                    self.memory_op(InstData::MemoryRange {
                        opcode: Opcode::Copy,
                        flags,
                        align: self.place_alignment(from, crate::ir::Align::BYTE),
                        args: [address, from, size],
                    }),
                    None,
                );
                Operand::Aggregate(address)
            },
            | (Place::Variable(var), _) => {
                let value = self.value(value, source)?;
                self.draft.def_var(var, value);
                Operand::Value(value)
            },
            | (Place::BitField(address, field), _) => {
                let value = self.value(value, source)?;
                let result = self.value_type(ty, source)?;
                let volatile = types::is_volatile(ty);
                Operand::Value(self.store_bit_field(address, field, volatile, value, result))
            },
            | (Place::Memory(address), _) => {
                let value = self.value(value, source)?;
                self.store(value, address, ty, source)?;
                Operand::Value(value)
            },
            | (Place::Function(_), _) =>
                return Err(LoweringError::missing("a function is assigned", source)),
        };
        Ok(Item {
            operand: stored,
            ty:      ty.unqualified(),
        })
    }

    /// Stores a scalar of C type `ty` at `address`.
    pub(super) fn store(
        &mut self,
        value: Value,
        address: Value,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        if matches!(self.unit.sema.types.kind(ty), TypeKind::Atomic(_)) {
            return self.atomic_write(
                address,
                ty,
                Item {
                    operand: Operand::Value(value),
                    ty:      self.unit.sema.types.non_atomic(ty),
                },
                source,
            );
        }
        let flags = self.mem_flags(ty, source)?;
        let value_type = self.draft.value_type(value);
        _ = self.draft.push(
            self.memory_op(InstData::Store {
                ty:    value_type,
                flags: volatile(flags),
                align: flags.align,
                tag:   None,
                args:  [value, address],
            }),
            None,
        );
        Ok(())
    }

    /// The access facts of an object of type `ty`: its alignment and
    /// whether it is volatile (§6.7.3p6).
    fn mem_flags(&self, ty: TypeId, source: SourceVectors) -> Result<MemFlags, LoweringError> {
        let align = super::alignment::storage_alignment(&self.unit.sema.types, ty)
            .map_err(|kind| at(kind, source))?;
        Ok(MemFlags {
            volatile: types::is_volatile(ty),
            align,
            tag: None,
        })
    }

    /// `base` plus a constant byte offset.
    pub(super) fn offset_address(&mut self, base: Value, offset: u64) -> Value {
        if offset == 0 {
            return base;
        }
        let offset = self.iconst(Type::I64, i128::from(offset));
        self.ptr_add(base, offset)
    }
}

// Semantic facts

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    /// The record semantic analysis retained for `expression`.
    pub(super) fn info(
        &self,
        expression: &'tu Expression<'tu>,
    ) -> Result<&'tu ExpressionInfo<'tu>, LoweringError> {
        self.unit.sema.expression_info(expression).ok_or_else(|| {
            LoweringError::missing("an expression has no record", expression.source_vectors)
        })
    }

    /// The conversions recorded on `expression`, in application order.
    pub(super) fn conversions(&self, expression: &'tu Expression<'tu>) -> &'tu [Conversion<'tu>] {
        self.unit.sema.expression_conversions(expression)
    }

    pub(super) fn repr(&self, ty: TypeId, source: SourceVectors) -> Result<Repr, LoweringError> {
        repr(&self.unit.sema.types, ty).map_err(|kind| at(kind, source))
    }

    /// The IR type of a scalar C type.
    pub(super) fn value_type(
        &self,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Type, LoweringError> {
        self.repr(ty, source)?
            .value_type()
            .ok_or_else(|| LoweringError::missing("a scalar has no value type", source))
    }

    /// Pointer representation includes the distinct C23 null pointer type.
    /// C23: §7.21.2p2, p. 316; PDF p. 329.
    pub(super) fn is_pointer(&self, ty: TypeId) -> bool {
        matches!(
            self.unit.sema.types.kind(ty),
            TypeKind::Pointer(_) | TypeKind::Nullptr
        )
    }

    /// The width of `int`, below which integers are promoted.
    pub(super) fn int_bits(&self) -> u32 {
        self.unit
            .sema
            .types
            .target
            .integer(crate::target::Scalar::Int)
            .map_or(32, |(bits, _)| bits)
    }

    /// Pops the top operand, rounding a `_Float16` excess (`half.rs`).
    pub(super) fn pop(&mut self) -> Item {
        let item = self.pop_excess();
        self.settle(item)
    }

    /// Pops the operand a truth test reads: GCC tests a `_Float16` excess
    /// itself, Clang the rounded value (`half.rs`).
    pub(super) fn pop_truth(&mut self) -> Item {
        if self.half_in_float() {
            self.pop_excess()
        } else {
            self.pop()
        }
    }

    /// Pops the top operand, keeping a `_Float16` excess.
    pub(super) fn pop_excess(&mut self) -> Item {
        self.operands.pop().expect("an operand is on the stack")
    }

    pub(super) fn push_item(&mut self, operand: Operand, ty: TypeId) {
        self.operands.push(Item { operand, ty });
    }
}

// Instructions

impl FunctionLowerer<'_, '_, '_, '_, '_> {
    /// Appends an instruction with a result of type `ty`.
    pub(super) fn inst(&mut self, mut inst: InstData, ty: Type) -> Value {
        self.align_memory(&mut inst);
        let alignment = self.instruction_alignment(inst);
        let value = self
            .draft
            .push(Op::Inst(inst), Some(ty))
            .expect("the instruction has a result");
        if let Some(align) = alignment {
            self.retain_alignment(value, align);
        }
        value
    }

    pub(super) fn iconst(&mut self, ty: Type, value: i128) -> Value {
        let value = self.cast_constant(value, ty, true, ty);
        self.draft
            .push(Op::Iconst(ty, value), Some(ty))
            .expect("a constant has a result")
    }

    /// The flags of signed integer `+`, `-`, `*` and `<<`: `nsw`, because
    /// signed overflow is undefined (C99 §6.5p5, p. 67; PDF p. 79), unless
    /// `-fwrapv` makes it wrap, an extension.
    pub(super) fn signed_wrap(&self) -> InstFlags {
        if self.unit.context.configuration.wrapv() {
            InstFlags::empty()
        } else {
            InstFlags::NSW
        }
    }

    pub(super) fn binary_inst(
        &mut self,
        opcode: Opcode,
        flags: InstFlags,
        a: Value,
        b: Value,
    ) -> Value {
        let ty = self.draft.value_type(a);
        self.inst(
            InstData::Binary {
                opcode,
                ty,
                flags,
                args: [a, b],
            },
            ty,
        )
    }

    pub(super) fn icmp(&mut self, cond: IntCC, a: Value, b: Value) -> Value {
        let ty = self.draft.value_type(a);
        self.inst(
            InstData::IntCompare {
                cond,
                ty,
                args: [a, b],
            },
            Type::I1,
        )
    }

    /// A floating constant of type `ty` given by its bits.
    pub(super) fn fconst(&mut self, ty: Type, bits: u128) -> Value {
        self.draft
            .push(Op::Fconst(ty, bits), Some(ty))
            .expect("a constant has a result")
    }

    pub(super) fn fcmp(&mut self, cond: FloatCC, a: Value, b: Value) -> Value {
        let ty = self.draft.value_type(a);
        self.inst(
            InstData::FloatCompare {
                cond,
                ty,
                args: [a, b],
            },
            Type::I1,
        )
    }

    pub(super) fn convert(&mut self, opcode: Opcode, to: Type, value: Value) -> Value {
        self.inst(
            InstData::Unary {
                opcode,
                ty: to,
                arg: value,
            },
            to,
        )
    }

    /// `null` or another operation without operands.
    pub(super) fn nullary(&mut self, opcode: Opcode) -> Value {
        self.inst(
            InstData::Nullary {
                opcode,
                ty: Type::Ptr,
            },
            Type::Ptr,
        )
    }

    /// `base` plus `offset` bytes, staying within the object it points into
    /// (§6.5.6p8).
    pub(super) fn ptr_add(&mut self, base: Value, offset: Value) -> Value {
        self.inst(
            InstData::Binary {
                opcode: Opcode::PtrAdd,
                ty:     Type::Ptr,
                flags:  InstFlags::INBOUNDS,
                args:   [base, offset],
            },
            Type::Ptr,
        )
    }

    /// An integer value converted to the integer type `to`: sign- or
    /// zero-extended from its own signedness, or truncated. Constants fold.
    /// C99: §6.3.1.3, p. 43; PDF p. 55.
    pub(super) fn extend(&mut self, value: Value, signed: bool, to: Type) -> Value {
        let from = self.draft.value_type(value);
        if from == to {
            return value;
        }
        if let Some(constant) = self.draft.constant(value) {
            let constant = self.cast_constant(constant, from, signed, to);
            return self.iconst(to, constant);
        }
        let opcode = if from.bits() > to.bits() {
            Opcode::Trunc
        } else if signed {
            Opcode::Sext
        } else {
            Opcode::Zext
        };
        self.convert(opcode, to, value)
    }

    /// A value of the representation `representation` extended again from
    /// its width to its IR type, after an operation computed it in that
    /// type: a bit-precise integer narrower than its IR type keeps the bits
    /// above its width as copies of its sign bit, or zero if unsigned, so
    /// that every operation of the IR type reads its value. This also wraps
    /// an unsigned result modulo 2^width. Other values are unchanged.
    /// C23: §6.2.6.2 paragraphs 1-2, pp. 42-43; PDF pp. 55-56; §6.2.5
    /// paragraph 11, p. 40; PDF p. 53.
    pub(super) fn normalize(&mut self, value: Value, representation: Repr) -> Value {
        let Some((ty, signed, width)) = representation.narrow() else {
            return value;
        };
        let spare = ty.bits() - width;
        if let Some(constant) = self.draft.constant(value) {
            let low = constant.cast_unsigned() & (u128::MAX >> (128 - width));
            let extended = if signed && (low >> (width - 1)) & 1 == 1 {
                (low | !(u128::MAX >> (128 - width))).cast_signed()
            } else {
                low.cast_signed()
            };
            return self.iconst(ty, extended);
        }
        if signed {
            let count = self.iconst(ty, i128::from(spare));
            let raised = self.binary_inst(Opcode::Shl, InstFlags::empty(), value, count);
            self.binary_inst(Opcode::Ashr, InstFlags::empty(), raised, count)
        } else {
            let mask = self.iconst(ty, (u128::MAX >> (128 - width)).cast_signed());
            self.binary_inst(Opcode::And, InstFlags::empty(), value, mask)
        }
    }

    /// A constant of type `from` converted to `to`, as the IR prints it:
    /// the low bits, read back signed.
    #[expect(
        clippy::unused_self,
        reason = "Kept beside the other constant helpers for discoverability."
    )]
    pub(super) fn cast_constant(&self, value: i128, from: Type, signed: bool, to: Type) -> i128 {
        let bits = value as u128 & from.mask();
        let extended = if signed && from.bits() < 128 && bits >> (from.bits() - 1) & 1 == 1 {
            (bits | !from.mask()) as i128
        } else {
            bits as i128
        };
        let truncated = extended as u128 & to.mask();
        if to.bits() < 128 && truncated >> (to.bits() - 1) & 1 == 1 {
            (truncated | !to.mask()) as i128
        } else {
            truncated as i128
        }
    }
}

fn volatile(flags: MemFlags) -> InstFlags {
    if flags.volatile {
        InstFlags::VOLATILE
    } else {
        InstFlags::empty()
    }
}
