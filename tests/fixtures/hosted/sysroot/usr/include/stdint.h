/* A fake C library header that follows glibc's <stdint.h>: its own guard,
   its own type definitions and limits, and a marker the probe tests. */
#ifndef _STDINT_H
#define _STDINT_H 1

#include <bits/wordsize.h>

#define FAKE_LIBC_STDINT 1

typedef signed char int8_t;
typedef short int int16_t;
typedef int int32_t;
typedef long int int64_t;
typedef unsigned char uint8_t;
typedef unsigned short int uint16_t;
typedef unsigned int uint32_t;
typedef unsigned long int uint64_t;

typedef signed char int_least8_t;
typedef short int int_least16_t;
typedef int int_least32_t;
typedef long int int_least64_t;
typedef unsigned char uint_least8_t;
typedef unsigned short int uint_least16_t;
typedef unsigned int uint_least32_t;
typedef unsigned long int uint_least64_t;

typedef signed char int_fast8_t;
typedef long int int_fast16_t;
typedef long int int_fast32_t;
typedef long int int_fast64_t;
typedef unsigned char uint_fast8_t;
typedef unsigned long int uint_fast16_t;
typedef unsigned long int uint_fast32_t;
typedef unsigned long int uint_fast64_t;

typedef long int intptr_t;
typedef unsigned long int uintptr_t;
typedef long int intmax_t;
typedef unsigned long int uintmax_t;

#define INT8_MAX (127)
#define INT64_MAX (9223372036854775807L)
#define UINT64_MAX (18446744073709551615UL)
#define SIZE_MAX (18446744073709551615UL)
#define INT64_C(c) c ## L
#define UINT64_C(c) c ## UL

#endif
