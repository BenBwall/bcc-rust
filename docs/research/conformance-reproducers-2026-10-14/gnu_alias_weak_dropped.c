/* `alias` produces no symbol (the link fails: undefined symbol alias_fn) and
   `weak` is dropped (weak_fn gets ordinary external linkage in --emit llvm).
   Clang and GCC print "12 5". Both targets. */
#include <stdio.h>
int real_fn(int x) { return x * 3; }
int alias_fn(int) __attribute__((alias("real_fn")));
__attribute__((weak)) int weak_fn(int x) { return x + 1; }
int main(void) { printf("%d %d\n", alias_fn(4), weak_fn(4)); return 0; }
