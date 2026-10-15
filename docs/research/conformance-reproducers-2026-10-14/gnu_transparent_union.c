/* GNU transparent_union: bcc refuses the call ("function argument cannot be
   converted to its parameter type"); Clang and GCC print "9 1". */
#include <stdio.h>
typedef union { int *ip; const long *lp; } __attribute__((transparent_union)) arg_t;
static int take(arg_t a) { return *a.ip; }
typedef int __attribute__((may_alias)) aliasing_int;
int main(void) { int v = 9; float f = 1.0f; aliasing_int *p = (aliasing_int *)&f; printf("%d %d\n", take(&v), *p == 0x3f800000); return 0; }
