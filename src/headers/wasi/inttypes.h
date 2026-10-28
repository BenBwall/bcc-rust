/* bcc's bundled C library for wasm32-wasip1: <inttypes.h> (ILP32). */
#ifndef _BCC_WASI_INTTYPES_H
#define _BCC_WASI_INTTYPES_H
#include <stdint.h>
#define PRId8 "d"
#define PRId16 "d"
#define PRId32 "d"
#define PRId64 "lld"
#define PRIi8 "i"
#define PRIi16 "i"
#define PRIi32 "i"
#define PRIi64 "lli"
#define PRIu8 "u"
#define PRIu16 "u"
#define PRIu32 "u"
#define PRIu64 "llu"
#define PRIx8 "x"
#define PRIx16 "x"
#define PRIx32 "x"
#define PRIx64 "llx"
#define PRIX8 "X"
#define PRIX16 "X"
#define PRIX32 "X"
#define PRIX64 "llX"
#define PRIo32 "o"
#define PRIo64 "llo"
#define PRIdMAX "lld"
#define PRIuMAX "llu"
#define PRIxMAX "llx"
#define PRIdPTR "ld"
#define PRIuPTR "lu"
#define PRIxPTR "lx"
#define SCNd32 "d"
#define SCNd64 "lld"
#define SCNu32 "u"
#define SCNu64 "llu"
typedef struct { intmax_t quot, rem; } imaxdiv_t;
intmax_t imaxabs(intmax_t j);
/* C23 reads a 0b prefix in base 0 and 2 (see <stdlib.h>). */
#if defined _GNU_SOURCE || defined _ISOC2X_SOURCE || defined _ISOC23_SOURCE || \
    (defined __STDC_VERSION__ && __STDC_VERSION__ > 201710L)
intmax_t strtoimax(const char *restrict nptr, char **restrict endptr, int base) __asm__("__isoc23_strtoimax");
uintmax_t strtoumax(const char *restrict nptr, char **restrict endptr, int base) __asm__("__isoc23_strtoumax");
#else
intmax_t strtoimax(const char *restrict nptr, char **restrict endptr, int base);
uintmax_t strtoumax(const char *restrict nptr, char **restrict endptr, int base);
#endif
#endif
