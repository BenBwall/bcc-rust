/* GNU casts to a union type in static initializers: the whole object, an
   element of a braced list, a member of a structure, nested casts and
   qualified or typedef unions. Each cast stores its operand in the member
   whose type it has and zero in the rest of the union. Built by GCC and
   Clang, this prints what bcc must print under each identity. */
#include <stdio.h>
#include <string.h>

union U { int i; float f; };
typedef union U UT;
typedef const union U CUT;
struct S { int a; union U u; char c; };
struct T { struct S s[2]; UT x[2]; };

union U whole = (union U)1;
union U as_float = (union U)2.5f;
static union U internal = __extension__ (union U)3;
const union U constant = (const union U)4;
CUT through_typedef = (CUT)5.5f;
const volatile union U qualified = (const volatile union U)6;
union U elements[] = { (union U)7, [3] = (UT)8.25f, [1] = (union U)9 };
union U range[3] = { [0 ... 2] = (union U)10 };
struct S member = { 11, (union U)12.5f, 'm' };
struct S designated = { .u = (union U)13, .c = 'd' };
struct S replaced = { .u = (union U)14, .u.f = 15.5f };
struct S replacing = { .u.i = 16, .u = (union U)17.5f };
struct T nested = { { { 1, (union U)18 }, { .u = (CUT)19.5f } }, { (UT)20 } };
struct S *literal = &(struct S){ 21, (union U)22, 'l' };

/* The member a cast fills starts the union; the rest stays zero. */
union W { char c; long long l; double d; };
union W narrow = (union W)(char)'w';
union O { union U u; double d; char pad[16]; };
union O inner = (union O)(union U)0.25f;
union D { double d; int i; };
union D folded = (union D)(2.0 / 3);
union C { _Complex double z; int i; };
union C complex_member = (union C)((_Complex double)1.0 + 2.0i);
union L { long double ld; char c; };
union L extended = (union L)1.25L;

/* Addresses: a static object, an element and a function. */
static int target = 42;
int numbers[4] = { 1, 2, 3, 4 };
union P { int *p; long l; };
union P address = (union P)&target;
union P element = (union P)&numbers[2];
static int answer(void) { return 23; }
union F { int (*f)(void); int i; };
union F function = (union F)answer;

/* A bit-field member of the operand's declared type: Clang matches it and
   stores the operand whole; GCC matches only the plain member. */
union B { int b : 3; int i; };
union B bits = (union B)13;

#ifdef __clang__
/* Clang's evaluator also reads a compound literal, a comma and a floating
   operation that traps, where GCC refuses the initializer. */
struct Q { int a, b; };
union V { struct Q q; int i; long long pad; };
union V record_literal = (union V)(struct Q){ 24, 25 };
union U scalar_literal = (union U)(int){ 26 };
union U comma = (union U)(0, 27);
union D infinite = (union D)(1.0 / 0.0);
union C mixed = (union C)(3.0 + 4.0i);
#endif

/* Prints the bytes of an object, so zeroed padding shows. */
static void bytes(const char *name, const void *object, size_t size) {
    const unsigned char *p = object;
    printf("%s:", name);
    for (size_t i = 0; i < size; i++)
        printf(" %02x", p[i]);
    printf("\n");
}

int main(void) {
    printf("%d %g %d %d %g %d\n", whole.i, as_float.f, internal.i, constant.i,
           through_typedef.f, qualified.i);
    printf("%d %d %d %g %zu\n", elements[0].i, elements[1].i, elements[2].i,
           elements[3].f, sizeof elements / sizeof elements[0]);
    printf("%d %d %d\n", range[0].i, range[1].i, range[2].i);
    printf("%d %g %c | %d %c | %g | %g\n", member.a, member.u.f, member.c,
           designated.u.i, designated.c, replaced.u.f, replacing.u.f);
    printf("%d %d %d %g %d %d | %d %d %c\n", nested.s[0].a, nested.s[0].u.i,
           nested.s[1].a, nested.s[1].u.f, nested.x[0].i, nested.x[1].i,
           literal->a, literal->u.i, literal->c);
    bytes("narrow", &narrow, sizeof narrow);
    bytes("inner", &inner, sizeof inner);
    printf("%.17g %g %g %Lg\n", folded.d, __real__ complex_member.z,
           __imag__ complex_member.z, extended.ld);
    printf("%d %d %d %d\n", *address.p, address.p == &target, *element.p,
           function.f());
    printf("%d %d\n", bits.b, bits.i);
#ifdef __clang__
    printf("%d %d %lld %d %d %g %g %g\n", record_literal.q.a, record_literal.q.b,
           record_literal.pad >> 32, scalar_literal.i, comma.i, infinite.d,
           __real__ mixed.z, __imag__ mixed.z);
#endif
    return 0;
}
