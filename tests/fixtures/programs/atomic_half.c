/* Compound assignment, increment and decrement of _Atomic 16-bit floating
   objects (C11 6.5.16.2p3), with each compiler's excess precision: GCC
   keeps a `float` excess in the result, Clang rounds it. HALF names the
   type: `_Float16`, `__bf16` or Clang's `__fp16`. Each line prints the
   result's size and value and the stored value. */
#include <stdio.h>

#ifndef HALF
#define HALF _Float16
#endif

static volatile double vd = 0.1;
static volatile float vf = 3.0f;
static volatile int vi = 7;
static volatile HALF vh = (HALF)0.3;
static volatile long double vl = 0.3L;

static _Atomic HALF ah;
static HALF plain;

#define SHOW(text, expression)                                             \
    do {                                                                    \
        ah = (HALF)1.5; plain = (HALF)1.5;                                  \
        double result_ = (expression);                                      \
        printf("%s: %zu %a %a\n", text, sizeof(expression), result_, (double)ah); \
    } while (0)

/* The atomic object computes as a plain one does, so each line also prints
   the plain object's result. */
#define BOTH(op, rhs)                                                       \
    SHOW("ah " #op " " #rhs, ah op rhs);                                    \
    SHOW("plain " #op " " #rhs, plain op rhs);

#define EVERY(rhs) BOTH(+=, rhs) BOTH(-=, rhs) BOTH(*=, rhs) BOTH(/=, rhs)

int main(void) {
    EVERY(0.1)
    EVERY(0.1f)
    EVERY(vd)
    EVERY(vf)
    EVERY(vi)
    EVERY(vh)
    EVERY(vl)
    EVERY(2049)  /* rounds to 2048 as a half */
    SHOW("++ah", ++ah);
    SHOW("ah++", ah++);
    SHOW("--ah", --ah);
    SHOW("ah--", ah--);
    /* The result in a wider expression: excess precision of the result. */
    SHOW("(ah += 0.1) * 3", (ah += 0.1) * 3);
    SHOW("(plain += 0.1) * 3", (plain += 0.1) * 3);
    SHOW("(ah += vh) + vh", (ah += vh) + vh);
    SHOW("(plain += vh) + vh", (plain += vh) + vh);
    SHOW("(ah *= vh) == (HALF)0.45", (ah *= vh) == (HALF)0.45);
    SHOW("(plain *= vh) == (HALF)0.45", (plain *= vh) == (HALF)0.45);
    /* Repeated small steps, rounding each time. */
    ah = (HALF)2048;
    for (int i = 0; i < 4; ++i) ah += 1;
    printf("steps %a\n", (double)ah);
    ah = (HALF)2048;
    for (int i = 0; i < 4; ++i) ah++;
    printf("increments %a\n", (double)ah);
    /* Integer objects with a half operand: both compilers convert the
       object straight to `float`, compute there and convert the result
       straight back, so 2047 and 4097 do not round to a half first. */
    static volatile HALF three = (HALF)3.0;
    static int pi;
    static _Atomic int ai;
    static _Atomic unsigned char auc;
    static _Atomic _Bool ab;
#define INTEGER(obj, init, op, rhs)                                         \
    obj = init; printf(#obj " = " #init ", " #obj " " #op " " #rhs ": %d", obj op rhs); \
    printf(" %d\n", obj);
#define EVERY_INTEGER(obj)                                                  \
    INTEGER(obj, 2047, +=, vh) INTEGER(obj, 4097, -=, vh)                   \
    INTEGER(obj, 2047, *=, vh) INTEGER(obj, 4097, *=, vh)                   \
    INTEGER(obj, 2047, /=, three) INTEGER(obj, -4097, /=, three)
    EVERY_INTEGER(pi)
    INTEGER(ai, 2047, *=, vh) INTEGER(ai, 4097, *=, vh)
    INTEGER(ai, 2047, /=, three)
    INTEGER(auc, 200, *=, vh) INTEGER(ab, 1, *=, vh)
#ifndef __clang__
    /* Clang 23 computes an atomic integer's `+=` and `-=` of a floating
       operand in the integer type, and converts a negative atomic integer
       to `float` as an unsigned one. */
    INTEGER(ai, 2047, +=, vh) INTEGER(ai, 4097, -=, vh)
    INTEGER(ai, -4097, /=, three)
    INTEGER(auc, 200, -=, vh) INTEGER(ab, 0, +=, vh)
#endif
    return 0;
}
