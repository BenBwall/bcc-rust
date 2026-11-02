/* C23 `constexpr` objects: named constants in integer constant
   expressions, static initializers and code, at file and block scope.
   GCC also takes `constexpr` compound literals and a cast of a floating
   named constant as an integer constant expression; the bundled Clang
   parses no such literal and folds the cast as an extension. */
#include <stdio.h>

constexpr int K = 47;
enum { A = K, B = K * 2 };
constexpr int L = K;
static int b = K + 1;
int array[K];

struct S { int a; double d; char c[4]; int *p; unsigned u : 3; };
constexpr struct S s = { 7, 2.5, "ab", nullptr, 5 };
constexpr struct S s2 = s;
struct S s3 = s2;
static struct S s4 = s;

union U { int i; float f; };
constexpr union U u = { .f = 1.5f };
static union U u2 = u;

constexpr double D = 0.5;
constexpr long double LD = 1.25L;
constexpr _Complex double CD = 1.0 + 2.0i;
constexpr bool BO = true;
constexpr char str[] = "hello";
constexpr unsigned char ustr[] = u8"\xff\x01";
static int *np = s.p;
static double dd = D * 4;
constexpr int arr[3] = { 1, 2, 3 };
struct Bits { int x : s.a; };
alignas(K > 40 ? 16 : 8) int aligned;
static const int *pk = &K;

static const int *addr_k(void) { return &K; }

static_assert(s.a == 7);
static_assert(s.u == 5 && s2.u == 5);

/* Bit-field members are named constants with their width's value. */
struct F { unsigned x : 3; int y : 3; bool b : 1; };
constexpr struct F f = { 6, -3, 1 };
enum { FX = f.x, FY = f.y };
static int fa[f.x + f.y];
static int fs = f.x * 10 + f.b;
static_assert(sizeof array == K * sizeof(int));

static int get(int i) { return arr[i]; }

int main(void) {
    constexpr int local = 3;
    static int sl = local * 10;
    constexpr struct S ls = { local, D, "xy", 0, 1 };
    struct S copy = ls;
    int va[local];
    switch (K) {
    case K: puts("case K"); break;
    default: puts("default");
    }
    switch (2) {
    case (int)s.d: puts("case s.d"); break;
    }
    printf("%d %d %d %d %zu\n", K, A, B, L, sizeof array / sizeof array[0]);
    printf("%d %g %s %d %u\n", s.a, s.d, s.c, s.p == 0, s.u);
    printf("%d %g %s %u\n", s2.a, s2.d, s2.c, s2.u);
    printf("%d %g %s %u\n", s3.a, s3.d, s3.c, s3.u);
    printf("%d %g %s %u\n", s4.a, s4.d, s4.c, s4.u);
    printf("%g %g\n", u.f, u2.f);
    printf("%g %g %g %g %d\n", D, (double)LD, __real__ CD, __imag__ CD, BO);
    printf("%s %zu %d %d\n", str, sizeof str, ustr[0], ustr[1]);
    printf("%d %g %d %d\n", np == 0, dd, get(2), b);
    printf("%d %d %zu %d %g %s\n", local, sl, sizeof va / sizeof va[0], copy.a, copy.d,
           copy.c);
    printf("%zu %d %d %d\n", sizeof(struct Bits), *pk, *addr_k(), pk == addr_k());
    printf("%zu\n", _Alignof(aligned));
    printf("%d %d %zu %d %u %d %d\n", FX, FY, sizeof fa / sizeof fa[0], fs, f.x, f.y, f.b);
#ifndef __clang__
    enum { E = (constexpr struct S){ 9, 1.0, "", 0, 0 }.a };
    const int *literal = &(constexpr int){ 12 };
    printf("%d %d %d\n", E, (constexpr int){ 5 } + 1, *literal);
#endif
    return 0;
}
