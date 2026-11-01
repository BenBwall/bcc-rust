//! Declarative GCC scalar and resource-header special-form contracts.
//! GNU extension: GCC Other/Bit Operation/Byte-Swapping/Integer Overflow
//! Builtins. <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
//! <https://gcc.gnu.org/onlinedocs/gcc/Integer-Overflow-Builtins.html>
//! C99: §6.5.2.2p2,p7, pp. 71-72; PDF pp. 83-84;
//! §7.15.1, pp. 249-251; PDF pp. 261-263 (varargs).
use super::{
    ArgumentEvaluation as E,
    ArgumentRule as A,
    Availability,
    BitOperation,
    BuiltinDescriptor,
    BuiltinEffectClass,
    BuiltinId as Id,
    BuiltinOperation as B,
    ComplexPart,
    ConstantConstraint as K,
    ConstraintDiagnostic as D,
    ConversionRule as C,
    DesignatorPolicy,
    FloatCompare,
    FloatOperation,
    NamingPolicy,
    OverflowOperation,
    Signature,
    SignatureType as T,
    SyntaxForm,
    TypeRule,
    VaOperation,
};
use crate::target::Scalar as S;

pub(super) const BUILTINS: &[BuiltinDescriptor] = &[
    descriptor(
        Id::Addc,
        "__builtin_addc",
        B::Carry(false, S::UnsignedInt),
        Signature {
            result:     T::Scalar(S::UnsignedInt),
            parameters: &[
                T::Scalar(S::UnsignedInt),
                T::Scalar(S::UnsignedInt),
                T::Scalar(S::UnsignedInt),
                T::Pointer(S::UnsignedInt, false),
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::UnsignedInt))),
            A::value(C::Parameter(T::Scalar(S::UnsignedInt))),
            A::value(C::Parameter(T::Scalar(S::UnsignedInt))),
            A::value(C::Parameter(T::Pointer(S::UnsignedInt, false))),
        ],
        4,
    ),
    descriptor(
        Id::Addcl,
        "__builtin_addcl",
        B::Carry(false, S::UnsignedLong),
        Signature {
            result:     T::Scalar(S::UnsignedLong),
            parameters: &[
                T::Scalar(S::UnsignedLong),
                T::Scalar(S::UnsignedLong),
                T::Scalar(S::UnsignedLong),
                T::Pointer(S::UnsignedLong, false),
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::UnsignedLong))),
            A::value(C::Parameter(T::Scalar(S::UnsignedLong))),
            A::value(C::Parameter(T::Scalar(S::UnsignedLong))),
            A::value(C::Parameter(T::Pointer(S::UnsignedLong, false))),
        ],
        4,
    ),
    descriptor(
        Id::Addcll,
        "__builtin_addcll",
        B::Carry(false, S::UnsignedLongLong),
        Signature {
            result:     T::Scalar(S::UnsignedLongLong),
            parameters: &[
                T::Scalar(S::UnsignedLongLong),
                T::Scalar(S::UnsignedLongLong),
                T::Scalar(S::UnsignedLongLong),
                T::Pointer(S::UnsignedLongLong, false),
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::UnsignedLongLong))),
            A::value(C::Parameter(T::Scalar(S::UnsignedLongLong))),
            A::value(C::Parameter(T::Scalar(S::UnsignedLongLong))),
            A::value(C::Parameter(T::Pointer(S::UnsignedLongLong, false))),
        ],
        4,
    ),
    descriptor(
        Id::Subc,
        "__builtin_subc",
        B::Carry(true, S::UnsignedInt),
        Signature {
            result:     T::Scalar(S::UnsignedInt),
            parameters: &[
                T::Scalar(S::UnsignedInt),
                T::Scalar(S::UnsignedInt),
                T::Scalar(S::UnsignedInt),
                T::Pointer(S::UnsignedInt, false),
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::UnsignedInt))),
            A::value(C::Parameter(T::Scalar(S::UnsignedInt))),
            A::value(C::Parameter(T::Scalar(S::UnsignedInt))),
            A::value(C::Parameter(T::Pointer(S::UnsignedInt, false))),
        ],
        4,
    ),
    descriptor(
        Id::Subcl,
        "__builtin_subcl",
        B::Carry(true, S::UnsignedLong),
        Signature {
            result:     T::Scalar(S::UnsignedLong),
            parameters: &[
                T::Scalar(S::UnsignedLong),
                T::Scalar(S::UnsignedLong),
                T::Scalar(S::UnsignedLong),
                T::Pointer(S::UnsignedLong, false),
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::UnsignedLong))),
            A::value(C::Parameter(T::Scalar(S::UnsignedLong))),
            A::value(C::Parameter(T::Scalar(S::UnsignedLong))),
            A::value(C::Parameter(T::Pointer(S::UnsignedLong, false))),
        ],
        4,
    ),
    descriptor(
        Id::Subcll,
        "__builtin_subcll",
        B::Carry(true, S::UnsignedLongLong),
        Signature {
            result:     T::Scalar(S::UnsignedLongLong),
            parameters: &[
                T::Scalar(S::UnsignedLongLong),
                T::Scalar(S::UnsignedLongLong),
                T::Scalar(S::UnsignedLongLong),
                T::Pointer(S::UnsignedLongLong, false),
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::UnsignedLongLong))),
            A::value(C::Parameter(T::Scalar(S::UnsignedLongLong))),
            A::value(C::Parameter(T::Scalar(S::UnsignedLongLong))),
            A::value(C::Parameter(T::Pointer(S::UnsignedLongLong, false))),
        ],
        4,
    ),
    descriptor(
        Id::Creal,
        "__builtin_creal",
        B::ComplexPart(ComplexPart::Real),
        Signature {
            result:     T::Scalar(S::Double),
            parameters: &[T::Scalar(S::ComplexDouble)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::ComplexDouble)))],
        1,
    ),
    descriptor(
        Id::Crealf,
        "__builtin_crealf",
        B::ComplexPart(ComplexPart::Real),
        Signature {
            result:     T::Scalar(S::Float),
            parameters: &[T::Scalar(S::ComplexFloat)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::ComplexFloat)))],
        1,
    ),
    descriptor(
        Id::Creall,
        "__builtin_creall",
        B::ComplexPart(ComplexPart::Real),
        Signature {
            result:     T::Scalar(S::LongDouble),
            parameters: &[T::Scalar(S::ComplexLongDouble)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::ComplexLongDouble)))],
        1,
    ),
    descriptor(
        Id::Cimag,
        "__builtin_cimag",
        B::ComplexPart(ComplexPart::Imaginary),
        Signature {
            result:     T::Scalar(S::Double),
            parameters: &[T::Scalar(S::ComplexDouble)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::ComplexDouble)))],
        1,
    ),
    descriptor(
        Id::Cimagf,
        "__builtin_cimagf",
        B::ComplexPart(ComplexPart::Imaginary),
        Signature {
            result:     T::Scalar(S::Float),
            parameters: &[T::Scalar(S::ComplexFloat)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::ComplexFloat)))],
        1,
    ),
    descriptor(
        Id::Cimagl,
        "__builtin_cimagl",
        B::ComplexPart(ComplexPart::Imaginary),
        Signature {
            result:     T::Scalar(S::LongDouble),
            parameters: &[T::Scalar(S::ComplexLongDouble)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::ComplexLongDouble)))],
        1,
    ),
    descriptor(
        Id::Conj,
        "__builtin_conj",
        B::ComplexPart(ComplexPart::Conjugate),
        Signature {
            result:     T::Scalar(S::ComplexDouble),
            parameters: &[T::Scalar(S::ComplexDouble)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::ComplexDouble)))],
        1,
    ),
    descriptor(
        Id::Conjf,
        "__builtin_conjf",
        B::ComplexPart(ComplexPart::Conjugate),
        Signature {
            result:     T::Scalar(S::ComplexFloat),
            parameters: &[T::Scalar(S::ComplexFloat)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::ComplexFloat)))],
        1,
    ),
    descriptor(
        Id::Conjl,
        "__builtin_conjl",
        B::ComplexPart(ComplexPart::Conjugate),
        Signature {
            result:     T::Scalar(S::ComplexLongDouble),
            parameters: &[T::Scalar(S::ComplexLongDouble)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::ComplexLongDouble)))],
        1,
    ),
    descriptor(
        Id::AllocaWithAlign,
        "__builtin_alloca_with_align",
        B::AllocaAligned,
        Signature {
            result:     T::Pointer(S::Void, false),
            parameters: &[T::Size, T::Size],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Size)),
            A::constrained(C::Parameter(T::Size), K::AllocaAlignment, D::Alignment),
        ],
        2,
    ),
    gcc_only(descriptor(
        Id::AllocaWithAlignAndMax,
        "__builtin_alloca_with_align_and_max",
        B::AllocaAligned,
        Signature {
            result:     T::Pointer(S::Void, false),
            parameters: &[T::Size, T::Size, T::Size],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Size)),
            A::constrained(C::Parameter(T::Size), K::AllocaAlignment, D::Alignment),
            A::constrained(
                C::Parameter(T::Size),
                K::ArithmeticConstant,
                D::AllocaMaximum,
            ),
        ],
        3,
    )),
    gcc_only(descriptor(
        Id::AssocBarrier,
        "__builtin_assoc_barrier",
        B::AssocBarrier,
        Signature {
            result:     T::Opaque,
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[A::value(C::Value)],
        1,
    )),
    descriptor(
        Id::ClearCache,
        "__builtin___clear_cache",
        B::ClearCache,
        Signature {
            result:     T::Scalar(S::Void),
            parameters: &[T::Pointer(S::Char, false), T::Pointer(S::Char, false)],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Pointer(S::Char, false))),
            A::value(C::Parameter(T::Pointer(S::Char, false))),
        ],
        2,
    ),
    descriptor(
        Id::AddOverflow,
        "__builtin_add_overflow",
        B::Overflow(OverflowOperation::Add, None, false),
        Signature {
            result:     T::Scalar(S::Bool),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[A::value(C::Value), A::value(C::Value), A::value(C::Value)],
        3,
    ),
    descriptor(
        Id::AddOverflowP,
        "__builtin_add_overflow_p",
        B::Overflow(OverflowOperation::Add, None, true),
        Signature {
            result:     T::Scalar(S::Bool),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[
            A::value(C::Value),
            A::value(C::Value),
            A::evaluation(E::EffectsOnly),
        ],
        3,
    ),
    descriptor(
        Id::Alloca,
        "__builtin_alloca",
        B::Alloca,
        Signature {
            result:     T::Pointer(S::Void, false),
            parameters: &[T::Size],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Size))],
        1,
    ),
    descriptor(
        Id::AssumeAligned,
        "__builtin_assume_aligned",
        B::Aligned,
        Signature {
            result:     T::Pointer(S::Void, false),
            parameters: &[T::Pointer(S::Void, true), T::Size],
            prototype:  true,
            variadic:   true,
        },
        &[
            A::value(C::Parameter(T::Pointer(S::Void, true))),
            A::constrained(C::Parameter(T::Size), K::ConvertedPowerOfTwo, D::Alignment),
            A::value(C::Parameter(T::Size)),
        ],
        2,
    ),
    descriptor(
        Id::Bswap16,
        "__builtin_bswap16",
        B::Bswap(16),
        Signature {
            result:     T::Unsigned(16),
            parameters: &[T::Unsigned(16)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Unsigned(16)))],
        1,
    ),
    descriptor(
        Id::Bswap32,
        "__builtin_bswap32",
        B::Bswap(32),
        Signature {
            result:     T::Unsigned(32),
            parameters: &[T::Unsigned(32)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Unsigned(32)))],
        1,
    ),
    descriptor(
        Id::Bswap64,
        "__builtin_bswap64",
        B::Bswap(64),
        Signature {
            result:     T::Unsigned(64),
            parameters: &[T::Unsigned(64)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Unsigned(64)))],
        1,
    ),
    gcc_only(descriptor(
        Id::Bswap128,
        "__builtin_bswap128",
        B::Bswap(128),
        Signature {
            result:     T::Scalar(S::UnsignedInt128),
            parameters: &[T::Scalar(S::UnsignedInt128)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::UnsignedInt128)))],
        1,
    )),
    descriptor(
        Id::Clrsb,
        "__builtin_clrsb",
        B::Bit(BitOperation::Clrsb, S::Int),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::Int)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Int)))],
        1,
    ),
    descriptor(
        Id::Clrsbl,
        "__builtin_clrsbl",
        B::Bit(BitOperation::Clrsb, S::Long),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::Long)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Long)))],
        1,
    ),
    descriptor(
        Id::Clrsbll,
        "__builtin_clrsbll",
        B::Bit(BitOperation::Clrsb, S::LongLong),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::LongLong)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::LongLong)))],
        1,
    ),
    descriptor(
        Id::Clz,
        "__builtin_clz",
        B::Bit(BitOperation::Clz, S::UnsignedInt),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::UnsignedInt)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::UnsignedInt)))],
        1,
    ),
    descriptor(
        Id::Clzl,
        "__builtin_clzl",
        B::Bit(BitOperation::Clz, S::UnsignedLong),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::UnsignedLong)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::UnsignedLong)))],
        1,
    ),
    descriptor(
        Id::Clzll,
        "__builtin_clzll",
        B::Bit(BitOperation::Clz, S::UnsignedLongLong),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::UnsignedLongLong)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::UnsignedLongLong)))],
        1,
    ),
    descriptor(
        Id::ConstantP,
        "__builtin_constant_p",
        B::Constant,
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[A::evaluation(E::Unevaluated)],
        1,
    ),
    descriptor(
        Id::Complex,
        "__builtin_complex",
        B::Complex,
        Signature {
            result:     T::Scalar(S::ComplexDouble),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[A::value(C::Value), A::value(C::Value)],
        2,
    ),
    descriptor(
        Id::Copysign,
        "__builtin_copysign",
        B::Float(FloatOperation::Copysign, Some(S::Double)),
        Signature {
            result:     T::Scalar(S::Double),
            parameters: &[T::Scalar(S::Double), T::Scalar(S::Double)],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::Double))),
            A::value(C::Parameter(T::Scalar(S::Double))),
        ],
        2,
    ),
    descriptor(
        Id::Copysignf,
        "__builtin_copysignf",
        B::Float(FloatOperation::Copysign, Some(S::Float)),
        Signature {
            result:     T::Scalar(S::Float),
            parameters: &[T::Scalar(S::Float), T::Scalar(S::Float)],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::Float))),
            A::value(C::Parameter(T::Scalar(S::Float))),
        ],
        2,
    ),
    float_descriptor(
        Id::Copysignf16,
        "__builtin_copysignf16",
        FloatOperation::Copysign,
        S::Float16,
        2,
        crate::configuration::Feature::Float16,
    ),
    float_descriptor(
        Id::Copysignf128,
        "__builtin_copysignf128",
        FloatOperation::Copysign,
        S::Float128,
        2,
        crate::configuration::Feature::GnuBuiltins,
    ),
    float_descriptor(
        Id::Copysignq,
        "__builtin_copysignq",
        FloatOperation::Copysign,
        S::Float128,
        2,
        crate::configuration::Feature::GccOnlyBuiltins,
    ),
    float_descriptor(
        Id::Fabsf16,
        "__builtin_fabsf16",
        FloatOperation::Fabs,
        S::Float16,
        1,
        crate::configuration::Feature::Float16,
    ),
    float_descriptor(
        Id::Fabsf128,
        "__builtin_fabsf128",
        FloatOperation::Fabs,
        S::Float128,
        1,
        crate::configuration::Feature::GnuBuiltins,
    ),
    float_descriptor(
        Id::Fabsq,
        "__builtin_fabsq",
        FloatOperation::Fabs,
        S::Float128,
        1,
        crate::configuration::Feature::GccOnlyBuiltins,
    ),
    descriptor(
        Id::Copysignl,
        "__builtin_copysignl",
        B::Float(FloatOperation::Copysign, Some(S::LongDouble)),
        Signature {
            result:     T::Scalar(S::LongDouble),
            parameters: &[T::Scalar(S::LongDouble), T::Scalar(S::LongDouble)],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::LongDouble))),
            A::value(C::Parameter(T::Scalar(S::LongDouble))),
        ],
        2,
    ),
    descriptor(
        Id::Ctz,
        "__builtin_ctz",
        B::Bit(BitOperation::Ctz, S::UnsignedInt),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::UnsignedInt)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::UnsignedInt)))],
        1,
    ),
    descriptor(
        Id::Ctzl,
        "__builtin_ctzl",
        B::Bit(BitOperation::Ctz, S::UnsignedLong),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::UnsignedLong)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::UnsignedLong)))],
        1,
    ),
    descriptor(
        Id::Ctzll,
        "__builtin_ctzll",
        B::Bit(BitOperation::Ctz, S::UnsignedLongLong),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::UnsignedLongLong)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::UnsignedLongLong)))],
        1,
    ),
    descriptor(
        Id::DynamicObjectSize,
        "__builtin_dynamic_object_size",
        B::ObjectSize(true),
        Signature {
            result:     T::Size,
            parameters: &[T::Pointer(S::Void, true), T::Scalar(S::Int)],
            prototype:  true,
            variadic:   false,
        },
        &[
            A {
                evaluation: E::Unevaluated,
                ..A::value(C::Parameter(T::Pointer(S::Void, true)))
            },
            A {
                evaluation: E::Unevaluated,
                ..A::constrained(
                    C::Parameter(T::Scalar(S::Int)),
                    K::ConvertedIntegerRange(3),
                    D::ObjectSizeMode,
                )
            },
        ],
        2,
    ),
    descriptor(
        Id::Expect,
        "__builtin_expect",
        B::Expect(false),
        Signature {
            result:     T::Scalar(S::Long),
            parameters: &[T::Scalar(S::Long), T::Scalar(S::Long)],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::Long))),
            A::value(C::Parameter(T::Scalar(S::Long))),
        ],
        2,
    ),
    descriptor(
        Id::ExpectWithProbability,
        "__builtin_expect_with_probability",
        B::Expect(true),
        Signature {
            result:     T::Scalar(S::Long),
            parameters: &[T::Scalar(S::Long), T::Scalar(S::Long), T::Scalar(S::Double)],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::Long))),
            A::value(C::Parameter(T::Scalar(S::Long))),
            A::constrained(
                C::Parameter(T::Scalar(S::Double)),
                K::ConvertedProbability,
                D::Probability,
            ),
        ],
        3,
    ),
    descriptor(
        Id::ExtractReturnAddr,
        "__builtin_extract_return_addr",
        B::ExtractReturnAddress,
        Signature {
            result:     T::Pointer(S::Void, false),
            parameters: &[T::Pointer(S::Void, false)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Pointer(S::Void, false)))],
        1,
    ),
    descriptor(
        Id::Ffs,
        "__builtin_ffs",
        B::Bit(BitOperation::Ffs, S::Int),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::Int)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Int)))],
        1,
    ),
    descriptor(
        Id::Ffsl,
        "__builtin_ffsl",
        B::Bit(BitOperation::Ffs, S::Long),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::Long)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Long)))],
        1,
    ),
    descriptor(
        Id::Ffsll,
        "__builtin_ffsll",
        B::Bit(BitOperation::Ffs, S::LongLong),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::LongLong)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::LongLong)))],
        1,
    ),
    descriptor(
        Id::Fpclassify,
        "__builtin_fpclassify",
        B::Float(FloatOperation::Fpclassify, None),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::Int))),
            A::value(C::Parameter(T::Scalar(S::Int))),
            A::value(C::Parameter(T::Scalar(S::Int))),
            A::value(C::Parameter(T::Scalar(S::Int))),
            A::value(C::Parameter(T::Scalar(S::Int))),
            A::value(C::Value),
        ],
        6,
    ),
    descriptor(
        Id::FrameAddress,
        "__builtin_frame_address",
        B::Address(false),
        Signature {
            result:     T::Pointer(S::Void, false),
            parameters: &[T::Scalar(S::UnsignedInt)],
            prototype:  true,
            variadic:   false,
        },
        &[A::constrained(
            C::Parameter(T::Scalar(S::UnsignedInt)),
            K::ConvertedIntegerRange(u32::MAX as u64),
            D::FrameLevel,
        )],
        1,
    ),
    descriptor(
        Id::Isfinite,
        "__builtin_isfinite",
        B::Float(FloatOperation::Isfinite, None),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[A::value(C::Value)],
        1,
    ),
    descriptor(
        Id::Isinf,
        "__builtin_isinf",
        B::Float(FloatOperation::Isinf, None),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[A::value(C::Value)],
        1,
    ),
    descriptor(
        Id::IsinfSign,
        "__builtin_isinf_sign",
        B::Float(FloatOperation::IsinfSign, None),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[A::value(C::Value)],
        1,
    ),
    descriptor(
        Id::Isinff,
        "__builtin_isinff",
        B::Float(FloatOperation::Isinf, Some(S::Float)),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::Float)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Float)))],
        1,
    ),
    descriptor(
        Id::Isinfl,
        "__builtin_isinfl",
        B::Float(FloatOperation::Isinf, Some(S::LongDouble)),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::LongDouble)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::LongDouble)))],
        1,
    ),
    descriptor(
        Id::Isnan,
        "__builtin_isnan",
        B::Float(FloatOperation::Isnan, None),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[A::value(C::Value)],
        1,
    ),
    descriptor(
        Id::Isnormal,
        "__builtin_isnormal",
        B::Float(FloatOperation::Isnormal, None),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[A::value(C::Value)],
        1,
    ),
    descriptor(
        Id::Iseqsig,
        "__builtin_iseqsig",
        B::Float(FloatOperation::Compare(FloatCompare::EqualSignaling), None),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[A::value(C::Value), A::value(C::Value)],
        2,
    ),
    descriptor(
        Id::Isgreater,
        "__builtin_isgreater",
        B::Float(FloatOperation::Compare(FloatCompare::Greater), None),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[A::value(C::Value), A::value(C::Value)],
        2,
    ),
    descriptor(
        Id::Isgreaterequal,
        "__builtin_isgreaterequal",
        B::Float(FloatOperation::Compare(FloatCompare::GreaterEqual), None),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[A::value(C::Value), A::value(C::Value)],
        2,
    ),
    descriptor(
        Id::Isless,
        "__builtin_isless",
        B::Float(FloatOperation::Compare(FloatCompare::Less), None),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[A::value(C::Value), A::value(C::Value)],
        2,
    ),
    descriptor(
        Id::Islessequal,
        "__builtin_islessequal",
        B::Float(FloatOperation::Compare(FloatCompare::LessEqual), None),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[A::value(C::Value), A::value(C::Value)],
        2,
    ),
    descriptor(
        Id::Islessgreater,
        "__builtin_islessgreater",
        B::Float(FloatOperation::Compare(FloatCompare::LessGreater), None),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[A::value(C::Value), A::value(C::Value)],
        2,
    ),
    descriptor(
        Id::Issignaling,
        "__builtin_issignaling",
        B::Float(FloatOperation::Issignaling, None),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[A::value(C::Value)],
        1,
    ),
    descriptor(
        Id::Isunordered,
        "__builtin_isunordered",
        B::Float(FloatOperation::Compare(FloatCompare::Unordered), None),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[A::value(C::Value), A::value(C::Value)],
        2,
    ),
    descriptor(
        Id::MulOverflow,
        "__builtin_mul_overflow",
        B::Overflow(OverflowOperation::Mul, None, false),
        Signature {
            result:     T::Scalar(S::Bool),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[A::value(C::Value), A::value(C::Value), A::value(C::Value)],
        3,
    ),
    descriptor(
        Id::MulOverflowP,
        "__builtin_mul_overflow_p",
        B::Overflow(OverflowOperation::Mul, None, true),
        Signature {
            result:     T::Scalar(S::Bool),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[
            A::value(C::Value),
            A::value(C::Value),
            A::evaluation(E::EffectsOnly),
        ],
        3,
    ),
    descriptor(
        Id::ObjectSize,
        "__builtin_object_size",
        B::ObjectSize(false),
        Signature {
            result:     T::Size,
            parameters: &[T::Pointer(S::Void, true), T::Scalar(S::Int)],
            prototype:  true,
            variadic:   false,
        },
        &[
            A {
                evaluation: E::Unevaluated,
                ..A::value(C::Parameter(T::Pointer(S::Void, true)))
            },
            A {
                evaluation: E::Unevaluated,
                ..A::constrained(
                    C::Parameter(T::Scalar(S::Int)),
                    K::ConvertedIntegerRange(3),
                    D::ObjectSizeMode,
                )
            },
        ],
        2,
    ),
    descriptor(
        Id::Offsetof,
        "__builtin_offsetof",
        B::Offsetof,
        Signature {
            result:     T::Size,
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[A {
            conversion: C::Type,
            ..A::evaluation(E::Type)
        }],
        1,
    ),
    descriptor(
        Id::Parity,
        "__builtin_parity",
        B::Bit(BitOperation::Parity, S::UnsignedInt),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::UnsignedInt)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::UnsignedInt)))],
        1,
    ),
    descriptor(
        Id::Parityl,
        "__builtin_parityl",
        B::Bit(BitOperation::Parity, S::UnsignedLong),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::UnsignedLong)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::UnsignedLong)))],
        1,
    ),
    descriptor(
        Id::Parityll,
        "__builtin_parityll",
        B::Bit(BitOperation::Parity, S::UnsignedLongLong),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::UnsignedLongLong)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::UnsignedLongLong)))],
        1,
    ),
    descriptor(
        Id::Popcount,
        "__builtin_popcount",
        B::Bit(BitOperation::Popcount, S::UnsignedInt),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::UnsignedInt)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::UnsignedInt)))],
        1,
    ),
    descriptor(
        Id::Popcountl,
        "__builtin_popcountl",
        B::Bit(BitOperation::Popcount, S::UnsignedLong),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::UnsignedLong)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::UnsignedLong)))],
        1,
    ),
    descriptor(
        Id::Popcountll,
        "__builtin_popcountll",
        B::Bit(BitOperation::Popcount, S::UnsignedLongLong),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::UnsignedLongLong)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::UnsignedLongLong)))],
        1,
    ),
    descriptor(
        Id::Prefetch,
        "__builtin_prefetch",
        B::Prefetch,
        Signature {
            result:     T::Scalar(S::Void),
            parameters: &[T::Pointer(S::Void, true)],
            prototype:  true,
            variadic:   true,
        },
        &[
            A::value(C::Parameter(T::Pointer(S::Void, true))),
            A::constrained(
                C::Parameter(T::Scalar(S::Int)),
                K::OriginalIntegerRange(1),
                D::PrefetchReadWrite,
            ),
            A::constrained(
                C::Parameter(T::Scalar(S::Int)),
                K::OriginalIntegerRange(3),
                D::PrefetchLocality,
            ),
        ],
        1,
    ),
    descriptor(
        Id::ReturnAddress,
        "__builtin_return_address",
        B::Address(true),
        Signature {
            result:     T::Pointer(S::Void, false),
            parameters: &[T::Scalar(S::UnsignedInt)],
            prototype:  true,
            variadic:   false,
        },
        &[A::constrained(
            C::Parameter(T::Scalar(S::UnsignedInt)),
            K::ConvertedIntegerRange(u32::MAX as u64),
            D::FrameLevel,
        )],
        1,
    ),
    descriptor(
        Id::SaddOverflow,
        "__builtin_sadd_overflow",
        B::Overflow(OverflowOperation::Add, Some(S::Int), false),
        Signature {
            result:     T::Scalar(S::Bool),
            parameters: &[
                T::Scalar(S::Int),
                T::Scalar(S::Int),
                T::Pointer(S::Int, false),
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::Int))),
            A::value(C::Parameter(T::Scalar(S::Int))),
            A::value(C::Parameter(T::Pointer(S::Int, false))),
        ],
        3,
    ),
    descriptor(
        Id::SaddlOverflow,
        "__builtin_saddl_overflow",
        B::Overflow(OverflowOperation::Add, Some(S::Long), false),
        Signature {
            result:     T::Scalar(S::Bool),
            parameters: &[
                T::Scalar(S::Long),
                T::Scalar(S::Long),
                T::Pointer(S::Long, false),
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::Long))),
            A::value(C::Parameter(T::Scalar(S::Long))),
            A::value(C::Parameter(T::Pointer(S::Long, false))),
        ],
        3,
    ),
    descriptor(
        Id::SaddllOverflow,
        "__builtin_saddll_overflow",
        B::Overflow(OverflowOperation::Add, Some(S::LongLong), false),
        Signature {
            result:     T::Scalar(S::Bool),
            parameters: &[
                T::Scalar(S::LongLong),
                T::Scalar(S::LongLong),
                T::Pointer(S::LongLong, false),
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::LongLong))),
            A::value(C::Parameter(T::Scalar(S::LongLong))),
            A::value(C::Parameter(T::Pointer(S::LongLong, false))),
        ],
        3,
    ),
    descriptor(
        Id::Signbit,
        "__builtin_signbit",
        B::Float(FloatOperation::Signbit, None),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[A::value(C::Value)],
        1,
    ),
    descriptor(
        Id::Signbitf,
        "__builtin_signbitf",
        B::Float(FloatOperation::Signbit, Some(S::Float)),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::Float)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Float)))],
        1,
    ),
    descriptor(
        Id::Signbitl,
        "__builtin_signbitl",
        B::Float(FloatOperation::Signbit, Some(S::LongDouble)),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::LongDouble)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::LongDouble)))],
        1,
    ),
    descriptor(
        Id::SmulOverflow,
        "__builtin_smul_overflow",
        B::Overflow(OverflowOperation::Mul, Some(S::Int), false),
        Signature {
            result:     T::Scalar(S::Bool),
            parameters: &[
                T::Scalar(S::Int),
                T::Scalar(S::Int),
                T::Pointer(S::Int, false),
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::Int))),
            A::value(C::Parameter(T::Scalar(S::Int))),
            A::value(C::Parameter(T::Pointer(S::Int, false))),
        ],
        3,
    ),
    descriptor(
        Id::SmullOverflow,
        "__builtin_smull_overflow",
        B::Overflow(OverflowOperation::Mul, Some(S::Long), false),
        Signature {
            result:     T::Scalar(S::Bool),
            parameters: &[
                T::Scalar(S::Long),
                T::Scalar(S::Long),
                T::Pointer(S::Long, false),
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::Long))),
            A::value(C::Parameter(T::Scalar(S::Long))),
            A::value(C::Parameter(T::Pointer(S::Long, false))),
        ],
        3,
    ),
    descriptor(
        Id::SmulllOverflow,
        "__builtin_smulll_overflow",
        B::Overflow(OverflowOperation::Mul, Some(S::LongLong), false),
        Signature {
            result:     T::Scalar(S::Bool),
            parameters: &[
                T::Scalar(S::LongLong),
                T::Scalar(S::LongLong),
                T::Pointer(S::LongLong, false),
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::LongLong))),
            A::value(C::Parameter(T::Scalar(S::LongLong))),
            A::value(C::Parameter(T::Pointer(S::LongLong, false))),
        ],
        3,
    ),
    descriptor(
        Id::SsubOverflow,
        "__builtin_ssub_overflow",
        B::Overflow(OverflowOperation::Sub, Some(S::Int), false),
        Signature {
            result:     T::Scalar(S::Bool),
            parameters: &[
                T::Scalar(S::Int),
                T::Scalar(S::Int),
                T::Pointer(S::Int, false),
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::Int))),
            A::value(C::Parameter(T::Scalar(S::Int))),
            A::value(C::Parameter(T::Pointer(S::Int, false))),
        ],
        3,
    ),
    descriptor(
        Id::SsublOverflow,
        "__builtin_ssubl_overflow",
        B::Overflow(OverflowOperation::Sub, Some(S::Long), false),
        Signature {
            result:     T::Scalar(S::Bool),
            parameters: &[
                T::Scalar(S::Long),
                T::Scalar(S::Long),
                T::Pointer(S::Long, false),
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::Long))),
            A::value(C::Parameter(T::Scalar(S::Long))),
            A::value(C::Parameter(T::Pointer(S::Long, false))),
        ],
        3,
    ),
    descriptor(
        Id::SsubllOverflow,
        "__builtin_ssubll_overflow",
        B::Overflow(OverflowOperation::Sub, Some(S::LongLong), false),
        Signature {
            result:     T::Scalar(S::Bool),
            parameters: &[
                T::Scalar(S::LongLong),
                T::Scalar(S::LongLong),
                T::Pointer(S::LongLong, false),
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::LongLong))),
            A::value(C::Parameter(T::Scalar(S::LongLong))),
            A::value(C::Parameter(T::Pointer(S::LongLong, false))),
        ],
        3,
    ),
    descriptor(
        Id::SubOverflow,
        "__builtin_sub_overflow",
        B::Overflow(OverflowOperation::Sub, None, false),
        Signature {
            result:     T::Scalar(S::Bool),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[A::value(C::Value), A::value(C::Value), A::value(C::Value)],
        3,
    ),
    descriptor(
        Id::SubOverflowP,
        "__builtin_sub_overflow_p",
        B::Overflow(OverflowOperation::Sub, None, true),
        Signature {
            result:     T::Scalar(S::Bool),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[
            A::value(C::Value),
            A::value(C::Value),
            A::evaluation(E::EffectsOnly),
        ],
        3,
    ),
    descriptor(
        Id::Trap,
        "__builtin_trap",
        B::Trap,
        Signature {
            result:     T::Scalar(S::Void),
            parameters: &[],
            prototype:  true,
            variadic:   false,
        },
        &[],
        0,
    ),
    // GCC's `int __builtin_setjmp(void *)` and
    // `void __builtin_longjmp(void *, int)`, whose value must be 1. GCC
    // types the buffer `void *` (Clang `void **`), so any object pointer
    // converts to it.
    descriptor(
        Id::Setjmp,
        "__builtin_setjmp",
        B::Setjmp,
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Pointer(S::Void, false)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Pointer(S::Void, false)))],
        1,
    ),
    descriptor(
        Id::Longjmp,
        "__builtin_longjmp",
        B::Longjmp,
        Signature {
            result:     T::Scalar(S::Void),
            parameters: &[T::Pointer(S::Void, false), T::Scalar(S::Int)],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Pointer(S::Void, false))),
            A::constrained(
                C::Parameter(T::Scalar(S::Int)),
                K::ConvertedIntegerValue(1),
                D::LongjmpValue,
            ),
        ],
        2,
    ),
    descriptor(
        Id::UaddOverflow,
        "__builtin_uadd_overflow",
        B::Overflow(OverflowOperation::Add, Some(S::UnsignedInt), false),
        Signature {
            result:     T::Scalar(S::Bool),
            parameters: &[
                T::Scalar(S::UnsignedInt),
                T::Scalar(S::UnsignedInt),
                T::Pointer(S::UnsignedInt, false),
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::UnsignedInt))),
            A::value(C::Parameter(T::Scalar(S::UnsignedInt))),
            A::value(C::Parameter(T::Pointer(S::UnsignedInt, false))),
        ],
        3,
    ),
    descriptor(
        Id::UaddlOverflow,
        "__builtin_uaddl_overflow",
        B::Overflow(OverflowOperation::Add, Some(S::UnsignedLong), false),
        Signature {
            result:     T::Scalar(S::Bool),
            parameters: &[
                T::Scalar(S::UnsignedLong),
                T::Scalar(S::UnsignedLong),
                T::Pointer(S::UnsignedLong, false),
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::UnsignedLong))),
            A::value(C::Parameter(T::Scalar(S::UnsignedLong))),
            A::value(C::Parameter(T::Pointer(S::UnsignedLong, false))),
        ],
        3,
    ),
    descriptor(
        Id::UaddllOverflow,
        "__builtin_uaddll_overflow",
        B::Overflow(OverflowOperation::Add, Some(S::UnsignedLongLong), false),
        Signature {
            result:     T::Scalar(S::Bool),
            parameters: &[
                T::Scalar(S::UnsignedLongLong),
                T::Scalar(S::UnsignedLongLong),
                T::Pointer(S::UnsignedLongLong, false),
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::UnsignedLongLong))),
            A::value(C::Parameter(T::Scalar(S::UnsignedLongLong))),
            A::value(C::Parameter(T::Pointer(S::UnsignedLongLong, false))),
        ],
        3,
    ),
    descriptor(
        Id::UmulOverflow,
        "__builtin_umul_overflow",
        B::Overflow(OverflowOperation::Mul, Some(S::UnsignedInt), false),
        Signature {
            result:     T::Scalar(S::Bool),
            parameters: &[
                T::Scalar(S::UnsignedInt),
                T::Scalar(S::UnsignedInt),
                T::Pointer(S::UnsignedInt, false),
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::UnsignedInt))),
            A::value(C::Parameter(T::Scalar(S::UnsignedInt))),
            A::value(C::Parameter(T::Pointer(S::UnsignedInt, false))),
        ],
        3,
    ),
    descriptor(
        Id::UmullOverflow,
        "__builtin_umull_overflow",
        B::Overflow(OverflowOperation::Mul, Some(S::UnsignedLong), false),
        Signature {
            result:     T::Scalar(S::Bool),
            parameters: &[
                T::Scalar(S::UnsignedLong),
                T::Scalar(S::UnsignedLong),
                T::Pointer(S::UnsignedLong, false),
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::UnsignedLong))),
            A::value(C::Parameter(T::Scalar(S::UnsignedLong))),
            A::value(C::Parameter(T::Pointer(S::UnsignedLong, false))),
        ],
        3,
    ),
    descriptor(
        Id::UmulllOverflow,
        "__builtin_umulll_overflow",
        B::Overflow(OverflowOperation::Mul, Some(S::UnsignedLongLong), false),
        Signature {
            result:     T::Scalar(S::Bool),
            parameters: &[
                T::Scalar(S::UnsignedLongLong),
                T::Scalar(S::UnsignedLongLong),
                T::Pointer(S::UnsignedLongLong, false),
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::UnsignedLongLong))),
            A::value(C::Parameter(T::Scalar(S::UnsignedLongLong))),
            A::value(C::Parameter(T::Pointer(S::UnsignedLongLong, false))),
        ],
        3,
    ),
    descriptor(
        Id::Unreachable,
        "__builtin_unreachable",
        B::Unreachable,
        Signature {
            result:     T::Scalar(S::Void),
            parameters: &[],
            prototype:  true,
            variadic:   false,
        },
        &[],
        0,
    ),
    descriptor(
        Id::UsubOverflow,
        "__builtin_usub_overflow",
        B::Overflow(OverflowOperation::Sub, Some(S::UnsignedInt), false),
        Signature {
            result:     T::Scalar(S::Bool),
            parameters: &[
                T::Scalar(S::UnsignedInt),
                T::Scalar(S::UnsignedInt),
                T::Pointer(S::UnsignedInt, false),
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::UnsignedInt))),
            A::value(C::Parameter(T::Scalar(S::UnsignedInt))),
            A::value(C::Parameter(T::Pointer(S::UnsignedInt, false))),
        ],
        3,
    ),
    descriptor(
        Id::UsublOverflow,
        "__builtin_usubl_overflow",
        B::Overflow(OverflowOperation::Sub, Some(S::UnsignedLong), false),
        Signature {
            result:     T::Scalar(S::Bool),
            parameters: &[
                T::Scalar(S::UnsignedLong),
                T::Scalar(S::UnsignedLong),
                T::Pointer(S::UnsignedLong, false),
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::UnsignedLong))),
            A::value(C::Parameter(T::Scalar(S::UnsignedLong))),
            A::value(C::Parameter(T::Pointer(S::UnsignedLong, false))),
        ],
        3,
    ),
    descriptor(
        Id::UsubllOverflow,
        "__builtin_usubll_overflow",
        B::Overflow(OverflowOperation::Sub, Some(S::UnsignedLongLong), false),
        Signature {
            result:     T::Scalar(S::Bool),
            parameters: &[
                T::Scalar(S::UnsignedLongLong),
                T::Scalar(S::UnsignedLongLong),
                T::Pointer(S::UnsignedLongLong, false),
            ],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::UnsignedLongLong))),
            A::value(C::Parameter(T::Scalar(S::UnsignedLongLong))),
            A::value(C::Parameter(T::Pointer(S::UnsignedLongLong, false))),
        ],
        3,
    ),
    descriptor(
        Id::VaArg,
        "__builtin_va_arg",
        B::Varargs(VaOperation::Arg),
        Signature {
            result:     T::Scalar(S::Void),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[
            A {
                diagnostic: D::VaList,
                ..A::evaluation(E::StorageAddress)
            },
            A {
                conversion: C::Type,
                diagnostic: D::VaArgType,
                ..A::evaluation(E::Type)
            },
        ],
        2,
    ),
    descriptor(
        Id::VaCopy,
        "__builtin_va_copy",
        B::Varargs(VaOperation::Copy),
        Signature {
            result:     T::Scalar(S::Void),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[
            A {
                diagnostic: D::VaList,
                ..A::evaluation(E::StorageAddress)
            },
            A {
                diagnostic: D::VaList,
                ..A::evaluation(E::StorageAddress)
            },
        ],
        2,
    ),
    descriptor(
        Id::VaEnd,
        "__builtin_va_end",
        B::Varargs(VaOperation::End),
        Signature {
            result:     T::Scalar(S::Void),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[A {
            diagnostic: D::VaList,
            ..A::evaluation(E::StorageAddress)
        }],
        1,
    ),
    descriptor(
        Id::VaStart,
        "__builtin_va_start",
        B::Varargs(VaOperation::Start),
        Signature {
            result:     T::Scalar(S::Void),
            parameters: &[],
            prototype:  false,
            variadic:   false,
        },
        &[
            A {
                diagnostic: D::VaList,
                ..A::evaluation(E::StorageAddress)
            },
            A::evaluation(E::Unevaluated),
        ],
        2,
    ),
    descriptor(
        Id::PlainAlloca,
        "alloca",
        B::Alloca,
        Signature {
            result:     T::Pointer(S::Void, false),
            parameters: &[T::Size],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Size))],
        1,
    ),
    descriptor(
        Id::Rotateleft8,
        "__builtin_rotateleft8",
        B::Rotate(8, false),
        Signature {
            result:     T::Scalar(S::UnsignedChar),
            parameters: &[T::Scalar(S::UnsignedChar), T::Scalar(S::UnsignedChar)],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::UnsignedChar))),
            A::value(C::Parameter(T::Scalar(S::UnsignedChar))),
        ],
        2,
    ),
    descriptor(
        Id::Rotateright8,
        "__builtin_rotateright8",
        B::Rotate(8, true),
        Signature {
            result:     T::Scalar(S::UnsignedChar),
            parameters: &[T::Scalar(S::UnsignedChar), T::Scalar(S::UnsignedChar)],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::UnsignedChar))),
            A::value(C::Parameter(T::Scalar(S::UnsignedChar))),
        ],
        2,
    ),
    descriptor(
        Id::Rotateleft16,
        "__builtin_rotateleft16",
        B::Rotate(16, false),
        Signature {
            result:     T::Scalar(S::UnsignedShort),
            parameters: &[T::Scalar(S::UnsignedShort), T::Scalar(S::UnsignedShort)],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::UnsignedShort))),
            A::value(C::Parameter(T::Scalar(S::UnsignedShort))),
        ],
        2,
    ),
    descriptor(
        Id::Rotateright16,
        "__builtin_rotateright16",
        B::Rotate(16, true),
        Signature {
            result:     T::Scalar(S::UnsignedShort),
            parameters: &[T::Scalar(S::UnsignedShort), T::Scalar(S::UnsignedShort)],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Scalar(S::UnsignedShort))),
            A::value(C::Parameter(T::Scalar(S::UnsignedShort))),
        ],
        2,
    ),
    descriptor(
        Id::Rotateleft32,
        "__builtin_rotateleft32",
        B::Rotate(32, false),
        Signature {
            result:     T::Unsigned(32),
            parameters: &[T::Unsigned(32), T::Unsigned(32)],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Unsigned(32))),
            A::value(C::Parameter(T::Unsigned(32))),
        ],
        2,
    ),
    descriptor(
        Id::Rotateright32,
        "__builtin_rotateright32",
        B::Rotate(32, true),
        Signature {
            result:     T::Unsigned(32),
            parameters: &[T::Unsigned(32), T::Unsigned(32)],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Unsigned(32))),
            A::value(C::Parameter(T::Unsigned(32))),
        ],
        2,
    ),
    descriptor(
        Id::Rotateleft64,
        "__builtin_rotateleft64",
        B::Rotate(64, false),
        Signature {
            result:     T::Unsigned(64),
            parameters: &[T::Unsigned(64), T::Unsigned(64)],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Unsigned(64))),
            A::value(C::Parameter(T::Unsigned(64))),
        ],
        2,
    ),
    descriptor(
        Id::Rotateright64,
        "__builtin_rotateright64",
        B::Rotate(64, true),
        Signature {
            result:     T::Unsigned(64),
            parameters: &[T::Unsigned(64), T::Unsigned(64)],
            prototype:  true,
            variadic:   false,
        },
        &[
            A::value(C::Parameter(T::Unsigned(64))),
            A::value(C::Parameter(T::Unsigned(64))),
        ],
        2,
    ),
    descriptor(
        Id::Isnanf,
        "__builtin_isnanf",
        B::Float(FloatOperation::Isnan, Some(S::Float)),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::Float)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Float)))],
        1,
    ),
    descriptor(
        Id::Isnanl,
        "__builtin_isnanl",
        B::Float(FloatOperation::Isnan, Some(S::LongDouble)),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::LongDouble)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::LongDouble)))],
        1,
    ),
    // Clang's x86 `__rdtsc` builtin, declared for every x86 target and
    // language mode: `unsigned long long __rdtsc(void)`.
    descriptor(
        Id::Rdtsc,
        "__rdtsc",
        B::ReadTimestamp,
        Signature {
            result:     T::Scalar(S::UnsignedLongLong),
            parameters: &[],
            prototype:  true,
            variadic:   false,
        },
        &[],
        0,
    ),
    // Microsoft's `void __va_start(va_list *, ...)`, with Microsoft
    // extensions: `<vadefs.h>`'s `va_start` on x64.
    descriptor(
        Id::MsVaStart,
        "__va_start",
        B::Varargs(VaOperation::StartAt),
        Signature {
            result:     T::Scalar(S::Void),
            parameters: &[T::VaListPointer],
            prototype:  true,
            variadic:   true,
        },
        &[
            A {
                diagnostic: D::VaList,
                ..A::evaluation(E::AdjustedPointer)
            },
            A::evaluation(E::Unevaluated),
        ],
        2,
    ),
    // Microsoft's `void __debugbreak(void)`, with Microsoft extensions.
    descriptor(
        Id::Debugbreak,
        "__debugbreak",
        B::DebugTrap,
        Signature {
            result:     T::Scalar(S::Void),
            parameters: &[],
            prototype:  true,
            variadic:   false,
        },
        &[],
        0,
    ),
    // Microsoft's `void __annotation(const wchar_t *, ...)`: one or more
    // wide string literals, with Microsoft extensions.
    BuiltinDescriptor {
        arity: (1, usize::MAX),
        ..descriptor(
            Id::Annotation,
            "__annotation",
            B::Annotation,
            Signature {
                result:     T::Scalar(S::Void),
                parameters: &[],
                prototype:  true,
                variadic:   true,
            },
            &[],
            0,
        )
    },
    // GCC's `int __builtin_va_arg_pack(void)` and
    // `int __builtin_va_arg_pack_len(void)`: the anonymous arguments of the
    // variadic function they appear in, at a call that inlines it.
    descriptor(
        Id::VaArgPack,
        "__builtin_va_arg_pack",
        B::ArgumentPack(false),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[],
            prototype:  true,
            variadic:   false,
        },
        &[],
        0,
    ),
    descriptor(
        Id::VaArgPackLen,
        "__builtin_va_arg_pack_len",
        B::ArgumentPack(true),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[],
            prototype:  true,
            variadic:   false,
        },
        &[],
        0,
    ),
    // GNU: GCC's `finite` family, `isfinite` of one fixed type, which
    // glibc's `<math.h>` may call (GCC manual, "Other Builtins").
    descriptor(
        Id::Finite,
        "__builtin_finite",
        B::Float(FloatOperation::Isfinite, Some(S::Double)),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::Double)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Double)))],
        1,
    ),
    // GNU: GCC's `finite` family, `isfinite` of one fixed type, which
    // glibc's `<math.h>` may call (GCC manual, "Other Builtins").
    descriptor(
        Id::Finitef,
        "__builtin_finitef",
        B::Float(FloatOperation::Isfinite, Some(S::Float)),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::Float)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::Float)))],
        1,
    ),
    // GNU: GCC's `finite` family, `isfinite` of one fixed type, which
    // glibc's `<math.h>` may call (GCC manual, "Other Builtins").
    descriptor(
        Id::Finitel,
        "__builtin_finitel",
        B::Float(FloatOperation::Isfinite, Some(S::LongDouble)),
        Signature {
            result:     T::Scalar(S::Int),
            parameters: &[T::Scalar(S::LongDouble)],
            prototype:  true,
            variadic:   false,
        },
        &[A::value(C::Parameter(T::Scalar(S::LongDouble)))],
        1,
    ),
];

/// A builtin only GCC has, which Clang 23 rejects as an unknown builtin.
const fn gcc_only(mut descriptor: BuiltinDescriptor) -> BuiltinDescriptor {
    descriptor.availability.feature = Some(crate::configuration::Feature::GccOnlyBuiltins);
    descriptor
}

/// A sign operation on one floating type, which exists where `feature` is
/// accepted: `_Float16`'s where the type is, and GCC's `q` spellings of the
/// `__float128` ones, which Clang lacks.
/// GNU extension: GCC manual, "Other Builtins" and "Additional Floating
/// Types". <https://gcc.gnu.org/onlinedocs/gcc/Floating-Types.html>
const fn float_descriptor(
    id: Id,
    spelling: &'static str,
    operation: FloatOperation,
    scalar: S,
    arguments: usize,
    feature: crate::configuration::Feature,
) -> BuiltinDescriptor {
    let (signature, rules) = match (scalar, arguments) {
        | (S::Float16, 1) => HALF_UNARY,
        | (S::Float16, _) => HALF_BINARY,
        | (_, 1) => QUAD_UNARY,
        | _ => QUAD_BINARY,
    };
    let mut descriptor = descriptor(
        id,
        spelling,
        B::Float(operation, Some(scalar)),
        signature,
        rules,
        arguments,
    );
    descriptor.availability.feature = Some(feature);
    descriptor
}

/// The signatures of the `_Float16` and `__float128` sign operations.
const HALF_UNARY: (Signature, &[A]) = sign_contract(
    &[T::Scalar(S::Float16)],
    &[A::value(C::Parameter(T::Scalar(S::Float16)))],
);
const HALF_BINARY: (Signature, &[A]) = sign_contract(
    &[T::Scalar(S::Float16), T::Scalar(S::Float16)],
    &[
        A::value(C::Parameter(T::Scalar(S::Float16))),
        A::value(C::Parameter(T::Scalar(S::Float16))),
    ],
);
const QUAD_UNARY: (Signature, &[A]) = sign_contract(
    &[T::Scalar(S::Float128)],
    &[A::value(C::Parameter(T::Scalar(S::Float128)))],
);
const QUAD_BINARY: (Signature, &[A]) = sign_contract(
    &[T::Scalar(S::Float128), T::Scalar(S::Float128)],
    &[
        A::value(C::Parameter(T::Scalar(S::Float128))),
        A::value(C::Parameter(T::Scalar(S::Float128))),
    ],
);

const fn sign_contract(parameters: &'static [T], rules: &'static [A]) -> (Signature, &'static [A]) {
    (
        Signature {
            result: parameters[0],
            parameters,
            prototype: true,
            variadic: false,
        },
        rules,
    )
}

const fn descriptor(
    id: Id,
    spelling: &'static str,
    operation: B,
    signature: Signature,
    arguments: &'static [A],
    minimum: usize,
) -> BuiltinDescriptor {
    let (type_rule, effects, feature, syntax) = match operation {
        | B::Varargs(VaOperation::Arg) => (
            TypeRule::Varargs,
            BuiltinEffectClass::Write,
            None,
            SyntaxForm::VaArg,
        ),
        | B::Varargs(_) => (
            TypeRule::Varargs,
            BuiltinEffectClass::Write,
            None,
            SyntaxForm::OrdinaryCall,
        ),
        | B::Offsetof => (
            TypeRule::Offsetof,
            BuiltinEffectClass::Query,
            None,
            SyntaxForm::Offsetof,
        ),
        | B::Float(..) | B::Complex => (
            TypeRule::Floating,
            BuiltinEffectClass::Pure,
            Some(crate::configuration::Feature::GnuBuiltins),
            SyntaxForm::OrdinaryCall,
        ),
        | B::Carry(..) => (
            TypeRule::Fixed,
            BuiltinEffectClass::Write,
            Some(crate::configuration::Feature::GnuBuiltins),
            SyntaxForm::OrdinaryCall,
        ),
        | B::Overflow(_, _, predicate) => (
            TypeRule::Overflow,
            if predicate {
                BuiltinEffectClass::Pure
            } else {
                BuiltinEffectClass::Write
            },
            Some(crate::configuration::Feature::GnuBuiltins),
            SyntaxForm::OrdinaryCall,
        ),
        | B::Constant | B::ObjectSize(_) | B::Address(_) | B::ArgumentPack(_) => (
            TypeRule::Fixed,
            BuiltinEffectClass::Query,
            Some(crate::configuration::Feature::GnuBuiltins),
            SyntaxForm::OrdinaryCall,
        ),
        | B::Alloca | B::AllocaAligned => (
            TypeRule::Fixed,
            BuiltinEffectClass::Allocate,
            Some(crate::configuration::Feature::GnuBuiltins),
            SyntaxForm::OrdinaryCall,
        ),
        | B::Trap | B::Unreachable | B::Setjmp | B::Longjmp => (
            TypeRule::Fixed,
            BuiltinEffectClass::Control,
            Some(crate::configuration::Feature::GnuBuiltins),
            SyntaxForm::OrdinaryCall,
        ),
        | B::ReadTimestamp | B::DebugTrap | B::ClearCache => (
            TypeRule::Fixed,
            BuiltinEffectClass::Write,
            None,
            SyntaxForm::OrdinaryCall,
        ),
        | B::Annotation => (
            TypeRule::WideStrings,
            BuiltinEffectClass::Query,
            None,
            SyntaxForm::OrdinaryCall,
        ),
        | _ => (
            TypeRule::Fixed,
            BuiltinEffectClass::Pure,
            Some(crate::configuration::Feature::GnuBuiltins),
            SyntaxForm::OrdinaryCall,
        ),
    };
    // Clang 23 has no `__builtin_iseqsig`.
    let feature = match id {
        | Id::Iseqsig => Some(crate::configuration::Feature::GccOnlyBuiltins),
        | _ => feature,
    };
    BuiltinDescriptor {
        id,
        spelling,
        operation,
        signature,
        arguments,
        type_rule,
        effects,
        syntax,
        arity: (minimum, arguments.len()),
        library: None,
        designator: DesignatorPolicy::DirectCallOnly,
        availability: Availability {
            feature,
            naming: match id {
                | Id::PlainAlloca => NamingPolicy::GnuOrdinary,
                | Id::Debugbreak | Id::Annotation | Id::MsVaStart => NamingPolicy::Microsoft,
                | _ => NamingPolicy::Reserved,
            },
        },
    }
}

/// GCC builtins this compiler knows exist but does not implement: lowering
/// refuses a call by name rather than as a type it could not analyze.
/// `__builtin_apply`, `__builtin_apply_args` and `__builtin_return` forward
/// a call's argument registers; `__builtin_call_with_static_chain` (which
/// Clang has) calls through a nested function's chain; and
/// `__builtin_clear_padding` needs every padding bit of a type.
/// GNU extension: GCC manual, "Constructing Calls" and "Other Builtins".
/// <https://gcc.gnu.org/onlinedocs/gcc/Constructing-Calls.html>
pub(crate) fn unimplemented(name: &str) -> Option<&'static str> {
    [
        "__builtin_apply",
        "__builtin_apply_args",
        "__builtin_call_with_static_chain",
        "__builtin_clear_padding",
        "__builtin_powi",
        "__builtin_powif",
        "__builtin_powil",
        "__builtin_return",
        "__builtin_speculation_safe_value",
        "__builtin_stack_restore",
        "__builtin_stack_save",
    ]
    .into_iter()
    .find(|&known| known == name)
}
