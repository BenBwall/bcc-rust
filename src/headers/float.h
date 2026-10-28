/* C99: §5.2.4.2.2, pp. 23-27; PDF pp. 35-39. Target definitions come from src/target.rs. */
#ifndef __BCC_FLOAT_H
#define __BCC_FLOAT_H
/* Hosted on MinGW-w64 or the MSVC runtime, the C library's <float.h> adds
   Windows definitions; the target's characteristics then replace its own. */
#if __STDC_HOSTED__
#if defined(__MINGW32__) || defined(_MSC_VER)
/* MinGW-w64's <float.h> would #include_next GCC's own, whose guard is
   _FLOAT_H___; claim that name so the chain ends here. */
#ifndef _FLOAT_H___
#define _FLOAT_H___
#endif
#if __has_include_next(<float.h>)
#include_next <float.h>
#endif
#undef FLT_RADIX
#undef FLT_ROUNDS
#undef FLT_EVAL_METHOD
#undef DECIMAL_DIG
#undef FLT_MANT_DIG
#undef FLT_DIG
#undef FLT_MIN_EXP
#undef FLT_MIN_10_EXP
#undef FLT_MAX_EXP
#undef FLT_MAX_10_EXP
#undef FLT_MAX
#undef FLT_EPSILON
#undef FLT_MIN
#undef FLT_DECIMAL_DIG
#undef FLT_TRUE_MIN
#undef FLT_HAS_SUBNORM
#undef DBL_MANT_DIG
#undef DBL_DIG
#undef DBL_MIN_EXP
#undef DBL_MIN_10_EXP
#undef DBL_MAX_EXP
#undef DBL_MAX_10_EXP
#undef DBL_MAX
#undef DBL_EPSILON
#undef DBL_MIN
#undef DBL_DECIMAL_DIG
#undef DBL_TRUE_MIN
#undef DBL_HAS_SUBNORM
#undef LDBL_MANT_DIG
#undef LDBL_DIG
#undef LDBL_MIN_EXP
#undef LDBL_MIN_10_EXP
#undef LDBL_MAX_EXP
#undef LDBL_MAX_10_EXP
#undef LDBL_MAX
#undef LDBL_EPSILON
#undef LDBL_MIN
#undef LDBL_DECIMAL_DIG
#undef LDBL_TRUE_MIN
#undef LDBL_HAS_SUBNORM
#endif
#endif
/* Clang's header gives the characteristics of later revisions outside
   strict ISO modes too (FLT_EVAL_METHOD aside); GCC's from their revision
   on, which the other identities follow. */
#if defined(__clang__) && !defined(__STRICT_ANSI__)
#define __BCC_FLOAT_ALL 1
#endif
#define FLT_RADIX __FLT_RADIX__
#define FLT_ROUNDS 1
#if defined(__STDC_VERSION__) && __STDC_VERSION__ >= 199901L
#define FLT_EVAL_METHOD 0
#endif
#if (defined(__STDC_VERSION__) && __STDC_VERSION__ >= 199901L) || defined(__BCC_FLOAT_ALL)
#define DECIMAL_DIG __LDBL_DECIMAL_DIG__
#endif
#define FLT_MANT_DIG __FLT_MANT_DIG__
#define FLT_DIG __FLT_DIG__
#define FLT_MIN_EXP __FLT_MIN_EXP__
#define FLT_MIN_10_EXP __FLT_MIN_10_EXP__
#define FLT_MAX_EXP __FLT_MAX_EXP__
#define FLT_MAX_10_EXP __FLT_MAX_10_EXP__
#define FLT_MAX __FLT_MAX__
#define FLT_EPSILON __FLT_EPSILON__
#define FLT_MIN __FLT_MIN__
#if (defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L) || defined(__BCC_FLOAT_ALL)
#define FLT_DECIMAL_DIG __FLT_DECIMAL_DIG__
#define FLT_TRUE_MIN __FLT_DENORM_MIN__
#define FLT_HAS_SUBNORM __FLT_HAS_DENORM__
#endif
#define DBL_MANT_DIG __DBL_MANT_DIG__
#define DBL_DIG __DBL_DIG__
#define DBL_MIN_EXP __DBL_MIN_EXP__
#define DBL_MIN_10_EXP __DBL_MIN_10_EXP__
#define DBL_MAX_EXP __DBL_MAX_EXP__
#define DBL_MAX_10_EXP __DBL_MAX_10_EXP__
#define DBL_MAX __DBL_MAX__
#define DBL_EPSILON __DBL_EPSILON__
#define DBL_MIN __DBL_MIN__
#if (defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L) || defined(__BCC_FLOAT_ALL)
#define DBL_DECIMAL_DIG __DBL_DECIMAL_DIG__
#define DBL_TRUE_MIN __DBL_DENORM_MIN__
#define DBL_HAS_SUBNORM __DBL_HAS_DENORM__
#endif
#define LDBL_MANT_DIG __LDBL_MANT_DIG__
#define LDBL_DIG __LDBL_DIG__
#define LDBL_MIN_EXP __LDBL_MIN_EXP__
#define LDBL_MIN_10_EXP __LDBL_MIN_10_EXP__
#define LDBL_MAX_EXP __LDBL_MAX_EXP__
#define LDBL_MAX_10_EXP __LDBL_MAX_10_EXP__
#define LDBL_MAX __LDBL_MAX__
#define LDBL_EPSILON __LDBL_EPSILON__
#define LDBL_MIN __LDBL_MIN__
#if (defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L) || defined(__BCC_FLOAT_ALL)
#define LDBL_DECIMAL_DIG __LDBL_DECIMAL_DIG__
#define LDBL_TRUE_MIN __LDBL_DENORM_MIN__
#define LDBL_HAS_SUBNORM __LDBL_HAS_DENORM__
#endif
/* C23 §5.2.5.3.3 paragraphs 28-32 (N3220, pp. 26-28): the largest
   normalized values, the signaling NaNs, and INFINITY and NAN, which
   <math.h> may have defined too; GCC also says which types are IEC 60559
   formats, from its __FLT_IS_IEC_60559__ family, and names the header's
   revision. */
#if (defined(__STDC_VERSION__) && __STDC_VERSION__ > 201710L) || defined(__BCC_FLOAT_ALL)
#define FLT_NORM_MAX __FLT_NORM_MAX__
#define DBL_NORM_MAX __DBL_NORM_MAX__
#define LDBL_NORM_MAX __LDBL_NORM_MAX__
#define FLT_SNAN (__builtin_nansf(""))
#define DBL_SNAN (__builtin_nans(""))
#define LDBL_SNAN (__builtin_nansl(""))
#undef INFINITY
#undef NAN
#define INFINITY (__builtin_inff())
#define NAN (__builtin_nanf(""))
#if defined(__FLT_IS_IEC_60559__) && __STDC_VERSION__ > 201710L
#define FLT_IS_IEC_60559 __FLT_IS_IEC_60559__
#define DBL_IS_IEC_60559 __DBL_IS_IEC_60559__
#define LDBL_IS_IEC_60559 __LDBL_IS_IEC_60559__
#define __STDC_VERSION_FLOAT_H__ 202311L
#endif
#endif
#undef __BCC_FLOAT_ALL
/* C23 §5.2.5.3.3 paragraph 21 and §F.5 (N3220, pp. 25, 517): the decimal
   digits that convert correctly rounded, which GCC's header gives under
   the IEC 60559 extension macros; Clang's has none. */
#if defined(__FLT_IS_IEC_60559__) && \
    (defined(__STDC_WANT_IEC_60559_BFP_EXT__) || defined(__STDC_WANT_IEC_60559_EXT__))
#define CR_DECIMAL_DIG __UINTMAX_MAX__
#endif
/* ISO/IEC TS 18661-3 (C23 Annex H.11): the _FloatN and _FloatNx
   characteristics, as GCC's <float.h> defines them for the types whose
   __FLTN_MANT_DIG__ the compiler predefines (_Float16 for the GCC and
   Clang identities, the others for the GCC identity and -ffloatn-types). */
#ifdef __STDC_WANT_IEC_60559_TYPES_EXT__
#ifdef __FLT16_MANT_DIG__
#define FLT16_MANT_DIG __FLT16_MANT_DIG__
#define FLT16_DIG __FLT16_DIG__
#define FLT16_MIN_EXP __FLT16_MIN_EXP__
#define FLT16_MIN_10_EXP __FLT16_MIN_10_EXP__
#define FLT16_MAX_EXP __FLT16_MAX_EXP__
#define FLT16_MAX_10_EXP __FLT16_MAX_10_EXP__
#define FLT16_MAX __FLT16_MAX__
#define FLT16_EPSILON __FLT16_EPSILON__
#define FLT16_MIN __FLT16_MIN__
#define FLT16_DECIMAL_DIG __FLT16_DECIMAL_DIG__
#define FLT16_TRUE_MIN __FLT16_DENORM_MIN__
#endif
#ifdef __FLT32_MANT_DIG__
#define FLT32_MANT_DIG __FLT32_MANT_DIG__
#define FLT32_DIG __FLT32_DIG__
#define FLT32_MIN_EXP __FLT32_MIN_EXP__
#define FLT32_MIN_10_EXP __FLT32_MIN_10_EXP__
#define FLT32_MAX_EXP __FLT32_MAX_EXP__
#define FLT32_MAX_10_EXP __FLT32_MAX_10_EXP__
#define FLT32_MAX __FLT32_MAX__
#define FLT32_EPSILON __FLT32_EPSILON__
#define FLT32_MIN __FLT32_MIN__
#define FLT32_DECIMAL_DIG __FLT32_DECIMAL_DIG__
#define FLT32_TRUE_MIN __FLT32_DENORM_MIN__
#if defined(__STDC_VERSION__) && __STDC_VERSION__ > 201710L
#define FLT32_SNAN (__builtin_nansf32(""))
#endif
#endif
#ifdef __FLT64_MANT_DIG__
#define FLT64_MANT_DIG __FLT64_MANT_DIG__
#define FLT64_DIG __FLT64_DIG__
#define FLT64_MIN_EXP __FLT64_MIN_EXP__
#define FLT64_MIN_10_EXP __FLT64_MIN_10_EXP__
#define FLT64_MAX_EXP __FLT64_MAX_EXP__
#define FLT64_MAX_10_EXP __FLT64_MAX_10_EXP__
#define FLT64_MAX __FLT64_MAX__
#define FLT64_EPSILON __FLT64_EPSILON__
#define FLT64_MIN __FLT64_MIN__
#define FLT64_DECIMAL_DIG __FLT64_DECIMAL_DIG__
#define FLT64_TRUE_MIN __FLT64_DENORM_MIN__
#if defined(__STDC_VERSION__) && __STDC_VERSION__ > 201710L
#define FLT64_SNAN (__builtin_nansf64(""))
#endif
#endif
#ifdef __FLT128_MANT_DIG__
#define FLT128_MANT_DIG __FLT128_MANT_DIG__
#define FLT128_DIG __FLT128_DIG__
#define FLT128_MIN_EXP __FLT128_MIN_EXP__
#define FLT128_MIN_10_EXP __FLT128_MIN_10_EXP__
#define FLT128_MAX_EXP __FLT128_MAX_EXP__
#define FLT128_MAX_10_EXP __FLT128_MAX_10_EXP__
#define FLT128_MAX __FLT128_MAX__
#define FLT128_EPSILON __FLT128_EPSILON__
#define FLT128_MIN __FLT128_MIN__
#define FLT128_DECIMAL_DIG __FLT128_DECIMAL_DIG__
#define FLT128_TRUE_MIN __FLT128_DENORM_MIN__
#if defined(__STDC_VERSION__) && __STDC_VERSION__ > 201710L
#define FLT128_SNAN (__builtin_nansf128(""))
#endif
#endif
#ifdef __FLT32X_MANT_DIG__
#define FLT32X_MANT_DIG __FLT32X_MANT_DIG__
#define FLT32X_DIG __FLT32X_DIG__
#define FLT32X_MIN_EXP __FLT32X_MIN_EXP__
#define FLT32X_MIN_10_EXP __FLT32X_MIN_10_EXP__
#define FLT32X_MAX_EXP __FLT32X_MAX_EXP__
#define FLT32X_MAX_10_EXP __FLT32X_MAX_10_EXP__
#define FLT32X_MAX __FLT32X_MAX__
#define FLT32X_EPSILON __FLT32X_EPSILON__
#define FLT32X_MIN __FLT32X_MIN__
#define FLT32X_DECIMAL_DIG __FLT32X_DECIMAL_DIG__
#define FLT32X_TRUE_MIN __FLT32X_DENORM_MIN__
#if defined(__STDC_VERSION__) && __STDC_VERSION__ > 201710L
#define FLT32X_SNAN (__builtin_nansf32x(""))
#endif
#endif
#ifdef __FLT64X_MANT_DIG__
#define FLT64X_MANT_DIG __FLT64X_MANT_DIG__
#define FLT64X_DIG __FLT64X_DIG__
#define FLT64X_MIN_EXP __FLT64X_MIN_EXP__
#define FLT64X_MIN_10_EXP __FLT64X_MIN_10_EXP__
#define FLT64X_MAX_EXP __FLT64X_MAX_EXP__
#define FLT64X_MAX_10_EXP __FLT64X_MAX_10_EXP__
#define FLT64X_MAX __FLT64X_MAX__
#define FLT64X_EPSILON __FLT64X_EPSILON__
#define FLT64X_MIN __FLT64X_MIN__
#define FLT64X_DECIMAL_DIG __FLT64X_DECIMAL_DIG__
#define FLT64X_TRUE_MIN __FLT64X_DENORM_MIN__
#if defined(__STDC_VERSION__) && __STDC_VERSION__ > 201710L
#define FLT64X_SNAN (__builtin_nansf64x(""))
#endif
#endif
#endif
#endif
