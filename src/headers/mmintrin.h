/* mmintrin.h - MMX intrinsics for bcc.
 *
 * Written for bcc; part of bcc and under its licence. Each intrinsic is
 * plain C over 8-byte GNU vectors, which bcc lowers lane by lane on every
 * back end, so no MMX register is used and `_mm_empty` does nothing. */

#ifndef __MMINTRIN_H
#define __MMINTRIN_H

#if !defined(__i386__) && !defined(__x86_64__)
#error "This header is only meant to be used on x86 and x64 architecture"
#endif

typedef long long __m64 __attribute__((__vector_size__(8), __aligned__(8)));

typedef long long __v1di __attribute__((__vector_size__(8)));
typedef int __v2si __attribute__((__vector_size__(8)));
typedef short __v4hi __attribute__((__vector_size__(8)));
typedef char __v8qi __attribute__((__vector_size__(8)));
typedef unsigned long long __v1du __attribute__((__vector_size__(8)));
typedef unsigned int __v2su __attribute__((__vector_size__(8)));
typedef unsigned short __v4hu __attribute__((__vector_size__(8)));
typedef unsigned char __v8qu __attribute__((__vector_size__(8)));
typedef signed char __v8qs __attribute__((__vector_size__(8)));

/* The SSE2 integer types, which GCC and Clang also declare here. */
typedef long long __m128i __attribute__((__vector_size__(16), __aligned__(16)));
typedef long long __v2di __attribute__((__vector_size__(16)));
typedef int __v4si __attribute__((__vector_size__(16)));
typedef short __v8hi __attribute__((__vector_size__(16)));
typedef char __v16qi __attribute__((__vector_size__(16)));

#define __BCC_MMX static __inline__ __attribute__((__always_inline__, __nodebug__))
#define __BCC_SATURATE(x, low, high) ((x) < (low) ? (low) : (x) > (high) ? (high) : (x))

__BCC_MMX void _mm_empty(void) {}

/* Conversions */

__BCC_MMX __m64 _mm_cvtsi32_si64(int __i) { return (__m64)(__v2si){__i, 0}; }
__BCC_MMX int _mm_cvtsi64_si32(__m64 __m) { return ((__v2si)__m)[0]; }
__BCC_MMX __m64 _mm_cvtsi64_m64(long long __i) { return (__m64){__i}; }
__BCC_MMX long long _mm_cvtm64_si64(__m64 __m) { return __m[0]; }

/* Packs and unpacks */

__BCC_MMX __m64 _mm_packs_pi16(__m64 __a, __m64 __b) {
  __v4hi __x = (__v4hi)__a, __y = (__v4hi)__b;
  __v8qs __r;
  int __i;
  for (__i = 0; __i < 4; ++__i) {
    __r[__i] = (signed char)__BCC_SATURATE(__x[__i], -128, 127);
    __r[__i + 4] = (signed char)__BCC_SATURATE(__y[__i], -128, 127);
  }
  return (__m64)__r;
}
__BCC_MMX __m64 _mm_packs_pi32(__m64 __a, __m64 __b) {
  __v2si __x = (__v2si)__a, __y = (__v2si)__b;
  return (__m64)(__v4hi){(short)__BCC_SATURATE(__x[0], -32768, 32767),
                         (short)__BCC_SATURATE(__x[1], -32768, 32767),
                         (short)__BCC_SATURATE(__y[0], -32768, 32767),
                         (short)__BCC_SATURATE(__y[1], -32768, 32767)};
}
__BCC_MMX __m64 _mm_packs_pu16(__m64 __a, __m64 __b) {
  __v4hi __x = (__v4hi)__a, __y = (__v4hi)__b;
  __v8qu __r;
  int __i;
  for (__i = 0; __i < 4; ++__i) {
    __r[__i] = (unsigned char)__BCC_SATURATE(__x[__i], 0, 255);
    __r[__i + 4] = (unsigned char)__BCC_SATURATE(__y[__i], 0, 255);
  }
  return (__m64)__r;
}
__BCC_MMX __m64 _mm_unpackhi_pi8(__m64 __a, __m64 __b) {
  return (__m64)__builtin_shufflevector((__v8qi)__a, (__v8qi)__b, 4, 12, 5, 13, 6, 14, 7, 15);
}
__BCC_MMX __m64 _mm_unpackhi_pi16(__m64 __a, __m64 __b) {
  return (__m64)__builtin_shufflevector((__v4hi)__a, (__v4hi)__b, 2, 6, 3, 7);
}
__BCC_MMX __m64 _mm_unpackhi_pi32(__m64 __a, __m64 __b) {
  return (__m64)__builtin_shufflevector((__v2si)__a, (__v2si)__b, 1, 3);
}
__BCC_MMX __m64 _mm_unpacklo_pi8(__m64 __a, __m64 __b) {
  return (__m64)__builtin_shufflevector((__v8qi)__a, (__v8qi)__b, 0, 8, 1, 9, 2, 10, 3, 11);
}
__BCC_MMX __m64 _mm_unpacklo_pi16(__m64 __a, __m64 __b) {
  return (__m64)__builtin_shufflevector((__v4hi)__a, (__v4hi)__b, 0, 4, 1, 5);
}
__BCC_MMX __m64 _mm_unpacklo_pi32(__m64 __a, __m64 __b) {
  return (__m64)__builtin_shufflevector((__v2si)__a, (__v2si)__b, 0, 2);
}

/* Arithmetic; lanes wrap unless the name says they saturate. */

__BCC_MMX __m64 _mm_add_pi8(__m64 __a, __m64 __b) { return (__m64)((__v8qu)__a + (__v8qu)__b); }
__BCC_MMX __m64 _mm_add_pi16(__m64 __a, __m64 __b) { return (__m64)((__v4hu)__a + (__v4hu)__b); }
__BCC_MMX __m64 _mm_add_pi32(__m64 __a, __m64 __b) { return (__m64)((__v2su)__a + (__v2su)__b); }
__BCC_MMX __m64 _mm_sub_pi8(__m64 __a, __m64 __b) { return (__m64)((__v8qu)__a - (__v8qu)__b); }
__BCC_MMX __m64 _mm_sub_pi16(__m64 __a, __m64 __b) { return (__m64)((__v4hu)__a - (__v4hu)__b); }
__BCC_MMX __m64 _mm_sub_pi32(__m64 __a, __m64 __b) { return (__m64)((__v2su)__a - (__v2su)__b); }
__BCC_MMX __m64 _mm_adds_pi8(__m64 __a, __m64 __b) {
  __v8qs __x = (__v8qs)__a, __y = (__v8qs)__b, __r;
  int __i;
  for (__i = 0; __i < 8; ++__i)
    __r[__i] = (signed char)__BCC_SATURATE(__x[__i] + __y[__i], -128, 127);
  return (__m64)__r;
}
__BCC_MMX __m64 _mm_adds_pi16(__m64 __a, __m64 __b) {
  __v4hi __x = (__v4hi)__a, __y = (__v4hi)__b, __r;
  int __i;
  for (__i = 0; __i < 4; ++__i)
    __r[__i] = (short)__BCC_SATURATE(__x[__i] + __y[__i], -32768, 32767);
  return (__m64)__r;
}
__BCC_MMX __m64 _mm_adds_pu8(__m64 __a, __m64 __b) {
  __v8qu __x = (__v8qu)__a, __y = (__v8qu)__b, __r;
  int __i;
  for (__i = 0; __i < 8; ++__i)
    __r[__i] = (unsigned char)__BCC_SATURATE(__x[__i] + __y[__i], 0, 255);
  return (__m64)__r;
}
__BCC_MMX __m64 _mm_adds_pu16(__m64 __a, __m64 __b) {
  __v4hu __x = (__v4hu)__a, __y = (__v4hu)__b, __r;
  int __i;
  for (__i = 0; __i < 4; ++__i)
    __r[__i] = (unsigned short)__BCC_SATURATE(__x[__i] + __y[__i], 0, 65535);
  return (__m64)__r;
}
__BCC_MMX __m64 _mm_subs_pi8(__m64 __a, __m64 __b) {
  __v8qs __x = (__v8qs)__a, __y = (__v8qs)__b, __r;
  int __i;
  for (__i = 0; __i < 8; ++__i)
    __r[__i] = (signed char)__BCC_SATURATE(__x[__i] - __y[__i], -128, 127);
  return (__m64)__r;
}
__BCC_MMX __m64 _mm_subs_pi16(__m64 __a, __m64 __b) {
  __v4hi __x = (__v4hi)__a, __y = (__v4hi)__b, __r;
  int __i;
  for (__i = 0; __i < 4; ++__i)
    __r[__i] = (short)__BCC_SATURATE(__x[__i] - __y[__i], -32768, 32767);
  return (__m64)__r;
}
__BCC_MMX __m64 _mm_subs_pu8(__m64 __a, __m64 __b) {
  __v8qu __x = (__v8qu)__a, __y = (__v8qu)__b, __r;
  int __i;
  for (__i = 0; __i < 8; ++__i)
    __r[__i] = (unsigned char)__BCC_SATURATE(__x[__i] - __y[__i], 0, 255);
  return (__m64)__r;
}
__BCC_MMX __m64 _mm_subs_pu16(__m64 __a, __m64 __b) {
  __v4hu __x = (__v4hu)__a, __y = (__v4hu)__b, __r;
  int __i;
  for (__i = 0; __i < 4; ++__i)
    __r[__i] = (unsigned short)__BCC_SATURATE(__x[__i] - __y[__i], 0, 65535);
  return (__m64)__r;
}
__BCC_MMX __m64 _mm_madd_pi16(__m64 __a, __m64 __b) {
  __v4hi __x = (__v4hi)__a, __y = (__v4hi)__b;
  return (__m64)(__v2su){(unsigned)(__x[0] * __y[0]) + (unsigned)(__x[1] * __y[1]),
                         (unsigned)(__x[2] * __y[2]) + (unsigned)(__x[3] * __y[3])};
}
__BCC_MMX __m64 _mm_mulhi_pi16(__m64 __a, __m64 __b) {
  __v4hi __x = (__v4hi)__a, __y = (__v4hi)__b, __r;
  int __i;
  for (__i = 0; __i < 4; ++__i)
    __r[__i] = (short)((__x[__i] * __y[__i]) >> 16);
  return (__m64)__r;
}
__BCC_MMX __m64 _mm_mullo_pi16(__m64 __a, __m64 __b) { return (__m64)((__v4hu)__a * (__v4hu)__b); }

/* Shifts; a count beyond the lane's width clears it, or fills it with the
 * sign for an arithmetic shift. */

__BCC_MMX __m64 _mm_slli_pi16(__m64 __m, int __n) {
  return (unsigned)__n > 15 ? (__m64){0} : (__m64)((__v4hu)__m << __n);
}
__BCC_MMX __m64 _mm_slli_pi32(__m64 __m, int __n) {
  return (unsigned)__n > 31 ? (__m64){0} : (__m64)((__v2su)__m << __n);
}
__BCC_MMX __m64 _mm_slli_si64(__m64 __m, int __n) {
  return (unsigned)__n > 63 ? (__m64){0} : (__m64)((__v1du)__m << __n);
}
__BCC_MMX __m64 _mm_srli_pi16(__m64 __m, int __n) {
  return (unsigned)__n > 15 ? (__m64){0} : (__m64)((__v4hu)__m >> __n);
}
__BCC_MMX __m64 _mm_srli_pi32(__m64 __m, int __n) {
  return (unsigned)__n > 31 ? (__m64){0} : (__m64)((__v2su)__m >> __n);
}
__BCC_MMX __m64 _mm_srli_si64(__m64 __m, int __n) {
  return (unsigned)__n > 63 ? (__m64){0} : (__m64)((__v1du)__m >> __n);
}
__BCC_MMX __m64 _mm_srai_pi16(__m64 __m, int __n) {
  return (__m64)((__v4hi)__m >> ((unsigned)__n > 15 ? 15 : __n));
}
__BCC_MMX __m64 _mm_srai_pi32(__m64 __m, int __n) {
  return (__m64)((__v2si)__m >> ((unsigned)__n > 31 ? 31 : __n));
}
__BCC_MMX int __bcc_mmx_count(__m64 __c, unsigned __limit) {
  unsigned long long __n = (unsigned long long)__c[0];
  return __n > __limit ? (int)__limit + 1 : (int)__n;
}
__BCC_MMX __m64 _mm_sll_pi16(__m64 __m, __m64 __c) { return _mm_slli_pi16(__m, __bcc_mmx_count(__c, 15)); }
__BCC_MMX __m64 _mm_sll_pi32(__m64 __m, __m64 __c) { return _mm_slli_pi32(__m, __bcc_mmx_count(__c, 31)); }
__BCC_MMX __m64 _mm_sll_si64(__m64 __m, __m64 __c) { return _mm_slli_si64(__m, __bcc_mmx_count(__c, 63)); }
__BCC_MMX __m64 _mm_srl_pi16(__m64 __m, __m64 __c) { return _mm_srli_pi16(__m, __bcc_mmx_count(__c, 15)); }
__BCC_MMX __m64 _mm_srl_pi32(__m64 __m, __m64 __c) { return _mm_srli_pi32(__m, __bcc_mmx_count(__c, 31)); }
__BCC_MMX __m64 _mm_srl_si64(__m64 __m, __m64 __c) { return _mm_srli_si64(__m, __bcc_mmx_count(__c, 63)); }
__BCC_MMX __m64 _mm_sra_pi16(__m64 __m, __m64 __c) { return _mm_srai_pi16(__m, __bcc_mmx_count(__c, 15)); }
__BCC_MMX __m64 _mm_sra_pi32(__m64 __m, __m64 __c) { return _mm_srai_pi32(__m, __bcc_mmx_count(__c, 31)); }

/* Bitwise operations and comparisons */

__BCC_MMX __m64 _mm_and_si64(__m64 __a, __m64 __b) { return __a & __b; }
__BCC_MMX __m64 _mm_andnot_si64(__m64 __a, __m64 __b) { return ~__a & __b; }
__BCC_MMX __m64 _mm_or_si64(__m64 __a, __m64 __b) { return __a | __b; }
__BCC_MMX __m64 _mm_xor_si64(__m64 __a, __m64 __b) { return __a ^ __b; }
__BCC_MMX __m64 _mm_cmpeq_pi8(__m64 __a, __m64 __b) { return (__m64)((__v8qs)__a == (__v8qs)__b); }
__BCC_MMX __m64 _mm_cmpeq_pi16(__m64 __a, __m64 __b) { return (__m64)((__v4hi)__a == (__v4hi)__b); }
__BCC_MMX __m64 _mm_cmpeq_pi32(__m64 __a, __m64 __b) { return (__m64)((__v2si)__a == (__v2si)__b); }
__BCC_MMX __m64 _mm_cmpgt_pi8(__m64 __a, __m64 __b) { return (__m64)((__v8qs)__a > (__v8qs)__b); }
__BCC_MMX __m64 _mm_cmpgt_pi16(__m64 __a, __m64 __b) { return (__m64)((__v4hi)__a > (__v4hi)__b); }
__BCC_MMX __m64 _mm_cmpgt_pi32(__m64 __a, __m64 __b) { return (__m64)((__v2si)__a > (__v2si)__b); }

/* Construction */

__BCC_MMX __m64 _mm_setzero_si64(void) { return (__m64){0}; }
__BCC_MMX __m64 _mm_set_pi32(int __i1, int __i0) { return (__m64)(__v2si){__i0, __i1}; }
__BCC_MMX __m64 _mm_set_pi16(short __s3, short __s2, short __s1, short __s0) {
  return (__m64)(__v4hi){__s0, __s1, __s2, __s3};
}
__BCC_MMX __m64 _mm_set_pi8(char __b7, char __b6, char __b5, char __b4, char __b3, char __b2,
                            char __b1, char __b0) {
  return (__m64)(__v8qi){__b0, __b1, __b2, __b3, __b4, __b5, __b6, __b7};
}
__BCC_MMX __m64 _mm_set1_pi32(int __i) { return _mm_set_pi32(__i, __i); }
__BCC_MMX __m64 _mm_set1_pi16(short __w) { return _mm_set_pi16(__w, __w, __w, __w); }
__BCC_MMX __m64 _mm_set1_pi8(char __b) { return _mm_set_pi8(__b, __b, __b, __b, __b, __b, __b, __b); }
__BCC_MMX __m64 _mm_setr_pi32(int __i0, int __i1) { return _mm_set_pi32(__i1, __i0); }
__BCC_MMX __m64 _mm_setr_pi16(short __w0, short __w1, short __w2, short __w3) {
  return _mm_set_pi16(__w3, __w2, __w1, __w0);
}
__BCC_MMX __m64 _mm_setr_pi8(char __b0, char __b1, char __b2, char __b3, char __b4, char __b5,
                             char __b6, char __b7) {
  return _mm_set_pi8(__b7, __b6, __b5, __b4, __b3, __b2, __b1, __b0);
}

/* GCC's spellings of the 64-bit conversions. */
#define _mm_cvtsi64x_si64(i) _mm_cvtsi64_m64(i)
#define _mm_cvtsi64_si64x(m) _mm_cvtm64_si64(m)
#define _mm_set_pi64x(i) _mm_cvtsi64_m64(i)

#define _m_empty _mm_empty
#define _m_from_int _mm_cvtsi32_si64
#define _m_from_int64 _mm_cvtsi64_m64
#define _m_to_int _mm_cvtsi64_si32
#define _m_to_int64 _mm_cvtm64_si64
#define _m_packsswb _mm_packs_pi16
#define _m_packssdw _mm_packs_pi32
#define _m_packuswb _mm_packs_pu16
#define _m_punpckhbw _mm_unpackhi_pi8
#define _m_punpckhwd _mm_unpackhi_pi16
#define _m_punpckhdq _mm_unpackhi_pi32
#define _m_punpcklbw _mm_unpacklo_pi8
#define _m_punpcklwd _mm_unpacklo_pi16
#define _m_punpckldq _mm_unpacklo_pi32
#define _m_paddb _mm_add_pi8
#define _m_paddw _mm_add_pi16
#define _m_paddd _mm_add_pi32
#define _m_paddsb _mm_adds_pi8
#define _m_paddsw _mm_adds_pi16
#define _m_paddusb _mm_adds_pu8
#define _m_paddusw _mm_adds_pu16
#define _m_psubb _mm_sub_pi8
#define _m_psubw _mm_sub_pi16
#define _m_psubd _mm_sub_pi32
#define _m_psubsb _mm_subs_pi8
#define _m_psubsw _mm_subs_pi16
#define _m_psubusb _mm_subs_pu8
#define _m_psubusw _mm_subs_pu16
#define _m_pmaddwd _mm_madd_pi16
#define _m_pmulhw _mm_mulhi_pi16
#define _m_pmullw _mm_mullo_pi16
#define _m_psllw _mm_sll_pi16
#define _m_psllwi _mm_slli_pi16
#define _m_pslld _mm_sll_pi32
#define _m_pslldi _mm_slli_pi32
#define _m_psllq _mm_sll_si64
#define _m_psllqi _mm_slli_si64
#define _m_psraw _mm_sra_pi16
#define _m_psrawi _mm_srai_pi16
#define _m_psrad _mm_sra_pi32
#define _m_psradi _mm_srai_pi32
#define _m_psrlw _mm_srl_pi16
#define _m_psrlwi _mm_srli_pi16
#define _m_psrld _mm_srl_pi32
#define _m_psrldi _mm_srli_pi32
#define _m_psrlq _mm_srl_si64
#define _m_psrlqi _mm_srli_si64
#define _m_pand _mm_and_si64
#define _m_pandn _mm_andnot_si64
#define _m_por _mm_or_si64
#define _m_pxor _mm_xor_si64
#define _m_pcmpeqb _mm_cmpeq_pi8
#define _m_pcmpeqw _mm_cmpeq_pi16
#define _m_pcmpeqd _mm_cmpeq_pi32
#define _m_pcmpgtb _mm_cmpgt_pi8
#define _m_pcmpgtw _mm_cmpgt_pi16
#define _m_pcmpgtd _mm_cmpgt_pi32

#endif /* __MMINTRIN_H */
