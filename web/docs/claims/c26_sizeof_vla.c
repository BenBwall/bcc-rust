// sizeof evaluates a variable length array operand (C99 §6.5.3.4p2): n++
// runs, so this prints 8 3. The Sizeof note says so.
#include <stdio.h>
int main(void) {
    int n = 2;
    size_t s = sizeof(int[n++]);
    printf("%zu %d\n", s, n);
    return 0;
}
