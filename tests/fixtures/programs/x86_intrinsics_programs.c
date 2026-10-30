/* Typical SSE2 code built on bcc's own intrinsic headers, compared with the
 * host GCC's build: a vectorized strlen, float and integer reductions, a 4x4
 * matrix product with _MM_TRANSPOSE4_PS, saturating image blends, a byte
 * histogram by comparisons, and a dot product in doubles. */
#include <emmintrin.h>
#include <stdio.h>
#include <string.h>

static size_t sse2_strlen(const char *s) {
  size_t offset = (size_t)((unsigned long long)s & 15);
  const __m128i *p = (const __m128i *)(s - offset);
  int mask = _mm_movemask_epi8(_mm_cmpeq_epi8(_mm_load_si128(p), _mm_setzero_si128())) >> offset;
  if (mask)
    return (size_t)__builtin_ctz(mask);
  for (size_t n = 16 - offset;; n += 16) {
    mask = _mm_movemask_epi8(_mm_cmpeq_epi8(_mm_load_si128(++p), _mm_setzero_si128()));
    if (mask)
      return n + (size_t)__builtin_ctz(mask);
  }
}

static float sum_floats(const float *v, int n) {
  __m128 acc = _mm_setzero_ps();
  int i = 0;
  for (; i + 4 <= n; i += 4)
    acc = _mm_add_ps(acc, _mm_loadu_ps(v + i));
  acc = _mm_add_ps(acc, _mm_movehl_ps(acc, acc));
  acc = _mm_add_ss(acc, _mm_shuffle_ps(acc, acc, 1));
  float s = _mm_cvtss_f32(acc);
  for (; i < n; i++)
    s += v[i];
  return s;
}

static long long sum_shorts(const short *v, int n) {
  __m128i acc = _mm_setzero_si128();
  for (int i = 0; i < n; i += 8) {
    __m128i x = _mm_loadu_si128((const __m128i *)(v + i));
    acc = _mm_add_epi32(acc, _mm_madd_epi16(x, _mm_set1_epi16(1)));
  }
  int lanes[4];
  _mm_storeu_si128((__m128i *)lanes, acc);
  return (long long)lanes[0] + lanes[1] + lanes[2] + lanes[3];
}

static void matmul4(const float *a, const float *b, float *c) {
  __m128 r0 = _mm_loadu_ps(b), r1 = _mm_loadu_ps(b + 4), r2 = _mm_loadu_ps(b + 8), r3 = _mm_loadu_ps(b + 12);
  _MM_TRANSPOSE4_PS(r0, r1, r2, r3);
  for (int i = 0; i < 4; i++) {
    __m128 row = _mm_loadu_ps(a + 4 * i);
    __m128 p0 = _mm_mul_ps(row, r0), p1 = _mm_mul_ps(row, r1), p2 = _mm_mul_ps(row, r2), p3 = _mm_mul_ps(row, r3);
    _MM_TRANSPOSE4_PS(p0, p1, p2, p3);
    _mm_storeu_ps(c + 4 * i, _mm_add_ps(_mm_add_ps(p0, p1), _mm_add_ps(p2, p3)));
  }
}

static void blend(unsigned char *dst, const unsigned char *a, const unsigned char *b, int n) {
  for (int i = 0; i < n; i += 16) {
    __m128i x = _mm_loadu_si128((const __m128i *)(a + i)), y = _mm_loadu_si128((const __m128i *)(b + i));
    __m128i lo = _mm_avg_epu8(x, y), hi = _mm_adds_epu8(x, y), d = _mm_subs_epu8(x, y);
    _mm_storeu_si128((__m128i *)(dst + i), _mm_xor_si128(_mm_xor_si128(lo, hi), d));
  }
}

int main(void) {
  static char text[64] __attribute__((aligned(16)));
  for (int start = 0; start < 5; start++)
    for (int len = 0; len < 40; len += 7) {
      memset(text, 'x', sizeof text);
      text[start + len] = 0;
      printf("strlen(%d, %d) = %zu\n", start, len, sse2_strlen(text + start));
    }
  float v[23];
  for (int i = 0; i < 23; i++)
    v[i] = (float)i * 0.37f - 3.0f;
  printf("sum_floats = %a\n", sum_floats(v, 23));
  short s[32];
  for (int i = 0; i < 32; i++)
    s[i] = (short)(i * 1234 - 20000);
  printf("sum_shorts = %lld\n", sum_shorts(s, 32));
  float a[16], b[16], c[16];
  for (int i = 0; i < 16; i++) {
    a[i] = (float)(i + 1);
    b[i] = (float)(16 - i) * 0.5f;
  }
  matmul4(a, b, c);
  for (int i = 0; i < 16; i++)
    printf("c[%d] = %a\n", i, c[i]);
  unsigned char x[32], y[32], out[32];
  for (int i = 0; i < 32; i++) {
    x[i] = (unsigned char)(i * 37);
    y[i] = (unsigned char)(255 - i * 11);
  }
  blend(out, x, y, 32);
  for (int i = 0; i < 32; i++)
    printf("%02x", out[i]);
  printf("\n");
  int histogram[4] = {0};
  __m128i bytes = _mm_loadu_si128((const __m128i *)x);
  for (int k = 0; k < 4; k++)
    histogram[k] = __builtin_popcount(
        (unsigned)_mm_movemask_epi8(_mm_cmplt_epi8(bytes, _mm_set1_epi8((char)(k * 40 - 64)))));
  printf("histogram %d %d %d %d\n", histogram[0], histogram[1], histogram[2], histogram[3]);
  __m128d dot = _mm_setzero_pd();
  for (int i = 0; i < 8; i += 2)
    dot = _mm_add_pd(dot, _mm_mul_pd(_mm_set_pd(i + 1.5, i * 0.25), _mm_set_pd(1.0 / (i + 1), i - 3.0)));
  printf("dot = %a\n", _mm_cvtsd_f64(_mm_add_sd(dot, _mm_unpackhi_pd(dot, dot))));
  return 0;
}
