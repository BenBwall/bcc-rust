/* cl reads a 64-bit unsigned constant of 2^63 or more as the signed one with
   its bits (~0ull is -1), and takes a negative constant to be as wide as its
   two's complement: C4244 and C4242 measure that, and a remainder by a
   negative constant as wide as its magnitude and a sign. */
#define ULLONG_MAX 0xffffffffffffffffULL
#define UINT64_MAX 0xffffffffffffffffULL
typedef unsigned long long uint64_t;
void take(int i);

unsigned long all_ones(int flag, short *s, unsigned char *c)
{
    unsigned long m = flag ? ~0ULL : 0;
    *s += flag ? ULLONG_MAX : 0;
    *c = flag ? 0xFFFFFFFFFFFFFF80ull : 0;
    take(flag + 0xFFFFFFFFFFFFFFFFull);
    return m;
}

unsigned int returned(int flag)
{
    return flag ? UINT64_MAX : 1u;
}

int arithmetic(int a)
{
    return (a - ~0ull) + (a ? (uint64_t)-1 : 0) + (~0ull >> a) + a % ~0ull;
}

int around_two_to_the_63(int a)
{
    int x = a ? 0x8000000000000000ull : 0;
    int y = a ? 0xFFFFFFFF80000000ull : 0;
    int z = a ? 0xFFFFFFFF7FFFFFFFull : 0;
    return x + y + z;
}

short negative(int a)
{
    short s = a ? -32768 : 0;
    signed char sc = a ? 0xFFFFFFFFFFFFFF7Full : 0;
    unsigned short us = a ? 0xFFFFFFFFFFFF8000ull : 0;
    s = a ? -32769 : 0;
    s += a ? 0xFFFFFFFFFFFF0001ull : 0;
    (void)sc;
    (void)us;
    return a ? 0xFFFFFFFFu : 0;
}

signed char remainders(int a)
{
    signed char r = a % -127;
    r = a % 0xFFFFFFFFFFFFFF80ull;
    r = a % -128;
    return a % 128;
}
