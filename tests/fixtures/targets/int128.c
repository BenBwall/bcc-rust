/* GNU integer types have no literal suffix; build wide values by conversion. */
#define UMAX ((__uint128_t)-1)
#define SMAX ((__int128)(UMAX >> 1))
#define SMIN (-SMAX - 1)
#if __SIZEOF_INT128__ != 16
#error missing target size
#endif
#if defined(__int128_t) || defined(__uint128_t)
#error builtin typedef names must not be macros
#endif
#if (0xffffffffffffffffULL >> 63) != 1
#error preprocessing arithmetic must remain 64 bit
#endif
_Static_assert(sizeof(__int128) == 16, "size");
_Static_assert(_Alignof(unsigned __int128) == 16, "alignment");
_Static_assert(sizeof(__int128_t) == 16, "signed builtin typedef");
_Static_assert(sizeof(__uint128_t) == 16, "unsigned builtin typedef");
_Static_assert(UMAX > SMAX, "unsigned comparison");
_Static_assert(UMAX + 1 == 0, "unsigned addition wraps");
_Static_assert((__uint128_t)0 - 1 == UMAX, "unsigned subtraction wraps");
_Static_assert(UMAX * UMAX == 1, "unsigned multiplication wraps");
_Static_assert(UMAX / 3 * 3 == UMAX, "full width division");
_Static_assert(UMAX % 7 == 3, "full width remainder");
_Static_assert(UMAX >> 127 == 1, "logical right shift");
_Static_assert(((__uint128_t)1 << 100) >> 100 == 1, "wide shift");
_Static_assert(SMIN / 3 == -SMAX / 3, "signed division");
_Static_assert(SMIN % 3 == -2, "signed remainder");
_Static_assert(SMIN >> 127 == -1, "arithmetic right shift");
_Static_assert((unsigned long long)UMAX == 0xffffffffffffffffULL, "narrow cast");
_Static_assert((__int128)UMAX == -1, "signed reinterpretation");
_Static_assert((_Bool)UMAX == 1, "boolean conversion");
_Static_assert((__uint128_t)0x1p127 == ((__uint128_t)1 << 127), "float cast");
_Static_assert((__int128)-0x1p127 == SMIN, "signed float minimum");
_Static_assert((__uint128_t)0x1.8p127L == ((__uint128_t)3 << 126),
               "long double high half");
_Static_assert((1 ? UMAX : 1) == UMAX, "conditional model");
_Static_assert((0 ? 1 / 0 : UMAX) == UMAX, "unselected exceptional arm");
struct Int128Record { char a; __int128 b; char c; };
struct Int128Fields { unsigned __int128 a:7; unsigned __int128 b:100; char c; };
union Int128Union { char c; __uint128_t u; };
_Static_assert(sizeof(struct Int128Record) == 48, "record size");
_Static_assert(_Alignof(struct Int128Record) == 16, "record alignment");
_Static_assert(__builtin_offsetof(struct Int128Record, b) == 16, "member offset");
_Static_assert(__builtin_offsetof(struct Int128Record, c) == 32, "tail offset");
_Static_assert(sizeof(union Int128Union) == 16, "union size");
#ifdef _WIN32
_Static_assert(sizeof(struct Int128Fields) == 32, "Microsoft fields");
_Static_assert(__builtin_offsetof(struct Int128Fields, c) == 16, "Microsoft unit");
#else
_Static_assert(sizeof(struct Int128Fields) == 16, "SysV fields");
_Static_assert(__builtin_offsetof(struct Int128Fields, c) == 14, "SysV unit");
#endif
__int128 signed_first;
signed __int128 explicit_signed;
__int128 signed signed_last;
unsigned __int128 unsigned_first;
__int128 unsigned unsigned_last;
__int128_t signed_alias;
__uint128_t unsigned_alias;
void int128_conversions(__int128 s, __uint128_t u, unsigned long long l, void *p) {
    __int128 signed_common = s + l;
    __uint128_t unsigned_common = s + u;
    _Bool truth = u;
    s = (__int128)p;
    u = (__uint128_t)p;
    p = (void *)u;
    (void)signed_common; (void)unsigned_common; (void)truth; (void)p;
}
