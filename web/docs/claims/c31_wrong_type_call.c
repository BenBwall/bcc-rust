/* A call through a pointer cast to the wrong function type (C99 6.3.2.3p8). */
#include <stdio.h>
static int twice(int x) { return 2 * x; }
int main(void) {
    double (*f)(double) = (double (*)(double))twice;
    printf("%g\n", f(21.0));
    return 0;
}
