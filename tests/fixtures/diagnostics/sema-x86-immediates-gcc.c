#include <immintrin.h>
__m256 dot(__m256 a, __m256 b) { return _mm256_dp_ps(a, b, 256); }
__m128i shuffled(__m128i v) { return _mm_shuffle_epi32(v, 256); }
__m128i product(__m128i a, __m128i b) { return _mm_clmulepi64_si128(a, b, 0x80); }
__m128i negative(__m128i a, __m128i b) { return _mm_clmulepi64_si128(a, b, -1); }
int extracted(__m128i v) { return _mm_extract_epi16(v, 8); }
int compared(__m128i a, __m128i b) { return _mm_cmpistri(a, b, 256); }
__m128 rounded(__m128 a) { return _mm_round_ps(a, 16); }
__m128d predicate(__m128d a, __m128d b) { return _mm_cmp_pd(a, b, 32); }
__m256i halves(__m256i a) { return _mm256_permute2x128_si256(a, a, 0x1ff); }
__m128i variable(__m128i v, int i) { return _mm_shuffle_epi32(v, i); }
int following;
