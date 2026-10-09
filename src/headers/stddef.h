/* C99: §7.17, p. 254; PDF p. 266. Target definitions come from src/target.rs. */
#ifndef __BCC_STDDEF_H
#define __BCC_STDDEF_H
typedef __SIZE_TYPE__ size_t;
typedef __PTRDIFF_TYPE__ ptrdiff_t;
typedef __WCHAR_TYPE__ wchar_t;
#define NULL ((void *)0)
#define offsetof(type, member) __builtin_offsetof(type, member)
#ifdef __STDC_VERSION__
#if __STDC_VERSION__ >= 201112L
typedef struct { long long __integer; long double __floating; } max_align_t;
#endif
#endif
#endif
