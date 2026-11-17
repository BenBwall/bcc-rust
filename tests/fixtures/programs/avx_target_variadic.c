/* A variadic function whose `target` attribute turns AVX on, in a unit built
 * without AVX, with a named 32-byte vector parameter: GCC passes it in YMM0,
 * which LLVM keeps for functions without `...`, so bcc refuses it under
 * GCC's convention. Clang passes it in memory, as LLVM does, and returns the
 * vector in YMM0. With CALLEE the definition, otherwise the call; each half
 * links with the other compiler's build of the other. */
#include <stdarg.h>
#include <stdio.h>

typedef float V __attribute__((vector_size(32)));

__attribute__((target("avx"))) V sum_vectors(V a, int n, ...);

#ifdef CALLEE
__attribute__((target("avx"))) V sum_vectors(V a, int n, ...) {
  va_list ap;
  va_start(ap, n);
  for (int i = 0; i < n; i++) a += va_arg(ap, V);
  va_end(ap);
  return a;
}
#else
__attribute__((target("avx"))) static void run(void) {
  V a = {3, 3, 3, 3, 3, 3, 3, 3};
  V b = {4, 5, 6, 7, 8, 9, 10, 11};
  V c = {0.5f, 0.25f, 1, 2, -1, -2, -3, 100};
  V total = sum_vectors(a, 2, b, c);
  for (int i = 0; i < 8; i++) printf("%g ", total[i]);
  printf("\n");
}

int main(void) {
  __builtin_cpu_init();
  if (!__builtin_cpu_supports("avx")) {
    printf("skip\n");
    return 0;
  }
  run();
  return 0;
}
#endif
