/* AVX, AVX2, AES-NI and PCLMULQDQ intrinsics from bcc's own <immintrin.h>
 * (the functions and macros whose `__builtin_ia32_*` and Clang elementwise
 * builtins lowering expands), compared with the builds of the host GCC and
 * the pinned Clang. Each intrinsic runs on six generated input sets: random
 * bytes, `float` and `double` lanes drawn from special values (zeros,
 * halves, infinities, NaNs, values beyond `int`), 16- and 32-bit boundary
 * integers, and small bytes for counts and indices. Each result prints in
 * hex.
 *
 * The test functions have a `target` attribute, so `main` needs no AVX and
 * prints only "skip" on a processor without AVX2, AES, PCLMULQDQ or
 * SSE4.2 (ALWAYS_FAST runs them unasked, for the interpreter). NO_NAMED
 * leaves out the intrinsics lowering keeps as named instructions (AES,
 * PCLMULQDQ, rounding, conversions, square roots, approximate reciprocals,
 * VZEROUPPER), which the native back end refuses; NO_APPROX leaves out the
 * approximate reciprocals, whose values each processor chooses, which the
 * interpreter does not compute. */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

#define T __attribute__((target("avx2,aes,pclmul,sse4.2,popcnt")))

static unsigned long long state = 0x9e3779b97f4a7c15ull;
static unsigned long long next(void) {
  state ^= state << 13;
  state ^= state >> 7;
  state ^= state << 17;
  return state;
}

static const float floats[] = {0.0f, -0.0f, 1.0f, -1.0f, 0.5f, -0.5f, 1.5f, 2.5f, -2.5f, 3.5f,
                               1e-40f, 3e38f, __builtin_inff(), -__builtin_inff(), __builtin_nanf(""),
                               -__builtin_nanf("7"), 2147483520.0f, -2147483648.0f, 2147483648.0f,
                               1e10f, 0.1f, 123456.7f, -7.75f, 4.0f};
static const double doubles[] = {0.0, -0.0, 1.0, -1.0, 0.5, -0.5, 1.5, 2.5, -2.5, 3.5, 4e-320,
                                 1.7e308, __builtin_inf(), -__builtin_inf(), __builtin_nan(""),
                                 -__builtin_nan("9"), 2147483647.5, -2147483648.5, 2147483648.0,
                                 1e10, 0.1, -123456.789, 9.0, 1e-3};
static const short shorts[] = {0, 1, -1, 2, 127, 128, 255, 256, -128, -129, 32767, -32768, 16384,
                               -16384, 12345, -300};
static const int ints[] = {0, 1, -1, 2, 31, 32, 65535, 65536, -65536, 2147483647, -2147483647 - 1,
                           1 << 30, -12345678, 100, 7, -3};

/* Input set `kind` into `p`, 32 bytes. */
static void generate(int kind, void *p) {
  unsigned char *b = p;
  for (int i = 0; i < 8; i++) {
    unsigned long long r = next();
    switch (kind) {
    case 0: memcpy(b + 4 * i, &r, 4); break;
    case 1: memcpy(b + 4 * i, &floats[r % (sizeof floats / sizeof *floats)], 4); break;
    case 2:
      if (i < 4) memcpy(b + 8 * i, &doubles[r % (sizeof doubles / sizeof *doubles)], 8);
      break;
    case 3:
      memcpy(b + 4 * i, &shorts[r % 16], 2);
      memcpy(b + 4 * i + 2, &shorts[(r >> 8) % 16], 2);
      break;
    case 4:
      for (int k = 0; k < 4; k++) b[4 * i + k] = (unsigned char)((r >> (8 * k)) % 40) ^ ((r >> 40 & 1) << 7);
      break;
    default: memcpy(b + 4 * i, &ints[r % 16], 4); break;
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

static int base_i[64];
static long long base_q[64];
static float base_f[64];
static double base_d[64];

/* Vectors travel through memory: GCC passes a 32-byte vector argument to a
 * function with AVX in YMM registers, which `main`, without AVX, cannot. */
T static void avx(const unsigned char (*in)[32], int round) {
  __m256 a = _mm256_loadu_ps((const float *)in[1]), b = _mm256_loadu_ps((const float *)in[(round & 1) ? 1 : 4]);
  __m256 c = _mm256_loadu_ps((const float *)in[0]);
  __m256d d = _mm256_loadu_pd((const double *)in[2]), e = _mm256_loadu_pd((const double *)in[(round & 1) ? 2 : 0]);
  __m256d f = _mm256_loadu_pd((const double *)in[5]);
  __m256i ai = _mm256_castps_si256(a), bi = _mm256_castps_si256(b);
  __m128 a4 = _mm256_castps256_ps128(a), b4 = _mm256_castps256_ps128(b);
  __m128d d2 = _mm256_castpd256_pd128(d), e2 = _mm256_castpd256_pd128(e);
  P("addsub_ps", _mm256_addsub_ps(a, b)); P("addsub_pd", _mm256_addsub_pd(d, e));
  P("hadd_ps", _mm256_hadd_ps(a, b)); P("hsub_ps", _mm256_hsub_ps(a, b));
  P("hadd_pd", _mm256_hadd_pd(d, e)); P("hsub_pd", _mm256_hsub_pd(d, e));
  P("min_ps", _mm256_min_ps(a, b)); P("max_ps", _mm256_max_ps(a, b));
  P("min_pd", _mm256_min_pd(d, e)); P("max_pd", _mm256_max_pd(d, e));
  P("blendv_ps", _mm256_blendv_ps(a, b, c)); P("blendv_pd", _mm256_blendv_pd(d, e, f));
  P("cvtpd_ps", _mm256_cvtpd_ps(d));
  P("movemask_ps", _mm256_movemask_ps(a)); P("movemask_pd", _mm256_movemask_pd(d));
  P("testz_ps", _mm256_testz_ps(a, b)); P("testc_ps", _mm256_testc_ps(a, b)); P("testnzc_ps", _mm256_testnzc_ps(a, b));
  P("testz_pd", _mm256_testz_pd(d, e)); P("testc_pd", _mm256_testc_pd(d, e)); P("testnzc_pd", _mm256_testnzc_pd(d, e));
  P("testz_ps128", _mm_testz_ps(a4, b4)); P("testc_ps128", _mm_testc_ps(a4, b4)); P("testnzc_ps128", _mm_testnzc_ps(a4, b4));
  P("testz_pd128", _mm_testz_pd(d2, e2)); P("testc_pd128", _mm_testc_pd(d2, e2)); P("testnzc_pd128", _mm_testnzc_pd(d2, e2));
  P("testz_si256", _mm256_testz_si256(ai, bi)); P("testc_si256", _mm256_testc_si256(ai, bi));
  P("testnzc_si256", _mm256_testnzc_si256(ai, bi));
  P("testz_self", _mm256_testz_si256(ai, _mm256_setzero_si256())); P("testc_self", _mm256_testc_si256(ai, ai));
  P("permutevar_ps", _mm256_permutevar_ps(a, bi)); P("permutevar_pd", _mm256_permutevar_pd(d, bi));
  P("permutevar_ps128", _mm_permutevar_ps(a4, _mm256_castsi256_si128(bi)));
  P("permutevar_pd128", _mm_permutevar_pd(d2, _mm256_castsi256_si128(bi)));
  P("permute_ps", _mm_permute_ps(a4, 0x1b)); P("permute_ps256", _mm256_permute_ps(a, 0xc6));
  P("permute_pd", _mm_permute_pd(d2, 1)); P("permute_pd256", _mm256_permute_pd(d, 0x9));
  P("permute2f128_ps", _mm256_permute2f128_ps(a, b, 0x21)); P("permute2f128_pd", _mm256_permute2f128_pd(d, e, 0x83));
  P("permute2f128_si256", _mm256_permute2f128_si256(ai, bi, 0x30)); P("permute2f128_z", _mm256_permute2f128_ps(a, b, 0x88));
  P("blend_ps", _mm256_blend_ps(a, b, 0xa6)); P("blend_pd", _mm256_blend_pd(d, e, 0x5));
  P("dp_ps", _mm256_dp_ps(a, b, 0xff)); P("dp_ps2", _mm256_dp_ps(a, b, 0x7a));
  P("shuffle_ps", _mm256_shuffle_ps(a, b, 0x4e)); P("shuffle_pd", _mm256_shuffle_pd(d, e, 0xa));
  P("extract_epi32", _mm256_extract_epi32(ai, 5)); P("extract_epi16", _mm256_extract_epi16(ai, 13));
  P("extract_epi8", _mm256_extract_epi8(ai, 30)); P("extract_epi64", _mm256_extract_epi64(ai, 3));
  P("insert_epi32", _mm256_insert_epi32(ai, -7, 6)); P("insert_epi16", _mm256_insert_epi16(ai, 0x1234, 9));
  P("insert_epi8", _mm256_insert_epi8(ai, 0x7f, 17)); P("insert_epi64", _mm256_insert_epi64(ai, -2, 1));
  P("insertf128_ps", _mm256_insertf128_ps(a, b4, 1)); P("insertf128_pd", _mm256_insertf128_pd(d, e2, 0));
  P("insertf128_si256", _mm256_insertf128_si256(ai, _mm256_castsi256_si128(bi), 1));
  P("extractf128_ps", _mm256_extractf128_ps(a, 1)); P("extractf128_pd", _mm256_extractf128_pd(d, 0));
  P("extractf128_si256", _mm256_extractf128_si256(ai, 1));
  P("lddqu", _mm256_lddqu_si256((const __m256i *)((const char *)base_i + 3)));
  P("stream_load", _mm256_stream_load_si256((const __m256i *)base_q));
  { __m256 s; _mm256_stream_ps((float *)&s, a); P("stream_ps", s); }
  { __m256d s; _mm256_stream_pd((double *)&s, d); P("stream_pd", s); }
  { __m256i s; _mm256_stream_si256(&s, ai); P("stream_si256", s); }
  P("castps128", _mm256_extractf128_ps(_mm256_insertf128_ps(_mm256_castps128_ps256(a4), b4, 1), 1));
  P("castsi128", _mm256_extractf128_si256(_mm256_insertf128_si256(_mm256_castsi128_si256(_mm256_castsi256_si128(ai)), _mm256_castsi256_si128(bi), 1), 1));
  P("castpd128", _mm256_extractf128_pd(_mm256_insertf128_pd(_mm256_castpd128_pd256(d2), e2, 1), 1));
  { __m256i u = _mm256_undefined_si256(); (void)u; __m256 v = _mm256_undefined_ps(); (void)v; __m256d w = _mm256_undefined_pd(); (void)w; }
#define CMP(i) do { char n_[24]; sprintf(n_, "cmp_ps_%d", i); P(n_, _mm256_cmp_ps(a, b, i)); \
    sprintf(n_, "cmp_pd_%d", i); P(n_, _mm256_cmp_pd(d, e, i)); \
    sprintf(n_, "cmp_ps128_%d", i); P(n_, _mm_cmp_ps(a4, b4, i)); \
    sprintf(n_, "cmp_pd128_%d", i); P(n_, _mm_cmp_pd(d2, e2, i)); \
    sprintf(n_, "cmp_ss_%d", i); P(n_, _mm_cmp_ss(a4, b4, i)); \
    sprintf(n_, "cmp_sd_%d", i); P(n_, _mm_cmp_sd(d2, e2, i)); } while (0)
  CMP(0); CMP(1); CMP(2); CMP(3); CMP(4); CMP(5); CMP(6); CMP(7); CMP(8); CMP(9); CMP(10);
  CMP(11); CMP(12); CMP(13); CMP(14); CMP(15); CMP(16); CMP(17); CMP(18); CMP(19); CMP(20);
  CMP(21); CMP(22); CMP(23); CMP(24); CMP(25); CMP(26); CMP(27); CMP(28); CMP(29); CMP(30); CMP(31);
  {
    float out[16]; double outd[8]; int i;
    for (i = 0; i < 16; i++) out[i] = (float)i;
    for (i = 0; i < 8; i++) outd[i] = i;
    __m256i m = bi; __m128i m4 = _mm256_castsi256_si128(bi);
    P("maskload_ps", _mm256_maskload_ps(base_f + 3, m)); P("maskload_pd", _mm256_maskload_pd(base_d + 1, m));
    P("maskload_ps128", _mm_maskload_ps(base_f + 5, m4)); P("maskload_pd128", _mm_maskload_pd(base_d + 2, m4));
    _mm256_maskstore_ps(out + 1, m, a); _mm_maskstore_ps(out + 9, m4, b4); hex("maskstore_ps", out, sizeof out);
    _mm256_maskstore_pd(outd, m, d); _mm_maskstore_pd(outd + 5, m4, e2); hex("maskstore_pd", outd, sizeof outd);
  }
}

T static void avx2(const unsigned char *pa, const unsigned char *pb, const unsigned char *pc) {
  __m256i a = _mm256_loadu_si256((const __m256i *)pa), b = _mm256_loadu_si256((const __m256i *)pb);
  __m256i c = _mm256_loadu_si256((const __m256i *)pc);
  __m128i a4 = _mm256_castsi256_si128(a), b4 = _mm256_castsi256_si128(b), c4 = _mm256_castsi256_si128(c);
  P("abs_epi8", _mm256_abs_epi8(a)); P("abs_epi16", _mm256_abs_epi16(a)); P("abs_epi32", _mm256_abs_epi32(a));
  P("adds_epi8", _mm256_adds_epi8(a, b)); P("adds_epi16", _mm256_adds_epi16(a, b));
  P("adds_epu8", _mm256_adds_epu8(a, b)); P("adds_epu16", _mm256_adds_epu16(a, b));
  P("subs_epi8", _mm256_subs_epi8(a, b)); P("subs_epi16", _mm256_subs_epi16(a, b));
  P("subs_epu8", _mm256_subs_epu8(a, b)); P("subs_epu16", _mm256_subs_epu16(a, b));
  P("avg_epu8", _mm256_avg_epu8(a, b)); P("avg_epu16", _mm256_avg_epu16(a, b));
  P("blendv_epi8", _mm256_blendv_epi8(a, b, c));
  P("hadd_epi16", _mm256_hadd_epi16(a, b)); P("hadd_epi32", _mm256_hadd_epi32(a, b)); P("hadds_epi16", _mm256_hadds_epi16(a, b));
  P("hsub_epi16", _mm256_hsub_epi16(a, b)); P("hsub_epi32", _mm256_hsub_epi32(a, b)); P("hsubs_epi16", _mm256_hsubs_epi16(a, b));
  P("madd_epi16", _mm256_madd_epi16(a, b)); P("maddubs_epi16", _mm256_maddubs_epi16(a, b));
  P("max_epi8", _mm256_max_epi8(a, b)); P("max_epi16", _mm256_max_epi16(a, b)); P("max_epi32", _mm256_max_epi32(a, b));
  P("max_epu8", _mm256_max_epu8(a, b)); P("max_epu16", _mm256_max_epu16(a, b)); P("max_epu32", _mm256_max_epu32(a, b));
  P("min_epi8", _mm256_min_epi8(a, b)); P("min_epi16", _mm256_min_epi16(a, b)); P("min_epi32", _mm256_min_epi32(a, b));
  P("min_epu8", _mm256_min_epu8(a, b)); P("min_epu16", _mm256_min_epu16(a, b)); P("min_epu32", _mm256_min_epu32(a, b));
  P("movemask_epi8", _mm256_movemask_epi8(a));
  P("mul_epi32", _mm256_mul_epi32(a, b)); P("mul_epu32", _mm256_mul_epu32(a, b));
  P("mulhi_epi16", _mm256_mulhi_epi16(a, b)); P("mulhi_epu16", _mm256_mulhi_epu16(a, b)); P("mulhrs_epi16", _mm256_mulhrs_epi16(a, b));
  P("packs_epi16", _mm256_packs_epi16(a, b)); P("packs_epi32", _mm256_packs_epi32(a, b));
  P("packus_epi16", _mm256_packus_epi16(a, b)); P("packus_epi32", _mm256_packus_epi32(a, b));
  P("permutevar8x32_epi32", _mm256_permutevar8x32_epi32(a, b));
  P("permutevar8x32_ps", _mm256_permutevar8x32_ps(_mm256_castsi256_ps(a), b));
  P("sad_epu8", _mm256_sad_epu8(a, b)); P("shuffle_epi8", _mm256_shuffle_epi8(a, b));
  P("sign_epi8", _mm256_sign_epi8(a, b)); P("sign_epi16", _mm256_sign_epi16(a, b)); P("sign_epi32", _mm256_sign_epi32(a, b));
  P("sll_epi16", _mm256_sll_epi16(a, c4)); P("sll_epi32", _mm256_sll_epi32(a, c4)); P("sll_epi64", _mm256_sll_epi64(a, c4));
  P("srl_epi16", _mm256_srl_epi16(a, c4)); P("srl_epi32", _mm256_srl_epi32(a, c4)); P("srl_epi64", _mm256_srl_epi64(a, c4));
  P("sra_epi16", _mm256_sra_epi16(a, c4)); P("sra_epi32", _mm256_sra_epi32(a, c4));
  {
    static const int counts[] = {0, 1, 7, 15, 16, 17, 31, 32, 33, 63, 64, 65, 200, -1};
    for (unsigned k = 0; k < sizeof counts / sizeof *counts; k++) {
      int n = counts[k];
      char name[32];
      sprintf(name, "count_%d", n); hex(name, &n, 0);
      P("slli_epi16", _mm256_slli_epi16(a, n)); P("slli_epi32", _mm256_slli_epi32(a, n)); P("slli_epi64", _mm256_slli_epi64(a, n));
      P("srli_epi16", _mm256_srli_epi16(a, n)); P("srli_epi32", _mm256_srli_epi32(a, n)); P("srli_epi64", _mm256_srli_epi64(a, n));
      P("srai_epi16", _mm256_srai_epi16(a, n)); P("srai_epi32", _mm256_srai_epi32(a, n));
      P("sll_n", _mm256_sll_epi32(a, _mm_cvtsi32_si128(n)));
    }
  }
  P("sllv_epi32", _mm256_sllv_epi32(a, c)); P("sllv_epi64", _mm256_sllv_epi64(a, c));
  P("srlv_epi32", _mm256_srlv_epi32(a, c)); P("srlv_epi64", _mm256_srlv_epi64(a, c)); P("srav_epi32", _mm256_srav_epi32(a, c));
  P("sllv_epi32_128", _mm_sllv_epi32(a4, c4)); P("sllv_epi64_128", _mm_sllv_epi64(a4, c4));
  P("srlv_epi32_128", _mm_srlv_epi32(a4, c4)); P("srlv_epi64_128", _mm_srlv_epi64(a4, c4)); P("srav_epi32_128", _mm_srav_epi32(a4, c4));
  P("mpsadbw", _mm256_mpsadbw_epu8(a, b, 0x2d)); P("mpsadbw2", _mm256_mpsadbw_epu8(a, b, 0x12));
  P("alignr_0", _mm256_alignr_epi8(a, b, 0)); P("alignr_5", _mm256_alignr_epi8(a, b, 5));
  P("alignr_16", _mm256_alignr_epi8(a, b, 16)); P("alignr_21", _mm256_alignr_epi8(a, b, 21)); P("alignr_40", _mm256_alignr_epi8(a, b, 40));
  P("blend_epi16", _mm256_blend_epi16(a, b, 0x96)); P("blend_epi32", _mm256_blend_epi32(a, b, 0x3c));
  P("blend_epi32_128", _mm_blend_epi32(a4, b4, 0x9));
  P("shuffle_epi32", _mm256_shuffle_epi32(a, 0x1b)); P("shufflehi_epi16", _mm256_shufflehi_epi16(a, 0x39));
  P("shufflelo_epi16", _mm256_shufflelo_epi16(a, 0x8d));
  P("slli_si256", _mm256_slli_si256(a, 3)); P("bslli_epi128", _mm256_bslli_epi128(a, 15)); P("slli_si256_17", _mm256_slli_si256(a, 17));
  P("srli_si256", _mm256_srli_si256(a, 5)); P("bsrli_epi128", _mm256_bsrli_epi128(a, 1)); P("srli_si256_16", _mm256_srli_si256(a, 16));
  P("broadcastsi128", _mm_broadcastsi128_si256(a4));
  P("permute4x64_pd", _mm256_permute4x64_pd(_mm256_castsi256_pd(a), 0x72)); P("permute4x64_epi64", _mm256_permute4x64_epi64(a, 0xd8));
  P("permute2x128", _mm256_permute2x128_si256(a, b, 0x13)); P("permute2x128_z", _mm256_permute2x128_si256(a, b, 0x80));
  P("extracti128", _mm256_extracti128_si256(a, 1)); P("inserti128", _mm256_inserti128_si256(a, b4, 0));
  {
    __m256i idx = _mm256_and_si256(c, _mm256_set1_epi32(31));
    __m128i idx4 = _mm256_castsi256_si128(idx);
    __m256i idx64 = _mm256_and_si256(c, _mm256_set1_epi64x(31));
    __m128i idx64_4 = _mm256_castsi256_si128(idx64);
    P("i32gather_epi32", _mm256_i32gather_epi32(base_i, idx, 4)); P("i32gather_epi32_128", _mm_i32gather_epi32(base_i, idx4, 2));
    P("i32gather_epi64", _mm256_i32gather_epi64(base_q, idx4, 8)); P("i32gather_epi64_128", _mm_i32gather_epi64(base_q, idx4, 4));
    P("i64gather_epi32", _mm256_i64gather_epi32(base_i, idx64, 4)); P("i64gather_epi32_128", _mm_i64gather_epi32(base_i, idx64_4, 1));
    P("i64gather_epi64", _mm256_i64gather_epi64(base_q, idx64, 8)); P("i64gather_epi64_128", _mm_i64gather_epi64(base_q, idx64_4, 8));
    P("i32gather_ps", _mm256_i32gather_ps(base_f, idx, 4)); P("i32gather_ps_128", _mm_i32gather_ps(base_f, idx4, 4));
    P("i32gather_pd", _mm256_i32gather_pd(base_d, idx4, 8)); P("i32gather_pd_128", _mm_i32gather_pd(base_d, idx4, 8));
    P("i64gather_ps", _mm256_i64gather_ps(base_f, idx64, 4)); P("i64gather_ps_128", _mm_i64gather_ps(base_f, idx64_4, 2));
    P("i64gather_pd", _mm256_i64gather_pd(base_d, idx64, 8)); P("i64gather_pd_128", _mm_i64gather_pd(base_d, idx64_4, 8));
    P("mask_i32gather_epi32", _mm256_mask_i32gather_epi32(a, base_i, idx, b, 4));
    P("mask_i32gather_epi32_128", _mm_mask_i32gather_epi32(a4, base_i, idx4, b4, 4));
    P("mask_i32gather_epi64", _mm256_mask_i32gather_epi64(a, base_q, idx4, b, 8));
    P("mask_i32gather_epi64_128", _mm_mask_i32gather_epi64(a4, base_q, idx4, b4, 8));
    P("mask_i64gather_epi32", _mm256_mask_i64gather_epi32(a4, base_i, idx64, b4, 4));
    P("mask_i64gather_epi32_128", _mm_mask_i64gather_epi32(a4, base_i, idx64_4, b4, 4));
    P("mask_i64gather_epi64", _mm256_mask_i64gather_epi64(a, base_q, idx64, b, 8));
    P("mask_i64gather_epi64_128", _mm_mask_i64gather_epi64(a4, base_q, idx64_4, b4, 8));
    P("mask_i32gather_ps", _mm256_mask_i32gather_ps(_mm256_castsi256_ps(a), base_f, idx, _mm256_castsi256_ps(b), 4));
    P("mask_i32gather_ps_128", _mm_mask_i32gather_ps(_mm_castsi128_ps(a4), base_f, idx4, _mm_castsi128_ps(b4), 4));
    P("mask_i32gather_pd", _mm256_mask_i32gather_pd(_mm256_castsi256_pd(a), base_d, idx4, _mm256_castsi256_pd(b), 8));
    P("mask_i32gather_pd_128", _mm_mask_i32gather_pd(_mm_castsi128_pd(a4), base_d, idx4, _mm_castsi128_pd(b4), 8));
    P("mask_i64gather_ps", _mm256_mask_i64gather_ps(_mm_castsi128_ps(a4), base_f, idx64, _mm_castsi128_ps(b4), 4));
    P("mask_i64gather_ps_128", _mm_mask_i64gather_ps(_mm_castsi128_ps(a4), base_f, idx64_4, _mm_castsi128_ps(b4), 4));
    P("mask_i64gather_pd", _mm256_mask_i64gather_pd(_mm256_castsi256_pd(a), base_d, idx64, _mm256_castsi256_pd(b), 8));
    P("mask_i64gather_pd_128", _mm_mask_i64gather_pd(_mm_castsi128_pd(a4), base_d, idx64_4, _mm_castsi128_pd(b4), 8));
  }
  {
    int out[16]; long long outq[8]; int i;
    for (i = 0; i < 16; i++) out[i] = -i;
    for (i = 0; i < 8; i++) outq[i] = i * 3;
    P("maskload_epi32", _mm256_maskload_epi32(base_i + 7, b)); P("maskload_epi64", _mm256_maskload_epi64(base_q + 3, b));
    P("maskload_epi32_128", _mm_maskload_epi32(base_i + 1, b4)); P("maskload_epi64_128", _mm_maskload_epi64(base_q + 9, b4));
    _mm256_maskstore_epi32(out, b, a); _mm_maskstore_epi32(out + 11, c4, a4); hex("maskstore_epi32", out, sizeof out);
    _mm256_maskstore_epi64(outq + 1, b, a); _mm_maskstore_epi64(outq + 5, c4, b4); hex("maskstore_epi64", outq, sizeof outq);
  }
}

#ifndef NO_NAMED
T static void named(const unsigned char (*in)[32]) {
  __m256 a = _mm256_loadu_ps((const float *)in[1]), b = _mm256_loadu_ps((const float *)in[4]);
  __m256d d = _mm256_loadu_pd((const double *)in[2]);
  __m256i x = _mm256_loadu_si256((const __m256i *)in[0]), y = _mm256_loadu_si256((const __m256i *)in[3]);
  __m128i x4 = _mm256_castsi256_si128(x), y4 = _mm256_castsi256_si128(y);
  __m128 a4 = _mm256_castps256_ps128(a), b4 = _mm256_castps256_ps128(b);
  __m128d d2 = _mm256_castpd256_pd128(d), e2 = _mm256_extractf128_pd(d, 1);
  P("aesenc", _mm_aesenc_si128(x4, y4)); P("aesenclast", _mm_aesenclast_si128(x4, y4));
  P("aesdec", _mm_aesdec_si128(x4, y4)); P("aesdeclast", _mm_aesdeclast_si128(x4, y4)); P("aesimc", _mm_aesimc_si128(x4));
  P("aeskeygen_00", _mm_aeskeygenassist_si128(x4, 0x00)); P("aeskeygen_01", _mm_aeskeygenassist_si128(x4, 0x01));
  P("aeskeygen_1b", _mm_aeskeygenassist_si128(x4, 0x1b)); P("aeskeygen_80", _mm_aeskeygenassist_si128(x4, 0x80));
  P("clmul_00", _mm_clmulepi64_si128(x4, y4, 0x00)); P("clmul_01", _mm_clmulepi64_si128(x4, y4, 0x01));
  P("clmul_10", _mm_clmulepi64_si128(x4, y4, 0x10)); P("clmul_11", _mm_clmulepi64_si128(x4, y4, 0x11));
  P("round_ps_near", _mm256_round_ps(a, _MM_FROUND_TO_NEAREST_INT | _MM_FROUND_NO_EXC));
  P("round_ps_down", _mm256_round_ps(a, _MM_FROUND_TO_NEG_INF)); P("round_ps_up", _mm256_round_ps(a, _MM_FROUND_TO_POS_INF));
  P("round_ps_zero", _mm256_round_ps(a, _MM_FROUND_TO_ZERO)); P("round_ps_cur", _mm256_round_ps(a, _MM_FROUND_CUR_DIRECTION));
  P("round_pd_near", _mm256_round_pd(d, _MM_FROUND_TO_NEAREST_INT)); P("round_pd_zero", _mm256_round_pd(d, _MM_FROUND_TO_ZERO));
  P("floor_ps", _mm256_floor_ps(a)); P("ceil_ps", _mm256_ceil_ps(a)); P("floor_pd", _mm256_floor_pd(d)); P("ceil_pd", _mm256_ceil_pd(d));
  P("cvtps_epi32", _mm256_cvtps_epi32(a)); P("cvttps_epi32", _mm256_cvttps_epi32(a));
  P("cvtpd_epi32", _mm256_cvtpd_epi32(d)); P("cvttpd_epi32", _mm256_cvttpd_epi32(d));
  P("sqrt_ps", _mm256_sqrt_ps(a)); P("sqrt_pd", _mm256_sqrt_pd(d));
  (void)a4; (void)b4; (void)d2; (void)e2;
  _mm256_zeroupper();
  P("after_zeroupper", x4);
#ifndef NO_APPROX
  P("rcp_ps", _mm256_rcp_ps(a)); P("rsqrt_ps", _mm256_rsqrt_ps(b));
#endif
}
#endif

int main(void) {
#ifndef ALWAYS_FAST
  __builtin_cpu_init();
  if (!__builtin_cpu_supports("avx2") || !__builtin_cpu_supports("aes") ||
      !__builtin_cpu_supports("pclmul") || !__builtin_cpu_supports("sse4.2")) {
    printf("skip\n");
    return 0;
  }
#endif
  for (int i = 0; i < 64; i++) {
    base_i[i] = i * 7 - 100;
    base_q[i] = (long long)i * 0x123456789ll - 5;
    base_f[i] = (float)i / 3.0f - 4.0f;
    base_d[i] = i * 1.25 - 30.0;
  }
  for (int round = 0; round < 6; round++) {
    unsigned char in[6][32];
    for (int k = 0; k < 6; k++) generate((round + k) % 6, in[k]);
    printf("round %d\n", round);
    avx((const unsigned char (*)[32])in, round);
    avx2(in[0], in[3], in[4]);
    avx2(in[3], in[0], in[5][0] & 1 ? in[0] : in[4]);
#ifndef NO_NAMED
    named((const unsigned char (*)[32])in);
#endif
  }
  return 0;
}
