/* bcc's bundled C library for wasm32-wasip1: <stdlib.h>. */
#ifndef _BCC_WASI_STDLIB_H
#define _BCC_WASI_STDLIB_H
#include <stddef.h>

#define EXIT_SUCCESS 0
#define EXIT_FAILURE 1
#define RAND_MAX 2147483647
#define MB_CUR_MAX 1

typedef struct { int quot, rem; } div_t;
typedef struct { long quot, rem; } ldiv_t;
typedef struct { long long quot, rem; } lldiv_t;

void *malloc(size_t size);
void *calloc(size_t count, size_t size);
void *realloc(void *ptr, size_t size);
void *aligned_alloc(size_t alignment, size_t size);
void free(void *ptr);

_Noreturn void abort(void);
_Noreturn void exit(int status);
_Noreturn void _Exit(int status);
int atexit(void (*function)(void));
char *getenv(const char *name);
int system(const char *command);

int abs(int j);
long labs(long j);
long long llabs(long long j);
div_t div(int numer, int denom);
ldiv_t ldiv(long numer, long denom);
lldiv_t lldiv(long long numer, long long denom);

int atoi(const char *nptr);
long atol(const char *nptr);
long long atoll(const char *nptr);
double atof(const char *nptr);
/* C23 reads a 0b prefix in base 0 and 2. As glibc's headers do, C23 and
   _GNU_SOURCE call the C23 functions by their own names. */
#if defined _GNU_SOURCE || defined _ISOC2X_SOURCE || defined _ISOC23_SOURCE || \
    defined _ISOC2Y_SOURCE || (defined __STDC_VERSION__ && __STDC_VERSION__ > 201710L)
long strtol(const char *restrict nptr, char **restrict endptr, int base) __asm__("__isoc23_strtol");
unsigned long strtoul(const char *restrict nptr, char **restrict endptr, int base) __asm__("__isoc23_strtoul");
long long strtoll(const char *restrict nptr, char **restrict endptr, int base) __asm__("__isoc23_strtoll");
unsigned long long strtoull(const char *restrict nptr, char **restrict endptr, int base)
    __asm__("__isoc23_strtoull");
#else
long strtol(const char *restrict nptr, char **restrict endptr, int base);
unsigned long strtoul(const char *restrict nptr, char **restrict endptr, int base);
long long strtoll(const char *restrict nptr, char **restrict endptr, int base);
unsigned long long strtoull(const char *restrict nptr, char **restrict endptr, int base);
#endif
double strtod(const char *restrict nptr, char **restrict endptr);
float strtof(const char *restrict nptr, char **restrict endptr);
long double strtold(const char *restrict nptr, char **restrict endptr);

int rand(void);
void srand(unsigned int seed);
void qsort(void *base, size_t nmemb, size_t size, int (*compar)(const void *, const void *));
void *bsearch(const void *key, const void *base, size_t nmemb, size_t size,
              int (*compar)(const void *, const void *));

#endif
