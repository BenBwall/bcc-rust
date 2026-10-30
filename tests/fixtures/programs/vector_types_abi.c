/* GNU vectors passed and returned by value between separately compiled
 * halves: the definitions (CALLEE_ONLY) and the calls that check them
 * (CALLER_ONLY), each built by bcc's back ends and by the compiler whose
 * identity bcc claims. Without either macro the program is whole. The
 * shapes cover System V's SSE (8 and 16 bytes), INTEGER (1 to 4 bytes) and
 * memory classes, Clang's 32-byte results in two XMM registers, vectors in
 * structures, register exhaustion, mixed arguments, `va_arg` of a vector,
 * and the one-lane vectors GCC and Clang pass differently. Clang returns a
 * 64-byte vector in four XMM registers, which bcc does not, so the Clang
 * build does not call `f8df`. */
#include <stdarg.h>
#include <stdio.h>

typedef float v4sf __attribute__((vector_size(16)));
typedef int v4si __attribute__((vector_size(16)));
typedef double v2df __attribute__((vector_size(16)));
typedef long long v2di __attribute__((vector_size(16)));
typedef char v16qi __attribute__((vector_size(16)));
typedef float v2sf __attribute__((vector_size(8)));
typedef int v2si __attribute__((vector_size(8)));
typedef short v4hi __attribute__((vector_size(8)));
typedef long long v1di __attribute__((vector_size(8)));
typedef char v4qi __attribute__((vector_size(4)));
typedef short v2hi __attribute__((vector_size(4)));
typedef char v2qi __attribute__((vector_size(2)));
typedef float v8sf __attribute__((vector_size(32)));
typedef int v8si __attribute__((vector_size(32)));
typedef double v8df __attribute__((vector_size(64)));
typedef __int128 v1ti __attribute__((vector_size(16)));
typedef float v1sf __attribute__((vector_size(4)));
typedef double v1df __attribute__((vector_size(8)));
typedef long double v1xf __attribute__((vector_size(16)));
struct sv { v4sf v; };
struct s2 { v2sf v; float f; };
struct s3 { v2si v; int i; };
struct s4 { v4qi a; float b; };
struct big { v4si a; v4si b; };
struct s8 { v8sf v; };
/* One-lane vector members: GCC puts all three records in memory (the
 * long double one as a result; as an argument it is x87 data, memory for
 * both), Clang the double one. */
struct lf { v1sf v; };
struct ld { v1df v; };
struct ll { v1xf v; };
void af(struct lf s, double x);
void ad(struct ld s, double x);
void al(struct ll s, double x);
struct lf rf(void);
struct ld rd(void);
struct ll rl(void);
v4sf f4sf(v4sf a, v4sf b);
v4si f4si(v4si a, int s, v4si b);
v2df f2df(v2df a, double d);
v2di f2di(long x, v2di a, v2di b);
v16qi f16qi(v16qi a);
v2sf f2sf(v2sf a, v2sf b);
v2si f2si(v2si a, v2si b);
v4hi f4hi(v4hi a);
v1di f1di(v1di a, v1di b);
v4qi f4qi(v4qi a, v4qi b);
v2hi f2hi(v2hi a, int x);
v2qi f2qi(v2qi a);
v8sf f8sf(v8sf a, v8sf b);
v8si f8si(v8si a);
v8df f8df(v8df a);
v1ti f1ti(v1ti a, v1ti b);
struct sv fsv(struct sv a);
struct s2 fs2(struct s2 a);
struct s3 fs3(struct s3 a);
struct s4 fs4(struct s4 a);
struct big fbig(struct big a);
struct s8 fs8(struct s8 a);
v4sf many(v4sf a, v4sf b, v4sf c, v4sf d, v4sf e, v4sf f, v4sf g, v4sf h, v4sf i, v4sf j, double x, v2df k);
v4si mixed(int a, v4si b, long c, v2sf d, float e, v4qi f, double g, v8si h, v2df i);
v4si varsum(int n, ...);
v1sf f1sf(v1sf a, v1sf b);
v1df f1df(v1df a, v1df b);
v1xf f1xf(v1xf a, v1xf b);

#ifndef CALLER_ONLY
v4sf f4sf(v4sf a, v4sf b) { return a * b + 1; }
v4si f4si(v4si a, int s, v4si b) { return a * s - b; }
v2df f2df(v2df a, double d) { return a / d; }
v2di f2di(long x, v2di a, v2di b) { return (a ^ b) + x; }
v16qi f16qi(v16qi a) { return a * 3; }
v2sf f2sf(v2sf a, v2sf b) { return a - b; }
v2si f2si(v2si a, v2si b) { return a * b; }
v4hi f4hi(v4hi a) { return -a; }
v1di f1di(v1di a, v1di b) { return a + b; }
v4qi f4qi(v4qi a, v4qi b) { return a + b; }
v2hi f2hi(v2hi a, int x) { return a * (short)x; }
v2qi f2qi(v2qi a) { return ~a; }
v8sf f8sf(v8sf a, v8sf b) { return a + b; }
v8si f8si(v8si a) { return a << 1; }
v8df f8df(v8df a) { return a * a; }
v1ti f1ti(v1ti a, v1ti b) { return a + b; }
struct sv fsv(struct sv a) { a.v = a.v * 2; return a; }
struct s2 fs2(struct s2 a) { a.v = a.v + a.f; a.f += 1; return a; }
struct s3 fs3(struct s3 a) { a.v = a.v + a.i; a.i *= 2; return a; }
struct s4 fs4(struct s4 a) { a.a = a.a + 1; a.b *= 3; return a; }
struct big fbig(struct big a) { struct big r = {a.b, a.a}; return r; }
struct s8 fs8(struct s8 a) { a.v = -a.v; return a; }
v4sf many(v4sf a, v4sf b, v4sf c, v4sf d, v4sf e, v4sf f, v4sf g, v4sf h, v4sf i, v4sf j, double x, v2df k) {
  return a + b * 2 + c * 3 + d * 4 + e * 5 + f * 6 + g * 7 + h * 8 + i * 9 + j * 10 + (float)x + (float)k[1];
}
v4si mixed(int a, v4si b, long c, v2sf d, float e, v4qi f, double g, v8si h, v2df i) {
  v4si r = b + a + (int)c;
  r[0] += (int)(d[0] * 10) + (int)(d[1] * 100);
  r[1] += (int)e + f[0] + f[3];
  r[2] += (int)g + h[0] + h[7];
  r[3] += (int)(i[0] + i[1]);
  return r;
}
v4si varsum(int n, ...) {
  va_list ap; va_start(ap, n);
  v4si s = {0};
  for (int k = 0; k < n; k++) s += va_arg(ap, v4si);
  va_end(ap);
  return s;
}
v1sf f1sf(v1sf a, v1sf b) { return a + b; }
v1df f1df(v1df a, v1df b) { return a * b; }
v1xf f1xf(v1xf a, v1xf b) { return a - b; }
void af(struct lf s, double x) { printf("af %g %g\n", s.v[0], x); }
void ad(struct ld s, double x) { printf("ad %g %g\n", s.v[0], x); }
void al(struct ll s, double x) { printf("al %Lg %g\n", s.v[0], x); }
struct lf rf(void) { return (struct lf){{1.5f}}; }
struct ld rd(void) { return (struct ld){{2.5}}; }
struct ll rl(void) { return (struct ll){{3.5L}}; }
#endif

#ifndef CALLEE_ONLY
#define P(name, v) do { printf("%s:", name); for (unsigned i_ = 0; i_ < sizeof(v) / sizeof((v)[0]); i_++) printf(" %g", (double)(v)[i_]); printf("\n"); } while (0)
int main(void) {
  v4sf a = {1, 2, 3, 4}, b = {0.5f, 0.25f, 2, -1};
  P("f4sf", f4sf(a, b));
  P("f4si", f4si((v4si){1, 2, 3, 4}, 7, (v4si){9, 8, 7, 6}));
  P("f2df", f2df((v2df){3, 9}, 3));
  P("f2di", f2di(5, (v2di){1, 2}, (v2di){3, 4}));
  P("f16qi", f16qi((v16qi){1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16}));
  P("f2sf", f2sf((v2sf){5, 6}, (v2sf){1, 2}));
  P("f2si", f2si((v2si){5, 6}, (v2si){3, 4}));
  P("f4hi", f4hi((v4hi){1, -2, 3, -4}));
  P("f1di", f1di((v1di){40}, (v1di){2}));
  P("f4qi", f4qi((v4qi){1, 2, 3, 4}, (v4qi){10, 20, 30, 40}));
  P("f2hi", f2hi((v2hi){7, 8}, 3));
  P("f2qi", f2qi((v2qi){1, 2}));
  P("f8sf", f8sf((v8sf){1, 2, 3, 4, 5, 6, 7, 8}, (v8sf){8, 7, 6, 5, 4, 3, 2, 1}));
  P("f8si", f8si((v8si){1, 2, 3, 4, 5, 6, 7, 8}));
#ifndef __clang__
  P("f8df", f8df((v8df){1, 2, 3, 4, 5, 6, 7, 8}));
#endif
  P("f1ti", f1ti((v1ti){5}, (v1ti){7}));
  struct sv sv = fsv((struct sv){a});
  P("fsv", sv.v);
  struct s2 s2 = fs2((struct s2){{1, 2}, 3});
  P("fs2", s2.v); printf("fs2.f: %g\n", s2.f);
  struct s3 s3 = fs3((struct s3){{1, 2}, 3});
  P("fs3", s3.v); printf("fs3.i: %d\n", s3.i);
  struct s4 s4 = fs4((struct s4){{1, 2, 3, 4}, 1.5f});
  P("fs4", s4.a); printf("fs4.b: %g\n", s4.b);
  struct big bg = fbig((struct big){{1, 2, 3, 4}, {5, 6, 7, 8}});
  P("fbig.a", bg.a); P("fbig.b", bg.b);
  struct s8 s8 = fs8((struct s8){{1, 2, 3, 4, 5, 6, 7, 8}});
  P("fs8", s8.v);
  P("many", many(a, a, a, a, a, a, a, a, a, b, 0.5, (v2df){1, 2}));
  P("mixed", mixed(1, (v4si){10, 20, 30, 40}, 2, (v2sf){0.5f, 0.25f}, 3.0f, (v4qi){1, 2, 3, 4}, 5.0, (v8si){1, 0, 0, 0, 0, 0, 0, 2}, (v2df){0.5, 0.5}));
  P("varsum", varsum(3, (v4si){1, 2, 3, 4}, (v4si){10, 20, 30, 40}, (v4si){100, 200, 300, 400}));
  P("f1sf", f1sf((v1sf){1.5f}, (v1sf){2}));
  P("f1df", f1df((v1df){1.5}, (v1df){4}));
  P("f1xf", f1xf((v1xf){1.5L}, (v1xf){0.25L}));
  af((struct lf){{4.5f}}, 7);
  ad((struct ld){{5.5}}, 8);
  al((struct ll){{6.5L}}, 9);
  printf("rf %g rd %g rl %Lg\n", rf().v[0], rd().v[0], rl().v[0]);
  return 0;
}
#endif
