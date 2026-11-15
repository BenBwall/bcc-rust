/* C99: §7.18, pp. 255-261; PDF pp. 267-273. Target definitions come from src/target.rs. */
#ifndef __BCC_STDINT_H
#define __BCC_STDINT_H
/* Hosted, the C library's <stdint.h> provides every definition, with any
   additions of its own. */
#if __STDC_HOSTED__
#if __has_include_next(<stdint.h>)
#include_next <stdint.h>
#define __BCC_STDINT_FROM_LIBRARY
#endif
#endif
#ifndef __BCC_STDINT_FROM_LIBRARY
#ifndef __INT8_TYPE__
/* bcc: TCC 0.9.27 predefines none of the target's type macros, which only
   the TCC identity on x86-64 Linux leaves out, so there this header spells
   the LP64 types out from those TCC does predefine, as TCC's own <stddef.h>
   spells out its exact-width types. They are the types the C library's
   <stdint.h> gives TCC hosted, which GCC's macros name too. The libraries
   differ only in int_fast16_t and int_fast32_t (glibc's long, musl's int),
   so those come from the fast-type macros of GCC's listing for the target,
   which the identity keeps for this header. */
typedef signed char int8_t;
#define INT8_MAX 127
#define INT8_MIN (-INT8_MAX - 1)
typedef unsigned char uint8_t;
#define UINT8_MAX 255
typedef short int16_t;
#define INT16_MAX 32767
#define INT16_MIN (-INT16_MAX - 1)
typedef unsigned short uint16_t;
#define UINT16_MAX 65535
typedef int int32_t;
#define INT32_MAX 2147483647
#define INT32_MIN (-INT32_MAX - 1)
typedef unsigned int uint32_t;
#define UINT32_MAX 4294967295U
typedef long int64_t;
#define INT64_MAX 9223372036854775807L
#define INT64_MIN (-INT64_MAX - 1)
typedef unsigned long uint64_t;
#define UINT64_MAX 18446744073709551615UL
typedef int8_t int_least8_t;
#define INT_LEAST8_MAX INT8_MAX
#define INT_LEAST8_MIN INT8_MIN
typedef uint8_t uint_least8_t;
#define UINT_LEAST8_MAX UINT8_MAX
typedef int16_t int_least16_t;
#define INT_LEAST16_MAX INT16_MAX
#define INT_LEAST16_MIN INT16_MIN
typedef uint16_t uint_least16_t;
#define UINT_LEAST16_MAX UINT16_MAX
typedef int32_t int_least32_t;
#define INT_LEAST32_MAX INT32_MAX
#define INT_LEAST32_MIN INT32_MIN
typedef uint32_t uint_least32_t;
#define UINT_LEAST32_MAX UINT32_MAX
typedef int64_t int_least64_t;
#define INT_LEAST64_MAX INT64_MAX
#define INT_LEAST64_MIN INT64_MIN
typedef uint64_t uint_least64_t;
#define UINT_LEAST64_MAX UINT64_MAX
typedef int8_t int_fast8_t;
#define INT_FAST8_MAX INT8_MAX
#define INT_FAST8_MIN INT8_MIN
typedef uint8_t uint_fast8_t;
#define UINT_FAST8_MAX UINT8_MAX
typedef __INT_FAST16_TYPE__ int_fast16_t;
#define INT_FAST16_MAX __INT_FAST16_MAX__
#define INT_FAST16_MIN (-INT_FAST16_MAX - 1)
typedef __UINT_FAST16_TYPE__ uint_fast16_t;
#define UINT_FAST16_MAX __UINT_FAST16_MAX__
typedef __INT_FAST32_TYPE__ int_fast32_t;
#define INT_FAST32_MAX __INT_FAST32_MAX__
#define INT_FAST32_MIN (-INT_FAST32_MAX - 1)
typedef __UINT_FAST32_TYPE__ uint_fast32_t;
#define UINT_FAST32_MAX __UINT_FAST32_MAX__
typedef int64_t int_fast64_t;
#define INT_FAST64_MAX INT64_MAX
#define INT_FAST64_MIN INT64_MIN
typedef uint64_t uint_fast64_t;
#define UINT_FAST64_MAX UINT64_MAX
typedef __PTRDIFF_TYPE__ intptr_t;
#define INTPTR_MAX INT64_MAX
#define INTPTR_MIN INT64_MIN
typedef __SIZE_TYPE__ uintptr_t;
#define UINTPTR_MAX UINT64_MAX
typedef int64_t intmax_t;
#define INTMAX_MAX INT64_MAX
#define INTMAX_MIN INT64_MIN
typedef uint64_t uintmax_t;
#define UINTMAX_MAX UINT64_MAX
#define PTRDIFF_MAX INT64_MAX
#define PTRDIFF_MIN INT64_MIN
#define SIG_ATOMIC_MAX INT32_MAX
#define SIG_ATOMIC_MIN INT32_MIN
#define WCHAR_MAX INT32_MAX
#define WCHAR_MIN INT32_MIN
#define WINT_MAX UINT32_MAX
#define WINT_MIN 0U
#define SIZE_MAX UINT64_MAX
#define INT8_C(c) c
#define INT16_C(c) c
#define INT32_C(c) c
#define INT64_C(c) c##L
#define UINT8_C(c) c
#define UINT16_C(c) c
#define UINT32_C(c) c##U
#define UINT64_C(c) c##UL
#define INTMAX_C(c) c##L
#define UINTMAX_C(c) c##UL
#else
typedef __INT8_TYPE__ int8_t;
#define INT8_MAX __INT8_MAX__
#define INT8_MIN (-INT8_MAX - 1)
typedef __UINT8_TYPE__ uint8_t;
#define UINT8_MAX __UINT8_MAX__
typedef __INT16_TYPE__ int16_t;
#define INT16_MAX __INT16_MAX__
#define INT16_MIN (-INT16_MAX - 1)
typedef __UINT16_TYPE__ uint16_t;
#define UINT16_MAX __UINT16_MAX__
typedef __INT32_TYPE__ int32_t;
#define INT32_MAX __INT32_MAX__
#define INT32_MIN (-INT32_MAX - 1)
typedef __UINT32_TYPE__ uint32_t;
#define UINT32_MAX __UINT32_MAX__
typedef __INT64_TYPE__ int64_t;
#define INT64_MAX __INT64_MAX__
#define INT64_MIN (-INT64_MAX - 1)
typedef __UINT64_TYPE__ uint64_t;
#define UINT64_MAX __UINT64_MAX__
typedef __INT_LEAST8_TYPE__ int_least8_t;
#define INT_LEAST8_MAX __INT_LEAST8_MAX__
#define INT_LEAST8_MIN (-INT_LEAST8_MAX - 1)
typedef __UINT_LEAST8_TYPE__ uint_least8_t;
#define UINT_LEAST8_MAX __UINT_LEAST8_MAX__
typedef __INT_LEAST16_TYPE__ int_least16_t;
#define INT_LEAST16_MAX __INT_LEAST16_MAX__
#define INT_LEAST16_MIN (-INT_LEAST16_MAX - 1)
typedef __UINT_LEAST16_TYPE__ uint_least16_t;
#define UINT_LEAST16_MAX __UINT_LEAST16_MAX__
typedef __INT_LEAST32_TYPE__ int_least32_t;
#define INT_LEAST32_MAX __INT_LEAST32_MAX__
#define INT_LEAST32_MIN (-INT_LEAST32_MAX - 1)
typedef __UINT_LEAST32_TYPE__ uint_least32_t;
#define UINT_LEAST32_MAX __UINT_LEAST32_MAX__
typedef __INT_LEAST64_TYPE__ int_least64_t;
#define INT_LEAST64_MAX __INT_LEAST64_MAX__
#define INT_LEAST64_MIN (-INT_LEAST64_MAX - 1)
typedef __UINT_LEAST64_TYPE__ uint_least64_t;
#define UINT_LEAST64_MAX __UINT_LEAST64_MAX__
typedef __INT_FAST8_TYPE__ int_fast8_t;
#define INT_FAST8_MAX __INT_FAST8_MAX__
#define INT_FAST8_MIN (-INT_FAST8_MAX - 1)
typedef __UINT_FAST8_TYPE__ uint_fast8_t;
#define UINT_FAST8_MAX __UINT_FAST8_MAX__
typedef __INT_FAST16_TYPE__ int_fast16_t;
#define INT_FAST16_MAX __INT_FAST16_MAX__
#define INT_FAST16_MIN (-INT_FAST16_MAX - 1)
typedef __UINT_FAST16_TYPE__ uint_fast16_t;
#define UINT_FAST16_MAX __UINT_FAST16_MAX__
typedef __INT_FAST32_TYPE__ int_fast32_t;
#define INT_FAST32_MAX __INT_FAST32_MAX__
#define INT_FAST32_MIN (-INT_FAST32_MAX - 1)
typedef __UINT_FAST32_TYPE__ uint_fast32_t;
#define UINT_FAST32_MAX __UINT_FAST32_MAX__
typedef __INT_FAST64_TYPE__ int_fast64_t;
#define INT_FAST64_MAX __INT_FAST64_MAX__
#define INT_FAST64_MIN (-INT_FAST64_MAX - 1)
typedef __UINT_FAST64_TYPE__ uint_fast64_t;
#define UINT_FAST64_MAX __UINT_FAST64_MAX__
typedef __INTPTR_TYPE__ intptr_t;
#define INTPTR_MAX __INTPTR_MAX__
#define INTPTR_MIN (-INTPTR_MAX - 1)
typedef __UINTPTR_TYPE__ uintptr_t;
#define UINTPTR_MAX __UINTPTR_MAX__
typedef __INTMAX_TYPE__ intmax_t;
#define INTMAX_MAX __INTMAX_MAX__
#define INTMAX_MIN (-INTMAX_MAX - 1)
typedef __UINTMAX_TYPE__ uintmax_t;
#define UINTMAX_MAX __UINTMAX_MAX__
#define PTRDIFF_MAX __PTRDIFF_MAX__
#define PTRDIFF_MIN (-PTRDIFF_MAX - 1)
#define SIG_ATOMIC_MAX __SIG_ATOMIC_MAX__
#define SIG_ATOMIC_MIN (-SIG_ATOMIC_MAX - 1)
#define WCHAR_MAX __WCHAR_MAX__
#define WCHAR_MIN __WCHAR_MIN__
#define WINT_MAX __WINT_MAX__
#define WINT_MIN __WINT_MIN__
#define SIZE_MAX __SIZE_MAX__
#define INT8_C(c) __INT8_C(c)
#define INT16_C(c) __INT16_C(c)
#define INT32_C(c) __INT32_C(c)
#define INT64_C(c) __INT64_C(c)
#define UINT8_C(c) __UINT8_C(c)
#define UINT16_C(c) __UINT16_C(c)
#define UINT32_C(c) __UINT32_C(c)
#define UINT64_C(c) __UINT64_C(c)
#define INTMAX_C(c) __INTMAX_C(c)
#define UINTMAX_C(c) __UINTMAX_C(c)
#endif
#endif
#endif
