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
        name:   "division_by_powers_of_two",
        source: r#"int printf(const char *, ...);
int values[] = {0, 1, -1, 7, -7, 8, -8, 100, -100, 2147483647, -2147483647 - 1, 12345, -12345};
long long wide[] = {0, 1, -1, 1000000000007LL, -1000000000007LL, 9223372036854775807LL, -9223372036854775807LL - 1};
int main(void) {
    for (unsigned i = 0; i < sizeof values / sizeof values[0]; i++) {
        int x = values[i];
        unsigned u = (unsigned)x;
        signed char c = (signed char)x;
        short h = (short)x;
        printf("%d %d %d %d %d %d %d %d | ", x / 1, x / 2, x / 4, x / 16, x / 1073741824, x % 1, x % 2, x % 8);
        printf("%d %d %d | ", x % 1073741824, c / 4, c % 4);
        printf("%d %d | ", h / 8, h % 8);
        printf("%u %u %u %u %u\n", u / 2, u / 2147483648u, u % 16, u % 2147483648u, (unsigned char)u / 4u);
    }
    for (unsigned i = 0; i < sizeof wide / sizeof wide[0]; i++) {
        long long x = wide[i];
        unsigned long long u = (unsigned long long)x;
        printf("%lld %lld %lld %lld %lld | ", x / 2, x / 2147483648LL, x / 4611686018427387904LL, x % 1024, x % 4294967296LL);
        printf("%llu %llu %llu\n", u / 8, u / 9223372036854775808ULL, u % 4294967296ULL);
    }
    return 0;
}"#,
    },
    Program {
        name:   "indexed_addresses",
        source: r#"int printf(const char *, ...);
struct point { short x; int y; long long z; };
static int ints[64];
static long long longs[64];
static struct point points[16];
static int get(const int *p, int i) { return p[i] + p[i + 1] + p[i - 1]; }
static long long walk(const long long *p, long n, long stride) {
    long long sum = 0;
    for (long i = 0; i < n; i += stride) sum += p[i] * (i + 1);
    return sum;
}
int main(void) {
    char chars[40];
    short shorts[40];
    for (int i = 0; i < 64; i++) { ints[i] = i * i - 50; longs[i] = (long long)i << 33 | i; }
    for (int i = 0; i < 40; i++) { chars[i] = (char)(i * 7); shorts[i] = (short)(i * -300); }
    for (int i = 0; i < 16; i++) { points[i].x = (short)-i; points[i].y = i * 1000; points[i].z = -i * 100000LL; }
    long long sum = 0;
    for (int i = 1; i < 39; i++) sum += chars[i] + shorts[i - 1] * 3 + get(ints, i) + longs[i + 20];
    for (int i = 15; i >= 0; i--) sum += points[i].x + points[i].y + points[i].z + points[15 - i].y;
    int *middle = &ints[32];
    for (int i = -30; i < 30; i += 7) sum ^= middle[i] + middle[-i];
    printf("%lld %lld %lld\n", sum, walk(longs, 64, 3), walk(longs + 5, 50, 7));
    unsigned idx = 3; long neg = -2;
    printf("%d %d %lld\n", ints[idx * 2], (&ints[10])[neg], longs[idx + (unsigned)neg]);
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
        // `i128` to and from `f32` and `f64` through the runtime's
        // `__floattidf`, `__fixsfti` and their unsigned forms, with values
        // that round, exceed 64 bits, and set the top bit.
        name:   "int128_float_conversions",
        source: r#"int printf(const char *, ...);
typedef __int128 i128;
typedef unsigned __int128 u128;
static unsigned long long bits(double d) { unsigned long long b; __builtin_memcpy(&b, &d, 8); return b; }
static unsigned fbits(float f) { unsigned b; __builtin_memcpy(&b, &f, 4); return b; }
static void show(u128 x) { printf(" %llx:%llx", (unsigned long long)(x >> 64), (unsigned long long)x); }
static i128 big(int k) { return ((i128)0x123456789ABCDEFLL << 40) * k + k; }
int main(void) {
    i128 values[] = { 0, 1, -1, 1000000007, -((i128)1 << 100), big(3), big(-7),
                      ((i128)1 << 64) + 1, ((i128)1 << 24) + 1, -(((i128)1 << 53) + 1) };
    u128 uvalues[] = { 0, 5, ~(u128)0, (u128)1 << 127, ((u128)1 << 64) - 1, big(9) };
    for (int i = 0; i < 10; i++) {
        double d = (double)values[i];
        float f = (float)values[i];
        printf("%llx %x", bits(d), fbits(f));
        show((u128)(i128)d);
        show((u128)(i128)f);
        printf("\n");
    }
    for (int i = 0; i < 6; i++) {
        double d = (double)uvalues[i];
        float f = (float)uvalues[i];
        printf("%llx %x", bits(d), fbits(f));
        if (i != 2) {
            show((u128)d);
            show((u128)f);
        }
        printf("\n");
    }
    double x = 1.5e30;
    float y = -2.5e20f;
    show((u128)(i128)x);
    show((u128)(i128)y);
    show((u128)3.0e38f);
    show((u128)-0.75);
    printf("\n");
    return 0;
}"#,
    },
    Program {
        // Locals aligned above the stack's 16 bytes, in a leaf, beside a
        // call, beside a dynamic allocation, and passed by address.
        name:   "over_aligned_locals",
        source: r#"int printf(const char *, ...);
typedef unsigned long long u64;
static int check(const void *p, u64 align) { return ((u64)p & (align - 1)) == 0; }
static int leaf(int k) {
    _Alignas(64) char buffer[100];
    __attribute__((aligned(256))) int table[3];
    for (int i = 0; i < 100; i++) buffer[i] = (char)(i * k);
    table[0] = buffer[99]; table[1] = k; table[2] = (int)((u64)buffer & 63) + (int)((u64)table & 255);
    return table[0] + table[1] + table[2];
}
static int caller(int k) {
    _Alignas(32) double values[4] = { 1, 2, 3, 4 };
    _Alignas(128) char small = (char)k;
    int ok = check(values, 32) && check(&small, 128);
    return ok * 1000 + leaf(k) + (int)values[3] + small;
}
static int dynamic(int n) {
    char vla[n];
    _Alignas(64) int aligned[2] = { n, 2 * n };
    for (int i = 0; i < n; i++) vla[i] = (char)i;
    return check(aligned, 64) * 100 + aligned[1] + vla[n - 1];
}
int main(void) {
    printf("%d %d %d\n", caller(3), caller(-7), dynamic(17));
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
    Program {
        // GNU extensions: GCC "Labels as Values" (label tables, label
        // differences and `goto *`) and `__thread` objects, initialized,
        // zeroed, holding an address and reached through a pointer.
        name:   "computed_goto_and_thread_locals",
        source: r#"int printf(const char *, ...);
static int run(const unsigned char *code, int n) {
    static void *const ops[] = { &&inc, &&dec, &&dbl, &&end };
    static const int offsets[] = { &&inc - &&inc, &&dec - &&inc, &&dbl - &&inc, &&end - &&inc };
    int acc = n, pc = 0;
    long long mix = 1;
    goto *ops[code[pc++]];
inc: acc++; mix = mix * 3 + acc; goto *(&&inc + offsets[code[pc++]]);
dec: acc--; mix ^= acc; goto *ops[code[pc++]];
dbl: acc *= 2; mix += acc; goto *ops[code[pc++]];
end: return acc + (int)(mix & 0xff);
}
int shared = 40;
__thread int counter = 7;
static __thread long zeroed;
static __thread int table[4] = { 1, 2, 3, 4 };
static __thread int *pointer = &shared;
static int *counter_address(void) { return &counter; }
int main(void) {
    unsigned char program[] = { 0, 0, 2, 1, 2, 0, 3 };
    int result = run(program, 5);
    for (int i = 0; i < 4; i++) { counter += table[i]; zeroed += i; }
    table[2] += table[1];
    *pointer += 2;
    *counter_address() += result;
    printf("%d %d %ld %d %d %d\n", result, counter, zeroed, table[2], shared,
           counter_address() == &counter);
    void *next = &&done;
    goto *next;
done:
    return result & 0x7f;
}"#,
    },
    Program {
        // GNU extensions (GCC manual, "Common Function Attributes" and
        // "Common Variable Attributes"): weak definitions and declarations,
        // aliases (a thread-local one too), sections, `used`, visibility,
        // alignment, prioritized constructors and destructors, and the
        // level-0 return and frame addresses.
        name:   "symbol_attributes",
        source: r#"int printf(const char *, ...);
int counter = 1;
__attribute__((weak)) int weak_data = 5;
__attribute__((weak)) int weak_fn(void) { return 3; }
extern int missing_data __attribute__((weak));
int missing_fn(void) __attribute__((weak));
int real(int x) { return x + counter; }
int alias_fn(int) __attribute__((alias("real")));
extern int alias_data __attribute__((alias("counter")));
static __thread int tls_target = 9;
extern __thread int tls_alias __attribute__((alias("tls_target")));
__attribute__((section("mysec"))) int in_section = 4;
__attribute__((section("mytext"))) int sectioned_fn(void) { return 6; }
__attribute__((section("myro"))) const int ro_section = 8;
__attribute__((section("mybss"))) int bss_section;
__attribute__((used)) static int kept = 2;
__attribute__((visibility("hidden"))) int hidden_data = 1;
__attribute__((aligned(64))) int aligned_fn(void) { return 1; }
__attribute__((constructor)) static void ctor(void) { counter += 10; }
__attribute__((constructor(200))) static void ctor200(void) { counter *= 2; }
__attribute__((destructor)) static void dtor(void) { printf("dtor\n"); }
__attribute__((destructor(300))) static void dtor300(void) { printf("dtor300\n"); }
__attribute__((noinline)) static int frames(void) {
    return __builtin_frame_address(0) != 0 && __builtin_return_address(0) != 0;
}
int main(void) {
    tls_alias += 1;
    printf("%d %d %d %d %d %d\n", counter, weak_data, weak_fn(), alias_fn(1), alias_data, tls_target);
    printf("%d %d %d %d %d %d\n", &missing_data == 0, missing_fn == 0, in_section, sectioned_fn(),
           ro_section + bss_section, kept + hidden_data + aligned_fn());
    printf("%d %d\n", frames(), ((unsigned long long)aligned_fn & 63) == 0);
    return 0;
}"#,
    },
    Program {
        // Values live into the target of an indirect jump are never split,
        // so with more of them than registers they stay in their slots and
        // every use goes through the scratch registers. An indexed store
        // then reads three spilled general registers: the base, the index
        // and the stored value (review of stage 4, finding 1).
        name:   "indexed_stores_across_an_indirect_jump",
        source: r#"int printf(const char *, ...);
volatile unsigned long long v[13] = { 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13 };
unsigned long long buffer[16];
unsigned long long *volatile ptr = buffer;
#define V(n) unsigned long long a##n = v[n];
static unsigned long long f(unsigned long long i) {
    V(0) V(1) V(2) V(3) V(4) V(5) V(6) V(7) V(8) V(9) V(10) V(11) V(12)
    unsigned long long *p = ptr;
    void *t = &&L;
    if (i & 1) t = &&R;
    goto *t;
L:  p[a12 & 15] = a12; goto End;
R:  p[a12 & 15] = a12 + 1;
End: return i + a0 + a1 + a2 + a3 + a4 + a5 + a6 + a7 + a8 + a9 + a10 + a11 + a12;
}
volatile unsigned long long input[32];
static unsigned long long run(unsigned long long i) {
#define A(n) unsigned long long b##n = input[n];
#define S(x, y) p[b##x & 31] = b##y;
    A(0) A(1) A(2) A(3) A(4) A(5) A(6) A(7) A(8) A(9) A(10) A(11) A(12) A(13) A(14) A(15)
    A(16) A(17) A(18) A(19) A(20) A(21) A(22) A(23)
    unsigned long long *p = ptr;
    void *dest = &&L1;
    if (i & 1) dest = &&L2;
    goto *dest;
L1: S(0, 1) S(1, 2) S(2, 3) S(3, 4) S(4, 5) S(5, 6) S(6, 7) S(7, 8) S(8, 9) S(9, 10) S(10, 11)
    S(11, 12) S(12, 13) S(13, 14) S(14, 15) S(15, 16) S(16, 17) S(17, 18) S(18, 19) S(19, 20)
    S(20, 21) S(21, 22) S(22, 23) S(23, 0)
    goto End;
L2: S(0, 2) S(1, 3) S(2, 4) S(3, 5) S(4, 6) S(5, 7) S(6, 8) S(7, 9) S(8, 10) S(9, 11) S(10, 12)
    S(11, 13) S(12, 14) S(13, 15) S(14, 16) S(15, 17) S(16, 18) S(17, 19) S(18, 20) S(19, 21)
    S(20, 22) S(21, 23) S(22, 0) S(23, 1)
End: return i + b0 + b1 + b2 + b3 + b4 + b5 + b6 + b7 + b8 + b9 + b10 + b11 + b12 + b13 + b14
        + b15 + b16 + b17 + b18 + b19 + b20 + b21 + b22 + b23;
}
int main(void) {
    unsigned long long first = f(0);
    printf("%llu %llu\n", first, buffer[13]);
    unsigned long long big[32] = { 0 };
    for (int i = 0; i < 32; i++) input[i] = i * 7 + 1;
    ptr = big;
    unsigned long long h = run(1) + run(0);
    for (int i = 0; i < 32; i++) h = h * 33 + big[i];
    printf("%llu\n", h);
    return first != 91 || buffer[13] != 13;
}"#,
    },
    Program {
        // Every instruction form the selector makes, with its operands
        // among more values live into the targets of an indirect jump than
        // there are registers of either class: indexed stores, loads and
        // `lea`s of every width, SSE loads and stores, `i128` pairs, shifts
        // by `%cl` (`shld` too), division, the `imul` forms, `cmov`, bit
        // scans, `setcc`, `bswap`, `neg`, `test`, `ucomisd` and the
        // conversions, inline copies and fills, an indirect call and a
        // variadic one. The end-to-end tests also run it with every value
        // spilled.
        name:   "every_form_across_an_indirect_jump",
        source: r#"int printf(const char *, ...);
volatile unsigned long long in[32];
unsigned long long q[64];
unsigned w32[64];
unsigned short w16[64];
signed char w8[64];
double wd[64];
float wf[64];
unsigned __int128 wq[16];
struct S { long long a, b, c; } ws[8];
unsigned long long *volatile qp = q;
unsigned long long *volatile kept;
static unsigned long long add3(unsigned long long a, unsigned long long b, unsigned long long c) {
    return a * 3 + b * 5 + c;
}
unsigned long long (*volatile fp)(unsigned long long, unsigned long long, unsigned long long) = add3;
#define A(n) unsigned long long a##n = in[n];
#define D(n) double d##n = (double)(a##n & 0xffff) + 0.5;
static unsigned long long run(int sel) {
    A(0) A(1) A(2) A(3) A(4) A(5) A(6) A(7) A(8) A(9) A(10) A(11) A(12) A(13) A(14) A(15)
    A(16) A(17) A(18) A(19) A(20) A(21) A(22) A(23) A(24) A(25) A(26) A(27)
    D(0) D(1) D(2) D(3) D(4) D(5) D(6) D(7) D(8) D(9) D(10) D(11) D(12) D(13) D(14) D(15)
    unsigned long long *p = qp;
    unsigned long long r = 0;
    void *t = sel == 0 ? &&L0 : sel == 1 ? &&L1 : &&L2;
    goto *t;
L0:
    p[a0 & 63] = a1;
    w32[a2 & 63] = (unsigned)a3;
    w16[a4 & 63] = (unsigned short)a5;
    w8[a6 & 63] = (signed char)a7;
    r += p[a8 & 63] + w32[a9 & 63] + w16[a10 & 63] + (unsigned long long)w8[a11 & 63];
    kept = &p[a12 & 63];
    wd[a13 & 63] = d0;
    wf[a14 & 63] = (float)d1;
    r += (unsigned long long)(wd[a15 & 63] + wf[a16 & 63]);
    p[(a17 & 31) * 2] += a18;
    r += p[a19 & 63] < a20 ? 1 : 3;
    goto End;
L1:
    wq[a20 & 15] = (unsigned __int128)a21 * a22;
    r += (unsigned long long)(wq[a23 & 15] >> 64) + (unsigned long long)wq[a24 & 15];
    r += (unsigned long long)(((unsigned __int128)a25 << (a26 & 127)) >> 64);
    r += (unsigned long long)(((unsigned __int128)a1 << 64 | a2) >> (a3 & 127));
    r += a27 << (a0 & 63) ^ a1 >> (a2 & 63) ^ (unsigned long long)((long long)a3 >> (a4 & 63));
    r += a5 / (a6 | 1) + a7 % (a8 | 1);
    r += (unsigned long long)((long long)a9 / ((long long)(a10 & 0xff) + 1));
    r += (unsigned long long)((long long)a11 % ((long long)(a12 & 0xff) - 300));
    r += (unsigned)a13 / ((unsigned)a14 | 1) + (unsigned)a15 % ((unsigned)a16 | 1);
    ws[a13 & 7] = ws[a14 & 7];
    __builtin_memset(&ws[a15 & 7], (int)a16, sizeof(struct S));
    goto End;
L2:
    r += a17 * 1000003 + a18 * a19;
    r += a20 < a21 ? a22 : a23;
    r += (unsigned long long)__builtin_clzll(a24 | 1) + (unsigned long long)__builtin_ctzll(a25 | 0x100)
        + (unsigned long long)__builtin_popcountll(a26);
    r += (a27 == a0) + (a1 < a2) + ((long long)a3 > (long long)a4);
    r += __builtin_bswap64(a5) ^ -a6;
    r += (a7 & a8) ? 1 : 2;
    r += (unsigned long long)(long long)(d2 * d3 - d4) + (d5 < d6) + (d7 == d8);
    r += (unsigned long long)(double)(long long)(a9 >> 12);
    r += fp(a9, a10, a11);
    r += (unsigned)(a12 + a13) + (unsigned)a14 * 3u;
    printf("%llu %llu %lld\n", a15, a16, (long long)(d9 * 10));
    goto End;
End:
    return r + a0 + a1 + a2 + a3 + a4 + a5 + a6 + a7 + a8 + a9 + a10 + a11 + a12 + a13 + a14
        + a15 + a16 + a17 + a18 + a19 + a20 + a21 + a22 + a23 + a24 + a25 + a26 + a27
        + (unsigned long long)(d0 + d1 + d2 + d3 + d4 + d5 + d6 + d7 + d8 + d9 + d10 + d11
                               + d12 + d13 + d14 + d15);
}
int main(void) {
    for (int i = 0; i < 32; i++) in[i] = (unsigned long long)i * 0x9E3779B97F4A7C15ULL + i;
    unsigned long long h = 0;
    for (int sel = 0; sel < 3; sel++) h = h * 31 + run(sel);
    for (int i = 0; i < 64; i++)
        h = h * 33 + q[i] + w32[i] + w16[i] + (unsigned long long)w8[i]
            + (unsigned long long)(long long)(wd[i] + wf[i]);
    for (int i = 0; i < 8; i++) h = h * 7 + (unsigned long long)ws[i].a + (unsigned long long)ws[i].b + (unsigned long long)ws[i].c;
    for (int i = 0; i < 16; i++) h = h * 5 + (unsigned long long)wq[i] + (unsigned long long)(wq[i] >> 64);
    printf("%llu %d\n", h, (int)(kept - q));
    return (int)(h & 0x7f);
}"#,
    },
    // C11 §7.17 and the GNU builtins: loads and stores in every order,
    // exchange, strong and weak compare-exchange that succeed and fail,
    // every fetch operation at every width with its old value used and
    // unused, `_Atomic` operators, fences, `atomic_flag`, floating fetch-add,
    // and the `i128` and structure operations that call libatomic.
    Program {
        name:   "atomics",
        source: r#"#include <stdatomic.h>
int printf(const char *, ...);
static unsigned char u8;
static signed char s8;
static short s16;
static unsigned short u16;
static int s32;
static long long s64;
static unsigned long long u64;
static _Atomic int counter;
static _Atomic double real;
static double plain_real;
static atomic_flag flag = ATOMIC_FLAG_INIT;
static __int128 big;
struct block { long long a, b, c; };
static _Atomic struct block blocks;
#define FETCH(x, v) do { \
    __atomic_fetch_add(&x, v, __ATOMIC_RELAXED); \
    __atomic_fetch_sub(&x, 1, __ATOMIC_RELEASE); \
    __atomic_fetch_and(&x, ~2, __ATOMIC_SEQ_CST); \
    __atomic_fetch_or(&x, 64, __ATOMIC_ACQ_REL); \
    __atomic_fetch_xor(&x, 5, __ATOMIC_ACQUIRE); \
    unsigned long long r = __atomic_fetch_add(&x, v, __ATOMIC_SEQ_CST); \
    r = r * 7 + __atomic_fetch_sub(&x, 3, __ATOMIC_SEQ_CST); \
    r = r * 7 + __atomic_fetch_and(&x, 0x5d, __ATOMIC_SEQ_CST); \
    r = r * 7 + __atomic_fetch_or(&x, 0x30, __ATOMIC_SEQ_CST); \
    r = r * 7 + __atomic_fetch_xor(&x, 0x11, __ATOMIC_SEQ_CST); \
    r = r * 7 + __atomic_fetch_nand(&x, 0x7c, __ATOMIC_SEQ_CST); \
    r = r * 7 + __atomic_fetch_min(&x, -3, __ATOMIC_SEQ_CST); \
    r = r * 7 + __atomic_fetch_max(&x, 100, __ATOMIC_SEQ_CST); \
    r = r * 7 + __atomic_add_fetch(&x, 9, __ATOMIC_SEQ_CST); \
    r = r * 7 + __atomic_exchange_n(&x, 42, __ATOMIC_SEQ_CST); \
    printf("%s %llu %lld\n", #x, r, (long long)__atomic_load_n(&x, __ATOMIC_SEQ_CST)); \
} while (0)
int main(void) {
    __atomic_store_n(&s64, 5, __ATOMIC_RELAXED);
    __atomic_store_n(&s64, 6, __ATOMIC_RELEASE);
    __atomic_store_n(&s64, 7, __ATOMIC_SEQ_CST);
    long long loads = __atomic_load_n(&s64, __ATOMIC_RELAXED) + __atomic_load_n(&s64, __ATOMIC_ACQUIRE)
        + __atomic_load_n(&s64, __ATOMIC_CONSUME) + __atomic_load_n(&s64, __ATOMIC_SEQ_CST);
    long long expected = 9;
    int failed = __atomic_compare_exchange_n(&s64, &expected, 11, 0, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST);
    int strong = __atomic_compare_exchange_n(&s64, &expected, 11, 0, __ATOMIC_ACQ_REL, __ATOMIC_ACQUIRE);
    long long weak_expected = 11;
    int tries = 0;
    while (!__atomic_compare_exchange_n(&s64, &weak_expected, 12, 1, __ATOMIC_RELEASE, __ATOMIC_RELAXED))
        tries++;
    printf("%lld %d %d %lld %lld %d\n", loads, failed, strong, expected, s64, tries < 2);
    FETCH(u8, 200);
    FETCH(s8, -100);
    FETCH(s16, -30000);
    FETCH(u16, 65000);
    FETCH(s32, 2000000000);
    FETCH(s64, 9000000000000000000LL);
    FETCH(u64, 18000000000000000000ULL);
    int c = atomic_fetch_add(&counter, 4);
    counter += 5;
    counter++;
    counter *= 3;
    int seen = 30;
    atomic_compare_exchange_strong(&counter, &seen, -1);
    static int legacy = 30;
    printf("%d %d %d %d\n", c, atomic_load_explicit(&counter, memory_order_relaxed), seen,
           __sync_val_compare_and_swap(&legacy, 30, 31));
    printf("%d %d %d\n", __sync_fetch_and_add(&legacy, 2), __sync_bool_compare_and_swap(&legacy, 33, 1),
           __sync_lock_test_and_set(&legacy, 8));
    __sync_lock_release(&legacy);
    __sync_synchronize();
    atomic_thread_fence(memory_order_seq_cst);
    atomic_thread_fence(memory_order_acquire);
    atomic_signal_fence(memory_order_seq_cst);
    int first = atomic_flag_test_and_set(&flag), second = atomic_flag_test_and_set(&flag);
    atomic_flag_clear(&flag);
    int third = atomic_flag_test_and_set_explicit(&flag, memory_order_acquire);
    printf("%d %d %d %d %d\n", counter, legacy, first, second, third);
    real += 1.5;
    real -= 0.25;
    double before = __atomic_fetch_add(&plain_real, 2.5, __ATOMIC_SEQ_CST);
    before += __atomic_fetch_sub(&plain_real, 0.5, __ATOMIC_SEQ_CST);
    printf("%d %d %d\n", (int)(atomic_load(&real) * 100), (int)(before * 100), (int)(plain_real * 100));
    __atomic_store_n(&big, (__int128)1 << 100, __ATOMIC_SEQ_CST);
    __int128 old = __atomic_fetch_add(&big, 3, __ATOMIC_SEQ_CST);
    __int128 want = old;
    int swapped = __atomic_compare_exchange_n(&big, &want, -5, 0, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST);
    __int128 now = __atomic_load_n(&big, __ATOMIC_ACQUIRE);
    printf("%d %d %lld %lld\n", (int)(old >> 100), swapped, (long long)want, (long long)now);
    struct block b = {1, 2, 3}, n = {4, 5, 6};
    atomic_store(&blocks, b);
    struct block e = atomic_exchange(&blocks, n);
    struct block l = atomic_load(&blocks);
    printf("%lld %lld\n", e.a + e.b + e.c, l.a * 100 + l.b * 10 + l.c);
    return 0;
}"#,
    },
];

/// Programs with `long double` values, which the interpreter does not
/// run: the LLVM back end's executable of the same module is the expected
/// behaviour. Each prints the bits of what it computes, so that printing
/// does not depend on the C library's `long double` support.
pub(super) const WIDE_FLOAT_PROGRAMS: &[Program] = &[
    Program {
        name:   "long_double_arithmetic",
        source: r#"int printf(const char *, ...);
long double fmodl(long double, long double);
typedef long double ld;
static void show(ld x) {
    // The sign and payload of a NaN that arithmetic makes are unspecified.
    if (x != x) { printf(" nan"); return; }
    unsigned long long words[2] = { 0, 0 };
    __builtin_memcpy(words, &x, 10);
    printf(" %llx:%llx", words[1] & 0xffff, words[0]);
}
static ld add(ld a, ld b) { return a + b; }
static ld sub(ld a, ld b) { return a - b; }
static ld mul(ld a, ld b) { return a * b; }
static ld quo(ld a, ld b) { return a / b; }
static ld neg(ld a) { return -a; }
static ld pick(int c, ld a, ld b) { return c ? a : b; }
int main(void) {
    ld zero = 0.0L;
    ld values[] = { 0.0L, -0.0L, 1.5L, -2.25L, 0.1L, 3.0L, 1e4000L, 1e-4000L, zero / zero, 1.0L / zero };
    for (int i = 0; i < 10; i++) {
        for (int j = 0; j < 10; j++) {
            ld a = values[i], b = values[j];
            printf("%d%d%d%d%d%d", a < b, a <= b, a == b, a != b, a > b, a >= b);
            printf("%d%d ", !(a >= b), !(a < b) && !(a >= b));
            if (j % 3 == 0) {
                show(add(a, b));
                show(sub(a, b));
                show(mul(a, b));
                show(quo(a, b));
                printf("\n");
            }
        }
        show(neg(values[i]));
        show(pick(i & 1, values[i], values[9 - i]));
        printf("\n");
    }
    ld sum = 0;
    for (int i = 1; i <= 1000; i++) sum += 1.0L / i;
    show(sum);
    ld product = 1;
    for (int i = 1; i <= 30; i++) product *= i;
    show(product);
    show(fmodl(values[2] * 1000, 3.25L));
    printf("\n");
    return 0;
}"#,
    },
    Program {
        name:   "long_double_conversions",
        source: r#"int printf(const char *, ...);
typedef long double ld;
static void show(ld x) {
    // The sign and payload of a NaN that arithmetic makes are unspecified.
    if (x != x) { printf(" nan"); return; }
    unsigned long long words[2] = { 0, 0 };
    __builtin_memcpy(words, &x, 10);
    printf(" %llx:%llx", words[1] & 0xffff, words[0]);
}
static unsigned long long dbits(double d) { unsigned long long b; __builtin_memcpy(&b, &d, 8); return b; }
static unsigned fbits(float f) { unsigned b; __builtin_memcpy(&b, &f, 4); return b; }
static void wide(unsigned __int128 x) { printf(" %llx:%llx", (unsigned long long)(x >> 64), (unsigned long long)x); }
int main(void) {
    ld values[] = { 0.0L, 1.0L, -1.0L, 2.5L, -2.5L, 3.75L, -3.75L, 100.9L, -127.9L, 32767.5L };
    for (int i = 0; i < 10; i++) {
        ld x = values[i];
        printf("%d %d %d %d %lld", (short)x, (int)x, (_Bool)(x != 0), (int)(long long)x, (long long)(x * 1e14L));
        if (x > -128 && x < 128) printf(" %d", (signed char)x);
        if (x >= 0 && x < 256) printf(" %u", (unsigned char)x);
        if (x >= 0)
            printf(" %u %u %llu", (unsigned short)x, (unsigned)(x * 1e5L), (unsigned long long)(x * 1e14L));
        printf(" %llx %x", dbits((double)x), fbits((float)x));
        wide((unsigned __int128)(__int128)(x * 1e30L));
        printf("\n");
    }
    long long ints[] = { 0, 1, -1, 123456789, -987654321012345LL, 9223372036854775807LL, (-9223372036854775807LL - 1) };
    for (int i = 0; i < 7; i++) {
        show((ld)ints[i]);
        show((ld)(int)ints[i]);
        show((ld)(short)ints[i]);
        show((ld)(signed char)ints[i]);
        show((ld)(unsigned long long)ints[i]);
        show((ld)(unsigned)ints[i]);
        show((ld)(unsigned short)ints[i]);
        show((ld)(unsigned char)ints[i]);
        show((ld)(_Bool)ints[i]);
        printf("\n");
    }
    unsigned long long big[] = { 9223372036854775808ULL, 18446744073709551615ULL, 18000000000000000000ULL };
    for (int i = 0; i < 3; i++) {
        ld x = (ld)big[i];
        show(x);
        printf(" %llu %llu\n", (unsigned long long)x, (unsigned long long)(x / 2));
    }
    double doubles[] = { 0.1, -1e300, 1e-310, 3.5 };
    float floats[] = { 0.1f, -3e38f, 1e-40f, 7.25f };
    for (int i = 0; i < 4; i++) {
        show((ld)doubles[i]);
        show((ld)floats[i]);
        printf(" %llx %x\n", dbits((double)((ld)doubles[i] / 3)), fbits((float)((ld)floats[i] / 3)));
    }
    __int128 wides[] = { 0, -1, (__int128)1 << 100, -((__int128)12345678901234567LL * 1000000007), ((__int128)1 << 64) + 3 };
    for (int i = 0; i < 5; i++) {
        ld x = (ld)wides[i];
        show(x);
        show((ld)(unsigned __int128)wides[i]);
        wide((unsigned __int128)(__int128)x);
        if (x >= 0) wide((unsigned __int128)x);
        printf("\n");
    }
    return 0;
}"#,
    },
    Program {
        // System V passes `long double` in memory and returns it in
        // `%st0`; Win64 passes it by reference and returns it through a
        // hidden pointer. Structures and complex values of `long double`
        // come back in `%st0` (and `%st1`) on System V.
        name:   "long_double_calls",
        source: r#"int printf(const char *, ...);
typedef long double ld;
struct wrap { ld x; };
struct mixed { ld x; int n; };
static void show(ld x) {
    // The sign and payload of a NaN that arithmetic makes are unspecified.
    if (x != x) { printf(" nan"); return; }
    unsigned long long words[2] = { 0, 0 };
    __builtin_memcpy(words, &x, 10);
    printf(" %llx:%llx", words[1] & 0xffff, words[0]);
}
static ld many(ld a, int i, ld b, double d, ld c, float f, ld e, long long k, ld g, ld h, int j, ld l) {
    return a * i + b - d * c + f / e + k - g * h + j * l;
}
static struct wrap wrapped(ld x) { struct wrap w = { x * 3 }; return w; }
static struct mixed mix(struct mixed m) { m.x += m.n; m.n *= 2; return m; }
static ld unwrap(struct wrap w, struct wrap v) { return w.x - v.x; }
static _Complex long double cplx(ld r, ld i) { return __builtin_complex(r * 2, i - 1); }
static ld (*pointer)(ld, int, ld, double, ld, float, ld, long long, ld, ld, int, ld) = many;
static ld global = 2.75L;
static ld array[3] = { 1.0L, 2.0L, 4.0L };
static ld recurse(int n, ld x) { return n == 0 ? x : recurse(n - 1, x * 1.5L) + n; }
int main(void) {
    show(many(1.5L, 2, 3.25L, 0.5, 4.0L, 0.25f, 8.0L, 7, 1.25L, 2.0L, 3, 0.1L));
    show(pointer(1.5L, 2, 3.25L, 0.5, 4.0L, 0.25f, 8.0L, 7, 1.25L, 2.0L, 3, 0.1L));
    struct wrap w = wrapped(1.25L);
    show(w.x);
    struct mixed m = mix((struct mixed){ 0.5L, 3 });
    show(m.x);
    printf(" %d", m.n);
    show(unwrap(w, (struct wrap){ 0.125L }));
    _Complex long double z = cplx(1.5L, 2.5L);
    show(__real__ z);
    show(__imag__ z);
    global += array[2];
    show(global);
    show(recurse(10, 1.0L));
    printf("\n");
    return 0;
}"#,
    },
    Program {
        // Each unsigned conversion that is in range raises no "invalid"
        // exception, as C99 §F.4p1 has it: 2^63 and above too.
        name:   "long_double_unsigned_conversions",
        source: r#"int printf(const char *, ...);
int feclearexcept(int);
int fetestexcept(int);
typedef long double ld;
// FE_INVALID is 0x01 in glibc but 0x10 in MinGW-w64 13, whose 0x01 is
// FE_INEXACT (0x10 is glibc's FE_UNDERFLOW). 0/0 raises only "invalid",
// so the one of the two bits that it sets is FE_INVALID.
static int invalid;
static int find_invalid(void) {
    volatile double zero = 0.0;
    feclearexcept(0x11);
    volatile double quiet = zero / zero;
    (void)quiet;
    invalid = fetestexcept(0x11);
    return invalid == 0x01 || invalid == 0x10;
}
static unsigned long long u64(ld x) { return x; }
static unsigned u32(ld x) { return x; }
static unsigned short u16(ld x) { return x; }
static unsigned char u8(ld x) { return x; }
int main(void) {
    if (!find_invalid())
        return 2;
    volatile ld values[] = {
        0x1p63L, 0x1p64L - 0x1p11L, 0.0L, 0x1p63L - 1, 0x1p64L - 1, 0.75L, 1e18L, 0x1p63L + 0x1p62L,
    };
    for (int i = 0; i < 8; i++) {
        feclearexcept(invalid);
        unsigned long long r = u64(values[i]);
        int raised = fetestexcept(invalid) != 0;
        printf("%llu %d\n", r, raised);
    }
    volatile ld narrow[] = { 0.0L, 255.5L, 65535.75L, 0x1p31L, 4294967295.0L };
    for (int i = 0; i < 5; i++) {
        feclearexcept(invalid);
        unsigned r32 = u32(narrow[i]);
        unsigned r16 = i < 3 ? u16(narrow[i]) : 0;
        unsigned r8 = i < 2 ? u8(narrow[i]) : 0;
        printf("%u %u %u %d\n", r32, r16, r8, fetestexcept(invalid) != 0);
    }
    return 0;
}"#,
    },
    Program {
        // x87 loads and stores through indexed addresses whose base and
        // index are among more values live into the targets of an indirect
        // jump than there are registers.
        name:   "long_double_across_an_indirect_jump",
        source: r#"int printf(const char *, ...);
typedef long double ld;
volatile unsigned long long in[20];
ld wl[16];
ld *volatile lp = wl;
#define A(n) unsigned long long a##n = in[n];
static unsigned long long run(int sel) {
    A(0) A(1) A(2) A(3) A(4) A(5) A(6) A(7) A(8) A(9) A(10) A(11) A(12) A(13) A(14) A(15)
    ld *p = lp;
    unsigned long long r = 0;
    void *t = sel ? &&L1 : &&L0;
    goto *t;
L0: p[a0 & 15] = (ld)(a1 >> 2) / 3;
    wl[a2 & 15] = (ld)a3 * 0.25L;
    r += (unsigned long long)(p[a4 & 15] + wl[a5 & 15]);
    goto End;
L1: p[a6 & 15] = p[a7 & 15] * 2 + (ld)a8;
    r += (unsigned long long)(long long)(wl[a9 & 15] - (ld)(a10 & 0xffff));
    goto End;
End:
    return r + a0 + a1 + a2 + a3 + a4 + a5 + a6 + a7 + a8 + a9 + a10 + a11 + a12 + a13 + a14
        + a15;
}
int main(void) {
    for (int i = 0; i < 20; i++) in[i] = (unsigned long long)i * 0x10001ULL + 3;
    unsigned long long h = run(0);
    h = h * 31 + run(1);
    for (int i = 0; i < 16; i++) {
        unsigned long long words[2] = { 0, 0 };
        __builtin_memcpy(words, &wl[i], 10);
        h = h * 33 + words[0] + (words[1] & 0xffff);
    }
    printf("%llu\n", h);
    return 0;
}"#,
    },
];

/// Unsigned conversions from `double` and `float` that are in range, with
/// whether each raised the "invalid" exception (C99 §F.4p1). The LLVM back
/// end's conversions raise it for values at or above 2^63, so the output
/// is checked against [`SSE_UNSIGNED_CONVERSIONS_OUTPUT`] instead.
pub(super) const SSE_UNSIGNED_CONVERSIONS: Program = Program {
    name:   "sse_unsigned_conversions",
    source: r#"int printf(const char *, ...);
int feclearexcept(int);
int fetestexcept(int);
// FE_INVALID is 0x01 in glibc but 0x10 in MinGW-w64 13, whose 0x01 is
// FE_INEXACT (0x10 is glibc's FE_UNDERFLOW). 0/0 raises only "invalid",
// so the one of the two bits that it sets is FE_INVALID.
static int invalid;
static int find_invalid(void) {
    volatile double zero = 0.0;
    feclearexcept(0x11);
    volatile double quiet = zero / zero;
    (void)quiet;
    invalid = fetestexcept(0x11);
    return invalid == 0x01 || invalid == 0x10;
}
static unsigned long long from_double(double x) { return x; }
static unsigned long long from_float(float x) { return x; }
static unsigned from_double32(double x) { return x; }
int main(void) {
    if (!find_invalid())
        return 2;
    volatile double doubles[] = { 0x1p63, 0x1p64 - 0x1p11, 0.0, 0x1p63 - 0x1p10, 0.75, 1e18, 0x1p63 + 0x1p62 };
    for (int i = 0; i < 7; i++) {
        feclearexcept(invalid);
        unsigned long long r = from_double(doubles[i]);
        printf("%llu %d\n", r, fetestexcept(invalid) != 0);
    }
    volatile float floats[] = { 0x1p63f, 0x1p64f - 0x1p40f, 0.0f, 0x1p63f - 0x1p39f, 0.75f };
    for (int i = 0; i < 5; i++) {
        feclearexcept(invalid);
        unsigned long long r = from_float(floats[i]);
        printf("%llu %d\n", r, fetestexcept(invalid) != 0);
    }
    volatile double narrow = 4294967295.0;
    feclearexcept(invalid);
    unsigned r = from_double32(narrow);
    printf("%u %d\n", r, fetestexcept(invalid) != 0);
    return 0;
}"#,
};

/// What [`SSE_UNSIGNED_CONVERSIONS`] prints: Clang's values, and no
/// exception.
pub(super) const SSE_UNSIGNED_CONVERSIONS_OUTPUT: &str = "\
9223372036854775808 0
18446744073709549568 0
0 0
9223372036854774784 0
0 0
1000000000000000000 0
13835058055282163712 0
9223372036854775808 0
18446742974197923840 0
0 0
9223371487098961920 0
0 0
4294967295 0
";
