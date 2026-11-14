// Your first program: text goes out through printf and puts, and a string
// is an array of chars that ends with the null character '\0'.
#include <stdio.h>

int length(const char *text);

int main(void) {
    puts("Hello, world!");
    char name[] = "Ada";
    int len = length(name);
    printf("%s has %d letters\n", name, len);
    return 0;
}

int length(const char *text) {
    int count = 0;
    while (text[count] != '\0') {
        count++;
    }
    return count;
}
