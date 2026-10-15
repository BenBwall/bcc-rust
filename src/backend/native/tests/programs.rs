//! C programs for the end-to-end tests. Each prints what it computes and
//! returns a status; the interpreter's run of the same post-ABI module is
//! the expected behaviour.

/// A named C program.
pub(super) struct Program {
    pub(super) name:   &'static str,
    pub(super) source: &'static str,
}

/// Programs the native back end must compile under both conventions.
pub(super) const PROGRAMS: &[Program] = &[
    Program {
        name:   "widths",
        source: r#"int printf(const char *, ...);
signed char sc(signed char a, signed char b) { return a * b + 3; }
unsigned char uc(unsigned char a, unsigned char b) { return a * b + 3; }
short ss(short a, short b) { return a - b * 7; }
unsigned short us(unsigned short a, unsigned short b) { return a - b * 7; }
long long ll(long long a, long long b) { return a * b - (a >> 3); }
unsigned long long ull(unsigned long long a, unsigned long long b) { return a * b + (a >> 3); }
int main(void) {
    printf("%d %d %d %d\n", sc(100, 7), uc(100, 7), ss(-30000, 9), us(3, 9));
    printf("%lld %llu\n", ll(-123456789012345LL, 77), ull(18446744073709551000ULL, 3));
    signed char x = -128; unsigned char y = 255;
    printf("%d %d %d\n", x / -1 == 128 ? 1 : 0, (signed char)(x - 1), (unsigned char)(y + 1));
    return 0;
}"#,
    },
    Program {
        name:   "division_and_shifts",
        source: r#"int printf(const char *, ...);
int sdiv(int a, int b) { return a / b; }
int srem(int a, int b) { return a % b; }
unsigned udiv(unsigned a, unsigned b) { return a / b; }
unsigned urem(unsigned a, unsigned b) { return a % b; }
long long sdiv64(long long a, long long b) { return a / b + a % b; }
signed char sdiv8(signed char a, signed char b) { return a / b; }
unsigned short urem16(unsigned short a, unsigned short b) { return a % b; }
int shifts(int x, int n) { return (int)((unsigned)x << n) ^ (x >> n) ^ (int)((unsigned)x >> n); }
long long shifts64(long long x, int n) { return (long long)((unsigned long long)x << n) + (x >> n) + (long long)((unsigned long long)x >> n); }
int narrow(signed char c, unsigned char u, int n) { return (c >> n) + (u >> n) + (signed char)(u << n); }
int main(void) {
    printf("%d %d %u %u\n", sdiv(-17, 5), srem(-17, 5), udiv(4000000000u, 7), urem(4000000000u, 7));
    printf("%lld %d %d\n", sdiv64(-1000000000000LL, 333), sdiv8(-100, 7), urem16(65000, 1000));
    for (int n = 0; n < 32; n += 5) printf("%d ", shifts(-123456, n));
    printf("\n");
    for (int n = 0; n < 64; n += 9) printf("%lld ", shifts64(-123456789012LL, n));
    printf("\n%d %d\n", narrow(-100, 200, 3), narrow(127, 255, 1));
    return 0;
}"#,
    },
    Program {
        name:   "comparisons",
        source: r#"int printf(const char *, ...);
int cmp(int a, int b) {
    return (a < b) | (a <= b) << 1 | (a > b) << 2 | (a >= b) << 3 | (a == b) << 4 | (a != b) << 5;
}
int ucmp(unsigned a, unsigned b) {
    return (a < b) | (a <= b) << 1 | (a > b) << 2 | (a >= b) << 3 | (a == b) << 4 | (a != b) << 5;
}
int cmp8(signed char a, signed char b, unsigned char c, unsigned char d) { return (a < b) + 2 * (c < d); }
int cmp64(long long a, long long b, unsigned long long c, unsigned long long d) { return (a < b) + 2 * (c < d); }
int ptrs(int *p, int *q) { return (p < q) + 2 * (p == q); }
int main(void) {
    int values[] = {-5, 0, 3, 3, 2147483647, -2147483647 - 1};
    for (int i = 0; i < 6; i++)
        for (int j = 0; j < 6; j++) printf("%d,%d ", cmp(values[i], values[j]), ucmp(values[i], values[j]));
    printf("\n%d %d %d %d\n", cmp8(-1, 1, 255, 1), cmp8(1, -1, 1, 255), cmp64(-1, 1, -1, 1), cmp64(5, 5, 5, 6));
    int a[2];
    printf("%d %d\n", ptrs(&a[0], &a[1]), ptrs(&a[1], &a[1]));
    return 0;
}"#,
    },
    Program {
        name:   "loops_and_recursion",
        source: r#"int printf(const char *, ...);
int fib(int n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
int ack(int m, int n) { return m == 0 ? n + 1 : n == 0 ? ack(m - 1, 1) : ack(m - 1, ack(m, n - 1)); }
static int odd(int n);
static int even(int n) { return n == 0 ? 1 : odd(n - 1); }
static int odd(int n) { return n == 0 ? 0 : even(n - 1); }
int main(void) {
    int sum = 0, i = 0;
    do { sum += i * i; i++; } while (i < 100);
    int primes = 0;
    for (int n = 2; n < 500; n++) {
        int prime = 1;
        for (int d = 2; d * d <= n; d++) if (n % d == 0) { prime = 0; break; }
        if (!prime) continue;
        primes++;
    }
    unsigned collatz = 27, steps = 0;
    while (collatz != 1) { collatz = collatz & 1 ? 3 * collatz + 1 : collatz / 2; steps++; }
    printf("%d %d %d %d %u %d %d\n", fib(22), ack(2, 3), sum, primes, steps, even(37), odd(37));
    return fib(10) - 55;
}"#,
    },
    Program {
        name:   "many_arguments",
        source: r#"int printf(const char *, ...);
long mix(int a, long long b, char c, short d, unsigned char e, int f, long long g, int h, signed char i, long long j) {
    return a + b * 2 + c * 3 + d * 4 + e * 5 + f * 6 + g * 7 + h * 8 + i * 9 + j * 10;
}
long long chain(long long a, long long b, long long c, long long d, long long e, long long f, long long g, long long h) {
    return a - b + c - d + e - f + g - h;
}
int *pick(int *a, int *b, int *c, int *d, int *e, int *f, int which) {
    int *all[6] = {a, b, c, d, e, f};
    return all[which];
}
int main(void) {
    int x[6] = {1, 2, 3, 4, 5, 6};
    long r = mix(1, 2, -3, -4, 250, 6, -7, 8, -9, 10);
    long long s = chain(1, 20, 300, 4000, 50000, 600000, 7000000, 80000000);
    printf("%ld %lld %d %d\n", r, s, *pick(x, x + 1, x + 2, x + 3, x + 4, x + 5, 4), *pick(x, x + 1, x + 2, x + 3, x + 4, x + 5, 5));
    printf("%d %d %d %d %d %d %d %d %d %d\n", 1, 2, 3, 4, 5, 6, 7, 8, 9, 10);
    return 0;
}"#,
    },
    Program {
        name:   "narrow_results",
        source: r#"int printf(const char *, ...);
signed char sc(int x) { return x; }
unsigned char uc(int x) { return x; }
short ss(int x) { return x; }
unsigned short us(int x) { return x; }
_Bool truth(int x) { return x; }
int take(signed char a, unsigned char b, short c, unsigned short d, _Bool e) { return a + b + c + d + e; }
int main(void) {
    printf("%d %d %d %d %d\n", sc(200), uc(-1), ss(70000), us(-2), truth(42));
    printf("%d\n", take(sc(-56), uc(300), ss(-40000), us(-3), truth(0)));
    return 0;
}"#,
    },
    Program {
        name:   "globals",
        source: r#"int printf(const char *, ...);
int counter = 5;
static long long big = 0x123456789abcdefLL;
int table[5] = {10, 20, 30, 40, 50};
int *cursor = &table[2];
const char message[] = "hello";
const char *messages[] = {message, "world"};
static int zeroed[100];
static int square(int x) { return x * x; }
static int cube(int x) { return x * x * x; }
int (*operations[2])(int) = {square, cube};
int bump(void) { static int calls; return ++calls + counter; }
int main(void) {
    counter += 2;
    zeroed[99] = *cursor;
    big ^= 0xffffffffffLL;
    printf("%d %d %d %lld %s %s\n", bump(), bump(), zeroed[99], big, messages[0], messages[1]);
    printf("%d %d %d\n", operations[0](7), operations[1](3), cursor[-1] + cursor[2]);
    return 0;
}"#,
    },
    Program {
        name:   "switches",
        source: r#"int printf(const char *, ...);
int dense(int x) {
    switch (x) { case 0: return 7; case 1: return 11; case 2: x += 5; case 3: return x * 2; case 4: return -1; default: return 0; }
}
int sparse(long long x) {
    switch (x) { case -1000000000000LL: return 1; case 17: return 2; case 1LL << 40: return 3; case 99: return 4; }
    return 5;
}
int chars(unsigned char c) {
    switch (c) { case 'a': case 'e': case 'i': case 'o': case 'u': return 1; case 255: return 2; default: return 0; }
}
int main(void) {
    for (int i = -1; i < 6; i++) printf("%d ", dense(i));
    printf("\n%d %d %d %d %d\n", sparse(-1000000000000LL), sparse(17), sparse(1LL << 40), sparse(99), sparse(0));
    printf("%d %d %d\n", chars('e'), chars(255), chars('z'));
    return 0;
}"#,
    },
    Program {
        name:   "arrays_and_pointers",
        source: r#"int printf(const char *, ...);
static void sort(int *a, int n) {
    for (int i = 0; i < n; i++)
        for (int j = 0; j + 1 < n - i; j++)
            if (a[j] > a[j + 1]) { int t = a[j]; a[j] = a[j + 1]; a[j + 1] = t; }
}
static int length(const char *s) { const char *p = s; while (*p) p++; return p - s; }
static void reverse(char *s) { int n = length(s); for (int i = 0; i < n / 2; i++) { char t = s[i]; s[i] = s[n - 1 - i]; s[n - 1 - i] = t; } }
int main(void) {
    int a[10] = {5, -3, 99, 0, 12, 7, 7, -100, 42, 1};
    sort(a, 10);
    for (int i = 0; i < 10; i++) printf("%d ", a[i]);
    char text[] = "native back end";
    reverse(text);
    long long grid[3][4];
    for (int i = 0; i < 3; i++) for (int j = 0; j < 4; j++) grid[i][j] = i * 10 + j;
    printf("%s %d %lld %ld\n", text, length(text), grid[2][3] - grid[1][1], (long)(&grid[2][0] - &grid[0][0]));
    return 0;
}"#,
    },
    Program {
        name:   "structures",
        source: r#"int printf(const char *, ...);
struct small { int x, y; };
struct pair { long long a, b; };
struct big { int v[9]; char tag; };
struct odd { char c[3]; };
static struct small make_small(int x) { struct small s = {x, x * 2}; return s; }
static struct pair make_pair(long long a) { struct pair p = {a, -a}; return p; }
static struct big make_big(int seed) { struct big b; for (int i = 0; i < 9; i++) b.v[i] = seed + i; b.tag = 'q'; return b; }
static int sum_big(struct big b) { int s = 0; for (int i = 0; i < 9; i++) s += b.v[i]; return s + b.tag; }
static int sum_small(struct small s, struct pair p, struct odd o) { return s.x + s.y + (int)(p.a + p.b) + o.c[0] + o.c[1] + o.c[2]; }
static struct odd make_odd(char c) { struct odd o = {{c, c + 1, c + 2}}; return o; }
int main(void) {
    struct small s = make_small(21);
    struct pair p = make_pair(1LL << 40);
    struct big b = make_big(100), copy;
    copy = b;
    copy.v[0] = 0;
    struct odd o = make_odd(10);
    printf("%d %d %lld %lld %d %d %d\n", s.x, s.y, p.a, p.b, sum_big(b), sum_big(copy), sum_small(s, p, o));
    return 0;
}"#,
    },
    Program {
        name:   "pressure_and_calls",
        source: r#"int printf(const char *, ...);
static int id(int x) { return x; }
int main(void) {
    int a = id(1), b = id(2), c = id(3), d = id(4), e = id(5), f = id(6), g = id(7), h = id(8);
    int i = id(9), j = id(10), k = id(11), l = id(12), m = id(13), n = id(14), o = id(15), p = id(16);
    long long q = id(17) * 1000000000LL, r = id(18) * 1000000000LL;
    int s = a * b + c * d - e * f + g * h - i * j + k * l - m * n + o * p;
    int t = id(a + p) * id(b + o) - id(c + n) * id(d + m) + id(e + l) * id(f + k) - id(g + j) * id(h + i);
    printf("%d %d %lld\n", s, t, q + r + a + b + c + d + e + f + g + h + i + j + k + l + m + n + o + p);
    return 0;
}"#,
    },
    Program {
        name:   "selects_and_booleans",
        source: r#"int printf(const char *, ...);
static int clamp(int x, int lo, int hi) { return x < lo ? lo : x > hi ? hi : x; }
static long long absolute(long long x) { return x < 0 ? -x : x; }
static int flags(int a, int b) { int x = (a < b) + (a == b) * 2 + !(a && b) * 4 + (a || b) * 8; return x; }
static _Bool both(int a, int b) { return a > 0 && b > 0; }
int main(void) {
    printf("%d %d %d %lld %lld\n", clamp(-5, 0, 10), clamp(50, 0, 10), clamp(5, 0, 10), absolute(-77), absolute(1LL << 50));
    for (int a = -1; a < 2; a++) for (int b = -1; b < 2; b++) printf("%d%d ", flags(a, b), both(a, b));
    printf("\n");
    return 0;
}"#,
    },
    Program {
        name:   "bit_builtins",
        source: r#"int printf(const char *, ...);
int main(void) {
    unsigned values[] = {1, 2, 0x80000000u, 0x00f0f000u, 12345};
    for (int i = 0; i < 5; i++)
        printf("%d %d %d %u | ", __builtin_clz(values[i]), __builtin_ctz(values[i]), __builtin_popcount(values[i]), __builtin_bswap32(values[i]));
    unsigned long long big = 0x0123456789abcdefULL;
    printf("\n%d %d %d %llx\n", __builtin_clzll(big), __builtin_ctzll(big), __builtin_popcountll(big), __builtin_bswap64(big));
    printf("%d %d\n", __builtin_parity(7), __builtin_ffs(96));
    return 0;
}"#,
    },
    Program {
        name:   "memory_ranges",
        source: r#"int printf(const char *, ...);
struct record { int id; char name[60]; long long stamp[20]; };
static struct record make(int id) { struct record r = {0}; r.id = id; r.name[0] = 'A' + id; r.stamp[19] = id * 1000LL; return r; }
int main(void) {
    struct record a = make(3), b;
    b = a;
    char buffer[300] = {0};
    for (int i = 0; i < 300; i += 37) buffer[i] = (char)i;
    int sum = 0;
    for (int i = 0; i < 300; i++) sum += buffer[i];
    struct record many[4];
    for (int i = 0; i < 4; i++) many[i] = make(i);
    printf("%d %c %lld %d %d %lld\n", b.id, b.name[0], b.stamp[19], sum, many[3].id, many[2].stamp[19]);
    return 0;
}"#,
    },
    Program {
        name:   "volatile_and_goto",
        source: r#"int printf(const char *, ...);
volatile int ticks;
int main(void) {
    volatile int local = 3;
    int i = 0;
again:
    ticks = ticks + local;
    if (++i < 10) goto again;
    printf("%d %d\n", ticks, local);
    return ticks != 30;
}"#,
    },
    Program {
        name:   "large_frame",
        source: r#"int printf(const char *, ...);
static int fill(int *a, int n) { int s = 0; for (int i = 0; i < n; i++) { a[i] = i * 3; s += a[i]; } return s; }
int main(void) {
    int big[5000];
    char bytes[20000];
    for (int i = 0; i < 20000; i++) bytes[i] = (char)(i * 7);
    int s = fill(big, 5000);
    printf("%d %d %d\n", s, big[4999], bytes[19999]);
    return 0;
}"#,
    },
    Program {
        name:   "function_pointers",
        source: r#"int printf(const char *, ...);
typedef int (*binary)(int, int);
static int add(int a, int b) { return a + b; }
static int sub(int a, int b) { return a - b; }
static int mul(int a, int b) { return a * b; }
static int fold(const int *values, int n, binary op, int start) { for (int i = 0; i < n; i++) start = op(start, values[i]); return start; }
static void apply(void (*each)(int *), int *values, int n) { for (int i = 0; i < n; i++) each(&values[i]); }
static void twice(int *x) { *x *= 2; }
int main(void) {
    int values[] = {3, 1, 4, 1, 5, 9, 2, 6};
    binary ops[] = {add, sub, mul};
    apply(twice, values, 8);
    for (int i = 0; i < 3; i++) printf("%d ", fold(values, 8, ops[i], i == 2));
    printf("\n");
    return 0;
}"#,
    },
    Program {
        name:   "loop_carried_swaps",
        source: r#"int printf(const char *, ...);
int main(void) {
    unsigned a = 0, b = 1, c = 2;
    for (int i = 0; i < 40; i++) { unsigned t = a; a = b; b = c; c = t + b; }
    long long x = 1, y = 2;
    for (int i = 0; i < 25; i++) { long long t = x; x = y; y = t; if (i & 1) y += x; }
    printf("%u %u %u %lld %lld\n", a, b, c, x, y);
    return 0;
}"#,
    },
    Program {
        name:   "float_comparisons_and_conversions",
        source: r#"int printf(const char *, ...);
int main(void) {
    double nan = 0.0 / 0.0;
    double values[] = {1.5, -2.25, 0.0, 1e300, nan, -0.0};
    for (int i = 0; i < 6; i++)
        for (int j = 0; j < 6; j++) {
            double a = values[i], b = values[j];
            printf("%d%d%d%d%d%d ", a < b, a <= b, a == b, a != b, a > b, a >= b);
            if (a < b) printf("<"); else if (!(a >= b)) printf("u");
        }
    float fn = nan;
    printf("\n%d %d %d\n", fn != fn, fn == fn, (float)values[0] < 2.0f);
    unsigned long long big = 18446744073709551000ULL, small = 12345;
    double d = (double)big;
    float f = (float)big;
    unsigned long long back = (unsigned long long)1.8e19, sback = (unsigned long long)12345.75;
    printf("%d %d %d %d %d\n", d > 1.8e19, (double)small == 12345.0, f > 1.8e19f, back == 18000000000000000000ULL, sback == 12345);
    unsigned u = 4000000000u;
    signed char c = -100; unsigned short us = 65000; _Bool flag = 1;
    printf("%d %d %d %d %d\n", (int)-3.7, (unsigned)3e9 == 3000000000u, (int)(float)-12.5f, (double)u == 4e9, (int)((double)c + us + flag));
    printf("%lld %d %d\n", (long long)-1e15, (int)(float)0.1 == 0, (int)(1.0f / 3 * 3000));
    float x = 1.25f; double y = x; float z = (float)(y * 3);
    printf("%d %d\n", (int)(z * 100), (int)-(double)x);
    return 0;
}"#,
    },
    Program {
        name:   "float_calls_and_structures",
        source: r#"int printf(const char *, ...);
struct pt { double x, y; };
struct mix { long long a; double b; };
struct fl { float a, b; };
struct big { double v[4]; };
static struct pt mid(struct pt a, struct pt b) { struct pt r = {(a.x + b.x) / 2, (a.y + b.y) / 2}; return r; }
static struct mix mk(long long a, double b) { struct mix m = {a, b}; return m; }
static struct fl mkf(float a, float b) { struct fl r = {a * 2, b - 1}; return r; }
static double total(struct big b) { return b.v[0] + b.v[1] + b.v[2] + b.v[3]; }
static double poly(double x, float y, int n, double z, double w, double v, double u, double t, double s, double r) {
    return x * y + n - z / w + v * u - t + s * r;
}
static float scale(float a, int n, float b, long long m) { return a * n + b * m; }
static double (*pick(int which))(double) ;
static double twice(double x) { return 2 * x; }
static double square(double x) { return x * x; }
static double (*pick(int which))(double) { return which ? square : twice; }
int main(void) {
    struct pt p = mid((struct pt){1, 2}, (struct pt){3, 7});
    struct mix m = mk(7, 2.5);
    struct fl q = mkf(1.5f, 4.0f);
    struct big b = {{1, 2, 3, 4.5}};
    printf("%d %d %lld %d %d %d %d\n", (int)(p.x * 10), (int)(p.y * 10), m.a, (int)(m.b * 4), (int)(q.a * 10), (int)q.b, (int)(total(b) * 2));
    printf("%d %d\n", (int)(poly(1.5, 2.0f, 3, 4, 2, 0.5, 6, 1, 2, 3) * 100), (int)scale(1.5f, 4, 0.25f, 8));
    printf("%d %d\n", (int)pick(0)(21), (int)pick(1)(9));
    double sum = 0; float fsum = 0;
    for (int i = 1; i <= 100; i++) { sum += 1.0 / i; fsum += 1.0f / i; }
    printf("%d %d\n", (int)(sum * 1000), (int)(fsum * 1000));
    return 0;
}"#,
    },
    Program {
        name:   "variadic_definitions",
        source: r#"int printf(const char *, ...);
static double dsum(int n, ...) {
    __builtin_va_list ap;
    __builtin_va_start(ap, n);
    double s = 0;
    for (int i = 0; i < n; i++) s += __builtin_va_arg(ap, double);
    __builtin_va_end(ap);
    return s;
}
static long long products(int n, ...) {
    __builtin_va_list ap, aq;
    __builtin_va_start(ap, n);
    __builtin_va_copy(aq, ap);
    long long s = 0;
    for (int i = 0; i < n; i++) s += __builtin_va_arg(ap, long long) * __builtin_va_arg(aq, long long);
    __builtin_va_end(ap);
    __builtin_va_end(aq);
    return s;
}
static int mixed(const char *format, ...) {
    __builtin_va_list ap;
    __builtin_va_start(ap, format);
    int total = 0;
    for (const char *p = format; *p; p++) {
        if (*p == 'i') total += __builtin_va_arg(ap, int);
        else if (*p == 'd') total += (int)(__builtin_va_arg(ap, double) * 10);
        else if (*p == 'p') total += *__builtin_va_arg(ap, int *);
    }
    __builtin_va_end(ap);
    return total;
}
int main(void) {
    int seven = 7;
    printf("%d\n", (int)(dsum(10, 1.0, 2.0, 3.5, 4.0, 10.25, 1.0, 1.0, 1.0, 1.0, 1.5) * 4));
    printf("%lld\n", products(9, 1LL, 2LL, 3LL, 4LL, 5LL, 6LL, 7LL, 8LL, 9LL));
    printf("%d\n", mixed("idipdidid", 1, 2.5, 3, &seven, 0.25, 100, 1.5, 1000, 9.75));
    return 0;
}"#,
    },
    Program {
        name:   "int128_and_dynamic_stack",
        source: r#"int printf(const char *, ...);
typedef __int128 i128;
typedef unsigned __int128 u128;
static void show(const char *tag, u128 x) {
    printf("%s %llx %llx\n", tag, (unsigned long long)(x >> 64), (unsigned long long)x);
}
static u128 mul(u128 a, u128 b) { return a * b; }
static u128 make(unsigned long long hi, unsigned long long lo) { return ((u128)hi << 64) | lo; }
static int vla(int n) {
    int a[n];
    for (int i = 0; i < n; i++) a[i] = i * i;
    int s = 0;
    for (int i = 0; i < n; i++) s += a[i];
    char *p = __builtin_alloca(n * 3);
    for (int i = 0; i < n * 3; i++) p[i] = (char)i;
    return s + p[n * 3 - 1];
}
int main(void) {
    u128 a = make(0x0123456789abcdefULL, 0xfedcba9876543210ULL);
    u128 b = make(3, 0xffffffffffffffffULL);
    show("add", a + b);
    show("sub", a - b);
    show("mul", mul(a, b));
    show("and", a & b);
    show("xor", a ^ ~b);
    for (int s = 0; s < 128; s += 37) {
        show("shl", a << s);
        show("shr", a >> s);
        show("sar", (u128)((i128)a >> s));
        show("sar-neg", (u128)(-(i128)a >> s));
    }
    show("shl70", a << 70);
    show("sar70", (u128)((i128)~a >> 70));
    show("div", a / b);
    show("mod", a % b);
    show("sdiv", (u128)(-(i128)a / (i128)b));
    show("smod", (u128)(-(i128)a % 1000));
    printf("%d %d %d %d %d\n", a < b, a > b, (i128)a < (i128)-b, a == a, a != b);
    long long x = -5;
    show("sext", (u128)(i128)x);
    show("zext", (u128)(unsigned long long)x);
    unsigned long long hi, lo;
    u128 big = (u128)-1;
    int over = __builtin_mul_overflow(big, (u128)2, &big);
    int carry = __builtin_add_overflow(a, b, &a);
    long long sres;
    int sover = __builtin_mul_overflow(1LL << 62, 4, &sres);
    printf("%d %d %d %lld\n", over, carry, sover, sres);
    hi = (unsigned long long)(a >> 64); lo = (unsigned long long)a;
    printf("%llx %llx\n", hi, lo);
    printf("%d %d\n", vla(10), vla(1000));
    return 0;
}"#,
    },
    Program {
        // An over-aligned dynamic block must not reach into the outgoing
        // arguments (`printf`'s stack arguments, and on Win64 the home area
        // it saves its registers into); a leaf's blocks are 16-byte aligned
        // too.
        name:   "aligned_dynamic_stack",
        source: r#"int printf(const char *, ...);
typedef struct { int x; } __attribute__((aligned(64))) Wide;
static int aligned(int n) {
    Wide v[n];
    int bad = (int)((unsigned long long)v & 63) != 0;
    for (int i = 0; i < n; i++) v[i].x = 1000 + i;
    printf("%d %d %d %d %d %d %d %d\n", n, 2, 3, 4, 5, 6, 7, 8);
    for (int i = 0; i < n; i++) bad |= (v[i].x != 1000 + i) << 1;
    return bad;
}
static int leaf(int n) {
    volatile int k = n;
    long long v[n];
    v[0] = k;
    return (int)((unsigned long long)v & 15) + (int)v[0] - n;
}
int main(void) {
    int bad = 0;
    for (int n = 1; n < 6; n++) bad |= aligned(n) | leaf(n) << 2;
    printf("%d\n", bad);
    return bad;
}"#,
    },
];
