//! Guaranteed place alignment, independent of the type of a stored value.
//! Storage owners register their guarantees; projections refine them. Unknown
//! C pointers use the retained pointee/place alignment supplied by the caller.
//! C99: §6.3.2.3p7, pp. 47-48; PDF pp. 59-60 (pointer alignment);
//! §6.7.2.1p12, p. 103; PDF p. 115 (member alignment).

use super::{
    FunctionLowerer,
    LoweringError,
    LoweringErrorKind,
    ssa::Op,
};
use crate::{
    ir::{
        Align,
        Entity,
        InstData,
        Opcode,
        StackSlot,
        Type,
        Value,
    },
    translation_phases::{
        SourceVectors,
        semantic_analysis::{
            TypeId,
            Types,
        },
    },
};

impl<'s> FunctionLowerer<'_, '_, '_, '_, 's> {
    /// The backing storage wins over any nominal or converted type. The
    /// default applies only to an opaque C pointer, never known storage.
    pub(super) fn place_alignment(&self, address: Value, default: Align) -> Align {
        self.alignments.get(&address).copied().unwrap_or(default)
    }

    pub(super) fn retain_alignment(&mut self, address: Value, align: Align) {
        _ = self.alignments.insert(address, align);
    }

    /// Establish C's default guarantee for an opaque object/record pointer
    /// before projecting members. A small first member cannot weaken the
    /// containing record's guarantee merely because it shares its address.
    pub(super) fn member_base(
        &mut self,
        address: Value,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        let object = match self.unit.sema.types.kind(ty) {
            | crate::translation_phases::semantic_analysis::TypeKind::Pointer(pointee) => pointee,
            | _ => ty,
        };
        let default = storage_alignment(&self.unit.sema.types, object)
            .map_err(|kind| super::module_items::at(kind, source))?;
        self.retain_alignment(address, self.place_alignment(address, default));
        Ok(())
    }

    /// All memory emission crosses this boundary, including byte operations
    /// and bit-field pieces. A copy promises only what both places guarantee.
    pub(super) fn memory_op(&self, mut data: InstData) -> Op<'s> {
        self.align_memory(&mut data);
        Op::Inst(data)
    }

    pub(super) fn align_memory(&self, data: &mut InstData) {
        match data {
            | InstData::Load { addr, align, .. } => *align = self.place_alignment(*addr, *align),
            | InstData::Store { args, align, .. } => *align = self.place_alignment(args[1], *align),
            | InstData::MemoryRange {
                opcode,
                args,
                align,
                ..
            } => {
                let destination = self.place_alignment(args[0], *align);
                *align = if *opcode == Opcode::Copy {
                    weaker(destination, self.place_alignment(args[1], *align))
                } else {
                    destination
                };
            },
            | _ => {},
        }
    }

    /// Fixed storage has one allocation owner; its retained alignment becomes
    /// the guarantee of every address produced for that slot.
    pub(super) fn object_slot(&mut self, size: u32, align: Align) -> StackSlot {
        self.draft.create_slot(size, align)
    }

    /// Lowering owns its stack allocation guarantee. VLA declarations supply
    /// their retained requirement; GNU alloca uses the x86-64 stack boundary.
    /// GCC: <https://gcc.gnu.org/onlinedocs/gcc/Stack-Allocation.html>.
    pub(super) fn dynamic_object(&mut self, size: Value, required: Option<Align>) -> Value {
        let align = required.unwrap_or_else(|| Align::from_bytes(16).unwrap_or(Align::BYTE));
        self.inst(
            InstData::DynAlloca {
                ty: Type::I64,
                size,
                align,
            },
            Type::Ptr,
        )
    }

    /// The address of a fresh stack slot of `size` bytes.
    pub(super) fn fresh_object(
        &mut self,
        size: u64,
        align: Align,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let size = u32::try_from(size).map_err(|_| {
            LoweringError::missing("an automatic object is larger than 4 GiB", source)
        })?;
        let slot = self.object_slot(size, align);
        Ok(self.inst(InstData::StackAddr { slot }, Type::Ptr))
    }

    /// Indexing refines the backing guarantee with the element stride. An
    /// opaque C pointer supplies its retained pointee alignment as the default.
    /// C99: 6.5.6p8, p. 83; PDF p. 95; 6.3.2.3p7, pp. 47-48; PDF pp. 59-60.
    pub(super) fn indexed_address(
        &mut self,
        base: Value,
        offset: Value,
        pointer: TypeId,
        stride: Option<u64>,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let element = super::types::target_type(&self.unit.sema.types, pointer)
            .ok_or_else(|| LoweringError::missing("an index has no pointee type", source))?;
        let default = storage_alignment(&self.unit.sema.types, element)
            .map_err(|kind| super::module_items::at(kind, source))?;
        let base_align = self.place_alignment(base, default);
        let align = if let Some(offset) = self.draft.constant(offset) {
            at_offset(base_align, offset.unsigned_abs())
        } else if let Some(stride) = stride {
            at_offset(base_align, u128::from(stride))
        } else {
            self.stride_alignment(base_align, element)
        };
        let address = self.ptr_add(base, offset);
        self.retain_alignment(address, align);
        Ok(address)
    }

    /// A runtime array stride is a multiple of its retained constant leaf
    /// size. A typedef's storage alignment need not divide that size.
    /// C99: §6.5.6p8, p. 83; PDF p. 95; GNU aligned typedef extension.
    fn stride_alignment(&self, base: Align, mut element: TypeId) -> Align {
        loop {
            if let Some(layout) = self.unit.sema.types.layout(element) {
                return at_offset(base, u128::from(layout.size));
            }
            match self.unit.sema.types.kind(element) {
                | crate::translation_phases::semantic_analysis::TypeKind::Array(next, _) =>
                    element = next,
                | _ => return Align::BYTE,
            }
        }
    }

    /// Facts owned by allocation instructions and retained ABI descriptors.
    pub(super) fn instruction_alignment(&self, data: InstData) -> Option<Align> {
        match data {
            | InstData::StackAddr { slot } => Some(self.draft.slots[slot.index()].align),
            | InstData::GlobalAddr { global } => Some(self.unit.module.global(global).align),
            | InstData::DynAlloca { align, .. } => Some(align),
            | InstData::VaArgAggregate { sig, .. } => self
                .unit
                .module
                .signature(sig)
                .param_abi(0)
                .and_then(crate::ir::ParamAbi::layout)
                .map(|layout| layout.align),
            | InstData::Binary {
                opcode: Opcode::PtrAdd,
                args,
                ..
            } => self.alignments.get(&args[0]).copied().map(|base| {
                self.draft
                    .constant(args[1])
                    .map_or(Align::BYTE, |offset| at_offset(base, offset.unsigned_abs()))
            }),
            | _ => None,
        }
    }
}

/// The language producer includes variable arrays and declaration overrides.
/// C11: §6.7.5p6, p. 127; PDF p. 145.
pub(super) fn storage_alignment(types: &Types<'_>, ty: TypeId) -> Result<Align, LoweringErrorKind> {
    types
        .storage_alignment(ty)
        .and_then(Align::from_bytes)
        .ok_or(LoweringErrorKind::MissingFact(
            "an object has no storage alignment",
        ))
}

/// A subobject at a byte offset has the common divisor of the base guarantee
/// and its offset. Supported alignments are powers of two.
pub(super) fn at_offset(base: Align, offset: u128) -> Align {
    if offset == 0 {
        base
    } else {
        Align::from_bytes(1 << u32::from(base.log2()).min(offset.trailing_zeros()))
            .unwrap_or(Align::BYTE)
    }
}

pub(super) fn weaker(a: Align, b: Align) -> Align {
    if a.bytes() < b.bytes() { a } else { b }
}
