/* pmmintrin.h - SSE3 intrinsics for bcc.
 *
 * Written for bcc; part of bcc and under its licence. Plain C over GNU
 * vector operations, as in <xmmintrin.h>. */

#ifndef __PMMINTRIN_H
#define __PMMINTRIN_H

#if !defined(__i386__) && !defined(__x86_64__)
#error "This header is only meant to be used on x86 and x64 architecture"
#endif

#include <emmintrin.h>

#define _MM_DENORMALS_ZERO_ON (0x0040U)
#define _MM_DENORMALS_ZERO_OFF (0x0000U)
#define _MM_DENORMALS_ZERO_MASK (0x0040U)
#define _MM_GET_DENORMALS_ZERO_MODE() (_mm_getcsr() & _MM_DENORMALS_ZERO_MASK)
#define _MM_SET_DENORMALS_ZERO_MODE(x) (_mm_setcsr((_mm_getcsr() & ~_MM_DENORMALS_ZERO_MASK) | (x)))

__BCC_SSE __m128i _mm_lddqu_si128(__m128i_u const *__p) { return *__p; }
__BCC_SSE __m128 _mm_addsub_ps(__m128 __a, __m128 __b) {
  return (__m128){__a[0] - __b[0], __a[1] + __b[1], __a[2] - __b[2], __a[3] + __b[3]};
}
__BCC_SSE __m128 _mm_hadd_ps(__m128 __a, __m128 __b) {
  return (__m128){__a[0] + __a[1], __a[2] + __a[3], __b[0] + __b[1], __b[2] + __b[3]};
}
__BCC_SSE __m128 _mm_hsub_ps(__m128 __a, __m128 __b) {
  return (__m128){__a[0] - __a[1], __a[2] - __a[3], __b[0] - __b[1], __b[2] - __b[3]};
}
__BCC_SSE __m128 _mm_movehdup_ps(__m128 __a) { return (__m128){__a[1], __a[1], __a[3], __a[3]}; }
__BCC_SSE __m128 _mm_moveldup_ps(__m128 __a) { return (__m128){__a[0], __a[0], __a[2], __a[2]}; }
__BCC_SSE __m128d _mm_addsub_pd(__m128d __a, __m128d __b) {
  return (__m128d){__a[0] - __b[0], __a[1] + __b[1]};
}
__BCC_SSE __m128d _mm_hadd_pd(__m128d __a, __m128d __b) { return (__m128d){__a[0] + __a[1], __b[0] + __b[1]}; }
__BCC_SSE __m128d _mm_hsub_pd(__m128d __a, __m128d __b) { return (__m128d){__a[0] - __a[1], __b[0] - __b[1]}; }
#define _mm_loaddup_pd(dp) _mm_load1_pd(dp)
__BCC_SSE __m128d _mm_movedup_pd(__m128d __a) { return (__m128d){__a[0], __a[0]}; }
/* MONITOR and MWAIT are instructions, which the interpreter cannot run. */
__BCC_SSE void _mm_monitor(void const *__p, unsigned __extensions, unsigned __hints) {
  __asm__ __volatile__("monitor" ::"a"(__p), "c"(__extensions), "d"(__hints));
}
__BCC_SSE void _mm_mwait(unsigned __extensions, unsigned __hints) {
  __asm__ __volatile__("mwait" ::"c"(__extensions), "a"(__hints));
}

#endif /* __PMMINTRIN_H */
