// A char is a small integer; char + char is an int.
#include <stdio.h>
int main(void) {
    char c = 'A';
    printf("%d %c %zu\n", c, c + 1, sizeof(c + c));
    return 0;
}
