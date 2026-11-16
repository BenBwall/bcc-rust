// (int)3e9 with a value the optimizer can see.
#include <stdio.h>
static int to_int(double d) { return (int)d; }
int main(void) {
    printf("(int)3e9 folded = %d\n", to_int(3e9));
    return 0;
}
