int a[]={1,2,3};
int f(const int *p, int n) { int x=n+1; x+=p[0]; return x?x:sizeof a; }
int g(double d, int *q) { return !d + !0.5 + !q; }
