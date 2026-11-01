//! C99 real math aliases and infinity/NaN constructors. Contracts cover each
//! float, double and long-double signature, and the infinity and NaN
//! constructors cover the `_FloatN` types too; the back ends use the C libm
//! ABI. Existing fabs/floor contracts stay in `library`; copysign and
//! classification retain their scalar operations. GNU extension: GCC "Library
//! Builtins" and "Other Builtins", <https://gcc.gnu.org/onlinedocs/gcc/Library-Builtins.html>.
//! C99: §7.12p3-5, pp. 212-213; PDF pp. 224-225 (math constants);
//! §7.12.4-§7.12.13, pp. 218-239; PDF pp. 230-251 (real math families).

use super::{
    ArgumentRule as A,
    Availability,
    BuiltinDescriptor,
    BuiltinEffectClass,
    BuiltinId as Id,
    BuiltinOperation as B,
    ConversionRule as C,
    DesignatorPolicy,
    NamingPolicy,
    Signature,
    SignatureType as T,
    SyntaxForm,
    TypeRule,
};
use crate::{
    configuration::Feature,
    target::Scalar as S,
};

/// Whether the C library function `name` is a real math function that,
/// but for setting `errno`, reads and writes no memory and has no other
/// effect: one whose parameters are all numbers, as Clang's
/// `ConstIgnoringErrno` library builtins are. `lgamma` and its relatives
/// also store the sign of the result in `signgam` (POSIX), and `nan` reads
/// a string, so neither counts. Under `-fno-math-errno` such a call is an
/// operation on its arguments, which the LLVM back end tells LLVM.
/// C99: §7.12.1 paragraph 1, p. 213; PDF p. 225 (the error conditions
/// of the math functions and `math_errhandling`).
pub(crate) fn computes_without_errno(name: &str) -> bool {
    !name.starts_with("lgamma")
        && BUILTINS.iter().any(|descriptor| {
            let parameters = descriptor.signature.parameters;
            descriptor.library == Some(name)
                && !parameters.is_empty()
                && parameters
                    .iter()
                    .all(|parameter| matches!(parameter, T::Scalar(_)))
        })
}

pub(super) const BUILTINS: &[BuiltinDescriptor] = &[
    descriptor(Id::LibraryAcos, "__builtin_acos", "acos", DoubleUnary),
    descriptor(Id::LibraryAcosf, "__builtin_acosf", "acosf", FloatUnary),
    descriptor(Id::LibraryAcosh, "__builtin_acosh", "acosh", DoubleUnary),
    descriptor(Id::LibraryAcoshf, "__builtin_acoshf", "acoshf", FloatUnary),
    descriptor(
        Id::LibraryAcoshl,
        "__builtin_acoshl",
        "acoshl",
        LongDoubleUnary,
    ),
    descriptor(
        Id::LibraryAcosl,
        "__builtin_acosl",
        "acosl",
        LongDoubleUnary,
    ),
    descriptor(Id::LibraryAsin, "__builtin_asin", "asin", DoubleUnary),
    descriptor(Id::LibraryAsinf, "__builtin_asinf", "asinf", FloatUnary),
    descriptor(Id::LibraryAsinh, "__builtin_asinh", "asinh", DoubleUnary),
    descriptor(Id::LibraryAsinhf, "__builtin_asinhf", "asinhf", FloatUnary),
    descriptor(
        Id::LibraryAsinhl,
        "__builtin_asinhl",
        "asinhl",
        LongDoubleUnary,
    ),
    descriptor(
        Id::LibraryAsinl,
        "__builtin_asinl",
        "asinl",
        LongDoubleUnary,
    ),
    descriptor(Id::LibraryAtan, "__builtin_atan", "atan", DoubleUnary),
    descriptor(Id::LibraryAtan2, "__builtin_atan2", "atan2", DoubleBinary),
    descriptor(Id::LibraryAtan2f, "__builtin_atan2f", "atan2f", FloatBinary),
    descriptor(
        Id::LibraryAtan2l,
        "__builtin_atan2l",
        "atan2l",
        LongDoubleBinary,
    ),
    descriptor(Id::LibraryAtanf, "__builtin_atanf", "atanf", FloatUnary),
    descriptor(Id::LibraryAtanh, "__builtin_atanh", "atanh", DoubleUnary),
    descriptor(Id::LibraryAtanhf, "__builtin_atanhf", "atanhf", FloatUnary),
    descriptor(
        Id::LibraryAtanhl,
        "__builtin_atanhl",
        "atanhl",
        LongDoubleUnary,
    ),
    descriptor(
        Id::LibraryAtanl,
        "__builtin_atanl",
        "atanl",
        LongDoubleUnary,
    ),
    descriptor(Id::LibraryCbrt, "__builtin_cbrt", "cbrt", DoubleUnary),
    descriptor(Id::LibraryCbrtf, "__builtin_cbrtf", "cbrtf", FloatUnary),
    descriptor(
        Id::LibraryCbrtl,
        "__builtin_cbrtl",
        "cbrtl",
        LongDoubleUnary,
    ),
    descriptor(Id::LibraryCeil, "__builtin_ceil", "ceil", DoubleUnary),
    descriptor(Id::LibraryCeilf, "__builtin_ceilf", "ceilf", FloatUnary),
    descriptor(
        Id::LibraryCeill,
        "__builtin_ceill",
        "ceill",
        LongDoubleUnary,
    ),
    descriptor(Id::LibraryCos, "__builtin_cos", "cos", DoubleUnary),
    descriptor(Id::LibraryCosf, "__builtin_cosf", "cosf", FloatUnary),
    descriptor(Id::LibraryCosh, "__builtin_cosh", "cosh", DoubleUnary),
    descriptor(Id::LibraryCoshf, "__builtin_coshf", "coshf", FloatUnary),
    descriptor(
        Id::LibraryCoshl,
        "__builtin_coshl",
        "coshl",
        LongDoubleUnary,
    ),
    descriptor(Id::LibraryCosl, "__builtin_cosl", "cosl", LongDoubleUnary),
    descriptor(Id::LibraryErf, "__builtin_erf", "erf", DoubleUnary),
    descriptor(Id::LibraryErfc, "__builtin_erfc", "erfc", DoubleUnary),
    descriptor(Id::LibraryErfcf, "__builtin_erfcf", "erfcf", FloatUnary),
    descriptor(
        Id::LibraryErfcl,
        "__builtin_erfcl",
        "erfcl",
        LongDoubleUnary,
    ),
    descriptor(Id::LibraryErff, "__builtin_erff", "erff", FloatUnary),
    descriptor(Id::LibraryErfl, "__builtin_erfl", "erfl", LongDoubleUnary),
    descriptor(Id::LibraryExp, "__builtin_exp", "exp", DoubleUnary),
    descriptor(Id::LibraryExp2, "__builtin_exp2", "exp2", DoubleUnary),
    descriptor(Id::LibraryExp2f, "__builtin_exp2f", "exp2f", FloatUnary),
    descriptor(
        Id::LibraryExp2l,
        "__builtin_exp2l",
        "exp2l",
        LongDoubleUnary,
    ),
    descriptor(Id::LibraryExpf, "__builtin_expf", "expf", FloatUnary),
    descriptor(Id::LibraryExpl, "__builtin_expl", "expl", LongDoubleUnary),
    descriptor(Id::LibraryExpm1, "__builtin_expm1", "expm1", DoubleUnary),
    descriptor(Id::LibraryExpm1f, "__builtin_expm1f", "expm1f", FloatUnary),
    descriptor(
        Id::LibraryExpm1l,
        "__builtin_expm1l",
        "expm1l",
        LongDoubleUnary,
    ),
    descriptor(Id::LibraryFdim, "__builtin_fdim", "fdim", DoubleBinary),
    descriptor(Id::LibraryFdimf, "__builtin_fdimf", "fdimf", FloatBinary),
    descriptor(
        Id::LibraryFdiml,
        "__builtin_fdiml",
        "fdiml",
        LongDoubleBinary,
    ),
    descriptor(Id::LibraryFloorf, "__builtin_floorf", "floorf", FloatUnary),
    descriptor(
        Id::LibraryFloorl,
        "__builtin_floorl",
        "floorl",
        LongDoubleUnary,
    ),
    descriptor(Id::LibraryFma, "__builtin_fma", "fma", DoubleTernary),
    descriptor(Id::LibraryFmaf, "__builtin_fmaf", "fmaf", FloatTernary),
    descriptor(Id::LibraryFmal, "__builtin_fmal", "fmal", LongDoubleTernary),
    descriptor(Id::LibraryFmax, "__builtin_fmax", "fmax", DoubleBinary),
    descriptor(Id::LibraryFmaxf, "__builtin_fmaxf", "fmaxf", FloatBinary),
    descriptor(
        Id::LibraryFmaxl,
        "__builtin_fmaxl",
        "fmaxl",
        LongDoubleBinary,
    ),
    descriptor(Id::LibraryFmin, "__builtin_fmin", "fmin", DoubleBinary),
    descriptor(Id::LibraryFminf, "__builtin_fminf", "fminf", FloatBinary),
    descriptor(
        Id::LibraryFminl,
        "__builtin_fminl",
        "fminl",
        LongDoubleBinary,
    ),
    descriptor(Id::LibraryFmod, "__builtin_fmod", "fmod", DoubleBinary),
    descriptor(Id::LibraryFmodf, "__builtin_fmodf", "fmodf", FloatBinary),
    descriptor(
        Id::LibraryFmodl,
        "__builtin_fmodl",
        "fmodl",
        LongDoubleBinary,
    ),
    descriptor(
        Id::LibraryFrexp,
        "__builtin_frexp",
        "frexp",
        DoubleIntPointer,
    ),
    descriptor(
        Id::LibraryFrexpf,
        "__builtin_frexpf",
        "frexpf",
        FloatIntPointer,
    ),
    descriptor(
        Id::LibraryFrexpl,
        "__builtin_frexpl",
        "frexpl",
        LongDoubleIntPointer,
    ),
    constant_descriptor(
        Id::HugeVal,
        "__builtin_huge_val",
        None,
        DoubleNullary,
        S::Double,
        false,
    ),
    constant_descriptor(
        Id::HugeValf,
        "__builtin_huge_valf",
        None,
        FloatNullary,
        S::Float,
        false,
    ),
    half_constant_descriptor(
        Id::HugeValf16,
        "__builtin_huge_valf16",
        None,
        Float16Nullary,
        S::Float16,
        false,
    ),
    interchange_constant_descriptor(
        Id::HugeValf32,
        "__builtin_huge_valf32",
        None,
        Float32Nullary,
        S::Float32,
        false,
    ),
    interchange_constant_descriptor(
        Id::HugeValf32x,
        "__builtin_huge_valf32x",
        None,
        Float32xNullary,
        S::Float32x,
        false,
    ),
    interchange_constant_descriptor(
        Id::HugeValf64,
        "__builtin_huge_valf64",
        None,
        Float64Nullary,
        S::Float64,
        false,
    ),
    interchange_constant_descriptor(
        Id::HugeValf64x,
        "__builtin_huge_valf64x",
        None,
        Float64xNullary,
        S::Float64x,
        false,
    ),
    constant_descriptor(
        Id::HugeVall,
        "__builtin_huge_vall",
        None,
        LongDoubleNullary,
        S::LongDouble,
        false,
    ),
    descriptor(Id::LibraryHypot, "__builtin_hypot", "hypot", DoubleBinary),
    descriptor(Id::LibraryHypotf, "__builtin_hypotf", "hypotf", FloatBinary),
    descriptor(
        Id::LibraryHypotl,
        "__builtin_hypotl",
        "hypotl",
        LongDoubleBinary,
    ),
    descriptor(
        Id::LibraryIlogb,
        "__builtin_ilogb",
        "ilogb",
        DoubleIntResult,
    ),
    descriptor(
        Id::LibraryIlogbf,
        "__builtin_ilogbf",
        "ilogbf",
        FloatIntResult,
    ),
    descriptor(
        Id::LibraryIlogbl,
        "__builtin_ilogbl",
        "ilogbl",
        LongDoubleIntResult,
    ),
    constant_descriptor(
        Id::Inf,
        "__builtin_inf",
        None,
        DoubleNullary,
        S::Double,
        false,
    ),
    constant_descriptor(
        Id::Inff,
        "__builtin_inff",
        None,
        FloatNullary,
        S::Float,
        false,
    ),
    half_constant_descriptor(
        Id::Inff16,
        "__builtin_inff16",
        None,
        Float16Nullary,
        S::Float16,
        false,
    ),
    interchange_constant_descriptor(
        Id::Inff32,
        "__builtin_inff32",
        None,
        Float32Nullary,
        S::Float32,
        false,
    ),
    interchange_constant_descriptor(
        Id::Inff32x,
        "__builtin_inff32x",
        None,
        Float32xNullary,
        S::Float32x,
        false,
    ),
    interchange_constant_descriptor(
        Id::Inff64,
        "__builtin_inff64",
        None,
        Float64Nullary,
        S::Float64,
        false,
    ),
    interchange_constant_descriptor(
        Id::Inff64x,
        "__builtin_inff64x",
        None,
        Float64xNullary,
        S::Float64x,
        false,
    ),
    constant_descriptor(
        Id::Infl,
        "__builtin_infl",
        None,
        LongDoubleNullary,
        S::LongDouble,
        false,
    ),
    descriptor(
        Id::LibraryLdexp,
        "__builtin_ldexp",
        "ldexp",
        DoubleIntExponent,
    ),
    descriptor(
        Id::LibraryLdexpf,
        "__builtin_ldexpf",
        "ldexpf",
        FloatIntExponent,
    ),
    descriptor(
        Id::LibraryLdexpl,
        "__builtin_ldexpl",
        "ldexpl",
        LongDoubleIntExponent,
    ),
    descriptor(Id::LibraryLgamma, "__builtin_lgamma", "lgamma", DoubleUnary),
    descriptor(
        Id::LibraryLgammaf,
        "__builtin_lgammaf",
        "lgammaf",
        FloatUnary,
    ),
    descriptor(
        Id::LibraryLgammal,
        "__builtin_lgammal",
        "lgammal",
        LongDoubleUnary,
    ),
    descriptor(
        Id::LibraryLlrint,
        "__builtin_llrint",
        "llrint",
        DoubleLongLongResult,
    ),
    descriptor(
        Id::LibraryLlrintf,
        "__builtin_llrintf",
        "llrintf",
        FloatLongLongResult,
    ),
    descriptor(
        Id::LibraryLlrintl,
        "__builtin_llrintl",
        "llrintl",
        LongDoubleLongLongResult,
    ),
    descriptor(
        Id::LibraryLlround,
        "__builtin_llround",
        "llround",
        DoubleLongLongResult,
    ),
    descriptor(
        Id::LibraryLlroundf,
        "__builtin_llroundf",
        "llroundf",
        FloatLongLongResult,
    ),
    descriptor(
        Id::LibraryLlroundl,
        "__builtin_llroundl",
        "llroundl",
        LongDoubleLongLongResult,
    ),
    descriptor(Id::LibraryLog, "__builtin_log", "log", DoubleUnary),
    descriptor(Id::LibraryLog10, "__builtin_log10", "log10", DoubleUnary),
    descriptor(Id::LibraryLog10f, "__builtin_log10f", "log10f", FloatUnary),
    descriptor(
        Id::LibraryLog10l,
        "__builtin_log10l",
        "log10l",
        LongDoubleUnary,
    ),
    descriptor(Id::LibraryLog1p, "__builtin_log1p", "log1p", DoubleUnary),
    descriptor(Id::LibraryLog1pf, "__builtin_log1pf", "log1pf", FloatUnary),
    descriptor(
        Id::LibraryLog1pl,
        "__builtin_log1pl",
        "log1pl",
        LongDoubleUnary,
    ),
    descriptor(Id::LibraryLog2, "__builtin_log2", "log2", DoubleUnary),
    descriptor(Id::LibraryLog2f, "__builtin_log2f", "log2f", FloatUnary),
    descriptor(
        Id::LibraryLog2l,
        "__builtin_log2l",
        "log2l",
        LongDoubleUnary,
    ),
    descriptor(Id::LibraryLogb, "__builtin_logb", "logb", DoubleUnary),
    descriptor(Id::LibraryLogbf, "__builtin_logbf", "logbf", FloatUnary),
    descriptor(
        Id::LibraryLogbl,
        "__builtin_logbl",
        "logbl",
        LongDoubleUnary,
    ),
    descriptor(Id::LibraryLogf, "__builtin_logf", "logf", FloatUnary),
    descriptor(Id::LibraryLogl, "__builtin_logl", "logl", LongDoubleUnary),
    descriptor(
        Id::LibraryLrint,
        "__builtin_lrint",
        "lrint",
        DoubleLongResult,
    ),
    descriptor(
        Id::LibraryLrintf,
        "__builtin_lrintf",
        "lrintf",
        FloatLongResult,
    ),
    descriptor(
        Id::LibraryLrintl,
        "__builtin_lrintl",
        "lrintl",
        LongDoubleLongResult,
    ),
    descriptor(
        Id::LibraryLround,
        "__builtin_lround",
        "lround",
        DoubleLongResult,
    ),
    descriptor(
        Id::LibraryLroundf,
        "__builtin_lroundf",
        "lroundf",
        FloatLongResult,
    ),
    descriptor(
        Id::LibraryLroundl,
        "__builtin_lroundl",
        "lroundl",
        LongDoubleLongResult,
    ),
    descriptor(
        Id::LibraryModf,
        "__builtin_modf",
        "modf",
        DoubleFloatPointer,
    ),
    descriptor(
        Id::LibraryModff,
        "__builtin_modff",
        "modff",
        FloatFloatPointer,
    ),
    descriptor(
        Id::LibraryModfl,
        "__builtin_modfl",
        "modfl",
        LongDoubleFloatPointer,
    ),
    constant_descriptor(
        Id::LibraryNan,
        "__builtin_nan",
        Some("nan"),
        DoubleNan,
        S::Double,
        true,
    ),
    constant_descriptor(
        Id::LibraryNanf,
        "__builtin_nanf",
        Some("nanf"),
        FloatNan,
        S::Float,
        true,
    ),
    half_constant_descriptor(
        Id::LibraryNanf16,
        "__builtin_nanf16",
        Some("nanf16"),
        Float16Nan,
        S::Float16,
        true,
    ),
    interchange_constant_descriptor(
        Id::LibraryNanf32,
        "__builtin_nanf32",
        Some("nanf32"),
        Float32Nan,
        S::Float32,
        true,
    ),
    interchange_constant_descriptor(
        Id::LibraryNanf32x,
        "__builtin_nanf32x",
        Some("nanf32x"),
        Float32xNan,
        S::Float32x,
        true,
    ),
    interchange_constant_descriptor(
        Id::LibraryNanf64,
        "__builtin_nanf64",
        Some("nanf64"),
        Float64Nan,
        S::Float64,
        true,
    ),
    interchange_constant_descriptor(
        Id::LibraryNanf64x,
        "__builtin_nanf64x",
        Some("nanf64x"),
        Float64xNan,
        S::Float64x,
        true,
    ),
    constant_descriptor(
        Id::LibraryNanl,
        "__builtin_nanl",
        Some("nanl"),
        LongDoubleNan,
        S::LongDouble,
        true,
    ),
    descriptor(
        Id::LibraryNearbyint,
        "__builtin_nearbyint",
        "nearbyint",
        DoubleUnary,
    ),
    descriptor(
        Id::LibraryNearbyintf,
        "__builtin_nearbyintf",
        "nearbyintf",
        FloatUnary,
    ),
    descriptor(
        Id::LibraryNearbyintl,
        "__builtin_nearbyintl",
        "nearbyintl",
        LongDoubleUnary,
    ),
    descriptor(
        Id::LibraryNextafter,
        "__builtin_nextafter",
        "nextafter",
        DoubleBinary,
    ),
    descriptor(
        Id::LibraryNextafterf,
        "__builtin_nextafterf",
        "nextafterf",
        FloatBinary,
    ),
    descriptor(
        Id::LibraryNextafterl,
        "__builtin_nextafterl",
        "nextafterl",
        LongDoubleBinary,
    ),
    descriptor(
        Id::LibraryNexttoward,
        "__builtin_nexttoward",
        "nexttoward",
        DoubleNexttoward,
    ),
    descriptor(
        Id::LibraryNexttowardf,
        "__builtin_nexttowardf",
        "nexttowardf",
        FloatNexttoward,
    ),
    descriptor(
        Id::LibraryNexttowardl,
        "__builtin_nexttowardl",
        "nexttowardl",
        LongDoubleNexttoward,
    ),
    descriptor(Id::LibraryPow, "__builtin_pow", "pow", DoubleBinary),
    descriptor(Id::LibraryPowf, "__builtin_powf", "powf", FloatBinary),
    descriptor(Id::LibraryPowl, "__builtin_powl", "powl", LongDoubleBinary),
    descriptor(
        Id::LibraryRemainder,
        "__builtin_remainder",
        "remainder",
        DoubleBinary,
    ),
    descriptor(
        Id::LibraryRemainderf,
        "__builtin_remainderf",
        "remainderf",
        FloatBinary,
    ),
    descriptor(
        Id::LibraryRemainderl,
        "__builtin_remainderl",
        "remainderl",
        LongDoubleBinary,
    ),
    descriptor(
        Id::LibraryRemquo,
        "__builtin_remquo",
        "remquo",
        DoubleRemquo,
    ),
    descriptor(
        Id::LibraryRemquof,
        "__builtin_remquof",
        "remquof",
        FloatRemquo,
    ),
    descriptor(
        Id::LibraryRemquol,
        "__builtin_remquol",
        "remquol",
        LongDoubleRemquo,
    ),
    descriptor(Id::LibraryRint, "__builtin_rint", "rint", DoubleUnary),
    descriptor(Id::LibraryRintf, "__builtin_rintf", "rintf", FloatUnary),
    descriptor(
        Id::LibraryRintl,
        "__builtin_rintl",
        "rintl",
        LongDoubleUnary,
    ),
    descriptor(Id::LibraryRound, "__builtin_round", "round", DoubleUnary),
    descriptor(
        Id::LibraryRoundeven,
        "__builtin_roundeven",
        "roundeven",
        DoubleUnary,
    ),
    descriptor(
        Id::LibraryRoundevenf,
        "__builtin_roundevenf",
        "roundevenf",
        FloatUnary,
    ),
    descriptor(
        Id::LibraryRoundevenl,
        "__builtin_roundevenl",
        "roundevenl",
        LongDoubleUnary,
    ),
    descriptor(Id::LibraryRoundf, "__builtin_roundf", "roundf", FloatUnary),
    descriptor(
        Id::LibraryRoundl,
        "__builtin_roundl",
        "roundl",
        LongDoubleUnary,
    ),
    descriptor(
        Id::LibraryScalbln,
        "__builtin_scalbln",
        "scalbln",
        DoubleLongExponent,
    ),
    descriptor(
        Id::LibraryScalblnf,
        "__builtin_scalblnf",
        "scalblnf",
        FloatLongExponent,
    ),
    descriptor(
        Id::LibraryScalblnl,
        "__builtin_scalblnl",
        "scalblnl",
        LongDoubleLongExponent,
    ),
    descriptor(
        Id::LibraryScalbn,
        "__builtin_scalbn",
        "scalbn",
        DoubleIntExponent,
    ),
    descriptor(
        Id::LibraryScalbnf,
        "__builtin_scalbnf",
        "scalbnf",
        FloatIntExponent,
    ),
    descriptor(
        Id::LibraryScalbnl,
        "__builtin_scalbnl",
        "scalbnl",
        LongDoubleIntExponent,
    ),
    descriptor(Id::LibrarySin, "__builtin_sin", "sin", DoubleUnary),
    descriptor(Id::LibrarySinf, "__builtin_sinf", "sinf", FloatUnary),
    descriptor(Id::LibrarySinh, "__builtin_sinh", "sinh", DoubleUnary),
    descriptor(Id::LibrarySinhf, "__builtin_sinhf", "sinhf", FloatUnary),
    descriptor(
        Id::LibrarySinhl,
        "__builtin_sinhl",
        "sinhl",
        LongDoubleUnary,
    ),
    descriptor(Id::LibrarySinl, "__builtin_sinl", "sinl", LongDoubleUnary),
    descriptor(Id::LibrarySqrt, "__builtin_sqrt", "sqrt", DoubleUnary),
    descriptor(Id::LibrarySqrtf, "__builtin_sqrtf", "sqrtf", FloatUnary),
    descriptor(
        Id::LibrarySqrtl,
        "__builtin_sqrtl",
        "sqrtl",
        LongDoubleUnary,
    ),
    descriptor(Id::LibraryTan, "__builtin_tan", "tan", DoubleUnary),
    descriptor(Id::LibraryTanf, "__builtin_tanf", "tanf", FloatUnary),
    descriptor(Id::LibraryTanh, "__builtin_tanh", "tanh", DoubleUnary),
    descriptor(Id::LibraryTanhf, "__builtin_tanhf", "tanhf", FloatUnary),
    descriptor(
        Id::LibraryTanhl,
        "__builtin_tanhl",
        "tanhl",
        LongDoubleUnary,
    ),
    descriptor(Id::LibraryTanl, "__builtin_tanl", "tanl", LongDoubleUnary),
    descriptor(Id::LibraryTgamma, "__builtin_tgamma", "tgamma", DoubleUnary),
    descriptor(
        Id::LibraryTgammaf,
        "__builtin_tgammaf",
        "tgammaf",
        FloatUnary,
    ),
    descriptor(
        Id::LibraryTgammal,
        "__builtin_tgammal",
        "tgammal",
        LongDoubleUnary,
    ),
    descriptor(Id::LibraryTrunc, "__builtin_trunc", "trunc", DoubleUnary),
    descriptor(Id::LibraryTruncf, "__builtin_truncf", "truncf", FloatUnary),
    descriptor(
        Id::LibraryTruncl,
        "__builtin_truncl",
        "truncl",
        LongDoubleUnary,
    ),
];

pub(super) fn ordinary(name: &str) -> Option<&'static BuiltinDescriptor> {
    BUILTINS
        .binary_search_by_key(&name, |descriptor| {
            descriptor
                .spelling
                .strip_prefix("__builtin_")
                .expect("math alias")
        })
        .ok()
        .map(|index| &BUILTINS[index])
        .filter(|descriptor| descriptor.library.is_some())
}

const fn descriptor(
    id: Id,
    spelling: &'static str,
    library: &'static str,
    contract: Contract,
) -> BuiltinDescriptor {
    BuiltinDescriptor {
        id,
        spelling,
        library: Some(library),
        signature: contract.signature,
        arguments: contract.arguments,
        operation: B::Library,
        type_rule: TypeRule::Fixed,
        syntax: SyntaxForm::OrdinaryCall,
        designator: DesignatorPolicy::AddressableAlias,
        arity: (contract.arguments.len(), contract.arguments.len()),
        effects: BuiltinEffectClass::Library,
        availability: Availability {
            feature: None,
            naming:  NamingPolicy::Reserved,
        },
    }
}

const fn constant_descriptor(
    id: Id,
    spelling: &'static str,
    library: Option<&'static str>,
    contract: Contract,
    scalar: S,
    nan: bool,
) -> BuiltinDescriptor {
    BuiltinDescriptor {
        library,
        operation: B::FloatConstant(scalar, nan),
        designator: if nan {
            DesignatorPolicy::AddressableAlias
        } else {
            DesignatorPolicy::DirectCallOnly
        },
        effects: if nan {
            BuiltinEffectClass::Library
        } else {
            BuiltinEffectClass::Pure
        },
        ..descriptor(id, spelling, "", contract)
    }
}

/// A constant builtin of an interchange or extended floating type, which
/// exists where the type's keyword does.
/// GNU extension: GCC manual, "Other Builtins", `__builtin_huge_valfN`,
/// `__builtin_inffN` and `__builtin_nanfN`.
/// <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
const fn interchange_constant_descriptor(
    id: Id,
    spelling: &'static str,
    library: Option<&'static str>,
    contract: Contract,
    scalar: S,
    nan: bool,
) -> BuiltinDescriptor {
    BuiltinDescriptor {
        availability: Availability {
            feature: Some(Feature::InterchangeFloatTypes),
            naming:  NamingPolicy::Reserved,
        },
        ..constant_descriptor(id, spelling, library, contract, scalar, nan)
    }
}

/// A constant builtin of `_Float16`, which exists where the type's keyword
/// does: for the GCC and Clang identities in every mode.
/// GNU extension: GCC manual, "Other Builtins"; Clang Language Extensions,
/// "Half-Precision Floating Point".
/// <https://clang.llvm.org/docs/LanguageExtensions.html#half-precision-floating-point>
const fn half_constant_descriptor(
    id: Id,
    spelling: &'static str,
    library: Option<&'static str>,
    contract: Contract,
    scalar: S,
    nan: bool,
) -> BuiltinDescriptor {
    BuiltinDescriptor {
        availability: Availability {
            feature: Some(Feature::Float16),
            naming:  NamingPolicy::Reserved,
        },
        ..constant_descriptor(id, spelling, library, contract, scalar, nan)
    }
}

#[derive(Clone, Copy)]
struct Contract {
    signature: Signature,
    arguments: &'static [A],
}

macro_rules! contract {
    ($name:ident, $result:expr; $($parameter:expr),* $(,)?) => {
        #[expect(non_upper_case_globals, reason = "Contract names read as precision and shape.")]
        const $name: Contract = Contract {
            signature: Signature {
                result: $result,
                parameters: &[$($parameter),*],
                prototype: true,
                variadic: false,
            },
            arguments: &[$(A::value(C::Parameter($parameter))),*],
        };
    };
}

contract!(DoubleNullary, T::Scalar(S::Double); );
contract!(DoubleUnary, T::Scalar(S::Double); T::Scalar(S::Double));
contract!(DoubleBinary, T::Scalar(S::Double); T::Scalar(S::Double), T::Scalar(S::Double));
contract!(DoubleTernary, T::Scalar(S::Double); T::Scalar(S::Double), T::Scalar(S::Double), T::Scalar(S::Double));
contract!(DoubleIntPointer, T::Scalar(S::Double); T::Scalar(S::Double), T::Pointer(S::Int, false));
contract!(DoubleFloatPointer, T::Scalar(S::Double); T::Scalar(S::Double), T::Pointer(S::Double, false));
contract!(DoubleRemquo, T::Scalar(S::Double); T::Scalar(S::Double), T::Scalar(S::Double), T::Pointer(S::Int, false));
contract!(DoubleIntResult, T::Scalar(S::Int); T::Scalar(S::Double));
contract!(DoubleIntExponent, T::Scalar(S::Double); T::Scalar(S::Double), T::Scalar(S::Int));
contract!(DoubleLongExponent, T::Scalar(S::Double); T::Scalar(S::Double), T::Scalar(S::Long));
contract!(DoubleLongResult, T::Scalar(S::Long); T::Scalar(S::Double));
contract!(DoubleLongLongResult, T::Scalar(S::LongLong); T::Scalar(S::Double));
contract!(DoubleNan, T::Scalar(S::Double); T::Pointer(S::Char, true));
contract!(DoubleNexttoward, T::Scalar(S::Double); T::Scalar(S::Double), T::Scalar(S::LongDouble));
contract!(FloatNullary, T::Scalar(S::Float); );
contract!(FloatUnary, T::Scalar(S::Float); T::Scalar(S::Float));
contract!(FloatBinary, T::Scalar(S::Float); T::Scalar(S::Float), T::Scalar(S::Float));
contract!(FloatTernary, T::Scalar(S::Float); T::Scalar(S::Float), T::Scalar(S::Float), T::Scalar(S::Float));
contract!(FloatIntPointer, T::Scalar(S::Float); T::Scalar(S::Float), T::Pointer(S::Int, false));
contract!(FloatFloatPointer, T::Scalar(S::Float); T::Scalar(S::Float), T::Pointer(S::Float, false));
contract!(FloatRemquo, T::Scalar(S::Float); T::Scalar(S::Float), T::Scalar(S::Float), T::Pointer(S::Int, false));
contract!(FloatIntResult, T::Scalar(S::Int); T::Scalar(S::Float));
contract!(FloatIntExponent, T::Scalar(S::Float); T::Scalar(S::Float), T::Scalar(S::Int));
contract!(FloatLongExponent, T::Scalar(S::Float); T::Scalar(S::Float), T::Scalar(S::Long));
contract!(FloatLongResult, T::Scalar(S::Long); T::Scalar(S::Float));
contract!(FloatLongLongResult, T::Scalar(S::LongLong); T::Scalar(S::Float));
contract!(FloatNan, T::Scalar(S::Float); T::Pointer(S::Char, true));
contract!(FloatNexttoward, T::Scalar(S::Float); T::Scalar(S::Float), T::Scalar(S::LongDouble));
contract!(LongDoubleNullary, T::Scalar(S::LongDouble); );
contract!(Float16Nullary, T::Scalar(S::Float16); );
contract!(Float16Nan, T::Scalar(S::Float16); T::Pointer(S::Char, true));
contract!(Float32Nullary, T::Scalar(S::Float32); );
contract!(Float32Nan, T::Scalar(S::Float32); T::Pointer(S::Char, true));
contract!(Float32xNullary, T::Scalar(S::Float32x); );
contract!(Float32xNan, T::Scalar(S::Float32x); T::Pointer(S::Char, true));
contract!(Float64Nullary, T::Scalar(S::Float64); );
contract!(Float64Nan, T::Scalar(S::Float64); T::Pointer(S::Char, true));
contract!(Float64xNullary, T::Scalar(S::Float64x); );
contract!(Float64xNan, T::Scalar(S::Float64x); T::Pointer(S::Char, true));
contract!(LongDoubleUnary, T::Scalar(S::LongDouble); T::Scalar(S::LongDouble));
contract!(LongDoubleBinary, T::Scalar(S::LongDouble); T::Scalar(S::LongDouble), T::Scalar(S::LongDouble));
contract!(LongDoubleTernary, T::Scalar(S::LongDouble); T::Scalar(S::LongDouble), T::Scalar(S::LongDouble), T::Scalar(S::LongDouble));
contract!(LongDoubleIntPointer, T::Scalar(S::LongDouble); T::Scalar(S::LongDouble), T::Pointer(S::Int, false));
contract!(LongDoubleFloatPointer, T::Scalar(S::LongDouble); T::Scalar(S::LongDouble), T::Pointer(S::LongDouble, false));
contract!(LongDoubleRemquo, T::Scalar(S::LongDouble); T::Scalar(S::LongDouble), T::Scalar(S::LongDouble), T::Pointer(S::Int, false));
contract!(LongDoubleIntResult, T::Scalar(S::Int); T::Scalar(S::LongDouble));
contract!(LongDoubleIntExponent, T::Scalar(S::LongDouble); T::Scalar(S::LongDouble), T::Scalar(S::Int));
contract!(LongDoubleLongExponent, T::Scalar(S::LongDouble); T::Scalar(S::LongDouble), T::Scalar(S::Long));
contract!(LongDoubleLongResult, T::Scalar(S::Long); T::Scalar(S::LongDouble));
contract!(LongDoubleLongLongResult, T::Scalar(S::LongLong); T::Scalar(S::LongDouble));
contract!(LongDoubleNan, T::Scalar(S::LongDouble); T::Pointer(S::Char, true));
contract!(LongDoubleNexttoward, T::Scalar(S::LongDouble); T::Scalar(S::LongDouble), T::Scalar(S::LongDouble));
