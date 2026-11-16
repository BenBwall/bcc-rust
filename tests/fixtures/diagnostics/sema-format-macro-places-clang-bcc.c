int printf(const char *, ...);
#define FMT "%d"
#define WIDTH "%*"
#define END "d"
void f(long l, int a, char *s) {
    printf(WIDTH END, l, a);
    printf(FMT " %s", l, a);
    printf("%s " FMT, a, l);
    printf("%s " FMT " %s", s, l, a);
    printf("%" END, l);
}
