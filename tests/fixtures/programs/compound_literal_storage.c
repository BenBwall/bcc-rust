/* C23 storage-class specifiers in compound literals: a `static` or
   `thread_local` literal in a function is created once and keeps its value
   between evaluations; a `register` one is an ordinary value. The bundled
   Clang parses no such literal. */
#include <stdio.h>

#ifndef __clang__
int g = 5;
struct P { int x, y; int *gp; };
static int *counter(void) { return &(static int){ 100 }; }
static struct P *point(void) { return &(static struct P){ 1, 2, &g }; }
static int *tls(void) { return (static thread_local int[2]){ 7, 8 }; }
static const char *name(void) { return (static const char[]){ "abc" }; }
int *ps = &(static int){ 1 };
int *psa = (static int[3]){ 1, 2, 3 };
static int calls(int n) {
    int *p = &(static int){ 0 };
    *p += n;
    return *p;
}
static int run(void) {
    for (int i = 0; i < 3; i++) (*counter())++;
    printf("%d\n", *counter());
    point()->x += 10;
    printf("%d %d %d\n", point()->x, point()->y, *point()->gp);
    tls()[1] += 1;
    printf("%d %d %s\n", tls()[0], tls()[1], name());
    printf("%d %d %d\n", *ps, psa[2], (register int){ 3 } + (register struct P){ 4, 5, 0 }.y);
    calls(2); calls(3);
    printf("%d %d\n", calls(4), counter() == counter());
    int *a = &(static int){ 1 }, *b = &(static int){ 1 };
    printf("%d\n", a != b);
    printf("%d\n", (constexpr static int){ 9 } + (static constexpr int){ 1 });
    return 0;
}
#endif

int main(void) {
#ifndef __clang__
    return run();
#else
    return 0;
#endif
}
