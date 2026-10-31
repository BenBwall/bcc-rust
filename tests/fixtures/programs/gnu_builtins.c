/* GCC's and Clang's library aliases, carry, alignment, complex-part and
   sign builtins, and the constructors of `_Float16`, `__bf16` and
   `__float128` NaNs and infinities. The GCC identity's reference is the
   host GCC's build, the Clang identity's the bundled Clang's; `__clang__`
   selects the builtins only GCC has, and GCC before 14 has no
   `__builtin_addc`. */
#include <stdint.h>
#include <stdio.h>

#if defined __clang__ || __GNUC__ >= 14
#define CARRY 1
#endif

static void dump(const void *p, int n) {
    const unsigned char *bytes = p;
    for (int i = n - 1; i >= 0; i--)
        printf("%02x", bytes[i]);
    printf("\n");
}

static void library(void) {
    char d[64] = "abc";
    const char *s = "Hello";
    printf("%s\n", __builtin_strcat(d, s));
    printf("%s\n", __builtin_strncat(d, "xyz", 2));
    printf("%s\n", __builtin_strstr(d, "ll"));
    printf("%zu %zu\n", __builtin_strspn(d, "abc"), __builtin_strcspn(d, "H"));
    printf("%s\n", __builtin_strpbrk(d, "lo"));
    char *p = __builtin_strdup(s);
    printf("%s\n", p);
    char *q = __builtin_strndup(s, 3);
    printf("%s\n", q);
    char *r = __builtin_realloc(q, 32);
    __builtin_strcpy(r, "realloc");
    printf("%s\n", r);
    char e[16];
    char *end = __builtin_stpcpy(e, "stp");
    printf("%s %d\n", e, (int)(end - e));
    char f[8];
    end = __builtin_stpncpy(f, "ab", 5);
    printf("%d %d\n", (int)(end - f), f[4]);
    char g[8] = "0000000";
    end = __builtin_mempcpy(g, "xyz", 3);
    printf("%s %d\n", g, (int)(end - g));
    __builtin_bcopy("QR", g + 1, 2);
    printf("%s\n", g);
    __builtin_bzero(g + 4, 2);
    printf("%s %d\n", g, g[5]);
    printf("%d %d\n", __builtin_bcmp("ab", "ab", 2) == 0, __builtin_bcmp("ab", "ac", 2) != 0);
    printf("%s %s\n", __builtin_index("banana", 'n'), __builtin_rindex("banana", 'n'));
    printf("%d %d\n", __builtin_strcasecmp("HeLLo", "hello"),
           __builtin_strncasecmp("abcX", "ABCy", 3));
    printf("%d %d\n", __builtin_strcasecmp("a", "B") < 0, __builtin_strncasecmp("b", "A", 1) > 0);
    printf("%ld %lld\n", __builtin_labs(-5L), __builtin_llabs(-7LL));
#ifndef __clang__
    printf("%zu\n", __builtin_strnlen("abcdef", 3));
    printf("%d %d %d %d\n", !!__builtin_isdigit('7'), !!__builtin_isalpha('7'),
           __builtin_toupper('q'), __builtin_tolower('Q'));
    printf("%d %d %d\n", !!__builtin_iscntrl(3), !!__builtin_isspace(' '),
           !!__builtin_isxdigit('f'));
    void *a = __builtin_aligned_alloc(64, 128);
    printf("%d\n", (int)((uintptr_t)a % 64));
    __builtin_free(a);
#endif
    __builtin_free(p);
    __builtin_free(r);
}

static void scalars(void) {
#ifndef __clang__
    typedef unsigned __int128 u128;
    volatile u128 x = ((u128)0x0102030405060708ULL << 64) | 0x090a0b0c0d0e0f10ULL;
    u128 y = __builtin_bswap128(x);
    printf("%016llx%016llx\n", (unsigned long long)(y >> 64), (unsigned long long)y);
    static const u128 c = __builtin_bswap128((u128)1);
    printf("%d\n", c == ((u128)1 << 120));
    volatile double b = 3.5;
    double s = __builtin_assoc_barrier(b) + 1;
    printf("%g %d\n", s, __builtin_assoc_barrier(7));
    typedef int v4 __attribute__((vector_size(16)));
    volatile v4 lanes = {1, 2, 3, 4};
    v4 doubled = lanes + __builtin_assoc_barrier(lanes);
    printf("%d %d\n", doubled[0], doubled[3]);
    void *m = __builtin_alloca_with_align_and_max(12, 256, 64);
    printf("%d\n", (int)((uintptr_t)m % 32));
#endif
    static const unsigned long swapped = __builtin_bswap64(1);
    printf("%lx\n", swapped);
#ifdef CARRY
    unsigned carry, r;
    unsigned long cl, rl;
    unsigned long long cll, rll;
    r = __builtin_addc(0xffffffffu, 1u, 0u, &carry);
    printf("%u %u\n", r, carry);
    r = __builtin_addc(1u, 42u, 1u, &carry);
    printf("%u %u\n", r, carry);
    r = __builtin_addc(0xfffffffeu, 1u, 1u, &carry);
    printf("%u %u\n", r, carry);
    r = __builtin_subc(0u, 1u, 0u, &carry);
    printf("%u %u\n", r, carry);
    r = __builtin_subc(5u, 3u, 1u, &carry);
    printf("%u %u\n", r, carry);
    r = __builtin_subc(3u, 3u, 1u, &carry);
    printf("%u %u\n", r, carry);
    rl = __builtin_addcl(~0ul, ~0ul, 1ul, &cl);
    printf("%lu %lu\n", rl, cl);
    rll = __builtin_subcll(0ull, ~0ull, 1ull, &cll);
    printf("%llu %llu\n", rll, cll);
#endif
    void *p = __builtin_alloca_with_align(10, 512);
    printf("%d\n", (int)((uintptr_t)p % 64));
    char buffer[4];
    __builtin___clear_cache(buffer, buffer + 4);
    volatile double _Complex vz = 3.0 + 4.0 * __builtin_complex(0.0, 1.0);
    printf("%g %g\n", __builtin_creal(vz), __builtin_cimag(vz));
    double _Complex w = __builtin_conj(vz);
    printf("%g %g\n", __real__ w, __imag__ w);
    float _Complex fz = __builtin_conjf(1.5f);
    printf("%g %g\n", (double)__real__ fz, (double)__imag__ fz);
    printf("%Lg %Lg\n", __builtin_creall(2.0L), __builtin_cimagl((long double _Complex)vz));
    printf("%g\n", (double)__builtin_crealf(vz));
}

static void floating(void) {
    static volatile _Float16 hs = __builtin_nansf16(""), hn = __builtin_nanf16(""),
                             hi = __builtin_inff16(), hh = __builtin_huge_valf16();
    static __float128 qa = __builtin_fabsf128((__float128)-2.5),
                      qc = __builtin_copysignf128((__float128)3, (__float128)-1);
    _Float16 h;
    __float128 q;
    h = hs;
    dump(&h, 2);
    h = hn;
    dump(&h, 2);
    h = hi;
    dump(&h, 2);
    h = hh;
    dump(&h, 2);
    dump(&qa, 16);
    dump(&qc, 16);
    /* The tag's payload sits in the 16-bit type's own fraction, and a
       signaling NaN stays one, folded or assigned at run time. */
    static volatile _Float16 hp = __builtin_nanf16("5"), hs3 = __builtin_nansf16("3");
    h = hp;
    dump(&h, 2);
    h = hs3;
    dump(&h, 2);
    volatile _Float16 run;
    run = __builtin_nansf16("3");
    dump((const void *)&run, 2);
    run = __builtin_nansf16("");
    dump((const void *)&run, 2);
    volatile _Float16 m = -3, n = 1;
    h = __builtin_fabsf16(m);
    dump(&h, 2);
    h = __builtin_copysignf16(n, m);
    dump(&h, 2);
    volatile __float128 mq = -5, nq = 7;
    q = __builtin_fabsf128(mq);
    dump(&q, 16);
    q = __builtin_copysignf128(nq, mq);
    dump(&q, 16);
#ifndef __clang__
    /* Clang folds neither in a static initializer. */
    static _Float16 ha = __builtin_fabsf16((_Float16)-1.5),
                    hc = __builtin_copysignf16((_Float16)2, (_Float16)-0.0);
    dump(&ha, 2);
    dump(&hc, 2);
    static volatile __bf16 bs = __builtin_nansf16b(""), bs3 = __builtin_nansf16b("3");
    __bf16 bf = bs;
    dump(&bf, 2);
    bf = bs3;
    dump(&bf, 2);
    volatile __bf16 brun = __builtin_nansf16b("3");
    dump((const void *)&brun, 2);
    volatile _Float16 signaling = __builtin_nansf16("");
    printf("%d\n", __builtin_issignaling(signaling));
    static volatile __float128 qn = __builtin_nanq(""), qs = __builtin_nansq(""),
                               qi = __builtin_infq(), qh = __builtin_huge_valq();
    q = qn;
    dump(&q, 16);
    q = qs;
    dump(&q, 16);
    q = qi;
    dump(&q, 16);
    q = qh;
    dump(&q, 16);
    q = __builtin_fabsq(mq);
    dump(&q, 16);
    q = __builtin_copysignq(nq, mq);
    dump(&q, 16);
#endif
}

int main(void) {
    library();
    scalars();
    floating();
    return 0;
}
