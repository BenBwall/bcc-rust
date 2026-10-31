/* GNU `__auto_type`: each object takes its initializer's type after
   lvalue conversion and decay. Lines starting `x86:` use types wasm32
   lacks. */
#include <stdio.h>
#include <stdlib.h>

#define max(a, b) ({ __auto_type _a = (a); __auto_type _b = (b); _a > _b ? _a : _b; })
#define swap(x, y) do { __auto_type _t = (x); (x) = (y); (y) = _t; } while (0)
#define KIND(e) _Generic((e), int: "int", unsigned: "unsigned", long: "long", \
    double: "double", float: "float", char *: "char *", const char *: "const char *", \
    int *: "int *", const int *: "const int *", _Atomic int: "_Atomic int", \
    default: "other")

struct point { int x, y; };
struct flags { int small : 3; unsigned wide : 20; };

static int twice(int x) { return 2 * x; }
static int calls;
static int next(void) { return ++calls; }
static struct point make(int v) { struct point p = {v, v + 1}; return p; }
static int cleaned;
static void clean(int *p) { cleaned += *p; }

__auto_type file_scale = 2.5;
static __auto_type file_count = 10u;
__auto_type file_pointer = &file_count;

int main(void) {
    /* Scalars, qualifiers and lvalue conversion. */
    __auto_type i = 3;
    __auto_type u = 3u + 1;
    __auto_type l = 3L * 2;
    __auto_type c = 'c';
    const int k = 7;
    __auto_type copy = k;
    copy++;
    const __auto_type fixed = k + 1;
    volatile int vol = 5;
    __auto_type plain = vol;
    plain *= 2;
    printf("scalars %s %s %s %s %s %d %d %d\n", KIND(i), KIND(u), KIND(l), KIND(c),
           KIND(copy), copy, fixed, plain);
    printf("sizes %d %d %d\n", (int)sizeof u, (int)sizeof l == (int)sizeof(long),
           (int)sizeof c);

    /* Arrays, strings and functions decay. */
    int numbers[4] = {1, 2, 3, 4};
    __auto_type first = numbers;
    first[1] = 20;
    const int constants[2] = {5, 6};
    __auto_type cfirst = constants;
    __auto_type text = "abc";
    __auto_type function = twice;
    printf("decay %s %d %s %s %d %d %d\n", KIND(first), numbers[1], KIND(cfirst),
           KIND(text), text[1], function(21), sizeof function == sizeof(int (*)(int)));

    /* Structures copy; atomic operands follow each compiler. */
    struct point origin = {1, 2};
    __auto_type moved = origin;
    moved.x = 9;
    _Atomic int counter = 4;
    __auto_type observed = counter;
    observed += 1;
    printf("objects %d %d %d %s %d\n", origin.x, moved.x, moved.y,
           _Generic(&observed, int *: "int", _Atomic int *: "_Atomic int", default: "other"),
           (int)observed);

    /* Bit-field operands promote before `__auto_type` sees them. */
    struct flags f = {-3, 1000000};
    __auto_type small = f.small + 0;
    __auto_type wide = f.wide + 0u;
    printf("bit-fields %s %d %s %u\n", KIND(small), small, KIND(wide), wide);

    /* Macros evaluate each argument once. */
    int a = 5, b = 8;
    int m = max(a++, b++);
    double dm = max(2.5, (double)a);
    swap(a, b);
    int nested = max(max(next(), next()), next());
    printf("macros %d %d %d %g %d %d\n", m, a, b, dm, nested, calls);

    /* Variably modified types keep their run-time bounds. */
    int n = 5;
    int vla[n];
    for (int j = 0; j < n; j++)
        vla[j] = j * j;
    __auto_type whole = &vla;
    __auto_type element = vla;
    int (*rows)[n] = malloc(3 * sizeof *rows);
    __auto_type row = rows;
    row[2][4] = 42;
    n = 100;
    printf("vla %d %d %d %d %d\n", (int)sizeof *whole, (*whole)[3], element[4],
           (int)sizeof *row, rows[2][4]);
    free(rows);

    /* A cast's variably modified type is evaluated where the cast is. */
    volatile int columns = 4;
    int *buffer = calloc(64, sizeof(int));
    __auto_type grid = (int (*)[columns])buffer;
    grid[1][2] = 7;
    columns = 9;
    printf("cast %d %d\n", (int)sizeof *grid, buffer[6]);
    free(buffer);

    /* Conditional, call and statement-expression operands. */
    __auto_type mixed = n ? 1 : 2.0;
    __auto_type made = make(4);
    __auto_type built = ({ struct point q = {8, 9}; q; });
    __auto_type maker = &make;
    printf("operands %s %d %d %d %d\n", KIND(mixed), made.y, built.x, built.y, maker(1).x);

    /* Attributes on the declaration, and jumps past one. */
    {
        __attribute__((cleanup(clean))) __auto_type tracked = 5;
        tracked += 1;
    }
    __attribute__((aligned(32))) __auto_type aligned = 1;
    goto over;
    __auto_type skipped = next();
    (void)skipped;
over:
    printf("attributes %d %d %d\n", cleaned, (int)((unsigned long)&aligned % 32), calls);

    /* Complex, vector and half-precision values. */
    __auto_type z = 1.0 + 2.0i;
    __auto_type zf = 1.0f + 2.0fi;
    typedef int v4 __attribute__((vector_size(16)));
    v4 lanes = {1, 2, 3, 4};
    __auto_type doubled = lanes + lanes;
    printf("types %g %g %d %d %d\n", __real__ z, __imag__ z, (int)sizeof zf, doubled[3],
           (int)sizeof doubled);
#ifdef __FLT16_MAX__
    /* The product's type is `_Float16`, whatever precision computes it. */
    _Float16 h = 1.5f16;
    __auto_type squared = h * h;
    printf("x86: float16 %d %g\n", (int)sizeof squared, (double)squared);
#endif
#ifdef __SIZEOF_FLOAT128__
    __float128 q = 1;
    __auto_type third = q / 3;
    printf("x86: float128 %d %.9f\n", (int)sizeof third, (double)third);
#endif

    /* Loops, file scope and static objects. */
    unsigned total = 0;
    for (__auto_type step = 0u; step < 4; step++)
        total += step;
    static __auto_type local_static = 12;
    local_static++;
    *file_pointer += 1;
    printf("scope %u %d %g %u %s\n", total, local_static, file_scale * 2, file_count,
           KIND(file_scale));

#ifndef __clang__
    /* GCC: the declared name is not yet in scope in its initializer. */
    int outer = 40;
    {
        __auto_type outer = outer + 2;
        printf("gcc %d\n", outer);
    }
#endif
    return 0;
}
