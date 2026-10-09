/* A fake C library header that follows glibc's <limits.h>: its own guard,
   MB_LEN_MAX 16, POSIX additions, ISO limits only without GCC, and an
   #include_next of GCC's <limits.h> unless _GCC_LIMITS_H_ says it was read. */
#ifndef _LIBC_LIMITS_H_
#define _LIBC_LIMITS_H_ 1

#define MB_LEN_MAX 16
#define PATH_MAX 4096
#define FAKE_LIBC_LIMITS 1

/* glibc writes `!defined __GNUC__ || __GNUC__ < 2`; bcc warns about the
   undefined name in strict modes, so the test is split. */
#if !defined __GNUC__
# define __FAKE_LIBC_ISO_LIMITS
#elif __GNUC__ < 2
# define __FAKE_LIBC_ISO_LIMITS
#endif
#ifdef __FAKE_LIBC_ISO_LIMITS
# ifndef _LIMITS_H
#  define _LIMITS_H 1
#  define CHAR_BIT 8
#  define SCHAR_MIN (-128)
#  define SCHAR_MAX 127
#  define UCHAR_MAX 255
#  define CHAR_MIN SCHAR_MIN
#  define CHAR_MAX SCHAR_MAX
#  define SHRT_MIN (-32768)
#  define SHRT_MAX 32767
#  define USHRT_MAX 65535
#  define INT_MIN (-INT_MAX - 1)
#  define INT_MAX 2147483647
#  define UINT_MAX 4294967295U
#  define LONG_MAX 9223372036854775807L
#  define LONG_MIN (-LONG_MAX - 1L)
#  define ULONG_MAX 18446744073709551615UL
# endif
#endif

#endif

#if defined __GNUC__ && !defined _GCC_LIMITS_H_
# include_next <limits.h>
#endif
