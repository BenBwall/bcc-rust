/* The 16-byte x86 instructions bcc keeps as named IR instructions (AES-NI,
 * PCLMULQDQ, the SSE4.1 roundings and the SSE4.2 string comparisons), each
 * compiled twice: in a function whose `target` attribute leaves AVX off,
 * where the instructions take their SSE forms, and in one with AVX, where
 * they take their VEX forms. The roundings and string comparisons are
 * called as the builtins both compilers share, since bcc's <smmintrin.h>
 * computes their intrinsics in C. `_xgetbv` reads XCR0 in a function with
 * XSAVE; only its x87, SSE and AVX bits print, since the others depend on
 * the processor. Compared with the builds of GCC and the pinned Clang. Each
 * instruction runs on six generated input sets: random bytes, special
 * `float` and `double` lanes, and strings of small bytes with zeros.
 *
 * `main` has no `target` attribute and prints only "skip" on a processor
 * without AES, PCLMULQDQ, SSE4.2 or AVX (ALWAYS_FAST runs the functions
 * unasked, for the interpreter). */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

typedef float v4sf __attribute__((vector_size(16)));
typedef double v2df __attribute__((vector_size(16)));
typedef char v16qi __attribute__((vector_size(16)));

static unsigned long long state = 0x2545f4914f6cdd1dull;
static unsigned long long next(void) {
  state ^= state << 13;
  state ^= state >> 7;
  state ^= state << 17;
  return state;
}

static const float floats[] = {0.0f, -0.0f, 1.0f, -1.0f, 0.5f, -0.5f, 1.5f, 2.5f, -2.5f, 3.5f,
                               1e-40f, 3e38f, __builtin_inff(), -__builtin_inff(), __builtin_nanf(""),
                               -__builtin_nanf("7"), 8388607.5f, -8388607.5f, 0.49999997f, 1e10f,
                               0.1f, 123456.7f, -7.75f, 4.0f};
static const double doubles[] = {0.0, -0.0, 1.0, -1.0, 0.5, -0.5, 1.5, 2.5, -2.5, 3.5, 4e-320,
                                 1.7e308, __builtin_inf(), -__builtin_inf(), __builtin_nan(""),
                                 -__builtin_nan("9"), 4503599627370495.5, -4503599627370495.5,
                                 0.49999999999999994, 1e10, 0.1, -123456.789, 9.0, 1e-3};

/* Input set `kind` into `p`, 16 bytes. */
static void generate(int kind, unsigned char *p) {
  for (int i = 0; i < 4; i++) {
    unsigned long long r = next();
    switch (kind) {
    case 0: memcpy(p + 4 * i, &r, 4); break;
    case 1: memcpy(p + 4 * i, &floats[r % (sizeof floats / sizeof *floats)], 4); break;
    case 2:
      if (i < 2) memcpy(p + 8 * i, &doubles[r % (sizeof doubles / sizeof *doubles)], 8);
      break;
    case 3:
      /* Letters, digits and zeros, for the string comparisons. */
      for (int k = 0; k < 4; k++) p[4 * i + k] = "abcz09AZ\0\0-~"[(r >> (8 * k)) % 12];
      break;
    case 4:
      for (int k = 0; k < 4; k++) p[4 * i + k] = (unsigned char)(((r >> (8 * k)) % 6) + 'a');
      break;
    default:
      for (int k = 0; k < 4; k++) p[4 * i + k] = (unsigned char)((r >> (8 * k)) % 5 ? 'a' + (r >> (8 * k + 3)) % 3 : 0);
      break;
    }
  }
}

static void hex(const char *name, const void *p, int n) {
  const unsigned char *b = p;
  printf("%s:", name);
  for (int i = n - 1; i >= 0; i--) printf("%02x", b[i]);
  printf("\n");
}
#define P(name, expr) do { __typeof__(expr) v_ = (expr); hex(name, &v_, sizeof v_); } while (0)
#define I(name, expr) printf("%s:%d\n", name, (int)(expr))

/* One body for both functions; `in` holds the six input sets. */
#define NAMED(function, features)                                                                  \
  __attribute__((target(features))) static void function(const unsigned char (*in)[16]) {        \
    __m128i x, y, s, t;                                                                           \
    v4sf f, g;                                                                                    \
    v2df d, e;                                                                                    \
    memcpy(&x, in[0], 16); memcpy(&y, in[5], 16); memcpy(&f, in[1], 16); memcpy(&g, in[4], 16);  \
    memcpy(&d, in[2], 16); memcpy(&e, in[0], 16); memcpy(&s, in[3], 16); memcpy(&t, in[4], 16);  \
    P("aesenc", _mm_aesenc_si128(x, y)); P("aesenclast", _mm_aesenclast_si128(x, y));             \
    P("aesdec", _mm_aesdec_si128(x, y)); P("aesdeclast", _mm_aesdeclast_si128(x, y));             \
    P("aesimc", _mm_aesimc_si128(x)); P("aeskeygen_00", _mm_aeskeygenassist_si128(x, 0x00));       \
    P("aeskeygen_36", _mm_aeskeygenassist_si128(y, 0x36));                                        \
    P("clmul_00", _mm_clmulepi64_si128(x, y, 0x00)); P("clmul_01", _mm_clmulepi64_si128(x, y, 0x01)); \
    P("clmul_10", _mm_clmulepi64_si128(x, y, 0x10)); P("clmul_11", _mm_clmulepi64_si128(x, y, 0x11)); \
    P("roundps_0", __builtin_ia32_roundps(f, 0)); P("roundps_1", __builtin_ia32_roundps(f, 1));    \
    P("roundps_2", __builtin_ia32_roundps(f, 2)); P("roundps_3", __builtin_ia32_roundps(f, 3));    \
    P("roundps_4", __builtin_ia32_roundps(f, 4)); P("roundps_9", __builtin_ia32_roundps(f, 9));    \
    P("roundpd_0", __builtin_ia32_roundpd(d, 0)); P("roundpd_1", __builtin_ia32_roundpd(d, 1));    \
    P("roundpd_2", __builtin_ia32_roundpd(d, 2)); P("roundpd_11", __builtin_ia32_roundpd(d, 11));  \
    P("roundss_1", __builtin_ia32_roundss(g, f, 1)); P("roundss_10", __builtin_ia32_roundss(g, f, 10)); \
    P("roundsd_2", __builtin_ia32_roundsd(e, d, 2)); P("roundsd_8", __builtin_ia32_roundsd(e, d, 8)); \
    v16qi a = (v16qi)s, b = (v16qi)t, c = (v16qi)y;                                                \
    I("istri_00", __builtin_ia32_pcmpistri128(a, b, 0x00));                                        \
    I("istri_0c", __builtin_ia32_pcmpistri128(a, b, 0x0c));                                        \
    I("istri_44", __builtin_ia32_pcmpistri128(b, a, 0x44));                                        \
    I("istri_19", __builtin_ia32_pcmpistri128(a, c, 0x19));                                        \
    I("istri_4a", __builtin_ia32_pcmpistri128(c, b, 0x4a));                                        \
    P("istrm_40", __builtin_ia32_pcmpistrm128(a, b, 0x40));                                        \
    P("istrm_08", __builtin_ia32_pcmpistrm128(a, b, 0x08));                                        \
    P("istrm_45", __builtin_ia32_pcmpistrm128(b, c, 0x45));                                        \
    I("istria", __builtin_ia32_pcmpistria128(a, b, 0x0c));                                         \
    I("istric", __builtin_ia32_pcmpistric128(a, b, 0x00));                                         \
    I("istrio", __builtin_ia32_pcmpistrio128(a, b, 0x08));                                         \
    I("istris", __builtin_ia32_pcmpistris128(a, b, 0x04));                                         \
    I("istriz", __builtin_ia32_pcmpistriz128(c, b, 0x01));                                         \
    int la = (int)(next() % 40) - 12, lb = (int)(next() % 40) - 12;                                \
    I("estri_00", __builtin_ia32_pcmpestri128(a, la, b, lb, 0x00));                                \
    I("estri_0c", __builtin_ia32_pcmpestri128(a, 5, b, 16, 0x0c));                                 \
    I("estri_4d", __builtin_ia32_pcmpestri128(b, lb, c, la, 0x4d));                                \
    I("estri_38", __builtin_ia32_pcmpestri128(c, -3, a, 99, 0x38));                                \
    P("estrm_40", __builtin_ia32_pcmpestrm128(a, la, b, lb, 0x40));                                \
    P("estrm_09", __builtin_ia32_pcmpestrm128(c, 7, b, lb, 0x09));                                 \
    I("estria", __builtin_ia32_pcmpestria128(a, la, b, lb, 0x0c));                                 \
    I("estric", __builtin_ia32_pcmpestric128(a, la, b, lb, 0x00));                                 \
    I("estrio", __builtin_ia32_pcmpestrio128(a, la, b, lb, 0x08));                                 \
    I("estris", __builtin_ia32_pcmpestris128(a, la, b, lb, 0x04));                                 \
    I("estriz", __builtin_ia32_pcmpestriz128(a, la, b, lb, 0x01));                                 \
  }

NAMED(sse, "aes,pclmul,sse4.2")
NAMED(vex, "avx,aes,pclmul")

__attribute__((target("xsave"))) static unsigned long long xcr0(void) {
  return (unsigned long long)_xgetbv(0);
}

int main(void) {
#ifndef ALWAYS_FAST
  __builtin_cpu_init();
  if (!__builtin_cpu_supports("aes") || !__builtin_cpu_supports("pclmul") ||
      !__builtin_cpu_supports("sse4.2") || !__builtin_cpu_supports("avx")) {
    printf("skip\n");
    return 0;
  }
#endif
  printf("xcr0 x87, sse, avx: %llx\n", xcr0() & 7);
  for (int round = 0; round < 6; round++) {
    unsigned char in[6][16];
    for (int k = 0; k < 6; k++) generate((round + k) % 6, in[k]);
    printf("round %d sse\n", round);
    sse((const unsigned char (*)[16])in);
    printf("round %d vex\n", round);
    vex((const unsigned char (*)[16])in);
  }
  return 0;
}
