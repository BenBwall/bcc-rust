/* Variadic arguments around unions that mix a __float128 with doubles,
   which bcc passes and reads as the psABI (and GCC) does, as two doubles in
   vector registers or wholly on the stack, so a variadic callee reads every
   later argument where its caller put it. take(id, ...) reads gp longs, fp
   doubles and n unions (id = gp * 1000 + fp * 10 + n), then a long, a
   double, a long double and structures of each register class. named()
   takes a union as a named parameter before the variadic ones, called
   directly and through a pointer. Define CALLEE_ONLY or CALLER_ONLY to
   build one half. */
#include <stdarg.h>
#include <stdio.h>

union U {
    __float128 q;
    double d[2];
};
struct P {
    double d;
    long i;
};
struct D {
    double d[2];
};
struct L {
    long double x;
};
struct Q {
    __float128 x;
};

int take(int id, ...);
int named(union U first, int count, ...);

#ifndef CALLER_ONLY
int take(int id, ...) {
    int gp = id / 1000, fp = (id / 10) % 100, n = id % 10;
    va_list list;
    va_start(list, id);
    for (int i = 0; i < gp; i++) {
        if (va_arg(list, long) != 100 + i) {
            return 1;
        }
    }
    for (int i = 0; i < fp; i++) {
        if (va_arg(list, double) != 200.25 + i) {
            return 2;
        }
    }
    for (int i = 0; i < n; i++) {
        union U u = va_arg(list, union U);
        if (u.q != 0x1.123456789abcdef0123456789abcp+0Q + i) {
            return 10 + i;
        }
    }
    if (va_arg(list, long) != 901) {
        return 20;
    }
    if (va_arg(list, double) != 902.25) {
        return 21;
    }
    if (va_arg(list, long double) != 903.5L) {
        return 22;
    }
    struct P p = va_arg(list, struct P);
    if (p.d != 904.25 || p.i != 905) {
        return 23;
    }
    struct D d = va_arg(list, struct D);
    if (d.d[0] != 906.25 || d.d[1] != 907.25) {
        return 24;
    }
    struct L l = va_arg(list, struct L);
    if (l.x != 908.5L) {
        return 25;
    }
    struct Q q = va_arg(list, struct Q);
    if (q.x != 0x1.987654321p+0Q) {
        return 26;
    }
    if (va_arg(list, long) != 909) {
        return 27;
    }
    va_end(list);
    return 0;
}

int named(union U first, int count, ...) {
    if (first.q != 1.5Q) {
        return 1;
    }
    va_list list;
    va_start(list, count);
    for (int i = 0; i < count; i++) {
        if (va_arg(list, double) != i + 1.0) {
            return 2;
        }
    }
    union U u = va_arg(list, union U);
    struct D d = va_arg(list, struct D);
    va_end(list);
    return u.q != 2.5Q || d.d[0] != 906.25 || d.d[1] != 907.25 ? 3 : 0;
}
#endif

#ifndef CALLEE_ONLY
int main(void) {
    union U u[3];
    for (int i = 0; i < 3; i++) {
        u[i].q = 0x1.123456789abcdef0123456789abcp+0Q + i;
    }
    struct P p = {904.25, 905};
    struct D d = {{906.25, 907.25}};
    struct L l = {908.5L};
    struct Q q = {0x1.987654321p+0Q};
    int failures = 0, result;
#define TAIL 901L, 902.25, 903.5L, p, d, l, q, 909L
#define CHECK(id, ...)                                         \
    result = take(id, __VA_ARGS__);                            \
    if (result != 0) {                                         \
        printf("take(%d) failed check %d\n", id, result);       \
        failures++;                                            \
    }
    CHECK(0, TAIL)
    CHECK(1, u[0], TAIL)
    CHECK(2, u[0], u[1], TAIL)
    CHECK(3, u[0], u[1], u[2], TAIL)
    CHECK(50, 200.25, 201.25, 202.25, 203.25, 204.25, TAIL)
    CHECK(51, 200.25, 201.25, 202.25, 203.25, 204.25, u[0], TAIL)
    CHECK(52, 200.25, 201.25, 202.25, 203.25, 204.25, u[0], u[1], TAIL)
    CHECK(53, 200.25, 201.25, 202.25, 203.25, 204.25, u[0], u[1], u[2], TAIL)
    CHECK(60, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, TAIL)
    CHECK(61, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, u[0], TAIL)
    CHECK(62, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, u[0], u[1], TAIL)
    CHECK(63, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, u[0], u[1], u[2], TAIL)
    CHECK(70, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, TAIL)
    CHECK(71, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, u[0], TAIL)
    CHECK(72, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, u[0], u[1], TAIL)
    CHECK(73, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, u[0], u[1], u[2], TAIL)
    CHECK(80, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, 207.25, TAIL)
    CHECK(81, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, 207.25, u[0], TAIL)
    CHECK(82, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, 207.25, u[0], u[1], TAIL)
    CHECK(83, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, 207.25, u[0], u[1], u[2], TAIL)
    CHECK(4000, 100L, 101L, 102L, 103L, TAIL)
    CHECK(4001, 100L, 101L, 102L, 103L, u[0], TAIL)
    CHECK(4002, 100L, 101L, 102L, 103L, u[0], u[1], TAIL)
    CHECK(4003, 100L, 101L, 102L, 103L, u[0], u[1], u[2], TAIL)
    CHECK(4050, 100L, 101L, 102L, 103L, 200.25, 201.25, 202.25, 203.25, 204.25, TAIL)
    CHECK(4051, 100L, 101L, 102L, 103L, 200.25, 201.25, 202.25, 203.25, 204.25, u[0], TAIL)
    CHECK(4052, 100L, 101L, 102L, 103L, 200.25, 201.25, 202.25, 203.25, 204.25, u[0], u[1], TAIL)
    CHECK(4053, 100L, 101L, 102L, 103L, 200.25, 201.25, 202.25, 203.25, 204.25, u[0], u[1], u[2], TAIL)
    CHECK(4060, 100L, 101L, 102L, 103L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, TAIL)
    CHECK(4061, 100L, 101L, 102L, 103L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, u[0], TAIL)
    CHECK(4062, 100L, 101L, 102L, 103L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, u[0], u[1], TAIL)
    CHECK(4063, 100L, 101L, 102L, 103L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, u[0], u[1], u[2], TAIL)
    CHECK(4070, 100L, 101L, 102L, 103L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, TAIL)
    CHECK(4071, 100L, 101L, 102L, 103L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, u[0], TAIL)
    CHECK(4072, 100L, 101L, 102L, 103L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, u[0], u[1], TAIL)
    CHECK(4073, 100L, 101L, 102L, 103L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, u[0], u[1], u[2], TAIL)
    CHECK(4080, 100L, 101L, 102L, 103L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, 207.25, TAIL)
    CHECK(4081, 100L, 101L, 102L, 103L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, 207.25, u[0], TAIL)
    CHECK(4082, 100L, 101L, 102L, 103L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, 207.25, u[0], u[1], TAIL)
    CHECK(4083, 100L, 101L, 102L, 103L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, 207.25, u[0], u[1], u[2], TAIL)
    CHECK(5000, 100L, 101L, 102L, 103L, 104L, TAIL)
    CHECK(5001, 100L, 101L, 102L, 103L, 104L, u[0], TAIL)
    CHECK(5002, 100L, 101L, 102L, 103L, 104L, u[0], u[1], TAIL)
    CHECK(5003, 100L, 101L, 102L, 103L, 104L, u[0], u[1], u[2], TAIL)
    CHECK(5050, 100L, 101L, 102L, 103L, 104L, 200.25, 201.25, 202.25, 203.25, 204.25, TAIL)
    CHECK(5051, 100L, 101L, 102L, 103L, 104L, 200.25, 201.25, 202.25, 203.25, 204.25, u[0], TAIL)
    CHECK(5052, 100L, 101L, 102L, 103L, 104L, 200.25, 201.25, 202.25, 203.25, 204.25, u[0], u[1], TAIL)
    CHECK(5053, 100L, 101L, 102L, 103L, 104L, 200.25, 201.25, 202.25, 203.25, 204.25, u[0], u[1], u[2], TAIL)
    CHECK(5060, 100L, 101L, 102L, 103L, 104L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, TAIL)
    CHECK(5061, 100L, 101L, 102L, 103L, 104L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, u[0], TAIL)
    CHECK(5062, 100L, 101L, 102L, 103L, 104L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, u[0], u[1], TAIL)
    CHECK(5063, 100L, 101L, 102L, 103L, 104L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, u[0], u[1], u[2], TAIL)
    CHECK(5070, 100L, 101L, 102L, 103L, 104L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, TAIL)
    CHECK(5071, 100L, 101L, 102L, 103L, 104L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, u[0], TAIL)
    CHECK(5072, 100L, 101L, 102L, 103L, 104L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, u[0], u[1], TAIL)
    CHECK(5073, 100L, 101L, 102L, 103L, 104L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, u[0], u[1], u[2], TAIL)
    CHECK(5080, 100L, 101L, 102L, 103L, 104L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, 207.25, TAIL)
    CHECK(5081, 100L, 101L, 102L, 103L, 104L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, 207.25, u[0], TAIL)
    CHECK(5082, 100L, 101L, 102L, 103L, 104L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, 207.25, u[0], u[1], TAIL)
    CHECK(5083, 100L, 101L, 102L, 103L, 104L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, 207.25, u[0], u[1], u[2], TAIL)
    CHECK(6000, 100L, 101L, 102L, 103L, 104L, 105L, TAIL)
    CHECK(6001, 100L, 101L, 102L, 103L, 104L, 105L, u[0], TAIL)
    CHECK(6002, 100L, 101L, 102L, 103L, 104L, 105L, u[0], u[1], TAIL)
    CHECK(6003, 100L, 101L, 102L, 103L, 104L, 105L, u[0], u[1], u[2], TAIL)
    CHECK(6050, 100L, 101L, 102L, 103L, 104L, 105L, 200.25, 201.25, 202.25, 203.25, 204.25, TAIL)
    CHECK(6051, 100L, 101L, 102L, 103L, 104L, 105L, 200.25, 201.25, 202.25, 203.25, 204.25, u[0], TAIL)
    CHECK(6052, 100L, 101L, 102L, 103L, 104L, 105L, 200.25, 201.25, 202.25, 203.25, 204.25, u[0], u[1], TAIL)
    CHECK(6053, 100L, 101L, 102L, 103L, 104L, 105L, 200.25, 201.25, 202.25, 203.25, 204.25, u[0], u[1], u[2], TAIL)
    CHECK(6060, 100L, 101L, 102L, 103L, 104L, 105L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, TAIL)
    CHECK(6061, 100L, 101L, 102L, 103L, 104L, 105L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, u[0], TAIL)
    CHECK(6062, 100L, 101L, 102L, 103L, 104L, 105L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, u[0], u[1], TAIL)
    CHECK(6063, 100L, 101L, 102L, 103L, 104L, 105L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, u[0], u[1], u[2], TAIL)
    CHECK(6070, 100L, 101L, 102L, 103L, 104L, 105L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, TAIL)
    CHECK(6071, 100L, 101L, 102L, 103L, 104L, 105L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, u[0], TAIL)
    CHECK(6072, 100L, 101L, 102L, 103L, 104L, 105L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, u[0], u[1], TAIL)
    CHECK(6073, 100L, 101L, 102L, 103L, 104L, 105L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, u[0], u[1], u[2], TAIL)
    CHECK(6080, 100L, 101L, 102L, 103L, 104L, 105L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, 207.25, TAIL)
    CHECK(6081, 100L, 101L, 102L, 103L, 104L, 105L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, 207.25, u[0], TAIL)
    CHECK(6082, 100L, 101L, 102L, 103L, 104L, 105L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, 207.25, u[0], u[1], TAIL)
    CHECK(6083, 100L, 101L, 102L, 103L, 104L, 105L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, 207.25, u[0], u[1], u[2], TAIL)
    CHECK(7000, 100L, 101L, 102L, 103L, 104L, 105L, 106L, TAIL)
    CHECK(7001, 100L, 101L, 102L, 103L, 104L, 105L, 106L, u[0], TAIL)
    CHECK(7002, 100L, 101L, 102L, 103L, 104L, 105L, 106L, u[0], u[1], TAIL)
    CHECK(7003, 100L, 101L, 102L, 103L, 104L, 105L, 106L, u[0], u[1], u[2], TAIL)
    CHECK(7050, 100L, 101L, 102L, 103L, 104L, 105L, 106L, 200.25, 201.25, 202.25, 203.25, 204.25, TAIL)
    CHECK(7051, 100L, 101L, 102L, 103L, 104L, 105L, 106L, 200.25, 201.25, 202.25, 203.25, 204.25, u[0], TAIL)
    CHECK(7052, 100L, 101L, 102L, 103L, 104L, 105L, 106L, 200.25, 201.25, 202.25, 203.25, 204.25, u[0], u[1], TAIL)
    CHECK(7053, 100L, 101L, 102L, 103L, 104L, 105L, 106L, 200.25, 201.25, 202.25, 203.25, 204.25, u[0], u[1], u[2], TAIL)
    CHECK(7060, 100L, 101L, 102L, 103L, 104L, 105L, 106L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, TAIL)
    CHECK(7061, 100L, 101L, 102L, 103L, 104L, 105L, 106L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, u[0], TAIL)
    CHECK(7062, 100L, 101L, 102L, 103L, 104L, 105L, 106L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, u[0], u[1], TAIL)
    CHECK(7063, 100L, 101L, 102L, 103L, 104L, 105L, 106L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, u[0], u[1], u[2], TAIL)
    CHECK(7070, 100L, 101L, 102L, 103L, 104L, 105L, 106L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, TAIL)
    CHECK(7071, 100L, 101L, 102L, 103L, 104L, 105L, 106L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, u[0], TAIL)
    CHECK(7072, 100L, 101L, 102L, 103L, 104L, 105L, 106L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, u[0], u[1], TAIL)
    CHECK(7073, 100L, 101L, 102L, 103L, 104L, 105L, 106L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, u[0], u[1], u[2], TAIL)
    CHECK(7080, 100L, 101L, 102L, 103L, 104L, 105L, 106L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, 207.25, TAIL)
    CHECK(7081, 100L, 101L, 102L, 103L, 104L, 105L, 106L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, 207.25, u[0], TAIL)
    CHECK(7082, 100L, 101L, 102L, 103L, 104L, 105L, 106L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, 207.25, u[0], u[1], TAIL)
    CHECK(7083, 100L, 101L, 102L, 103L, 104L, 105L, 106L, 200.25, 201.25, 202.25, 203.25, 204.25, 205.25, 206.25, 207.25, u[0], u[1], u[2], TAIL)

    /* The two minimal cases: a union after five doubles, then a structure
       of two doubles; and after four longs, then a mixed structure. */
    CHECK(51, 200.25, 201.25, 202.25, 203.25, 204.25, u[0], TAIL)
    CHECK(4001, 100L, 101L, 102L, 103L, u[0], TAIL)
    union U first = {1.5Q}, second = {2.5Q};
    int (*pointer)(union U, int, ...) = named;
    for (int count = 4; count <= 7; count++) {
        result = count == 4   ? named(first, count, 1.0, 2.0, 3.0, 4.0, second, d)
                 : count == 5 ? pointer(first, count, 1.0, 2.0, 3.0, 4.0, 5.0, second, d)
                 : count == 6 ? named(first, count, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, second, d)
                              : pointer(first, count, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, second, d);
        if (result != 0) {
            printf("named(%d) failed check %d\n", count, result);
            failures++;
        }
    }
    printf("%d failures\n", failures);
    return failures != 0;
}
#endif
