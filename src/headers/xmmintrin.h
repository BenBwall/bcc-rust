/* xmmintrin.h - SSE intrinsics for bcc.
 *
 * Written for bcc; part of bcc and under its licence. Each intrinsic is
 * plain C over GNU vector operations, which bcc lowers lane by lane on
 * every back end, as Intel's Intrinsics Guide specifies its result. An
 * intrinsic with an immediate operand is a macro that also names the
 * corresponding GCC/Clang builtin inside `sizeof`, which is not evaluated:
 * the call only lets semantic analysis check the immediate, as GCC and
 * Clang do. Status: language-standards.md, "x86 intrinsic headers". */

#ifndef __XMMINTRIN_H
#define __XMMINTRIN_H

#if !defined(__i386__) && !defined(__x86_64__)
#error "This header is only meant to be used on x86 and x64 architecture"
#endif

#include <mmintrin.h>

typedef float __v4sf __attribute__((__vector_size__(16)));
typedef float __m128 __attribute__((__vector_size__(16), __aligned__(16)));
typedef float __m128_u __attribute__((__vector_size__(16), __aligned__(1)));
typedef unsigned int __v4su __attribute__((__vector_size__(16)));
typedef unsigned short __v8hu __attribute__((__vector_size__(16)));
typedef unsigned char __v16qu __attribute__((__vector_size__(16)));
typedef float __bcc_v2sf __attribute__((__vector_size__(8)));

#if __STDC_HOSTED__
#include <mm_malloc.h>
#endif

#define __BCC_SSE static __inline__ __attribute__((__always_inline__, __nodebug__))
/* Checks `check`, an unevaluated builtin call naming an immediate, then
 * evaluates `value`. */
#define __BCC_IMMEDIATE(check, value) ((void)sizeof(check), (value))

/* Lane 0 of `__low` with lanes 1-3 of `__high`: the scalar `_ss` forms. */
__BCC_SSE __m128 __bcc_low_ss(__m128 __high, __m128 __low) {
  __v4sf __r = (__v4sf)__high;
  __r[0] = ((__v4sf)__low)[0];
  return (__m128)__r;
}

/* Lanes chosen by an all-ones or all-zeros mask: `__mask ? __a : __b`. */
__BCC_SSE __m128 __bcc_select_ps(__v4si __mask, __m128 __a, __m128 __b) {
  return (__m128)((__mask & (__v4si)__a) | (~__mask & (__v4si)__b));
}

/* CVTSS2SI and friends: an integer of `bits` bits, or the integer
 * indefinite value (the most negative one) for a NaN or a value out of
 * range. The value has already been rounded or truncated. */
__BCC_SSE int __bcc_to_i32(double __x) {
  return (__x >= -2147483648.0 && __x < 2147483648.0) ? (int)__x : (int)(-2147483647 - 1);
}
__BCC_SSE long long __bcc_to_i64(double __x) {
  return (__x >= -9223372036854775808.0 && __x < 9223372036854775808.0)
             ? (long long)__x
             : (long long)(-9223372036854775807LL - 1);
}

/* Arithmetic */

__BCC_SSE __m128 _mm_add_ss(__m128 __a, __m128 __b) { return __bcc_low_ss(__a, __a + __b); }
__BCC_SSE __m128 _mm_add_ps(__m128 __a, __m128 __b) { return (__m128)((__v4sf)__a + (__v4sf)__b); }
__BCC_SSE __m128 _mm_sub_ss(__m128 __a, __m128 __b) { return __bcc_low_ss(__a, __a - __b); }
__BCC_SSE __m128 _mm_sub_ps(__m128 __a, __m128 __b) { return (__m128)((__v4sf)__a - (__v4sf)__b); }
__BCC_SSE __m128 _mm_mul_ss(__m128 __a, __m128 __b) { return __bcc_low_ss(__a, __a * __b); }
__BCC_SSE __m128 _mm_mul_ps(__m128 __a, __m128 __b) { return (__m128)((__v4sf)__a * (__v4sf)__b); }
__BCC_SSE __m128 _mm_div_ss(__m128 __a, __m128 __b) { return __bcc_low_ss(__a, __a / __b); }
__BCC_SSE __m128 _mm_div_ps(__m128 __a, __m128 __b) { return (__m128)((__v4sf)__a / (__v4sf)__b); }

__BCC_SSE __m128 _mm_sqrt_ss(__m128 __a) {
  __v4sf __r = (__v4sf)__a;
  __r[0] = __builtin_sqrtf(__r[0]);
  return (__m128)__r;
}
__BCC_SSE __m128 _mm_sqrt_ps(__m128 __a) {
  __v4sf __r = (__v4sf)__a;
  int __i;
  for (__i = 0; __i < 4; ++__i)
    __r[__i] = __builtin_sqrtf(__r[__i]);
  return (__m128)__r;
}

/* RCPPS and RSQRTPS approximate within 1.5 * 2^-12; these give the
 * correctly rounded values, which lie within that bound but differ from
 * a processor's table. */
__BCC_SSE __m128 _mm_rcp_ps(__m128 __a) { return (__m128)(1.0f / (__v4sf)__a); }
__BCC_SSE __m128 _mm_rcp_ss(__m128 __a) { return __bcc_low_ss(__a, _mm_rcp_ps(__a)); }
__BCC_SSE __m128 _mm_rsqrt_ps(__m128 __a) { return _mm_rcp_ps(_mm_sqrt_ps(__a)); }
__BCC_SSE __m128 _mm_rsqrt_ss(__m128 __a) { return __bcc_low_ss(__a, _mm_rsqrt_ps(__a)); }

/* MINPS and MAXPS return the second operand when either is a NaN or both
 * are zeros. */
__BCC_SSE __m128 _mm_min_ps(__m128 __a, __m128 __b) {
  return __bcc_select_ps((__v4sf)__a < (__v4sf)__b, __a, __b);
}
__BCC_SSE __m128 _mm_max_ps(__m128 __a, __m128 __b) {
  return __bcc_select_ps((__v4sf)__a > (__v4sf)__b, __a, __b);
}
__BCC_SSE __m128 _mm_min_ss(__m128 __a, __m128 __b) { return __bcc_low_ss(__a, _mm_min_ps(__a, __b)); }
__BCC_SSE __m128 _mm_max_ss(__m128 __a, __m128 __b) { return __bcc_low_ss(__a, _mm_max_ps(__a, __b)); }

/* Bitwise operations */

__BCC_SSE __m128 _mm_and_ps(__m128 __a, __m128 __b) { return (__m128)((__v4su)__a & (__v4su)__b); }
__BCC_SSE __m128 _mm_andnot_ps(__m128 __a, __m128 __b) { return (__m128)(~(__v4su)__a & (__v4su)__b); }
__BCC_SSE __m128 _mm_or_ps(__m128 __a, __m128 __b) { return (__m128)((__v4su)__a | (__v4su)__b); }
__BCC_SSE __m128 _mm_xor_ps(__m128 __a, __m128 __b) { return (__m128)((__v4su)__a ^ (__v4su)__b); }

/* Comparisons: all ones for true. The negated predicates and `neq` are
 * true for an unordered pair. */

__BCC_SSE __m128 _mm_cmpeq_ps(__m128 __a, __m128 __b) { return (__m128)((__v4sf)__a == (__v4sf)__b); }
__BCC_SSE __m128 _mm_cmplt_ps(__m128 __a, __m128 __b) { return (__m128)((__v4sf)__a < (__v4sf)__b); }
__BCC_SSE __m128 _mm_cmple_ps(__m128 __a, __m128 __b) { return (__m128)((__v4sf)__a <= (__v4sf)__b); }
__BCC_SSE __m128 _mm_cmpgt_ps(__m128 __a, __m128 __b) { return (__m128)((__v4sf)__a > (__v4sf)__b); }
__BCC_SSE __m128 _mm_cmpge_ps(__m128 __a, __m128 __b) { return (__m128)((__v4sf)__a >= (__v4sf)__b); }
__BCC_SSE __m128 _mm_cmpneq_ps(__m128 __a, __m128 __b) { return (__m128)((__v4sf)__a != (__v4sf)__b); }
__BCC_SSE __m128 _mm_cmpnlt_ps(__m128 __a, __m128 __b) { return (__m128)~((__v4sf)__a < (__v4sf)__b); }
__BCC_SSE __m128 _mm_cmpnle_ps(__m128 __a, __m128 __b) { return (__m128)~((__v4sf)__a <= (__v4sf)__b); }
__BCC_SSE __m128 _mm_cmpngt_ps(__m128 __a, __m128 __b) { return (__m128)~((__v4sf)__a > (__v4sf)__b); }
__BCC_SSE __m128 _mm_cmpnge_ps(__m128 __a, __m128 __b) { return (__m128)~((__v4sf)__a >= (__v4sf)__b); }
__BCC_SSE __m128 _mm_cmpord_ps(__m128 __a, __m128 __b) {
  return (__m128)(((__v4sf)__a == (__v4sf)__a) & ((__v4sf)__b == (__v4sf)__b));
}
__BCC_SSE __m128 _mm_cmpunord_ps(__m128 __a, __m128 __b) {
  return (__m128)(((__v4sf)__a != (__v4sf)__a) | ((__v4sf)__b != (__v4sf)__b));
}
__BCC_SSE __m128 _mm_cmpeq_ss(__m128 __a, __m128 __b) { return __bcc_low_ss(__a, _mm_cmpeq_ps(__a, __b)); }
__BCC_SSE __m128 _mm_cmplt_ss(__m128 __a, __m128 __b) { return __bcc_low_ss(__a, _mm_cmplt_ps(__a, __b)); }
__BCC_SSE __m128 _mm_cmple_ss(__m128 __a, __m128 __b) { return __bcc_low_ss(__a, _mm_cmple_ps(__a, __b)); }
__BCC_SSE __m128 _mm_cmpgt_ss(__m128 __a, __m128 __b) { return __bcc_low_ss(__a, _mm_cmpgt_ps(__a, __b)); }
__BCC_SSE __m128 _mm_cmpge_ss(__m128 __a, __m128 __b) { return __bcc_low_ss(__a, _mm_cmpge_ps(__a, __b)); }
__BCC_SSE __m128 _mm_cmpneq_ss(__m128 __a, __m128 __b) { return __bcc_low_ss(__a, _mm_cmpneq_ps(__a, __b)); }
__BCC_SSE __m128 _mm_cmpnlt_ss(__m128 __a, __m128 __b) { return __bcc_low_ss(__a, _mm_cmpnlt_ps(__a, __b)); }
__BCC_SSE __m128 _mm_cmpnle_ss(__m128 __a, __m128 __b) { return __bcc_low_ss(__a, _mm_cmpnle_ps(__a, __b)); }
__BCC_SSE __m128 _mm_cmpngt_ss(__m128 __a, __m128 __b) { return __bcc_low_ss(__a, _mm_cmpngt_ps(__a, __b)); }
__BCC_SSE __m128 _mm_cmpnge_ss(__m128 __a, __m128 __b) { return __bcc_low_ss(__a, _mm_cmpnge_ps(__a, __b)); }
__BCC_SSE __m128 _mm_cmpord_ss(__m128 __a, __m128 __b) { return __bcc_low_ss(__a, _mm_cmpord_ps(__a, __b)); }
__BCC_SSE __m128 _mm_cmpunord_ss(__m128 __a, __m128 __b) { return __bcc_low_ss(__a, _mm_cmpunord_ps(__a, __b)); }

/* COMISS and UCOMISS compare lane 0; only `neq` is true when unordered. */
__BCC_SSE int _mm_comieq_ss(__m128 __a, __m128 __b) { return __a[0] == __b[0]; }
__BCC_SSE int _mm_comilt_ss(__m128 __a, __m128 __b) { return __a[0] < __b[0]; }
__BCC_SSE int _mm_comile_ss(__m128 __a, __m128 __b) { return __a[0] <= __b[0]; }
__BCC_SSE int _mm_comigt_ss(__m128 __a, __m128 __b) { return __a[0] > __b[0]; }
__BCC_SSE int _mm_comige_ss(__m128 __a, __m128 __b) { return __a[0] >= __b[0]; }
__BCC_SSE int _mm_comineq_ss(__m128 __a, __m128 __b) { return __a[0] != __b[0]; }
__BCC_SSE int _mm_ucomieq_ss(__m128 __a, __m128 __b) { return __a[0] == __b[0]; }
__BCC_SSE int _mm_ucomilt_ss(__m128 __a, __m128 __b) { return __a[0] < __b[0]; }
__BCC_SSE int _mm_ucomile_ss(__m128 __a, __m128 __b) { return __a[0] <= __b[0]; }
__BCC_SSE int _mm_ucomigt_ss(__m128 __a, __m128 __b) { return __a[0] > __b[0]; }
__BCC_SSE int _mm_ucomige_ss(__m128 __a, __m128 __b) { return __a[0] >= __b[0]; }
__BCC_SSE int _mm_ucomineq_ss(__m128 __a, __m128 __b) { return __a[0] != __b[0]; }

/* The AVX comparison predicates, which `_mm_cmp_ps` also takes. */
#define _CMP_EQ_OQ 0x00
#define _CMP_LT_OS 0x01
#define _CMP_LE_OS 0x02
#define _CMP_UNORD_Q 0x03
#define _CMP_NEQ_UQ 0x04
#define _CMP_NLT_US 0x05
#define _CMP_NLE_US 0x06
#define _CMP_ORD_Q 0x07
#define _CMP_EQ_UQ 0x08
#define _CMP_NGE_US 0x09
#define _CMP_NGT_US 0x0a
#define _CMP_FALSE_OQ 0x0b
#define _CMP_NEQ_OQ 0x0c
#define _CMP_GE_OS 0x0d
#define _CMP_GT_OS 0x0e
#define _CMP_TRUE_UQ 0x0f
#define _CMP_EQ_OS 0x10
#define _CMP_LT_OQ 0x11
#define _CMP_LE_OQ 0x12
#define _CMP_UNORD_S 0x13
#define _CMP_NEQ_US 0x14
#define _CMP_NLT_UQ 0x15
#define _CMP_NLE_UQ 0x16
#define _CMP_ORD_S 0x17
#define _CMP_EQ_US 0x18
#define _CMP_NGE_UQ 0x19
#define _CMP_NGT_UQ 0x1a
#define _CMP_FALSE_OS 0x1b
#define _CMP_NEQ_OS 0x1c
#define _CMP_GE_OQ 0x1d
#define _CMP_GT_OQ 0x1e
#define _CMP_TRUE_US 0x1f

/* The result of predicate `__p` (its low four bits; bit 4 only selects
 * signalling) for one pair. */
__BCC_SSE int __bcc_compare(double __a, double __b, int __p) {
  int __unordered = __a != __a || __b != __b;
  switch (__p & 15) {
  case 0: return __a == __b;
  case 1: return __a < __b;
  case 2: return __a <= __b;
  case 3: return __unordered;
  case 4: return __a != __b;
  case 5: return !(__a < __b);
  case 6: return !(__a <= __b);
  case 7: return !__unordered;
  case 8: return __unordered || __a == __b;
  case 9: return !(__a >= __b);
  case 10: return !(__a > __b);
  case 11: return 0;
  case 12: return !__unordered && __a != __b;
  case 13: return __a >= __b;
  case 14: return __a > __b;
  default: return 1;
  }
}
__BCC_SSE __m128 __bcc_cmp_ps(__m128 __a, __m128 __b, int __p) {
  __v4si __r;
  int __i;
  for (__i = 0; __i < 4; ++__i)
    __r[__i] = -__bcc_compare(__a[__i], __b[__i], __p);
  return (__m128)__r;
}
#define _mm_cmp_ps(a, b, c)                                                     \
  __BCC_IMMEDIATE(__builtin_ia32_cmpps((__v4sf)(__m128)(a), (__v4sf)(__m128)(b), (c)), \
                  __bcc_cmp_ps((__m128)(a), (__m128)(b), (c)))
#define _mm_cmp_ss(a, b, c)                                                     \
  __BCC_IMMEDIATE(__builtin_ia32_cmpss((__v4sf)(__m128)(a), (__v4sf)(__m128)(b), (c)), \
                  __bcc_low_ss((__m128)(a), __bcc_cmp_ps((__m128)(a), (__m128)(b), (c))))

/* Conversions. CVTSS2SI rounds by the current rounding mode, as
 * `nearbyint` does; CVTTSS2SI truncates. */

__BCC_SSE int _mm_cvtss_si32(__m128 __a) { return __bcc_to_i32(__builtin_nearbyintf(__a[0])); }
__BCC_SSE int _mm_cvt_ss2si(__m128 __a) { return _mm_cvtss_si32(__a); }
__BCC_SSE int _mm_cvttss_si32(__m128 __a) { return __bcc_to_i32(__builtin_truncf(__a[0])); }
__BCC_SSE int _mm_cvtt_ss2si(__m128 __a) { return _mm_cvttss_si32(__a); }
__BCC_SSE long long _mm_cvtss_si64(__m128 __a) { return __bcc_to_i64(__builtin_nearbyintf(__a[0])); }
__BCC_SSE long long _mm_cvttss_si64(__m128 __a) { return __bcc_to_i64(__builtin_truncf(__a[0])); }
__BCC_SSE __m128 _mm_cvtsi32_ss(__m128 __a, int __b) {
  __a[0] = (float)__b;
  return __a;
}
__BCC_SSE __m128 _mm_cvt_si2ss(__m128 __a, int __b) { return _mm_cvtsi32_ss(__a, __b); }
__BCC_SSE __m128 _mm_cvtsi64_ss(__m128 __a, long long __b) {
  __a[0] = (float)__b;
  return __a;
}
__BCC_SSE float _mm_cvtss_f32(__m128 __a) { return __a[0]; }
/* GCC's spellings of the 64-bit conversions. */
#define _mm_cvtss_si64x(a) _mm_cvtss_si64(a)
#define _mm_cvttss_si64x(a) _mm_cvttss_si64(a)
#define _mm_cvtsi64x_ss(a, b) _mm_cvtsi64_ss((a), (b))

/* Conversions to and from MMX registers. */
__BCC_SSE __m64 _mm_cvtps_pi32(__m128 __a) {
  __v2si __r;
  __r[0] = __bcc_to_i32(__builtin_nearbyintf(__a[0]));
  __r[1] = __bcc_to_i32(__builtin_nearbyintf(__a[1]));
  return (__m64)__r;
}
__BCC_SSE __m64 _mm_cvt_ps2pi(__m128 __a) { return _mm_cvtps_pi32(__a); }
__BCC_SSE __m64 _mm_cvttps_pi32(__m128 __a) {
  __v2si __r;
  __r[0] = __bcc_to_i32(__builtin_truncf(__a[0]));
  __r[1] = __bcc_to_i32(__builtin_truncf(__a[1]));
  return (__m64)__r;
}
__BCC_SSE __m64 _mm_cvtt_ps2pi(__m128 __a) { return _mm_cvttps_pi32(__a); }
__BCC_SSE __m128 _mm_cvtpi32_ps(__m128 __a, __m64 __b) {
  __a[0] = (float)((__v2si)__b)[0];
  __a[1] = (float)((__v2si)__b)[1];
  return __a;
}
__BCC_SSE __m128 _mm_cvt_pi2ps(__m128 __a, __m64 __b) { return _mm_cvtpi32_ps(__a, __b); }
__BCC_SSE __m128 _mm_cvtpi16_ps(__m64 __a) {
  __m128 __r;
  int __i;
  for (__i = 0; __i < 4; ++__i)
    __r[__i] = (float)((__v4hi)__a)[__i];
  return __r;
}
__BCC_SSE __m128 _mm_cvtpu16_ps(__m64 __a) {
  __m128 __r;
  int __i;
  for (__i = 0; __i < 4; ++__i)
    __r[__i] = (float)((__v4hu)__a)[__i];
  return __r;
}
__BCC_SSE __m128 _mm_cvtpi8_ps(__m64 __a) {
  __m128 __r;
  int __i;
  for (__i = 0; __i < 4; ++__i)
    __r[__i] = (float)((__v8qs)__a)[__i];
  return __r;
}
__BCC_SSE __m128 _mm_cvtpu8_ps(__m64 __a) {
  __m128 __r;
  int __i;
  for (__i = 0; __i < 4; ++__i)
    __r[__i] = (float)((__v8qu)__a)[__i];
  return __r;
}
__BCC_SSE __m128 _mm_cvtpi32x2_ps(__m64 __a, __m64 __b) {
  __m128 __r;
  __r[0] = (float)((__v2si)__a)[0];
  __r[1] = (float)((__v2si)__a)[1];
  __r[2] = (float)((__v2si)__b)[0];
  __r[3] = (float)((__v2si)__b)[1];
  return __r;
}
/* CVTPS2PI then PACKSSDW: saturated to 16 bits, out of range 0x8000. */
__BCC_SSE __m64 _mm_cvtps_pi16(__m128 __a) {
  __v4hi __r;
  int __i;
  for (__i = 0; __i < 4; ++__i) {
    int __v = __bcc_to_i32(__builtin_nearbyintf(__a[__i]));
    __r[__i] = (short)(__v > 32767 ? 32767 : __v < -32768 ? -32768 : __v);
  }
  return (__m64)__r;
}
__BCC_SSE __m64 _mm_cvtps_pi8(__m128 __a) {
  __v8qs __r = (__v8qs){0};
  int __i;
  for (__i = 0; __i < 4; ++__i) {
    int __v = __bcc_to_i32(__builtin_nearbyintf(__a[__i]));
    __v = __v > 32767 ? 32767 : __v < -32768 ? -32768 : __v;
    __r[__i] = (signed char)(__v > 127 ? 127 : __v < -128 ? -128 : __v);
  }
  return (__m64)__r;
}

/* Loads and stores */

__BCC_SSE __m128 _mm_loadu_ps(const float *__p) { return *(const __m128_u *)__p; }
__BCC_SSE __m128 _mm_load_ps(const float *__p) { return *(const __m128 *)__p; }
__BCC_SSE __m128 _mm_load_ss(const float *__p) {
  __m128 __r = (__m128){0};
  __builtin_memcpy(&__r, __p, sizeof(float));
  return __r;
}
__BCC_SSE __m128 _mm_load1_ps(const float *__p) {
  float __x;
  __builtin_memcpy(&__x, __p, sizeof __x);
  return (__m128){__x, __x, __x, __x};
}
#define _mm_load_ps1(p) _mm_load1_ps(p)
__BCC_SSE __m128 _mm_loadr_ps(const float *__p) {
  __m128 __a = *(const __m128 *)__p;
  return (__m128){__a[3], __a[2], __a[1], __a[0]};
}
__BCC_SSE __m128 _mm_loadh_pi(__m128 __a, const __m64 *__p) {
  __bcc_v2sf __b;
  __builtin_memcpy(&__b, __p, sizeof __b);
  __a[2] = __b[0];
  __a[3] = __b[1];
  return __a;
}
__BCC_SSE __m128 _mm_loadl_pi(__m128 __a, const __m64 *__p) {
  __bcc_v2sf __b;
  __builtin_memcpy(&__b, __p, sizeof __b);
  __a[0] = __b[0];
  __a[1] = __b[1];
  return __a;
}
__BCC_SSE void _mm_storeu_ps(float *__p, __m128 __a) { *(__m128_u *)__p = __a; }
__BCC_SSE void _mm_store_ps(float *__p, __m128 __a) { *(__m128 *)__p = __a; }
__BCC_SSE void _mm_store_ss(float *__p, __m128 __a) { __builtin_memcpy(__p, &__a, sizeof(float)); }
__BCC_SSE void _mm_store1_ps(float *__p, __m128 __a) {
  *(__m128 *)__p = (__m128){__a[0], __a[0], __a[0], __a[0]};
}
#define _mm_store_ps1(p, a) _mm_store1_ps((p), (a))
__BCC_SSE void _mm_storer_ps(float *__p, __m128 __a) {
  *(__m128 *)__p = (__m128){__a[3], __a[2], __a[1], __a[0]};
}
__BCC_SSE void _mm_storeh_pi(__m64 *__p, __m128 __a) {
  __bcc_v2sf __b = {__a[2], __a[3]};
  __builtin_memcpy(__p, &__b, sizeof __b);
}
__BCC_SSE void _mm_storel_pi(__m64 *__p, __m128 __a) {
  __bcc_v2sf __b = {__a[0], __a[1]};
  __builtin_memcpy(__p, &__b, sizeof __b);
}
/* Non-temporal stores are ordinary stores here. */
__BCC_SSE void _mm_stream_ps(float *__p, __m128 __a) { *(__m128 *)__p = __a; }
__BCC_SSE void _mm_stream_pi(__m64 *__p, __m64 __a) { __builtin_memcpy(__p, &__a, sizeof __a); }

/* Construction */

__BCC_SSE __m128 _mm_set_ss(float __w) { return (__m128){__w, 0, 0, 0}; }
__BCC_SSE __m128 _mm_set1_ps(float __w) { return (__m128){__w, __w, __w, __w}; }
__BCC_SSE __m128 _mm_set_ps1(float __w) { return _mm_set1_ps(__w); }
__BCC_SSE __m128 _mm_set_ps(float __z, float __y, float __x, float __w) {
  return (__m128){__w, __x, __y, __z};
}
__BCC_SSE __m128 _mm_setr_ps(float __z, float __y, float __x, float __w) {
  return (__m128){__z, __y, __x, __w};
}
__BCC_SSE __m128 _mm_setzero_ps(void) { return (__m128){0, 0, 0, 0}; }
/* An undefined vector is zero here. */
__BCC_SSE __m128 _mm_undefined_ps(void) { return (__m128){0, 0, 0, 0}; }
__BCC_SSE __m128 _mm_move_ss(__m128 __a, __m128 __b) { return __bcc_low_ss(__a, __b); }

/* Shuffles and moves */

__BCC_SSE __m128 __bcc_shuffle_ps(__m128 __a, __m128 __b, int __imm) {
  return (__m128){__a[__imm & 3], __a[(__imm >> 2) & 3], __b[(__imm >> 4) & 3],
                  __b[(__imm >> 6) & 3]};
}
#define _mm_shuffle_ps(a, b, mask)                                              \
  __BCC_IMMEDIATE(__builtin_ia32_shufps((__v4sf)(__m128)(a), (__v4sf)(__m128)(b), (int)(mask)), \
                  __bcc_shuffle_ps((__m128)(a), (__m128)(b), (int)(mask)))
__BCC_SSE __m128 _mm_unpackhi_ps(__m128 __a, __m128 __b) { return (__m128){__a[2], __b[2], __a[3], __b[3]}; }
__BCC_SSE __m128 _mm_unpacklo_ps(__m128 __a, __m128 __b) { return (__m128){__a[0], __b[0], __a[1], __b[1]}; }
__BCC_SSE __m128 _mm_movehl_ps(__m128 __a, __m128 __b) { return (__m128){__b[2], __b[3], __a[2], __a[3]}; }
__BCC_SSE __m128 _mm_movelh_ps(__m128 __a, __m128 __b) { return (__m128){__a[0], __a[1], __b[0], __b[1]}; }
__BCC_SSE int _mm_movemask_ps(__m128 __a) {
  __v4si __s = (__v4si)__a < 0;
  return (__s[0] & 1) | (__s[1] & 2) | (__s[2] & 4) | (__s[3] & 8);
}

/* MMX extensions of SSE */

__BCC_SSE __m64 _mm_max_pi16(__m64 __a, __m64 __b) {
  __v4hi __x = (__v4hi)__a, __y = (__v4hi)__b;
  return (__m64)((__x > __y) & __x | ~(__x > __y) & __y);
}
__BCC_SSE __m64 _mm_min_pi16(__m64 __a, __m64 __b) {
  __v4hi __x = (__v4hi)__a, __y = (__v4hi)__b;
  return (__m64)((__x < __y) & __x | ~(__x < __y) & __y);
}
__BCC_SSE __m64 _mm_max_pu8(__m64 __a, __m64 __b) {
  __v8qu __x = (__v8qu)__a, __y = (__v8qu)__b;
  return (__m64)((__v8qu)(__x > __y) & __x | (__v8qu)~(__x > __y) & __y);
}
__BCC_SSE __m64 _mm_min_pu8(__m64 __a, __m64 __b) {
  __v8qu __x = (__v8qu)__a, __y = (__v8qu)__b;
  return (__m64)((__v8qu)(__x < __y) & __x | (__v8qu)~(__x < __y) & __y);
}
__BCC_SSE int _mm_movemask_pi8(__m64 __a) {
  __v8qs __x = (__v8qs)__a;
  int __r = 0, __i;
  for (__i = 0; __i < 8; ++__i)
    __r |= (__x[__i] < 0) << __i;
  return __r;
}
__BCC_SSE __m64 _mm_mulhi_pu16(__m64 __a, __m64 __b) {
  __v4hu __x = (__v4hu)__a, __y = (__v4hu)__b, __r;
  int __i;
  for (__i = 0; __i < 4; ++__i)
    __r[__i] = (unsigned short)(((unsigned)__x[__i] * __y[__i]) >> 16);
  return (__m64)__r;
}
__BCC_SSE void _mm_maskmove_si64(__m64 __d, __m64 __n, char *__p) {
  __v8qs __x = (__v8qs)__d, __m = (__v8qs)__n;
  int __i;
  for (__i = 0; __i < 8; ++__i)
    if (__m[__i] < 0)
      __p[__i] = __x[__i];
}
__BCC_SSE __m64 _mm_avg_pu8(__m64 __a, __m64 __b) {
  __v8qu __x = (__v8qu)__a, __y = (__v8qu)__b, __r;
  int __i;
  for (__i = 0; __i < 8; ++__i)
    __r[__i] = (unsigned char)((__x[__i] + __y[__i] + 1) >> 1);
  return (__m64)__r;
}
__BCC_SSE __m64 _mm_avg_pu16(__m64 __a, __m64 __b) {
  __v4hu __x = (__v4hu)__a, __y = (__v4hu)__b, __r;
  int __i;
  for (__i = 0; __i < 4; ++__i)
    __r[__i] = (unsigned short)((__x[__i] + __y[__i] + 1u) >> 1);
  return (__m64)__r;
}
__BCC_SSE __m64 _mm_sad_pu8(__m64 __a, __m64 __b) {
  __v8qu __x = (__v8qu)__a, __y = (__v8qu)__b;
  int __s = 0, __i;
  for (__i = 0; __i < 8; ++__i)
    __s += __x[__i] > __y[__i] ? __x[__i] - __y[__i] : __y[__i] - __x[__i];
  return (__m64)(__v4hi){(short)__s, 0, 0, 0};
}
__BCC_SSE int __bcc_extract_pi16(__m64 __a, int __n) { return ((__v4hu)__a)[__n & 3]; }
#define _mm_extract_pi16(a, n)                                                  \
  __BCC_IMMEDIATE(__builtin_ia32_vec_ext_v4hi((__v4hi)(__m64)(a), (int)(n)),    \
                  __bcc_extract_pi16((__m64)(a), (int)(n)))
__BCC_SSE __m64 __bcc_insert_pi16(__m64 __a, int __d, int __n) {
  __v4hi __r = (__v4hi)__a;
  __r[__n & 3] = (short)__d;
  return (__m64)__r;
}
#define _mm_insert_pi16(a, d, n)                                                \
  __BCC_IMMEDIATE(__builtin_ia32_vec_set_v4hi((__v4hi)(__m64)(a), (int)(d), (int)(n)), \
                  __bcc_insert_pi16((__m64)(a), (int)(d), (int)(n)))
__BCC_SSE __m64 __bcc_shuffle_pi16(__m64 __a, int __n) {
  __v4hi __x = (__v4hi)__a;
  return (__m64)(__v4hi){__x[__n & 3], __x[(__n >> 2) & 3], __x[(__n >> 4) & 3],
                         __x[(__n >> 6) & 3]};
}
#define _mm_shuffle_pi16(a, n)                                                  \
  __BCC_IMMEDIATE(__builtin_shufflevector((__v4hi)(__m64)(a), (__v4hi)(__m64)(a), \
                                          (n) & 3, ((n) >> 2) & 3, ((n) >> 4) & 3, \
                                          ((n) >> 6) & 3),                      \
                  __bcc_shuffle_pi16((__m64)(a), (int)(n)))

#define _m_pextrw _mm_extract_pi16
#define _m_pinsrw _mm_insert_pi16
#define _m_pmaxsw _mm_max_pi16
#define _m_pmaxub _mm_max_pu8
#define _m_pminsw _mm_min_pi16
#define _m_pminub _mm_min_pu8
#define _m_pmovmskb _mm_movemask_pi8
#define _m_pmulhuw _mm_mulhi_pu16
#define _m_pshufw _mm_shuffle_pi16
#define _m_maskmovq _mm_maskmove_si64
#define _m_pavgb _mm_avg_pu8
#define _m_pavgw _mm_avg_pu16
#define _m_psadbw _mm_sad_pu8

/* Cache control, fences and the control/status register. These are
 * instructions, so the interpreter cannot run them. */

#define _MM_HINT_ET0 7
#define _MM_HINT_ET1 6
#define _MM_HINT_T0 3
#define _MM_HINT_T1 2
#define _MM_HINT_T2 1
#define _MM_HINT_NTA 0
#define _mm_prefetch(a, sel) (__builtin_prefetch((const void *)(a), ((sel) >> 2) & 1, (sel) & 3))

__BCC_SSE void _mm_sfence(void) { __asm__ __volatile__("sfence" ::: "memory"); }
__BCC_SSE unsigned int _mm_getcsr(void) {
  unsigned int __csr;
  __asm__ __volatile__("stmxcsr %0" : "=m"(__csr));
  return __csr;
}
__BCC_SSE void _mm_setcsr(unsigned int __csr) { __asm__ __volatile__("ldmxcsr %0" ::"m"(__csr)); }

#define _MM_EXCEPT_INVALID (0x0001U)
#define _MM_EXCEPT_DENORM (0x0002U)
#define _MM_EXCEPT_DIV_ZERO (0x0004U)
#define _MM_EXCEPT_OVERFLOW (0x0008U)
#define _MM_EXCEPT_UNDERFLOW (0x0010U)
#define _MM_EXCEPT_INEXACT (0x0020U)
#define _MM_EXCEPT_MASK (0x003fU)
#define _MM_MASK_INVALID (0x0080U)
#define _MM_MASK_DENORM (0x0100U)
#define _MM_MASK_DIV_ZERO (0x0200U)
#define _MM_MASK_OVERFLOW (0x0400U)
#define _MM_MASK_UNDERFLOW (0x0800U)
#define _MM_MASK_INEXACT (0x1000U)
#define _MM_MASK_MASK (0x1f80U)
#define _MM_ROUND_NEAREST (0x0000U)
#define _MM_ROUND_DOWN (0x2000U)
#define _MM_ROUND_UP (0x4000U)
#define _MM_ROUND_TOWARD_ZERO (0x6000U)
#define _MM_ROUND_MASK (0x6000U)
#define _MM_FLUSH_ZERO_MASK (0x8000U)
#define _MM_FLUSH_ZERO_ON (0x8000U)
#define _MM_FLUSH_ZERO_OFF (0x0000U)
#define _MM_GET_EXCEPTION_MASK() (_mm_getcsr() & _MM_MASK_MASK)
#define _MM_GET_EXCEPTION_STATE() (_mm_getcsr() & _MM_EXCEPT_MASK)
#define _MM_GET_FLUSH_ZERO_MODE() (_mm_getcsr() & _MM_FLUSH_ZERO_MASK)
#define _MM_GET_ROUNDING_MODE() (_mm_getcsr() & _MM_ROUND_MASK)
#define _MM_SET_EXCEPTION_MASK(x) (_mm_setcsr((_mm_getcsr() & ~_MM_MASK_MASK) | (x)))
#define _MM_SET_EXCEPTION_STATE(x) (_mm_setcsr((_mm_getcsr() & ~_MM_EXCEPT_MASK) | (x)))
#define _MM_SET_FLUSH_ZERO_MODE(x) (_mm_setcsr((_mm_getcsr() & ~_MM_FLUSH_ZERO_MASK) | (x)))
#define _MM_SET_ROUNDING_MODE(x) (_mm_setcsr((_mm_getcsr() & ~_MM_ROUND_MASK) | (x)))

#define _MM_SHUFFLE(z, y, x, w) (((z) << 6) | ((y) << 4) | ((x) << 2) | (w))
#define _MM_TRANSPOSE4_PS(row0, row1, row2, row3)                               \
  do {                                                                          \
    __m128 __t0 = _mm_unpacklo_ps((row0), (row1));                              \
    __m128 __t1 = _mm_unpacklo_ps((row2), (row3));                              \
    __m128 __t2 = _mm_unpackhi_ps((row0), (row1));                              \
    __m128 __t3 = _mm_unpackhi_ps((row2), (row3));                              \
    (row0) = _mm_movelh_ps(__t0, __t1);                                         \
    (row1) = _mm_movehl_ps(__t1, __t0);                                         \
    (row2) = _mm_movelh_ps(__t2, __t3);                                         \
    (row3) = _mm_movehl_ps(__t3, __t2);                                         \
  } while (0)
#define _MM_ALIGN16 __attribute__((__aligned__(16)))

/* GCC and Clang declare SSE2 with SSE. */
#include <emmintrin.h>

#endif /* __XMMINTRIN_H */
