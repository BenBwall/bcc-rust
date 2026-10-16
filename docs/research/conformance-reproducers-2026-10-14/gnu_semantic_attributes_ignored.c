/* GNU attributes that change program behavior are accepted without a
   diagnostic and have no effect (both targets, every arm).
   Clang and GCC print "21 12" then "bye 7"; bcc prints "0 0" and no "bye". */
#include <stdio.h>
static int order = 0, released = 0;
__attribute__((constructor)) static void early(void) { order = order * 10 + 1; }
__attribute__((constructor(200))) static void early2(void) { order = order * 10 + 2; }
__attribute__((destructor)) static void late(void) { printf("bye %d\n", order); }
static void release(int *p) { released += *p; }
int main(void) {
    { int a __attribute__((cleanup(release))) = 7; }
    { int b __attribute__((cleanup(release))) = 5; }
    printf("%d %d\n", order, released);
    order = 7;
    return 0;
}
