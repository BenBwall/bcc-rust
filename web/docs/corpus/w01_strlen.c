#include <stdio.h>
int length(const char *s) {
    const char *start = s;
    while (*s++)
        ;
    return s - start - 1;
}
int main(void) {
    printf("%d\n", length("hello"));
    return length("");
}
