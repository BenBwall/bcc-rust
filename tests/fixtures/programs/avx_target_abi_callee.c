/* External functions whose `target` attribute turns AVX on, in a unit built
 * without AVX: each compiler passes their 32-byte vectors as it classifies
 * them for such a function (GCC in YMM registers, Clang's arguments in
 * memory and its result in YMM0, but a record holding one in memory, as
 * its arguments). Linked with `avx_target_abi_caller.c` built by the other
 * compiler. */
#include <immintrin.h>

__attribute__((target("avx2"))) __m256i avx_add(__m256i a, __m256i b, int scale) {
  return _mm256_add_epi32(a, _mm256_mullo_epi32(b, _mm256_set1_epi32(scale)));
}

__attribute__((target("avx"))) __m256d avx_mix(double x, __m256d a, __m256d b) {
  return _mm256_add_pd(_mm256_mul_pd(a, _mm256_set1_pd(x)), b);
}

/* Reached through a pointer: the address escapes to the caller's table. */
__attribute__((target("avx2"))) static __m256i avx_xor(__m256i a, __m256i b) {
  return _mm256_xor_si256(a, b);
}
__m256i (*const avx_table[1])(__m256i, __m256i) = {avx_xor};

/* A record holding one vector: GCC passes and returns it in YMM registers
 * as the vector, Clang passes and returns it in memory. */
struct wrap {
  __m256 v;
};
__attribute__((target("avx"))) struct wrap avx_wrap(struct wrap w, float x) {
  w.v = _mm256_add_ps(w.v, _mm256_set1_ps(x));
  return w;
}
