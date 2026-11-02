/* GCC 14 in GNU89: only some of the permerrors warn, the rest are silent. */
static x = 1;
g(void) { return x; }
int h(register q) { return q; }
int old(a, b) int a; { return a + b; }
void listed(n);
int nothing(void) { return; }
void value(void) { return 1; }
void takep(int *p);
int *convert(int i, int *p, long *lp, int (*fp)(double)) {
  int j = p;
  int *k = i;
  p = i;
  lp = p;
  fp = h;
  takep(i);
  undeclared(j, k);
  printf("%d", j);
  return i;
}
