/* C99: §7.15, pp. 249-252; PDF pp. 261-264. Target definitions come from src/target.rs. */
/* Like GCC's and Clang's, this header may be included repeatedly. With any
   __need_ macro defined it provides only the requested subset; glibc's
   <stdio.h>, for example, requests __gnuc_va_list with __need___va_list.
   Otherwise it provides everything. */
#if !defined(__need___va_list) && !defined(__need_va_list) &&                  \
    !defined(__need_va_arg) && !defined(__need___va_copy) &&                   \
    !defined(__need_va_copy)
#define __need___va_list
#define __need_va_list
#define __need_va_arg
#define __need___va_copy
#define __need_va_copy
#define __BCC_STDARG_H
#endif
#ifdef __need___va_list
#ifndef __GNUC_VA_LIST
#define __GNUC_VA_LIST
typedef __builtin_va_list __gnuc_va_list;
#endif
#undef __need___va_list
#endif
#ifdef __need_va_list
#ifndef _VA_LIST
#define _VA_LIST
typedef __builtin_va_list va_list;
#endif
#undef __need_va_list
#endif
#ifdef __need_va_arg
#ifndef va_arg
/* C23 §7.16.1.4p1,p4 (N3220 p. 290; PDF p. 303): va_start takes only ap,
   and further arguments are not evaluated. GCC 15 and Clang choose this
   form after C17 too. GCC accepts any number of further arguments; Clang's
   builtin takes at most one, so under Clang a third is an error. */
#if defined(__STDC_VERSION__) && __STDC_VERSION__ > 201710L
#ifdef __clang__
#define __bcc_va_start_1(ap) __builtin_va_start(ap, 0)
#define __bcc_va_start_2(ap, last) __builtin_va_start(ap, 0)
#define __bcc_va_start_more(...) __builtin_va_start(__VA_ARGS__)
#define __bcc_va_start_select(_1, _2, _3, _4, _5, _6, _7, _8, name, ...) name
#define va_start(...)                                                          \
  __bcc_va_start_select(__VA_ARGS__, __bcc_va_start_more, __bcc_va_start_more, \
                        __bcc_va_start_more, __bcc_va_start_more,              \
                        __bcc_va_start_more, __bcc_va_start_more,              \
                        __bcc_va_start_2, __bcc_va_start_1, )(__VA_ARGS__)
#else
#define va_start(ap, ...) __builtin_va_start(ap, 0)
#endif
#else
#define va_start(ap, last) __builtin_va_start(ap, last)
#endif
#define va_arg(ap, type) __builtin_va_arg(ap, type)
#define va_end(ap) __builtin_va_end(ap)
#endif
#undef __need_va_arg
#endif
#ifdef __need___va_copy
#ifndef __va_copy
#define __va_copy(dest, src) __builtin_va_copy(dest, src)
#endif
#undef __need___va_copy
#endif
#ifdef __need_va_copy
#ifndef va_copy
#define va_copy(dest, src) __builtin_va_copy(dest, src)
#endif
#undef __need_va_copy
#endif
