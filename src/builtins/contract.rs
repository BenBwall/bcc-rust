//! Closed, phase-neutral signature, operand and executable-operation
//! vocabulary. C99: §6.5.2.2p2,p7, pp. 71-72; PDF pp. 83-84.

use crate::{
    configuration::Feature,
    target::Scalar,
};

/// One declarative language contract; diagnostics are catalog identities,
/// adapted to `SemanticErrorKind` by the semantic checker, never message
/// strings.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BuiltinDescriptor {
    pub(crate) id:           super::BuiltinId,
    pub(crate) spelling:     &'static str,
    pub(crate) availability: Availability,
    pub(crate) syntax:       SyntaxForm,
    pub(crate) designator:   DesignatorPolicy,
    pub(crate) signature:    Signature,
    pub(crate) type_rule:    TypeRule,
    pub(crate) arguments:    &'static [ArgumentRule],
    pub(crate) arity:        (usize, usize),
    pub(crate) effects:      BuiltinEffectClass,
    pub(crate) operation:    BuiltinOperation,
    pub(crate) library:      Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Availability {
    pub(crate) feature: Option<Feature>,
    pub(crate) naming:  NamingPolicy,
}

/// All M4 entries apply to all four current x86-64 targets. Reserved aliases
/// keep their contracts in freestanding mode; ordinary library lookup is
/// hosted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NamingPolicy {
    Reserved,
    GnuOrdinary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SyntaxForm {
    OrdinaryCall,
    VaArg,
    Offsetof,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DesignatorPolicy {
    DirectCallOnly,
    AddressableAlias,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Signature {
    pub(crate) result:     SignatureType,
    pub(crate) parameters: &'static [SignatureType],
    pub(crate) prototype:  bool,
    pub(crate) variadic:   bool,
}

/// Target data-model choices remain symbolic until semantic type projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SignatureType {
    Scalar(Scalar),
    Size,
    Unsigned(u8),
    Pointer(Scalar, bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TypeRule {
    Fixed,
    Floating,
    Overflow,
    Varargs,
    Offsetof,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ArgumentRule {
    pub(crate) conversion: ConversionRule,
    pub(crate) evaluation: ArgumentEvaluation,
    pub(crate) constraint: ConstantConstraint,
    pub(crate) diagnostic: ConstraintDiagnostic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConversionRule {
    Parameter(SignatureType),
    Value,
    Unconverted,
    Type,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArgumentEvaluation {
    Value,
    StorageAddress,
    AdjustedPointer,
    EffectsOnly,
    Unevaluated,
    Type,
}

/// `OriginalIntegerRange` deliberately checks the full ICE before fixed int
/// narrowing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConstantConstraint {
    None,
    ConvertedIntegerRange(u64),
    OriginalIntegerRange(u64),
    ConvertedPowerOfTwo,
    ConvertedProbability,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConstraintDiagnostic {
    ArgumentType,
    ObjectSizeMode,
    Alignment,
    Probability,
    PrefetchReadWrite,
    PrefetchLocality,
    VaList,
    VaArgType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BuiltinEffectClass {
    Pure,
    Query,
    Write,
    Control,
    Allocate,
    Library,
}

/// The checker and lowerer each match this closed vocabulary in their own
/// phase. No callbacks or backend opcodes are stored in descriptors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BuiltinOperation {
    Varargs(VaOperation),
    Bit(BitOperation, Scalar),
    Float(FloatOperation, Option<Scalar>),
    Bswap(u32),
    Expect(bool),
    Aligned,
    Unreachable,
    Trap,
    Prefetch,
    Constant,
    ObjectSize(bool),
    Alloca,
    Overflow(OverflowOperation, Option<Scalar>, bool),
    Library,
    Offsetof,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VaOperation {
    Start,
    End,
    Copy,
    Arg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FloatOperation {
    Signbit,
    Isinf,
    Copysign,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BitOperation {
    Clz,
    Ctz,
    Popcount,
    Parity,
    Ffs,
    Clrsb,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OverflowOperation {
    Add,
    Sub,
    Mul,
}

impl ArgumentRule {
    pub(crate) const fn value(conversion: ConversionRule) -> Self {
        Self {
            conversion,
            evaluation: ArgumentEvaluation::Value,
            constraint: ConstantConstraint::None,
            diagnostic: ConstraintDiagnostic::ArgumentType,
        }
    }

    pub(crate) const fn evaluation(evaluation: ArgumentEvaluation) -> Self {
        Self {
            evaluation,
            ..Self::value(ConversionRule::Unconverted)
        }
    }

    pub(crate) const fn constrained(
        conversion: ConversionRule,
        constraint: ConstantConstraint,
        diagnostic: ConstraintDiagnostic,
    ) -> Self {
        Self {
            constraint,
            diagnostic,
            ..Self::value(conversion)
        }
    }
}
