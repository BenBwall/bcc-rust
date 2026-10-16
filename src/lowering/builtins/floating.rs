//! Floating classification and sign transfer in existing scalar/memory IR.
//! C99: §7.12.3.3, p. 217; PDF p. 229; §7.12.3.6, p. 218;
//! PDF p. 230; §7.12.11.1, p. 236; PDF p. 248. GNU aliases:
//! <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>.

use super::{
    FunctionLowerer,
    Item,
    LoweringError,
    Operand,
    types,
};
use crate::{
    ir::{
        Align,
        FloatCC,
        InstData,
        InstFlags,
        IntCC,
        Opcode,
        Type,
        Value,
    },
    translation_phases::{
        SourceVectors,
        semantic_analysis::{
            FloatOperation,
            TypeId,
        },
    },
};

impl FunctionLowerer<'_, '_, '_, '_, '_> {
    /// Inspect the object representation instead of comparing with zero:
    /// signed zero and both signs of NaN must preserve their sign bit.
    pub(super) fn floating_builtin(
        &mut self,
        operation: FloatOperation,
        items: &[Item],
        operand: TypeId,
        source: SourceVectors,
    ) -> Result<Operand, LoweringError> {
        let value = self.value(items[0], source)?;
        let ty = self.draft.value_type(value);
        let result = if operation == FloatOperation::Isinf {
            let infinity = crate::float_parsing::LongDouble::from_double(f64::INFINITY);
            let positive = super::super::floats::constant_bits(&infinity, ty).ok_or_else(|| {
                LoweringError::unsupported(super::super::Construct::Builtin, source)
            })?;
            let negative =
                super::super::floats::constant_bits(&infinity.negate(), ty).ok_or_else(|| {
                    LoweringError::unsupported(super::super::Construct::Builtin, source)
                })?;
            let positive = self.fconst(ty, positive);
            let negative = self.fconst(ty, negative);
            let yes = self.fcmp(FloatCC::Oeq, value, positive);
            let negative = self.fcmp(FloatCC::Oeq, value, negative);
            let classified = self.plain(Opcode::Or, yes, negative);
            self.extend(classified, false, Type::I32)
        } else {
            let (address, byte) = self.floating_sign_byte(value, operand, source)?;
            let mask = self.iconst(Type::I8, 0x80);
            if operation == FloatOperation::Signbit {
                let sign = self.plain(Opcode::And, byte, mask);
                let zero = self.iconst(Type::I8, 0);
                let yes = self.icmp(IntCC::Ne, sign, zero);
                self.extend(yes, false, Type::I32)
            } else {
                let second = self.value(items[1], source)?;
                let (_, sign) = self.floating_sign_byte(second, operand, source)?;
                let sign = self.plain(Opcode::And, sign, mask);
                let magnitude_mask = self.iconst(Type::I8, 0x7F);
                let magnitude = self.plain(Opcode::And, byte, magnitude_mask);
                let bits = self.plain(Opcode::Or, magnitude, sign);
                let sign_address = self.offset_address(address, u64::from(ty.bits() / 8 - 1));
                _ = self.draft.push(
                    self.memory_op(InstData::Store {
                        ty:    Type::I8,
                        flags: InstFlags::default(),
                        align: Align::BYTE,
                        tag:   None,
                        args:  [bits, sign_address],
                    }),
                    None,
                );
                self.inst(
                    InstData::Load {
                        ty,
                        flags: InstFlags::default(),
                        align: types::align(
                            types::layout(&self.unit.sema.types, operand)
                                .map_err(|kind| super::super::module_items::at(kind, source))?,
                        ),
                        tag: None,
                        addr: address,
                    },
                    ty,
                )
            }
        };
        Ok(Operand::Value(result))
    }

    /// F80 occupies ten value bytes in a sixteen-byte slot; the sign lives
    /// in byte nine, never in ABI padding. The other real formats likewise
    /// store the sign in their highest value byte on the supported targets.
    fn floating_sign_byte(
        &mut self,
        value: Value,
        operand: TypeId,
        source: SourceVectors,
    ) -> Result<(Value, Value), LoweringError> {
        let ty = self.draft.value_type(value);
        let layout = types::layout(&self.unit.sema.types, operand)
            .map_err(|kind| super::super::module_items::at(kind, source))?;
        let address = self.fresh_object(
            layout.size,
            super::super::alignment::storage_alignment(&self.unit.sema.types, operand)
                .map_err(|kind| super::super::module_items::at(kind, source))?,
            source,
        )?;
        self.store(value, address, operand, source)?;
        let sign = self.offset_address(address, u64::from(ty.bits() / 8 - 1));
        let byte = self.inst(
            InstData::Load {
                ty:    Type::I8,
                flags: InstFlags::default(),
                align: Align::BYTE,
                tag:   None,
                addr:  sign,
            },
            Type::I8,
        );
        Ok((address, byte))
    }
}
