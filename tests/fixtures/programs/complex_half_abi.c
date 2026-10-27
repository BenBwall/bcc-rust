/* _Complex _Float16 across the System V x86-64 ABI: both compilers pass
   one in the low 32 bits of a vector register, real part first, as a
   structure of two halves; structures holding them, mixed with float and
   _Float16, and variadic arguments. Built whole, or as the callee half
   (-DCALLEE_ONLY) and the caller half (-DCALLER_ONLY) by different
   compilers; the caller prints what came back. */
#include <stdarg.h>
#include <stdio.h>
#include <string.h>
typedef _Float16 H;
typedef _Complex _Float16 C;
struct P { C a; C b; };
struct Q { C a; float f; };
struct R { C a, b, c, d, e; };
struct T { H h; C c; };
C lib_c(C a, float f, C b, double d, C c);
C lib_many(C a, C b, C c, C d, C e, C f, C g, C h, C i, C j);
struct P lib_p(struct P p);
struct Q lib_q(struct Q q);
struct R lib_r(struct R r);
struct T lib_t(struct T t);
double lib_va(int n, ...);
#ifndef CALLER_ONLY
C lib_c(C a, float f, C b, double d, C c) { return a * b + c * (H)f + (H)d; }
C lib_many(C a, C b, C c, C d, C e, C f, C g, C h, C i, C j) { return a + b + c + d + e + f + g + h + i * 2 + j * 4; }
struct P lib_p(struct P p) { p.a *= p.b; p.b /= 2; return p; }
struct Q lib_q(struct Q q) { q.a += q.f; q.f *= 2; return q; }
struct R lib_r(struct R r) { r.a += r.e; r.e = r.b * r.c; return r; }
struct T lib_t(struct T t) { t.c *= t.h; t.h += 1; return t; }
double lib_va(int n, ...) {
  va_list ap; va_start(ap, n); double sum = 0;
  for (int i = 0; i < n; i++) {
    if (i % 2 == 0) { C c = va_arg(ap, C); sum += (double)__real__ c + (double)__imag__ c * 10; }
    else sum += va_arg(ap, double) * 100;
  }
  va_end(ap); return sum;
}
#endif
#ifndef CALLEE_ONLY
static unsigned hb(C h){ unsigned u; memcpy(&u,&h,4); return u; }
static C mk(float r, float i) { C c; __real__ c = r; __imag__ c = i; return c; }
int main(void) {
  printf("c %08x\n", hb(lib_c(mk(1.5, 2), 3.0f, mk(0.5, -1), 0.25, mk(2, 3))));
  printf("many %08x\n", hb(lib_many(mk(1,2), mk(3,4), mk(5,6), mk(7,8), mk(9,10), mk(11,12), mk(13,14), mk(15,16), mk(17,18), mk(19,20))));
  struct P p = lib_p((struct P){ mk(1, 2), mk(3, 4) }); printf("p %08x %08x\n", hb(p.a), hb(p.b));
  struct Q q = lib_q((struct Q){ mk(1, 2), 1.5f }); printf("q %08x %g\n", hb(q.a), q.f);
  struct R r = lib_r((struct R){ mk(1, 2), mk(3, 4), mk(5, 6), mk(7, 8), mk(9, 10) }); printf("r %08x %08x\n", hb(r.a), hb(r.e));
  struct T t = lib_t((struct T){ 2, mk(1, 2) }); printf("t %04x %08x\n", (unsigned)*(unsigned short*)&t.h, hb(t.c));
  printf("va %g\n", lib_va(5, mk(1, 2), 3.0, mk(4, 5), 6.0, mk(7, 8)));
  printf("sz %zu %zu %zu %zu\n", sizeof(C), _Alignof(C), sizeof(struct T), _Alignof(struct T));
  return 0;
}
#endif
