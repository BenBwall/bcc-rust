/* GNU nested functions: an enclosing function's objects read and written
   through the static chain, a trampoline passed as a function pointer, a
   nested function inside a nested function, structure and variadic
   results, recursion, and a cleanup function that is nested; a sibling
   returning a trampoline, which lives in the defining frame; captured
   structure parameters passed in registers and in memory; `auto`
   declarations; and trampolines that qsort, atexit and exit call. Prints
   what GCC prints. */
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
struct big { long a, b, c; };
static int apply(int (*f)(int), int v) { return f(v); }
static int outer(int n) {
  int total = 0;
  int add(int x) { total += x; return total; }
  int twice(int x) {
    int inner(int y) { return add(y) + n; }
    return inner(x) + apply(inner, x);
  }
  struct big make(int k) { struct big b = {k, n, total}; return b; }
  long sum(int count, ...) {
    va_list ap; long s = n; va_start(ap, count);
    for (int i = 0; i < count; i++) s += va_arg(ap, int);
    va_end(ap); return s;
  }
  int fact(int k) { return k <= 1 ? 1 : k * fact(k - 1); }
  int first = apply(add, 2);
  int second = twice(3);
  printf("%d %d %d\n", first, second, total);
  struct big b = make(7);
  long s = sum(3, 1, 2, 3);
  printf("%ld %ld %ld %ld %d\n", b.a, b.b, b.c, s, fact(5));
  return total;
}
typedef int (*fp)(int);
static __attribute__((noinline)) int clobber(int n) {
  volatile char junk[256];
  for (int i = 0; i < 256; i++) junk[i] = (char)(n + i);
  return junk[n];
}
/* The sibling's frame is gone when the trampoline it returned is called. */
static int sibling(int n) {
  int h(int x) { return x + n; }
  fp get(void) { return h; }
  fp p = get();
  clobber(7);
  return p(1);
}
struct small { int a, b; };
struct large { long a, b, c, d; };
static int captured(struct small s, struct large l) {
  int get(void) { return s.a * 10 + s.b + (int)(l.a + l.d); }
  s.b = 9;
  l.d = 100;
  return get();
}
static int forward(int n) {
  auto int even(int);
  auto int odd(int);
  int calls = 0;
  int even(int k) { calls++; return k == 0 ? 1 : odd(k - 1); }
  int odd(int k) { calls++; return k == 0 ? 0 : even(k - 1); }
  int (*pe)(int) = even;
  return even(n) * 100 + pe(n + 1) * 10 + calls;
}
static int sorted(void) {
  int compares = 0;
  int by_value(const void *a, const void *b) {
    compares++;
    return *(const int *)a - *(const int *)b;
  }
  int values[5] = {4, 1, 3, 5, 2};
  qsort(values, 5, sizeof values[0], by_value);
  return values[0] * 10000 + values[4] * 1000 + (compares > 0);
}
static void report(int *p) { printf("cleanup %d\n", *p); }
int main(void) {
  int seen = 0;
  void note(int *p) { seen = *p; report(p); }
  { int x __attribute__((cleanup(note))) = 42; }
  int result = outer(10);
  printf("%d %d\n", result, seen);
  int fwd = forward(7);
  printf("%d %d %d %d\n", sibling(41), captured((struct small){1, 2}, (struct large){3, 0, 0, 4}), fwd, sorted());
  void goodbye(void) { printf("exit %d\n", seen); }
  atexit(goodbye);
  exit(0);
}
