/* The NaN an invalid floating operation (0/0, inf - inf, 0 * inf, inf/inf)
 * folds to in the initializer of an object with static storage duration,
 * printed as bits. GCC and Clang compute every such initializer at
 * translation time, under every floating-point option, and their arithmetic
 * gives the positive quiet NaN, where x86-64 gives its default NaN, whose
 * sign bit is set, at run time. A NaN operand passes through with its sign,
 * and a negation flips it. The tests build it with no options,
 * -fno-trapping-math, -ftrapping-math, -ffast-math and -fcx-limited-range.
 * In a function bcc computes every invalid operation as x86-64 does, which
 * GCC and Clang do only in some cases, so nothing here prints one.
 * C99 §6.7.8p4, §F.7.5p1. */
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

    puts("block-scope static");
    static double b_div = 0.0 / 0.0, b_mul = 0 * INF;
    static float b_float = __builtin_inff() / __builtin_inff();
    static long double b_long_double = __builtin_infl() - __builtin_infl();
    static _Complex double b_complex_quotient = (0.0 + 0.0i) / (0.0 + 0.0i);
    static __float128 b_quad = 0.0Q / 0.0Q;
    static _Thread_local double b_thread = 0.0 / 0.0;
    static double b_negated = -(INF - INF);
    show_double("0/0", b_div);
    show_double("0*inf", b_mul);
    show_float("inff/inff", b_float);
    show_long_double("infl-infl", b_long_double);
    show_complex("(0+0i)/(0+0i)", b_complex_quotient);
    show_quad("0Q/0Q", b_quad);
    show_double("_Thread_local 0/0", b_thread);
    show_double("-(inf-inf)", b_negated);
    return 0;
}
