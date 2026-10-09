/* A fake C library header that requests wchar_t and wint_t from the
   compiler's <stddef.h>, as some C libraries do. */
#ifndef _WCHAR_H
#define _WCHAR_H 1

#define __need_size_t
#define __need_wchar_t
#define __need_wint_t
#define __need_NULL
#include <stddef.h>

extern size_t wcslen(const wchar_t *__s);
extern wint_t btowc(int __c);

#endif
