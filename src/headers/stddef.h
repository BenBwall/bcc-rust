/* C99: §7.17, p. 254; PDF p. 266. Target definitions come from src/target.rs. */
/* Like GCC's and Clang's, this header may be included repeatedly. With any
   __need_ macro defined it provides only the requested subset, the protocol
   C library headers such as glibc's use; otherwise it provides everything.
   Each type has the conventional guard, so a C library's own definition is
   kept. */
#if !defined(__need_ptrdiff_t) && !defined(__need_size_t) &&                  \
    !defined(__need_rsize_t) && !defined(__need_wchar_t) &&                   \
    !defined(__need_NULL) && !defined(__need_max_align_t) &&                   \
    !defined(__need_offsetof) && !defined(__need_wint_t)
#define __need_ptrdiff_t
#define __need_size_t
#ifdef __STDC_WANT_LIB_EXT1__
#if __STDC_WANT_LIB_EXT1__ >= 1
#define __need_rsize_t
#endif
#endif
#define __need_wchar_t
/* NULL is redefined only by the first complete inclusion, or on request. */
#ifndef __BCC_STDDEF_H
#define __need_NULL
#endif
#ifdef __STDC_VERSION__
#if __STDC_VERSION__ >= 201112L
#define __need_max_align_t
#endif
#endif
#define __need_offsetof
#define __BCC_STDDEF_H
#endif
#ifdef __need_ptrdiff_t
#ifndef _PTRDIFF_T
#define _PTRDIFF_T
typedef __PTRDIFF_TYPE__ ptrdiff_t;
#endif
#undef __need_ptrdiff_t
#endif
#ifdef __need_size_t
#ifndef _SIZE_T
#define _SIZE_T
typedef __SIZE_TYPE__ size_t;
#endif
#undef __need_size_t
#endif
#ifdef __need_rsize_t
#ifndef _RSIZE_T
#define _RSIZE_T
typedef __SIZE_TYPE__ rsize_t;
#endif
#undef __need_rsize_t
#endif
#ifdef __need_wchar_t
#ifndef _WCHAR_T
#define _WCHAR_T
/* As in Clang, MSVC's own headers then keep this definition. */
#ifdef _MSC_EXTENSIONS
#define _WCHAR_T_DEFINED
#endif
typedef __WCHAR_TYPE__ wchar_t;
#endif
#undef __need_wchar_t
#endif
/* A system header may have defined NULL as 0; requesting it restores the
   null pointer constant. */
#ifdef __need_NULL
#undef NULL
#define NULL ((void *)0)
#undef __need_NULL
#endif
#ifdef __need_max_align_t
#ifndef __BCC_MAX_ALIGN_T_DEFINED
#define __BCC_MAX_ALIGN_T_DEFINED
/* Clang's choices: MSVC's `double`, otherwise GCC's record. */
#ifdef _MSC_VER
typedef double max_align_t;
#else
typedef struct { long long __integer; long double __floating; } max_align_t;
#endif
#endif
#undef __need_max_align_t
#endif
#ifdef __need_offsetof
#ifndef offsetof
#define offsetof(type, member) __builtin_offsetof(type, member)
#endif
#undef __need_offsetof
#endif
/* wint_t belongs to <wchar.h>; some C libraries request it here. */
#ifdef __need_wint_t
#ifndef _WINT_T
#define _WINT_T
typedef __WINT_TYPE__ wint_t;
#endif
#undef __need_wint_t
#endif
