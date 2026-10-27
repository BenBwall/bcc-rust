/* _Float16 across the System V x86-64 ABI: scalars in vector registers and
   on the stack, structures of halves alone and mixed with other classes, a
   union, and variadic arguments, which are not promoted. Built whole, or as
   the callee half (-DCALLEE_ONLY) and the caller half (-DCALLER_ONLY) by
   different compilers; the caller prints what came back. */
#include <stdarg.h>
#include <stdio.h>
#include <string.h>
typedef _Float16 H;
struct S1 { H a; };
struct S2 { H a, b; };
struct S3 { H a, b, c; };
struct S5 { H a[5]; };
struct S9 { H a[9]; };
struct SF { H a; float b; };
struct SI { H a; int b; };
struct SD { H a; double d; };
struct S4F { H a[4]; float f; };
union U { H a; short s; };
H lib_scalar(H a, float f, H b, double d, int i, H c);
H lib_many(H a, H b, H c, H d, H e, H f, H g, H h, H i, H j, H k);
struct S1 lib_s1(struct S1 s);
struct S2 lib_s2(struct S2 s);
struct S3 lib_s3(struct S3 s);
struct S5 lib_s5(struct S5 s);
struct S9 lib_s9(struct S9 s);
struct SF lib_sf(struct SF s);
struct SI lib_si(struct SI s);
struct SD lib_sd(struct SD s);
struct S4F lib_s4f(struct S4F s);
union U lib_u(union U u);
double lib_va(int n, ...);
void lib_call_va(double (*f)(int, ...));
H lib_ptr(H *p);
#ifndef CALLER_ONLY
H lib_scalar(H a, float f, H b, double d, int i, H c) { return (H)(a + b * c) + (H)f + (H)d + i; }
H lib_many(H a, H b, H c, H d, H e, H f, H g, H h, H i, H j, H k) { return a + b + c + d + e + f + g + h + i + j * 2 + k * 4; }
struct S1 lib_s1(struct S1 s) { s.a += 1; return s; }
struct S2 lib_s2(struct S2 s) { s.a += s.b; s.b *= 2; return s; }
struct S3 lib_s3(struct S3 s) { s.a += s.c; s.c -= 1; return s; }
struct S5 lib_s5(struct S5 s) { for (int i = 0; i < 5; i++) s.a[i] *= (H)(i + 1); return s; }
struct S9 lib_s9(struct S9 s) { for (int i = 0; i < 9; i++) s.a[i] += (H)i; return s; }
struct SF lib_sf(struct SF s) { s.a += 1; s.b *= 2; return s; }
struct SI lib_si(struct SI s) { s.a += s.b; s.b += 1; return s; }
struct SD lib_sd(struct SD s) { s.a += 1; s.d *= 3; return s; }
struct S4F lib_s4f(struct S4F s) { s.a[3] += s.f; s.f += 1; return s; }
union U lib_u(union U u) { u.a *= 2; return u; }
double lib_va(int n, ...) {
  va_list ap; va_start(ap, n); double sum = 0;
  for (int i = 0; i < n; i++) {
    switch (i % 4) {
      case 0: sum += (double)va_arg(ap, H); break;
      case 1: sum += va_arg(ap, double) * 10; break;
      case 2: { struct S2 s = va_arg(ap, struct S2); sum += (double)s.a * 100 + (double)s.b * 1000; break; }
      default: sum += va_arg(ap, int) * 10000; break;
    }
  }
  va_end(ap); return sum;
}
void lib_call_va(double (*f)(int, ...)) {
  struct S2 s = { 1.5f16, -2.25f16 };
  printf("lib->va %g\n", f(13, (H)0.5, 2.0, s, 3, (H)1.25, 4.0, s, 5, (H)-3.5, 6.0, s, 7, (H)9.75));
}
H lib_ptr(H *p) { return p[0] + p[1]; }
#endif
#ifndef CALLEE_ONLY
static unsigned short hb(H h){ unsigned short u; memcpy(&u,&h,2); return u; }
double my_va(int n, ...) {
  va_list ap; va_start(ap, n); double sum = 0;
  for (int i = 0; i < n; i++) {
    switch (i % 4) {
      case 0: sum += (double)va_arg(ap, H); break;
      case 1: sum += va_arg(ap, double) * 10; break;
      case 2: { struct S2 s = va_arg(ap, struct S2); sum += (double)s.a * 100 + (double)s.b * 1000; break; }
      default: sum += va_arg(ap, int) * 10000; break;
    }
  }
  va_end(ap); return sum;
}
int main(void) {
  printf("scalar %04x\n", hb(lib_scalar(1.5f16, 2.5f, 3.0f16, 4.5, 7, -0.5f16)));
  printf("many %04x\n", hb(lib_many(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11)));
  struct S1 s1 = lib_s1((struct S1){ 2.5f16 }); printf("s1 %04x\n", hb(s1.a));
  struct S2 s2 = lib_s2((struct S2){ 2.5f16, 3.0f16 }); printf("s2 %04x %04x\n", hb(s2.a), hb(s2.b));
  struct S3 s3 = lib_s3((struct S3){ 1, 2, 3 }); printf("s3 %04x %04x %04x\n", hb(s3.a), hb(s3.b), hb(s3.c));
  struct S5 s5 = lib_s5((struct S5){ { 1, 2, 3, 4, 5 } }); for (int i = 0; i < 5; i++) printf("%04x ", hb(s5.a[i])); printf("s5\n");
  struct S9 s9 = lib_s9((struct S9){ { 1, 2, 3, 4, 5, 6, 7, 8, 9 } }); for (int i = 0; i < 9; i++) printf("%04x ", hb(s9.a[i])); printf("s9\n");
  struct SF sf = lib_sf((struct SF){ 0.25f16, 1.5f }); printf("sf %04x %g\n", hb(sf.a), sf.b);
  struct SI si = lib_si((struct SI){ 0.25f16, 3 }); printf("si %04x %d\n", hb(si.a), si.b);
  struct SD sd = lib_sd((struct SD){ 0.75f16, 1.25 }); printf("sd %04x %g\n", hb(sd.a), sd.d);
  struct S4F s4 = lib_s4f((struct S4F){ { 1, 2, 3, 4 }, 0.5f }); printf("s4f %04x %04x %g\n", hb(s4.a[0]), hb(s4.a[3]), s4.f);
  union U u = lib_u((union U){ .a = 3.0f16 }); printf("u %04x\n", hb(u.a));
  struct S2 s = { 1.5f16, -2.25f16 };
  printf("main->va %g\n", lib_va(13, (H)0.5, 2.0, s, 3, (H)1.25, 4.0, s, 5, (H)-3.5, 6.0, s, 7, (H)9.75));
  lib_call_va(my_va);
  H arr[2] = { 1.25f16, 2.5f16 }; printf("ptr %04x\n", hb(lib_ptr(arr)));
  return 0;
}
#endif
