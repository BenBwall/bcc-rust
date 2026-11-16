int printf(const char *, ...);
int scanf(const char *, ...);
#define FORMAT "%s\n"
void f(int i, long l, char *s, _Bool b, double d) {
    printf("%5");
    printf("%0$d\n", i);
    printf("%1$*0$d\n", i, i);
    printf("%1$*d\n", i, i);
    printf("%*.*d\n", i);
    printf("%.*d\n", l, i);
    printf("%1$d %d\n", i, i);
    printf("%+ d\n", i);
    printf("%#d\n", i);
    printf("%.3c\n", i);
    printf("%Lf %qd\n", d, l);
    printf("%c\n", i < 2);
    printf("%c\n", b);
    printf("%d %s\n",
           l, s);
    printf("%d\n", (l + 1));
    printf(FORMAT, i);
    printf("%\xff" "d\n", i);
    scanf("%[abc", s);
    scanf("%5d", &l);
}
