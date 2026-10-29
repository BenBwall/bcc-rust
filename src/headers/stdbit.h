/* C23: 7.18, pp. 304-311; PDF pp. 317-324 (N3220). */
/* A C library's <stdbit.h> (glibc 2.39 and later) provides every
   definition, also in a freestanding translation, as GCC and Clang find
   it; its type-generic macros use the compiler's type-generic builtins
   where it has them, as bcc does. The Clang identities find only that
   header (headers::provides), as Clang ships none. */
#if __has_include_next(<stdbit.h>)
#include_next <stdbit.h>
#endif
/* Otherwise every function is also a macro over its bcc builtin. This
   text is also the <stdbit.h> of bcc's WASI C library, which defines the
   functions; on other targets a C library that lacks this header must,
   for a call that suppresses the macro. The library's header, or this
   one, defines __STDC_VERSION_STDBIT_H__ once. */
#ifndef __STDC_VERSION_STDBIT_H__
#define __need_size_t
#include <stddef.h>
#include <stdint.h>

#define __STDC_VERSION_STDBIT_H__ 202311L

/* The values glibc gives these macros. Every bcc target is little-endian. */
#define __STDC_ENDIAN_LITTLE__ 1234
#define __STDC_ENDIAN_BIG__ 4321
#if defined(__BYTE_ORDER__) && defined(__ORDER_BIG_ENDIAN__) &&               \
    __BYTE_ORDER__ == __ORDER_BIG_ENDIAN__
#define __STDC_ENDIAN_NATIVE__ __STDC_ENDIAN_BIG__
#else
#define __STDC_ENDIAN_NATIVE__ __STDC_ENDIAN_LITTLE__
#endif

/* Declares a function for each standard unsigned integer type, of result
   `result`, or of that type where `result` is empty, and defines each as a
   macro over the type-generic builtin. */
#define __BCC_STDBIT(name, result)                                             \
  __BCC_STDBIT_ONE(name, _uc, unsigned char, result)                           \
  __BCC_STDBIT_ONE(name, _us, unsigned short, result)                          \
  __BCC_STDBIT_ONE(name, _ui, unsigned int, result)                            \
  __BCC_STDBIT_ONE(name, _ul, unsigned long, result)                           \
  __BCC_STDBIT_ONE(name, _ull, unsigned long long, result)
#define __BCC_STDBIT_ONE(name, suffix, type, result)                           \
  extern __BCC_STDBIT_RESULT(result, type) stdc_##name##suffix(type __x);
#define __BCC_STDBIT_RESULT(result, type) __BCC_STDBIT_RESULT_##result(type)
#define __BCC_STDBIT_RESULT_count(type) unsigned int
#define __BCC_STDBIT_RESULT_bool(type) _Bool
#define __BCC_STDBIT_RESULT_value(type) type

__BCC_STDBIT(leading_zeros, count)
__BCC_STDBIT(leading_ones, count)
__BCC_STDBIT(trailing_zeros, count)
__BCC_STDBIT(trailing_ones, count)
__BCC_STDBIT(first_leading_zero, count)
__BCC_STDBIT(first_leading_one, count)
__BCC_STDBIT(first_trailing_zero, count)
__BCC_STDBIT(first_trailing_one, count)
__BCC_STDBIT(count_zeros, count)
__BCC_STDBIT(count_ones, count)
__BCC_STDBIT(has_single_bit, bool)
__BCC_STDBIT(bit_width, count)
__BCC_STDBIT(bit_floor, value)
__BCC_STDBIT(bit_ceil, value)

#undef __BCC_STDBIT
#undef __BCC_STDBIT_ONE
#undef __BCC_STDBIT_RESULT
#undef __BCC_STDBIT_RESULT_count
#undef __BCC_STDBIT_RESULT_bool
#undef __BCC_STDBIT_RESULT_value

#define stdc_leading_zeros(x) (__builtin_stdc_leading_zeros(x))
#define stdc_leading_ones(x) (__builtin_stdc_leading_ones(x))
#define stdc_trailing_zeros(x) (__builtin_stdc_trailing_zeros(x))
#define stdc_trailing_ones(x) (__builtin_stdc_trailing_ones(x))
#define stdc_first_leading_zero(x) (__builtin_stdc_first_leading_zero(x))
#define stdc_first_leading_one(x) (__builtin_stdc_first_leading_one(x))
#define stdc_first_trailing_zero(x) (__builtin_stdc_first_trailing_zero(x))
#define stdc_first_trailing_one(x) (__builtin_stdc_first_trailing_one(x))
#define stdc_count_zeros(x) (__builtin_stdc_count_zeros(x))
#define stdc_count_ones(x) (__builtin_stdc_count_ones(x))
#define stdc_has_single_bit(x) (__builtin_stdc_has_single_bit(x))
#define stdc_bit_width(x) (__builtin_stdc_bit_width(x))
#define stdc_bit_floor(x) (__builtin_stdc_bit_floor(x))
#define stdc_bit_ceil(x) (__builtin_stdc_bit_ceil(x))

/* Each function converts its argument as its prototype does. */
#define stdc_leading_zeros_uc(x) stdc_leading_zeros((unsigned char)(x))
#define stdc_leading_zeros_us(x) stdc_leading_zeros((unsigned short)(x))
#define stdc_leading_zeros_ui(x) stdc_leading_zeros((unsigned int)(x))
#define stdc_leading_zeros_ul(x) stdc_leading_zeros((unsigned long)(x))
#define stdc_leading_zeros_ull(x) stdc_leading_zeros((unsigned long long)(x))
#define stdc_leading_ones_uc(x) stdc_leading_ones((unsigned char)(x))
#define stdc_leading_ones_us(x) stdc_leading_ones((unsigned short)(x))
#define stdc_leading_ones_ui(x) stdc_leading_ones((unsigned int)(x))
#define stdc_leading_ones_ul(x) stdc_leading_ones((unsigned long)(x))
#define stdc_leading_ones_ull(x) stdc_leading_ones((unsigned long long)(x))
#define stdc_trailing_zeros_uc(x) stdc_trailing_zeros((unsigned char)(x))
#define stdc_trailing_zeros_us(x) stdc_trailing_zeros((unsigned short)(x))
#define stdc_trailing_zeros_ui(x) stdc_trailing_zeros((unsigned int)(x))
#define stdc_trailing_zeros_ul(x) stdc_trailing_zeros((unsigned long)(x))
#define stdc_trailing_zeros_ull(x) stdc_trailing_zeros((unsigned long long)(x))
#define stdc_trailing_ones_uc(x) stdc_trailing_ones((unsigned char)(x))
#define stdc_trailing_ones_us(x) stdc_trailing_ones((unsigned short)(x))
#define stdc_trailing_ones_ui(x) stdc_trailing_ones((unsigned int)(x))
#define stdc_trailing_ones_ul(x) stdc_trailing_ones((unsigned long)(x))
#define stdc_trailing_ones_ull(x) stdc_trailing_ones((unsigned long long)(x))
#define stdc_first_leading_zero_uc(x) stdc_first_leading_zero((unsigned char)(x))
#define stdc_first_leading_zero_us(x) stdc_first_leading_zero((unsigned short)(x))
#define stdc_first_leading_zero_ui(x) stdc_first_leading_zero((unsigned int)(x))
#define stdc_first_leading_zero_ul(x) stdc_first_leading_zero((unsigned long)(x))
#define stdc_first_leading_zero_ull(x)                                         \
  stdc_first_leading_zero((unsigned long long)(x))
#define stdc_first_leading_one_uc(x) stdc_first_leading_one((unsigned char)(x))
#define stdc_first_leading_one_us(x) stdc_first_leading_one((unsigned short)(x))
#define stdc_first_leading_one_ui(x) stdc_first_leading_one((unsigned int)(x))
#define stdc_first_leading_one_ul(x) stdc_first_leading_one((unsigned long)(x))
#define stdc_first_leading_one_ull(x)                                          \
  stdc_first_leading_one((unsigned long long)(x))
#define stdc_first_trailing_zero_uc(x)                                         \
  stdc_first_trailing_zero((unsigned char)(x))
#define stdc_first_trailing_zero_us(x)                                         \
  stdc_first_trailing_zero((unsigned short)(x))
#define stdc_first_trailing_zero_ui(x) stdc_first_trailing_zero((unsigned int)(x))
#define stdc_first_trailing_zero_ul(x)                                         \
  stdc_first_trailing_zero((unsigned long)(x))
#define stdc_first_trailing_zero_ull(x)                                        \
  stdc_first_trailing_zero((unsigned long long)(x))
#define stdc_first_trailing_one_uc(x) stdc_first_trailing_one((unsigned char)(x))
#define stdc_first_trailing_one_us(x)                                          \
  stdc_first_trailing_one((unsigned short)(x))
#define stdc_first_trailing_one_ui(x) stdc_first_trailing_one((unsigned int)(x))
#define stdc_first_trailing_one_ul(x)                                          \
  stdc_first_trailing_one((unsigned long)(x))
#define stdc_first_trailing_one_ull(x)                                         \
  stdc_first_trailing_one((unsigned long long)(x))
#define stdc_count_zeros_uc(x) stdc_count_zeros((unsigned char)(x))
#define stdc_count_zeros_us(x) stdc_count_zeros((unsigned short)(x))
#define stdc_count_zeros_ui(x) stdc_count_zeros((unsigned int)(x))
#define stdc_count_zeros_ul(x) stdc_count_zeros((unsigned long)(x))
#define stdc_count_zeros_ull(x) stdc_count_zeros((unsigned long long)(x))
#define stdc_count_ones_uc(x) stdc_count_ones((unsigned char)(x))
#define stdc_count_ones_us(x) stdc_count_ones((unsigned short)(x))
#define stdc_count_ones_ui(x) stdc_count_ones((unsigned int)(x))
#define stdc_count_ones_ul(x) stdc_count_ones((unsigned long)(x))
#define stdc_count_ones_ull(x) stdc_count_ones((unsigned long long)(x))
#define stdc_has_single_bit_uc(x) stdc_has_single_bit((unsigned char)(x))
#define stdc_has_single_bit_us(x) stdc_has_single_bit((unsigned short)(x))
#define stdc_has_single_bit_ui(x) stdc_has_single_bit((unsigned int)(x))
#define stdc_has_single_bit_ul(x) stdc_has_single_bit((unsigned long)(x))
#define stdc_has_single_bit_ull(x) stdc_has_single_bit((unsigned long long)(x))
#define stdc_bit_width_uc(x) stdc_bit_width((unsigned char)(x))
#define stdc_bit_width_us(x) stdc_bit_width((unsigned short)(x))
#define stdc_bit_width_ui(x) stdc_bit_width((unsigned int)(x))
#define stdc_bit_width_ul(x) stdc_bit_width((unsigned long)(x))
#define stdc_bit_width_ull(x) stdc_bit_width((unsigned long long)(x))
#define stdc_bit_floor_uc(x) stdc_bit_floor((unsigned char)(x))
#define stdc_bit_floor_us(x) stdc_bit_floor((unsigned short)(x))
#define stdc_bit_floor_ui(x) stdc_bit_floor((unsigned int)(x))
#define stdc_bit_floor_ul(x) stdc_bit_floor((unsigned long)(x))
#define stdc_bit_floor_ull(x) stdc_bit_floor((unsigned long long)(x))
#define stdc_bit_ceil_uc(x) stdc_bit_ceil((unsigned char)(x))
#define stdc_bit_ceil_us(x) stdc_bit_ceil((unsigned short)(x))
#define stdc_bit_ceil_ui(x) stdc_bit_ceil((unsigned int)(x))
#define stdc_bit_ceil_ul(x) stdc_bit_ceil((unsigned long)(x))
#define stdc_bit_ceil_ull(x) stdc_bit_ceil((unsigned long long)(x))
#endif /* __STDC_VERSION_STDBIT_H__ */
