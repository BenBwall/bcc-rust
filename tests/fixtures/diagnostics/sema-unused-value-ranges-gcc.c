/* GCC places each tree at a caret with a range around it, which it draws
   only when both ends are in the caret's file and expansion. */
#include <stdbool.h>
#define T long
#define CAST(x) ((long)(x))
#define C(x) (long)x
#define ADD(x, y) x + y
#define ONE 1
int h(void);
struct S { int m; } s, *ps;
void f(int a, int b, int *p) {
  (_Bool) a;
  (T) a;
  CAST(a);
  (bool) a;
  C(a);
  ADD(a, b);
  a + ONE;
  ONE + a;
  a +
    b;
  (T)
    a;
  a ? b + 1 : b;
  a ? b
    : a;
  s.m;
  ps->m;
  p[1];
  1[p];
  -a;
  &a;
  "str";
  sizeof a;
  (a);
  (a + 1);
  0 ? 1 : 2;
  (int) a;
  (int) (a + 1);
  (void)0, a + 1;
  (a, b);
  a, 1;
  (int)(*p = 2, a);
  (h(), a + 1);
  ((long) (h() + 1));
  ((long) h()) + 1;
  (char) h();
  a, h() + 1;
  h(), a + 1;
  a && h() == 1;
  a && !h();
  a && (b = h());
  a || (b = 1);
}
