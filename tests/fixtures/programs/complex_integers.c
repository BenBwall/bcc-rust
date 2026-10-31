/* GNU complex integer types: sizes, the usual arithmetic conversions,
   + - * / (GCC and Clang divide differently, each in the parts' type),
   unary - and ~ (the conjugate), equality, truth, conversions to and from
   real and complex floating and integer types, __real__ and __imag__ as
   lvalues, compound assignment, static initializers, arrays, structures
   and calls. Each identity's reference is its compiler's build. */
#include <stdio.h>

#define SHOW(T, z) printf("%s %lld %lld\n", #T, (long long)__real__ (z), (long long)__imag__ (z))

typedef _Complex char cc;
typedef _Complex unsigned char cuc;
typedef _Complex short cs;
typedef _Complex unsigned short cus;
typedef _Complex int ci;
typedef _Complex unsigned cu;
typedef _Complex long cl;
typedef _Complex unsigned long long cull;

struct pair { char tag; ci z; cs w; };

static cus statics[3] = { 100 + 200i, 300 - 2i, 65535 + 65536i };
static struct pair initialized = { 'p', 7 - 8i, 3 + 300000i };
static const cc folded = (cc)(100 + 50i) * (cc)(3 + 1i);
/* A part of a complex integer constant is an integer constant. */
static int imaginary_part = __imag__ 3i + 1;
static int real_part = __real__ (2 + 3i) * 5;
static int bounded[__imag__ (2 + 3i)];
enum { IMAGINARY = __imag__ (4 - 2i), WRAPPED = __real__ (cuc)(300 + 1i) };

static ci add_many(ci a, ci b, cs c, cc d, cl e, ci f, ci g, cl h)
{
    return a + b + c + d + (ci)e + f + g + (ci)h;
}

static cl twice(cl z) { return z * 2; }

static struct pair swap(struct pair p)
{
    ci t = p.z;
    p.z = p.w;
    p.w = t;
    return p;
}

#define DIVIDE(T, ar, ai, br, bi)                                      \
    do {                                                               \
        volatile T a, b;                                               \
        __real__ a = ar;                                               \
        __imag__ a = ai;                                               \
        __real__ b = br;                                               \
        __imag__ b = bi;                                               \
        T q = a / b;                                                   \
        SHOW(T, q);                                                    \
    } while (0)

int main(void)
{
    printf("%zu %zu %zu %zu %zu %zu\n", sizeof(cc), sizeof(cs), sizeof(ci), sizeof(cl),
           _Alignof(ci), sizeof(struct pair));
    printf("%zu %zu %zu %zu\n", sizeof((cc)0 + (cc)0), sizeof((cc)0 + (char)0),
           sizeof((cs)0 * 1), sizeof((ci)0 + 1.0));

    volatile ci x = 3 + 4i, y = -2 + 5i;
    ci s = x + y, d = x - y, p = x * y;
    SHOW(ci, s);
    SHOW(ci, d);
    SHOW(ci, p);
    SHOW(ci, -x);
    SHOW(ci, ~x);
    SHOW(ci, +x);
    printf("%d %d %d %d\n", x == x, x != y, x == 3 + 4i, !x);
    if (x)
        puts("nonzero");
    volatile ci zero = 0;
    if (!zero && !(zero != 0))
        puts("zero");

    volatile cc c = 100 + 100i;
    cc c2 = c * c;
    SHOW(cc, c2);
    cuc u = (cuc)c + (cuc)200;
    SHOW(cuc, u);
    volatile cu big = 4000000000u + 3i;
    cu sum = big + big;
    SHOW(cu, sum);

    DIVIDE(ci, 7, 5, 2, 0);
    DIVIDE(ci, 100000, 3, 50000, 1);
    DIVIDE(ci, 1, 1, 0, 1);
    DIVIDE(cc, 3, 4, 10, 10);
    DIVIDE(cc, 100, 7, 3, 2);
    DIVIDE(cs, 3000, 4, 100, 100);
    DIVIDE(cu, 5, 9, 2, 3);
    DIVIDE(cl, 123456789, -987654321, 1000, 7);
    ci by_real = x / 2;
    SHOW(ci, by_real);
    /* GCC divides each part by a real divisor; Clang takes it as c + 0i,
       so c * c wraps. */
    volatile ci wide = 100000 + 7i;
    volatile int divisor = 70000;
    ci wrapped = wide / divisor;
    SHOW(ci, wrapped);
    static const cu unsigned_quotient = (5u + 4000000000ui) / 4294967293u;
    SHOW(cu, unsigned_quotient);

    volatile double real = 7.75;
    ci from_double = real;
    SHOW(ci, from_double);
    volatile double _Complex dz = 2.9 - 3.9i;
    ci from_complex = dz;
    SHOW(ci, from_complex);
    double _Complex back = x;
    printf("%g %g\n", __real__ back, __imag__ back);
    float _Complex fz = y + 0.5f;
    printf("%g %g\n", (double)__real__ fz, (double)__imag__ fz);
    long r = x;
    _Bool truth = (ci)(0 + 1i);
    double dr = y;
    printf("%ld %d %g\n", r, truth, dr);
    cs narrowed = (cs)(cl)(70000 + 70001i);
    SHOW(cs, narrowed);

    ci z = 1 + 2i;
    __real__ z = 10;
    __imag__ z += 5;
    z *= 3;
    z -= 1i;
    z /= (ci)(2 + 0i);
    SHOW(ci, z);
    cc ch = 5;
    ch += 120;
    SHOW(cc, ch);

    for (int i = 0; i < 3; i++)
        SHOW(cus, statics[i]);
    printf("%c\n", initialized.tag);
    SHOW(ci, initialized.z);
    SHOW(cs, initialized.w);
    SHOW(cc, folded);

    ci total = add_many(1 + 1i, 2 + 2i, 3 + 3i, 4 + 4i, 5 + 5i, 6 + 6i, 7 + 7i, 8 + 8i);
    SHOW(ci, total);
    SHOW(cl, twice(1234567890123LL - 5i));
    struct pair swapped = swap(initialized);
    SHOW(ci, swapped.z);
    SHOW(cs, swapped.w);
    printf("%d %d %zu %d %d\n", imaginary_part, real_part, sizeof bounded / sizeof bounded[0],
           IMAGINARY, WRAPPED);
    ci array[4] = { 1, 2i, 3 + 3i };
    ci *pointer = &array[1];
    pointer[2] = pointer[0] * pointer[1];
    SHOW(ci, array[3]);
    return 0;
}
