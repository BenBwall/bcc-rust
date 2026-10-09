/* C99: §5.2.4.2.1, pp. 21-23; PDF pp. 33-35. Target definitions come from src/target.rs. */
#ifndef __BCC_LIMITS_H
#define __BCC_LIMITS_H
/* A C library's <limits.h> may #include_next GCC's own, which defines
   _GCC_LIMITS_H_; claim that name so the chain ends here. */
#if defined(__GNUC__) && !defined(_GCC_LIMITS_H_)
#define _GCC_LIMITS_H_
#endif
/* Hosted, the C library's <limits.h> adds POSIX and other limits. */
#if __STDC_HOSTED__
#if __has_include_next(<limits.h>)
#include_next <limits.h>
#endif
#endif
/* The target's limits replace any that the C library defined. */
#undef CHAR_BIT
#undef SCHAR_MAX
#undef SCHAR_MIN
#undef UCHAR_MAX
#undef CHAR_MIN
#undef CHAR_MAX
#undef SHRT_MAX
#undef SHRT_MIN
#undef USHRT_MAX
#undef INT_MAX
#undef INT_MIN
#undef UINT_MAX
#undef LONG_MAX
#undef LONG_MIN
#undef ULONG_MAX
#define CHAR_BIT __CHAR_BIT__
/* The C library's value describes its multibyte encodings. Without one,
   4 bytes hold any UTF-8 character, the encoding of bcc's literals. */
#ifndef MB_LEN_MAX
#define MB_LEN_MAX @MB_LEN_MAX@
#endif
#define SCHAR_MAX __SCHAR_MAX__
#define SCHAR_MIN (-SCHAR_MAX - 1)
#define UCHAR_MAX (SCHAR_MAX * 2 + 1)
#define CHAR_MIN SCHAR_MIN
#define CHAR_MAX SCHAR_MAX
#define SHRT_MAX __SHRT_MAX__
#define SHRT_MIN (-SHRT_MAX - 1)
#define USHRT_MAX (SHRT_MAX * 2 + 1)
#define INT_MAX __INT_MAX__
#define INT_MIN (-INT_MAX - 1)
#define UINT_MAX (INT_MAX * 2U + 1U)
#define LONG_MAX __LONG_MAX__
#define LONG_MIN (-LONG_MAX - 1L)
#define ULONG_MAX (LONG_MAX * 2UL + 1UL)
#ifdef __STDC_VERSION__
#if __STDC_VERSION__ >= 199901L
#undef LLONG_MAX
#undef LLONG_MIN
#undef ULLONG_MAX
#define LLONG_MAX __LONG_LONG_MAX__
#define LLONG_MIN (-LLONG_MAX - 1LL)
#define ULLONG_MAX (LLONG_MAX * 2ULL + 1ULL)
#endif
#endif
#endif
