/* emmintrin.h - SSE2 intrinsics for bcc.
 *
 * Written for bcc; part of bcc and under its licence. As in
 * <xmmintrin.h>, each intrinsic is plain C over GNU vector operations,
 * and an intrinsic with an immediate operand names the corresponding
 * builtin only inside `sizeof`, to have its immediate checked. */

#ifndef __EMMINTRIN_H
#define __EMMINTRIN_H

#if !defined(__i386__) && !defined(__x86_64__)
#error "This header is only meant to be used on x86 and x64 architecture"
#endif

#include <xmmintrin.h>

typedef double __m128d __attribute__((__vector_size__(16), __aligned__(16)));
typedef double __m128d_u __attribute__((__vector_size__(16), __aligned__(1)));
typedef long long __m128i_u __attribute__((__vector_size__(16), __aligned__(1)));
typedef double __v2df __attribute__((__vector_size__(16)));
typedef unsigned long long __v2du __attribute__((__vector_size__(16)));
typedef signed char __v16qs __attribute__((__vector_size__(16)));

/* Lane 0 of `__low` with lane 1 of `__high`: the scalar `_sd` forms. */
__BCC_SSE __m128d __bcc_low_sd(__m128d __high, __m128d __low) {
  __v2df __r = (__v2df)__high;
  __r[0] = ((__v2df)__low)[0];
  return (__m128d)__r;
}
__BCC_SSE __m128d __bcc_select_pd(__v2di __mask, __m128d __a, __m128d __b) {
  return (__m128d)((__mask & (__v2di)__a) | (~__mask & (__v2di)__b));
}

/* Double-precision arithmetic */

__BCC_SSE __m128d _mm_add_sd(__m128d __a, __m128d __b) { return __bcc_low_sd(__a, __a + __b); }
__BCC_SSE __m128d _mm_add_pd(__m128d __a, __m128d __b) { return (__m128d)((__v2df)__a + (__v2df)__b); }
__BCC_SSE __m128d _mm_sub_sd(__m128d __a, __m128d __b) { return __bcc_low_sd(__a, __a - __b); }
__BCC_SSE __m128d _mm_sub_pd(__m128d __a, __m128d __b) { return (__m128d)((__v2df)__a - (__v2df)__b); }
__BCC_SSE __m128d _mm_mul_sd(__m128d __a, __m128d __b) { return __bcc_low_sd(__a, __a * __b); }
__BCC_SSE __m128d _mm_mul_pd(__m128d __a, __m128d __b) { return (__m128d)((__v2df)__a * (__v2df)__b); }
__BCC_SSE __m128d _mm_div_sd(__m128d __a, __m128d __b) { return __bcc_low_sd(__a, __a / __b); }
__BCC_SSE __m128d _mm_div_pd(__m128d __a, __m128d __b) { return (__m128d)((__v2df)__a / (__v2df)__b); }
__BCC_SSE __m128d _mm_sqrt_sd(__m128d __a, __m128d __b) {
  __a[0] = __builtin_sqrt(__b[0]);
  return __a;
}
__BCC_SSE __m128d _mm_sqrt_pd(__m128d __a) {
  return (__m128d){__builtin_sqrt(__a[0]), __builtin_sqrt(__a[1])};
}
__BCC_SSE __m128d _mm_min_pd(__m128d __a, __m128d __b) {
  return __bcc_select_pd((__v2df)__a < (__v2df)__b, __a, __b);
}
__BCC_SSE __m128d _mm_max_pd(__m128d __a, __m128d __b) {
  return __bcc_select_pd((__v2df)__a > (__v2df)__b, __a, __b);
}
__BCC_SSE __m128d _mm_min_sd(__m128d __a, __m128d __b) { return __bcc_low_sd(__a, _mm_min_pd(__a, __b)); }
__BCC_SSE __m128d _mm_max_sd(__m128d __a, __m128d __b) { return __bcc_low_sd(__a, _mm_max_pd(__a, __b)); }

__BCC_SSE __m128d _mm_and_pd(__m128d __a, __m128d __b) { return (__m128d)((__v2du)__a & (__v2du)__b); }
__BCC_SSE __m128d _mm_andnot_pd(__m128d __a, __m128d __b) { return (__m128d)(~(__v2du)__a & (__v2du)__b); }
__BCC_SSE __m128d _mm_or_pd(__m128d __a, __m128d __b) { return (__m128d)((__v2du)__a | (__v2du)__b); }
__BCC_SSE __m128d _mm_xor_pd(__m128d __a, __m128d __b) { return (__m128d)((__v2du)__a ^ (__v2du)__b); }

/* Double-precision comparisons */

__BCC_SSE __m128d _mm_cmpeq_pd(__m128d __a, __m128d __b) { return (__m128d)((__v2df)__a == (__v2df)__b); }
__BCC_SSE __m128d _mm_cmplt_pd(__m128d __a, __m128d __b) { return (__m128d)((__v2df)__a < (__v2df)__b); }
__BCC_SSE __m128d _mm_cmple_pd(__m128d __a, __m128d __b) { return (__m128d)((__v2df)__a <= (__v2df)__b); }
__BCC_SSE __m128d _mm_cmpgt_pd(__m128d __a, __m128d __b) { return (__m128d)((__v2df)__a > (__v2df)__b); }
__BCC_SSE __m128d _mm_cmpge_pd(__m128d __a, __m128d __b) { return (__m128d)((__v2df)__a >= (__v2df)__b); }
__BCC_SSE __m128d _mm_cmpneq_pd(__m128d __a, __m128d __b) { return (__m128d)((__v2df)__a != (__v2df)__b); }
__BCC_SSE __m128d _mm_cmpnlt_pd(__m128d __a, __m128d __b) { return (__m128d)~((__v2df)__a < (__v2df)__b); }
__BCC_SSE __m128d _mm_cmpnle_pd(__m128d __a, __m128d __b) { return (__m128d)~((__v2df)__a <= (__v2df)__b); }
__BCC_SSE __m128d _mm_cmpngt_pd(__m128d __a, __m128d __b) { return (__m128d)~((__v2df)__a > (__v2df)__b); }
__BCC_SSE __m128d _mm_cmpnge_pd(__m128d __a, __m128d __b) { return (__m128d)~((__v2df)__a >= (__v2df)__b); }
__BCC_SSE __m128d _mm_cmpord_pd(__m128d __a, __m128d __b) {
  return (__m128d)(((__v2df)__a == (__v2df)__a) & ((__v2df)__b == (__v2df)__b));
}
__BCC_SSE __m128d _mm_cmpunord_pd(__m128d __a, __m128d __b) {
  return (__m128d)(((__v2df)__a != (__v2df)__a) | ((__v2df)__b != (__v2df)__b));
}
__BCC_SSE __m128d _mm_cmpeq_sd(__m128d __a, __m128d __b) { return __bcc_low_sd(__a, _mm_cmpeq_pd(__a, __b)); }
__BCC_SSE __m128d _mm_cmplt_sd(__m128d __a, __m128d __b) { return __bcc_low_sd(__a, _mm_cmplt_pd(__a, __b)); }
__BCC_SSE __m128d _mm_cmple_sd(__m128d __a, __m128d __b) { return __bcc_low_sd(__a, _mm_cmple_pd(__a, __b)); }
__BCC_SSE __m128d _mm_cmpgt_sd(__m128d __a, __m128d __b) { return __bcc_low_sd(__a, _mm_cmpgt_pd(__a, __b)); }
__BCC_SSE __m128d _mm_cmpge_sd(__m128d __a, __m128d __b) { return __bcc_low_sd(__a, _mm_cmpge_pd(__a, __b)); }
__BCC_SSE __m128d _mm_cmpneq_sd(__m128d __a, __m128d __b) { return __bcc_low_sd(__a, _mm_cmpneq_pd(__a, __b)); }
__BCC_SSE __m128d _mm_cmpnlt_sd(__m128d __a, __m128d __b) { return __bcc_low_sd(__a, _mm_cmpnlt_pd(__a, __b)); }
__BCC_SSE __m128d _mm_cmpnle_sd(__m128d __a, __m128d __b) { return __bcc_low_sd(__a, _mm_cmpnle_pd(__a, __b)); }
__BCC_SSE __m128d _mm_cmpngt_sd(__m128d __a, __m128d __b) { return __bcc_low_sd(__a, _mm_cmpngt_pd(__a, __b)); }
__BCC_SSE __m128d _mm_cmpnge_sd(__m128d __a, __m128d __b) { return __bcc_low_sd(__a, _mm_cmpnge_pd(__a, __b)); }
__BCC_SSE __m128d _mm_cmpord_sd(__m128d __a, __m128d __b) { return __bcc_low_sd(__a, _mm_cmpord_pd(__a, __b)); }
__BCC_SSE __m128d _mm_cmpunord_sd(__m128d __a, __m128d __b) { return __bcc_low_sd(__a, _mm_cmpunord_pd(__a, __b)); }

__BCC_SSE int _mm_comieq_sd(__m128d __a, __m128d __b) { return __a[0] == __b[0]; }
__BCC_SSE int _mm_comilt_sd(__m128d __a, __m128d __b) { return __a[0] < __b[0]; }
__BCC_SSE int _mm_comile_sd(__m128d __a, __m128d __b) { return __a[0] <= __b[0]; }
__BCC_SSE int _mm_comigt_sd(__m128d __a, __m128d __b) { return __a[0] > __b[0]; }
__BCC_SSE int _mm_comige_sd(__m128d __a, __m128d __b) { return __a[0] >= __b[0]; }
__BCC_SSE int _mm_comineq_sd(__m128d __a, __m128d __b) { return __a[0] != __b[0]; }
__BCC_SSE int _mm_ucomieq_sd(__m128d __a, __m128d __b) { return __a[0] == __b[0]; }
__BCC_SSE int _mm_ucomilt_sd(__m128d __a, __m128d __b) { return __a[0] < __b[0]; }
__BCC_SSE int _mm_ucomile_sd(__m128d __a, __m128d __b) { return __a[0] <= __b[0]; }
__BCC_SSE int _mm_ucomigt_sd(__m128d __a, __m128d __b) { return __a[0] > __b[0]; }
__BCC_SSE int _mm_ucomige_sd(__m128d __a, __m128d __b) { return __a[0] >= __b[0]; }
__BCC_SSE int _mm_ucomineq_sd(__m128d __a, __m128d __b) { return __a[0] != __b[0]; }

__BCC_SSE __m128d __bcc_cmp_pd(__m128d __a, __m128d __b, int __p) {
  __v2di __r;
  __r[0] = -__bcc_compare(__a[0], __b[0], __p);
  __r[1] = -__bcc_compare(__a[1], __b[1], __p);
  return (__m128d)__r;
}
#define _mm_cmp_pd(a, b, c)                                                     \
  __BCC_IMMEDIATE(__builtin_ia32_cmppd((__v2df)(__m128d)(a), (__v2df)(__m128d)(b), (c)), \
                  __bcc_cmp_pd((__m128d)(a), (__m128d)(b), (c)))
#define _mm_cmp_sd(a, b, c)                                                     \
  __BCC_IMMEDIATE(__builtin_ia32_cmpsd((__v2df)(__m128d)(a), (__v2df)(__m128d)(b), (c)), \
                  __bcc_low_sd((__m128d)(a), __bcc_cmp_pd((__m128d)(a), (__m128d)(b), (c))))

/* Conversions. CVTPD2DQ and friends round by the current rounding mode;
 * the `cvtt` forms truncate; out of range is the integer indefinite. */

__BCC_SSE __m128 _mm_cvtpd_ps(__m128d __a) { return (__m128){(float)__a[0], (float)__a[1], 0, 0}; }
__BCC_SSE __m128d _mm_cvtps_pd(__m128 __a) { return (__m128d){__a[0], __a[1]}; }
__BCC_SSE __m128d _mm_cvtepi32_pd(__m128i __a) {
  return (__m128d){((__v4si)__a)[0], ((__v4si)__a)[1]};
}
__BCC_SSE __m128i _mm_cvtpd_epi32(__m128d __a) {
  return (__m128i)(__v4si){__bcc_to_i32(__builtin_nearbyint(__a[0])),
                           __bcc_to_i32(__builtin_nearbyint(__a[1])), 0, 0};
}
__BCC_SSE __m128i _mm_cvttpd_epi32(__m128d __a) {
  return (__m128i)(__v4si){__bcc_to_i32(__builtin_trunc(__a[0])),
                           __bcc_to_i32(__builtin_trunc(__a[1])), 0, 0};
}
__BCC_SSE __m64 _mm_cvtpd_pi32(__m128d __a) {
  return (__m64)(__v2si){__bcc_to_i32(__builtin_nearbyint(__a[0])),
                         __bcc_to_i32(__builtin_nearbyint(__a[1]))};
}
__BCC_SSE __m64 _mm_cvttpd_pi32(__m128d __a) {
  return (__m64)(__v2si){__bcc_to_i32(__builtin_trunc(__a[0])),
                         __bcc_to_i32(__builtin_trunc(__a[1]))};
}
__BCC_SSE __m128d _mm_cvtpi32_pd(__m64 __a) {
  return (__m128d){((__v2si)__a)[0], ((__v2si)__a)[1]};
}
__BCC_SSE int _mm_cvtsd_si32(__m128d __a) { return __bcc_to_i32(__builtin_nearbyint(__a[0])); }
__BCC_SSE int _mm_cvttsd_si32(__m128d __a) { return __bcc_to_i32(__builtin_trunc(__a[0])); }
__BCC_SSE long long _mm_cvtsd_si64(__m128d __a) { return __bcc_to_i64(__builtin_nearbyint(__a[0])); }
__BCC_SSE long long _mm_cvttsd_si64(__m128d __a) { return __bcc_to_i64(__builtin_trunc(__a[0])); }
#define _mm_cvtsd_si64x(a) _mm_cvtsd_si64(a)
#define _mm_cvttsd_si64x(a) _mm_cvttsd_si64(a)
__BCC_SSE __m128 _mm_cvtsd_ss(__m128 __a, __m128d __b) {
  __a[0] = (float)__b[0];
  return __a;
}
__BCC_SSE __m128d _mm_cvtsi32_sd(__m128d __a, int __b) {
  __a[0] = __b;
  return __a;
}
__BCC_SSE __m128d _mm_cvtsi64_sd(__m128d __a, long long __b) {
  __a[0] = (double)__b;
  return __a;
}
#define _mm_cvtsi64x_sd(a, b) _mm_cvtsi64_sd((a), (b))
__BCC_SSE __m128d _mm_cvtss_sd(__m128d __a, __m128 __b) {
  __a[0] = __b[0];
  return __a;
}
__BCC_SSE double _mm_cvtsd_f64(__m128d __a) { return __a[0]; }
__BCC_SSE __m128 _mm_cvtepi32_ps(__m128i __a) {
  __v4si __x = (__v4si)__a;
  return (__m128){(float)__x[0], (float)__x[1], (float)__x[2], (float)__x[3]};
}
__BCC_SSE __m128i _mm_cvtps_epi32(__m128 __a) {
  __v4si __r;
  int __i;
  for (__i = 0; __i < 4; ++__i)
    __r[__i] = __bcc_to_i32(__builtin_nearbyintf(__a[__i]));
  return (__m128i)__r;
}
__BCC_SSE __m128i _mm_cvttps_epi32(__m128 __a) {
  __v4si __r;
  int __i;
  for (__i = 0; __i < 4; ++__i)
    __r[__i] = __bcc_to_i32(__builtin_truncf(__a[__i]));
  return (__m128i)__r;
}
__BCC_SSE __m128i _mm_cvtsi32_si128(int __a) { return (__m128i)(__v4si){__a, 0, 0, 0}; }
__BCC_SSE __m128i _mm_cvtsi64_si128(long long __a) { return (__m128i){__a, 0}; }
#define _mm_cvtsi64x_si128(a) _mm_cvtsi64_si128(a)
__BCC_SSE int _mm_cvtsi128_si32(__m128i __a) { return ((__v4si)__a)[0]; }
__BCC_SSE long long _mm_cvtsi128_si64(__m128i __a) { return __a[0]; }
#define _mm_cvtsi128_si64x(a) _mm_cvtsi128_si64(a)

/* Double-precision loads, stores and construction */

__BCC_SSE __m128d _mm_load_pd(const double *__p) { return *(const __m128d *)__p; }
__BCC_SSE __m128d _mm_loadu_pd(const double *__p) { return *(const __m128d_u *)__p; }
__BCC_SSE __m128d _mm_load1_pd(const double *__p) {
  double __x;
  __builtin_memcpy(&__x, __p, sizeof __x);
  return (__m128d){__x, __x};
}
#define _mm_load_pd1(p) _mm_load1_pd(p)
__BCC_SSE __m128d _mm_loadr_pd(const double *__p) {
  __m128d __a = *(const __m128d *)__p;
  return (__m128d){__a[1], __a[0]};
}
__BCC_SSE __m128d _mm_load_sd(const double *__p) {
  double __x;
  __builtin_memcpy(&__x, __p, sizeof __x);
  return (__m128d){__x, 0};
}
__BCC_SSE __m128d _mm_loadh_pd(__m128d __a, const double *__p) {
  __builtin_memcpy((char *)&__a + 8, __p, 8);
  return __a;
}
__BCC_SSE __m128d _mm_loadl_pd(__m128d __a, const double *__p) {
  __builtin_memcpy(&__a, __p, 8);
  return __a;
}
__BCC_SSE void _mm_store_pd(double *__p, __m128d __a) { *(__m128d *)__p = __a; }
__BCC_SSE void _mm_storeu_pd(double *__p, __m128d __a) { *(__m128d_u *)__p = __a; }
__BCC_SSE void _mm_store1_pd(double *__p, __m128d __a) { *(__m128d *)__p = (__m128d){__a[0], __a[0]}; }
#define _mm_store_pd1(p, a) _mm_store1_pd((p), (a))
__BCC_SSE void _mm_storer_pd(double *__p, __m128d __a) { *(__m128d *)__p = (__m128d){__a[1], __a[0]}; }
__BCC_SSE void _mm_store_sd(double *__p, __m128d __a) { __builtin_memcpy(__p, &__a, 8); }
__BCC_SSE void _mm_storeh_pd(double *__p, __m128d __a) { __builtin_memcpy(__p, (char *)&__a + 8, 8); }
__BCC_SSE void _mm_storel_pd(double *__p, __m128d __a) { __builtin_memcpy(__p, &__a, 8); }
__BCC_SSE void _mm_stream_pd(double *__p, __m128d __a) { *(__m128d *)__p = __a; }
__BCC_SSE __m128d _mm_set_sd(double __w) { return (__m128d){__w, 0}; }
__BCC_SSE __m128d _mm_set1_pd(double __w) { return (__m128d){__w, __w}; }
__BCC_SSE __m128d _mm_set_pd1(double __w) { return (__m128d){__w, __w}; }
__BCC_SSE __m128d _mm_set_pd(double __w, double __x) { return (__m128d){__x, __w}; }
__BCC_SSE __m128d _mm_setr_pd(double __w, double __x) { return (__m128d){__w, __x}; }
__BCC_SSE __m128d _mm_setzero_pd(void) { return (__m128d){0, 0}; }
__BCC_SSE __m128d _mm_undefined_pd(void) { return (__m128d){0, 0}; }
__BCC_SSE __m128d _mm_move_sd(__m128d __a, __m128d __b) { return __bcc_low_sd(__a, __b); }
__BCC_SSE __m128d _mm_unpackhi_pd(__m128d __a, __m128d __b) { return (__m128d){__a[1], __b[1]}; }
__BCC_SSE __m128d _mm_unpacklo_pd(__m128d __a, __m128d __b) { return (__m128d){__a[0], __b[0]}; }
__BCC_SSE int _mm_movemask_pd(__m128d __a) {
  __v2di __s = (__v2di)__a < 0;
  return (int)((__s[0] & 1) | (__s[1] & 2));
}
__BCC_SSE __m128d __bcc_shuffle_pd(__m128d __a, __m128d __b, int __i) {
  return (__m128d){__a[__i & 1], __b[(__i >> 1) & 1]};
}
/* Clang takes a selector from 0 to 3, GCC any constant, of which the
 * instruction reads the low two bits; the checked builtin has Clang's range. */
#if defined(__clang__)
#define __BCC_SHUFPD_IMMEDIATE(i) (i)
#else
#define __BCC_SHUFPD_IMMEDIATE(i) ((int)(i) & 3)
#endif
#define _mm_shuffle_pd(a, b, i)                                                 \
  __BCC_IMMEDIATE(__builtin_ia32_shufpd((__v2df)(__m128d)(a), (__v2df)(__m128d)(b),        \
                                        __BCC_SHUFPD_IMMEDIATE(i)),                         \
                  __bcc_shuffle_pd((__m128d)(a), (__m128d)(b), (int)(i)))
#define _MM_SHUFFLE2(x, y) (((x) << 1) | (y))

/* Integer arithmetic. Lanes wrap unless the name says they saturate. */

__BCC_SSE __m128i _mm_add_epi8(__m128i __a, __m128i __b) { return (__m128i)((__v16qu)__a + (__v16qu)__b); }
__BCC_SSE __m128i _mm_add_epi16(__m128i __a, __m128i __b) { return (__m128i)((__v8hu)__a + (__v8hu)__b); }
__BCC_SSE __m128i _mm_add_epi32(__m128i __a, __m128i __b) { return (__m128i)((__v4su)__a + (__v4su)__b); }
__BCC_SSE __m128i _mm_add_epi64(__m128i __a, __m128i __b) { return (__m128i)((__v2du)__a + (__v2du)__b); }
__BCC_SSE __m64 _mm_add_si64(__m64 __a, __m64 __b) { return (__m64)((__v1du)__a + (__v1du)__b); }
__BCC_SSE __m128i _mm_sub_epi8(__m128i __a, __m128i __b) { return (__m128i)((__v16qu)__a - (__v16qu)__b); }
__BCC_SSE __m128i _mm_sub_epi16(__m128i __a, __m128i __b) { return (__m128i)((__v8hu)__a - (__v8hu)__b); }
__BCC_SSE __m128i _mm_sub_epi32(__m128i __a, __m128i __b) { return (__m128i)((__v4su)__a - (__v4su)__b); }
__BCC_SSE __m128i _mm_sub_epi64(__m128i __a, __m128i __b) { return (__m128i)((__v2du)__a - (__v2du)__b); }
__BCC_SSE __m64 _mm_sub_si64(__m64 __a, __m64 __b) { return (__m64)((__v1du)__a - (__v1du)__b); }

#define __BCC_CLAMP(x, low, high) ((x) < (low) ? (low) : (x) > (high) ? (high) : (x))

__BCC_SSE __m128i _mm_adds_epi8(__m128i __a, __m128i __b) {
  __v16qs __x = (__v16qs)__a, __y = (__v16qs)__b, __r;
  int __i;
  for (__i = 0; __i < 16; ++__i)
    __r[__i] = (signed char)__BCC_CLAMP(__x[__i] + __y[__i], -128, 127);
  return (__m128i)__r;
}
__BCC_SSE __m128i _mm_adds_epi16(__m128i __a, __m128i __b) {
  __v8hi __x = (__v8hi)__a, __y = (__v8hi)__b, __r;
  int __i;
  for (__i = 0; __i < 8; ++__i)
    __r[__i] = (short)__BCC_CLAMP(__x[__i] + __y[__i], -32768, 32767);
  return (__m128i)__r;
}
__BCC_SSE __m128i _mm_adds_epu8(__m128i __a, __m128i __b) {
  __v16qu __x = (__v16qu)__a, __y = (__v16qu)__b, __r;
  int __i;
  for (__i = 0; __i < 16; ++__i)
    __r[__i] = (unsigned char)__BCC_CLAMP(__x[__i] + __y[__i], 0, 255);
  return (__m128i)__r;
}
__BCC_SSE __m128i _mm_adds_epu16(__m128i __a, __m128i __b) {
  __v8hu __x = (__v8hu)__a, __y = (__v8hu)__b, __r;
  int __i;
  for (__i = 0; __i < 8; ++__i)
    __r[__i] = (unsigned short)__BCC_CLAMP(__x[__i] + __y[__i], 0, 65535);
  return (__m128i)__r;
}
__BCC_SSE __m128i _mm_subs_epi8(__m128i __a, __m128i __b) {
  __v16qs __x = (__v16qs)__a, __y = (__v16qs)__b, __r;
  int __i;
  for (__i = 0; __i < 16; ++__i)
    __r[__i] = (signed char)__BCC_CLAMP(__x[__i] - __y[__i], -128, 127);
  return (__m128i)__r;
}
__BCC_SSE __m128i _mm_subs_epi16(__m128i __a, __m128i __b) {
  __v8hi __x = (__v8hi)__a, __y = (__v8hi)__b, __r;
  int __i;
  for (__i = 0; __i < 8; ++__i)
    __r[__i] = (short)__BCC_CLAMP(__x[__i] - __y[__i], -32768, 32767);
  return (__m128i)__r;
}
__BCC_SSE __m128i _mm_subs_epu8(__m128i __a, __m128i __b) {
  __v16qu __x = (__v16qu)__a, __y = (__v16qu)__b, __r;
  int __i;
  for (__i = 0; __i < 16; ++__i)
    __r[__i] = (unsigned char)__BCC_CLAMP(__x[__i] - __y[__i], 0, 255);
  return (__m128i)__r;
}
__BCC_SSE __m128i _mm_subs_epu16(__m128i __a, __m128i __b) {
  __v8hu __x = (__v8hu)__a, __y = (__v8hu)__b, __r;
  int __i;
  for (__i = 0; __i < 8; ++__i)
    __r[__i] = (unsigned short)__BCC_CLAMP(__x[__i] - __y[__i], 0, 65535);
  return (__m128i)__r;
}
__BCC_SSE __m128i _mm_avg_epu8(__m128i __a, __m128i __b) {
  __v16qu __x = (__v16qu)__a, __y = (__v16qu)__b, __r;
  int __i;
  for (__i = 0; __i < 16; ++__i)
    __r[__i] = (unsigned char)((__x[__i] + __y[__i] + 1) >> 1);
  return (__m128i)__r;
}
__BCC_SSE __m128i _mm_avg_epu16(__m128i __a, __m128i __b) {
  __v8hu __x = (__v8hu)__a, __y = (__v8hu)__b, __r;
  int __i;
  for (__i = 0; __i < 8; ++__i)
    __r[__i] = (unsigned short)((__x[__i] + __y[__i] + 1u) >> 1);
  return (__m128i)__r;
}
__BCC_SSE __m128i _mm_madd_epi16(__m128i __a, __m128i __b) {
  __v8hi __x = (__v8hi)__a, __y = (__v8hi)__b;
  __v4su __r;
  int __i;
  for (__i = 0; __i < 4; ++__i)
    __r[__i] = (unsigned)(__x[2 * __i] * __y[2 * __i]) + (unsigned)(__x[2 * __i + 1] * __y[2 * __i + 1]);
  return (__m128i)__r;
}
__BCC_SSE __m128i _mm_max_epi16(__m128i __a, __m128i __b) {
  __v8hi __x = (__v8hi)__a, __y = (__v8hi)__b;
  return (__m128i)((__x > __y) & __x | ~(__x > __y) & __y);
}
__BCC_SSE __m128i _mm_min_epi16(__m128i __a, __m128i __b) {
  __v8hi __x = (__v8hi)__a, __y = (__v8hi)__b;
  return (__m128i)((__x < __y) & __x | ~(__x < __y) & __y);
}
__BCC_SSE __m128i _mm_max_epu8(__m128i __a, __m128i __b) {
  __v16qu __x = (__v16qu)__a, __y = (__v16qu)__b, __m = (__v16qu)(__x > __y);
  return (__m128i)(__m & __x | ~__m & __y);
}
__BCC_SSE __m128i _mm_min_epu8(__m128i __a, __m128i __b) {
  __v16qu __x = (__v16qu)__a, __y = (__v16qu)__b, __m = (__v16qu)(__x < __y);
  return (__m128i)(__m & __x | ~__m & __y);
}
__BCC_SSE __m128i _mm_mulhi_epi16(__m128i __a, __m128i __b) {
  __v8hi __x = (__v8hi)__a, __y = (__v8hi)__b, __r;
  int __i;
  for (__i = 0; __i < 8; ++__i)
    __r[__i] = (short)((__x[__i] * __y[__i]) >> 16);
  return (__m128i)__r;
}
__BCC_SSE __m128i _mm_mulhi_epu16(__m128i __a, __m128i __b) {
  __v8hu __x = (__v8hu)__a, __y = (__v8hu)__b, __r;
  int __i;
  for (__i = 0; __i < 8; ++__i)
    __r[__i] = (unsigned short)(((unsigned)__x[__i] * __y[__i]) >> 16);
  return (__m128i)__r;
}
__BCC_SSE __m128i _mm_mullo_epi16(__m128i __a, __m128i __b) { return (__m128i)((__v8hu)__a * (__v8hu)__b); }
__BCC_SSE __m64 _mm_mul_su32(__m64 __a, __m64 __b) {
  return (__m64)(__v1du){(unsigned long long)((__v2su)__a)[0] * ((__v2su)__b)[0]};
}
__BCC_SSE __m128i _mm_mul_epu32(__m128i __a, __m128i __b) {
  __v4su __x = (__v4su)__a, __y = (__v4su)__b;
  return (__m128i)(__v2du){(unsigned long long)__x[0] * __y[0], (unsigned long long)__x[2] * __y[2]};
}
__BCC_SSE __m128i _mm_sad_epu8(__m128i __a, __m128i __b) {
  __v16qu __x = (__v16qu)__a, __y = (__v16qu)__b;
  __v2du __r = {0, 0};
  int __i;
  for (__i = 0; __i < 16; ++__i)
    __r[__i / 8] += __x[__i] > __y[__i] ? __x[__i] - __y[__i] : __y[__i] - __x[__i];
  return (__m128i)__r;
}

/* Integer bitwise operations and shifts. A count beyond a lane's width
 * clears it, or fills it with the sign for an arithmetic shift. */

__BCC_SSE __m128i _mm_and_si128(__m128i __a, __m128i __b) { return (__m128i)((__v2du)__a & (__v2du)__b); }
__BCC_SSE __m128i _mm_andnot_si128(__m128i __a, __m128i __b) { return (__m128i)(~(__v2du)__a & (__v2du)__b); }
__BCC_SSE __m128i _mm_or_si128(__m128i __a, __m128i __b) { return (__m128i)((__v2du)__a | (__v2du)__b); }
__BCC_SSE __m128i _mm_xor_si128(__m128i __a, __m128i __b) { return (__m128i)((__v2du)__a ^ (__v2du)__b); }

__BCC_SSE __m128i _mm_slli_epi16(__m128i __a, int __n) {
  return (unsigned)__n > 15 ? (__m128i){0, 0} : (__m128i)((__v8hu)__a << __n);
}
__BCC_SSE __m128i _mm_slli_epi32(__m128i __a, int __n) {
  return (unsigned)__n > 31 ? (__m128i){0, 0} : (__m128i)((__v4su)__a << __n);
}
__BCC_SSE __m128i _mm_slli_epi64(__m128i __a, int __n) {
  return (unsigned)__n > 63 ? (__m128i){0, 0} : (__m128i)((__v2du)__a << __n);
}
__BCC_SSE __m128i _mm_srli_epi16(__m128i __a, int __n) {
  return (unsigned)__n > 15 ? (__m128i){0, 0} : (__m128i)((__v8hu)__a >> __n);
}
__BCC_SSE __m128i _mm_srli_epi32(__m128i __a, int __n) {
  return (unsigned)__n > 31 ? (__m128i){0, 0} : (__m128i)((__v4su)__a >> __n);
}
__BCC_SSE __m128i _mm_srli_epi64(__m128i __a, int __n) {
  return (unsigned)__n > 63 ? (__m128i){0, 0} : (__m128i)((__v2du)__a >> __n);
}
__BCC_SSE __m128i _mm_srai_epi16(__m128i __a, int __n) {
  return (__m128i)((__v8hi)__a >> ((unsigned)__n > 15 ? 15 : __n));
}
__BCC_SSE __m128i _mm_srai_epi32(__m128i __a, int __n) {
  return (__m128i)((__v4si)__a >> ((unsigned)__n > 31 ? 31 : __n));
}
/* The `sll`, `srl` and `sra` forms take the count from the low 64 bits of
 * a vector. */
__BCC_SSE int __bcc_count(__m128i __c, unsigned __limit) {
  unsigned long long __n = (unsigned long long)__c[0];
  return __n > __limit ? (int)__limit + 1 : (int)__n;
}
__BCC_SSE __m128i _mm_sll_epi16(__m128i __a, __m128i __c) { return _mm_slli_epi16(__a, __bcc_count(__c, 15)); }
__BCC_SSE __m128i _mm_sll_epi32(__m128i __a, __m128i __c) { return _mm_slli_epi32(__a, __bcc_count(__c, 31)); }
__BCC_SSE __m128i _mm_sll_epi64(__m128i __a, __m128i __c) { return _mm_slli_epi64(__a, __bcc_count(__c, 63)); }
__BCC_SSE __m128i _mm_srl_epi16(__m128i __a, __m128i __c) { return _mm_srli_epi16(__a, __bcc_count(__c, 15)); }
__BCC_SSE __m128i _mm_srl_epi32(__m128i __a, __m128i __c) { return _mm_srli_epi32(__a, __bcc_count(__c, 31)); }
__BCC_SSE __m128i _mm_srl_epi64(__m128i __a, __m128i __c) { return _mm_srli_epi64(__a, __bcc_count(__c, 63)); }
__BCC_SSE __m128i _mm_sra_epi16(__m128i __a, __m128i __c) { return _mm_srai_epi16(__a, __bcc_count(__c, 15)); }
__BCC_SSE __m128i _mm_sra_epi32(__m128i __a, __m128i __c) { return _mm_srai_epi32(__a, __bcc_count(__c, 31)); }

/* Byte shifts of the whole register; a count above 15 clears it. */
__BCC_SSE __m128i __bcc_byte_shift(__m128i __a, int __n, int __left) {
  __v16qu __x = (__v16qu)__a, __r = (__v16qu){0};
  int __i;
  if (__n > 15)
    return (__m128i)__r;
  for (__i = 0; __i < 16; ++__i) {
    int __from = __left ? __i - __n : __i + __n;
    if (__from >= 0 && __from < 16)
      __r[__i] = __x[__from];
  }
  return (__m128i)__r;
}
#define _mm_slli_si128(a, imm)                                                  \
  __BCC_IMMEDIATE(__builtin_ia32_pslldqi128_byteshift((__v16qi)(__m128i)(a), (int)(imm)), \
                  __bcc_byte_shift((__m128i)(a), (int)(imm) & 255, 1))
#define _mm_srli_si128(a, imm)                                                  \
  __BCC_IMMEDIATE(__builtin_ia32_psrldqi128_byteshift((__v16qi)(__m128i)(a), (int)(imm)), \
                  __bcc_byte_shift((__m128i)(a), (int)(imm) & 255, 0))
#define _mm_bslli_si128(a, imm) _mm_slli_si128((a), (imm))
#define _mm_bsrli_si128(a, imm) _mm_srli_si128((a), (imm))

/* Integer comparisons */

__BCC_SSE __m128i _mm_cmpeq_epi8(__m128i __a, __m128i __b) { return (__m128i)((__v16qs)__a == (__v16qs)__b); }
__BCC_SSE __m128i _mm_cmpeq_epi16(__m128i __a, __m128i __b) { return (__m128i)((__v8hi)__a == (__v8hi)__b); }
__BCC_SSE __m128i _mm_cmpeq_epi32(__m128i __a, __m128i __b) { return (__m128i)((__v4si)__a == (__v4si)__b); }
__BCC_SSE __m128i _mm_cmpgt_epi8(__m128i __a, __m128i __b) { return (__m128i)((__v16qs)__a > (__v16qs)__b); }
__BCC_SSE __m128i _mm_cmpgt_epi16(__m128i __a, __m128i __b) { return (__m128i)((__v8hi)__a > (__v8hi)__b); }
__BCC_SSE __m128i _mm_cmpgt_epi32(__m128i __a, __m128i __b) { return (__m128i)((__v4si)__a > (__v4si)__b); }
__BCC_SSE __m128i _mm_cmplt_epi8(__m128i __a, __m128i __b) { return _mm_cmpgt_epi8(__b, __a); }
__BCC_SSE __m128i _mm_cmplt_epi16(__m128i __a, __m128i __b) { return _mm_cmpgt_epi16(__b, __a); }
__BCC_SSE __m128i _mm_cmplt_epi32(__m128i __a, __m128i __b) { return _mm_cmpgt_epi32(__b, __a); }

/* Integer construction, loads and stores */

__BCC_SSE __m128i _mm_set_epi64x(long long __q1, long long __q0) { return (__m128i){__q0, __q1}; }
__BCC_SSE __m128i _mm_set_epi64(__m64 __q1, __m64 __q0) {
  return (__m128i){((__v1di)__q0)[0], ((__v1di)__q1)[0]};
}
__BCC_SSE __m128i _mm_set_epi32(int __i3, int __i2, int __i1, int __i0) {
  return (__m128i)(__v4si){__i0, __i1, __i2, __i3};
}
__BCC_SSE __m128i _mm_set_epi16(short __w7, short __w6, short __w5, short __w4, short __w3,
                                short __w2, short __w1, short __w0) {
  return (__m128i)(__v8hi){__w0, __w1, __w2, __w3, __w4, __w5, __w6, __w7};
}
__BCC_SSE __m128i _mm_set_epi8(char __b15, char __b14, char __b13, char __b12, char __b11,
                               char __b10, char __b9, char __b8, char __b7, char __b6, char __b5,
                               char __b4, char __b3, char __b2, char __b1, char __b0) {
  return (__m128i)(__v16qi){__b0, __b1, __b2,  __b3,  __b4,  __b5,  __b6,  __b7,
                            __b8, __b9, __b10, __b11, __b12, __b13, __b14, __b15};
}
__BCC_SSE __m128i _mm_set1_epi64x(long long __q) { return (__m128i){__q, __q}; }
__BCC_SSE __m128i _mm_set1_epi64(__m64 __q) { return _mm_set1_epi64x(((__v1di)__q)[0]); }
__BCC_SSE __m128i _mm_set1_epi32(int __i) { return (__m128i)(__v4si){__i, __i, __i, __i}; }
__BCC_SSE __m128i _mm_set1_epi16(short __w) {
  return (__m128i)(__v8hi){__w, __w, __w, __w, __w, __w, __w, __w};
}
__BCC_SSE __m128i _mm_set1_epi8(char __b) {
  return (__m128i)(__v16qi){__b, __b, __b, __b, __b, __b, __b, __b,
                            __b, __b, __b, __b, __b, __b, __b, __b};
}
__BCC_SSE __m128i _mm_setr_epi64(__m64 __q0, __m64 __q1) { return _mm_set_epi64(__q1, __q0); }
__BCC_SSE __m128i _mm_setr_epi32(int __i0, int __i1, int __i2, int __i3) {
  return (__m128i)(__v4si){__i0, __i1, __i2, __i3};
}
__BCC_SSE __m128i _mm_setr_epi16(short __w0, short __w1, short __w2, short __w3, short __w4,
                                 short __w5, short __w6, short __w7) {
  return (__m128i)(__v8hi){__w0, __w1, __w2, __w3, __w4, __w5, __w6, __w7};
}
__BCC_SSE __m128i _mm_setr_epi8(char __b0, char __b1, char __b2, char __b3, char __b4, char __b5,
                                char __b6, char __b7, char __b8, char __b9, char __b10,
                                char __b11, char __b12, char __b13, char __b14, char __b15) {
  return (__m128i)(__v16qi){__b0, __b1, __b2,  __b3,  __b4,  __b5,  __b6,  __b7,
                            __b8, __b9, __b10, __b11, __b12, __b13, __b14, __b15};
}
__BCC_SSE __m128i _mm_setzero_si128(void) { return (__m128i){0, 0}; }
__BCC_SSE __m128i _mm_undefined_si128(void) { return (__m128i){0, 0}; }

__BCC_SSE __m128i _mm_load_si128(const __m128i *__p) { return *__p; }
__BCC_SSE __m128i _mm_loadu_si128(const __m128i_u *__p) { return *__p; }
__BCC_SSE __m128i _mm_loadl_epi64(const __m128i_u *__p) {
  long long __x;
  __builtin_memcpy(&__x, __p, 8);
  return (__m128i){__x, 0};
}
__BCC_SSE __m128i _mm_loadu_si64(const void *__p) {
  long long __x;
  __builtin_memcpy(&__x, __p, 8);
  return (__m128i){__x, 0};
}
__BCC_SSE __m128i _mm_loadu_si32(const void *__p) {
  int __x;
  __builtin_memcpy(&__x, __p, 4);
  return (__m128i)(__v4si){__x, 0, 0, 0};
}
__BCC_SSE __m128i _mm_loadu_si16(const void *__p) {
  short __x;
  __builtin_memcpy(&__x, __p, 2);
  return (__m128i)(__v8hi){__x, 0, 0, 0, 0, 0, 0, 0};
}
__BCC_SSE void _mm_store_si128(__m128i *__p, __m128i __b) { *__p = __b; }
__BCC_SSE void _mm_storeu_si128(__m128i_u *__p, __m128i __b) { *__p = __b; }
__BCC_SSE void _mm_storeu_si64(void *__p, __m128i __b) { __builtin_memcpy(__p, &__b, 8); }
__BCC_SSE void _mm_storeu_si32(void *__p, __m128i __b) { __builtin_memcpy(__p, &__b, 4); }
__BCC_SSE void _mm_storeu_si16(void *__p, __m128i __b) { __builtin_memcpy(__p, &__b, 2); }
__BCC_SSE void _mm_storel_epi64(__m128i_u *__p, __m128i __a) { __builtin_memcpy(__p, &__a, 8); }
__BCC_SSE void _mm_maskmoveu_si128(__m128i __d, __m128i __n, char *__p) {
  __v16qs __x = (__v16qs)__d, __m = (__v16qs)__n;
  int __i;
  for (__i = 0; __i < 16; ++__i)
    if (__m[__i] < 0)
      __p[__i] = __x[__i];
}
__BCC_SSE void _mm_stream_si128(__m128i *__p, __m128i __a) { *__p = __a; }
__BCC_SSE void _mm_stream_si32(int *__p, int __a) { *__p = __a; }
__BCC_SSE void _mm_stream_si64(long long *__p, long long __a) { *__p = __a; }
__BCC_SSE __m128i _mm_move_epi64(__m128i __a) { return (__m128i){__a[0], 0}; }
__BCC_SSE __m64 _mm_movepi64_pi64(__m128i __a) { return (__m64)(__v1di){__a[0]}; }
__BCC_SSE __m128i _mm_movpi64_epi64(__m64 __a) { return (__m128i){((__v1di)__a)[0], 0}; }

/* Packs, unpacks and shuffles */

__BCC_SSE __m128i _mm_packs_epi16(__m128i __a, __m128i __b) {
  __v8hi __x = (__v8hi)__a, __y = (__v8hi)__b;
  __v16qs __r;
  int __i;
  for (__i = 0; __i < 8; ++__i) {
    __r[__i] = (signed char)__BCC_CLAMP(__x[__i], -128, 127);
    __r[__i + 8] = (signed char)__BCC_CLAMP(__y[__i], -128, 127);
  }
  return (__m128i)__r;
}
__BCC_SSE __m128i _mm_packs_epi32(__m128i __a, __m128i __b) {
  __v4si __x = (__v4si)__a, __y = (__v4si)__b;
  __v8hi __r;
  int __i;
  for (__i = 0; __i < 4; ++__i) {
    __r[__i] = (short)__BCC_CLAMP(__x[__i], -32768, 32767);
    __r[__i + 4] = (short)__BCC_CLAMP(__y[__i], -32768, 32767);
  }
  return (__m128i)__r;
}
__BCC_SSE __m128i _mm_packus_epi16(__m128i __a, __m128i __b) {
  __v8hi __x = (__v8hi)__a, __y = (__v8hi)__b;
  __v16qu __r;
  int __i;
  for (__i = 0; __i < 8; ++__i) {
    __r[__i] = (unsigned char)__BCC_CLAMP(__x[__i], 0, 255);
    __r[__i + 8] = (unsigned char)__BCC_CLAMP(__y[__i], 0, 255);
  }
  return (__m128i)__r;
}
__BCC_SSE int __bcc_extract_epi16(__m128i __a, int __n) { return ((__v8hu)__a)[__n & 7]; }
#define _mm_extract_epi16(a, imm)                                               \
  __BCC_IMMEDIATE(__builtin_ia32_vec_ext_v8hi((__v8hi)(__m128i)(a), (int)(imm)), \
                  __bcc_extract_epi16((__m128i)(a), (int)(imm)))
__BCC_SSE __m128i __bcc_insert_epi16(__m128i __a, int __b, int __n) {
  __v8hi __r = (__v8hi)__a;
  __r[__n & 7] = (short)__b;
  return (__m128i)__r;
}
#define _mm_insert_epi16(a, b, imm)                                             \
  __BCC_IMMEDIATE(__builtin_ia32_vec_set_v8hi((__v8hi)(__m128i)(a), (int)(b), (int)(imm)), \
                  __bcc_insert_epi16((__m128i)(a), (int)(b), (int)(imm)))
__BCC_SSE int _mm_movemask_epi8(__m128i __a) {
  __v16qs __x = (__v16qs)__a;
  int __r = 0, __i;
  for (__i = 0; __i < 16; ++__i)
    __r |= (__x[__i] < 0) << __i;
  return __r;
}
__BCC_SSE __m128i __bcc_shuffle_epi32(__m128i __a, int __imm) {
  __v4si __x = (__v4si)__a;
  return (__m128i)(__v4si){__x[__imm & 3], __x[(__imm >> 2) & 3], __x[(__imm >> 4) & 3],
                           __x[(__imm >> 6) & 3]};
}
#define _mm_shuffle_epi32(a, imm)                                               \
  __BCC_IMMEDIATE(__builtin_ia32_pshufd((__v4si)(__m128i)(a), (int)(imm)),      \
                  __bcc_shuffle_epi32((__m128i)(a), (int)(imm)))
__BCC_SSE __m128i __bcc_shuffle_epi16(__m128i __a, int __imm, int __high) {
  __v8hi __x = (__v8hi)__a, __r = __x;
  int __base = __high ? 4 : 0, __i;
  for (__i = 0; __i < 4; ++__i)
    __r[__base + __i] = __x[__base + ((__imm >> (2 * __i)) & 3)];
  return (__m128i)__r;
}
#define _mm_shufflelo_epi16(a, imm)                                             \
  __BCC_IMMEDIATE(__builtin_ia32_pshuflw((__v8hi)(__m128i)(a), (int)(imm)),     \
                  __bcc_shuffle_epi16((__m128i)(a), (int)(imm), 0))
#define _mm_shufflehi_epi16(a, imm)                                             \
  __BCC_IMMEDIATE(__builtin_ia32_pshufhw((__v8hi)(__m128i)(a), (int)(imm)),     \
                  __bcc_shuffle_epi16((__m128i)(a), (int)(imm), 1))

__BCC_SSE __m128i _mm_unpackhi_epi8(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_shufflevector((__v16qi)__a, (__v16qi)__b, 8, 24, 9, 25, 10, 26, 11, 27,
                                          12, 28, 13, 29, 14, 30, 15, 31);
}
__BCC_SSE __m128i _mm_unpackhi_epi16(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_shufflevector((__v8hi)__a, (__v8hi)__b, 4, 12, 5, 13, 6, 14, 7, 15);
}
__BCC_SSE __m128i _mm_unpackhi_epi32(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_shufflevector((__v4si)__a, (__v4si)__b, 2, 6, 3, 7);
}
__BCC_SSE __m128i _mm_unpackhi_epi64(__m128i __a, __m128i __b) { return (__m128i){__a[1], __b[1]}; }
__BCC_SSE __m128i _mm_unpacklo_epi8(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_shufflevector((__v16qi)__a, (__v16qi)__b, 0, 16, 1, 17, 2, 18, 3, 19,
                                          4, 20, 5, 21, 6, 22, 7, 23);
}
__BCC_SSE __m128i _mm_unpacklo_epi16(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_shufflevector((__v8hi)__a, (__v8hi)__b, 0, 8, 1, 9, 2, 10, 3, 11);
}
__BCC_SSE __m128i _mm_unpacklo_epi32(__m128i __a, __m128i __b) {
  return (__m128i)__builtin_shufflevector((__v4si)__a, (__v4si)__b, 0, 4, 1, 5);
}
__BCC_SSE __m128i _mm_unpacklo_epi64(__m128i __a, __m128i __b) { return (__m128i){__a[0], __b[0]}; }

/* Casts reinterpret the bits. */

__BCC_SSE __m128 _mm_castpd_ps(__m128d __a) { return (__m128)__a; }
__BCC_SSE __m128i _mm_castpd_si128(__m128d __a) { return (__m128i)__a; }
__BCC_SSE __m128d _mm_castps_pd(__m128 __a) { return (__m128d)__a; }
__BCC_SSE __m128i _mm_castps_si128(__m128 __a) { return (__m128i)__a; }
__BCC_SSE __m128 _mm_castsi128_ps(__m128i __a) { return (__m128)__a; }
__BCC_SSE __m128d _mm_castsi128_pd(__m128i __a) { return (__m128d)__a; }

/* Cache control and fences */

__BCC_SSE void _mm_clflush(const void *__p) { __asm__ __volatile__("clflush %0" ::"m"(*(const char *)__p)); }
__BCC_SSE void _mm_lfence(void) { __asm__ __volatile__("lfence" ::: "memory"); }
__BCC_SSE void _mm_mfence(void) { __asm__ __volatile__("mfence" ::: "memory"); }
__BCC_SSE void _mm_pause(void) { __asm__ __volatile__("pause"); }

#endif /* __EMMINTRIN_H */
