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
    builtins::{
        FloatCompare,
        FloatOperation,
    },
    configuration::CompilerIdentity,
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
        semantic_analysis::TypeId,
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
        let value = self.value(items[operation.operand_index()], source)?;
        let ty = self.draft.value_type(value);
        if let FloatOperation::Compare(compare) = operation {
            let other = self.value(items[1], source)?;
            return Ok(Operand::Value(
                self.floating_compare(compare, value, other, source)?,
            ));
        }
        let result = if operation == FloatOperation::Issignaling {
            self.issignaling(value, operand, source)?
        } else if matches!(
            operation,
            FloatOperation::Signbit | FloatOperation::Copysign | FloatOperation::Fabs
        ) {
            let (address, byte) = self.floating_sign_byte(value, operand, source)?;
            let mask = self.iconst(Type::I8, 0x80);
            if operation == FloatOperation::Signbit {
                let sign = self.plain(Opcode::And, byte, mask);
                let zero = self.iconst(Type::I8, 0);
                let yes = self.icmp(IntCC::Ne, sign, zero);
                self.extend(yes, false, Type::I32)
            } else {
                let magnitude_mask = self.iconst(Type::I8, 0x7F);
                let magnitude = self.plain(Opcode::And, byte, magnitude_mask);
                let bits = if operation == FloatOperation::Fabs {
                    magnitude
                } else {
                    let second = self.value(items[1], source)?;
                    let (_, sign) = self.floating_sign_byte(second, operand, source)?;
                    let sign = self.plain(Opcode::And, sign, mask);
                    self.plain(Opcode::Or, magnitude, sign)
                };
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
        } else {
            self.classify(
                operation,
                &items[..operation.operand_index()],
                value,
                source,
            )?
        };
        Ok(Operand::Value(result))
    }

    /// Both operands already have their common real type. The ordered
    /// predicates are false for a NaN operand, `uno` true, so no comparison
    /// needs a separate NaN test; a 16-bit type compares exactly in `float`.
    /// The macros raise no "invalid" for a quiet NaN (§7.12.14p1), but a
    /// binary128 ordered comparison becomes libgcc's `__lttf2` and its
    /// siblings, which do, so there a NaN operand is first replaced by zero
    /// and the result masked by the quiet `uno` test, as GCC compares it
    /// (Clang 23 raises "invalid" there).
    /// C99: §7.12.14.1-§7.12.14.6, pp. 240-242; PDF pp. 252-254.
    fn floating_compare(
        &mut self,
        compare: FloatCompare,
        left: Value,
        right: Value,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let (mut left, mut right) = if self.draft.value_type(left).half_format().is_some() {
            (self.widen_half(left), self.widen_half(right))
        } else {
            (left, right)
        };
        let ty = self.draft.value_type(left);
        let unordered = (ty == Type::F128 && compare != FloatCompare::Unordered)
            .then(|| self.fcmp(FloatCC::Uno, left, right));
        if let Some(unordered) = unordered {
            let zero = super::super::floats::constant_parts((false, 0, 0), ty)
                .map(|bits| self.fconst(ty, bits))
                .ok_or_else(|| LoweringError::missing("binary128 has no zero constant", source))?;
            left = self.choose(unordered, zero, left);
            right = self.choose(unordered, zero, right);
        }
        let cc = match compare {
            | FloatCompare::Greater => FloatCC::Ogt,
            | FloatCompare::GreaterEqual => FloatCC::Oge,
            | FloatCompare::Less => FloatCC::Olt,
            | FloatCompare::LessEqual => FloatCC::Ole,
            | FloatCompare::LessGreater => FloatCC::One,
            | FloatCompare::Unordered => FloatCC::Uno,
            | FloatCompare::EqualSignaling => FloatCC::Oeq,
        };
        let mut holds = self.fcmp(cc, left, right);
        if let Some(unordered) = unordered {
            let one = self.iconst(Type::I1, 1);
            let ordered = self.plain(Opcode::Xor, unordered, one);
            holds = self.plain(Opcode::And, holds, ordered);
        }
        Ok(self.extend(holds, false, Type::I32))
    }

    /// A NaN whose quiet bit, the highest fraction bit, is clear. The bit is
    /// read from the stored representation, which a register copy of a
    /// signaling NaN keeps; the byte and mask locate it in each format.
    /// C23: §7.12.3.8, pp. 240-241; PDF pp. 253-254 (N3220).
    fn issignaling(
        &mut self,
        value: Value,
        operand: TypeId,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let ty = self.draft.value_type(value);
        if ty == Type::F80 {
            return self.x87_signaling(value, operand, source);
        }
        let (offset, mask) = match ty {
            | Type::F16 => (1, 0x02),
            | Type::BF16 => (0, 0x40),
            | Type::F32 => (2, 0x40),
            | Type::F64 => (6, 0x08),
            | Type::F80 => (7, 0x40),
            | Type::F128 => (13, 0x80),
            | _ =>
                return Err(LoweringError::unsupported(
                    super::super::Construct::Builtin,
                    source,
                )),
        };
        let (address, _) = self.floating_sign_byte(value, operand, source)?;
        let address = self.offset_address(address, offset);
        let byte = self.inst(
            InstData::Load {
                ty:    Type::I8,
                flags: InstFlags::default(),
                align: Align::BYTE,
                tag:   None,
                addr:  address,
            },
            Type::I8,
        );
        let mask = self.iconst(Type::I8, mask);
        let quiet = self.plain(Opcode::And, byte, mask);
        let zero = self.iconst(Type::I8, 0);
        let signaling = self.icmp(IntCC::Eq, quiet, zero);
        let wide = if ty.half_format().is_some() {
            self.widen_half(value)
        } else {
            value
        };
        let nan = self.fcmp(FloatCC::Uno, wide, wide);
        let both = self.plain(Opcode::And, nan, signaling);
        Ok(self.extend(both, false, Type::I32))
    }

    /// An x87 value is signaling when it is a NaN with the quiet bit clear
    /// and the explicit integer bit set. Under every identity but Clang's,
    /// the encodings the format leaves unsupported, a nonzero exponent with
    /// the integer bit clear (pseudo-NaN, pseudo-infinity, unnormal), are
    /// signaling too, as GCC and glibc's `issignaling` classify them; Clang
    /// calls them not signaling. Only integer tests are used, since an x87
    /// comparison of an unsupported encoding raises "invalid".
    /// C23: §7.12.3.8, pp. 240-241; PDF pp. 253-254 (N3220).
    fn x87_signaling(
        &mut self,
        value: Value,
        operand: TypeId,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let (address, _) = self.floating_sign_byte(value, operand, source)?;
        let load = |this: &mut Self, ty: Type, offset: u64| {
            let addr = this.offset_address(address, offset);
            this.inst(
                InstData::Load {
                    ty,
                    flags: InstFlags::default(),
                    align: Align::BYTE,
                    tag: None,
                    addr,
                },
                ty,
            )
        };
        let significand = load(self, Type::I64, 0);
        let sign_exponent = load(self, Type::I16, 8);
        let exponent_mask = self.iconst(Type::I16, 0x7FFF);
        let exponent = self.plain(Opcode::And, sign_exponent, exponent_mask);
        let top_shift = self.iconst(Type::I64, 62);
        let top = self.plain(Opcode::Lshr, significand, top_shift);
        let integer_only = self.iconst(Type::I64, 0b10);
        let unquiet = self.icmp(IntCC::Eq, top, integer_only);
        let fraction_mask = self.iconst(Type::I64, (1 << 62) - 1);
        let fraction = self.plain(Opcode::And, significand, fraction_mask);
        let zero = self.iconst(Type::I64, 0);
        let payload = self.icmp(IntCC::Ne, fraction, zero);
        let all_ones = self.icmp(IntCC::Eq, exponent, exponent_mask);
        let nan = self.plain(Opcode::And, all_ones, unquiet);
        let mut signaling = self.plain(Opcode::And, nan, payload);
        if !matches!(
            self.unit.context.configuration.compiler_identity(),
            CompilerIdentity::Clang | CompilerIdentity::ClangCl
        ) {
            let integer_shift = self.iconst(Type::I64, 63);
            let integer_bit = self.plain(Opcode::Lshr, significand, integer_shift);
            let no_integer_bit = self.icmp(IntCC::Eq, integer_bit, zero);
            let zero_exponent = self.iconst(Type::I16, 0);
            let nonzero_exponent = self.icmp(IntCC::Ne, exponent, zero_exponent);
            let unsupported = self.plain(Opcode::And, nonzero_exponent, no_integer_bit);
            signaling = self.plain(Opcode::Or, signaling, unsupported);
        }
        Ok(self.extend(signaling, false, Type::I32))
    }

    /// Classifies with ordered comparisons against the format's infinities
    /// and smallest normal magnitude, which every real format represents
    /// exactly; a NaN compares unordered with everything.
    /// C99: §7.12.3.1-§7.12.3.5, pp. 216-217; PDF pp. 228-229;
    /// §5.2.4.2.2p2, p. 23; PDF p. 35 (normalized numbers).
    fn classify(
        &mut self,
        operation: FloatOperation,
        classes: &[Item],
        value: Value,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        // A `_Float16` classifies in `float`, which holds it exactly, against
        // binary16's smallest normal; a `__bf16` has `float`'s exponent range.
        let half = self.draft.value_type(value) == Type::F16;
        let value = if self.draft.value_type(value).half_format().is_some() {
            self.widen_half(value)
        } else {
            value
        };
        let ty = self.draft.value_type(value);
        // Exact x87 parts: biased exponent and explicit integer bit.
        let minimum_exponent = match ty {
            | Type::F32 if half => 16383 - 14,
            | Type::F32 => 16383 - 126,
            | Type::F64 => 16383 - 1022,
            | _ => 1,
        };
        let constant = |this: &mut Self, sign: bool, exponent: u16| {
            super::super::floats::constant_parts((sign, exponent, 1 << 63), ty)
                .map(|bits| this.fconst(ty, bits))
                .ok_or_else(|| LoweringError::unsupported(super::super::Construct::Builtin, source))
        };
        let positive = constant(self, false, 0x7FFF)?;
        let negative = constant(self, true, 0x7FFF)?;
        let is_positive = self.fcmp(FloatCC::Oeq, value, positive);
        let is_negative = self.fcmp(FloatCC::Oeq, value, negative);
        let infinite = self.plain(Opcode::Or, is_positive, is_negative);
        let nan = self.fcmp(FloatCC::Uno, value, value);
        let normal = |this: &mut Self| -> Result<Value, LoweringError> {
            let smallest = constant(this, false, minimum_exponent)?;
            let largest = constant(this, true, minimum_exponent)?;
            let above = this.fcmp(FloatCC::Oge, value, smallest);
            let finite = this.fcmp(FloatCC::Olt, value, positive);
            let upper = this.plain(Opcode::And, above, finite);
            let below = this.fcmp(FloatCC::Ole, value, largest);
            let finite = this.fcmp(FloatCC::Ogt, value, negative);
            let lower = this.plain(Opcode::And, below, finite);
            Ok(this.plain(Opcode::Or, upper, lower))
        };
        Ok(match operation {
            | FloatOperation::Isinf => self.extend(infinite, false, Type::I32),
            | FloatOperation::Isnan => self.extend(nan, false, Type::I32),
            | FloatOperation::Isfinite => {
                let not_positive = self.fcmp(FloatCC::One, value, positive);
                let not_negative = self.fcmp(FloatCC::One, value, negative);
                let finite = self.plain(Opcode::And, not_positive, not_negative);
                self.extend(finite, false, Type::I32)
            },
            | FloatOperation::Isnormal => {
                let normal = normal(self)?;
                self.extend(normal, false, Type::I32)
            },
            | FloatOperation::IsinfSign => {
                let one = self.iconst(Type::I32, 1);
                let minus_one = self.iconst(Type::I32, -1);
                let zero = self.iconst(Type::I32, 0);
                let negative = self.choose(is_negative, minus_one, zero);
                self.choose(is_positive, one, negative)
            },
            | FloatOperation::Fpclassify => {
                let [
                    nan_class,
                    infinite_class,
                    normal_class,
                    subnormal_class,
                    zero_class,
                ] = [0, 1, 2, 3, 4].map(|index| classes[index]);
                let nan_class = self.value(nan_class, source)?;
                let infinite_class = self.value(infinite_class, source)?;
                let normal_class = self.value(normal_class, source)?;
                let subnormal_class = self.value(subnormal_class, source)?;
                let zero_class = self.value(zero_class, source)?;
                let zero =
                    super::super::floats::constant_parts((false, 0, 0), ty).ok_or_else(|| {
                        LoweringError::unsupported(super::super::Construct::Builtin, source)
                    })?;
                let zero = self.fconst(ty, zero);
                let is_zero = self.fcmp(FloatCC::Oeq, value, zero);
                let normal = normal(self)?;
                let tiny = self.choose(is_zero, zero_class, subnormal_class);
                let finite = self.choose(normal, normal_class, tiny);
                let ordered = self.choose(infinite, infinite_class, finite);
                self.choose(nan, nan_class, ordered)
            },
            | FloatOperation::Signbit
            | FloatOperation::Copysign
            | FloatOperation::Fabs
            | FloatOperation::Issignaling
            | FloatOperation::Compare(_) =>
                return Err(LoweringError::missing("not a classification", source)),
        })
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
