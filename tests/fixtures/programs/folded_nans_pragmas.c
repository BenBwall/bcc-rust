/* The NaN of an invalid floating operation in a translation unit that holds
 * floating-point pragmas, printed as bits. bcc does not follow where such a
 * pragma is in effect: one anywhere in the translation unit computes every
 * function's invalid operations at run time (the x86-64 default NaN, sign
 * bit set) under the rules of a compiler that reads it, while a static
 * initializer still folds to the positive NaN. Clang reads `clang fp
 * exceptions`, `float_control` and `STDC FENV_ACCESS`, and GCC `#pragma GCC
 * optimize`. So that the compilers compute every function's operations at
 * run time too, the first pragma of each kind makes them strict for the
 * rest of the file: Clang's comes after a declaration's attributes, where
 * it still reaches every later function, and GCC's turns trapping math back
 * on under -fno-trapping-math. The later pragmas keep it strict, in the
 * forms bcc once scoped itself: after `__extension__`, through `_Pragma`, in
 * a block, and around `float_control`'s `push` and `pop`. A function where
 * Clang or GCC would fold the NaN, and bcc gives up that precision, is in
 * the unit tests (`folded_nans.rs`) instead. The tests build it with no
 * options, -ftrapping-math and -fno-trapping-math, at -O0. C99 §7.6.1. */
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

static double static_quotient = 0.0 / 0.0;

#define BODY(name)                                                          \
    {                                                                       \
        static double kept = INF - INF;                                     \
        double quotient = 0.0 / 0.0;                                        \
        show(name " static", kept);                                         \
        show(name " quotient", quotient);                                   \
        show(name " negated", -(INF - INF));                                \
        show_float(name " float", 0.0f * __builtin_inff());                 \
        double members[2] = {INF / INF, x};                                 \
        show(name " members", members[0]);                                  \
    }

#pragma GCC optimize("trapping-math")

__attribute__((noinline))
#pragma clang fp exceptions(strict)
static void after_attributes(double x) BODY("after-attributes")

__extension__
#pragma float_control(except, on)
static void after_extension(double x) BODY("after-extension")

#pragma float_control(push)
#pragma STDC FENV_ACCESS ON
static void fenv_pushed(double x) BODY("fenv-pushed")
#pragma float_control(pop)
static void fenv_popped(double x) BODY("fenv-popped")

#define STRICT _Pragma("clang fp exceptions(strict)")
static void in_block(double x) {
    {
        STRICT
        double inside = 0.0 / 0.0;
        show("in-block inside", inside);
    }
    double after = 0.0 / 0.0;
    show("in-block after", after);
    (void)x;
}

#pragma STDC FENV_ACCESS OFF
static void fenv_off(double x) BODY("fenv-off")

int main(int argc, char **argv) {
    (void)argv;
    double x = argc;
    show("static", static_quotient);
    after_attributes(x);
    after_extension(x);
    fenv_pushed(x);
    fenv_popped(x);
    in_block(x);
    fenv_off(x);
    return 0;
}
