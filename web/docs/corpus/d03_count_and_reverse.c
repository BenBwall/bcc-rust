#include <stdio.h>
int main(void) {
    char s[] = "abcd";
    char *end = s;
    while (*end)
        end++;
    printf("%d\n", (int)(end - s));
    while (end > s)
        putchar(*--end);
    putchar('\n');
    return 0;
}
