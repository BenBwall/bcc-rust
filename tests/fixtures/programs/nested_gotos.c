/* GNU non-local gotos: a nested function jumps to a `__label__` of a
   function around it, which resumes with its objects as the nested
   function left them and its stack as it was at the call. No cleanup runs
   on the way out, in either function. Covers recursion inside the nested
   function, `alloca` storage and variable length arrays that stay live,
   several labels, a jump two functions out, a jump to an outer activation
   of a recursive definer, calls in tail position, and repeated jumps,
   which must not grow the stack. Prints what GCC prints. */
#include <alloca.h>
#include <stdio.h>
#include <stdlib.h>
static void report(int *p) { printf("cleanup %d\n", *p); }
static int unwind(int n) {
  __label__ out;
  int v = 1;
  char *a = alloca(64);
  a[0] = 'x';
  int dive(int k) {
    int c __attribute__((cleanup(report))) = k;
    if (k == 0) {
      v = 42;
      goto out;
    }
    return dive(k - 1) + 1;
  }
  {
    int d __attribute__((cleanup(report))) = -1;
    printf("not reached %d\n", dive(n));
  }
  return -1;
out:;
  char *b = alloca(64);
  b[0] = 'y';
  printf("unwind %d %c %c\n", v, a[0], b[0]);
  return v;
}
static int arrays(int n) {
  __label__ l;
  int vla[n];
  for (int i = 0; i < n; i++)
    vla[i] = i * 3;
  int s = 0;
  void leave(void) { s = 7; goto l; }
  for (int i = 0; i < 3; i++) {
    int inner[n + i];
    inner[0] = 99;
    if (i == 2)
      leave();
  }
  return -1;
l:
  for (int i = 0; i < n; i++)
    s += vla[i];
  return s;
}
static int tail(int n) {
  __label__ done;
  int down(int k) {
    if (k == 0)
      goto done;
    return down(k - 1);
  }
  return down(n);
done:
  return 100 + n;
}
static int labels(int which) {
  __label__ a, b, c;
  int t = 1;
  void go(int w) {
    t *= 10;
    if (w == 0)
      goto a;
    if (w == 1)
      goto b;
    goto c;
  }
  t = 2;
  go(which);
  return -1;
a:
  return t + 1;
b:
  return t + 2;
c:
  return t + 3;
}
static int two(int far) {
  __label__ fout;
  int fv = 0;
  int middle(void) {
    __label__ gout;
    int gv = 0;
    void inner(void) {
      fv = 5;
      gv = 6;
      if (far)
        goto fout;
      goto gout;
    }
    inner();
    return -2;
  gout:
    return gv * 10 + fv;
  }
  return middle();
fout:
  return 1000 + fv;
}
static int activation(int n, void (*k)(void)) {
  __label__ back;
  void jump(void) { goto back; }
  if (n == 0) {
    k();
    return -3;
  }
  return activation(n - 1, n == 4 ? jump : k);
back:
  return n;
}
static int values(int n) {
  __label__ l;
  int plain = n, captured = 0;
  void j(void) {
    captured = plain * 2;
    goto l;
  }
  plain = n + 1;
  if (n)
    j();
  plain = -1;
l:
  return plain * 1000 + captured;
}
static int repeated(void) {
  int s = 0;
  for (int i = 0; i < 20000; i++) {
    __label__ next;
    int scratch[i % 7 + 1];
    scratch[0] = i;
    void skip(void) { goto next; }
    if (i & 1)
      skip();
    s += scratch[0] % 3;
  next:;
  }
  return s;
}
int main(void) {
  printf("%d\n", unwind(3));
  printf("%d %d\n", arrays(5), arrays(4));
  printf("%d %d\n", tail(5), tail(0));
  printf("%d %d %d\n", labels(0), labels(1), labels(2));
  printf("%d %d\n", two(0), two(1));
  printf("%d\n", activation(6, 0));
  printf("%d %d\n", values(3), values(0));
  printf("%d\n", repeated());
  return 0;
}
