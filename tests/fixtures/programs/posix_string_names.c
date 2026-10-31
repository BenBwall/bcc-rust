/* A program may define the POSIX and BSD names outside ISO C that bcc's
   WASI runtime also provides (`index`, `bcmp`, `stpcpy`, ...): its own
   definitions win over the runtime's weak ones, as over a C library's. */
#include <stdio.h>
#include <string.h>
int index;
int bcmp = 7;
char *stpcpy(char *d, const char *s) {
    strcpy(d, s);
    return d;
}
int main(void) {
    char buffer[16] = "ab";
    index = 3;
    strcat(buffer, "cd");
    printf("%s %d %d\n", buffer, index, bcmp);
    printf("%s\n", stpcpy(buffer, "xy"));
    return 0;
}
