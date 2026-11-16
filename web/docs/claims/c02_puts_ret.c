// What puts, printf and putchar return in glibc.
#include <stdio.h>
int main(void) {
    int r = puts("Hello, world!");
    int p = printf("%s has %d letters\n", "Ada", 3);
    int q = putchar('x');
    printf("\nputs=%d printf=%d putchar=%d\n", r, p, q);
    return 0;
}
