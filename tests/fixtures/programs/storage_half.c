/* Clang's __fp16 on every back end: binary16 storage whose values promote
   to float before any operation (and to double through ...), which no
   parameter or result may have. Assignment, ++ and compound assignment
   round once to binary16. Only Clang provides the type on x86-64, so the
   test compares the program with Clang's build alone. */
#include <stdio.h>
#include <string.h>

#define T(e) _Generic((e), __fp16: "fp16", _Float16: "f16", float: "float", double: "double", int: "int", default: "other")

__fp16 g = 0.1, table[3] = {1, 2.5, 65520.0};
struct S {
    char c;
    __fp16 h;
} s = {1, 3.0};
volatile __fp16 v = 1.5;
_Float16 f16 = 2;

static unsigned bits(const __fp16 *h) {
    unsigned short u;
    memcpy(&u, h, 2);
    return u;
}

static double sum(int n, ...) {
    __builtin_va_list ap;
    __builtin_va_start(ap, n);
    double total = 0;
    for (int i = 0; i < n; i++)
        total += __builtin_va_arg(ap, double);
    __builtin_va_end(ap);
    return total;
}

static float load(const __fp16 *p) { return *p * 2; }
static void store(__fp16 *p, double d) { *p = d; }

int main(void) {
    __fp16 a = 0.1;
    __fp16 b = a * 3;
    printf("%s %s %s %s %s %s %s\n", T(a), T(+a), T(-a), T(a + a), T(a + f16), T(a + 1),
           T(a ? a : a));
    printf("%s %s %s\n", T(a = 2), T(a++), T((0, a)));
    printf("%zu %zu %zu %zu\n", sizeof(__fp16), _Alignof(__fp16), sizeof(s), sizeof(a + a));
    printf("%a %a %a %04x %04x %04x\n", (double)a, (double)b, table[1] + table[2], bits(&a),
           bits(&table[2]), bits(&g));
    b = 1e-8;
    printf("tiny %04x\n", bits(&b));
    double d = 0x1.0020000000001p0;
    b = d;
    printf("once %04x\n", bits(&b));
    int i = v * 3;
    printf("integer %d\n", i);
    b += 1;
    b *= f16;
    printf("compound %04x\n", bits(&b));
    b = 65519.0;
    printf("largest %04x %d\n", bits(&b), __builtin_isinf(b));
    b = 2048;
    b++;
    printf("step %04x %a\n", bits(&b), (double)(b * b));
    printf("variadic %a\n", sum(3, a, b, s.h));
    printf("pointers %a ", (double)load(&s.h));
    store(&table[0], 1.0 / 3);
    printf("%04x\n", bits(&table[0]));
    {
        static __fp16 folded = (__fp16)0.1 * 3;
        static float widened = (__fp16)0.1;
        printf("static %04x %a\n", bits(&folded), (double)widened);
    }
    return 0;
}
