//! Clang vector builtins: the lane shuffle, the lane conversion and bit-cast
//! special forms, and the operand-overloaded elementwise and non-temporal
//! calls. None has a single C prototype, so each names a family
//! [`TypeRule`] that `semantic_analysis/vectors.rs` and
//! `semantic_analysis/builtins/x86_builtins.rs` check. Lowering has no
//! vector instructions yet; [`UnsupportedReason::Vector`] keeps the existing
//! refusal rather than an external call.
//!
//! Clang extension: Clang Language Extensions, "Vectors and Extended
//! Vectors" and "Builtin Functions".
//! <https://clang.llvm.org/docs/LanguageExtensions.html#vectors-and-extended-vectors>
//! <https://clang.llvm.org/docs/LanguageExtensions.html#builtin-functions>
//! C99: §6.5.2.2, pp. 71-72; PDF pp. 83-84 (extended function calls).
use super::{
    Availability,
    BuiltinDescriptor,
    BuiltinEffectClass,
    BuiltinId as Id,
    BuiltinOperation,
    DesignatorPolicy,
    NamingPolicy,
    OPAQUE_SIGNATURE,
    SyntaxForm,
    TypeRule,
    UnsupportedReason,
    VectorOverload as O,
};
use crate::configuration::Feature;

pub(super) const BUILTINS: &[BuiltinDescriptor] = &[
    descriptor(
        Id::Shufflevector,
        "__builtin_shufflevector",
        SyntaxForm::OrdinaryCall,
        TypeRule::VectorShuffle,
        (2, usize::MAX),
    ),
    descriptor(
        Id::Convertvector,
        "__builtin_convertvector",
        SyntaxForm::ConvertVector,
        TypeRule::ConvertVector,
        (2, 2),
    ),
    descriptor(
        Id::BitCast,
        "__builtin_bit_cast",
        SyntaxForm::BitCast,
        TypeRule::BitCast,
        (2, 2),
    ),
    overload(Id::ElementwiseAbs, "__builtin_elementwise_abs", O::Abs),
    overload(Id::ElementwiseSqrt, "__builtin_elementwise_sqrt", O::Sqrt),
    overload(
        Id::ElementwiseAddSat,
        "__builtin_elementwise_add_sat",
        O::AddSat,
    ),
    overload(
        Id::ElementwiseSubSat,
        "__builtin_elementwise_sub_sat",
        O::SubSat,
    ),
    overload(Id::ElementwiseMin, "__builtin_elementwise_min", O::Min),
    overload(Id::ElementwiseMax, "__builtin_elementwise_max", O::Max),
    overload(
        Id::NondeterministicValue,
        "__builtin_nondeterministic_value",
        O::NondeterministicValue,
    ),
    overload(
        Id::NontemporalLoad,
        "__builtin_nontemporal_load",
        O::NontemporalLoad,
    ),
    overload(
        Id::NontemporalStore,
        "__builtin_nontemporal_store",
        O::NontemporalStore,
    ),
];

const fn overload(id: Id, spelling: &'static str, operation: O) -> BuiltinDescriptor {
    let arity = operation.arity();
    descriptor(
        id,
        spelling,
        SyntaxForm::OrdinaryCall,
        TypeRule::VectorOverload(operation),
        (arity, arity),
    )
}

/// The checker reports [`Feature::VectorBuiltins`] itself, with the same
/// diagnostic text as before the registry owned these spellings.
const fn descriptor(
    id: Id,
    spelling: &'static str,
    syntax: SyntaxForm,
    type_rule: TypeRule,
    arity: (usize, usize),
) -> BuiltinDescriptor {
    BuiltinDescriptor {
        id,
        spelling,
        availability: Availability {
            feature: Some(Feature::VectorBuiltins),
            naming:  NamingPolicy::Reserved,
        },
        syntax,
        designator: DesignatorPolicy::DirectCallOnly,
        signature: OPAQUE_SIGNATURE,
        type_rule,
        arguments: &[],
        arity,
        effects: BuiltinEffectClass::Unknown,
        operation: BuiltinOperation::Unsupported(UnsupportedReason::Vector),
        library: None,
    }
}
