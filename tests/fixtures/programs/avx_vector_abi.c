/* 32-byte vectors with AVX on the command line (-mavx2) in each place the
 * System V psABI gives them (§3.2.3): arguments in %ymm0-%ymm7 and, once
 * those are used, on the stack aligned to 32, mixed with scalar arguments;
 * a result in %ymm0; a record holding one vector, which travels as the
 * vector, and a larger one, which is memory; unnamed vectors that `va_arg`
 * reads from the stack (§3.5.7); a 64-byte result, in memory for GCC and in
 * %ymm0 and %ymm1 for Clang; calls through pointers in both directions; and
 * a static function only this unit calls. With CALLEE the definitions,
 * otherwise the calls; with WHOLE both. Each half links with the other
 * compiler's build of the other. ALWAYS_FAST skips the processor check,
 * which the interpreter cannot make. */
#include <immintrin.h>
#include <stdarg.h>
#include <stdio.h>

typedef float v16sf __attribute__((vector_size(64)));
struct wrap {
  __m256 v;
};
struct tagged {
  __m256d v;
  int tag;
};
typedef __m256i (*binary)(__m256i, __m256i);

__m256i add8(__m256i a, __m256i b);
__m256d mix(double x, __m256d a, int k, __m128 q, __m256d b);
__m256 nine(__m256 a0, __m256 a1, __m256 a2, __m256 a3, __m256 a4, __m256 a5, __m256 a6,
            __m256 a7, __m256 a8, float f, double g, double h, double i, double j, double k,
            double l, double m);
struct wrap wrapped(struct wrap w, float x);
struct tagged tagged_add(struct tagged t, __m256d d);
__m256 sum_va(int n, ...);
v16sf scale16(v16sf a, float s);
__m256i apply(binary f, __m256i a, __m256i b);
extern const binary table[1];

#if defined(CALLEE) || defined(WHOLE)
__m256i add8(__m256i a, __m256i b) { return _mm256_add_epi32(a, b); }

__m256d mix(double x, __m256d a, int k, __m128 q, __m256d b) {
  __m256d wide_q = _mm256_cvtps_pd(q);
  return _mm256_add_pd(_mm256_mul_pd(a, _mm256_set1_pd(x * k)), _mm256_add_pd(b, wide_q));
}

__m256 nine(__m256 a0, __m256 a1, __m256 a2, __m256 a3, __m256 a4, __m256 a5, __m256 a6,
            __m256 a7, __m256 a8, float f, double g, double h, double i, double j, double k,
            double l, double m) {
  __m256 sum = _mm256_add_ps(_mm256_add_ps(_mm256_add_ps(a0, a1), _mm256_add_ps(a2, a3)),
                             _mm256_add_ps(_mm256_add_ps(a4, a5), _mm256_add_ps(a6, a7)));
  sum = _mm256_sub_ps(sum, a8);
  return _mm256_add_ps(sum, _mm256_set1_ps(f + (float)(g - h + i - j + k - l + m)));
}

struct wrap wrapped(struct wrap w, float x) {
  w.v = _mm256_mul_ps(w.v, _mm256_set1_ps(x));
  return w;
}

struct tagged tagged_add(struct tagged t, __m256d d) {
  t.v = _mm256_add_pd(t.v, d);
  t.tag = -t.tag;
  return t;
}

/* Pairs of a double, in the next free %xmm register while one is, and a
 * vector, always on the stack. */
__m256 sum_va(int n, ...) {
  va_list ap;
  va_start(ap, n);
  __m256 sum = _mm256_setzero_ps();
  for (int i = 0; i < n; i++) {
    float scale = (float)va_arg(ap, double);
    __m256 v = va_arg(ap, __m256);
    sum = _mm256_add_ps(sum, _mm256_mul_ps(v, _mm256_set1_ps(scale)));
  }
  va_end(ap);
  return sum;
}

v16sf scale16(v16sf a, float s) { return a * s; }

__m256i apply(binary f, __m256i a, __m256i b) { return f(_mm256_slli_epi32(a, 1), b); }

/* Reached through a pointer: the address escapes to the caller's table. */
static __m256i xor8(__m256i a, __m256i b) { return _mm256_xor_si256(a, b); }
const binary table[1] = {xor8};
#endif

#if !defined(CALLEE) || defined(WHOLE)
static void print(const char *name, const void *p, int n) {
  const unsigned char *b = p;
  printf("%s:", name);
  for (int i = n - 1; i >= 0; i--) printf("%02x", b[i]);
  printf("\n");
}

/* Only this unit calls it, so it keeps bcc's own convention. */
static __attribute__((noinline)) __m256 twice(__m256 a, int n) {
  return n ? twice(_mm256_add_ps(a, a), n - 1) : a;
}

/* Called back from the other half, through a pointer. */
static __m256i sub8(__m256i a, __m256i b) { return _mm256_sub_epi32(a, b); }

static void run(void) {
  __m256i a = _mm256_setr_epi32(1, -2, 3, -4, 5, -6, 7, 0x7fffffff);
  __m256i b = _mm256_setr_epi32(10, 20, 30, 40, 50, 60, 70, 1);
  __m256i sum = add8(a, b);
  print("add8", &sum, 32);
  __m256d m = mix(2.5, _mm256_setr_pd(1, -2, 0.5, 1e10), 3, _mm_setr_ps(1, 2, 4, 8),
                  _mm256_setr_pd(-1, 4, 8, 0.25));
  print("mix", &m, 32);
  __m256 v[9];
  for (int i = 0; i < 9; i++)
    v[i] = _mm256_setr_ps(i, i + 0.5f, -i, i * 2, i * 0.25f, 1, -1, i * i);
  __m256 n = nine(v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7], v[8], 0.75f, 1, 2, 4, 8, 16, 32,
                  64);
  print("nine", &n, 32);
  struct wrap w = wrapped((struct wrap){_mm256_setr_ps(1, -2, 3.5f, -4, 5, 6, 0.25f, 8)}, 1.5f);
  print("wrapped", &w, 32);
  struct tagged t = tagged_add((struct tagged){_mm256_setr_pd(1, 2, 3, 4), 17},
                               _mm256_set1_pd(0.5));
  print("tagged", &t.v, 32);
  printf("tag:%d\n", t.tag);
  __m256 s = sum_va(3, 2.0, v[1], 0.5, v[2], -1.0, v[3]);
  print("sum_va", &s, 32);
  v16sf big = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16};
  v16sf scaled = scale16(big, -0.5f);
  print("scale16", &scaled, 64);
  __m256i applied = apply(sub8, a, b);
  print("apply", &applied, 32);
  __m256i x = table[0](a, b);
  print("table", &x, 32);
  __m256 doubled = twice(v[3], 3);
  print("twice", &doubled, 32);
}

int main(void) {
#ifndef ALWAYS_FAST
  __builtin_cpu_init();
  if (!__builtin_cpu_supports("avx2")) {
    printf("skip\n");
    return 0;
  }
#endif
  run();
  return 0;
}
#endif
