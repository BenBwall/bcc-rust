/* GNU complex integer types across the System V x86-64 ABI: GCC and Clang
   pass and return each as a structure of its two parts (`_Complex char`
   in one byte pair of a register, `_Complex long` in two registers or on
   the stack whole), in structures, past the argument registers and as
   variadic arguments. Built whole, or as the callee half (-DCALLEE_ONLY)
   and the caller half (-DCALLER_ONLY) by different compilers; the caller
   prints what came back. */
#include <stdarg.h>
#include <stdio.h>
typedef _Complex char cc;
typedef _Complex short cs;
typedef _Complex int ci;
typedef _Complex unsigned cu;
typedef _Complex long cl;
typedef _Complex unsigned long long cull;
struct mixed { char c; cs s; ci i; };
struct wide { cl a; cc b; };
cc lib_cc(cc a, cc b, cc c, cc d, cc e, cc f, cc g);
cs lib_cs(int n, cs a, double d, cs b);
ci lib_ci(ci a, ci b, ci c, ci d, ci e, ci f, ci g);
cu lib_cu(cu a, float f, cu b);
cl lib_cl(int n, cl a, cl b, cl c);
cull lib_cull(cull a, cull b);
struct mixed lib_mixed(struct mixed m);
struct wide lib_wide(struct wide w);
long lib_va(int n, ...);
#ifndef CALLER_ONLY
cc lib_cc(cc a, cc b, cc c, cc d, cc e, cc f, cc g) { return a + b + c + d + e + f * 2 + g * 4; }
cs lib_cs(int n, cs a, double d, cs b) { return a * b + (cs)(n + (int)d); }
ci lib_ci(ci a, ci b, ci c, ci d, ci e, ci f, ci g) { return a * g - b + c + d + e + f; }
cu lib_cu(cu a, float f, cu b) { return a - b + (cu)(unsigned)f; }
cl lib_cl(int n, cl a, cl b, cl c) { return a + b * c + n; }
cull lib_cull(cull a, cull b) { return a * b; }
struct mixed lib_mixed(struct mixed m) { m.s = ~m.s; m.i *= m.i; m.c++; return m; }
struct wide lib_wide(struct wide w) { w.a = -w.a; w.b += 1; return w; }
long lib_va(int n, ...) {
    va_list ap;
    va_start(ap, n);
    long sum = 0;
    for (int i = 0; i < n; i++) {
        if (i % 3 == 0) {
            ci z = va_arg(ap, ci);
            sum += __real__ z + 10 * __imag__ z;
        } else if (i % 3 == 1) {
            cl z = va_arg(ap, cl);
            sum += 100 * __real__ z + 1000 * __imag__ z;
        } else {
            sum += 10000 * va_arg(ap, int);
        }
    }
    va_end(ap);
    return sum;
}
#endif
#ifndef CALLEE_ONLY
#define SHOW(z) printf("%lld %lld\n", (long long)__real__ (z), (long long)__imag__ (z))
int main(void) {
    SHOW(lib_cc(1 + 2i, 3 + 4i, 5 + 6i, 7 + 8i, 9 + 10i, 11 + 12i, 13 + 14i));
    SHOW(lib_cs(5, 3 + 2i, 2.5, 7 - 1i));
    SHOW(lib_ci(1 + 1i, 2 - 3i, 30 + 4i, 5 + 50i, 6 + 7i, 8 + 9i, 100 - 100i));
    SHOW(lib_cu(4000000000u + 7i, 2.75f, 1 + 9i));
    SHOW(lib_cl(1, 1000000000000 + 2i, 3 + 4i, 5000000000 - 6i));
    SHOW(lib_cull(0xffffffffffffull + 3i, 2 + 0x100000000ull * 1i));
    struct mixed m = lib_mixed((struct mixed){'a', 2 + 3i, 4 + 5i});
    printf("%c ", m.c);
    SHOW(m.s);
    SHOW(m.i);
    struct wide w = lib_wide((struct wide){-7 + 8i, 120 + 1i});
    SHOW(w.a);
    SHOW(w.b);
    printf("%ld\n", lib_va(7, (ci)(1 + 2i), (cl)(3 + 4i), 5, (ci)(6 + 7i), (cl)(8 + 9i), 10, (ci)(11 + 12i)));
    return 0;
}
#endif
