/* Compound assignment, increment and decrement of _Atomic floating objects
   (C11 6.5.16.2p3): each attempt loads the object, computes in the type the
   usual arithmetic conversions give, converts back and compare-exchanges.
   Each line prints the expression's result, its type and the stored value.
   GCC 15 and Clang print the same; `atomic_compound_integers.c` holds the
   integer objects, which Clang miscompiles. */
#include <float.h>
#include <stdio.h>
#include <string.h>

#ifdef __SIZEOF_FLOAT128__
#define FLOAT128_NAME __float128: "__float128",
#else
#define FLOAT128_NAME
#endif
#define TYPE_NAME(e) _Generic((e), float: "float", double: "double", \
    long double: "long double", FLOAT128_NAME int *: "int *", default: "other")

static volatile double vd = 0.1;
static volatile float vf = 3.0f;
static volatile int vi = 7;
static volatile long long vll = -3;
static volatile long double vl = 0.3L;

static void show_f(const char *what, const char *type, double result, double stored) {
    printf("%s: %s %a %a\n", what, type, result, stored);
}
static void show_l(const char *what, const char *type, long double result, long double stored) {
#if LDBL_MANT_DIG == 64
    printf("%s: %s %La %La\n", what, type, result, stored);
#else
    printf("%s: %s %a %a\n", what, type, (double)result, (double)stored);
#endif
}
typedef double show_f_type;
typedef long double show_l_type;

/* Stores `init`, evaluates `expression` once and shows its result, its type
   and the value then stored. */
#define UPDATE(obj, show, init, expression, text)                          \
    do {                                                                    \
        obj = init;                                                         \
        show##_type result_ = (expression);                                 \
        show(text, TYPE_NAME(expression), result_, obj);                    \
    } while (0)

#define FLOATING(obj, show, init, rhs)                                      \
    UPDATE(obj, show, init, obj += rhs, #obj " += " #rhs);                  \
    UPDATE(obj, show, init, obj -= rhs, #obj " -= " #rhs);                  \
    UPDATE(obj, show, init, obj *= rhs, #obj " *= " #rhs);                  \
    UPDATE(obj, show, init, obj /= rhs, #obj " /= " #rhs);

#define STEPS(obj, show, init)                                              \
    UPDATE(obj, show, init, ++obj, "++" #obj);                              \
    UPDATE(obj, show, init, obj++, #obj "++");                              \
    UPDATE(obj, show, init, --obj, "--" #obj);                              \
    UPDATE(obj, show, init, obj--, #obj "--");

_Atomic float af;
_Atomic double ad;
_Atomic long double al;

static int calls;
static _Atomic float *fp(void) { ++calls; return &af; }
static double rhs(void) { ++calls; af = 2.0f; return 0.5; }

#ifdef __SIZEOF_FLOAT128__
static _Atomic __float128 aq;
typedef __float128 show_q_type;
/* Prints the binary128 encodings, which printf cannot. */
static void show_q(const char *what, const char *type, __float128 result, __float128 stored) {
    unsigned long long r[2], s[2];
    memcpy(r, &result, sizeof r);
    memcpy(s, &stored, sizeof s);
    printf("%s: %s %016llx%016llx %016llx%016llx\n", what, type, r[1], r[0], s[1], s[0]);
}
#endif

int main(void) {
    FLOATING(af, show_f, 1.5f, 0.25)
    FLOATING(af, show_f, 1.5f, vd)
    FLOATING(af, show_f, 0.1f, vf)
    FLOATING(af, show_f, 0.1f, vi)
    FLOATING(af, show_f, 0.1f, vll)
    FLOATING(af, show_f, 0.1f, vl)
    FLOATING(af, show_f, 16777216.0f, 1)
    FLOATING(ad, show_f, 1.5, 0.25f)
    FLOATING(ad, show_f, 0.1, vd)
    FLOATING(ad, show_f, 0.1, vi)
    FLOATING(ad, show_f, 0.1, vl)
    FLOATING(ad, show_f, 9007199254740992.0, 1)
    FLOATING(al, show_l, 1.5L, 0.25)
    FLOATING(al, show_l, 0.1L, vd)
    FLOATING(al, show_l, 0.1L, vf)
    FLOATING(al, show_l, 0.1L, vi)
    FLOATING(al, show_l, 0.1L, vl)
    FLOATING(al, show_l, 18446744073709551616.0L, 1)
    STEPS(af, show_f, 0.5f)
    STEPS(af, show_f, 16777216.0f)
    STEPS(ad, show_f, -0.5)
    STEPS(ad, show_f, 9007199254740992.0)
    STEPS(al, show_l, 0.1L)
    STEPS(al, show_l, 18446744073709551616.0L)
#ifdef __SIZEOF_FLOAT128__
    FLOATING(aq, show_q, 1.5Q, 0.1)
    FLOATING(aq, show_q, 0.1Q, vl)
    FLOATING(aq, show_q, 0.1Q, vi)
    STEPS(aq, show_q, 0.1Q)
#else
    puts("no __float128");
#endif

    /* Pointers step by an element. */
    {
        int array[8] = {0};
        static _Atomic(int *) ap;
        int *p;
        ap = array + 1; p = ap++;
        printf("ap++: %s %td %td\n", TYPE_NAME(ap++), p - array, ap - array);
        p = ++ap; printf("++ap: %td %td\n", p - array, ap - array);
        p = ap--; printf("ap--: %td %td\n", p - array, ap - array);
        p = --ap; printf("--ap: %td %td\n", p - array, ap - array);
    }

    /* The lvalue and the right operand are evaluated once. The update
       reads the object after the right operand's call, as GCC, Clang and
       bcc order it. */
    calls = 0; af = 1.0f;
    double r = (*fp() *= 3.5);
    printf("calls %d %a %a\n", calls, r, (double)af);
    calls = 0; af = 1.0f;
    r = (*fp())++;
    printf("calls %d %a %a\n", calls, r, (double)af);
    calls = 0; af = 1.0f;
    r = (*fp() += rhs());
    printf("calls %d %a %a\n", calls, r, (double)af);

    /* The result is the stored value, of the object's type. */
    af = 0.1f;
    printf("chain %a\n", (double)((af += 0.2) * 3.0));
    af = 1.0f;
    printf("compare %d\n", (af += 1e-10) == 1.0);
    printf("sizes %zu %zu %d\n", sizeof(af += 1.0), sizeof(ad -= 1.0f),
           sizeof(al *= 2) == sizeof(long double));
    return 0;
}
