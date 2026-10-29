//! The type-generic bit builtins: `__builtin_popcountg`, `__builtin_clzg`
//! and `__builtin_ctzg` (Clang 19, GCC 14), GCC 14's `__builtin_parityg`,
//! `__builtin_ffsg` and `__builtin_clrsbg`, and the `__builtin_stdc_*` forms
//! of C23's `<stdbit.h>` functions (GCC 14 and 15, Clang). Each computes on
//! its operand at the width of the operand's type, so a call is typed here
//! from its operand rather than from a prototype. The operand is only
//! lvalue converted: no integer promotion widens it.
//!
//! The operand types each accepts follow the identity's rules
//! ([`crate::configuration::CompilerConfiguration::bit_builtin_rules`]):
//! GCC takes an unsigned integer type that is neither `bool` nor an
//! enumeration (a signed one for `ffsg` and `clrsbg`) and counts a
//! bit-field at its own width; Clang takes an unsigned integer type at the
//! width of its declared type, and refuses `bool` and enumerations only for
//! the `__builtin_stdc_*` forms other than the rotations. A call whose
//! operands are integer constant expressions is one, except that Clang
//! does not fold `__builtin_clzg` or `__builtin_ctzg` of zero with no
//! result for zero (undefined), which GCC folds to the width.
//!
//! GNU extension: GCC manual, "Bit Operation Builtins".
//! <https://gcc.gnu.org/onlinedocs/gcc/Bit-Operation-Builtins.html>
//! Clang Language Extensions, "`__builtin_popcountg`".
//! <https://clang.llvm.org/docs/LanguageExtensions.html#builtin-popcountg>
//! C23: §7.18.3-§7.18.16, pp. 305-311; PDF pp. 318-324.

use crate::{
    builtins::{
        BuiltinDescriptor,
        GenericBit,
        GenericBitResult,
    },
    configuration::RuleSet,
    translation_phases::{
        parsing::syntax::{
            BinaryOperator,
            Expression,
            Identifier,
        },
        semantic_analysis::{
            Analyzer,
            ConstantClass,
            ConversionKind,
            ExpressionInfo,
            Integer,
            Scalar,
            SemanticErrorKind,
            TagKind,
            TypeId,
            TypeKind,
        },
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Checks and types a call of the type-generic bit builtin `operation`,
    /// folding it when its operands are integer constant expressions. The
    /// expression's operation type is the type whose width the operation
    /// computes at: the operand's, or under GCC's rules an unsigned or
    /// signed `_BitInt` of a narrower bit-field's width, and for Clang's
    /// `bool` operand `unsigned _BitInt(1)`.
    /// GNU extension: GCC manual, "Bit Operation Builtins".
    pub(in crate::translation_phases::semantic_analysis) fn generic_bit_builtin(
        &mut self,
        e: &'tu Expression<'tu>,
        name: Identifier,
        descriptor: &BuiltinDescriptor,
        operation: GenericBit,
        args: &[&'tu Expression<'tu>],
    ) -> ExpressionInfo<'tu> {
        if let Some(feature) = descriptor.availability.feature {
            self.context
                .report_extension(feature, descriptor.spelling, name.source_vectors);
        }
        let (min, max) = descriptor.arity;
        if !(min..=max).contains(&args.len()) {
            self.error(
                SemanticErrorKind::InvalidArgumentCount,
                e.source_vectors,
                Some(name.name),
                None,
            );
            return Self::expression_result(e, self.types.unknown());
        }
        let gcc = self.context.configuration.bit_builtin_rules() == RuleSet::Gcc;
        let operand = self.expression_info(args[0]);
        let ty = self.converted(operand);
        let checked = self.bit_operand(args[0], ty, operation, gcc, name);
        let mut valid = checked.is_some();
        let second = args.get(1).copied();
        if let Some(argument) = second {
            valid &= if operation.rotate() {
                self.rotation_count(argument, gcc, name)
            } else {
                self.zero_result(argument, gcc, name)
            };
        }
        let Some((width, signed)) = checked.filter(|_| valid) else {
            return Self::expression_result(e, self.types.unknown());
        };
        let ty = self.types.non_atomic(ty).unqualified();
        // GCC computes on a bit-field at its own width.
        let width = match operand.bit_field {
            | Some(field) if gcc && field < width => field,
            | _ => width,
        };
        let operation_type = if self.integer_type(ty) == Some((width, signed))
            && self.types.nodes[ty.index] != TypeKind::Scalar(Scalar::Bool)
        {
            ty
        } else {
            self.types.intern(TypeKind::BitInt { width, signed })
        };
        let result = match operation.result() {
            | GenericBitResult::Int => self.types.scalar(Scalar::Int),
            | GenericBitResult::UnsignedInt => self.types.scalar(Scalar::UnsignedInt),
            | GenericBitResult::Bool => self.types.scalar(Scalar::Bool),
            | GenericBitResult::Operand => ty,
        };
        let mut info = Self::expression_result(e, result);
        info.operation_type = Some(operation_type);
        let constant = |info: ExpressionInfo<'tu>| info.integer.filter(|_| info.ice);
        let value = constant(operand);
        let second = second.map(|argument| constant(self.expression_info(argument)));
        if let Some(value) = value
            && second.is_none_or(|second| second.is_some())
        {
            let value = self.wide.convert(value, width, signed);
            let folded = self.generic_bit_value(operation, value, second.flatten(), gcc);
            if let Some(folded) = folded {
                let (bits, signed) = self.integer_type(result).unwrap_or((32, true));
                info.integer = Some(self.wide.convert(folded, bits, signed));
                info.ice = true;
                info.constant = ConstantClass::Arithmetic;
            }
        }
        info
    }

    /// The width and signedness `operation` computes its operand of
    /// lvalue-converted type `ty` at, after reporting a type the rules
    /// refuse: none for an invalid or unanalyzed operand.
    fn bit_operand(
        &mut self,
        argument: &'tu Expression<'tu>,
        ty: TypeId,
        operation: GenericBit,
        gcc: bool,
        name: Identifier,
    ) -> Option<(u32, bool)> {
        if self.types.unanalyzed(ty) {
            return None;
        }
        let ty = self.types.non_atomic(ty);
        let Some((width, signed)) = self.integer_type(ty) else {
            return self.bit_operand_error(SemanticErrorKind::BitBuiltinNotInteger, argument, name);
        };
        let bool_or_enum = match self.types.nodes[ty.index] {
            | TypeKind::Scalar(Scalar::Bool) => true,
            | TypeKind::Tag(id) => self.types.tags[id].kind == TagKind::Enum,
            | _ => false,
        };
        let kind = if gcc {
            if bool_or_enum {
                Some(SemanticErrorKind::BitBuiltinBoolOrEnum)
            } else if signed != operation.signed_operand() {
                Some(if signed {
                    SemanticErrorKind::BitBuiltinSigned
                } else {
                    SemanticErrorKind::BitBuiltinUnsigned
                })
            } else {
                None
            }
        } else if signed {
            Some(SemanticErrorKind::BitBuiltinSigned)
        } else if bool_or_enum && operation.stdc() && !operation.rotate() {
            Some(SemanticErrorKind::BitBuiltinBoolOrEnum)
        } else {
            None
        };
        match kind {
            | Some(kind) => self.bit_operand_error(kind, argument, name),
            | None => Some((width, signed)),
        }
    }

    fn bit_operand_error(
        &mut self,
        kind: SemanticErrorKind,
        argument: &'tu Expression<'tu>,
        name: Identifier,
    ) -> Option<(u32, bool)> {
        self.error(kind, argument.source_vectors, Some(name.name), None);
        None
    }

    /// Checks the result for zero of `__builtin_clzg` or `__builtin_ctzg`,
    /// which must have type `int` after the integer promotions; under GCC's
    /// rules, as GCC passes it, a `_BitInt` narrower than `int`, or a
    /// signed one as wide, becomes an `int` too. Converts it to `int`.
    fn zero_result(&mut self, argument: &'tu Expression<'tu>, gcc: bool, name: Identifier) -> bool {
        let info = self.expression_info(argument);
        let ty = self.converted(info);
        if self.types.unanalyzed(ty) {
            return false;
        }
        let ty = self.types.non_atomic(ty);
        let int = self.types.scalar(Scalar::Int);
        let valid = match self.types.nodes[ty.index] {
            | TypeKind::BitInt { width, signed } => gcc && (width < 32 || (width == 32 && signed)),
            | _ if self.integer_type(ty).is_some() => self.promote(info, ty) == int,
            | _ => false,
        };
        if !valid {
            self.error(
                SemanticErrorKind::BitBuiltinFallback,
                argument.source_vectors,
                Some(name.name),
                None,
            );
            return false;
        }
        self.convert(argument, int, ConversionKind::DefaultArgument);
        true
    }

    /// Checks a rotation count: an integer of any type, but under GCC's
    /// rules no enumeration. It keeps its type; the rotation reduces it
    /// modulo the width.
    fn rotation_count(
        &mut self,
        argument: &'tu Expression<'tu>,
        gcc: bool,
        name: Identifier,
    ) -> bool {
        let info = self.expression_info(argument);
        let ty = self.converted(info);
        if self.types.unanalyzed(ty) {
            return false;
        }
        let ty = self.types.non_atomic(ty);
        let enumerated = matches!(
            self.types.nodes[ty.index],
            TypeKind::Tag(id) if self.types.tags[id].kind == TagKind::Enum
        );
        if self.integer_type(ty).is_none() || (gcc && enumerated) {
            self.error(
                SemanticErrorKind::BitBuiltinRotateCount,
                argument.source_vectors,
                Some(name.name),
                None,
            );
            return false;
        }
        true
    }

    /// The value of `operation` on the constant `value`, already reduced to
    /// the operation's width and signedness, with the constant result for
    /// zero or rotation count `second`, if any: an `int` count, a `bool`, or
    /// a value of the operand's width. None where the call has no constant
    /// value: `__builtin_clzg` or `__builtin_ctzg` of zero with no result
    /// for zero under Clang's rules.
    /// GNU extension: GCC manual, "Bit Operation Builtins".
    /// C23: §7.18.3-§7.18.16, pp. 305-311; PDF pp. 318-324.
    pub(in crate::translation_phases::semantic_analysis) fn generic_bit_value(
        &self,
        operation: GenericBit,
        value: Integer,
        second: Option<Integer>,
        gcc: bool,
    ) -> Option<Integer> {
        use GenericBit as G;
        let width = value.bits;
        let count = match (operation.rotate(), second) {
            | (true, Some(count)) => Some(self.rotation(count, width)?),
            | _ => None,
        };
        let ((counted, zero), bits) = self.wide.bit_operation(value, width, |limbs, out| {
            let counted = match operation {
                | G::BitFloor | G::BitCeil => {
                    let length = width - leading(limbs, width, false);
                    let single = ones(limbs) == 1;
                    let bit = match operation {
                        | G::BitFloor if length == 0 => None,
                        | G::BitFloor => Some(length - 1),
                        | _ if length == 0 || single => Some(length.saturating_sub(1)),
                        | _ if length == width => None,
                        | _ => Some(length),
                    };
                    if let Some(bit) = bit {
                        set(out, bit);
                    }
                    0
                },
                | G::RotateLeft | G::RotateRight => {
                    let count = count.unwrap_or(0);
                    let shift = if operation == G::RotateLeft {
                        count
                    } else {
                        (width - count) % width
                    };
                    for bit in 0..width {
                        if get(limbs, bit) {
                            set(out, (bit + shift) % width);
                        }
                    }
                    0
                },
                | _ => count_bits(operation, limbs, width),
            };
            (counted, limbs.iter().all(|&limb| limb == 0))
        });
        let count = |value: u32| Some(Integer::int(i128::from(value)));
        match operation {
            | G::BitFloor | G::BitCeil | G::RotateLeft | G::RotateRight => Some(bits),
            | G::Clz | G::Ctz if zero => match second {
                | Some(fallback) => Some(fallback),
                | None if gcc => count(width),
                | None => None,
            },
            | G::HasSingleBit => Some(Integer::int(i128::from(counted == 1)).cast(1, false)),
            | _ => count(counted),
        }
    }

    /// The rotation `count` reduced to `0..width`, a negative count turning
    /// the other way, as GCC and Clang reduce it.
    fn rotation(&self, count: Integer, width: u32) -> Option<u32> {
        let modulus = i128::from(width);
        let remainder = if count.wide() {
            let divisor = self
                .wide
                .convert(Integer::int(modulus), count.bits, count.signed);
            self.wide
                .binary(count, BinaryOperator::Modulo, divisor)?
                .to_i128()?
        } else if count.signed {
            count.value % modulus
        } else {
            i128::try_from(count.value.cast_unsigned() % modulus.cast_unsigned()).ok()?
        };
        u32::try_from(remainder.rem_euclid(modulus)).ok()
    }
}

/// The count `operation` gives for the `width`-bit value in `limbs`, whose
/// bits from `width` up are clear: the number of ones for a count of
/// ones, zeros or parity, the number of leading or trailing zeros or ones,
/// or a first index plus one (zero where there is none). A signed
/// operation reads the value's sign from bit `width - 1`.
fn count_bits(operation: GenericBit, limbs: &[u64], width: u32) -> u32 {
    use GenericBit as G;
    let index = |count: u32| if count == width { 0 } else { count + 1 };
    match operation {
        | G::Popcount | G::CountOnes | G::HasSingleBit => ones(limbs),
        | G::Parity => ones(limbs) % 2,
        | G::CountZeros => width - ones(limbs),
        | G::Clz | G::LeadingZeros => leading(limbs, width, false),
        | G::Ctz | G::TrailingZeros => trailing(limbs, width, false),
        | G::LeadingOnes => leading(limbs, width, true),
        | G::TrailingOnes => trailing(limbs, width, true),
        | G::FirstLeadingZero => index(leading(limbs, width, true)),
        | G::FirstLeadingOne => index(leading(limbs, width, false)),
        | G::FirstTrailingZero => index(trailing(limbs, width, true)),
        | G::Ffs | G::FirstTrailingOne => index(trailing(limbs, width, false)),
        | G::Clrsb => leading(limbs, width, get(limbs, width - 1)) - 1,
        | G::BitWidth => width - leading(limbs, width, false),
        | G::BitFloor | G::BitCeil | G::RotateLeft | G::RotateRight => 0,
    }
}

fn ones(limbs: &[u64]) -> u32 {
    limbs.iter().map(|limb| limb.count_ones()).sum()
}

/// The number of bits equal to `one`, from bit `width - 1` down.
fn leading(limbs: &[u64], width: u32, one: bool) -> u32 {
    let mut count = 0;
    for (index, &limb) in limbs.iter().enumerate().rev() {
        let size = (width - 64 * u32::try_from(index).unwrap_or(0)).min(64);
        let mask = u64::MAX >> (64 - size);
        let bits = if one { !limb & mask } else { limb & mask };
        if bits != 0 {
            return count + bits.leading_zeros() - (64 - size);
        }
        count += size;
    }
    count
}

/// The number of bits equal to `one`, from bit 0 up, at most `width`.
fn trailing(limbs: &[u64], width: u32, one: bool) -> u32 {
    let mut count = 0;
    for &limb in limbs {
        let bits = if one { !limb } else { limb };
        if bits != 0 {
            return (count + bits.trailing_zeros()).min(width);
        }
        count += 64;
    }
    width
}

fn get(limbs: &[u64], bit: u32) -> bool {
    limbs[(bit / 64) as usize] >> (bit % 64) & 1 == 1
}

fn set(limbs: &mut [u64], bit: u32) {
    limbs[(bit / 64) as usize] |= 1 << (bit % 64);
}
