int printf(const char *, ...);
int scanf(const char *, ...);
enum { FMT = 1, FIRST = 2 };
/* Operand numbers and amounts past every integer type. */
void numbers(int *ip) {
    printf("%999999999999999999999999999999d", 1);
    printf("%.999999999999999999999999999999d", 1);
    printf("%999999999999999999999999999999$d", 1);
    printf("%1$*999999999999999999999999999999$d", 1, 2);
    scanf("%999999999999999999999999999999d", ip);
    printf("%4294967297$d", 1);
    printf("%2147483648$d", 1);
}
/* A UTF-8 string literal is a format too. */
void utf8(long l) {
    printf(u8"%d\n", l);
    printf(u8"%y");
}
/* The format a selection picks. */
void selected(long l, char *s) {
    printf(__builtin_choose_expr(0, s, "%d\n"), l);
    printf(_Generic(1L, int: s, long: "%d\n"), l);
    printf(__builtin_choose_expr(1, s, "x"));
}
/* Offsets folded into the string's address. */
void offsets(long l) {
    printf("ab%d" - 0, l);
    printf(("abc%d" + 3) - 1, l);
    printf(&"x%d"[1] - 1, l);
    printf("ab" - 0, l);
}
/* Function pointers and members with a format. */
extern int (*logger)(const char *, ...) __attribute__((format(printf, 1, 2)));
struct ops { int (*say)(const char *, ...) __attribute__((format(printf, 1, 2))); } ops;
int (*plain)(const char *, ...);
void pointers(int (*lp)(const char *, ...) __attribute__((format(printf, 1, 2)))) {
    logger("%d", 1L);
    ops.say("%d", 1L);
    lp("%d", 1L);
    (*logger)("%d", 1L);
    plain("%d", 1L);
}
/* Attribute operands that are integer constant expressions. */
void sum(const char *, ...) __attribute__((format(printf, 1 + 0, 2)));
void named(const char *, ...) __attribute__((format(printf, FMT, FIRST)));
void chosen(int, const char *, ...) __attribute__((format(printf, (2), 1 ? 3 : 4)));
void indices(void) {
    sum("%d", 1L);
    named("%d", 1L);
    chosen(0, "%d", 1L);
}
/* A constant array shorter than its string. */
const char cut[2] = "%d";
const char kept[3] = "%d";
void extents(void) {
    printf(cut, 1);
    printf(kept, 1L);
}
/* A parameter's argument converts to the parameter's type. */
void log_long(const char *, long) __attribute__((format(printf, 1, 2)));
void log_char(const char *, char) __attribute__((format(printf, 1, 2)));
void fixed(int *ip) {
    log_long("%ld", 1);
    log_long("%d", 1);
    log_char("%c", 1);
    log_char("%s", 1);
}
