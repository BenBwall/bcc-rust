/* Resource headers chained to the fake C library under sysroot/, which
   follows glibc's patterns. Compiled hosted and freestanding with
   --sysroot tests/fixtures/hosted/sysroot. */
#include <stdio.h>
#include <wchar.h>
#include <limits.h>
#include <stdint.h>
#include <stddef.h>
#include <stdarg.h>
#include <limits.h>
#include <stdint.h>
#include <stddef.h>
#include <stdarg.h>

#define CHECK(name, condition) _Static_assert((condition), #name)

#if __STDC_HOSTED__
/* The C library's values and additions win where it has them. */
CHECK(library_mb_len_max, MB_LEN_MAX == 16);
CHECK(library_posix_limit, PATH_MAX == 4096);
CHECK(library_limits_read, FAKE_LIBC_LIMITS == 1);
CHECK(library_stdint, FAKE_LIBC_STDINT == 1);
CHECK(library_wordsize, __WORDSIZE == 64);
#else
/* Freestanding, the resource headers stand alone. */
CHECK(resource_mb_len_max, MB_LEN_MAX == 4);
#ifdef PATH_MAX
#error freestanding <limits.h> read the C library
#endif
#ifdef FAKE_LIBC_STDINT
#error freestanding <stdint.h> read the C library
#endif
#endif

/* The target's own limits replace the library's either way. */
CHECK(char_bit, CHAR_BIT == 8);
CHECK(int_max, INT_MAX == 2147483647);
CHECK(long_max, LONG_MAX == 9223372036854775807L);
CHECK(llong_max, LLONG_MAX == 9223372036854775807LL);

/* The partial <stddef.h> and <stdarg.h> requests, then the full ones. */
size_t size = sizeof(int64_t);
ptrdiff_t difference = 0;
wchar_t wide = L'w';
wint_t wide_int = 0;
int *null_pointer = NULL;
CHECK(null_is_a_pointer, sizeof NULL == sizeof(void *));
struct pair { int first, second; };
CHECK(offsetof_works, offsetof(struct pair, second) == sizeof(int));
CHECK(int64, sizeof(int64_t) == 8 && INT64_MAX == 9223372036854775807L);

int print(const char *format, ...)
{
    va_list arguments;
    __gnuc_va_list copy;
    int result;
    va_start(arguments, format);
    va_copy(copy, arguments);
    result = vprintf(format, copy);
    va_end(copy);
    va_end(arguments);
    return result + printf("%d", 1);
}
