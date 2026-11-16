// Integer division by a zero the optimizer can see.
#include <stdio.h>
static int divide(int a, int b) { return a / b; }
int main(void) {
    printf("start\n");
    fflush(stdout);
    return divide(1, 0);
}
