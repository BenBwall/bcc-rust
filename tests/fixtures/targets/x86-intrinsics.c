#include <x86intrin.h>
#include <immintrin.h>
#ifndef _MSC_VER
#include <cpuid.h>
#endif
_Static_assert(sizeof(__m64) == 8, "MMX size");
_Static_assert(sizeof(__m128) == 16 && _Alignof(__m128) == 16, "SSE size");
_Static_assert(sizeof(__m128i) == 16 && _Alignof(__m128i_u) == 1, "SSE2 alignment");
_Static_assert(sizeof(__m256d) == 32 && _Alignof(__m256d) == 32, "AVX size");
__m128 add(__m128 a, __m128 b) { return _mm_add_ps(a,b); }
__m128i broadcast(int x) { return _mm_set1_epi32(x); }
__m128i shuffle(__m128i a) { return _mm_shuffle_epi32(a, 0x1b); }
__m128i align(__m128i a, __m128i b) { return _mm_alignr_epi8(a,b,3); }
__m128i blend(__m128i a, __m128i b) { return _mm_blend_epi16(a,b,42); }
__m128i aes(__m128i a) { return _mm_aeskeygenassist_si128(a,1); }
__m128 round_down(__m128 a) { return _mm_round_ps(a, _MM_FROUND_TO_NEG_INF); }
__m256d wide(__m256d a, __m256d b) { return _mm256_add_pd(a,b); }
__m256i integer_wide(__m256i a, __m256i b) { return _mm256_add_epi32(a,b); }
__m256i wide_shuffle(__m256i a) { return _mm256_shuffle_epi32(a,27); }
unsigned crc(unsigned a, unsigned b) { return _mm_crc32_u32(a,b); }
int extract(__m128 a) { return _mm_extract_ps(a,2); }
#ifdef _MSC_VER
#include <intrin.h>
void cpuid(int out[4]) { __cpuid(out, 1); }
long interlocked(long volatile *p) { return _InterlockedIncrement(p); }
#endif
void prefetch(const float *p) { _mm_prefetch((const char *)p, _MM_HINT_T0); }
int population(unsigned x) { return _mm_popcnt_u32(x); }
