#include <stdio.h>

int add(int a, int b);
int sub(int a, int b);

int (*ops[])(int, int) = {add, sub};

int add(int a, int b) {
    return a + b;
}

int sub(int a, int b) {
    return a - b;
}

int main(void) {
    int i;
    for (i = 0; i < 2; i++)
        printf("%d\n", ops[i](8, 3));
    return 0;
}
