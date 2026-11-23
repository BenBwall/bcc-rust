// A callback: sum_of calls back
// whichever function main passes.
#include <stdio.h>

int twice(int x) {
    return 2 * x;
}

int square(int x) {
    return x * x;
}

// f points to a function that takes
// an int and returns an int.
int sum_of(const int *v, int n,
           int (*f)(int)) {
    int total = 0;
    // A call through the pointer f:
    for (int i = 0; i < n; i++)
        total += f(v[i]);
    return total;
}

int main(void) {
    int v[2] = {3, 4};
    // A function's name becomes a
    // pointer to the function:
    int a = sum_of(v, 2, twice);
    int b = sum_of(v, 2, square);
    printf("%d %d\n", a, b);
    return 0;
}
