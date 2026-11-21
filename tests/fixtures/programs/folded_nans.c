/* The NaN an invalid floating operation (0/0, inf - inf, 0 * inf, inf/inf)
 * folds to, printed as bits. GCC's and Clang's arithmetic give the positive
 * quiet NaN in a static initializer, where x86-64 gives its default NaN,
 * whose sign bit is set, at run time. In a function Clang folds the
 * operation too, while GCC, as math may trap, leaves it for run time; a
 * complex quotient is computed by `__divdc3` at run time under both. A NaN
 * operand passes through with its sign, and a negation flips it. The tests
 * also build it with -fno-trapping-math, where GCC folds real arithmetic in
 * a function but still calls `__muldc3` and `__divdc3`, and with Clang's
 * strict exceptions (-ftrapping-math), where
 * Clang computes the operation in a function at run time unless it
 * initializes an automatic array or structure whose whole initializer its
 * evaluator folds: none with a run-time member, the address of an automatic
 * object, or another member that rounds, overflows or divides by zero. */
#include <stdio.h>
#include <string.h>

#define INF __builtin_inf()

static void show_double(const char *name, double value) {
    unsigned long long bits;
    memcpy(&bits, &value, sizeof bits);
    printf("%-22s %016llx\n", name, bits);
}

static void show_float(const char *name, float value) {
    unsigned bits;
    memcpy(&bits, &value, sizeof bits);
    printf("%-22s %08x\n", name, bits);
}

static void show_long_double(const char *name, long double value) {
    unsigned long long low;
    unsigned short high;
    memcpy(&low, &value, sizeof low);
    memcpy(&high, (char *)&value + 8, sizeof high);
    printf("%-22s %04x%016llx\n", name, high, low);
}

static void show_complex(const char *name, _Complex double value) {
    unsigned long long bits[2];
    memcpy(bits, &value, sizeof bits);
    printf("%-22s %016llx %016llx\n", name, bits[0], bits[1]);
}

static void show_quad(const char *name, __float128 value) {
    unsigned long long bits[2];
    memcpy(bits, &value, sizeof bits);
    printf("%-22s %016llx%016llx\n", name, bits[1], bits[0]);
}

static void show_half(const char *name, _Float16 value) {
    unsigned short bits;
    memcpy(&bits, &value, sizeof bits);
    printf("%-22s %04x\n", name, bits);
}

static double d_div = 0.0 / 0.0, d_sub = INF - INF, d_mul = 0 * INF, d_quotient = INF / INF;
static double d_negated = -(0.0 / 0.0), d_plus = (0.0 / 0.0) + 1.0, d_minus = 1.0 - (0.0 / 0.0);
static double d_times = (0.0 / 0.0) * -1.0, d_negated_plus = -(0.0 / 0.0) + 1.0;
static double d_both = -(0.0 / 0.0) + (0.0 / 0.0), d_widened = (double)(0.0f / 0.0f);
static double d_nan = __builtin_nan(""), d_negative_nan = -__builtin_nan("");
static float f_div = 0.0f / 0.0f, f_sub = __builtin_inff() - __builtin_inff();
static float f_negated = -(0.0f / 0.0f), f_narrowed = (float)(0.0 / 0.0);
static long double l_div = 0.0L / 0.0L, l_mul = 0 * __builtin_infl();
static long double l_negated = -(0.0L / 0.0L), l_widened = (long double)(0.0 / 0.0);
static _Complex double c_product = (INF + 0.0i) * (0.0 + 1.0i);
static _Complex double c_scaled = (0.0 + INF * 1.0i) * 0.0;
static _Complex double c_quotient = (0.0 + 0.0i) / (0.0 + 0.0i);
static _Complex double c_sum = (INF + 1.0i) + (-INF + 1.0i);
static __float128 q_div = 0.0Q / 0.0Q, q_negated = -(0.0Q / 0.0Q);
static _Float16 h_div = (_Float16)0.0 / (_Float16)0.0;

int main(void) {
    puts("static");
    show_double("0/0", d_div);
    show_double("inf-inf", d_sub);
    show_double("0*inf", d_mul);
    show_double("inf/inf", d_quotient);
    show_double("-(0/0)", d_negated);
    show_double("(0/0)+1", d_plus);
    show_double("1-(0/0)", d_minus);
    show_double("(0/0)*-1", d_times);
    show_double("-(0/0)+1", d_negated_plus);
    show_double("-(0/0)+(0/0)", d_both);
    show_double("(double)(0f/0f)", d_widened);
    show_double("nan(\"\")", d_nan);
    show_double("-nan(\"\")", d_negative_nan);
    show_float("0f/0f", f_div);
    show_float("inff-inff", f_sub);
    show_float("-(0f/0f)", f_negated);
    show_float("(float)(0/0)", f_narrowed);
    show_long_double("0L/0L", l_div);
    show_long_double("0*infl", l_mul);
    show_long_double("-(0L/0L)", l_negated);
    show_long_double("(long double)(0/0)", l_widened);
    show_complex("(inf+0i)*(0+1i)", c_product);
    show_complex("(0+inf*i)*0", c_scaled);
    show_complex("(0+0i)/(0+0i)", c_quotient);
    show_complex("(inf+i)+(-inf+i)", c_sum);
    show_quad("0Q/0Q", q_div);
    show_quad("-(0Q/0Q)", q_negated);
    show_half("0f16/0f16", h_div);

    puts("automatic");
    double a_div = 0.0 / 0.0, a_sub = INF - INF, a_mul = 0 * INF, a_quotient = INF / INF;
    double a_negated = -(0.0 / 0.0), a_minus = 1.0 - (0.0 / 0.0);
    float a_float = 0.0f / 0.0f;
    long double a_long_double = 0.0L / 0.0L, a_long_double_sub = __builtin_infl() - __builtin_infl();
    _Complex double a_complex_quotient = (0.0 + 0.0i) / (0.0 + 0.0i);
    _Complex double a_complex_product = (INF + 0.0i) * (0.0 + 1.0i);
    _Complex double a_complex_sum = (INF + 1.0i) + (-INF + 1.0i);
    __float128 a_quad = 0.0Q / 0.0Q;
    static double a_static = 0.0 / 0.0;
    double a_literal = (double){0.0 / 0.0};
    double a_array[2] = {0.0 / 0.0, INF - INF};
    struct { int tag; double value; } a_struct = {1, 0 * INF};
    static volatile double one = 1.0;
    double run_time = one;
    double a_run_time[2] = {0.0 / 0.0, run_time};
    double a_inexact[2] = {1.0 / 3.0, 0.0 / 0.0};
    float a_rounded[2] = {0.1, 0.0f / 0.0f};
    struct { double *where; double value; } a_address = {&run_time, 0.0 / 0.0};
    double a_exact[3] = {0.5 + 0.25, -(0.0 / 0.0), (double)(0.0f / 0.0f)};
    show_double("0/0", a_div);
    show_double("inf-inf", a_sub);
    show_double("0*inf", a_mul);
    show_double("inf/inf", a_quotient);
    show_double("-(0/0)", a_negated);
    show_double("1-(0/0)", a_minus);
    show_float("0f/0f", a_float);
    show_long_double("0L/0L", a_long_double);
    show_long_double("infl-infl", a_long_double_sub);
    show_complex("(0+0i)/(0+0i)", a_complex_quotient);
    show_complex("(inf+0i)*(0+1i)", a_complex_product);
    show_complex("(inf+i)+(-inf+i)", a_complex_sum);
    show_quad("0Q/0Q", a_quad);
    show_double("static 0/0", a_static);
    show_double("(double){0/0}", a_literal);
    show_double("{0/0, ...}", a_array[0]);
    show_double("{..., inf-inf}", a_array[1]);
    show_double("{1, 0*inf}", a_struct.value);
    show_double("{0/0, run_time}", a_run_time[0]);
    show_double("{1/3, 0/0}", a_inexact[1]);
    show_float("{0.1f, 0f/0f}", a_rounded[1]);
    show_double("{&run_time, 0/0}", a_address.value);
    show_double("{.75, -(0/0), ...}", a_exact[1]);
    show_double("{..., (double)(0f/0f)}", a_exact[2]);
    return 0;
}
