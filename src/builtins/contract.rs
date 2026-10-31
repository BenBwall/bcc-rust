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

impl Availability {
    /// Whether the configuration accepts the builtin's feature, so that it
    /// exists at all: `__builtin_inff32` only where `_Float32` is a type.
    pub(crate) const fn accepted(
        self,
        configuration: crate::configuration::CompilerConfiguration,
    ) -> bool {
        match self.feature {
            | Some(feature) => configuration.accepts(feature),
            | None => true,
        }
    }
}

/// Every entry applies to all four current x86-64 targets. Reserved aliases
/// keep their contracts in freestanding mode; ordinary library lookup is
/// hosted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NamingPolicy {
    Reserved,
    GnuOrdinary,
    /// A Microsoft compiler intrinsic, which Clang provides only with
    /// Microsoft extensions (`__debugbreak`).
    Microsoft,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SyntaxForm {
    OrdinaryCall,
    VaArg,
    Offsetof,
    /// GNU `__builtin_types_compatible_p(type, type)`.
    TypesCompatible,
    /// GNU `__builtin_choose_expr(constant, expression, expression)`.
    ChooseExpr,
    /// Clang `__builtin_convertvector(expression, type)`.
    ConvertVector,
    /// Clang `__builtin_bit_cast(type, expression)`.
    BitCast,
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
    /// The target's `__builtin_va_list` as a parameter: adjusted to a
    /// pointer to its element where the list is an array (System V), the
    /// `char *` itself on Windows. C99: §7.15p3, p. 249; PDF p. 261;
    /// §6.7.5.3p7, p. 119; PDF p. 131.
    VaList,
    /// A pointer to the target's `__builtin_va_list` object, `char **` on
    /// Windows (Microsoft `__va_start`).
    VaListPointer,
    /// `FILE *`, naming the program's own `<stdio.h>` typedef.
    /// C99: §7.19.1p2, p. 262; PDF p. 274.
    File,
    /// A GNU vector of `count` lanes of `Scalar`, aligned to its size.
    /// GNU extension: GCC manual, "Vector Extensions".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Vector-Extensions.html>
    Vector(Scalar, u8),
    /// A pointer to such a vector, `true` for a `const` vector.
    VectorPointer(Scalar, u8, bool),
    /// No C type: the family's type rule computes each call's result, and
    /// an undeclared use binds an unprototyped function of unknown result.
    Opaque,
}

/// The signature of a family whose type rule types every call.
pub(crate) const OPAQUE_SIGNATURE: Signature = Signature {
    result:     SignatureType::Opaque,
    parameters: &[],
    prototype:  false,
    variadic:   false,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TypeRule {
    Fixed,
    Floating,
    Overflow,
    Varargs,
    Offsetof,
    Atomic,
    /// Every argument is a wide string literal (Microsoft `__annotation`).
    WideStrings,
    /// A generated x86 prototype whose immediates carry
    /// [`ConstantConstraint::Immediate`].
    X86Signature,
    /// Two vectors of one type, then constant lane indices.
    VectorShuffle,
    /// GCC's `__builtin_shuffle`: one or two vectors of one type, then an
    /// integer mask vector with as many lanes of the same size.
    GnuShuffle,
    /// One operand-overloaded elementwise or non-temporal operation.
    VectorOverload(VectorOverload),
    ConvertVector,
    BitCast,
    TypesCompatible,
    ChooseExpr,
    /// GNU type classification of an unevaluated operand.
    TypeClassify,
    Math128(Math128),
    /// A type-generic bit operation on one integer operand, whose type
    /// selects the width and, for some, the result type.
    GenericBit,
}

/// Clang's operand-overloaded vector builtins; the operand types select the
/// result. Clang extension: Clang Language Extensions, "Builtin Functions".
/// <https://clang.llvm.org/docs/LanguageExtensions.html#builtin-functions>
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VectorOverload {
    Abs,
    Sqrt,
    AddSat,
    SubSat,
    Min,
    Max,
    NondeterministicValue,
    NontemporalLoad,
    NontemporalStore,
}

impl VectorOverload {
    pub(crate) const fn arity(self) -> usize {
        match self {
            | Self::Abs | Self::Sqrt | Self::NondeterministicValue | Self::NontemporalLoad => 1,
            | Self::AddSat | Self::SubSat | Self::Min | Self::Max | Self::NontemporalStore => 2,
        }
    }
}

/// A binary128 math primitive: its result type, argument count, whether
/// each argument is a `const char *` NaN tag, and whether each is an
/// arithmetic operand converted to the result.
/// GNU extension: GCC manual, "Floating Types".
/// <https://gcc.gnu.org/onlinedocs/gcc/Floating-Types.html>
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Math128 {
    pub(crate) result:     Scalar,
    pub(crate) arguments:  usize,
    pub(crate) tag:        bool,
    pub(crate) arithmetic: bool,
    /// The tag names the payload of a quiet NaN (`__builtin_nanf128`)
    /// rather than of a signaling one.
    pub(crate) quiet:      bool,
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
    /// The descriptor signature's parameter at the operand's position, as
    /// a prototyped call converts it. C99: §6.5.2.2p7, p. 72; PDF p. 84.
    Prototype,
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
    /// `__builtin_alloca_with_align`'s alignment in bits: after conversion,
    /// a constant power of two from 8 to 2^31 (GCC) or 2^30 (Clang, which
    /// takes at most 2147483647).
    AllocaAlignment,
    ConvertedProbability,
    /// An x86 immediate: after prototype conversion, an integer constant in
    /// `[low, high]` that, with a nonzero mask, names a set bit of the mask.
    Immediate(i32, i32, u16),
    /// After conversion, an integer constant equal to the value, as GCC and
    /// Clang check `__builtin_longjmp`'s; a constant the conversion changes
    /// gets `-Wconstant-conversion`.
    ConvertedIntegerValue(u64),
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
    /// An x86 immediate operand outside its constraint.
    Immediate,
    /// A `__builtin_longjmp` value that is not the constant 1.
    LongjmpValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BuiltinEffectClass {
    Pure,
    Query,
    Write,
    Control,
    Allocate,
    Library,
    /// Not modeled: a family whose calls retain no checked instance, so
    /// their effects stay unknown.
    Unknown,
}

/// The checker and lowerer each match this closed vocabulary in their own
/// phase. No callbacks or backend opcodes are stored in descriptors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BuiltinOperation {
    Atomic(AtomicBuiltin),
    Varargs(VaOperation),
    Bit(BitOperation, Scalar),
    /// A type-generic bit operation at its operand's width.
    GenericBit(GenericBit),
    Float(FloatOperation, Option<Scalar>),
    Bswap(u32),
    /// Clang rotate builtins reduce the count modulo the unsigned operand
    /// width.
    Rotate(u32, bool),
    /// GCC infinity (`false`) and quiet NaN (`true`) constructors.
    FloatConstant(Scalar, bool),
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
    /// GNU and Clang `__builtin_addc` (`false`) and `__builtin_subc`
    /// (`true`) at the unsigned type: the sum or difference of the first
    /// two operands and the incoming carry, with the outgoing carry stored
    /// through the fourth. GNU extension: GCC manual, "Integer Overflow
    /// Builtins".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Integer-Overflow-Builtins.html>
    Carry(bool, Scalar),
    /// The BSD and GNU memory aliases `__builtin_bcopy`, `__builtin_bzero`
    /// and `__builtin_mempcpy`, which execute as a memory move, fill or
    /// copy, as Clang lowers them on every target, so no library needs the
    /// symbols. GNU extension: GCC manual, "Library Builtins".
    Memory(MemoryBuiltin),
    /// GNU `__builtin_alloca_with_align` and
    /// `__builtin_alloca_with_align_and_max`: `alloca` storage with the
    /// alignment the second operand gives in bits; a maximum size is only
    /// a hint. GNU extension: GCC manual, "Stack Allocation".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Stack-Allocation.html>
    AllocaAligned,
    /// GNU `__builtin___clear_cache`: makes the instruction cache coherent
    /// with written code in a range, which x86 needs nothing for.
    /// GNU extension: GCC manual, "Other Builtins".
    ClearCache,
    /// GCC `__builtin_assoc_barrier`: its operand's value, across which
    /// the optimizer may not reassociate floating arithmetic. GNU
    /// extension: GCC manual, "Other Builtins".
    AssocBarrier,
    /// GNU and Clang `__builtin_creal`, `__builtin_cimag` and
    /// `__builtin_conj` with their `f` and `l` forms: a part of the operand
    /// converted to the complex type, or its conjugate.
    /// C99: §7.3.9.2-§7.3.9.4, pp. 189-190; PDF pp. 201-202.
    ComplexPart(ComplexPart),
    /// GNU: the complex value of a real and an imaginary part of one real
    /// floating type. <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
    Complex,
    Library,
    Offsetof,
    /// x86 `__rdtsc`: the processor's time-stamp counter.
    /// <https://clang.llvm.org/docs/LanguageExtensions.html#builtin-readcyclecounter>
    ReadTimestamp,
    /// Microsoft `__debugbreak`: a breakpoint trap after which execution
    /// may continue. MSVC extension: Microsoft Learn, "__debugbreak".
    /// <https://learn.microsoft.com/en-us/cpp/intrinsics/debugbreak>
    DebugTrap,
    /// Microsoft `__annotation`: wide strings for the debug information,
    /// with no effect on execution. MSVC extension: Microsoft Learn,
    /// "__annotation". <https://learn.microsoft.com/en-us/cpp/intrinsics/annotation>
    Annotation,
    /// GNU `__builtin_setjmp`: saves the frame address, a resume address and
    /// the stack pointer in a five-word buffer, then returns 0, or 1 when a
    /// `__builtin_longjmp` resumes it. Unlike `setjmp` it saves no other
    /// register and no signal mask. GNU extension: GCC manual, "Nonlocal
    /// Gotos". <https://gcc.gnu.org/onlinedocs/gcc/Nonlocal-Gotos.html>
    Setjmp,
    /// GNU `__builtin_longjmp`: resumes the `__builtin_setjmp` that filled
    /// the buffer, whose second argument is the constant 1. Does not
    /// return. GNU extension: GCC manual, "Nonlocal Gotos".
    Longjmp,
    /// GNU `__builtin_va_arg_pack` (`false`) and `__builtin_va_arg_pack_len`
    /// (`true`): the anonymous arguments of the enclosing variadic function,
    /// or their number, at a call that inlines it. Lowering substitutes them
    /// in a specialized copy of that function for each call; anywhere else
    /// they are invalid. GNU extension: GCC manual, "Constructing Calls".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Constructing-Calls.html>
    ArgumentPack(bool),
    /// Checked and folded or selected by semantic analysis; lowering reads
    /// the retained constant or selected operand and otherwise refuses.
    Semantic(SemanticOperation),
    /// Semantic-only support: lowering refuses the call as an unsupported
    /// builtin, never as an external symbol.
    Unsupported(UnsupportedReason),
}

impl BuiltinOperation {
    /// Whether semantic analysis retains a checked `BuiltinCallFacts`
    /// instance for lowering to execute. Family operations keep their result
    /// in the expression record instead.
    pub(crate) const fn retains_instance(self) -> bool {
        !matches!(self, Self::Semantic(_) | Self::Unsupported(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SemanticOperation {
    ClassifyType,
    TypesCompatible,
    ChooseExpr,
    Math128,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnsupportedReason {
    /// x86 SIMD and system intrinsics have no lowering.
    X86,
    /// Vector shuffles, conversions, bit casts and overloads.
    Vector,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VaOperation {
    Start,
    /// Microsoft's `__va_start(&ap, last)`, which `<vadefs.h>` calls for
    /// `va_start` on x64: `va_start` on the list its first operand points
    /// to. MSVC extension: Clang implements it as `va_start` of
    /// `*(&ap)` (its `Builtins.td`, `__va_start`).
    StartAt,
    End,
    Copy,
    Arg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FloatOperation {
    Signbit,
    Isinf,
    Copysign,
    /// The magnitude: the operand with its sign bit clear, which keeps a
    /// NaN's payload. C99: §7.12.7.2, p. 228; PDF p. 240.
    Fabs,
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
    /// C23's `issignaling`: a NaN whose quiet bit is clear, as GCC's and
    /// Clang's builtin. C23: §7.12.3.8, pp. 240-241; PDF pp. 253-254 (N3220).
    Issignaling,
    /// The comparison macros of C99 §7.12.14, pp. 240-242; PDF pp. 252-254:
    /// two real operands, at least one floating, compared in their common
    /// real type without raising "invalid" for quiet NaNs.
    Compare(FloatCompare),
}

/// One comparison of C99 §7.12.14, pp. 240-242; PDF pp. 252-254, or GCC's
/// `__builtin_iseqsig` (C23 §7.12.17.7, pp. 281-282; PDF pp. 294-295
/// (N3220)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FloatCompare {
    Greater,
    GreaterEqual,
    Less,
    LessEqual,
    LessGreater,
    Unordered,
    /// Equality; C23 has it raise "invalid" for any NaN operand, which bcc
    /// does not model (the value is exact).
    EqualSignaling,
}

impl FloatCompare {
    /// Whether the comparison holds for operands that compare as `ordering`,
    /// or are unordered (`None`, a NaN operand).
    pub(crate) fn holds(self, ordering: Option<core::cmp::Ordering>) -> bool {
        let Some(ordering) = ordering else {
            return self == Self::Unordered;
        };
        match self {
            | Self::Greater => ordering.is_gt(),
            | Self::GreaterEqual => ordering.is_ge(),
            | Self::Less => ordering.is_lt(),
            | Self::LessEqual => ordering.is_le(),
            | Self::LessGreater => ordering.is_ne(),
            | Self::Unordered => false,
            | Self::EqualSignaling => ordering.is_eq(),
        }
    }
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

/// The type-generic bit operations: Clang 19's and GCC 14's
/// `__builtin_popcountg`, `__builtin_clzg` and `__builtin_ctzg` (the last
/// two with an optional result for zero), GCC 14's `__builtin_parityg`,
/// `__builtin_ffsg` and `__builtin_clrsbg`, and GCC 15's and Clang's
/// `__builtin_stdc_*` forms of C23's `<stdbit.h>` functions, each computed
/// at the width of its operand's type.
/// GNU extension: GCC manual, "Bit Operation Builtins",
/// <https://gcc.gnu.org/onlinedocs/gcc/Bit-Operation-Builtins.html>;
/// Clang Language Extensions, "`__builtin_popcountg`",
/// <https://clang.llvm.org/docs/LanguageExtensions.html#builtin-popcountg>.
/// C23: §7.18.3-§7.18.16, pp. 305-311; PDF pp. 318-324.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GenericBit {
    Popcount,
    Clz,
    Ctz,
    Parity,
    Ffs,
    Clrsb,
    LeadingZeros,
    LeadingOnes,
    TrailingZeros,
    TrailingOnes,
    FirstLeadingZero,
    FirstLeadingOne,
    FirstTrailingZero,
    FirstTrailingOne,
    CountZeros,
    CountOnes,
    HasSingleBit,
    BitWidth,
    BitFloor,
    BitCeil,
    RotateLeft,
    RotateRight,
}

impl GenericBit {
    /// Whether the operand must be signed (`__builtin_ffsg` and
    /// `__builtin_clrsbg`) rather than unsigned.
    pub(crate) const fn signed_operand(self) -> bool {
        matches!(self, Self::Ffs | Self::Clrsb)
    }

    /// Whether a second argument may give the result for a zero operand.
    pub(crate) const fn fallback(self) -> bool {
        matches!(self, Self::Clz | Self::Ctz)
    }

    /// Whether the second argument is a rotation count.
    pub(crate) const fn rotate(self) -> bool {
        matches!(self, Self::RotateLeft | Self::RotateRight)
    }

    /// Whether the operation is one of the `__builtin_stdc_*` forms, which
    /// GCC checks and reports apart from the older type-generic builtins.
    pub(crate) const fn stdc(self) -> bool {
        !matches!(
            self,
            Self::Popcount | Self::Clz | Self::Ctz | Self::Parity | Self::Ffs | Self::Clrsb
        )
    }

    /// The type of the result.
    pub(crate) const fn result(self) -> GenericBitResult {
        match self {
            | Self::Popcount | Self::Clz | Self::Ctz | Self::Parity | Self::Ffs | Self::Clrsb =>
                GenericBitResult::Int,
            | Self::HasSingleBit => GenericBitResult::Bool,
            | Self::BitFloor | Self::BitCeil | Self::RotateLeft | Self::RotateRight =>
                GenericBitResult::Operand,
            | _ => GenericBitResult::UnsignedInt,
        }
    }
}

/// The result type of a [`GenericBit`] operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GenericBitResult {
    Int,
    UnsignedInt,
    Bool,
    /// The operand's own type.
    Operand,
}

/// What a [`BuiltinOperation::Memory`] alias does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MemoryBuiltin {
    /// `bcopy (source, destination, n)`: `memmove` with the pointers
    /// swapped.
    Bcopy,
    /// `bzero (destination, n)`: `memset (destination, 0, n)`.
    Bzero,
    /// `mempcpy (destination, source, n)`: `memcpy`, returning the end of
    /// the copy.
    Mempcpy,
}

/// The part of a complex value `__builtin_creal`, `__builtin_cimag` and
/// `__builtin_conj` compute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ComplexPart {
    Real,
    Imaginary,
    Conjugate,
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
