#include <stdio.h>
int main(void) {
    char source[] = "copy";
    char target[8];
    char *d = target;
    const char *s = source;
    while ((*d++ = *s++) != 0)
        ;
    printf("%s %d\n", target, (int)(d - target));
    return 0;
}
