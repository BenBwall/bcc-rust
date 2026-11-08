int printf(const char *, ...);
int scanf(const char *, ...);
void gnu(const char *, ...) __attribute__((format(gnu_printf, 1, 2)));
void plain(const char *, ...) __attribute__((format(printf, 1, 2)));
typedef __SIZE_TYPE__ size_t;
typedef unsigned short wchar_t;
void f(long l, long long ll, int i, unsigned long long u, size_t z, char *s, wchar_t *ws,
       short sh, int *ip, long *lp, long long *llp, double d, long double ld) {
    printf("%I64d %I64u %I32d %Id\n", ll, u, i, ll);
    printf("%I64x %I32x %Iu\n", i, lp, i);
    printf("%lld %zu %td %jd\n", ll, z, z, ll);
    printf("%hhd %ld %d\n", i, l, l);
    printf("%ls %S %hs %ws\n", ws, ws, s, ws);
    printf("%lc %C %hc\n", i, i, i);
    printf("%m %Lf %Lg %Ld\n", ld, ld, ll);
    printf("%Z %F %'d\n", s, d, i);
    printf("%s\n", (char *)0);
    gnu("%zu %I64d\n", u, ll);
    plain("%zu\n", u);
    scanf("%0d %I64d %ms %Ld\n", ip, llp, &s, llp);
    scanf("%*Ld %hs %hc %S %C\n", s, s, s, s);
    scanf("%I64d %lld %hhd %zu\n", llp, llp, ip, ip);
}
