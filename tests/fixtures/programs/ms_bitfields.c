/* Record layouts under System V's bit-field rules and, with MS defined,
   Microsoft's, as GCC 14, Clang 23 and TCC lay them out on x86-64; the
   ms_struct and gcc_struct attributes decide their own record. Compiles
   exactly when every layout matches. */
#ifdef MS
#define L(sysv, ms) (ms)
#else
#define L(sysv, ms) (sysv)
#endif
#ifdef __TINYC__
#define TCC(tcc, other) (tcc)
#else
#define TCC(tcc, other) (other)
#endif
#define LAYOUT(T, size, align) \
    _Static_assert(sizeof(T) == (size) && _Alignof(T) == (align), #T)
#define AT(T, m, offset) \
    _Static_assert(__builtin_offsetof(T, m) == (offset), #T "." #m)

struct A { char a:4; int b:8; char c; };
LAYOUT(struct A, L(4, 12), 4);
AT(struct A, c, L(2, 8));
struct B { int a:3; long long b:40; short c:5; };
LAYOUT(struct B, L(8, 24), 8);
struct C { char a; int :0; char b; };
LAYOUT(struct C, L(5, 2), 1);
AT(struct C, b, L(4, 1));
/* TCC ends a run only at a zero-width field of the run's own type. */
struct D { char a:2; int :0; char b:3; };
LAYOUT(struct D, L(5, TCC(2, 8)), L(1, TCC(1, 4)));
struct E { short a:9; short b:9; short c:9; };
LAYOUT(struct E, 6, 2);
/* Clang's Microsoft layout ignores a union bit-field's alignment. */
union U { char a:3; int b:20; };
#ifdef __clang__
LAYOUT(union U, 4, L(4, 1));
#else
LAYOUT(union U, 4, 4);
#endif
struct G { long long a:1; char b:7; long long c:1; };
LAYOUT(struct G, L(8, 24), 8);
/* TCC's Microsoft layout ignores packed. */
struct __attribute__((packed)) P { char a:4; int b:8; char c; };
LAYOUT(struct P, L(3, TCC(12, 6)), L(1, TCC(4, 1)));
#pragma pack(push, 2)
struct Q { char a:4; int b:20; char c:2; };
#pragma pack(pop)
LAYOUT(struct Q, L(4, 8), 2);
/* TCC ignores both attributes, which bcc's TCC personality still reads. */
#ifndef __TINYC__
struct __attribute__((gcc_struct)) H { char a:4; int b:8; char c; };
LAYOUT(struct H, 4, 4);
struct __attribute__((ms_struct)) I { char a:4; int b:8; char c; };
LAYOUT(struct I, 12, 4);
#endif
struct J { char a:3; short b:3; char c; long long d:5; };
LAYOUT(struct J, L(8, 16), 8);
AT(struct J, c, L(1, 4));
/* TCC lets _Bool and char bit-fields share no run. */
struct K { _Bool a:1; char b:1; };
LAYOUT(struct K, L(1, TCC(2, 1)), 1);
