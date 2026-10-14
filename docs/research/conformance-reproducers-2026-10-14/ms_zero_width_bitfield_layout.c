/* Windows (Microsoft bit-field layout): a zero-width bit-field ends the
   current allocation unit; the next bit-field starts after the whole unit.
   Clang and MinGW GCC agree on every line; bcc differs on each. */
#include <stdio.h>
#include <string.h>
struct N1 { int a : 5; short : 0; char b : 6; };
struct N2 { long long a : 15; unsigned char : 0; int b; };
#pragma pack(push, 1)
struct P1 { char a; unsigned b : 18; unsigned : 0; unsigned c : 4; };
struct P2 { long long a : 27; char : 0; };
#pragma pack(4)
struct P3 { long long a : 9; _Bool : 0; short b; };
#pragma pack(pop)
struct A1 { char a; unsigned b : 18; unsigned : 0; unsigned c : 4; } __attribute__((packed));
#define FIRST(T, f) do { struct T s; unsigned char x[sizeof s]; memset(&s, 0, sizeof s); s.f = 1; \
    memcpy(x, &s, sizeof s); int at = -1; for (unsigned i = 0; i < sizeof s; i++) if (x[i]) { at = (int)i; break; } \
    printf(#T " size %d, " #f " in byte %d\n", (int)sizeof s, at); } while (0)
int main(void) {
    FIRST(N1, b); FIRST(N2, b); FIRST(P1, c); FIRST(P2, a); FIRST(P3, b); FIRST(A1, c);
    return 0;
}
