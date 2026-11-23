#include <stdio.h>

int check(int value) {
    if (value < 0) {
        goto fail;
    }
    printf("ok %d\n", value);
    return 0;
fail:
    printf("bad %d\n", value);
    return 1;
}

int main(void) {
    int first = check(3);
    int second = check(-2);
    return first + second * 2;
}
