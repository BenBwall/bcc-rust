/* GNU vector types on every back end: declarations, initializers and
 * compound literals, element-wise arithmetic, bitwise operators, shifts and
 * comparisons (with scalar operands), unary operators, subscripts, casts,
 * the shuffle and conversion builtins, sizes and alignments, unaligned
 * loads and stores, atomics, and the operators only one compiler accepts:
 * GCC's `++` and `&v[i]`, Clang's lax conversions. Built by GCC for the GCC
 * identity and by Clang for the Clang identity, the output is bcc's
 * reference; `vector_types.wasm32.expected` is what wasi-sdk's Clang build
 * prints for wasm32-wasip1. */
#include <stdio.h>
#include <string.h>

typedef signed char v16qi __attribute__((vector_size(16)));
typedef unsigned char v16qu __attribute__((vector_size(16)));
typedef short v8hi __attribute__((vector_size(16)));
typedef unsigned short v8hu __attribute__((vector_size(16)));
typedef int v4si __attribute__((vector_size(16)));
typedef unsigned v4su __attribute__((vector_size(16)));
typedef long long v2di __attribute__((vector_size(16)));
typedef unsigned long long v2du __attribute__((vector_size(16)));
typedef float v4sf __attribute__((vector_size(16)));
typedef double v2df __attribute__((vector_size(16)));
typedef int v2si __attribute__((vector_size(8)));
typedef float v2sf __attribute__((vector_size(8)));
typedef short v4hi __attribute__((vector_size(8)));
typedef char v4qi __attribute__((vector_size(4)));
typedef int v8si __attribute__((vector_size(32)));
typedef double v4df __attribute__((vector_size(32)));
typedef float v16sf __attribute__((vector_size(64)));
typedef int v16si __attribute__((vector_size(64)));
typedef float v4sf_u __attribute__((vector_size(16), aligned(1)));

#define SHOW(name, ...) do { __typeof__(__VA_ARGS__) v = (__VA_ARGS__); printf("%s:", name); for (unsigned i_ = 0; i_ < sizeof(v) / sizeof((v)[0]); i_++) printf(" %lld", (long long)(v)[i_]); printf("\n"); } while (0)
#define SHOWU(name, ...) do { __typeof__(__VA_ARGS__) v = (__VA_ARGS__); printf("%s:", name); for (unsigned i_ = 0; i_ < sizeof(v) / sizeof((v)[0]); i_++) printf(" %llu", (unsigned long long)(v)[i_]); printf("\n"); } while (0)
#define SHOWF(name, ...) do { __typeof__(__VA_ARGS__) v = (__VA_ARGS__); printf("%s:", name); for (unsigned i_ = 0; i_ < sizeof(v) / sizeof((v)[0]); i_++) printf(" %a", (double)(v)[i_]); printf("\n"); } while (0)

v4si gi = {1, -2, 3, -4};
static v4sf gf = {1.5f, -2.25f};
v2df gd[2] = {{1.0, 2.0}, {3.0}};
struct holder { int tag; v4si v; v2sf f; } gh = {7, {9, 8, 7, 6}, {0.5f, 0.25f}};
v16qi gq = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16};

volatile int vol_zero = 0;

static int id(int x) { return x + vol_zero; }

static void arithmetic(void) {
  v4si a = {id(7), id(-8), id(100), id(-2147483647 - 1)};
  v4si b = {id(3), id(5), id(-7), id(-1 + 2)};
  SHOW("add", a + b);
  SHOW("sub", a - b);
  SHOW("mul", a * b);
  SHOW("div", a / b);
  SHOW("mod", a % b);
  SHOW("and", a & b);
  SHOW("or", a | b);
  SHOW("xor", a ^ b);
  SHOW("shl", a << (v4si){1, 2, 3, 0});
  SHOW("shr", a >> (v4si){1, 2, 3, 31});
  SHOW("neg", -a);
  SHOW("not", ~a);
  SHOW("plus", +a);
  SHOW("splat_add", a + 1);
  SHOW("splat_mul", 3 * a);
  int s = id(2);
  SHOW("splat_var_shl", a << s);
  SHOW("splat_var_shr", a >> s);
  SHOW("splat_sub", s - a);
  v4su ua = (v4su)a;
  SHOWU("ushr", ua >> 3);
  SHOWU("udiv", ua / (v4su){3, 5, 7, 9});
  SHOWU("umod", ua % 10u);
  v16qi q = gq * (v16qi){id(30)};
  SHOW("qmul", q);
  v16qi q2 = gq * 30;
  SHOW("qmul_splat", q2);
  v16qu uq = (v16qu)gq + 250;
  SHOWU("uqadd", uq);
  v8hi h = {id(30000), 2, 3, 4, -5, 6, 7, -32768};
  SHOW("hadd", h + h);
  SHOW("hneg", -h);
  SHOW("hshr", h >> 2);
  v8hu uh = (v8hu)h;
  SHOWU("uhshr", uh >> 2);
  v2di l = {id(1) * 0x100000000LL, -id(5)};
  SHOW("ladd", l + l * 7);
  SHOW("lshr", l >> 3);
  v2du ul = (v2du)l;
  SHOWU("ulshr", ul >> 3);
}

static void floating(void) {
  v4sf a = {id(1) * 1.5f, -2.0f, 0.1f, 1e30f};
  v4sf b = {2.0f, id(3) * 0.5f, 3.0f, 1e10f};
  SHOWF("fadd", a + b);
  SHOWF("fsub", a - b);
  SHOWF("fmul", a * b);
  SHOWF("fdiv", a / b);
  SHOWF("fneg", -a);
  SHOWF("fsplat", a * 2);
  SHOWF("fsplat2", 0.5f + a);
  float s = 0.25f * id(1);
  SHOWF("fsplatvar", a - s);
  v2df d = {id(1) / 3.0, -0.0};
  SHOWF("dmul", d * d);
  SHOWF("ddiv", d / 7);
  SHOWF("dsplat", d + 1.0);
  v2sf h = {1.0f, 2.0f};
  SHOWF("v2sf", h * h + h);
  v4df wd = {1, 2, 3, 4};
  SHOWF("v4df", wd * 0.5 - wd);
}

static void compare(void) {
  v4si a = {id(1), 2, 3, -4};
  v4si b = {1, id(3), 2, 4};
  SHOW("eq", a == b);
  SHOW("ne", a != b);
  SHOW("lt", a < b);
  SHOW("le", a <= b);
  SHOW("gt", a > b);
  SHOW("ge", a >= b);
  v4su ua = (v4su)a, ub = (v4su)b;
  SHOW("ult", ua < ub);
  SHOW("lt_splat", a < 2);
  float nan = __builtin_nanf("") * id(1);
  v4sf f = {1.0f, nan, -0.0f, 2.0f};
  v4sf g = {1.0f, nan, 0.0f, 1.0f};
  SHOW("feq", f == g);
  SHOW("fne", f != g);
  SHOW("flt", f < g);
  SHOW("fge", f >= g);
  v2df d = {1.0, nan};
  SHOW("deq", d == d);
  v16qi q = gq > 8;
  SHOW("qgt", q);
  v8hi h = (v8hi){1, 2, 3, 4, 5, 6, 7, 8} <= 4;
  SHOW("hle", h);
  v2di m = (v2di){1, 2} != (v2di){1, 3};
  SHOW("lne", m);
}

static void subscripts(void) {
  v4si a = gi;
  int i = id(2);
  a[i] = 42;
  a[0] += 10;
  a[i + 1]++;
  SHOW("sub_write", a);
  printf("sub_read: %d %d\n", a[i], (a + 1)[3]);
  v4sf f = gf;
  f[1] = f[0] * 2;
  SHOWF("fsub_write", f);
  struct holder h = gh;
  h.v[3] = h.tag;
  SHOW("member", h.v);
  v4si *p = &a;
  (*p)[1] = -99;
  printf("ptr_sub: %d\n", (*p)[1]);
}

static void casts(void) {
  v4si a = {0x3f800000, 0x40000000, -0x3f800000 - 0x40000000, 0};
  v4sf f = (v4sf)a;
  SHOWF("bitcast_f", f);
  v16qi q = (v16qi)a;
  SHOW("bitcast_q", q);
  v2di l = (v2di)a;
  SHOW("bitcast_l", l);
  long long x = 0x0000000200000001LL + vol_zero;
  v2si s = (v2si)x;
  SHOW("scalar_to_vec", s);
  long long back = (long long)(s + 1);
  printf("vec_to_scalar: %lld\n", back);
  v4hi h = (v4hi)s;
  SHOW("v2si_to_v4hi", h);
  v4si lax = (v4si)(v4su){1, 2, 3, 4};
  SHOW("lax", lax);
}

static void builtins(void) {
  v4si a = {10, 20, 30, 40};
  v4si b = {50, 60, 70, 80};
  SHOW("shufflevector", __builtin_shufflevector(a, b, 0, 5, 2, 7));
  SHOW("shufflevector_narrow", __builtin_shufflevector(a, b, 3, 4));
  SHOW("shufflevector_wide", __builtin_shufflevector(a, a, 0, 1, 2, 3, 3, 2, 1, 0));
  SHOWF("convertvector", __builtin_convertvector(a, v4sf) / 3);
  v4sf f = {1.75f, -2.5f, 1e9f, -0.5f};
  SHOW("convertvector_trunc", __builtin_convertvector(f, v4si));
  SHOWF("convertvector_df", __builtin_convertvector(__builtin_shufflevector(f, f, 0, 1), v2df));
  v2df d = {1.5, -3.25};
  SHOW("convertvector_di", __builtin_convertvector(d, v2di));
#ifndef __clang__
  v4si m = {3, 0, 6, 5};
  SHOW("shuffle1", __builtin_shuffle(a, m));
  SHOW("shuffle2", __builtin_shuffle(a, b, m));
  SHOWF("shufflef", __builtin_shuffle(f, (v4si){1, 1, 0, 7}));
#else
  v4si m = {3, 0, 6, 5};
  SHOW("shuffle1", __builtin_shufflevector(a, m));
  SHOW("shuffle2", __builtin_shufflevector(a, b, 3, 0, 6, 5));
  SHOWF("shufflef", __builtin_shufflevector(f, (v4si){1, 1, 0, 7}));
#endif
}

static void sizes(void) {
  printf("sizes: %zu %zu %zu %zu %zu %zu %zu\n", sizeof(v16qi), sizeof(v2si), sizeof(v4qi),
         sizeof(v8si), sizeof(v16sf), sizeof(struct holder), sizeof(v4sf_u));
  printf("aligns: %zu %zu %zu %zu %zu %zu\n", _Alignof(v16qi), _Alignof(v2si), _Alignof(v4qi),
         __alignof__(v8si), __alignof__(v16sf), _Alignof(v4sf_u));
  v4si x;
  printf("sizeof_expr: %zu %zu\n", sizeof x, sizeof x[0]);
}

static void memory(void) {
  float buf[9] = {0, 1, 2, 3, 4, 5, 6, 7, 8};
  v4sf_u u = *(v4sf_u *)(buf + 1);
  SHOWF("unaligned_load", u);
  *(v4sf_u *)(buf + 3) = u * 10;
  for (int i = 0; i < 9; i++) printf(" %g", buf[i]);
  printf("\n");
  v4si arr[3] = {{1}, {2, 2}, {3, 3, 3}};
  arr[1] += arr[2];
  SHOW("array0", arr[0]);
  SHOW("array1", arr[1]);
  unsigned char bytes[16];
  memcpy(bytes, &arr[2], 16);
  for (int i = 0; i < 16; i++) printf("%02x", bytes[i]);
  printf("\n");
  v4si *heap = &arr[0];
  heap[2] = heap[0] - heap[1];
  SHOW("ptr_index", arr[2]);
  volatile v4si vv = {5, 6, 7, 8};
  vv = vv + 1;
  v4si copy = vv;
  SHOW("volatile", copy);
}

static void compound_literals(void) {
  SHOW("lit", (v4si){1, 2, 3, 4} * (v4si){5, 6, 7, 8});
  SHOW("lit_partial", (v4si){9});
  SHOW("lit_empty", (v4si){});
  int k = id(4);
  SHOW("lit_runtime", (v4si){k, k + 1, k * k, -k});
  v4si nested[2] = {(v4si){1, 2, 3, 4}, gi};
  SHOW("nested", nested[0] + nested[1]);
}

static void compound_assignment(void) {
  v4si a = {100, 200, 300, 400};
  a += 1;
  a -= (v4si){1, 2, 3, 4};
  a *= 2;
  a /= (v4si){1, 2, 3, 4};
  a %= 7;
  a <<= 2;
  a >>= 1;
  a &= 0xff;
  a |= 0x100;
  a ^= (v4si){1, 1, 1, 1};
  SHOW("compound", a);
  v4sf f = {1, 2, 3, 4};
  f *= 0.5f;
  f += f;
  f /= (v4sf){3, 3, 3, 3};
  SHOWF("fcompound", f);
}

static v4si pick(int c, v4si a, v4si b) { return c ? a : b; }

static void conditional(void) {
  v4si a = {1, 2, 3, 4}, b = {5, 6, 7, 8};
  SHOW("cond_true", pick(id(1), a, b));
  SHOW("cond_false", pick(id(0), a, b));
  SHOW("cond_expr", (id(1) > 0 ? a : b) + (id(0) ? a : b));
  v4si c = (a, b);
  SHOW("comma", c);
}

_Atomic v4si av;
_Atomic v8si av8;
_Atomic v16sf av64;
_Atomic v16si av64q;
volatile int z = 0;
struct pair { v2si a; v4df b; };

static void extras(void) {
  v4si a = {1, 2, 3, 4};
#ifndef __wasm__
  /* wasi-sdk has no libatomic for 16-byte atomic objects. */
  av = a;
  av = av + a;
  av += (v4si){10, 20, 30, 40};
  v4si r = av;
  SHOW("atomic", r);
  printf("atomic_lane: %d\n", av[2]);
  /* Atomic objects no register holds update through byte-wise
   * compare-exchange. */
  av8 = (v8si){1, 2, 3, 4, 5, 6, 7, 8};
  av8 += (v8si){1, 1, 1, 1, 1, 1, 1, 1};
  av8 = av8 * (v8si){2, 2, 2, 2, 2, 2, 2, 2};
  v8si r8 = av8;
  SHOW("atomic32", r8);
  av64 = (v16sf){1, 2, 3};
  av64 -= (v16sf){0.5f};
  v16sf r64 = av64;
  SHOWF("atomic64", r64);
  v16si init = {1, 2, 3};
  init[15] = 64;
  av64q = init;
#ifndef __clang__
  v8si old8 = av8++;
  r8 = av8;
  SHOW("atomic32_inc", r8);
  SHOW("atomic32_old", old8);
  v16si q = av64q--;
  SHOW("atomic64_old", q);
  q = av64q;
  SHOW("atomic64_dec", q);
#endif
#endif
#ifndef __clang__
  v4si b = a;
  b++;
  ++b;
  v4si old = b--;
  SHOW("inc", b);
  SHOW("postinc_old", old);
  int *p = &b[2];
  *p = 77;
  SHOW("addr_lane", b);
  v4sf f = {1.5f, 2.5f};
  f++;
  SHOW("finc", f * 2);
#else
  v4su u = {5, 6, 7, 8};
  v4si c = z ? a : u;
  SHOW("lax_cond", c);
  v2si s = {1, 2};
  long long l = 0x0000000300000004LL + z;
  SHOW("lax_scalar", s + l);
  v4si fb = a + 1.0f;
  SHOW("float_scalar_int_lanes", fb);
#endif
  struct pair pr = {{1, 2}, {3, 4, 5, 6}};
  struct pair pc = pr;
  pc.b *= 2;
  SHOW("struct_member", pc.b);
  SHOW("struct_member_a", pc.a + pr.a);
  v4si mask = a > 2;
  v4sf sel = (v4sf)(((v4si)(v4sf){1, 2, 3, 4} & mask) | ((v4si)(v4sf){9, 9, 9, 9} & ~mask));
  SHOW("select", sel);
  }

#ifndef __wasm__
/* Half-precision lanes compute in float and round each operation;
 * Clang also takes bit-precise lanes of a power-of-two width, and its
 * __builtin_bit_cast reinterprets any object of the same size. Both
 * compilers refuse _Float16 on wasm32. */
typedef _Float16 v8hf __attribute__((vector_size(16)));
typedef __bf16 v8bf __attribute__((vector_size(16)));
struct pairf { float x, y; };

static void halves(void) {
  v8hf a = {1.5f16, 2.25f16, -3.0f16, 0.1f16, 1000.0f16, 0.333f16, -0.0f16, 65504.0f16};
  SHOWF("half_ops", a * a + a);
  SHOWF("half_neg_div", -a / 3);
  SHOW("half_lt", a < a * 2);
  v8bf x = {1.5, 2.25, -3.0, 0.1, 1000.0, 0.333, -0.0, 3e38};
  SHOWF("bf16_ops", x * x - x);
#ifdef __clang__
  typedef _BitInt(32) v4bi __attribute__((vector_size(16)));
  typedef unsigned _BitInt(8) v16bu __attribute__((vector_size(16)));
  v4bi p = {1, -2, 3, 2147483647};
  p = p * 3 + p;
  SHOW("bitint_lanes", p);
  v16bu q = {200, 3};
  SHOW("bitint_u8_lanes", q * 3);
  float f = 1.0f + z;
  printf("bit_cast_scalar: %x\n", __builtin_bit_cast(int, f));
  v2sf v = {1.5f, -2.0f};
  v2si bits = __builtin_bit_cast(v2si, v);
  SHOW("bit_cast_vector", bits);
  long long whole = __builtin_bit_cast(long long, v);
  printf("bit_cast_to_scalar: %llx\n", whole);
  struct pairf pf = __builtin_bit_cast(struct pairf, v);
  printf("bit_cast_struct: %a %a\n", pf.x, pf.y);
  SHOWF("bit_cast_from_struct", __builtin_bit_cast(v2sf, ((struct pairf){3.0f, 4.0f})));
#endif
}
#endif

int main(void) {
  extras();
#ifndef __wasm__
  halves();
#endif
  arithmetic();
  floating();
  compare();
  subscripts();
  casts();
  builtins();
  sizes();
  memory();
  compound_literals();
  compound_assignment();
  conditional();
  SHOW("global", gi);
  SHOWF("global_f", gf);
  SHOWF("global_d0", gd[0]);
  SHOWF("global_d1", gd[1]);
  SHOW("global_struct", gh.v);
  SHOWF("global_struct_f", gh.f);
  return 0;
}
