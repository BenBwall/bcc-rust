#define MAX ((__int128)(((__uint128_t)-1) >> 1))
#define MIN (-MAX - 1)
_Static_assert(MAX + 1, "addition overflow");
_Static_assert(MIN - 1, "subtraction overflow");
_Static_assert(MAX * 2, "multiplication overflow");
_Static_assert(-MIN, "negation overflow");
_Static_assert(MIN / -1, "division overflow");
_Static_assert(MIN % -1, "remainder overflow");
_Static_assert(((__uint128_t)1 << 128), "excessive shift");
_Static_assert(((__uint128_t)1 << (__uint128_t)-1), "unsigned shift count");
_Static_assert(((__uint128_t)1 / 0), "zero divisor");
int following;
