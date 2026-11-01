//! GCC's folding of a floating constant converted to an integer type out of
//! the type's range. The conversion is undefined (C99 §6.3.1.4 paragraph 1,
//! p. 43; PDF p. 55); GCC's folder (`fold_convert_const_int_from_real`)
//! gives 0 for a NaN and otherwise the nearest bound of the type, in code
//! as in initializers, and its `convert_and_check` saturates to a
//! bit-field's own width. Clang converts at run time, so this applies under
//! GCC's `float_conversion_rules` only. Semantic analysis folds the explicit
//! casts; this folds every other conversion of a floating constant, by the
//! constant's bits: assignment, initialization, argument, return and the
//! usual arithmetic conversions of its operand.

use super::FunctionLowerer;
use crate::ir::{
    Type,
    Value,
};

impl FunctionLowerer<'_, '_, '_, '_, '_> {
    /// The bits of `value` when it is a floating constant whose conversion
    /// to an integer GCC folds, under GCC's rules.
    pub(super) fn gcc_floating_constant(&self, value: Value) -> Option<u128> {
        (self.unit.context.configuration.float_conversion_rules()
            == crate::configuration::RuleSet::Gcc)
            .then(|| self.draft.float_constant(value))
            .flatten()
    }

    /// The integer of type `ty`, `width` bits wide and of signedness
    /// `signed`, that GCC folds the floating constant with bits `bits` of
    /// type `from` to when it is out of the integer's range; `None` in
    /// range, where the conversion is exact.
    pub(super) fn saturated_constant(
        &mut self,
        bits: u128,
        from: Type,
        ty: Type,
        width: u32,
        signed: bool,
    ) -> Option<Value> {
        let saturated = saturate(bits, from, width, signed)?;
        Some(self.iconst(ty, saturated))
    }

    /// Records that `result` is the conversion of the floating constant
    /// with bits `bits` of type `from`, for the store that takes the
    /// converted operand ([`Self::field_constant`]). The record lasts until
    /// the next conversions are applied or the next store.
    pub(super) fn note_floating_conversion(&mut self, result: Value, bits: u128, from: Type) {
        self.floating_conversion = Some((result, bits, from));
    }

    /// The value of type `ty` a store of `value` into a bit-field `width`
    /// bits wide and of signedness `signed` takes when `value` is the
    /// conversion of a floating constant the store's own operand had, and
    /// the constant is out of the field's range: GCC saturates it to the
    /// field's width (`bs.b = 100.0` stores 15 in an `int b : 5`), where an
    /// integer, even one converted from a constant earlier, keeps its low
    /// bits.
    pub(super) fn field_constant(
        &mut self,
        value: Value,
        ty: Type,
        width: u32,
        signed: bool,
    ) -> Option<Value> {
        let (result, bits, from) = self.floating_conversion.take()?;
        if result != value {
            return None;
        }
        self.saturated_constant(bits, from, ty, width, signed)
    }
}

/// The bound of an integer `width` bits wide and of signedness `signed`
/// that GCC folds the floating value `encoding` of type `from` to,
/// as a two's-complement pattern: 0 for a NaN, else the nearest bound when
/// the value truncated toward zero is out of range; `None` in range.
fn saturate(encoding: u128, from: Type, width: u32, signed: bool) -> Option<i128> {
    // Exponent and fraction widths, and whether the integer bit is explicit
    // (the x87 format).
    let (exponent_bits, fraction_bits, explicit) = match from {
        | Type::F16 => (5, 10, false),
        | Type::BF16 => (8, 7, false),
        | Type::F32 => (8, 23, false),
        | Type::F64 => (11, 52, false),
        | Type::F80 => (15, 63, true),
        | Type::F128 => (15, 112, false),
        | _ => return None,
    };
    let significand_bits = fraction_bits + u32::from(explicit);
    let negative = (encoding >> (significand_bits + exponent_bits)) & 1 == 1;
    let maximum_exponent = (1_u128 << exponent_bits) - 1;
    let exponent = (encoding >> significand_bits) & maximum_exponent;
    let fraction = encoding & ((1_u128 << fraction_bits) - 1);
    let bound = |negative: bool| -> i128 {
        match (signed, negative) {
            | (true, true) => i128::MIN >> (128 - width),
            | (true, false) => u128::MAX
                .checked_shr(129 - width)
                .unwrap_or(0)
                .cast_signed(),
            | (false, true) => 0,
            | (false, false) => (u128::MAX >> (128 - width)).cast_signed(),
        }
    };
    if exponent == maximum_exponent {
        return Some(if fraction != 0 { 0 } else { bound(negative) });
    }
    if exponent == 0 {
        // Zero or subnormal: no integer part.
        return None;
    }
    let bias = maximum_exponent >> 1;
    let Some(scale) = exponent.checked_sub(bias) else {
        // Below 1 in magnitude: truncates to zero, in range.
        return None;
    };
    let significand = if explicit {
        encoding & ((1_u128 << significand_bits) - 1)
    } else {
        fraction | (1_u128 << fraction_bits)
    };
    let magnitude = if scale <= u128::from(fraction_bits) {
        Some(significand >> (u128::from(fraction_bits) - scale))
    } else {
        u32::try_from(scale - u128::from(fraction_bits))
            .ok()
            .filter(|&shift| shift < 128 && significand.leading_zeros() >= shift)
            .map(|shift| significand << shift)
    };
    let in_range = magnitude.is_some_and(|magnitude| match (signed, negative) {
        | (true, true) => magnitude <= 1_u128 << (width - 1),
        | (true, false) => magnitude < 1_u128 << (width - 1),
        | (false, true) => magnitude == 0,
        | (false, false) => width == 128 || magnitude < 1_u128 << width,
    });
    (!in_range).then(|| bound(negative))
}

#[cfg(test)]
mod tests {
    use super::saturate;
    use crate::ir::Type;

    #[test]
    fn out_of_range_constants_saturate_as_gcc_folds_them() {
        let f64 = |value: f64| u128::from(value.to_bits());
        let f32 = |value: f32| u128::from(value.to_bits());
        assert_eq!(
            saturate(f64(3e9), Type::F64, 32, true),
            Some(i128::from(i32::MAX))
        );
        assert_eq!(
            saturate(f64(-3e9), Type::F64, 32, true),
            Some(i128::from(i32::MIN))
        );
        assert_eq!(saturate(f64(2_147_483_647.0), Type::F64, 32, true), None);
        assert_eq!(saturate(f64(-2_147_483_648.0), Type::F64, 32, true), None);
        assert_eq!(
            saturate(f32(2_147_483_648.0), Type::F32, 32, true),
            Some(i128::from(i32::MAX))
        );
        assert_eq!(saturate(f64(300.0), Type::F64, 8, false), Some(255));
        assert_eq!(saturate(f64(255.9), Type::F64, 8, false), None);
        assert_eq!(saturate(f64(-1.0), Type::F64, 32, false), Some(0));
        assert_eq!(saturate(f64(-0.5), Type::F64, 32, false), None);
        assert_eq!(saturate(f64(1e10), Type::F64, 5, true), Some(15));
        assert_eq!(saturate(f64(-1e10), Type::F64, 5, true), Some(-16));
        assert_eq!(saturate(f64(1e10), Type::F64, 5, false), Some(31));
        assert_eq!(saturate(f64(2.0), Type::F64, 1, true), Some(0));
        assert_eq!(saturate(f64(-2.0), Type::F64, 1, true), Some(-1));
        assert_eq!(saturate(f64(f64::NAN), Type::F64, 32, true), Some(0));
        assert_eq!(
            saturate(f64(f64::INFINITY), Type::F64, 64, true),
            Some(i128::from(i64::MAX))
        );
        assert_eq!(
            saturate(f64(f64::NEG_INFINITY), Type::F64, 64, false),
            Some(0)
        );
        assert_eq!(saturate(f64(1e300), Type::F64, 128, false), Some(-1));
        assert_eq!(saturate(f64(1e30), Type::F64, 128, false), None);
        assert_eq!(saturate(f64(5e-324), Type::F64, 8, true), None);
        // x87: 2^64 out of 64 signed bits, 2^63 - 1 is no x87 constant
        // of its own, 2^62 fits.
        let x87 = |exponent: u128, significand: u128| exponent << 64 | significand;
        assert_eq!(
            saturate(x87(16383 + 64, 1 << 63), Type::F80, 64, true),
            Some(i128::from(i64::MAX))
        );
        assert_eq!(
            saturate(x87(16383 + 62, 1 << 63), Type::F80, 64, true),
            None
        );
        assert_eq!(
            saturate(0x7C00, Type::F16, 16, true),
            Some(i128::from(i16::MAX))
        );
        assert_eq!(
            saturate(0x7BFF, Type::F16, 16, true),
            Some(i128::from(i16::MAX))
        );
        assert_eq!(saturate(0x7BFF, Type::F16, 16, false), None);
    }
}
