/* SSE3, SSSE3, SSE4.1 and SSE4.2 intrinsics from bcc's own <smmintrin.h>,
 * compared with the host GCC's -msse4.2 build: horizontal operations, byte
 * shuffles, blends, rounding modes, dot products, insertions and extractions,
 * tests, extensions, MPSADBW, PHMINPOSUW, the string comparisons in every
 * aggregation and polarity, and CRC32. */
#include <smmintrin.h>
#include <stdio.h>

static void hex(const char *name, const void *p, int n) {
  const unsigned char *b = p;
  printf("%s:", name);
  for (int i = n - 1; i >= 0; i--) printf("%02x", b[i]);
  printf("\n");
}
#define P(name, expr) do { __typeof__(expr) v_ = (expr); hex(name, &v_, sizeof v_); } while (0)
#define PI(name, expr) printf("%s: %lld\n", name, (long long)(expr))

static float fa[4] = {1.5f, -2.5f, 2.5f, -0.5f}, fb[4] = {0.25f, 3.0f, -7.75f, 1e30f};
static double da[2] = {-1.5, 2.5}, db[2] = {3.25, -0.0};
static unsigned char bytes[48] = {200, 3, 255, 0, 17, 128, 64, 1, 9, 250, 33, 34, 35, 7, 0, 99,
                                  5, 200, 1, 2, 3, 127, 128, 129, 130, 10, 20, 30, 40, 50, 60, 70,
                                  'h', 'e', 'l', 'l', 'o', ' ', 'w', 'o', 'r', 'l', 'd', 0, 'x', 'y', 'z', 0};

int main(void) {
  __m128 a = _mm_loadu_ps(fa), b = _mm_loadu_ps(fb);
  __m128d d = _mm_loadu_pd(da), e = _mm_loadu_pd(db);
  __m128i x = _mm_loadu_si128((const __m128i *)bytes), y = _mm_loadu_si128((const __m128i *)(bytes + 16));
  __m128i s = _mm_loadu_si128((const __m128i *)(bytes + 32));
  /* SSE3 */
  P("addsub_ps", _mm_addsub_ps(a, b)); P("hadd_ps", _mm_hadd_ps(a, b)); P("hsub_ps", _mm_hsub_ps(a, b));
  P("movehdup", _mm_movehdup_ps(a)); P("moveldup", _mm_moveldup_ps(a)); P("addsub_pd", _mm_addsub_pd(d, e));
  P("hadd_pd", _mm_hadd_pd(d, e)); P("hsub_pd", _mm_hsub_pd(d, e)); P("movedup", _mm_movedup_pd(d));
  P("loaddup", _mm_loaddup_pd(db + 1)); P("lddqu", _mm_lddqu_si128((const __m128i *)(bytes + 3)));
  /* SSSE3 */
  P("abs_epi8", _mm_abs_epi8(x)); P("abs_epi16", _mm_abs_epi16(x)); P("abs_epi32", _mm_abs_epi32(y));
  P("abs_epi32_min", _mm_abs_epi32(_mm_set1_epi32(-2147483647 - 1)));
  for (int n = 0; n <= 32; n += 5) { char name[32]; sprintf(name, "alignr_%d", n);
    switch (n) { case 0: P(name, _mm_alignr_epi8(x, y, 0)); break; case 5: P(name, _mm_alignr_epi8(x, y, 5)); break;
      case 10: P(name, _mm_alignr_epi8(x, y, 10)); break; case 15: P(name, _mm_alignr_epi8(x, y, 15)); break;
      case 20: P(name, _mm_alignr_epi8(x, y, 20)); break; case 25: P(name, _mm_alignr_epi8(x, y, 25)); break;
      default: P(name, _mm_alignr_epi8(x, y, 30)); } }
  P("hadd_epi16", _mm_hadd_epi16(x, y)); P("hadd_epi32", _mm_hadd_epi32(x, y)); P("hadds_epi16", _mm_hadds_epi16(x, y));
  P("hsub_epi16", _mm_hsub_epi16(x, y)); P("hsub_epi32", _mm_hsub_epi32(x, y)); P("hsubs_epi16", _mm_hsubs_epi16(x, y));
  P("maddubs", _mm_maddubs_epi16(x, y)); P("mulhrs", _mm_mulhrs_epi16(x, y)); P("shuffle_epi8", _mm_shuffle_epi8(x, y));
  P("sign_epi8", _mm_sign_epi8(x, y)); P("sign_epi16", _mm_sign_epi16(x, y)); P("sign_epi32", _mm_sign_epi32(x, y));
  /* SSE4.1 */
  P("blend_ps", _mm_blend_ps(a, b, 5)); P("blend_pd", _mm_blend_pd(d, e, 2)); P("blend_epi16", _mm_blend_epi16(x, y, 0xa5));
  P("blendv_ps", _mm_blendv_ps(a, b, a)); P("blendv_pd", _mm_blendv_pd(d, e, d)); P("blendv_epi8", _mm_blendv_epi8(x, y, x));
  __m128 r = _mm_set_ps(-2.5f, 3.5f, -0.5f, 2.5f);
  P("round_near", _mm_round_ps(r, _MM_FROUND_TO_NEAREST_INT)); P("round_down", _mm_round_ps(r, _MM_FROUND_TO_NEG_INF));
  P("round_up", _mm_round_ps(r, _MM_FROUND_TO_POS_INF)); P("round_zero", _mm_round_ps(r, _MM_FROUND_TO_ZERO));
  P("round_cur", _mm_round_ps(r, _MM_FROUND_CUR_DIRECTION)); P("floor_ps", _mm_floor_ps(b)); P("ceil_ps", _mm_ceil_ps(a));
  P("floor_pd", _mm_floor_pd(d)); P("ceil_pd", _mm_ceil_pd(e)); P("round_pd", _mm_round_pd(_mm_set_pd(-0.5, 1.5), _MM_FROUND_NINT));
  P("floor_ss", _mm_floor_ss(a, b)); P("ceil_sd", _mm_ceil_sd(d, e)); P("round_sd", _mm_round_sd(d, _mm_set_sd(-1.5), 0));
  P("mullo_epi32", _mm_mullo_epi32(x, y)); P("mul_epi32", _mm_mul_epi32(x, y));
  P("dp_ps", _mm_dp_ps(a, b, 0xf1)); P("dp_ps2", _mm_dp_ps(a, b, 0x5a)); P("dp_pd", _mm_dp_pd(d, e, 0x31)); P("dp_pd2", _mm_dp_pd(d, e, 0x12));
  P("min_epi8", _mm_min_epi8(x, y)); P("max_epi8", _mm_max_epi8(x, y)); P("min_epu16", _mm_min_epu16(x, y)); P("max_epu16", _mm_max_epu16(x, y));
  P("min_epi32", _mm_min_epi32(x, y)); P("max_epi32", _mm_max_epi32(x, y)); P("min_epu32", _mm_min_epu32(x, y)); P("max_epu32", _mm_max_epu32(x, y));
  P("insert_ps", _mm_insert_ps(a, b, 0x9a)); PI("extract_ps", _mm_extract_ps(b, 2));
  { float f; _MM_EXTRACT_FLOAT(f, a, 3); printf("extract_float: %a\n", f); }
  P("insert_epi8", _mm_insert_epi8(x, 0x77, 9)); P("insert_epi32", _mm_insert_epi32(x, -5, 2)); P("insert_epi64", _mm_insert_epi64(x, -6, 1));
  PI("extract_epi8", _mm_extract_epi8(x, 2)); PI("extract_epi32", _mm_extract_epi32(y, 3)); PI("extract_epi64", _mm_extract_epi64(y, 1));
  PI("testz", _mm_testz_si128(x, y)); PI("testc", _mm_testc_si128(x, y)); PI("testnzc", _mm_testnzc_si128(x, y));
  PI("testz2", _mm_testz_si128(_mm_set1_epi8(0x0f), _mm_set1_epi8((char)0xf0))); PI("test_all_ones", _mm_test_all_ones(_mm_set1_epi32(-1)));
  PI("testc2", _mm_testc_si128(x, _mm_and_si128(x, y)));
  P("cmpeq_epi64", _mm_cmpeq_epi64(x, _mm_insert_epi64(x, 0, 1))); P("packus_epi32", _mm_packus_epi32(x, y));
  P("cvtepi8_epi16", _mm_cvtepi8_epi16(x)); P("cvtepi8_epi32", _mm_cvtepi8_epi32(x)); P("cvtepi8_epi64", _mm_cvtepi8_epi64(x));
  P("cvtepi16_epi32", _mm_cvtepi16_epi32(x)); P("cvtepi16_epi64", _mm_cvtepi16_epi64(x)); P("cvtepi32_epi64", _mm_cvtepi32_epi64(x));
  P("cvtepu8_epi16", _mm_cvtepu8_epi16(x)); P("cvtepu8_epi32", _mm_cvtepu8_epi32(x)); P("cvtepu8_epi64", _mm_cvtepu8_epi64(x));
  P("cvtepu16_epi32", _mm_cvtepu16_epi32(x)); P("cvtepu16_epi64", _mm_cvtepu16_epi64(x)); P("cvtepu32_epi64", _mm_cvtepu32_epi64(x));
  P("mpsadbw0", _mm_mpsadbw_epu8(x, y, 0)); P("mpsadbw5", _mm_mpsadbw_epu8(x, y, 5)); P("mpsadbw7", _mm_mpsadbw_epu8(y, x, 7));
  P("minpos", _mm_minpos_epu16(x)); P("minpos2", _mm_minpos_epu16(_mm_set1_epi16(7)));
  P("stream_load", _mm_stream_load_si128((void *)(bytes + 16)));
  /* SSE4.2 */
  P("cmpgt_epi64", _mm_cmpgt_epi64(x, y));
  __m128i needle = _mm_setr_epi8('o', 'l', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0);
  __m128i range = _mm_setr_epi8('a', 'k', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0);
  __m128i word = _mm_setr_epi8('w', 'o', 'r', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0);
  int modes[] = {0x00, 0x04, 0x08, 0x0c, 0x10, 0x14, 0x30, 0x38, 0x40, 0x4c, 0x01, 0x09, 0x02, 0x06, 0x0b, 0x0d, 0x3c, 0x70};
  for (unsigned k = 0; k < sizeof modes / sizeof modes[0]; k++) {
    int m = modes[k]; __m128i n2 = (m & 0xc) == 4 ? range : (m & 0xc) == 0xc ? word : needle;
#define STR(M) printf("mode %02x: i=%d m=%016llx%016llx a=%d c=%d o=%d s=%d z=%d ei=%d em=%016llx ec=%d ez=%d es=%d\n", M, \
    _mm_cmpistri(n2, s, M), (unsigned long long)_mm_cmpistrm(n2, s, M)[1], (unsigned long long)_mm_cmpistrm(n2, s, M)[0], \
    _mm_cmpistra(n2, s, M), _mm_cmpistrc(n2, s, M), _mm_cmpistro(n2, s, M), _mm_cmpistrs(n2, s, M), _mm_cmpistrz(n2, s, M), \
    _mm_cmpestri(n2, 2, x, 9, M), (unsigned long long)_mm_cmpestrm(y, -3, x, 20, M)[0], _mm_cmpestrc(x, 16, y, -2147483647 - 1, M), \
    _mm_cmpestrz(y, 5, s, 12, M), _mm_cmpestrs(y, 5, s, 12, M))
    switch (m) {
    case 0x00: STR(0x00); break; case 0x04: STR(0x04); break; case 0x08: STR(0x08); break; case 0x0c: STR(0x0c); break;
    case 0x10: STR(0x10); break; case 0x14: STR(0x14); break; case 0x30: STR(0x30); break; case 0x38: STR(0x38); break;
    case 0x40: STR(0x40); break; case 0x4c: STR(0x4c); break; case 0x01: STR(0x01); break; case 0x09: STR(0x09); break;
    case 0x02: STR(0x02); break; case 0x06: STR(0x06); break; case 0x0b: STR(0x0b); break; case 0x0d: STR(0x0d); break;
    case 0x3c: STR(0x3c); break; default: STR(0x70); break; }
  }
  PI("crc32_u8", _mm_crc32_u8(0xffffffff, 0x61)); PI("crc32_u16", _mm_crc32_u16(1, 0xbeef));
  PI("crc32_u32", _mm_crc32_u32(0x12345678, 0xdeadbeef)); PI("crc32_u64", _mm_crc32_u64(~0ull, 0x0123456789abcdefull));
  PI("popcnt", _mm_popcnt_u32(0xf0f0f00f));
  return 0;
}
