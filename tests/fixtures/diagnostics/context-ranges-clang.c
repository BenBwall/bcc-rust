/* Clang underlines the operands a diagnostic names beside its caret: an
   initializer's or an assignment's value, and both operands of a binary
   operator. */
struct S { int a; } s;
int f(int y, int *p, long *lp) {
  int x = s + y;
  int *q = y + 1;
  p = y;
  p = lp;
  x = p * y;
  x = s.a < p;
  return x + *q;
}
