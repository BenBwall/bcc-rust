/* Thread storage duration. `__thread` (GNU spelling) is "unknown type name";
   with `_Thread_local` alone every arm reports "construct not supported by
   code generation: its type was not analyzed". Clang and GCC print "11"
   (Linux target; the bundled Clang's MinGW program itself faults). */
#include <stdio.h>
static _Thread_local int tl = 5;
static __thread int t2 = 6;
int main(void) { tl += t2; printf("%d\n", tl); return 0; }
