/* smmintrin.h - SSE4.1 and SSE4.2 intrinsics for bcc.
 *
 * Written for bcc; part of bcc and under its licence. Plain C over GNU
 * vector operations, as in <xmmintrin.h>; the string comparisons follow
 * the PCMPxSTRx definition of Intel's Software Developer's Manual. */

#ifndef __SMMINTRIN_H
#define __SMMINTRIN_H

#if !defined(__i386__) && !defined(__x86_64__)
#error "This header is only meant to be used on x86 and x64 architecture"
#endif

#include <tmmintrin.h>

/* Rounding */

#define _MM_FROUND_TO_NEAREST_INT 0x00
#define _MM_FROUND_TO_NEG_INF 0x01
#define _MM_FROUND_TO_POS_INF 0x02
#define _MM_FROUND_TO_ZERO 0x03
#define _MM_FROUND_CUR_DIRECTION 0x04
#define _MM_FROUND_RAISE_EXC 0x00
#define _MM_FROUND_NO_EXC 0x08
#define _MM_FROUND_NINT (_MM_FROUND_RAISE_EXC | _MM_FROUND_TO_NEAREST_INT)
#define _MM_FROUND_FLOOR (_MM_FROUND_RAISE_EXC | _MM_FROUND_TO_NEG_INF)
#define _MM_FROUND_CEIL (_MM_FROUND_RAISE_EXC | _MM_FROUND_TO_POS_INF)
#define _MM_FROUND_TRUNC (_MM_FROUND_RAISE_EXC | _MM_FROUND_TO_ZERO)
#define _MM_FROUND_RINT (_MM_FROUND_RAISE_EXC | _MM_FROUND_CUR_DIRECTION)
#define _MM_FROUND_NEARBYINT (_MM_FROUND_NO_EXC | _MM_FROUND_CUR_DIRECTION)

/* `__x` rounded to an integer as the ROUNDPS immediate `__m` asks: by the
 * current mode, or to nearest even, down, up or toward zero. */
__BCC_SSE double __bcc_round(double __x, int __m) {
  double __r;
  if (__m & 4)
    return __builtin_nearbyint(__x);
  switch (__m & 3) {
  case 1: return __builtin_floor(__x);
  case 2: return __builtin_ceil(__x);
  case 3: return __builtin_trunc(__x);
  default:
    if (!(__builtin_fabs(__x) < 0x1p52))
      return __x;
    __r = __builtin_floor(__x);
    if (__x - __r > 0.5 || (__x - __r == 0.5 && __builtin_fmod(__r, 2.0) != 0))
      __r += 1;
    return __builtin_copysign(__r, __x);
  }
}
__BCC_SSE __m128 __bcc_round_ps(__m128 __a, int __m) {
  return (__m128){(float)__bcc_round(__a[0], __m), (float)__bcc_round(__a[1], __m),
                  (float)__bcc_round(__a[2], __m), (float)__bcc_round(__a[3], __m)};
}
__BCC_SSE __m128d __bcc_round_pd(__m128d __a, int __m) {
  return (__m128d){__bcc_round(__a[0], __m), __bcc_round(__a[1], __m)};
}
#define _mm_round_ps(X, M)                                                      \
  __BCC_IMMEDIATE(__builtin_ia32_roundps((__v4sf)(__m128)(X), (M)),             \
                  __bcc_round_ps((__m128)(X), (int)(M)))
#define _mm_round_ss(X, Y, M)                                                   \
  __BCC_IMMEDIATE(__builtin_ia32_roundss((__v4sf)(__m128)(X), (__v4sf)(__m128)(Y), (M)), \
                  __bcc_low_ss((__m128)(X), __bcc_round_ps((__m128)(Y), (int)(M))))
#define _mm_round_pd(X, M)                                                      \
  __BCC_IMMEDIATE(__builtin_ia32_roundpd((__v2df)(__m128d)(X), (M)),            \
                  __bcc_round_pd((__m128d)(X), (int)(M)))
#define _mm_round_sd(X, Y, M)                                                   \
  __BCC_IMMEDIATE(__builtin_ia32_roundsd((__v2df)(__m128d)(X), (__v2df)(__m128d)(Y), (M)), \
                  __bcc_low_sd((__m128d)(X), __bcc_round_pd((__m128d)(Y), (int)(M))))
#define _mm_ceil_ps(X) _mm_round_ps((X), _MM_FROUND_CEIL)
#define _mm_ceil_pd(X) _mm_round_pd((X), _MM_FROUND_CEIL)
#define _mm_ceil_ss(X, Y) _mm_round_ss((X), (Y), _MM_FROUND_CEIL)
#define _mm_ceil_sd(X, Y) _mm_round_sd((X), (Y), _MM_FROUND_CEIL)
#define _mm_floor_ps(X) _mm_round_ps((X), _MM_FROUND_FLOOR)
#define _mm_floor_pd(X) _mm_round_pd((X), _MM_FROUND_FLOOR)
#define _mm_floor_ss(X, Y) _mm_round_ss((X), (Y), _MM_FROUND_FLOOR)
#define _mm_floor_sd(X, Y) _mm_round_sd((X), (Y), _MM_FROUND_FLOOR)

/* Blends: lane `i` from `V2` where bit `i` of `M`, or the top bit of the
 * mask's lane, is set. */

__BCC_SSE __m128d __bcc_blend_pd(__m128d __a, __m128d __b, int __m) {
  return (__m128d){(__m & 1) ? __b[0] : __a[0], (__m & 2) ? __b[1] : __a[1]};
}
__BCC_SSE __m128 __bcc_blend_ps(__m128 __a, __m128 __b, int __m) {
  return (__m128){(__m & 1) ? __b[0] : __a[0], (__m & 2) ? __b[1] : __a[1],
                  (__m & 4) ? __b[2] : __a[2], (__m & 8) ? __b[3] : __a[3]};
}
__BCC_SSE __m128i __bcc_blend_epi16(__m128i __a, __m128i __b, int __m) {
  __v8hi __x = (__v8hi)__a, __y = (__v8hi)__b, __r;
  int __i;
  for (__i = 0; __i < 8; ++__i)
    __r[__i] = (__m >> __i) & 1 ? __y[__i] : __x[__i];
  return (__m128i)__r;
}
#define _mm_blend_pd(V1, V2, M)                                                 \
  __BCC_IMMEDIATE(__builtin_ia32_blendpd((__v2df)(__m128d)(V1), (__v2df)(__m128d)(V2), (int)(M)), \
                  __bcc_blend_pd((__m128d)(V1), (__m128d)(V2), (int)(M)))
#define _mm_blend_ps(V1, V2, M)                                                 \
  __BCC_IMMEDIATE(__builtin_ia32_blendps((__v4sf)(__m128)(V1), (__v4sf)(__m128)(V2), (int)(M)), \
                  __bcc_blend_ps((__m128)(V1), (__m128)(V2), (int)(M)))
#define _mm_blend_epi16(V1, V2, M)                                              \
  __BCC_IMMEDIATE(__builtin_ia32_pblendw128((__v8hi)(__m128i)(V1), (__v8hi)(__m128i)(V2), (int)(M)), \
                  __bcc_blend_epi16((__m128i)(V1), (__m128i)(V2), (int)(M)))
__BCC_SSE __m128d _mm_blendv_pd(__m128d __a, __m128d __b, __m128d __m) {
  return __bcc_select_pd((__v2di)__m < 0, __b, __a);
}
__BCC_SSE __m128 _mm_blendv_ps(__m128 __a, __m128 __b, __m128 __m) {
  return __bcc_select_ps((__v4si)__m < 0, __b, __a);
}
__BCC_SSE __m128i _mm_blendv_epi8(__m128i __a, __m128i __b, __m128i __m) {
  __v16qs __s = (__v16qs)__m < 0;
  return (__m128i)((__s & (__v16qs)__b) | (~__s & (__v16qs)__a));
}

/* Multiplication */

__BCC_SSE __m128i _mm_mullo_epi32(__m128i __a, __m128i __b) { return (__m128i)((__v4su)__a * (__v4su)__b); }
__BCC_SSE __m128i _mm_mul_epi32(__m128i __a, __m128i __b) {
  __v4si __x = (__v4si)__a, __y = (__v4si)__b;
  return (__m128i){(long long)__x[0] * __y[0], (long long)__x[2] * __y[2]};
}

/* Dot products: the products `M`'s high bits select, summed in pairs as
 * the instruction sums them, stored in the lanes its low bits select. */
__BCC_SSE __m128 __bcc_dp_ps(__m128 __x, __m128 __y, int __m) {
  float __p[4], __s;
  int __i;
  for (__i = 0; __i < 4; ++__i)
    __p[__i] = (__m >> (4 + __i)) & 1 ? __x[__i] * __y[__i] : 0.0f;
  __s = (__p[0] + __p[1]) + (__p[2] + __p[3]);
  return (__m128){__m & 1 ? __s : 0.0f, __m & 2 ? __s : 0.0f, __m & 4 ? __s : 0.0f,
                  __m & 8 ? __s : 0.0f};
}
__BCC_SSE __m128d __bcc_dp_pd(__m128d __x, __m128d __y, int __m) {
  double __s = ((__m & 16) ? __x[0] * __y[0] : 0.0) + ((__m & 32) ? __x[1] * __y[1] : 0.0);
  return (__m128d){__m & 1 ? __s : 0.0, __m & 2 ? __s : 0.0};
}
#define _mm_dp_ps(X, Y, M)                                                      \
  __BCC_IMMEDIATE(__builtin_ia32_dpps((__v4sf)(__m128)(X), (__v4sf)(__m128)(Y), (M)), \
                  __bcc_dp_ps((__m128)(X), (__m128)(Y), (int)(M)))
#define _mm_dp_pd(X, Y, M)                                                      \
  __BCC_IMMEDIATE(__builtin_ia32_dppd((__v2df)(__m128d)(X), (__v2df)(__m128d)(Y), (M)), \
                  __bcc_dp_pd((__m128d)(X), (__m128d)(Y), (int)(M)))

/* Loads */

__BCC_SSE __m128i _mm_stream_load_si128(const void *__p) { return *(const __m128i *)__p; }

/* Minimum and maximum */

#define __BCC_MINMAX(name, type, op)                                            \
  __BCC_SSE __m128i name(__m128i __a, __m128i __b) {                            \
    type __x = (type)__a, __y = (type)__b;                                      \
    type __m = (type)(__x op __y);                                              \
    return (__m128i)((__m & __x) | (~__m & __y));                               \
  }
__BCC_MINMAX(_mm_min_epi8, __v16qs, <)
__BCC_MINMAX(_mm_max_epi8, __v16qs, >)
__BCC_MINMAX(_mm_min_epu16, __v8hu, <)
__BCC_MINMAX(_mm_max_epu16, __v8hu, >)
__BCC_MINMAX(_mm_min_epi32, __v4si, <)
__BCC_MINMAX(_mm_max_epi32, __v4si, >)
__BCC_MINMAX(_mm_min_epu32, __v4su, <)
__BCC_MINMAX(_mm_max_epu32, __v4su, >)

/* Insertion and extraction */

__BCC_SSE __m128 __bcc_insert_ps(__m128 __x, __m128 __y, int __n) {
  __m128 __r = __x;
  int __i;
  __r[(__n >> 4) & 3] = __y[(__n >> 6) & 3];
  for (__i = 0; __i < 4; ++__i)
    if ((__n >> __i) & 1)
      __r[__i] = 0.0f;
  return __r;
}
#define _mm_insert_ps(X, Y, N)                                                  \
  __BCC_IMMEDIATE(__builtin_ia32_insertps128((__v4sf)(__m128)(X), (__v4sf)(__m128)(Y), (N)), \
                  __bcc_insert_ps((__m128)(X), (__m128)(Y), (int)(N)))
__BCC_SSE int __bcc_extract_ps(__m128 __x, int __n) { return ((__v4si)__x)[__n & 3]; }
#define _mm_extract_ps(X, N)                                                    \
  __BCC_IMMEDIATE(__builtin_ia32_vec_ext_v4sf((__v4sf)(__m128)(X), (int)(N)),   \
                  __bcc_extract_ps((__m128)(X), (int)(N)))
#define _MM_EXTRACT_FLOAT(D, X, N)                                              \
  do {                                                                          \
    int __bcc_lane = _mm_extract_ps((X), (N));                                  \
    __builtin_memcpy(&(D), &__bcc_lane, sizeof(float));                         \
  } while (0)
#define _MM_MK_INSERTPS_NDX(X, Y, Z) (((X) << 6) | ((Y) << 4) | (Z))
#define _MM_PICK_OUT_PS(X, N) _mm_insert_ps(_mm_setzero_ps(), (X), _MM_MK_INSERTPS_NDX((N), 0, 0x0e))

__BCC_SSE __m128i __bcc_insert_epi8(__m128i __x, int __i, int __n) {
  __v16qi __r = (__v16qi)__x;
  __r[__n & 15] = (char)__i;
  return (__m128i)__r;
}
__BCC_SSE __m128i __bcc_insert_epi32(__m128i __x, int __i, int __n) {
  __v4si __r = (__v4si)__x;
  __r[__n & 3] = __i;
  return (__m128i)__r;
}
__BCC_SSE __m128i __bcc_insert_epi64(__m128i __x, long long __i, int __n) {
  __x[__n & 1] = __i;
  return __x;
}
#define _mm_insert_epi8(X, I, N)                                                \
  __BCC_IMMEDIATE(__builtin_ia32_vec_set_v16qi((__v16qi)(__m128i)(X), (int)(I), (int)(N)), \
                  __bcc_insert_epi8((__m128i)(X), (int)(I), (int)(N)))
#define _mm_insert_epi32(X, I, N)                                               \
  __BCC_IMMEDIATE(__builtin_ia32_vec_set_v4si((__v4si)(__m128i)(X), (int)(I), (int)(N)), \
                  __bcc_insert_epi32((__m128i)(X), (int)(I), (int)(N)))
#define _mm_insert_epi64(X, I, N)                                               \
  __BCC_IMMEDIATE(__builtin_ia32_vec_set_v2di((__v2di)(__m128i)(X), (long long)(I), (int)(N)), \
                  __bcc_insert_epi64((__m128i)(X), (long long)(I), (int)(N)))
__BCC_SSE int __bcc_extract_epi8(__m128i __x, int __n) { return ((__v16qu)__x)[__n & 15]; }
__BCC_SSE int __bcc_extract_epi32(__m128i __x, int __n) { return ((__v4si)__x)[__n & 3]; }
__BCC_SSE long long __bcc_extract_epi64(__m128i __x, int __n) { return __x[__n & 1]; }
#define _mm_extract_epi8(X, N)                                                  \
  __BCC_IMMEDIATE(__builtin_ia32_vec_ext_v16qi((__v16qi)(__m128i)(X), (int)(N)), \
                  __bcc_extract_epi8((__m128i)(X), (int)(N)))
#define _mm_extract_epi32(X, N)                                                 \
  __BCC_IMMEDIATE(__builtin_ia32_vec_ext_v4si((__v4si)(__m128i)(X), (int)(N)), \
                  __bcc_extract_epi32((__m128i)(X), (int)(N)))
#define _mm_extract_epi64(X, N)                                                 \
  __BCC_IMMEDIATE(__builtin_ia32_vec_ext_v2di((__v2di)(__m128i)(X), (int)(N)), \
                  __bcc_extract_epi64((__m128i)(X), (int)(N)))

/* Tests */

__BCC_SSE int _mm_testz_si128(__m128i __m, __m128i __v) {
  __m128i __t = __m & __v;
  return (__t[0] | __t[1]) == 0;
}
__BCC_SSE int _mm_testc_si128(__m128i __m, __m128i __v) {
  __m128i __t = ~__m & __v;
  return (__t[0] | __t[1]) == 0;
}
__BCC_SSE int _mm_testnzc_si128(__m128i __m, __m128i __v) {
  return !_mm_testz_si128(__m, __v) && !_mm_testc_si128(__m, __v);
}
#define _mm_test_all_ones(V) _mm_testc_si128((V), _mm_set1_epi32(-1))
#define _mm_test_mix_ones_zeros(M, V) _mm_testnzc_si128((M), (V))
#define _mm_test_all_zeros(M, V) _mm_testz_si128((M), (V))

/* Comparison, packing and conversion */

__BCC_SSE __m128i _mm_cmpeq_epi64(__m128i __a, __m128i __b) { return (__m128i)(__a == __b); }
__BCC_SSE __m128i _mm_packus_epi32(__m128i __a, __m128i __b) {
  __v4si __x = (__v4si)__a, __y = (__v4si)__b;
  __v8hu __r;
  int __i;
  for (__i = 0; __i < 4; ++__i) {
    __r[__i] = (unsigned short)__BCC_CLAMP(__x[__i], 0, 65535);
    __r[__i + 4] = (unsigned short)__BCC_CLAMP(__y[__i], 0, 65535);
  }
  return (__m128i)__r;
}
/* The low lanes of `V` sign- or zero-extended to wider lanes. */
#define __BCC_EXTEND(name, from, to, lanes)                                     \
  __BCC_SSE __m128i name(__m128i __v) {                                         \
    from __x = (from)__v;                                                       \
    to __r;                                                                     \
    int __i;                                                                    \
    for (__i = 0; __i < (lanes); ++__i)                                         \
      __r[__i] = __x[__i];                                                      \
    return (__m128i)__r;                                                        \
  }
__BCC_EXTEND(_mm_cvtepi8_epi16, __v16qs, __v8hi, 8)
__BCC_EXTEND(_mm_cvtepi8_epi32, __v16qs, __v4si, 4)
__BCC_EXTEND(_mm_cvtepi8_epi64, __v16qs, __v2di, 2)
__BCC_EXTEND(_mm_cvtepi16_epi32, __v8hi, __v4si, 4)
__BCC_EXTEND(_mm_cvtepi16_epi64, __v8hi, __v2di, 2)
__BCC_EXTEND(_mm_cvtepi32_epi64, __v4si, __v2di, 2)
__BCC_EXTEND(_mm_cvtepu8_epi16, __v16qu, __v8hi, 8)
__BCC_EXTEND(_mm_cvtepu8_epi32, __v16qu, __v4si, 4)
__BCC_EXTEND(_mm_cvtepu8_epi64, __v16qu, __v2di, 2)
__BCC_EXTEND(_mm_cvtepu16_epi32, __v8hu, __v4si, 4)
__BCC_EXTEND(_mm_cvtepu16_epi64, __v8hu, __v2di, 2)
__BCC_EXTEND(_mm_cvtepu32_epi64, __v4su, __v2di, 2)

/* MPSADBW: eight sums of absolute differences of four bytes, `X`'s sliding
 * from the offset `M`'s bit 2 selects, `Y`'s fixed at the block bits 0-1
 * select. */
__BCC_SSE __m128i __bcc_mpsadbw(__m128i __a, __m128i __b, int __m) {
  __v16qu __x = (__v16qu)__a, __y = (__v16qu)__b;
  __v8hu __r;
  int __from = (__m & 4) ? 4 : 0, __block = (__m & 3) * 4, __i, __k;
  for (__i = 0; __i < 8; ++__i) {
    int __s = 0;
    for (__k = 0; __k < 4; ++__k) {
      int __d = __x[__from + __i + __k] - __y[__block + __k];
      __s += __d < 0 ? -__d : __d;
    }
    __r[__i] = (unsigned short)__s;
  }
  return (__m128i)__r;
}
#define _mm_mpsadbw_epu8(X, Y, M)                                               \
  __BCC_IMMEDIATE(__builtin_ia32_mpsadbw128((__v16qi)(__m128i)(X), (__v16qi)(__m128i)(Y), (M)), \
                  __bcc_mpsadbw((__m128i)(X), (__m128i)(Y), (int)(M)))
/* The smallest unsigned word and the lowest index holding it. */
__BCC_SSE __m128i _mm_minpos_epu16(__m128i __v) {
  __v8hu __x = (__v8hu)__v;
  int __at = 0, __i;
  for (__i = 1; __i < 8; ++__i)
    if (__x[__i] < __x[__at])
      __at = __i;
  return (__m128i)(__v8hu){__x[__at], (unsigned short)__at, 0, 0, 0, 0, 0, 0};
}

/* SSE4.2 */

#define _SIDD_UBYTE_OPS 0x00
#define _SIDD_UWORD_OPS 0x01
#define _SIDD_SBYTE_OPS 0x02
#define _SIDD_SWORD_OPS 0x03
#define _SIDD_CMP_EQUAL_ANY 0x00
#define _SIDD_CMP_RANGES 0x04
#define _SIDD_CMP_EQUAL_EACH 0x08
#define _SIDD_CMP_EQUAL_ORDERED 0x0c
#define _SIDD_POSITIVE_POLARITY 0x00
#define _SIDD_NEGATIVE_POLARITY 0x10
#define _SIDD_MASKED_POSITIVE_POLARITY 0x20
#define _SIDD_MASKED_NEGATIVE_POLARITY 0x30
#define _SIDD_LEAST_SIGNIFICANT 0x00
#define _SIDD_MOST_SIGNIFICANT 0x40
#define _SIDD_BIT_MASK 0x00
#define _SIDD_UNIT_MASK 0x40

/* Element `__i` of a PCMPxSTRx operand in the format `__m` selects. */
__BCC_SSE int __bcc_string_element(__m128i __v, int __i, int __m) {
  switch (__m & 3) {
  case 0: return ((__v16qu)__v)[__i];
  case 1: return ((__v8hu)__v)[__i];
  case 2: return ((__v16qs)__v)[__i];
  default: return ((__v8hi)__v)[__i];
  }
}
/* The length of an operand: up to its first zero element when
 * `__length` is `__BCC_IMPLICIT`, else `__length`, the absolute value of
 * the length register, saturated to the element count. */
__BCC_SSE int __bcc_string_length(__m128i __v, long long __length, int __m) {
  int __n = (__m & 1) ? 8 : 16, __i;
  if (__length >= 0)
    return __length > __n ? __n : (int)__length;
  for (__i = 0; __i < __n && __bcc_string_element(__v, __i, __m) != 0; ++__i)
    ;
  return __i;
}
/* The comparison of PCMPxSTRx: IntRes2, one bit per element of `b`, and
 * its flags, as the SDM's "Compare Packed Strings" defines them. */
__BCC_SSE int __bcc_pcmpstr(__m128i __a, int __la, __m128i __b, int __lb, int __m, int *__flags) {
  int __n = (__m & 1) ? 8 : 16, __i, __j, __result = 0;
  int __bool[16][16];
  for (__i = 0; __i < __n; ++__i)
    for (__j = 0; __j < __n; ++__j) {
      int __x = __bcc_string_element(__a, __i, __m), __y = __bcc_string_element(__b, __j, __m);
      int __av = __i < __la, __bv = __j < __lb, __r;
      if ((__m & 12) == 4)
        __r = (__i & 1) ? __x >= __y : __x <= __y;
      else
        __r = __x == __y;
      if (!__av || !__bv) {
        switch (__m & 12) {
        case 0: case 4: __r = 0; break;
        case 8: __r = !__av && !__bv; break;
        default: __r = !__av; break;
        }
      }
      __bool[__i][__j] = __r;
    }
  for (__j = 0; __j < __n; ++__j) {
    int __bit = 0;
    switch (__m & 12) {
    case 0:
      for (__i = 0; __i < __n; ++__i)
        __bit |= __bool[__i][__j];
      break;
    case 4:
      for (__i = 0; __i + 1 < __n; __i += 2)
        __bit |= __bool[__i][__j] && __bool[__i + 1][__j];
      break;
    case 8:
      __bit = __bool[__j][__j];
      break;
    default:
      __bit = 1;
      for (__i = 0; __i < __n - __j; ++__i)
        __bit &= __bool[__i][__i + __j];
      break;
    }
    __result |= __bit << __j;
  }
  switch (__m & 0x30) {
  case 0x10: __result ^= (1 << __n) - 1; break;
  case 0x30: __result ^= (1 << __lb) - 1; break;
  default: break;
  }
  *__flags = (__result != 0) | (__lb < __n) << 1 | (__la < __n) << 2 | (__result & 1) << 3;
  return __result;
}
__BCC_SSE int __bcc_string_index(int __result, int __m) {
  int __n = (__m & 1) ? 8 : 16, __i;
  if (__result == 0)
    return __n;
  if (__m & 0x40) {
    for (__i = __n - 1; !((__result >> __i) & 1); --__i)
      ;
    return __i;
  }
  for (__i = 0; !((__result >> __i) & 1); ++__i)
    ;
  return __i;
}
__BCC_SSE __m128i __bcc_string_mask(int __result, int __m) {
  int __i;
  if (!(__m & 0x40))
    return (__m128i){__result, 0};
  if (__m & 1) {
    __v8hi __r;
    for (__i = 0; __i < 8; ++__i)
      __r[__i] = (short)-((__result >> __i) & 1);
    return (__m128i)__r;
  } else {
    __v16qs __r;
    for (__i = 0; __i < 16; ++__i)
      __r[__i] = (signed char)-((__result >> __i) & 1);
    return (__m128i)__r;
  }
}
/* What one string-comparison intrinsic returns: `__what` 0 the index, 1 the
 * mask's low word (unused), 2-7 the flags A, C, O, S, Z. */
__BCC_SSE int __bcc_string(__m128i __a, long long __la, __m128i __b, long long __lb, int __m,
                           int __what) {
  int __flags, __lena, __lenb, __result;
  __lena = __bcc_string_length(__a, __la, __m);
  __lenb = __bcc_string_length(__b, __lb, __m);
  __result = __bcc_pcmpstr(__a, __lena, __b, __lenb, __m, &__flags);
  switch (__what) {
  case 0: return __bcc_string_index(__result, __m);
  case 2: return !(__flags & 1) && !(__flags & 2);
  case 3: return __flags & 1;
  case 4: return (__flags >> 3) & 1;
  case 5: return (__flags >> 2) & 1;
  default: return (__flags >> 1) & 1;
  }
}
__BCC_SSE __m128i __bcc_string_masked(__m128i __a, long long __la, __m128i __b, long long __lb,
                                      int __m) {
  int __flags, __lena, __lenb;
  __lena = __bcc_string_length(__a, __la, __m);
  __lenb = __bcc_string_length(__b, __lb, __m);
  return __bcc_string_mask(__bcc_pcmpstr(__a, __lena, __b, __lenb, __m, &__flags), __m);
}
/* An explicit length is the absolute value of the register. */
__BCC_SSE long long __bcc_explicit_length(int __l) { return __l < 0 ? -(long long)__l : __l; }
#define __BCC_IMPLICIT (-1 - 0x7fffffffffffffffLL)
#define _mm_cmpistrm(A, B, M)                                                   \
  __BCC_IMMEDIATE(__builtin_ia32_pcmpistrm128((__v16qi)(__m128i)(A), (__v16qi)(__m128i)(B), (int)(M)), \
                  __bcc_string_masked((__m128i)(A), __BCC_IMPLICIT, (__m128i)(B), __BCC_IMPLICIT, (int)(M)))
#define __BCC_ISTR(builtin, A, B, M, what)                                      \
  __BCC_IMMEDIATE(builtin((__v16qi)(__m128i)(A), (__v16qi)(__m128i)(B), (int)(M)), \
                  __bcc_string((__m128i)(A), __BCC_IMPLICIT, (__m128i)(B), __BCC_IMPLICIT, (int)(M), (what)))
#define _mm_cmpistri(A, B, M) __BCC_ISTR(__builtin_ia32_pcmpistri128, A, B, M, 0)
#define _mm_cmpistra(A, B, M) __BCC_ISTR(__builtin_ia32_pcmpistria128, A, B, M, 2)
#define _mm_cmpistrc(A, B, M) __BCC_ISTR(__builtin_ia32_pcmpistric128, A, B, M, 3)
#define _mm_cmpistro(A, B, M) __BCC_ISTR(__builtin_ia32_pcmpistrio128, A, B, M, 4)
#define _mm_cmpistrs(A, B, M) __BCC_ISTR(__builtin_ia32_pcmpistris128, A, B, M, 5)
#define _mm_cmpistrz(A, B, M) __BCC_ISTR(__builtin_ia32_pcmpistriz128, A, B, M, 6)
#define _mm_cmpestrm(A, LA, B, LB, M)                                           \
  __BCC_IMMEDIATE(__builtin_ia32_pcmpestrm128((__v16qi)(__m128i)(A), (int)(LA), (__v16qi)(__m128i)(B), \
                                              (int)(LB), (int)(M)),             \
                  __bcc_string_masked((__m128i)(A), __bcc_explicit_length((int)(LA)), (__m128i)(B), \
                                      __bcc_explicit_length((int)(LB)), (int)(M)))
#define __BCC_ESTR(builtin, A, LA, B, LB, M, what)                              \
  __BCC_IMMEDIATE(builtin((__v16qi)(__m128i)(A), (int)(LA), (__v16qi)(__m128i)(B), (int)(LB), (int)(M)), \
                  __bcc_string((__m128i)(A), __bcc_explicit_length((int)(LA)), (__m128i)(B), \
                               __bcc_explicit_length((int)(LB)), (int)(M), (what)))
#define _mm_cmpestri(A, LA, B, LB, M) __BCC_ESTR(__builtin_ia32_pcmpestri128, A, LA, B, LB, M, 0)
#define _mm_cmpestra(A, LA, B, LB, M) __BCC_ESTR(__builtin_ia32_pcmpestria128, A, LA, B, LB, M, 2)
#define _mm_cmpestrc(A, LA, B, LB, M) __BCC_ESTR(__builtin_ia32_pcmpestric128, A, LA, B, LB, M, 3)
#define _mm_cmpestro(A, LA, B, LB, M) __BCC_ESTR(__builtin_ia32_pcmpestrio128, A, LA, B, LB, M, 4)
#define _mm_cmpestrs(A, LA, B, LB, M) __BCC_ESTR(__builtin_ia32_pcmpestris128, A, LA, B, LB, M, 5)
#define _mm_cmpestrz(A, LA, B, LB, M) __BCC_ESTR(__builtin_ia32_pcmpestriz128, A, LA, B, LB, M, 6)

__BCC_SSE __m128i _mm_cmpgt_epi64(__m128i __a, __m128i __b) { return (__m128i)(__a > __b); }

#include <popcntintrin.h>

#include <crc32intrin.h>

#endif /* __SMMINTRIN_H */
