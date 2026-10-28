//! Bit-precise integers at calls. A `_BitInt(N)` of up to 128 bits is an
//! IR integer of its storage's width, extended from N bits
//! ([`super::types::Repr::narrow`]). On the x86-64 targets one of 65 to 128
//! bits crosses a call as the structure of its two 64-bit limbs that the
//! calling conventions classify it as: System V passes it in two general
//! registers or on the stack 8-aligned, where an `__int128` would be
//! 16-aligned, and Microsoft x64 passes it by reference and returns it in
//! memory. So the pre-ABI signature describes it as an aggregate, and the
//! value is stored to a fresh object for a call and loaded from one at
//! function entry and after a call. The WebAssembly C ABI passes it as the
//! `i128` it is.
//! C23: §6.2.5 paragraphs 5-8, p. 39; PDF p. 52; §6.2.6.2 paragraphs 1-2,
//! pp. 42-43; PDF pp. 55-56.

use super::{
    FunctionLowerer,
    LoweringError,
    module_items::at,
    types,
};
use crate::{
    ir::{
        Align,
        InstData,
        InstFlags,
        Value,
    },
    translation_phases::{
        SourceVectors,
        semantic_analysis::TypeId,
    },
};

impl FunctionLowerer<'_, '_, '_, '_, '_> {
    /// The address of a fresh object holding the value `value` of the
    /// bit-precise type `ty`, which a call passes or a function returns in
    /// memory ([`types::limb_argument`]).
    pub(super) fn materialize_limbs(
        &mut self,
        value: Value,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let layout = types::layout(&self.unit.sema.types, ty).map_err(|kind| at(kind, source))?;
        let address = self.fresh_object(layout.size, types::align(layout), source)?;
        self.store_limbs(value, address, ty, source)?;
        Ok(address)
    }

    /// Stores the value `value` of the bit-precise type `ty` at `address`,
    /// which holds an object of that type.
    pub(super) fn store_limbs(
        &mut self,
        value: Value,
        address: Value,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        let align = self.limb_alignment(address, ty, source)?;
        _ = self.draft.push(
            self.memory_op(InstData::Store {
                ty: self.draft.value_type(value),
                flags: InstFlags::empty(),
                align,
                tag: None,
                args: [value, address],
            }),
            None,
        );
        Ok(())
    }

    /// The value of the bit-precise type `ty` in the object at `address`,
    /// extended from its width: the caller's copy of an argument or the
    /// object a call returned.
    pub(super) fn load_limbs(
        &mut self,
        address: Value,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let representation = self.repr(ty, source)?;
        let value_type = representation.value_type().ok_or_else(|| {
            LoweringError::missing("a bit-precise integer has no value type", source)
        })?;
        let align = self.limb_alignment(address, ty, source)?;
        let loaded = self.inst(
            InstData::Load {
                ty: value_type,
                flags: InstFlags::empty(),
                align,
                tag: None,
                addr: address,
            },
            value_type,
        );
        Ok(self.normalize(loaded, representation))
    }

    fn limb_alignment(
        &mut self,
        address: Value,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Align, LoweringError> {
        let layout = types::layout(&self.unit.sema.types, ty).map_err(|kind| at(kind, source))?;
        Ok(self.place_alignment(address, types::align(layout)))
    }
}
