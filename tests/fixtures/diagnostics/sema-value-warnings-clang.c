struct S { int a; int b[4]; };
int *a1(void) { int x; return &x; }
int *a2(void) { int x[3]; return x; }
int *a3(void) { int x[3]; return &x[1]; }
int *a4(void) { static int x; return &x; }
int *a5(int p) { return &p; }
int *a6(void) { struct S s; return &s.a; }
int *a7(void) { struct S s; return s.b; }
int *a8(void) { int x; int *p = &x; return p; }
char *a9(void) { char b[4]; return b + 1; }
int *b1(void) { int x; return (int *)&x; }
int *b2(int c) { int x, y; return c ? &x : &y; }
const char *b3(void) { return "lit"; }
void *b5(void) { int x; return (void *)&x; }
int *b6(void) { register int x; return 0; }
int *b7(void) { int x; return &*&x; }
long b8(void) { int x; return (long)&x; }
char c1 = 1000;
unsigned char c2 = 256;
short c3 = 70000;
signed char c5 = -129;
unsigned c6 = -1;
char c7 = 255;
_Bool c8 = 2;
int c9 = 2147483648;
void d(char c) { c = 300; char e = 128; unsigned char f = -1; (void)e; (void)f; }
char d2(void) { return 1000; }
void d3(char);
void d4(void) { d3(1000); }
_Bool e1(void) { int x; return (_Bool)&x; }
_Bool e2(void) { int x; return &x; }
long e3(void) { int x; return (long)&x - (long)&x; }
long e4(void) { int x; return (long)&x + 1; }
int e5(void) { int x; return (int)(long)&x; }
char *e6(void) { char x; return (char *)(&x - &x); }
char *e7(void) { char x; return (char *)(long)&x; }
char *e8(void) { char x; return (char *)((long)&x - (long)&x); }
char *e9(void) { char x; return (char *)((long)&x + 1); }
char *e10(void) { char x; return (char *)(1 + (long)&x); }
char *e11(void) { char x; return (char *)(_Bool)&x; }
