// (int)3e9 at run time, and truncation toward zero.
#include <stdio.h>
int main(void) {
    volatile double big = 3e9;
    double neg = -2.99, pos = 2.99;
    printf("(int)3e9 at run time = %d\n", (int)big);
    printf("(int)2.99 = %d, (int)-2.99 = %d\n", (int)pos, (int)neg);
    return 0;
}
