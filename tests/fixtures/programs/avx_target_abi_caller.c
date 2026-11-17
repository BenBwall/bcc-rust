/* Calls the functions of `avx_target_abi_callee.c` from functions whose
 * `target` attribute turns AVX on, and prints the results. */
#include <immintrin.h>
#include <stdio.h>

/* Clang refuses a call that passes a 32-byte vector to a function declared
 * without AVX from one with it. */
__attribute__((target("avx2"))) __m256i avx_add(__m256i, __m256i, int);
__attribute__((target("avx"))) __m256d avx_mix(double, __m256d, __m256d);
extern __m256i (*const avx_table[1])(__m256i, __m256i);
struct wrap {
  __m256 v;
};
__attribute__((target("avx"))) struct wrap avx_wrap(struct wrap, float);

static void print(const char *name, const void *p, int n) {
  const unsigned char *b = p;
  printf("%s:", name);
  for (int i = n - 1; i >= 0; i--) printf("%02x", b[i]);
  printf("\n");
}

__attribute__((target("avx2"))) static void run(void) {
  __m256i a = _mm256_setr_epi32(1, -2, 3, -4, 5, -6, 7, 0x7fffffff);
  __m256i b = _mm256_setr_epi32(10, 20, 30, 40, 50, 60, 70, 1);
  __m256i sum = avx_add(a, b, 3);
  print("avx_add", &sum, 32);
  __m256d mix = avx_mix(2.5, _mm256_setr_pd(1, -2, 0.5, 1e10), _mm256_setr_pd(-1, 4, 8, 0.25));
  print("avx_mix", &mix, 32);
  __m256i x = avx_table[0](a, b);
  print("avx_xor", &x, 32);
  struct wrap w = avx_wrap((struct wrap){_mm256_setr_ps(1, -2, 3.5f, -4, 5, 6, 0.25f, 8)}, 1.5f);
  print("avx_wrap", &w, 32);
}

int main(void) {
  __builtin_cpu_init();
  if (!__builtin_cpu_supports("avx2")) {
    printf("skip\n");
    return 0;
  }
  run();
  return 0;
}
