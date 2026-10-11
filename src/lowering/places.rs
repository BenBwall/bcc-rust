//! Lvalues: where an identifier's object is, reading a place and writing
//! one, and the small instruction helpers the rest of lowering builds on.
//!
//! An automatic scalar whose address is never taken is an SSA variable; any
//! other automatic object has a stack slot; a static-duration object is a
//! global. A read of a variable is a [`Draft::use_var`](super::ssa::Draft)
//! and a write a `def_var`; memory is read and written with `load` and
//! `store` carrying the type's alignment and `volatile`, and a structure is
//! copied with `copy`.
//! C99: §6.3.2.1 paragraphs 1-2, p. 46; PDF p. 58; §6.5.16.1 paragraph 2,
//! p. 92; PDF p. 104.

use super::{
    FunctionLowerer,
    Local,
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
        Operand,
        Place,
    },
};
use crate::{
    ir::{
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
        let binding = info
            .binding
            .ok_or_else(|| LoweringError::missing("an identifier has no binding", source))?;
        let data = self.unit.sema.bindings[binding];
        let place = match data.kind {
            | BindingKind::Function => Place::Function(
                self.unit
                    .function(binding)
                    .map_err(|kind| at(kind, source))?,
            ),
            | BindingKind::Object | BindingKind::Parameter => match data.duration {
                | Duration::Static => {
                    let global = self.unit.object(binding).map_err(|kind| at(kind, source))?;
                    Place::Memory(self.inst(InstData::GlobalAddr { global }, Type::Ptr))
                },
                | _ => match self.locals.get(&binding).copied() {
                    | Some(Local::Variable(var)) => Place::Variable(var),
                    | Some(Local::Slot(slot)) =>
                        Place::Memory(self.inst(InstData::StackAddr { slot }, Type::Ptr)),
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
            return Err(LoweringError::unanalyzed(source));
        }
        let representation = self.repr(ty, source)?;
        let local = match representation.value_type() {
            | Some(value_type)
                if !types::is_volatile(ty) && !self.unit.address_taken.contains(&binding) =>
                Local::Variable(self.draft.declare_var(value_type)),
            | _ => {
                let layout =
                    types::layout(&self.unit.sema.types, ty).map_err(|kind| at(kind, source))?;
                let size = u32::try_from(layout.size).map_err(|_| {
                    LoweringError::missing("an automatic object is larger than 4 GiB", source)
                })?;
                Local::Slot(self.draft.create_slot(size, types::align(layout)))
            },
        };
        _ = self.locals.insert(binding, local);
        Ok(local)
    }

    /// The value of the object at `place`, whose lvalue type is `ty`,
    /// converted to type `to` by lvalue conversion. A structure is not
    /// loaded: its value is its address.
    pub(super) fn read(
        &mut self,
        place: Place,
        ty: TypeId,
        to: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let operand = match (place, self.repr(ty, source)?) {
            | (Place::Memory(address), Repr::Aggregate) => Operand::Aggregate(address),
            | (Place::Variable(var), _) => Operand::Value(self.draft.use_var(var)),
            | (Place::Memory(address), representation) => {
                let value_type = representation
                    .value_type()
                    .ok_or_else(|| LoweringError::missing("a read has no value type", source))?;
                let flags = self.mem_flags(ty, source)?;
                Operand::Value(self.inst(
                    InstData::Load {
                        ty:    value_type,
                        flags: volatile(flags),
                        align: flags.align,
                        tag:   None,
                        addr:  address,
                    },
                    value_type,
                ))
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
        let ty = place.ty;
        let Operand::Place(target) = place.operand else {
            return Err(LoweringError::missing(
                "an assignment target is not an lvalue",
                source,
            ));
        };
        let stored = match (target, value.operand) {
            | (Place::Memory(address), Operand::Aggregate(from)) => {
                // C99 §6.5.16.1p3: the objects may overlap only exactly.
                let layout =
                    types::layout(&self.unit.sema.types, ty).map_err(|kind| at(kind, source))?;
                let size = self.iconst(Type::I64, i128::from(layout.size));
                let flags = if types::is_volatile(ty) {
                    InstFlags::MAY_OVERLAP | InstFlags::VOLATILE
                } else {
                    InstFlags::MAY_OVERLAP
                };
                _ = self.draft.push(
                    Op::Inst(InstData::MemoryRange {
                        opcode: Opcode::Copy,
                        flags,
                        align: types::align(layout),
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
        let flags = self.mem_flags(ty, source)?;
        let value_type = self.draft.value_type(value);
        _ = self.draft.push(
            Op::Inst(InstData::Store {
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
        let layout = types::layout(&self.unit.sema.types, ty).map_err(|kind| at(kind, source))?;
        Ok(MemFlags {
            volatile: types::is_volatile(ty),
            align:    types::align(layout),
            tag:      None,
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

    pub(super) fn is_pointer(&self, ty: TypeId) -> bool {
        matches!(self.unit.sema.types.kind(ty), TypeKind::Pointer(_))
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

    pub(super) fn pop(&mut self) -> Item {
        self.operands.pop().expect("an operand is on the stack")
    }

    pub(super) fn push_item(&mut self, operand: Operand, ty: TypeId) {
        self.operands.push(Item { operand, ty });
    }
}

// Instructions

impl FunctionLowerer<'_, '_, '_, '_, '_> {
    /// Appends an instruction with a result of type `ty`.
    pub(super) fn inst(&mut self, inst: InstData, ty: Type) -> Value {
        self.draft
            .push(Op::Inst(inst), Some(ty))
            .expect("the instruction has a result")
    }

    pub(super) fn iconst(&mut self, ty: Type, value: i128) -> Value {
        let value = self.cast_constant(value, ty, true, ty);
        self.draft
            .push(Op::Iconst(ty, value), Some(ty))
            .expect("a constant has a result")
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
