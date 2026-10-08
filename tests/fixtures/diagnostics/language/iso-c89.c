enum E { A, };
struct S { int n; int tail[]; };
int f(void) { int x=0; x++; int y=1; for(int i=0;i<2;i++) x+=i; return ((struct S){.n=y}).n; }
