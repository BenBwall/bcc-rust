//! GNU type-generic forms and binary128 math primitives. Type
//! classification, type compatibility and constant selection are checked
//! and folded, or selected, by `semantic_analysis/type_generic.rs`;
//! lowering reads the retained constant or selected operand. The binary128
//! spellings are the Clang primitives behind glibc's older-compiler
//! `_Float128` aliases; their constants fold to binary128 values. No
//! `__builtin_tgmath` support is invented.
//!
//! GNU extension: GCC manual, "Other Builtins" and "Floating Types".
//! <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
//! <https://gcc.gnu.org/onlinedocs/gcc/Floating-Types.html>
//! C99: §6.5.2.2, pp. 71-72; PDF pp. 83-84 (extended function calls).
use super::{
    Availability,
    BuiltinDescriptor,
    BuiltinEffectClass,
    BuiltinId as Id,
    BuiltinOperation,
    DesignatorPolicy,
    Math128,
    NamingPolicy,
    OPAQUE_SIGNATURE,
    SemanticOperation as O,
    SyntaxForm,
    TypeRule,
};
use crate::{
    configuration::Feature,
    target::Scalar as S,
};

pub(super) const BUILTINS: &[BuiltinDescriptor] = &[
    descriptor(
        Id::ClassifyType,
        "__builtin_classify_type",
        None,
        SyntaxForm::OrdinaryCall,
        TypeRule::TypeClassify,
        O::ClassifyType,
        1,
    ),
    descriptor(
        Id::TypesCompatible,
        "__builtin_types_compatible_p",
        Some(Feature::BuiltinTypesCompatible),
        SyntaxForm::TypesCompatible,
        TypeRule::TypesCompatible,
        O::TypesCompatible,
        2,
    ),
    descriptor(
        Id::ChooseExpr,
        "__builtin_choose_expr",
        Some(Feature::BuiltinChooseExpr),
        SyntaxForm::ChooseExpr,
        TypeRule::ChooseExpr,
        O::ChooseExpr,
        3,
    ),
    // GCC's signaling NaN of type double, folded like `__builtin_nan`.
    math128(Id::Nans, "__builtin_nans", S::Double, 1, true, false),
    // GCC's signaling NaNs of the interchange and extended types.
    interchange_math128(Id::Nansf32, "__builtin_nansf32", S::Float32),
    interchange_math128(Id::Nansf32x, "__builtin_nansf32x", S::Float32x),
    interchange_math128(Id::Nansf64, "__builtin_nansf64", S::Float64),
    interchange_math128(Id::Nansf64x, "__builtin_nansf64x", S::Float64x),
    math128(
        Id::HugeValf128,
        "__builtin_huge_valf128",
        S::Float128,
        0,
        false,
        false,
    ),
    math128(
        Id::Inff128,
        "__builtin_inff128",
        S::Float128,
        0,
        false,
        false,
    ),
    math128(
        Id::Nanf128,
        "__builtin_nanf128",
        S::Float128,
        1,
        true,
        false,
    ),
    math128(
        Id::Nansf128,
        "__builtin_nansf128",
        S::Float128,
        1,
        true,
        false,
    ),
    math128(
        Id::Fabsf128,
        "__builtin_fabsf128",
        S::Float128,
        1,
        false,
        true,
    ),
    math128(
        Id::Copysignf128,
        "__builtin_copysignf128",
        S::Float128,
        2,
        false,
        true,
    ),
];

/// GCC's `__builtin_nansfN`: a signaling NaN of an interchange or extended
/// type, which exists where the type's keyword does.
/// GNU extension: GCC manual, "Other Builtins".
/// <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
const fn interchange_math128(id: Id, spelling: &'static str, result: S) -> BuiltinDescriptor {
    let mut descriptor = math128(id, spelling, result, 1, true, false);
    descriptor.availability.feature = Some(Feature::InterchangeFloatTypes);
    descriptor
}

const fn math128(
    id: Id,
    spelling: &'static str,
    result: S,
    arguments: usize,
    tag: bool,
    arithmetic: bool,
) -> BuiltinDescriptor {
    descriptor(
        id,
        spelling,
        None,
        SyntaxForm::OrdinaryCall,
        TypeRule::Math128(Math128 {
            result,
            arguments,
            tag,
            arithmetic,
            quiet: matches!(id, Id::Nanf128),
        }),
        O::Math128,
        arguments,
    )
}

/// The family checker owns arity diagnostics; a binding keeps the opaque
/// signature these spellings had before the registry owned them.
const fn descriptor(
    id: Id,
    spelling: &'static str,
    feature: Option<Feature>,
    syntax: SyntaxForm,
    type_rule: TypeRule,
    operation: O,
    arity: usize,
) -> BuiltinDescriptor {
    BuiltinDescriptor {
        id,
        spelling,
        availability: Availability {
            feature,
            naming: NamingPolicy::Reserved,
        },
        syntax,
        designator: DesignatorPolicy::DirectCallOnly,
        signature: OPAQUE_SIGNATURE,
        type_rule,
        arguments: &[],
        arity: (arity, arity),
        effects: BuiltinEffectClass::Unknown,
        operation: BuiltinOperation::Semantic(operation),
        library: None,
    }
}
