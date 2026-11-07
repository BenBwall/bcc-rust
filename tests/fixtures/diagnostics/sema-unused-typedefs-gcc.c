typedef int G;
void f1(void) { typedef int T; }
void f2(void) { typedef int T; T x = 0; (void)x; }
void f3(void) { typedef int T; (void)sizeof(T); }
void f4(void) { typedef int T __attribute__((unused)); }
void f5(void) { typedef int T; typedef T U; }
void f6(void) { typedef struct { int a; } S; }
void f7(void) { { typedef char C; } }
void f8(int n) { typedef int A[n]; }
void f9(void) { typedef int T; (void)(T)1; }
void g1(void) { typedef int T; typedef int T; }
int g2(void) { typedef int T; return ((T *)0) == 0; }
void p1(void) {
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wunused-local-typedefs"
typedef int T;
#pragma GCC diagnostic pop
}
void p2(void) {
typedef int T;
#pragma GCC diagnostic ignored "-Wunused-local-typedefs"
}
#pragma GCC diagnostic warning "-Wunused-local-typedefs"
void p3(void) {
  { typedef int T; }
#pragma GCC diagnostic ignored "-Wunused-local-typedefs"
}
