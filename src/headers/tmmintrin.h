/* tmmintrin.h - SSSE3 intrinsics for bcc.
 *
 * Written for bcc; part of bcc and under its licence. Plain C over GNU
 * vector operations, as in <xmmintrin.h>. */

#ifndef __TMMINTRIN_H
#define __TMMINTRIN_H

#if !defined(__i386__) && !defined(__x86_64__)
#error "This header is only meant to be used on x86 and x64 architecture"
#endif

#include <pmmintrin.h>

/* Absolute values; the most negative lane stays itself. */
__BCC_SSE __m128i _mm_abs_epi8(__m128i __a) {
  __v16qs __x = (__v16qs)__a;
  return (__m128i)(__v16qs)((__v16qu)(__x < 0) & (__v16qu)-__x | (__v16qu)~(__x < 0) & (__v16qu)__x);
}
__BCC_SSE __m128i _mm_abs_epi16(__m128i __a) {
  __v8hi __x = (__v8hi)__a;
  return (__m128i)((__x < 0) & -__x | ~(__x < 0) & __x);
}
__BCC_SSE __m128i _mm_abs_epi32(__m128i __a) {
  __v4si __x = (__v4si)__a;
  return (__m128i)((__x < 0) & -__x | ~(__x < 0) & __x);
}
__BCC_SSE __m64 _mm_abs_pi8(__m64 __a) {
  __v8qs __x = (__v8qs)__a, __r;
  int __i;
  for (__i = 0; __i < 8; ++__i)
    __r[__i] = (signed char)(__x[__i] < 0 ? -__x[__i] : __x[__i]);
  return (__m64)__r;
}
__BCC_SSE __m64 _mm_abs_pi16(__m64 __a) {
  __v4hi __x = (__v4hi)__a, __r;
  int __i;
  for (__i = 0; __i < 4; ++__i)
    __r[__i] = (short)(__x[__i] < 0 ? -__x[__i] : __x[__i]);
  return (__m64)__r;
}
__BCC_SSE __m64 _mm_abs_pi32(__m64 __a) {
  __v2su __x = (__v2su)__a;
  return (__m64)(__v2su){(int)__x[0] < 0 ? 0u - __x[0] : __x[0], (int)__x[1] < 0 ? 0u - __x[1] : __x[1]};
}

/* Byte alignment: the bytes of `b` then `a` shifted right by `n` bytes. */
__BCC_SSE __m128i __bcc_alignr(__m128i __a, __m128i __b, int __n) {
  __v16qu __x = (__v16qu)__a, __y = (__v16qu)__b, __r;
  int __i;
  for (__i = 0; __i < 16; ++__i) {
    int __at = __i + __n;
    __r[__i] = __at < 16 ? __y[__at] : __at < 32 ? __x[__at - 16] : 0;
  }
  return (__m128i)__r;
}
#define _mm_alignr_epi8(a, b, n)                                                \
  __BCC_IMMEDIATE(__builtin_ia32_palignr128((__v16qi)(__m128i)(a), (__v16qi)(__m128i)(b), (n)), \
                  __bcc_alignr((__m128i)(a), (__m128i)(b), (int)(n) & 255))
__BCC_SSE __m64 __bcc_alignr_pi8(__m64 __a, __m64 __b, int __n) {
  __v8qu __x = (__v8qu)__a, __y = (__v8qu)__b, __r;
  int __i;
  for (__i = 0; __i < 8; ++__i) {
    int __at = __i + __n;
    __r[__i] = __at < 8 ? __y[__at] : __at < 16 ? __x[__at - 8] : 0;
  }
  return (__m64)__r;
}
#define _mm_alignr_pi8(a, b, n)                                                 \
  __BCC_IMMEDIATE(__builtin_ia32_palignr128((__v16qi)(__m128i){}, (__v16qi)(__m128i){}, (n)), \
                  __bcc_alignr_pi8((__m64)(a), (__m64)(b), (int)(n) & 255))

/* Horizontal additions and subtractions of adjacent pairs, `a`'s first. */
#define __BCC_HORIZONTAL(name, vector, lanes, type, op, saturate)               \
  __BCC_SSE vector name(vector __a, vector __b) {                               \
    type __x = (type)__a, __y = (type)__b, __r;                                 \
    int __i;                                                                    \
    for (__i = 0; __i < (lanes) / 2; ++__i) {                                   \
      __r[__i] = saturate(__x[2 * __i] op __x[2 * __i + 1]);                    \
      __r[__i + (lanes) / 2] = saturate(__y[2 * __i] op __y[2 * __i + 1]);      \
    }                                                                           \
    return (vector)__r;                                                         \
  }
#define __BCC_WRAP16(x) ((short)(x))
#define __BCC_WRAP32(x) ((int)((unsigned)(x)))
#define __BCC_SATURATE16(x) ((short)__BCC_CLAMP((x), -32768, 32767))
__BCC_HORIZONTAL(_mm_hadd_epi16, __m128i, 8, __v8hi, +, __BCC_WRAP16)
__BCC_HORIZONTAL(_mm_hsub_epi16, __m128i, 8, __v8hi, -, __BCC_WRAP16)
__BCC_HORIZONTAL(_mm_hadds_epi16, __m128i, 8, __v8hi, +, __BCC_SATURATE16)
__BCC_HORIZONTAL(_mm_hsubs_epi16, __m128i, 8, __v8hi, -, __BCC_SATURATE16)
__BCC_HORIZONTAL(_mm_hadd_pi16, __m64, 4, __v4hi, +, __BCC_WRAP16)
__BCC_HORIZONTAL(_mm_hsub_pi16, __m64, 4, __v4hi, -, __BCC_WRAP16)
__BCC_HORIZONTAL(_mm_hadds_pi16, __m64, 4, __v4hi, +, __BCC_SATURATE16)
__BCC_HORIZONTAL(_mm_hsubs_pi16, __m64, 4, __v4hi, -, __BCC_SATURATE16)
__BCC_HORIZONTAL(_mm_hadd_epi32, __m128i, 4, __v4su, +, )
__BCC_HORIZONTAL(_mm_hsub_epi32, __m128i, 4, __v4su, -, )
__BCC_HORIZONTAL(_mm_hadd_pi32, __m64, 2, __v2su, +, )
__BCC_HORIZONTAL(_mm_hsub_pi32, __m64, 2, __v2su, -, )

/* Unsigned bytes of `a` times signed bytes of `b`, pairs added with
 * saturation. */
__BCC_SSE __m128i _mm_maddubs_epi16(__m128i __a, __m128i __b) {
  __v16qu __x = (__v16qu)__a;
  __v16qs __y = (__v16qs)__b;
  __v8hi __r;
  int __i;
  for (__i = 0; __i < 8; ++__i)
    __r[__i] = (short)__BCC_CLAMP(__x[2 * __i] * __y[2 * __i] + __x[2 * __i + 1] * __y[2 * __i + 1],
                                  -32768, 32767);
  return (__m128i)__r;
}
__BCC_SSE __m64 _mm_maddubs_pi16(__m64 __a, __m64 __b) {
  __v8qu __x = (__v8qu)__a;
  __v8qs __y = (__v8qs)__b;
  __v4hi __r;
  int __i;
  for (__i = 0; __i < 4; ++__i)
    __r[__i] = (short)__BCC_CLAMP(__x[2 * __i] * __y[2 * __i] + __x[2 * __i + 1] * __y[2 * __i + 1],
                                  -32768, 32767);
  return (__m64)__r;
}
/* High halves of the products, rounded: (a * b + 0x4000) >> 15. */
__BCC_SSE __m128i _mm_mulhrs_epi16(__m128i __a, __m128i __b) {
  __v8hi __x = (__v8hi)__a, __y = (__v8hi)__b, __r;
  int __i;
  for (__i = 0; __i < 8; ++__i)
    __r[__i] = (short)(((__x[__i] * __y[__i] >> 14) + 1) >> 1);
  return (__m128i)__r;
}
__BCC_SSE __m64 _mm_mulhrs_pi16(__m64 __a, __m64 __b) {
  __v4hi __x = (__v4hi)__a, __y = (__v4hi)__b, __r;
  int __i;
  for (__i = 0; __i < 4; ++__i)
    __r[__i] = (short)(((__x[__i] * __y[__i] >> 14) + 1) >> 1);
  return (__m64)__r;
}
/* PSHUFB: each byte of `b` selects a byte of `a` by its low bits, or zero
 * when its top bit is set. */
__BCC_SSE __m128i _mm_shuffle_epi8(__m128i __a, __m128i __b) {
  __v16qu __x = (__v16qu)__a, __y = (__v16qu)__b, __r;
  int __i;
  for (__i = 0; __i < 16; ++__i)
    __r[__i] = (__y[__i] & 0x80) ? 0 : __x[__y[__i] & 15];
  return (__m128i)__r;
}
__BCC_SSE __m64 _mm_shuffle_pi8(__m64 __a, __m64 __b) {
  __v8qu __x = (__v8qu)__a, __y = (__v8qu)__b, __r;
  int __i;
  for (__i = 0; __i < 8; ++__i)
    __r[__i] = (__y[__i] & 0x80) ? 0 : __x[__y[__i] & 7];
  return (__m64)__r;
}
/* PSIGN: each lane of `a` negated, kept or zeroed by the sign of `b`'s. */
#define __BCC_SIGN(name, vector, lanes, type)                                   \
  __BCC_SSE vector name(vector __a, vector __b) {                               \
    type __x = (type)__a, __y = (type)__b, __r;                                 \
    int __i;                                                                    \
    for (__i = 0; __i < (lanes); ++__i)                                         \
      __r[__i] = __y[__i] < 0 ? 0 - (long long)__x[__i] : __y[__i] == 0 ? 0 : __x[__i]; \
    return (vector)__r;                                                         \
  }
__BCC_SIGN(_mm_sign_epi8, __m128i, 16, __v16qs)
__BCC_SIGN(_mm_sign_epi16, __m128i, 8, __v8hi)
__BCC_SIGN(_mm_sign_epi32, __m128i, 4, __v4si)
__BCC_SIGN(_mm_sign_pi8, __m64, 8, __v8qs)
__BCC_SIGN(_mm_sign_pi16, __m64, 4, __v4hi)
__BCC_SIGN(_mm_sign_pi32, __m64, 2, __v2si)

#endif /* __TMMINTRIN_H */
