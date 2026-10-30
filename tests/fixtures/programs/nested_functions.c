/* GNU nested functions: an enclosing function's objects read and written
   through the static chain, a trampoline passed as a function pointer, a
   nested function inside a nested function, structure and variadic
   results, recursion, and a cleanup function that is nested. Prints what
   GCC prints. */
#include <stdarg.h>
#include <stdio.h>
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
static void report(int *p) { printf("cleanup %d\n", *p); }
int main(void) {
  int seen = 0;
  void note(int *p) { seen = *p; report(p); }
  { int x __attribute__((cleanup(note))) = 42; }
  int result = outer(10);
  printf("%d %d\n", result, seen);
  return 0;
}
