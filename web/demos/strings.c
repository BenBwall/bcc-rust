// A pointer walks a string: *p++
// reads a char, then moves p on.
#include <stdio.h>

int length(const char *s) {
    const char *start = s;
    // Stop at the null character:
    while (*s)
        s++;
    // Elements apart, not bytes:
    return s - start;
}

int main(void) {
    char word[8];
    char *to = word;
    const char *from = "walk";
    // Copy up to and with the '\0':
    while ((*to++ = *from++) != '\0')
        ;
    printf("%s has %d chars\n", word,
           length(word));
    return 0;
}
