#include <float.h>
#include <iso646.h>
#include <limits.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdalign.h>
#include <stdnoreturn.h>
#define CHECK(n, e) _Static_assert((e), #n)
CHECK(char_bit, CHAR_BIT == 8);
/* Clang's freestanding fallback is 1; bcc's is 4, enough for UTF-8. */
#ifdef __clang__
CHECK(mb_len, MB_LEN_MAX == 1);
#else
CHECK(mb_len, MB_LEN_MAX == 4);
#endif
CHECK(char_min, CHAR_MIN == -128);
CHECK(char_max, CHAR_MAX == 127);
CHECK(size_t, sizeof(size_t) == 8);
CHECK(ptrdiff_t, sizeof(ptrdiff_t) == 8);
CHECK(intptr_t, sizeof(intptr_t) == 8);
CHECK(uintptr_t, sizeof(uintptr_t) == 8);
CHECK(intmax_t, sizeof(intmax_t) == 8);
CHECK(uintmax_t, sizeof(uintmax_t) == 8);
CHECK(wchar_t, sizeof(wchar_t) == 4);
/* C99 §6.7p4: repeated declarations require compatible types. Pin the
   typedef identities as well as their LP64 sizes (C99 §7.17p2). */
extern size_t size_type_probe;
extern unsigned long size_type_probe;
extern ptrdiff_t ptrdiff_type_probe;
extern long ptrdiff_type_probe;
extern wchar_t wchar_type_probe;
extern int wchar_type_probe;
CHECK(va_list, sizeof(va_list) == 24);
CHECK(va_align, _Alignof(va_list) == 8);
CHECK(null, sizeof(NULL) == 8);
int *null_pointer = NULL;
int null_value(void) { return NULL == 0; }
CHECK(bool, sizeof(bool) == 1 and true == 1 and false == 0);
CHECK(bool_defined, __bool_true_false_are_defined == 1);
CHECK(alignment, __alignas_is_defined == 1 and __alignof_is_defined == 1);
CHECK(noreturn, __noreturn_is_defined == 1);
CHECK(decimal_dig, DECIMAL_DIG == 21);
int target_rounding(void) { return FLT_ROUNDS; }
CHECK(float_eval, FLT_EVAL_METHOD == 0);
CHECK(SCHAR_MAX, SCHAR_MAX == 127);
CHECK(SCHAR_MIN, SCHAR_MIN == (-127 - 1));
CHECK(UCHAR_MAX, UCHAR_MAX == 255);
CHECK(SHRT_MAX, SHRT_MAX == 32767);
CHECK(SHRT_MIN, SHRT_MIN == (-32767 - 1));
CHECK(USHRT_MAX, USHRT_MAX == 65535);
CHECK(INT_MAX, INT_MAX == 2147483647);
CHECK(INT_MIN, INT_MIN == (-2147483647 - 1));
CHECK(UINT_MAX, UINT_MAX == 4294967295U);
CHECK(LONG_MAX, LONG_MAX == 9223372036854775807L);
CHECK(LONG_MIN, LONG_MIN == (-9223372036854775807L - 1));
CHECK(ULONG_MAX, ULONG_MAX == 18446744073709551615UL);
CHECK(LLONG_MAX, LLONG_MAX == 9223372036854775807LL);
CHECK(LLONG_MIN, LLONG_MIN == (-9223372036854775807LL - 1));
CHECK(ULLONG_MAX, ULLONG_MAX == 18446744073709551615ULL);
CHECK(INT8_MAX, INT8_MAX == 127);
CHECK(INT8_size, sizeof(int8_t) == 1);
CHECK(INT8_MIN, INT8_MIN == (-127 - 1));
CHECK(UINT8_MAX, UINT8_MAX == 255);
CHECK(UINT8_size, sizeof(uint8_t) == 1);
CHECK(INT_LEAST8_MAX, INT_LEAST8_MAX == 127);
CHECK(INT_LEAST8_size, sizeof(int_least8_t) == 1);
CHECK(INT_LEAST8_MIN, INT_LEAST8_MIN == (-127 - 1));
CHECK(UINT_LEAST8_MAX, UINT_LEAST8_MAX == 255);
CHECK(UINT_LEAST8_size, sizeof(uint_least8_t) == 1);
CHECK(INT_FAST8_MAX, INT_FAST8_MAX == 127);
CHECK(INT_FAST8_size, sizeof(int_fast8_t) == 1);
CHECK(INT_FAST8_MIN, INT_FAST8_MIN == (-127 - 1));
CHECK(UINT_FAST8_MAX, UINT_FAST8_MAX == 255);
CHECK(UINT_FAST8_size, sizeof(uint_fast8_t) == 1);
CHECK(INT16_MAX, INT16_MAX == 32767);
CHECK(INT16_size, sizeof(int16_t) == 2);
CHECK(INT16_MIN, INT16_MIN == (-32767 - 1));
CHECK(UINT16_MAX, UINT16_MAX == 65535);
CHECK(UINT16_size, sizeof(uint16_t) == 2);
CHECK(INT_LEAST16_MAX, INT_LEAST16_MAX == 32767);
CHECK(INT_LEAST16_size, sizeof(int_least16_t) == 2);
CHECK(INT_LEAST16_MIN, INT_LEAST16_MIN == (-32767 - 1));
CHECK(UINT_LEAST16_MAX, UINT_LEAST16_MAX == 65535);
CHECK(UINT_LEAST16_size, sizeof(uint_least16_t) == 2);
CHECK(INT_FAST16_MAX, INT_FAST16_MAX == 32767);
CHECK(INT_FAST16_size, sizeof(int_fast16_t) == 2);
CHECK(INT_FAST16_MIN, INT_FAST16_MIN == (-32767 - 1));
CHECK(UINT_FAST16_MAX, UINT_FAST16_MAX == 65535);
CHECK(UINT_FAST16_size, sizeof(uint_fast16_t) == 2);
CHECK(INT32_MAX, INT32_MAX == 2147483647);
CHECK(INT32_size, sizeof(int32_t) == 4);
CHECK(INT32_MIN, INT32_MIN == (-2147483647 - 1));
CHECK(UINT32_MAX, UINT32_MAX == 4294967295U);
CHECK(UINT32_size, sizeof(uint32_t) == 4);
CHECK(INT_LEAST32_MAX, INT_LEAST32_MAX == 2147483647);
CHECK(INT_LEAST32_size, sizeof(int_least32_t) == 4);
CHECK(INT_LEAST32_MIN, INT_LEAST32_MIN == (-2147483647 - 1));
CHECK(UINT_LEAST32_MAX, UINT_LEAST32_MAX == 4294967295U);
CHECK(UINT_LEAST32_size, sizeof(uint_least32_t) == 4);
CHECK(INT_FAST32_MAX, INT_FAST32_MAX == 2147483647);
CHECK(INT_FAST32_size, sizeof(int_fast32_t) == 4);
CHECK(INT_FAST32_MIN, INT_FAST32_MIN == (-2147483647 - 1));
CHECK(UINT_FAST32_MAX, UINT_FAST32_MAX == 4294967295U);
CHECK(UINT_FAST32_size, sizeof(uint_fast32_t) == 4);
CHECK(INT64_MAX, INT64_MAX == 9223372036854775807L);
CHECK(INT64_size, sizeof(int64_t) == 8);
CHECK(INT64_MIN, INT64_MIN == (-9223372036854775807L - 1));
CHECK(UINT64_MAX, UINT64_MAX == 18446744073709551615UL);
CHECK(UINT64_size, sizeof(uint64_t) == 8);
CHECK(INT_LEAST64_MAX, INT_LEAST64_MAX == 9223372036854775807L);
CHECK(INT_LEAST64_size, sizeof(int_least64_t) == 8);
CHECK(INT_LEAST64_MIN, INT_LEAST64_MIN == (-9223372036854775807L - 1));
CHECK(UINT_LEAST64_MAX, UINT_LEAST64_MAX == 18446744073709551615UL);
CHECK(UINT_LEAST64_size, sizeof(uint_least64_t) == 8);
CHECK(INT_FAST64_MAX, INT_FAST64_MAX == 9223372036854775807L);
CHECK(INT_FAST64_size, sizeof(int_fast64_t) == 8);
CHECK(INT_FAST64_MIN, INT_FAST64_MIN == (-9223372036854775807L - 1));
CHECK(UINT_FAST64_MAX, UINT_FAST64_MAX == 18446744073709551615UL);
CHECK(UINT_FAST64_size, sizeof(uint_fast64_t) == 8);
/* Check both sides of every integer typedef pair. The positivity check
   catches signed 64-bit -1 even when comparison with an unsigned maximum
   would convert it to that maximum (C99 §7.18.1.1-5). */
#define CHECK_INTEGER_PAIR(i, u, m) \
    CHECK(i##_signed, (i)-1 < 0); \
    CHECK(u##_unsigned, (u)-1 > 0 && (u)-1 == (m))
CHECK_INTEGER_PAIR(int8_t, uint8_t, UINT8_MAX);
CHECK_INTEGER_PAIR(int16_t, uint16_t, UINT16_MAX);
CHECK_INTEGER_PAIR(int32_t, uint32_t, UINT32_MAX);
CHECK_INTEGER_PAIR(int64_t, uint64_t, UINT64_MAX);
CHECK_INTEGER_PAIR(int_least8_t, uint_least8_t, UINT_LEAST8_MAX);
CHECK_INTEGER_PAIR(int_least16_t, uint_least16_t, UINT_LEAST16_MAX);
CHECK_INTEGER_PAIR(int_least32_t, uint_least32_t, UINT_LEAST32_MAX);
CHECK_INTEGER_PAIR(int_least64_t, uint_least64_t, UINT_LEAST64_MAX);
CHECK_INTEGER_PAIR(int_fast8_t, uint_fast8_t, UINT_FAST8_MAX);
CHECK_INTEGER_PAIR(int_fast16_t, uint_fast16_t, UINT_FAST16_MAX);
CHECK_INTEGER_PAIR(int_fast32_t, uint_fast32_t, UINT_FAST32_MAX);
CHECK_INTEGER_PAIR(int_fast64_t, uint_fast64_t, UINT_FAST64_MAX);
CHECK_INTEGER_PAIR(intptr_t, uintptr_t, UINTPTR_MAX);
CHECK_INTEGER_PAIR(intmax_t, uintmax_t, UINTMAX_MAX);
#undef CHECK_INTEGER_PAIR
CHECK(INTMAX_MAX, INTMAX_MAX == 9223372036854775807L);
CHECK(INTMAX_MIN, INTMAX_MIN == (-9223372036854775807L - 1));
CHECK(UINTMAX_MAX, UINTMAX_MAX == 18446744073709551615UL);
CHECK(INTPTR_MAX, INTPTR_MAX == 9223372036854775807L);
CHECK(INTPTR_MIN, INTPTR_MIN == (-9223372036854775807L - 1));
CHECK(UINTPTR_MAX, UINTPTR_MAX == 18446744073709551615UL);
CHECK(PTRDIFF_MAX, PTRDIFF_MAX == 9223372036854775807L);
CHECK(PTRDIFF_MIN, PTRDIFF_MIN == (-9223372036854775807L - 1));
CHECK(SIG_ATOMIC_MAX, SIG_ATOMIC_MAX == 2147483647);
CHECK(SIG_ATOMIC_MIN, SIG_ATOMIC_MIN == (-2147483647 - 1));
CHECK(WCHAR_MAX, WCHAR_MAX == 2147483647);
CHECK(WCHAR_MIN, WCHAR_MIN == (-2147483647 - 1));
CHECK(WINT_MAX, WINT_MAX == 4294967295U);
CHECK(WINT_MIN, WINT_MIN == 0U);
CHECK(SIZE_MAX, SIZE_MAX == 18446744073709551615UL);
CHECK(FLTMANT_DIG, FLT_MANT_DIG == (24));
CHECK(FLTDIG, FLT_DIG == (6));
CHECK(FLTMIN_EXP, FLT_MIN_EXP == (-125));
CHECK(FLTMIN_10_EXP, FLT_MIN_10_EXP == (-37));
CHECK(FLTMAX_EXP, FLT_MAX_EXP == (128));
CHECK(FLTMAX_10_EXP, FLT_MAX_10_EXP == (38));
CHECK(DBLMANT_DIG, DBL_MANT_DIG == (53));
CHECK(DBLDIG, DBL_DIG == (15));
CHECK(DBLMIN_EXP, DBL_MIN_EXP == (-1021));
CHECK(DBLMIN_10_EXP, DBL_MIN_10_EXP == (-307));
CHECK(DBLMAX_EXP, DBL_MAX_EXP == (1024));
CHECK(DBLMAX_10_EXP, DBL_MAX_10_EXP == (308));
CHECK(LDBLMANT_DIG, LDBL_MANT_DIG == (64));
CHECK(LDBLDIG, LDBL_DIG == (18));
CHECK(LDBLMIN_EXP, LDBL_MIN_EXP == (-16381));
CHECK(LDBLMIN_10_EXP, LDBL_MIN_10_EXP == (-4931));
CHECK(LDBLMAX_EXP, LDBL_MAX_EXP == (16384));
CHECK(LDBLMAX_10_EXP, LDBL_MAX_10_EXP == (4932));
CHECK(INT8C, INT8_C(123) == 123 and sizeof(INT8_C(123)) == 4);
CHECK(INT16C, INT16_C(123) == 123 and sizeof(INT16_C(123)) == 4);
CHECK(INT32C, INT32_C(123) == 123 and sizeof(INT32_C(123)) == 4);
CHECK(INT64C, INT64_C(123) == 123 and sizeof(INT64_C(123)) == 8);
CHECK(UINT8C, UINT8_C(123) == 123 and sizeof(UINT8_C(123)) == 4);
CHECK(UINT16C, UINT16_C(123) == 123 and sizeof(UINT16_C(123)) == 4);
CHECK(UINT32C, UINT32_C(123) == 123 and sizeof(UINT32_C(123)) == 4);
CHECK(UINT64C, UINT64_C(123) == 123 and sizeof(UINT64_C(123)) == 8);
CHECK(INTMAXC, INTMAX_C(123) == 123 and sizeof(INTMAX_C(123)) == 8);
CHECK(UINTMAXC, UINTMAX_C(123) == 123 and sizeof(UINTMAX_C(123)) == 8);
struct offset_probe { char c; long x; int a[3]; };
CHECK(offset_x, offsetof(struct offset_probe, x) == 8);
CHECK(offset_index, offsetof(struct offset_probe, a[2]) == 24);
alignas(16) int aligned;
noreturn void forever(void) { for (;;) {} }
int sum(int n, ...) { va_list ap, copy; va_start(ap, n); va_copy(copy, ap); int value = va_arg(copy, int); va_end(copy); va_end(ap); return value; }
CHECK(radix, FLT_RADIX == 2);

#if __STDC_VERSION__ >= 201112L
CHECK(max_align_size, sizeof(max_align_t) == 32);
CHECK(max_align_alignment, _Alignof(max_align_t) == 16);
CHECK(FLT_DECIMAL_DIG, FLT_DECIMAL_DIG == 9);
CHECK(DBL_DECIMAL_DIG, DBL_DECIMAL_DIG == 17);
CHECK(LDBL_DECIMAL_DIG, LDBL_DECIMAL_DIG == 21);
CHECK(FLT_HAS_SUBNORM, FLT_HAS_SUBNORM == 1);
CHECK(DBL_HAS_SUBNORM, DBL_HAS_SUBNORM == 1);
CHECK(LDBL_HAS_SUBNORM, LDBL_HAS_SUBNORM == 1);
#endif
int alternative_operators(int a, int b) {
    a and_eq b; a or_eq b; a xor_eq b;
    return (a bitand b) bitor (a xor b) or (not a and b not_eq compl a);
}
#ifndef __STRICT_ANSI__
CHECK(FLT_MAX, FLT_MAX == 3.40282347e+38F);
CHECK(FLT_MIN, FLT_MIN == 1.17549435e-38F);
CHECK(FLT_EPSILON, FLT_EPSILON == 1.19209290e-7F);
CHECK(FLT_TRUE_MIN, FLT_TRUE_MIN == 1.40129846e-45F);
CHECK(DBL_MAX, DBL_MAX == 1.7976931348623157e+308);
CHECK(DBL_MIN, DBL_MIN == 2.2250738585072014e-308);
CHECK(DBL_EPSILON, DBL_EPSILON == 2.2204460492503131e-16);
CHECK(DBL_TRUE_MIN, DBL_TRUE_MIN == 4.9406564584124654e-324);
CHECK(LDBL_MAX, LDBL_MAX == 1.18973149535723176502e+4932L);
CHECK(LDBL_MIN, LDBL_MIN == 3.36210314311209350626e-4932L);
CHECK(LDBL_EPSILON, LDBL_EPSILON == 1.08420217248550443401e-19L);
CHECK(LDBL_TRUE_MIN, LDBL_TRUE_MIN == 3.64519953188247460253e-4951L);
#endif
