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
    Atomic,
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
    /// A frame level that is not an integer constant.
    FrameLevel,
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
    Atomic(AtomicBuiltin),
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
    /// GNU frame (`false`) or return (`true`) address of a constant frame
    /// level. <https://gcc.gnu.org/onlinedocs/gcc/Return-Address.html>
    Address(bool),
    /// GNU: the return address as an address; the identity on x86-64.
    ExtractReturnAddress,
    Overflow(OverflowOperation, Option<Scalar>, bool),
    /// GNU: the complex value of a real and an imaginary part of one real
    /// floating type. <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
    Complex,
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
    /// The type-generic classification macros of C99 §7.12.3, pp. 216-217;
    /// PDF pp. 228-229, as GCC and Clang builtins.
    Isnan,
    Isfinite,
    Isnormal,
    /// GNU: -1, 0 or 1 for negative infinity, non-infinity or infinity.
    IsinfSign,
    /// GNU: five `int` results (NaN, infinite, normal, subnormal, zero),
    /// then the classified operand. C99: §7.12.3.1, p. 216; PDF p. 228.
    Fpclassify,
}

impl FloatOperation {
    /// The index of the floating operand among the call's arguments.
    pub(crate) const fn operand_index(self) -> usize {
        if matches!(self, Self::Fpclassify) {
            5
        } else {
            0
        }
    }
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

/// Neutral atomic builtin contracts; spelling is resolved once by the registry.
/// GCC atomic/sync builtins and Clang C11 atomic builtins are extensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AtomicBuiltin {
    pub(crate) family:     AtomicFamily,
    pub(crate) operation:  AtomicOperation,
    pub(crate) generic:    bool,
    pub(crate) count:      usize,
    pub(crate) rmw:        AtomicRmw,
    pub(crate) return_new: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AtomicFamily {
    C11,
    Gnu,
    Sync,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AtomicOperation {
    Init,
    ThreadFence,
    SignalFence,
    AlwaysLockFree,
    IsLockFree,
    Load,
    Store,
    Exchange,
    Compare,
    TestSet,
    Clear,
    FetchAdd,
    FetchSub,
    FetchBitwise,
    FetchMinMax,
    ValueCompare,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AtomicRmw {
    Exchange,
    Add,
    Sub,
    And,
    Or,
    Xor,
    Nand,
    Min,
    Max,
}
