/* Functions whose x86 `target` attribute lets their code use AVX2 or BMI2,
 * called only when the processor has those features, as zstd and other
 * libraries choose a fast path at run time. Either path prints the same. */
#include <immintrin.h>
#include <stdio.h>

static int values[67];

__attribute__((target("avx2"))) static int sum_avx2(const int *v, int n) {
  __m256i total = _mm256_setzero_si256();
  int i = 0;
  for (; i + 8 <= n; i += 8)
    total = _mm256_add_epi32(total, _mm256_loadu_si256((const __m256i_u *)(v + i)));
  int lanes[8];
  _mm256_storeu_si256((__m256i_u *)lanes, total);
  int sum = 0;
  for (int lane = 0; lane < 8; lane++)
    sum += lanes[lane];
  for (; i < n; i++)
    sum += v[i];
  return sum;
}

static int sum_plain(const int *v, int n) {
  int sum = 0;
  for (int i = 0; i < n; i++)
    sum += v[i];
  return sum;
}

/* Plain C that BMI2 compiles to SHRX and BZHI, as zstd's bit readers. */
static inline __attribute__((always_inline)) unsigned bits(unsigned x, unsigned start,
                                                          unsigned count) {
  return (x >> start) & ((1u << count) - 1);
}

__attribute__((target("lzcnt,bmi,bmi2"))) static unsigned fields_bmi2(unsigned x) {
  unsigned total = 0;
  for (unsigned start = 0; start < 24; start += 3)
    total += bits(x, start, 5) * (start + 1);
  return total + (unsigned)__builtin_clz(x | 1);
}

static unsigned fields_plain(unsigned x) {
  unsigned total = 0;
  for (unsigned start = 0; start < 24; start += 3)
    total += bits(x, start, 5) * (start + 1);
  return total + (unsigned)__builtin_clz(x | 1);
}

/* A declaration's `target` holds for the definition without one. */
__attribute__((target("popcnt"))) int population(unsigned x);
int population(unsigned x) { return __builtin_popcount(x); }

int main(void) {
  for (int i = 0; i < 67; i++)
    values[i] = i * 37 - 1000;
  __builtin_cpu_init();
  int avx2 = __builtin_cpu_supports("avx2");
  int bmi2 = __builtin_cpu_supports("bmi2") && __builtin_cpu_supports("lzcnt");
  int sum = avx2 ? sum_avx2(values, 67) : sum_plain(values, 67);
  printf("sum %d\n", sum);
  unsigned x = 1;
  for (int step = 0; step < 12; step++, x = x * 7 + 12345)
    printf("fields %u %u\n", x, bmi2 ? fields_bmi2(x) : fields_plain(x));
  printf("population %d\n", population(0xf0f0f0f1u));
  return 0;
}
