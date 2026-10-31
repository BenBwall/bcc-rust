/* C23 `auto`: the object's type is its initializer's after lvalue
   conversion and decay, qualified as the specifiers are. Lines starting
   `x86:` use types wasm32 lacks. */
#include <stdio.h>

#define KIND(e) _Generic((e), int: "int", unsigned: "unsigned", long: "long", \
    double: "double", float: "float", char *: "char *", int *: "int *", \
    double *: "double *", const int *: "const int *", default: "other")

struct pair { int a; double b; };

static int add(int x, int y) { return x + y; }

/* C23 §6.7.10p4's example. */
static auto a = 3.5;
auto p = &a;
auto total = 0L;
static const auto limit = 4u;

static int counter(void) {
    static auto count = 0;
    return ++count;
}

int main(void) {
    auto i = 1 + 2;
    const auto ci = i * 2;
    auto copy = ci;
    copy++;
    register auto r = 6;
    static auto s = 1.5f;
    auto words = "words";
    int values[3] = {4, 5, 6};
    auto element = values;
    auto function = add;
    printf("basic %s %s %d %s %d %s %g %s %d %s %d %d\n", KIND(i), KIND(ci), copy, KIND(r),
           r, KIND(s), s, KIND(words), element[2], KIND(element), function(2, 3),
           sizeof function == sizeof(int (*)(int, int)));
    printf("file %s %s %g %s %ld %u %d\n", KIND(a), KIND(p), *p, KIND(total), total, limit,
           _Generic(&limit, const unsigned *: 1, default: 0));
    counter();
    printf("static %d\n", counter());

    struct pair pr = {1, 2.5};
    auto pc = pr;
    pc.b *= 2;
    _Atomic long al = 8;
    auto av = al;
    av -= 3;
    printf("objects %d %g %g %s %ld\n", pc.a, pc.b, pr.b,
           _Generic(&av, long *: "long", _Atomic long *: "_Atomic long", default: "other"),
           (long)av);

    /* The name is not in scope in its own initializer, but a statement
       expression may declare and use its own. */
    auto shadowed = ({ int shadowed = 3; shadowed * 4; });
    for (auto step = 0; step < 3; step++)
        total += step;
    printf("scope %d %ld\n", shadowed, total);

    int n = 3;
    double grid[n][n + 1];
    grid[2][3] = 7.5;
    auto row = grid;
    auto cells = &grid;
    n = 50;
    printf("vla %d %d %g\n", (int)sizeof *row, (int)sizeof *cells, row[2][3]);

    _Complex double z = 3.0 - 1.0i;
    auto conjugate = __builtin_conj(z);
    typedef short v8 __attribute__((vector_size(16)));
    v8 lanes = {1, 2, 3, 4, 5, 6, 7, 8};
    auto mask = lanes > 4;
    printf("types %g %g %d %d\n", __real__ conjugate, __imag__ conjugate, (int)sizeof mask,
           -mask[6]);
#ifdef __FLT16_MAX__
    _Float16 h = 0.5f16;
    auto sum = h + h;
    printf("x86: float16 %d %g\n", (int)sizeof sum, (double)sum);
#endif
#ifdef __clang__
    /* Clang deduces through pointer and function declarators. */
    auto *pointer = &i;
    auto (*callback)(int, int) = add;
    auto first = 1, second = 2;
    printf("clang %d %d %d\n", *pointer, callback(4, 5), first + second);
#endif
    return 0;
}
