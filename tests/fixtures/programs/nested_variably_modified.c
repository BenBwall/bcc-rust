/* GNU nested functions using variably modified objects and types of the
   functions around them: a VLA read and written through the static chain,
   its size, a typedef's saved bound, a pointer to a VLA, a record with a
   member of variable size passed in and returned by value, a nested
   function whose own type is variably modified (its bounds evaluated at
   the definition), a bound reached two functions out, and a bound saved
   anew on each loop iteration. Prints what GCC prints. */
#include <stdio.h>
#include <string.h>
static int arrays(int n) {
  int a[n];
  typedef char T[n * 2];
  int (*p)[n] = &a;
  int fill(int base) {
    for (int i = 0; i < n; i++)
      a[i] = base + i;
    return (int)(sizeof a + sizeof(T) + sizeof *p);
  }
  int sizes = fill(10);
  int k = n;
  int last(void) { return (*p)[k - 1] + a[0]; }
  int again(void) { return (int)(sizeof a + sizeof(T) + sizeof *p); }
  n = 99; /* the bounds were saved at the declarations */
  return sizes * 10000 + again() * 100 + last();
}
static int records(int n) {
  struct S { char tag; int v[n]; short s; };
  struct S make(int k) {
    struct S r;
    memset(&r, 0, sizeof r);
    r.tag = (char)k;
    for (int i = 0; i < n; i++)
      r.v[i] = k * 10 + i;
    r.s = (short)(k + 1);
    return r;
  }
  int sum(struct S x) {
    int t = x.tag + x.s;
    for (int i = 0; i < n; i++)
      t += x.v[i];
    x.v[0] = -1; /* the caller's copy is unchanged */
    return t;
  }
  auto struct S twice(int k);
  struct S twice(int k) { return make(k * 2); }
  struct S a, b;
  a = make(3);
  b = twice(2);
  int first = sum(a) + sum(b) + a.v[0];
  return first * 100 + (int)sizeof(make(1)) + make(5).v[n - 1];
}
static int definition(void) {
  int n = 1;
  char buffer[4] = "abc";
  char (*view(void))[++n] { return (char (*)[n])buffer; }
  int before = n;
  n = 10;
  return before * 100 + (int)sizeof *view() * 10 + (*view())[1];
}
static int outer(int m) {
  int grid[m][m + 1];
  int middle(int k) {
    int inner(void) { return (int)sizeof grid[0] + grid[m - 1][m] + k; }
    grid[m - 1][m] = 7;
    return inner();
  }
  return middle(100);
}
static int loops(void) {
  int total = 0;
  for (int i = 1; i <= 4; i++) {
    int v[i];
    int size(void) { v[i - 1] = i; return (int)sizeof v + v[i - 1]; }
    total += size();
  }
  return total;
}
int main(void) {
  printf("%d %d\n", arrays(3), arrays(1));
  printf("%d %d\n", records(2), records(5));
  printf("%d\n", definition());
  printf("%d %d\n", outer(2), outer(4));
  printf("%d\n", loops());
  return 0;
}
