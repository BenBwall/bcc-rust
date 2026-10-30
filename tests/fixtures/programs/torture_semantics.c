/* Behaviours GCC's torture tests rely on and both compilers give.
 *
 * A signed left shift into or past the sign bit wraps: GCC documents that it
 * does not treat it as undefined, and Clang emits a plain shift, so an
 * optimizer must not assume it cannot overflow (gcc.c-torture/execute/
 * 20121108-1.c packs an IPv4 address this way).
 *
 * `__LINE__` written as a macro argument inside another macro's replacement
 * list expands to the line where the outer macro is invoked, not the line of
 * its definition (gcc.dg/pr69391-1.c). */
#include <stdio.h>

static int __attribute__((noinline)) pack(const unsigned char *bytes, int count) {
    int address = 0;
    for (int i = 0; i < count; i++) {
        address <<= 8;
        address |= bytes[i] & 0xff;
    }
    return address;
}

static long long __attribute__((noinline)) rotate_right(long long value, int count) {
    return (value >> count) | (value << (64 - count));
}

static int __attribute__((noinline)) doubled_is_negative(int value) {
    return (value << 1) < 0;
}

#define STR_(x) #x
#define STR(x) STR_(x)
#define HERE STR(__LINE__)
#define TWICE STR(__LINE__) STR(__LINE__)
#define NUMBER __LINE__
/* A name an argument supplies to a body that completes its call: GCC gives
   the outer invocation's line, Clang the name's own. */
#define LINE_OF() __LINE__
#define CALL(m) m()
#define PASS(x) CALL(x)

int main(void) {
    static const unsigned char ip[] = {192, 168, 190, 160};
    printf("%x %llx %d\n", (unsigned)pack(ip, 4),
           (unsigned long long)rotate_right(0x12345678abcdef0LL, 4),
           doubled_is_negative(0x40000000));
    const char *here = HERE;
    const char *twice = TWICE;
    int number = NUMBER;
    printf("%s %s %d %d\n", here, twice, number, __LINE__);
    int passed = PASS(
        LINE_OF);
    printf("%d\n", passed);
    return 0;
}
