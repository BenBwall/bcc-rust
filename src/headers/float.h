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
#define FLT_RADIX __FLT_RADIX__
#define FLT_ROUNDS 1
#define FLT_EVAL_METHOD 0
#define DECIMAL_DIG __LDBL_DECIMAL_DIG__
#define FLT_MANT_DIG __FLT_MANT_DIG__
#define FLT_DIG __FLT_DIG__
#define FLT_MIN_EXP __FLT_MIN_EXP__
#define FLT_MIN_10_EXP __FLT_MIN_10_EXP__
#define FLT_MAX_EXP __FLT_MAX_EXP__
#define FLT_MAX_10_EXP __FLT_MAX_10_EXP__
#define FLT_MAX __FLT_MAX__
#define FLT_EPSILON __FLT_EPSILON__
#define FLT_MIN __FLT_MIN__
#ifdef __STDC_VERSION__
#if __STDC_VERSION__ >= 201112L
#define FLT_DECIMAL_DIG __FLT_DECIMAL_DIG__
#define FLT_TRUE_MIN __FLT_DENORM_MIN__
#define FLT_HAS_SUBNORM __FLT_HAS_DENORM__
#endif
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
#ifdef __STDC_VERSION__
#if __STDC_VERSION__ >= 201112L
#define DBL_DECIMAL_DIG __DBL_DECIMAL_DIG__
#define DBL_TRUE_MIN __DBL_DENORM_MIN__
#define DBL_HAS_SUBNORM __DBL_HAS_DENORM__
#endif
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
#ifdef __STDC_VERSION__
#if __STDC_VERSION__ >= 201112L
#define LDBL_DECIMAL_DIG __LDBL_DECIMAL_DIG__
#define LDBL_TRUE_MIN __LDBL_DENORM_MIN__
#define LDBL_HAS_SUBNORM __LDBL_HAS_DENORM__
#endif
#endif
#endif
