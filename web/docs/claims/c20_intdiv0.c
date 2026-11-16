// Integer division by zero at run time.
#include <stdio.h>
int divide(int a, int b) { return a / b; }
int main(void) {
    volatile int zero = 0;
    printf("start\n");
    fflush(stdout);
    return divide(1, zero);
}
