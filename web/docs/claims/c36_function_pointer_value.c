/* Is a pointer to a function an address in compiled code? */
#include <stdio.h>
#include <stdint.h>
static int twice(int x) { return 2 * x; }
int main(void) {
    int (*f)(int) = twice;
    printf("%lu\n", (unsigned long)(uintptr_t)f);
    return f(3) == 6 ? 0 : 1;
}
