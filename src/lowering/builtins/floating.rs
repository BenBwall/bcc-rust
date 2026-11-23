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
    pub(in crate::lowering) fn floating_builtin(
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
        let result = if let Some(mask) = operation.fixed_fpclass() {
            self.isfpclass(value, operand, mask, source)?
        } else if operation == FloatOperation::Issignaling {
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

    /// `__builtin_isfpclass(x, mask)`: whether `x` is in a class the
    /// constant `mask` names, read from its stored representation with
    /// integer tests, as Clang's `llvm.is.fpclass` expands
    /// (`TargetLowering::expandIS_FPCLASS`), so that no comparison raises
    /// "invalid" for a signaling NaN and denormal modes do not matter. The
    /// magnitude bits order the classes of an IEEE format: zero, subnormal
    /// below the smallest normal's encoding, normal below infinity's, then
    /// NaN, quiet from the quiet bit up.
    /// C99: §5.2.4.2.2 paragraphs 2-4, pp. 23-24; PDF pp. 35-36.
    /// Clang extension: Clang Language Extensions, "`__builtin_isfpclass`".
    pub(in crate::lowering) fn isfpclass(
        &mut self,
        value: Value,
        operand: TypeId,
        mask: u16,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        // Clang folds the empty and the full test, whatever the operand.
        if mask & FPCLASS_ALL == 0 || mask & FPCLASS_ALL == FPCLASS_ALL {
            return Ok(self.iconst(Type::I32, i128::from(mask != 0)));
        }
        let ty = self.draft.value_type(value);
        let (address, _) = self.floating_sign_byte(value, operand, source)?;
        if ty == Type::F80 {
            return Ok(self.x87_fpclass(address, mask));
        }
        let (bits_type, fraction_bits) = match ty {
            | Type::F16 => (Type::I16, 10),
            | Type::BF16 => (Type::I16, 7),
            | Type::F32 => (Type::I32, 23),
            | Type::F64 => (Type::I64, 52),
            | Type::F128 => (Type::I128, 112),
            | _ =>
                return Err(LoweringError::unsupported(
                    super::super::Construct::Builtin,
                    source,
                )),
        };
        let bits = self.inst(
            InstData::Load {
                ty:    bits_type,
                flags: InstFlags::default(),
                align: Align::BYTE,
                tag:   None,
                addr:  address,
            },
            bits_type,
        );
        let width = bits_type.bits();
        let sign_bit = 1_u128 << (width - 1);
        let smallest_normal = 1_u128 << fraction_bits;
        let infinity = (sign_bit - 1) & !(smallest_normal - 1);
        let quiet = infinity | 1 << (fraction_bits - 1);
        // A bit pattern of the format's width, which `iconst` truncates to it.
        let constant = |this: &mut Self, value: u128| {
            this.iconst(bits_type, i128::from_ne_bytes(value.to_ne_bytes()))
        };
        let magnitude_mask = constant(self, sign_bit - 1);
        let magnitude = self.plain(Opcode::And, bits, magnitude_mask);
        let zero_bits = constant(self, 0);
        let negative = self.icmp(IntCC::Slt, bits, zero_bits);
        let infinity_bits = constant(self, infinity);
        let one = constant(self, 1);
        let below = self.plain(Opcode::Isub, magnitude, one);
        let subnormal_limit = constant(self, smallest_normal - 1);
        let smallest = constant(self, smallest_normal);
        let above_smallest = self.plain(Opcode::Isub, magnitude, smallest);
        let normal_limit = constant(self, infinity - smallest_normal);
        let quiet_bits = constant(self, quiet);
        let classes = FpClasses {
            zero: self.icmp(IntCC::Eq, magnitude, zero_bits),
            subnormal: self.icmp(IntCC::Ult, below, subnormal_limit),
            normal: self.icmp(IntCC::Ult, above_smallest, normal_limit),
            infinite: self.icmp(IntCC::Eq, magnitude, infinity_bits),
            nan: self.icmp(IntCC::Ugt, magnitude, infinity_bits),
            quiet: self.icmp(IntCC::Uge, magnitude, quiet_bits),
            negative,
        };
        let found = self.fpclass_test(&classes, mask);
        Ok(self.extend(found, false, Type::I32))
    }

    /// `__builtin_isfpclass` of the x87 value stored at `address`: its
    /// 64-bit significand, with the explicit integer bit on top, and then
    /// its sign and 15-bit exponent. A canonical value is zero, subnormal
    /// (exponent 0, integer bit clear), normal (integer bit set), infinite
    /// or NaN (exponent all ones, integer bit set). The encodings the format
    /// leaves unsupported (a nonzero exponent with the integer bit clear:
    /// pseudo-NaN, pseudo-infinity, unnormal) and pseudo-denormals (exponent
    /// 0, integer bit set) are in no class alone; whether a test holds for
    /// them depends on the whole mask, as Clang tests them
    /// ([`x87_unsupported_in`], [`x87_pseudo_denormal_in`]).
    fn x87_fpclass(&mut self, address: Value, mask: u16) -> Value {
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
        let zero_exponent = self.iconst(Type::I16, 0);
        let negative = self.icmp(IntCC::Slt, sign_exponent, zero_exponent);
        let tiny = self.icmp(IntCC::Eq, exponent, zero_exponent);
        let special = self.icmp(IntCC::Eq, exponent, exponent_mask);
        let zero_significand = self.iconst(Type::I64, 0);
        // The integer bit is the significand's top bit.
        let integer = self.icmp(IntCC::Slt, significand, zero_significand);
        let empty = self.icmp(IntCC::Eq, significand, zero_significand);
        let one = self.iconst(Type::I64, 1);
        let below = self.plain(Opcode::Isub, significand, one);
        let fraction_limit = self.iconst(Type::I64, i64::MAX.into());
        let fraction_only = self.icmp(IntCC::Ult, below, fraction_limit);
        let one_exponent = self.iconst(Type::I16, 1);
        let lowered = self.plain(Opcode::Isub, exponent, one_exponent);
        let finite_limit = self.iconst(Type::I16, 0x7FFE);
        let ordinary = self.icmp(IntCC::Ult, lowered, finite_limit);
        let integer_only = self.iconst(Type::I64, i128::from(1_u64 << 63));
        let bare = self.icmp(IntCC::Eq, significand, integer_only);
        let payload = self.icmp(IntCC::Ugt, significand, integer_only);
        let quiet_bits = self.iconst(Type::I64, i128::from(0xC000_0000_0000_0000_u64));
        let quiet_significand = self.icmp(IntCC::Uge, significand, quiet_bits);
        let classes = FpClasses {
            zero: self.plain(Opcode::And, tiny, empty),
            subnormal: self.plain(Opcode::And, tiny, fraction_only),
            normal: self.plain(Opcode::And, ordinary, integer),
            infinite: self.plain(Opcode::And, special, bare),
            nan: self.plain(Opcode::And, special, payload),
            quiet: self.plain(Opcode::And, special, quiet_significand),
            negative,
        };
        let mut found = self.fpclass_test(&classes, mask);
        let true_value = self.iconst(Type::I1, 1);
        let fraction = self.plain(Opcode::Xor, integer, true_value);
        let wide = self.plain(Opcode::Xor, tiny, true_value);
        if x87_unsupported_in(mask) {
            let unsupported = self.plain(Opcode::And, wide, fraction);
            found = self.plain(Opcode::Or, found, unsupported);
        }
        if x87_pseudo_denormal_in(mask) {
            let pseudo = self.plain(Opcode::And, tiny, integer);
            found = self.plain(Opcode::Or, found, pseudo);
        }
        self.extend(found, false, Type::I32)
    }

    /// Whether a value of `classes` is in a class `mask` names. `nan` holds
    /// for every NaN and `quiet` for a quiet one, and `quiet` implies `nan`;
    /// each other class names both signs, which `negative` tells apart.
    fn fpclass_test(&mut self, classes: &FpClasses, mask: u16) -> Value {
        let true_value = self.iconst(Type::I1, 1);
        let positive = self.plain(Opcode::Xor, classes.negative, true_value);
        let mut found = self.iconst(Type::I1, 0);
        let signaling = self.plain(Opcode::Xor, classes.quiet, true_value);
        let signaling = self.plain(Opcode::And, classes.nan, signaling);
        let nan = match mask & FPCLASS_NAN {
            | FPCLASS_NAN => Some(classes.nan),
            | FPCLASS_SIGNALING => Some(signaling),
            | FPCLASS_QUIET => Some(classes.quiet),
            | _ => None,
        };
        if let Some(nan) = nan {
            found = self.plain(Opcode::Or, found, nan);
        }
        // Each sign's bit of zero, subnormal, normal and infinity: the
        // positive ones count up from bit 6, the negative ones down from 5.
        let signed = [
            classes.zero,
            classes.subnormal,
            classes.normal,
            classes.infinite,
        ];
        for (index, class) in signed.into_iter().enumerate() {
            let tested = (mask >> (6 + index) & 1 != 0, mask >> (5 - index) & 1 != 0);
            let holds = match tested {
                | (true, true) => class,
                | (true, false) => self.plain(Opcode::And, class, positive),
                | (false, true) => self.plain(Opcode::And, class, classes.negative),
                | (false, false) => continue,
            };
            found = self.plain(Opcode::Or, found, holds);
        }
        found
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
            | FloatOperation::Isfpclass
            | FloatOperation::Issubnormal
            | FloatOperation::Iszero
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

/// The class bits of `__builtin_isfpclass`, which are Clang's `__FPCLASS_*`.
const FPCLASS_SIGNALING: u16 = 0x001;
const FPCLASS_QUIET: u16 = 0x002;
const FPCLASS_NAN: u16 = FPCLASS_SIGNALING | FPCLASS_QUIET;
const FPCLASS_INFINITE: u16 = 0x204;
const FPCLASS_NORMAL: u16 = 0x108;
const FPCLASS_SUBNORMAL: u16 = 0x090;
const FPCLASS_ZERO: u16 = 0x060;
const FPCLASS_POSITIVE_NORMAL: u16 = 0x100;
const FPCLASS_ALL: u16 = 0x3FF;

/// The class tests of one value, as `i1` values.
struct FpClasses {
    zero:      Value,
    subnormal: Value,
    normal:    Value,
    infinite:  Value,
    /// Any NaN.
    nan:       Value,
    /// A quiet NaN.
    quiet:     Value,
    negative:  Value,
}

/// The test Clang's `llvm.is.fpclass` performs instead of `mask`, and
/// whether it inverts the result: the complement when that is one of the
/// tests it finds simpler (`invertFPClassTestIfSimpler`, LLVM 23.1.5).
fn simpler_fpclass_test(mask: u16) -> (u16, bool) {
    let complement = !mask & FPCLASS_ALL;
    let simpler = matches!(
        complement,
        FPCLASS_NAN
            | FPCLASS_SIGNALING
            | FPCLASS_QUIET
            | FPCLASS_INFINITE
            | 0x200
            | 0x004
            | FPCLASS_NORMAL
            | 0x100
            | 0x008
            | FPCLASS_SUBNORMAL
            | 0x080
            | 0x010
            | FPCLASS_ZERO
            | 0x040
            | 0x020
            | 0x1F8
            | 0x1C0
            | 0x038
            | 0x063
            | 0x0F0
            | 0x0F3
    );
    if simpler {
        (complement, true)
    } else {
        (mask, false)
    }
}

/// Whether Clang's `__builtin_isfpclass(x, mask)` holds for an x87 `x` the
/// format leaves unsupported. Its integer expansion counts such a value a
/// NaN only when it tests for both kinds of NaN, after inverting a test it
/// finds simpler, as glibc's `isnan` does
/// (`TargetLowering::expandIS_FPCLASS`, LLVM 23.1.5).
fn x87_unsupported_in(mask: u16) -> bool {
    let (test, inverted) = simpler_fpclass_test(mask);
    (test & FPCLASS_NAN == FPCLASS_NAN) != inverted
}

/// Whether Clang's `__builtin_isfpclass(x, mask)` holds for an x87
/// pseudo-denormal `x` at `-O0`. Its integer expansion counts one a NaN
/// when it tests for both kinds of NaN, and finds its exponent zero when it
/// tests for zero and subnormal both; but the tests it makes with an x87
/// comparison, which reads a pseudo-denormal as the normal number it
/// equals, see a normal number (`TargetLowering::expandIS_FPCLASS`, LLVM
/// 23.1.5). Clang's `-O2` gives other answers for a few masks.
fn x87_pseudo_denormal_in(mask: u16) -> bool {
    const COMPARED: [u16; 8] = [0x003, 0x007, 0x203, 0x063, 0x0F3, 0x3FC, 0x39C, 0x30C];
    if COMPARED.contains(&mask) {
        return mask & FPCLASS_POSITIVE_NORMAL != 0;
    }
    let (test, inverted) = simpler_fpclass_test(mask);
    let tiny = FPCLASS_ZERO | FPCLASS_SUBNORMAL;
    (test & FPCLASS_NAN == FPCLASS_NAN || test & tiny == tiny) != inverted
}
