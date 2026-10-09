/* A fake C library header that follows glibc's <stdio.h>: it requests
   subsets of the compiler's <stddef.h> and <stdarg.h> with __need_ macros
   and declares functions with __gnuc_va_list. */
#ifndef _STDIO_H
#define _STDIO_H 1

#define __need_size_t
#define __need_NULL
#include <stddef.h>

#define __need___va_list
#include <stdarg.h>

typedef struct _IO_FILE FILE;
typedef __gnuc_va_list fake_libc_va_list;

extern int printf(const char *__restrict __format, ...);
extern int vprintf(const char *__restrict __format, __gnuc_va_list __arg);
extern size_t fwrite(const void *__restrict __ptr, size_t __size, size_t __n,
                     FILE *__restrict __s);

#endif
