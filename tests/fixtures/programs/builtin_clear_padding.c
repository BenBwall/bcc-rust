/* GCC's __builtin_clear_padding (GCC manual, "Other Builtins"): each object is
   filled with ones, cleared, and compared with the bytes GCC 14.2 leaves, as
   hexadecimal text that repeats to the object's size. GCC 14.2 and Clang 23.1
   both run this program to "failures: 0"; every array here is spelled directly
   because Clang leaves one alone that a typedef, typeof or parentheses spelled.
   Under __clang__, the expected bytes are Clang's where the two differ. */

int printf(const char *, ...);

#ifdef __wasm__
#define LD_VALUE "ffffffffffffffffffff"
#define LD_REST "ffffffffffff"
#else
#define LD_VALUE "ffffffffffffffffffff"
#define LD_REST "000000000000"
#endif
#define LD LD_VALUE LD_REST
#define ZERO8 "0000000000000000"
#define ONES8 "ffffffffffffffff"
#define PAIR "ff00ffff"
#define PAIRS8 PAIR PAIR PAIR PAIR PAIR PAIR PAIR PAIR
#define PAIRS64 PAIRS8 PAIRS8 PAIRS8 PAIRS8 PAIRS8 PAIRS8 PAIRS8 PAIRS8

#ifdef __clang__
#define BOTH(gcc, clang) clang
#else
#define BOTH(gcc, clang) gcc
#endif

typedef int v4si __attribute__((vector_size(16)));
struct pair { char a; short b; };

static int failures;

static int digit(int c) { return c <= '9' ? c - '0' : c - 'a' + 10; }

/* Whether the object's bytes are `expected`, repeated. */
static void compare(const char *what, const void *object, unsigned long size,
                    const char *expected) {
    const unsigned char *bytes = object;
    unsigned long length = 0;
    int same = 1;
    while (expected[length]) {
        length++;
    }
    length /= 2;
    for (unsigned long i = 0; i < size; i++) {
        const char *text = expected + i % length * 2;
        same &= bytes[i] == digit(text[0]) * 16 + digit(text[1]);
    }
    if (!same || size % length != 0) {
        failures++;
        printf("FAIL %s:", what);
        for (unsigned long i = 0; i < size; i++) {
            printf(" %02x", bytes[i]);
        }
        printf("\n");
    }
}

#define CHECK(declaration, expected)                 \
    do {                                             \
        declaration;                                 \
        __builtin_memset(&x, 0xff, sizeof x);        \
        __builtin_clear_padding(&x);                 \
        compare(#declaration, &x, sizeof x, expected); \
    } while (0)

static void scalars(void) {
    CHECK(int x, "ffffffff");
    CHECK(_Bool x, "ff");
    CHECK(void *x, "ff");
    CHECK(float x, "ffffffff");
    CHECK(double x, ONES8);
    CHECK(long double x, LD);
    CHECK(_Complex double x, ONES8);
    CHECK(_Complex long double x, LD);
    CHECK(__int128 x, ONES8);
    CHECK(v4si x, ONES8);
    CHECK(_BitInt(3) x, "07");
    CHECK(unsigned _BitInt(9) x, "ff01");
    CHECK(_BitInt(17) x, "ffff0100");
    CHECK(_BitInt(33) x, "ffffffff01000000");
    CHECK(_BitInt(64) x, ONES8);
    CHECK(_BitInt(65) x, ONES8 "0100000000000000");
    CHECK(_BitInt(127) x, ONES8 "ffffffffffffff7f");
}

static void records(void) {
    CHECK(struct { int a; char b; } x, "ffffffffff000000");
    CHECK(struct { char a; int b; } x, "ff000000ffffffff");
    CHECK(struct { char a; struct { char p; long long q; } n; char z; } x,
          "ff" "00000000000000" "ff" "00000000000000" ONES8 "ff" "00000000000000");
    CHECK(struct { char a; union { int b; char c; }; struct { char d; long long e; }; } x,
          "ff000000ffffffff" "ff" "00000000000000" ONES8);
    CHECK(struct { char a; long double b; } x, "ff" "000000000000000000000000000000" LD);
    CHECK(struct { char a; _Complex long double b; } x,
          "ff" "000000000000000000000000000000" LD LD);
    CHECK(struct { char a; v4si b; } x,
          "ff" "000000000000000000000000000000" ONES8 ONES8);
    CHECK(struct { char a; _BitInt(9) b; } x, "ff00ff01");
    CHECK(struct { int a; char z[0]; char b; } x, "ffffffffff000000");
    CHECK(struct { char a; struct {} e; int b; } x, "ff000000ffffffff");
    CHECK(struct { char a __attribute__((aligned(8))); char b; } x, "ffff" "000000000000");
    CHECK(struct __attribute__((aligned(16))) { char a; } x,
          "ff" "000000000000000000000000000000");
}

static void packed_records(void) {
    CHECK(struct __attribute__((packed)) { char a; int b; char c; } x, "ffffffffffff");
    CHECK(struct __attribute__((packed)) { char a; long double b; char c; } x,
          "ff" LD "ff");
    CHECK(struct __attribute__((packed)) { int a : 3; long long b : 40; char c; } x,
          "ffffffffff07ff");
}

static void bit_fields(void) {
    CHECK(struct { int a : 3; int : 2; int b : 15; int : 14; int c : 1; int : 0; int : 3;
                   int d : 2; int : 3; int e : 13; int : 3; signed char f; } x,
          "e7ff0f000040000018ff1fff");
    CHECK(struct { unsigned a : 1; unsigned b : 1; } x, "03000000");
    CHECK(struct { long long a : 35; char c; } x, "ffffffff07ff0000");
    CHECK(struct { char c; int a : 4; short s; } x, "ff0fffff");
    CHECK(struct { _Bool a : 1; _Bool b : 1; } x, "03");
    CHECK(struct { int a : 3; int : 0; char c; } x, "07000000ff000000");
    CHECK(struct { char a : 3; char b : 7; char c : 3; } x, "077f07");
    CHECK(struct { long long a : 63; long long b : 3; } x,
          "ffffffffffffff7f" "0700000000000000");
    CHECK(struct { unsigned _BitInt(9) a : 5; char c; } x, "1fff");
    CHECK(struct { char a; __int128 b : 100; } x, "ffffffffffffffffffffffffff0f0000");
}

static void unions(void) {
    CHECK(union { int a; char b; } x, "ffffffff");
    CHECK(union { char a; struct { char p; int q; } s; } x, "ff000000ffffffff");
    CHECK(union { struct { char a; short b; } x; struct { short c; char d; } y; } x, "ffffffff");
    CHECK(union { long double a; int b; } x, LD);
    CHECK(union { _Complex long double a; char c; } x, LD LD);
    CHECK(union { char a[3]; int b : 5; long long c __attribute__((aligned(16))); } x,
          ONES8 ZERO8);
    CHECK(union { struct pair a[4]; char b; } x, PAIR);
    CHECK(union { struct { int a : 3; } s; } x, "07000000");
    CHECK(union { int : 7; } x, "00");
    /* A union too large to merge, which a member covers with value bits. */
    static union { char a[1 << 21]; struct { char p; int q; } s; } large;
    __builtin_memset(&large, 0xff, 64);
    __builtin_clear_padding(&large);
    compare("large", &large, 64, "ff");
    CHECK(union { int a : 13; } x, BOTH("ffff0000", "ff1f0000"));
    CHECK(union { _Bool a : 1; } x, BOTH("ff", "01"));
    CHECK(union { long long a : 35; } x, BOTH("ffffffffff000000", "ffffffff07000000"));
    CHECK(struct { char p; union { int a : 13; } u; char q; } x,
          BOTH("ff000000ffff0000ff000000", "ff000000ff1f0000ff000000"));
}

static void arrays(void) {
    CHECK(struct pair x[3], PAIR);
    CHECK(struct pair x[2][2], PAIR);
    CHECK(long double x[2], LD);
    CHECK(char x[1000], "ff");
    CHECK(struct { long double a; char b; } x[2],
          LD "ff" "000000000000000000000000000000");
    /* Enough elements for a loop rather than a fill for each. */
    CHECK(struct pair x[1000], PAIR);
    CHECK(struct pair x[40][30], PAIR);
    CHECK(struct { char a; struct pair b[64]; char c; } x, "ff00" PAIRS64 "ff00");
    CHECK(struct { struct pair a[64]; struct pair b[64]; } x, PAIRS64 PAIRS64);
    CHECK(struct { char a; short b; struct pair c[50]; } x[50], PAIR);
}

static void variable_length(int n, int m, struct pair (*p)[n], long double (*q)[m][n]) {
    struct pair a[n];
    struct pair b[n][2];
    struct pair c[2][n];
    __builtin_memset(&a, 0xff, sizeof a);
    __builtin_memset(&b, 0xff, sizeof b);
    __builtin_memset(&c, 0xff, sizeof c);
    __builtin_memset(p, 0xff, sizeof *p);
    __builtin_memset(q, 0xff, sizeof *q);
    __builtin_clear_padding(&a);
    __builtin_clear_padding(&b);
    __builtin_clear_padding(&c);
    __builtin_clear_padding(p);
    __builtin_clear_padding(q);
    compare("a[n]", &a, sizeof a, BOTH(PAIR, "ff"));
    compare("b[n][2]", &b, sizeof b, BOTH(PAIR, "ff"));
    compare("c[2][n]", &c, sizeof c, BOTH(PAIR, "ff"));
    compare("(*p)[n]", p, sizeof *p, BOTH(PAIR, "ff"));
    compare("(*q)[m][n]", q, sizeof *q, BOTH(LD, "ff"));
    /* A row of constant size is an array the operand spells directly. */
    __builtin_memset(&b, 0xff, sizeof b);
    __builtin_clear_padding(&b[0]);
    compare("b[0]", &b[0], sizeof b[0], PAIR);
    compare("b[1]", &b[n - 1], sizeof b[0], n > 1 ? "ff" : PAIR);
}

/* The value bits come through unchanged, written or not. */
static void values(void) {
    struct {
        char a;
        short b : 5;
        long long c : 40;
        long double d;
        _Complex long double e;
        union { char f; int g; } h;
    } s;
    __builtin_memset(&s, 0x55, sizeof s);
    s.a = 'a';
    s.b = -3;
    s.c = -123456789012;
    s.d = -18.5L;
    s.e = 2.5L + 3.5iL;
    s.h.g = 0x12345678;
    __builtin_clear_padding(&s);
    if (s.a != 'a' || s.b != -3 || s.c != -123456789012 || s.d != -18.5L
        || __real__ s.e != 2.5L || __imag__ s.e != 3.5L || s.h.g != 0x12345678) {
        failures++;
        printf("FAIL values\n");
    }
    /* An object whose value bits were never written may be cleared. */
    struct { int a : 3; int b; } u;
    u.b = 7;
    __builtin_clear_padding(&u);
    if (u.b != 7) {
        failures++;
        printf("FAIL unwritten\n");
    }
}

/* The operand is evaluated once, and a pointer to something without padding
   only for its effects. */
static void operands(void) {
    struct pair a[2];
    struct pair *p = a;
    int i = 5;
    int *q = &i;
    __builtin_memset(a, 0xff, sizeof a);
    __builtin_clear_padding(p++);
    if (p != a + 1 || a[0].b != -1) {
        failures++;
        printf("FAIL operand\n");
    }
    compare("&a[0]", &a[0], sizeof a[0], PAIR);
    compare("&a[1]", &a[1], sizeof a[1], "ff");
    __builtin_clear_padding(q++);
    if (q != &i + 1 || i != 5) {
        failures++;
        printf("FAIL scalar operand\n");
    }
    volatile struct pair v;
    __builtin_memset((void *)&v, 0xff, sizeof v);
    __builtin_clear_padding(&v);
    compare("volatile", (const void *)&v, sizeof v, PAIR);
#ifndef __clang__
    /* GCC takes a function, which has no padding. */
    __builtin_clear_padding(operands);
    /* Arrays decay. */
    char s[3] = "ab";
    __builtin_clear_padding(s);
    if (s[0] != 'a') {
        failures++;
        printf("FAIL decayed\n");
    }
#endif
}

int main(void) {
    struct pair p[3];
    long double q[2][3];
    scalars();
    records();
    packed_records();
    bit_fields();
    unions();
    arrays();
    variable_length(3, 2, &p, &q);
    variable_length(1, 2, (struct pair (*)[1])&p, (long double (*)[2][1])&q);
    values();
    operands();
    printf("failures: %d\n", failures);
    return failures != 0;
}
