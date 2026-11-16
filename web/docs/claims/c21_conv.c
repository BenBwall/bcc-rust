// unsigned to int and long to int conversions that do not fit.
#include <stdio.h>
int main(void) {
    volatile unsigned big = 4000000000u;
    int i = big;          /* unsigned -> int, does not fit */
    volatile long l = 5000000000;
    int j = l;            /* long -> int */
    printf("%d %d\n", i, j);
    return 0;
}
