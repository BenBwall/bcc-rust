/* A 64-byte vector, aligned to 64 bytes, passed through `...` after a
 * variable length array moved the stack: the caller realigns the outgoing
 * arguments at the call (CALLER_ONLY), the callee reads them where the
 * System V convention puts them (CALLEE_ONLY, built by GCC). */
#include <stdarg.h>
#include <stdio.h>

typedef int v16si __attribute__((vector_size(64)));
void take(int n, ...);

#ifndef CALLER_ONLY
void take(int n, ...) {
  va_list ap;
  va_start(ap, n);
  v16si c = va_arg(ap, v16si);
  printf("%d %d %d\n", n, c[0], c[15]);
  va_end(ap);
}
#endif

#ifndef CALLEE_ONLY
void mid(int n) {
  char vla[n];
  vla[0] = (char)n;
  v16si v = {1};
  v[15] = 16 * n;
  take(vla[0], v);
}
int main(void) {
  mid(3);
  mid(9);
  mid(17);
  return 0;
}
#endif
