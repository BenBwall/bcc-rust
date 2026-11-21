/* The NaN of an invalid floating operation in the initializer of an object
 * with static storage duration, under the floating-point pragmas that make
 * Clang's code for a function strict, printed as bits. Such an initializer
 * is still computed at translation time (C99 §F.7.5p1), so GCC and Clang
 * store their arithmetic's positive quiet NaN whatever pragma is in effect:
 * Clang's `STDC FENV_ACCESS`, `clang fp exceptions` and `float_control`,
 * and GCC's `#pragma GCC optimize`, which each compiler alone reads. The
 * tests build it with no options, -ftrapping-math and -fno-trapping-math,
 * at -O0. Clang rejects these pragmas under -ffast-math. C99 §7.6.1. */
#include <stdio.h>
#include <string.h>

#define INF __builtin_inf()

static void show(const char *name, double value) {
    unsigned long long bits;
    memcpy(&bits, &value, sizeof bits);
    printf("%-24s %016llx\n", name, bits);
}

static void show_float(const char *name, float value) {
    unsigned bits;
    memcpy(&bits, &value, sizeof bits);
    printf("%-24s %08x\n", name, bits);
}

#pragma GCC optimize("trapping-math")
static double after_optimize = 0.0 / 0.0;

#pragma STDC FENV_ACCESS ON
static double fenv_access = INF - INF;
static float fenv_access_float = 0.0f * __builtin_inff();

#pragma clang fp exceptions(strict)
static double fp_exceptions = INF / INF;

#pragma float_control(push)
#pragma float_control(except, on)
static _Complex double float_control = (0.0 + 0.0i) / (0.0 + 0.0i);
#pragma float_control(pop)

int main(void) {
#pragma STDC FENV_ACCESS ON
    static double in_block = 0.0 / 0.0;
    static _Thread_local double thread_local_in_block = 0 * INF;
    show("optimize", after_optimize);
    show("FENV_ACCESS", fenv_access);
    show_float("FENV_ACCESS float", fenv_access_float);
    show("fp exceptions", fp_exceptions);
    show("float_control real", __real__ float_control);
    show("float_control imag", __imag__ float_control);
    show("block FENV_ACCESS", in_block);
    show("block _Thread_local", thread_local_in_block);
    return 0;
}
