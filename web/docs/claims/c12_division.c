// Integer division and remainder truncate toward zero.
#include <stdio.h>
int main(void) {
    volatile int a = -7, b = 2;
    printf("%d %d\n", a / b, a % b);
    return 0;
}
