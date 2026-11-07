int printf(const char *, ...);
int scanf(const char *, ...);
void log_line(int level, const char *fmt, ...) __attribute__((format(printf, 2, 3)));
void bad_index(const char *fmt, ...) __attribute__((format(printf, 3, 4)));
void bad_string(int fmt, ...) __attribute__((format(printf, 1, 2)));
void f(long l, int i, char *s, int c, int *ip, const int *cip) {
    printf("%d\n", l);
    printf("%d %s\n", i);
    printf("%d\n", i, i);
    printf("%y\n", i);
    printf("%*d\n", l, i);
    printf("%hhd %ls %p\n", l, s, i);
    printf(s);
    printf(s, i);
    printf("");
    printf("a\0b");
    printf("a" "%d\n", l);
    printf(c ? "%d\n" : "%s\n", l);
    printf("%-05d\n", i);
    scanf("%d %s", l, ip);
    scanf("%d", cip);
    log_line(1, "%s\n", i);
}
