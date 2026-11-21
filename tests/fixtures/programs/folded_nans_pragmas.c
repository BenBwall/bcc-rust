/* The NaN of an invalid floating operation in functions whose
 * floating-point environment a pragma or attribute may change, printed as
 * bits. Clang computes the operation at run time (the x86-64 default NaN,
 * sign bit set) where `#pragma clang fp exceptions(strict|maytrap)`,
 * `#pragma float_control(except, on)` or `#pragma STDC FENV_ACCESS ON` is
 * in effect, and folds it to the positive NaN elsewhere; `float_control`'s
 * `push` and `pop` save and restore the whole state, and a pragma at file
 * scope overrides -ftrapping-math. A pragma in a compound statement makes
 * Clang compute every operation of the function at run time at -O0. GCC
 * ignores those pragmas, but `optimize("trapping-math")`, as an attribute
 * on any declaration or a pragma in effect at one, keeps the operation for
 * run time under -fno-trapping-math. Clang ignores the GCC forms. The tests
 * build it with no options, -ftrapping-math and -fno-trapping-math, at
 * -O0. C99 §7.6.1. */
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

#define BODY(name)                                                          \
    {                                                                       \
        double quotient = 0.0 / 0.0;                                        \
        show(name " quotient", quotient);                                   \
        show(name " negated", -(INF - INF));                                \
        show_float(name " float", 0.0f * __builtin_inff());                 \
        double members[2] = {INF / INF, x};                                 \
        show(name " members", members[0]);                                  \
    }

static void plain(double x) BODY("plain")
#pragma clang fp exceptions(strict)
static void fp_strict(double x) BODY("fp-strict")
#pragma clang fp exceptions(ignore)
static void fp_ignore(double x) BODY("fp-ignore")
#pragma clang fp exceptions(maytrap)
static void fp_maytrap(double x) BODY("fp-maytrap")
#pragma float_control(except, off)
static void control_off(double x) BODY("control-off")
#pragma float_control(except, on, push)
static void control_pushed(double x) BODY("control-pushed")
#pragma float_control(pop)
static void control_popped(double x) BODY("control-popped")
#pragma float_control(push)
#pragma STDC FENV_ACCESS ON
static void fenv_pushed(double x) BODY("fenv-pushed")
#pragma float_control(pop)
static void fenv_popped(double x) BODY("fenv-popped")
#pragma STDC FENV_ACCESS ON
#pragma clang fp exceptions(ignore)
static void fenv_ignore(double x) BODY("fenv-ignore")
#pragma STDC FENV_ACCESS OFF

static void in_block(double x) {
    double before = 0.0 / 0.0;
    show("in-block before", before);
    {
#pragma clang fp exceptions(strict)
        double inside = 0.0 / 0.0;
        show("in-block inside", inside);
    }
}

static void after_block(double x) BODY("after-block")

__attribute__((optimize("trapping-math"))) static void
optimize_attribute(double x) BODY("optimize-attribute")

static void optimize_declared(double x) __attribute__((optimize("trapping-math")));
static void optimize_declared(double x) BODY("optimize-declared")

#pragma GCC push_options
#pragma GCC optimize("trapping-math")
static void optimize_pragma(double x) BODY("optimize-pragma")
static void optimize_pragma_declared(double x);
#pragma GCC pop_options
static void optimize_pragma_declared(double x) BODY("optimize-pragma-declared")
static void options_popped(double x) BODY("options-popped")

int main(int argc, char **argv) {
    (void)argv;
    double x = argc;
    plain(x);
    fp_strict(x);
    fp_ignore(x);
    fp_maytrap(x);
    control_off(x);
    control_pushed(x);
    control_popped(x);
    fenv_pushed(x);
    fenv_popped(x);
    fenv_ignore(x);
    in_block(x);
    after_block(x);
    optimize_attribute(x);
    optimize_declared(x);
    optimize_pragma(x);
    optimize_pragma_declared(x);
    options_popped(x);
    return 0;
}
