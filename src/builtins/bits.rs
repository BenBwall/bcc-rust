//! The type-generic bit builtins: `__builtin_popcountg` and its siblings,
//! and the `__builtin_stdc_*` forms of C23's `<stdbit.h>` functions. Their
//! operand's type selects the width, so each call is typed and checked by
//! `semantic_analysis/builtins/bits.rs`, which follows GCC's or Clang's
//! rules for the operand types each accepts; lowering computes the result
//! at the operand's width.
//!
//! GNU extension: GCC manual, "Bit Operation Builtins".
//! <https://gcc.gnu.org/onlinedocs/gcc/Bit-Operation-Builtins.html>
//! Clang Language Extensions, "`__builtin_popcountg`".
//! <https://clang.llvm.org/docs/LanguageExtensions.html#builtin-popcountg>
//! C23: §7.18, pp. 304-311; PDF pp. 317-324.
use super::{
    Availability,
    BuiltinDescriptor,
    BuiltinEffectClass,
    BuiltinId as Id,
    BuiltinOperation,
    DesignatorPolicy,
    GenericBit as G,
    NamingPolicy,
    OPAQUE_SIGNATURE,
    SyntaxForm,
    TypeRule,
};
use crate::configuration::Feature;

pub(super) const BUILTINS: &[BuiltinDescriptor] = &[
    descriptor(Id::Popcountg, "__builtin_popcountg", G::Popcount),
    descriptor(Id::Clzg, "__builtin_clzg", G::Clz),
    descriptor(Id::Ctzg, "__builtin_ctzg", G::Ctz),
    descriptor(Id::Parityg, "__builtin_parityg", G::Parity),
    descriptor(Id::Ffsg, "__builtin_ffsg", G::Ffs),
    descriptor(Id::Clrsbg, "__builtin_clrsbg", G::Clrsb),
    descriptor(
        Id::StdcLeadingZeros,
        "__builtin_stdc_leading_zeros",
        G::LeadingZeros,
    ),
    descriptor(
        Id::StdcLeadingOnes,
        "__builtin_stdc_leading_ones",
        G::LeadingOnes,
    ),
    descriptor(
        Id::StdcTrailingZeros,
        "__builtin_stdc_trailing_zeros",
        G::TrailingZeros,
    ),
    descriptor(
        Id::StdcTrailingOnes,
        "__builtin_stdc_trailing_ones",
        G::TrailingOnes,
    ),
    descriptor(
        Id::StdcFirstLeadingZero,
        "__builtin_stdc_first_leading_zero",
        G::FirstLeadingZero,
    ),
    descriptor(
        Id::StdcFirstLeadingOne,
        "__builtin_stdc_first_leading_one",
        G::FirstLeadingOne,
    ),
    descriptor(
        Id::StdcFirstTrailingZero,
        "__builtin_stdc_first_trailing_zero",
        G::FirstTrailingZero,
    ),
    descriptor(
        Id::StdcFirstTrailingOne,
        "__builtin_stdc_first_trailing_one",
        G::FirstTrailingOne,
    ),
    descriptor(
        Id::StdcCountZeros,
        "__builtin_stdc_count_zeros",
        G::CountZeros,
    ),
    descriptor(Id::StdcCountOnes, "__builtin_stdc_count_ones", G::CountOnes),
    descriptor(
        Id::StdcHasSingleBit,
        "__builtin_stdc_has_single_bit",
        G::HasSingleBit,
    ),
    descriptor(Id::StdcBitWidth, "__builtin_stdc_bit_width", G::BitWidth),
    descriptor(Id::StdcBitFloor, "__builtin_stdc_bit_floor", G::BitFloor),
    descriptor(Id::StdcBitCeil, "__builtin_stdc_bit_ceil", G::BitCeil),
    descriptor(
        Id::StdcRotateLeft,
        "__builtin_stdc_rotate_left",
        G::RotateLeft,
    ),
    descriptor(
        Id::StdcRotateRight,
        "__builtin_stdc_rotate_right",
        G::RotateRight,
    ),
];

/// The checker owns operand and arity diagnostics; an undeclared use keeps
/// the opaque signature of a builtin whose type rule types each call. The
/// operations Clang lacks exist for the GCC identity only.
const fn descriptor(id: Id, spelling: &'static str, operation: G) -> BuiltinDescriptor {
    let arity = if operation.fallback() {
        (1, 2)
    } else if operation.rotate() {
        (2, 2)
    } else {
        (1, 1)
    };
    let feature = if matches!(operation, G::Parity | G::Ffs | G::Clrsb) {
        Feature::GccBitBuiltins
    } else {
        Feature::GnuBuiltins
    };
    BuiltinDescriptor {
        id,
        spelling,
        availability: Availability {
            feature: Some(feature),
            naming:  NamingPolicy::Reserved,
        },
        syntax: SyntaxForm::OrdinaryCall,
        designator: DesignatorPolicy::DirectCallOnly,
        signature: OPAQUE_SIGNATURE,
        type_rule: TypeRule::GenericBit,
        arguments: &[],
        arity,
        effects: BuiltinEffectClass::Pure,
        operation: BuiltinOperation::GenericBit(operation),
        library: None,
    }
}
