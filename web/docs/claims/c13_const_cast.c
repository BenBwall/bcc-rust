// A store through a pointer whose const a cast removed.
#include <stdio.h>
int main(void) {
    const int step = 3;
    int *p = (int *)&step;
    *p = 4;
    printf("step %d *p %d\n", step, *p);
    return 0;
}
