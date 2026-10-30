/* SSE and SSE2 intrinsics from bcc's own <emmintrin.h>, compared with the host GCC's
 * build, which runs the instructions: every arithmetic, comparison, conversion,
 * shuffle, load, store and MMX extension, with NaNs, infinities, signed zeros and
 * out-of-range conversions. The reciprocal approximations are only checked to be
 * within their documented bound, since processors differ. */
#include <emmintrin.h>
#include <stdio.h>
#include <string.h>
#include <math.h>

static void hex(const char *name, const void *p, int n) {
  const unsigned char *b = p;
  printf("%s:", name);
  for (int i = n - 1; i >= 0; i--) printf("%02x", b[i]);
  printf("\n");
}
#define P(name, expr) do { __typeof__(expr) v_ = (expr); hex(name, &v_, sizeof v_); } while (0)
#define PI(name, expr) printf("%s: %lld\n", name, (long long)(expr))

static float fa[8] = {1.5f, -2.25f, 3.0e9f, -0.0f, 0.5f, 2.5f, -3.5f, 1e-40f};
static float fb[8] = {2.0f, -2.25f, -1.0e10f, 0.0f, 0.75f, -1.0f, 7.0f, 3.0f};
static double da[4] = {1.5, -2.5e300, 1.0, -7.25};
static double db[4] = {2.5, 3.0, -0.0, -7.25};
static int ia[8] = {1, -2, 0x7fffffff, (int)0x80000000u, 12345, -6789, 255, -256};
static int ib[8] = {3, 4, 1, -1, 0x10000, 7, -255, 1000};

int main(void) {
  float nanf_ = 0.0f / (fa[3] == 0.0f ? 0.0f : 1.0f);
  fa[6] = nanf_ != nanf_ ? fa[6] : 0;
  __m128 a = _mm_loadu_ps(fa), b = _mm_loadu_ps(fb), a2 = _mm_loadu_ps(fa + 4), b2 = _mm_loadu_ps(fb + 4);
  __m128 n = _mm_set_ps(1.0f, __builtin_nanf(""), -1.0f, __builtin_inff());
  __m128d da0 = _mm_loadu_pd(da), db0 = _mm_loadu_pd(db), da1 = _mm_loadu_pd(da + 2), db1 = _mm_loadu_pd(db + 2);
  __m128d dn = _mm_set_pd(__builtin_nan(""), 1.0);
  __m128i x = _mm_loadu_si128((const __m128i *)ia), y = _mm_loadu_si128((const __m128i *)ib);
  __m128i x2 = _mm_loadu_si128((const __m128i *)(ia + 4)), y2 = _mm_loadu_si128((const __m128i *)(ib + 4));
  /* SSE arithmetic */
  P("add_ps", _mm_add_ps(a, b)); P("add_ss", _mm_add_ss(a, b)); P("sub_ps", _mm_sub_ps(a, b));
  P("sub_ss", _mm_sub_ss(a2, b2)); P("mul_ps", _mm_mul_ps(a, b)); P("mul_ss", _mm_mul_ss(a, b));
  P("div_ps", _mm_div_ps(a, b)); P("div_ss", _mm_div_ss(a2, b2)); P("sqrt_ps", _mm_sqrt_ps(a2));
  P("sqrt_ss", _mm_sqrt_ss(a2)); P("min_ps", _mm_min_ps(a, b)); P("max_ps", _mm_max_ps(a, b));
  P("min_ps_nan", _mm_min_ps(n, a)); P("max_ps_nan", _mm_max_ps(a, n)); P("min_ss", _mm_min_ss(a2, b2));
  P("max_ss", _mm_max_ss(a, b));
  P("and_ps", _mm_and_ps(a, b)); P("andnot_ps", _mm_andnot_ps(a, b)); P("or_ps", _mm_or_ps(a, b)); P("xor_ps", _mm_xor_ps(a, b));
  /* comparisons */
  P("cmpeq_ps", _mm_cmpeq_ps(a, b)); P("cmplt_ps", _mm_cmplt_ps(a, n)); P("cmple_ps", _mm_cmple_ps(a, b));
  P("cmpgt_ps", _mm_cmpgt_ps(a, b)); P("cmpge_ps", _mm_cmpge_ps(n, b)); P("cmpneq_ps", _mm_cmpneq_ps(n, n));
  P("cmpnlt_ps", _mm_cmpnlt_ps(a, n)); P("cmpnle_ps", _mm_cmpnle_ps(a, b)); P("cmpngt_ps", _mm_cmpngt_ps(n, b));
  P("cmpnge_ps", _mm_cmpnge_ps(a, b)); P("cmpord_ps", _mm_cmpord_ps(a, n)); P("cmpunord_ps", _mm_cmpunord_ps(n, b));
  P("cmpeq_ss", _mm_cmpeq_ss(a, b)); P("cmplt_ss", _mm_cmplt_ss(a, b)); P("cmpunord_ss", _mm_cmpunord_ss(n, b));
  P("cmpnge_ss", _mm_cmpnge_ss(a2, b2));
  PI("comieq", _mm_comieq_ss(a, b)); PI("comilt", _mm_comilt_ss(a, b)); PI("comile", _mm_comile_ss(n, a));
  PI("comigt", _mm_comigt_ss(a2, b2)); PI("comige", _mm_comige_ss(a, a)); PI("comineq", _mm_comineq_ss(a, b));
  PI("ucomieq", _mm_ucomieq_ss(b, b)); PI("ucomilt_nan", _mm_ucomilt_ss(_mm_shuffle_ps(n, n, 2), a));
  PI("comineq_nan", _mm_comineq_ss(_mm_shuffle_ps(n, n, 2), a)); PI("comieq_nan", _mm_comieq_ss(_mm_shuffle_ps(n, n, 2), a));
  /* conversions */
  PI("cvtss_si32", _mm_cvtss_si32(_mm_set_ss(2.5f))); PI("cvtss_si32b", _mm_cvtss_si32(_mm_set_ss(-3.5f)));
  PI("cvtss_si32c", _mm_cvtss_si32(_mm_set_ss(3e9f))); PI("cvttss_si32", _mm_cvttss_si32(_mm_set_ss(-2.75f)));
  PI("cvttss_si32n", _mm_cvttss_si32(n)); PI("cvtss_si64", _mm_cvtss_si64(_mm_set_ss(-1e19f)));
  PI("cvttss_si64", _mm_cvttss_si64(_mm_set_ss(1e18f)));
  P("cvtsi32_ss", _mm_cvtsi32_ss(a, -7)); P("cvtsi64_ss", _mm_cvtsi64_ss(a, 123456789012LL));
  printf("cvtss_f32: %a\n", _mm_cvtss_f32(a));
  /* moves and shuffles */
  P("shuffle_ps", _mm_shuffle_ps(a, b, _MM_SHUFFLE(3, 0, 2, 1))); P("shuffle_ps2", _mm_shuffle_ps(a, b, 0x1b));
  P("unpackhi_ps", _mm_unpackhi_ps(a, b)); P("unpacklo_ps", _mm_unpacklo_ps(a, b));
  P("movehl_ps", _mm_movehl_ps(a, b)); P("movelh_ps", _mm_movelh_ps(a, b)); P("move_ss", _mm_move_ss(a, b));
  PI("movemask_ps", _mm_movemask_ps(a)); PI("movemask_ps2", _mm_movemask_ps(b));
  P("set_ps", _mm_set_ps(1, 2, 3, 4)); P("setr_ps", _mm_setr_ps(1, 2, 3, 4)); P("set1_ps", _mm_set1_ps(-0.5f));
  P("set_ss", _mm_set_ss(9)); P("setzero_ps", _mm_setzero_ps()); P("load_ss", _mm_load_ss(fa + 1));
  P("load1_ps", _mm_load1_ps(fb + 2));
  { float out[8] = {0}; _mm_storeu_ps(out + 1, a); _mm_store_ss(out + 6, b); P("storeu_ps", *(__m128_u *)(out + 1)); P("store_ss", out[6]);
    __attribute__((aligned(16))) float al[4]; _mm_store_ps(al, b); P("store_ps", al[2]); _mm_storer_ps(al, a); P("storer_ps", _mm_load_ps(al));
    __m64 h; _mm_storeh_pi(&h, a); P("storeh_pi", h); _mm_storel_pi(&h, b); P("storel_pi", h);
    P("loadh_pi", _mm_loadh_pi(a, &h)); P("loadl_pi", _mm_loadl_pi(b, &h)); }
  { __m128 r0 = a, r1 = b, r2 = a2, r3 = b2; _MM_TRANSPOSE4_PS(r0, r1, r2, r3); P("transpose0", r0); P("transpose3", r3); }
  /* SSE2 double */
  P("add_pd", _mm_add_pd(da0, db0)); P("sub_sd", _mm_sub_sd(da1, db1)); P("mul_pd", _mm_mul_pd(da0, db0));
  P("div_pd", _mm_div_pd(da1, db1)); P("sqrt_pd", _mm_sqrt_pd(db0)); P("sqrt_sd", _mm_sqrt_sd(da0, db0));
  P("min_pd", _mm_min_pd(da1, db1)); P("max_pd", _mm_max_pd(dn, da0)); P("min_sd", _mm_min_sd(da0, db0));
  P("and_pd", _mm_and_pd(da0, db0)); P("xor_pd", _mm_xor_pd(da1, db1)); P("andnot_pd", _mm_andnot_pd(da0, db1));
  P("cmpeq_pd", _mm_cmpeq_pd(da1, db1)); P("cmplt_pd", _mm_cmplt_pd(dn, da0)); P("cmpnlt_pd", _mm_cmpnlt_pd(dn, da0));
  P("cmpunord_pd", _mm_cmpunord_pd(dn, da0)); P("cmpord_sd", _mm_cmpord_sd(dn, da0)); P("cmpge_sd", _mm_cmpge_sd(da1, db1));
  PI("comieq_sd", _mm_comieq_sd(da1, db1)); PI("comilt_sd", _mm_comilt_sd(da0, db0)); PI("ucomineq_sd", _mm_ucomineq_sd(dn, da0));
  P("cvtpd_ps", _mm_cvtpd_ps(da0)); P("cvtps_pd", _mm_cvtps_pd(a)); P("cvtepi32_pd", _mm_cvtepi32_pd(x));
  P("cvtpd_epi32", _mm_cvtpd_epi32(_mm_set_pd(-2.5, 3.5))); P("cvttpd_epi32", _mm_cvttpd_epi32(_mm_set_pd(-2.5, 3e10)));
  PI("cvtsd_si32", _mm_cvtsd_si32(_mm_set_sd(0.5))); PI("cvttsd_si32", _mm_cvttsd_si32(_mm_set_sd(-1.9)));
  PI("cvtsd_si64", _mm_cvtsd_si64(_mm_set_sd(-1e20))); PI("cvttsd_si64", _mm_cvttsd_si64(_mm_set_sd(1.5e15)));
  P("cvtsd_ss", _mm_cvtsd_ss(a, da0)); P("cvtsi32_sd", _mm_cvtsi32_sd(da0, -9)); P("cvtsi64_sd", _mm_cvtsi64_sd(da0, -1LL << 60));
  P("cvtss_sd", _mm_cvtss_sd(da0, b)); printf("cvtsd_f64: %a\n", _mm_cvtsd_f64(da1));
  P("cvtepi32_ps", _mm_cvtepi32_ps(x)); P("cvtps_epi32", _mm_cvtps_epi32(a)); P("cvttps_epi32", _mm_cvttps_epi32(a2));
  P("cvtps_epi32n", _mm_cvtps_epi32(n));
  P("shuffle_pd", _mm_shuffle_pd(da0, db0, 1)); P("shuffle_pd2", _mm_shuffle_pd(da0, db0, 2));
  P("unpackhi_pd", _mm_unpackhi_pd(da0, db0)); P("unpacklo_pd", _mm_unpacklo_pd(da0, db0));
  PI("movemask_pd", _mm_movemask_pd(da0)); P("set_pd", _mm_set_pd(1, 2)); P("setr_pd", _mm_setr_pd(1, 2));
  P("load1_pd", _mm_load1_pd(db + 1)); P("loadr_pd", _mm_loadr_pd(da)); P("load_sd", _mm_load_sd(db + 3));
  P("loadh_pd", _mm_loadh_pd(da0, db + 2)); P("loadl_pd", _mm_loadl_pd(da0, db + 2)); P("move_sd", _mm_move_sd(da0, db0));
  { double out[2]; _mm_storeh_pd(out, da0); _mm_storel_pd(out + 1, db0); P("storehl_pd", *(__m128d_u *)out); }
  /* SSE2 integer */
  P("add_epi8", _mm_add_epi8(x, y)); P("add_epi16", _mm_add_epi16(x, y)); P("add_epi32", _mm_add_epi32(x, y)); P("add_epi64", _mm_add_epi64(x, y));
  P("sub_epi8", _mm_sub_epi8(x, y)); P("sub_epi16", _mm_sub_epi16(x2, y2)); P("sub_epi32", _mm_sub_epi32(x, y)); P("sub_epi64", _mm_sub_epi64(x, y));
  P("adds_epi8", _mm_adds_epi8(x, y)); P("adds_epi16", _mm_adds_epi16(x, x)); P("adds_epu8", _mm_adds_epu8(x, y)); P("adds_epu16", _mm_adds_epu16(x, y));
  P("subs_epi8", _mm_subs_epi8(x2, y)); P("subs_epi16", _mm_subs_epi16(x, y)); P("subs_epu8", _mm_subs_epu8(x, y)); P("subs_epu16", _mm_subs_epu16(y, x));
  P("avg_epu8", _mm_avg_epu8(x, y)); P("avg_epu16", _mm_avg_epu16(x, y)); P("madd_epi16", _mm_madd_epi16(x, y));
  P("madd_epi16_min", _mm_madd_epi16(_mm_set1_epi16(-32768), _mm_set1_epi16(-32768)));
  P("max_epi16", _mm_max_epi16(x, y)); P("min_epi16", _mm_min_epi16(x, y)); P("max_epu8", _mm_max_epu8(x, y)); P("min_epu8", _mm_min_epu8(x, y));
  P("mulhi_epi16", _mm_mulhi_epi16(x, y)); P("mulhi_epu16", _mm_mulhi_epu16(x, y)); P("mullo_epi16", _mm_mullo_epi16(x, y));
  P("mul_epu32", _mm_mul_epu32(x, y)); P("sad_epu8", _mm_sad_epu8(x, y));
  P("and_si128", _mm_and_si128(x, y)); P("andnot_si128", _mm_andnot_si128(x, y)); P("or_si128", _mm_or_si128(x, y)); P("xor_si128", _mm_xor_si128(x, y));
  P("slli_epi16", _mm_slli_epi16(x, 3)); P("slli_epi16_big", _mm_slli_epi16(x, 16)); P("slli_epi32", _mm_slli_epi32(x, 31)); P("slli_epi64", _mm_slli_epi64(x, 63));
  P("srli_epi16", _mm_srli_epi16(x, 1)); P("srli_epi32", _mm_srli_epi32(x, 32)); P("srli_epi64", _mm_srli_epi64(x, 4));
  P("srai_epi16", _mm_srai_epi16(x, 15)); P("srai_epi16_big", _mm_srai_epi16(x, 99)); P("srai_epi32", _mm_srai_epi32(x, 5));
  P("sll_epi16", _mm_sll_epi16(x, _mm_cvtsi32_si128(2))); P("sll_epi32", _mm_sll_epi32(x, _mm_set_epi32(0, 0, 1, 1)));
  P("srl_epi64", _mm_srl_epi64(x, _mm_cvtsi32_si128(7))); P("sra_epi16", _mm_sra_epi16(x, _mm_cvtsi32_si128(200)));
  P("sra_epi32", _mm_sra_epi32(x, _mm_cvtsi32_si128(3)));
  P("slli_si128", _mm_slli_si128(x, 3)); P("srli_si128", _mm_srli_si128(x, 5)); P("slli_si128_16", _mm_slli_si128(x, 16));
  P("bslli_si128", _mm_bslli_si128(x, 15)); P("bsrli_si128", _mm_bsrli_si128(x, 1));
  P("cmpeq_epi8", _mm_cmpeq_epi8(x, x2)); P("cmpeq_epi16", _mm_cmpeq_epi16(x, y)); P("cmpeq_epi32", _mm_cmpeq_epi32(x, x));
  P("cmpgt_epi8", _mm_cmpgt_epi8(x, y)); P("cmpgt_epi16", _mm_cmpgt_epi16(x, y)); P("cmpgt_epi32", _mm_cmpgt_epi32(x, y));
  P("cmplt_epi8", _mm_cmplt_epi8(x, y)); P("cmplt_epi16", _mm_cmplt_epi16(x, y)); P("cmplt_epi32", _mm_cmplt_epi32(x, y));
  P("set_epi64x", _mm_set_epi64x(1, -2)); P("set_epi32", _mm_set_epi32(1, 2, 3, 4)); P("set_epi16", _mm_set_epi16(1, 2, 3, 4, 5, 6, 7, -8));
  P("set_epi8", _mm_set_epi8(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, -16));
  P("set1_epi64x", _mm_set1_epi64x(-3)); P("set1_epi32", _mm_set1_epi32(7)); P("set1_epi16", _mm_set1_epi16(-2)); P("set1_epi8", _mm_set1_epi8(0x55));
  P("setr_epi32", _mm_setr_epi32(1, 2, 3, 4)); P("setr_epi16", _mm_setr_epi16(1, 2, 3, 4, 5, 6, 7, 8));
  P("setr_epi8", _mm_setr_epi8(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16)); P("setzero_si128", _mm_setzero_si128());
  P("loadl_epi64", _mm_loadl_epi64((const __m128i *)ib)); P("loadu_si32", _mm_loadu_si32(ia + 3)); P("loadu_si16", _mm_loadu_si16(ia + 2));
  { int out[6] = {0}; _mm_storel_epi64((__m128i *)(out + 1), x); _mm_storeu_si32(out + 4, y); P("storel_epi64", *(__m128i_u *)(out + 1));
    char buf[16] = {0}; _mm_maskmoveu_si128(x, y, buf); P("maskmoveu", *(__m128i_u *)buf); }
  P("move_epi64", _mm_move_epi64(x)); PI("cvtsi128_si32", _mm_cvtsi128_si32(x)); PI("cvtsi128_si64", _mm_cvtsi128_si64(y));
  P("cvtsi32_si128", _mm_cvtsi32_si128(-5)); P("cvtsi64_si128", _mm_cvtsi64_si128(-5));
  P("packs_epi16", _mm_packs_epi16(x, y)); P("packs_epi32", _mm_packs_epi32(x, y)); P("packus_epi16", _mm_packus_epi16(x, y2));
  PI("extract_epi16", _mm_extract_epi16(x, 3)); P("insert_epi16", _mm_insert_epi16(x, -1, 6)); PI("movemask_epi8", _mm_movemask_epi8(x));
  P("shuffle_epi32", _mm_shuffle_epi32(x, 0x1b)); P("shufflelo_epi16", _mm_shufflelo_epi16(x, 0x93)); P("shufflehi_epi16", _mm_shufflehi_epi16(x, 0x4e));
  P("unpackhi_epi8", _mm_unpackhi_epi8(x, y)); P("unpackhi_epi16", _mm_unpackhi_epi16(x, y)); P("unpackhi_epi32", _mm_unpackhi_epi32(x, y)); P("unpackhi_epi64", _mm_unpackhi_epi64(x, y));
  P("unpacklo_epi8", _mm_unpacklo_epi8(x, y)); P("unpacklo_epi16", _mm_unpacklo_epi16(x, y)); P("unpacklo_epi32", _mm_unpacklo_epi32(x, y)); P("unpacklo_epi64", _mm_unpacklo_epi64(x, y));
  P("castps_si128", _mm_castps_si128(a)); P("castsi128_pd", _mm_castsi128_pd(x)); P("castpd_ps", _mm_castpd_ps(da0));
  /* MMX extensions */
  { __m64 m = _mm_set_pi16(-3, 300, 7, -32768), k = _mm_set_pi16(4, -200, 7, 5);
    P("max_pi16", _mm_max_pi16(m, k)); P("min_pu8", _mm_min_pu8(m, k)); P("avg_pu8", _mm_avg_pu8(m, k)); P("sad_pu8", _mm_sad_pu8(m, k));
    P("mulhi_pu16", _mm_mulhi_pu16(m, k)); PI("movemask_pi8", _mm_movemask_pi8(m)); P("shuffle_pi16", _mm_shuffle_pi16(m, 0x1b));
    PI("extract_pi16", _mm_extract_pi16(m, 2)); P("insert_pi16", _mm_insert_pi16(m, 9, 1));
    P("cvtpi16_ps", _mm_cvtpi16_ps(m)); P("cvtpu8_ps", _mm_cvtpu8_ps(m)); P("cvtps_pi16", _mm_cvtps_pi16(b)); P("cvtps_pi8", _mm_cvtps_pi8(a));
    P("cvtpi32_ps", _mm_cvtpi32_ps(a, k)); P("cvtps_pi32", _mm_cvtps_pi32(b)); P("cvtpi32x2_ps", _mm_cvtpi32x2_ps(m, k));
    P("cvtpd_pi32", _mm_cvtpd_pi32(da0)); P("cvtpi32_pd", _mm_cvtpi32_pd(k)); P("mul_su32", _mm_mul_su32(m, k));
    P("add_si64", _mm_add_si64(m, k)); P("movpi64_epi64", _mm_movpi64_epi64(m)); P("movepi64_pi64", _mm_movepi64_pi64(x));
    _mm_empty(); }
  /* the reciprocal approximations are only within 1.5 * 2^-12 */
  { __m128 r = _mm_rcp_ps(b), s = _mm_rsqrt_ps(_mm_set_ps(4, 2, 0.25f, 9)); int ok = 1;
    for (int i = 0; i < 4; i++) ok &= fabsf(r[i] * b[i] - 1) < 0x1.8p-12f;
    ok &= fabsf(s[0] - 1.0f/3) < 0x1p-12f && fabsf(s[3] - 0.5f) < 0x1p-12f; PI("rcp_rsqrt_close", ok); }
  return 0;
}
