#include <stdio.h>
int position = 0;
int next(void) {
    const char *text = "abc";
    return text[position++];
}
int main(void) {
    int c;
    int n = 0;
    while ((c = next()) != 0) {
        putchar(c);
        n++;
    }
    putchar('\n');
    return n;
}
