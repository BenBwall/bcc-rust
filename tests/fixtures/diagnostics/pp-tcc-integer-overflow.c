/* TCC 0.9.27 warns of a constant whose digits overflow 64 bits, and of a
   decimal one without `u` of 2^63 or more, and keeps the wrapped value. */
unsigned long long a = 9223372036854775808;
unsigned long long b = 18446744073709551615;
unsigned long long c = 18446744073709551616;
unsigned long long d = 0x10000000000000000;
unsigned long long e = 18446744073709551616u;
unsigned long long f = 18446744073709551615u;
unsigned long long g = 0xffffffffffffffff;
_Static_assert(_Generic(9223372036854775808, unsigned long: 1), "unsigned long");
_Static_assert(_Generic(9223372036854775808ll, unsigned long long: 1), "ll");
_Static_assert(18446744073709551617 == 1, "wrapped");
#if 18446744073709551616 == 0
int wrapped;
#endif
