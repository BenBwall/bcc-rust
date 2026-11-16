// Exact values of doubles, and what printf writes for them.
#include <stdio.h>
int main(void) {
    printf("%.55f\n%.17g\n%.17g\n", 0.1, 0.1 + 0.2, 0.3);
    double tenth = 0.0;
    for (int i = 0; i < 10; i++) tenth += 0.1;
    printf("%.17g %d\n", tenth, 0.1 + 0.2 == 0.3);
    printf("%f %f %f\n", 1.0/0.0, -1.0/0.0, 0.0/0.0);
    return 0;
}
