/* A call through a pointer cast to a function type passed in the same
   registers (C99 6.5.2.2p9): still undefined, though it seems to work. */
#include <stdio.h>
static int twice(int x) { return 2 * x; }
int main(void) {
    unsigned (*f)(unsigned) = (unsigned (*)(unsigned))twice;
    printf("%u\n", f(21));
    return 0;
}
