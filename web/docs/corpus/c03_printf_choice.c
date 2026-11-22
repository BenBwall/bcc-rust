#include <stdio.h>

int main(void) {
    int count = 1;
    printf("%d item%s\n", count, count == 1 ? "" : "s");
    return 0;
}
