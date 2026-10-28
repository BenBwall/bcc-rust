/* Compound assignment, increment and decrement of _Atomic integer and
   pointer objects with operands of other types (C11 6.5.16.2p3): the
   computation uses the usual arithmetic conversions, so `ai -= 0.5`
   subtracts in double. Each line prints the expression's result, its type
   and the stored value. GCC 15 is the reference for every identity: the
   bundled Clang 23 computes an integer object's `+=` and `-=` of a floating
   operand in the integer type, zero-extends narrow signed objects in its
   compare-exchange loops, stores 255 for `--` of a `_Bool` 0 and rejects
   compound assignment of an atomic pointer. */
#include <stdio.h>

#define TYPE_NAME(e) _Generic((e), int: "int", unsigned: "unsigned", \
    long long: "long long", char: "char", signed char: "signed char", \
    unsigned char: "unsigned char", short: "short", _Bool: "_Bool", \
    unsigned long: "unsigned long", int *: "int *", char *: "char *", \
    default: "other")

static volatile double vd = 0.1;
static volatile int vi = 7;
static volatile long double vl = 0.3L;

static void show_i(const char *what, const char *type, long long result, long long stored) {
    printf("%s: %s %lld %lld\n", what, type, result, stored);
}

/* Stores `init`, evaluates `expression` once and shows its result, its type
   and the value then stored. */
#define UPDATE(obj, init, expression, text)                                \
    do {                                                                    \
        obj = init;                                                         \
        long long result_ = (expression);                                   \
        show_i(text, TYPE_NAME(expression), result_, obj);                  \
    } while (0)

#define ARITHMETIC(obj, init, rhs)                                          \
    UPDATE(obj, init, obj += rhs, #obj " += " #rhs);                        \
    UPDATE(obj, init, obj -= rhs, #obj " -= " #rhs);                        \
    UPDATE(obj, init, obj *= rhs, #obj " *= " #rhs);                        \
    UPDATE(obj, init, obj /= rhs, #obj " /= " #rhs);

/* The other operators, without the left shift a negative value must not
   take. */
#define BITWISE(obj, init, rhs)                                             \
    UPDATE(obj, init, obj %= rhs, #obj " %= " #rhs);                        \
    UPDATE(obj, init, obj >>= rhs, #obj " >>= " #rhs);                      \
    UPDATE(obj, init, obj &= rhs, #obj " &= " #rhs);                        \
    UPDATE(obj, init, obj |= rhs, #obj " |= " #rhs);                        \
    UPDATE(obj, init, obj ^= rhs, #obj " ^= " #rhs);

#define SHIFT(obj, init, rhs) UPDATE(obj, init, obj <<= rhs, #obj " <<= " #rhs);

#define STEPS(obj, init)                                                    \
    UPDATE(obj, init, ++obj, "++" #obj);                                    \
    UPDATE(obj, init, obj++, #obj "++");                                    \
    UPDATE(obj, init, --obj, "--" #obj);                                    \
    UPDATE(obj, init, obj--, #obj "--");

static _Atomic int ai;
static _Atomic unsigned au;
static _Atomic long long all;
static _Atomic char ac;
static _Atomic unsigned char auc;
static _Atomic short as;
static _Atomic _Bool ab;
static _Atomic unsigned long aul;

int main(void) {
    /* Floating operands: the computation is floating, the result truncated
       (C99 6.3.1.4p1); every value is in range. */
    ARITHMETIC(ai, 7, 0.5)
    ARITHMETIC(ai, -7, 2.5f)
    ARITHMETIC(ai, 1000000, vd)
    ARITHMETIC(ai, 3, vl)
    ARITHMETIC(au, 10u, 0.75)
    ARITHMETIC(all, 1LL << 60, 3.0)
    ARITHMETIC(all, 123456789012LL, vl)
    ARITHMETIC(ac, 50, 1.5)
    ARITHMETIC(auc, 100, 0.5f)
    ARITHMETIC(as, -300, 2.25)
    ARITHMETIC(aul, 4000000000ul, 0.5)
    UPDATE(ab, 0, ab += 0.5, "ab += 0.5");
    UPDATE(ab, 1, ab -= 0.5, "ab -= 0.5");
    UPDATE(ab, 1, ab *= 0.25, "ab *= 0.25");
    UPDATE(ab, 1, ab /= 4.0, "ab /= 4.0");
    UPDATE(ab, 0, ab -= 0.0, "ab -= 0.0");

    /* Integer operands of other types. */
    ARITHMETIC(ai, 7, 3LL)
    ARITHMETIC(ac, 100, 100)
    ARITHMETIC(auc, 250, 10u)
    ARITHMETIC(au, 1u, -2)
    ARITHMETIC(as, -2, 3)
    BITWISE(ai, 1000, 3)
    SHIFT(ai, 1000, 3)
    BITWISE(ac, 100, 2LL)
    SHIFT(ac, 100, 2LL)
    BITWISE(auc, 0x81, 1u)
    SHIFT(auc, 0x81, 1u)
    BITWISE(all, -12345, 4)
    BITWISE(all, 12345, 4)
    SHIFT(all, 12345, 4)
    BITWISE(as, -2, 3)
    BITWISE(as, 300, 7)
    SHIFT(as, 300, 7)
    UPDATE(ab, 1, ab &= 2, "ab &= 2");
    UPDATE(ab, 0, ab |= 4, "ab |= 4");
    UPDATE(ab, 1, ab <<= 1, "ab <<= 1");
    STEPS(ai, 2147483647 - 1)
    STEPS(ac, 0)
    STEPS(auc, 255)
    STEPS(as, -1)
    STEPS(ab, 0)
    STEPS(ab, 1)

    /* Pointers add elements, or bytes for `char *`. */
    {
        int array[8] = {0};
        static _Atomic(int *) ap;
        static _Atomic(char *) cp;
        int *p;
        ap = array + 2; p = (ap += 3);
        printf("ap += 3: %s %td %td\n", TYPE_NAME(ap += 3), p - array, ap - array);
        ap = array + 6; p = (ap -= 4);
        printf("ap -= 4: %td %td\n", p - array, ap - array);
        ap = array + 6; p = (ap -= 4LL);
        printf("ap -= 4LL: %td %td\n", p - array, ap - array);
        ap = array + 1; p = (ap += vi);
        printf("ap += vi: %td %td\n", p - array, ap - array);
        cp = (char *)array; p = (int *)(cp += 8);
        printf("cp += 8: %s %td %td\n", TYPE_NAME(cp += 8), p - array, cp - (char *)array);
    }
    return 0;
}
