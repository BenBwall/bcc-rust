int g;
int h(void);
typedef int T;
struct S { int g; };
enum E { EA, EB };
int f1(int g) { return g; }
int f2(void) { int g = 1; return g; }
int f3(int a) { { int a = 2; return a; } }
int f4(int a) { int b = a; { int b = 3; (void)b; } return b; }
int f5(void) { int h = 1; return h; }
int f6(void) { int T = 1; return T; }
int f7(void) { int EA = 1; return EA; }
int f8(int x) { for (int x = 0; x < 3; x++) ; return x; }
int f9(void) { int i = 0; for (int i = 0; i < 3; i++) ; return i; }
void f10(void) { extern int g; (void)g; }
int f11(void) { static int g = 1; return g; }
void f12(int n) { int a[n]; { int n = 2; (void)n; } (void)a; }
int f13(void) { int s; { struct S s2; (void)s2; } int S = 1; return S + s; }
void f14(void) { int x; { int x; (void)x; } (void)x; { void x(void); } }
int f15(int p) { int q = ({ int p = 1; p; }); return q; }
void f16(void) { int (*fp)(int g) = 0; (void)fp; }
