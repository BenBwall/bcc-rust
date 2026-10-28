/* <stdlib.h> conversions and utilities, <time.h>, and a small scanf for
   wasm32-wasip1. C99: §7.20, pp. 306-324; §7.19.6.2, pp. 282-289.

   strtod is exact for up to 19 significant digits with a decimal exponent
   whose power of ten is exact in binary64 (|e| <= 22); otherwise it
   scales by powers of ten and may differ from the correctly rounded value
   in the last place. */
#include <ctype.h>
#include <errno.h>
#include <limits.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <time.h>
#include "wasi.h"

int abs(int j) { return j < 0 ? -j : j; }
long labs(long j) { return j < 0 ? -j : j; }
long long llabs(long long j) { return j < 0 ? -j : j; }
div_t div(int n, int d) { div_t r = {n / d, n % d}; return r; }
ldiv_t ldiv(long n, long d) { ldiv_t r = {n / d, n % d}; return r; }
lldiv_t lldiv(long long n, long long d) { lldiv_t r = {n / d, n % d}; return r; }

/* Parses an integer: the magnitude, its sign, and how far it got. */
static unsigned long long parse_unsigned(const char *s, char **end, int base, int *negative, int *overflow) {
    const char *p = s;
    while (isspace((unsigned char)*p))
        p++;
    *negative = 0;
    *overflow = 0;
    if (*p == '+' || *p == '-')
        *negative = *p++ == '-';
    if ((base == 0 || base == 16) && p[0] == '0' && (p[1] | 32) == 'x' && isxdigit((unsigned char)p[2])) {
        p += 2;
        base = 16;
    } else if (base == 0) {
        base = *p == '0' ? 8 : 10;
    }
    unsigned long long value = 0;
    const char *digits = p;
    for (;; p++) {
        int c = (unsigned char)*p, digit;
        if (isdigit(c))
            digit = c - '0';
        else if (isalpha(c))
            digit = (c | 32) - 'a' + 10;
        else
            break;
        if (digit >= base)
            break;
        if (value > (ULLONG_MAX - (unsigned)digit) / (unsigned)base)
            *overflow = 1;
        value = value * (unsigned)base + (unsigned)digit;
    }
    if (end)
        *end = (char *)(p == digits ? s : p);
    return value;
}

long long strtoll(const char *s, char **end, int base) {
    int negative, overflow;
    unsigned long long v = parse_unsigned(s, end, base, &negative, &overflow);
    if (overflow || v > (negative ? (unsigned long long)LLONG_MAX + 1 : (unsigned long long)LLONG_MAX)) {
        errno = ERANGE;
        return negative ? LLONG_MIN : LLONG_MAX;
    }
    return negative ? (long long)(0 - v) : (long long)v;
}

unsigned long long strtoull(const char *s, char **end, int base) {
    int negative, overflow;
    unsigned long long v = parse_unsigned(s, end, base, &negative, &overflow);
    if (overflow) {
        errno = ERANGE;
        return ULLONG_MAX;
    }
    return negative ? 0 - v : v;
}

long strtol(const char *s, char **end, int base) {
    long long v = strtoll(s, end, base);
    if (v > LONG_MAX) {
        errno = ERANGE;
        return LONG_MAX;
    }
    if (v < LONG_MIN) {
        errno = ERANGE;
        return LONG_MIN;
    }
    return (long)v;
}

unsigned long strtoul(const char *s, char **end, int base) {
    int negative, overflow;
    unsigned long long v = parse_unsigned(s, end, base, &negative, &overflow);
    if (overflow || v > ULONG_MAX) {
        errno = ERANGE;
        return ULONG_MAX;
    }
    return negative ? (unsigned long)(0 - v) : (unsigned long)v;
}

int atoi(const char *s) { return (int)strtol(s, 0, 10); }
long atol(const char *s) { return strtol(s, 0, 10); }
long long atoll(const char *s) { return strtoll(s, 0, 10); }

static const double exact_powers[] = {1e0,  1e1,  1e2,  1e3,  1e4,  1e5,  1e6,  1e7,
                                      1e8,  1e9,  1e10, 1e11, 1e12, 1e13, 1e14, 1e15,
                                      1e16, 1e17, 1e18, 1e19, 1e20, 1e21, 1e22};

double strtod(const char *s, char **end) {
    const char *p = s;
    while (isspace((unsigned char)*p))
        p++;
    int negative = 0;
    if (*p == '+' || *p == '-')
        negative = *p++ == '-';
    double sign = negative ? -1.0 : 1.0;
    if (!strncasecmp(p, "inf", 3)) {
        p += 3;
        if (!strncasecmp(p, "inity", 5))
            p += 5;
        if (end)
            *end = (char *)p;
        return sign * __builtin_inf();
    }
    if (!strncasecmp(p, "nan", 3)) {
        p += 3;
        unsigned long long payload = 0;
        if (*p == '(') {
            const char *q = p + 1;
            while (isalnum((unsigned char)*q) || *q == '_')
                q++;
            if (*q == ')') {
                /* The payload is the sequence read as an integer in base
                   0 when all of it is one, as in glibc. */
                char *digits;
                unsigned long long v = strtoull(p + 1, &digits, 0);
                if (digits == q)
                    payload = v;
                p = q + 1;
            }
        }
        if (end)
            *end = (char *)p;
        union {
            unsigned long long u;
            double d;
        } bits = {(negative ? 1ull << 63 : 0) | 0x7ff8000000000000ull | (payload & 0x7ffffffffffffull)};
        return bits.d;
    }
    if (p[0] == '0' && (p[1] | 32) == 'x') {
        /* Hexadecimal: exact up to 16 significant digits. */
        const char *q = p + 2;
        unsigned long long m = 0;
        int exponent = 0, digits = 0, any = 0;
        for (;; q++) {
            int c = (unsigned char)*q;
            if (!isxdigit(c))
                break;
            any = 1;
            int d = isdigit(c) ? c - '0' : (c | 32) - 'a' + 10;
            if (digits < 16) {
                m = m * 16 + (unsigned)d;
                if (m)
                    digits++;
            } else {
                exponent += 4;
            }
        }
        if (*q == '.') {
            q++;
            for (;; q++) {
                int c = (unsigned char)*q;
                if (!isxdigit(c))
                    break;
                any = 1;
                int d = isdigit(c) ? c - '0' : (c | 32) - 'a' + 10;
                if (digits < 16) {
                    m = m * 16 + (unsigned)d;
                    exponent -= 4;
                    if (m)
                        digits++;
                }
            }
        }
        if (!any) {
            if (end)
                *end = (char *)p + 1;
            return sign * 0.0;
        }
        if ((*q | 32) == 'p') {
            char *after;
            long e = strtol(q + 1, &after, 10);
            if (after != q + 1) {
                exponent += (int)e;
                q = after;
            }
        }
        if (end)
            *end = (char *)q;
        double v = (double)m;
        while (exponent > 0) {
            v *= 2.0;
            exponent--;
        }
        while (exponent < 0) {
            v *= 0.5;
            exponent++;
        }
        return sign * v;
    }
    unsigned long long m = 0;
    int digits = 0, exponent = 0, any = 0;
    for (; isdigit((unsigned char)*p); p++) {
        any = 1;
        if (digits < 19) {
            m = m * 10 + (unsigned)(*p - '0');
            if (m)
                digits++;
        } else {
            exponent++;
        }
    }
    if (*p == '.') {
        p++;
        for (; isdigit((unsigned char)*p); p++) {
            any = 1;
            if (digits < 19) {
                m = m * 10 + (unsigned)(*p - '0');
                exponent--;
                if (m)
                    digits++;
            }
        }
    }
    if (!any) {
        if (end)
            *end = (char *)s;
        return 0.0;
    }
    if ((*p | 32) == 'e') {
        char *after;
        long e = strtol(p + 1, &after, 10);
        if (after != p + 1 && isdigit((unsigned char)after[-1])) {
            exponent += e > 100000 ? 100000 : e < -100000 ? -100000 : (int)e;
            p = after;
        }
    }
    if (end)
        *end = (char *)p;
    double v = (double)m;
    if (m == 0)
        return sign * 0.0;
    if (exponent >= 0 && exponent <= 22 && m < (1ull << 53)) {
        v *= exact_powers[exponent];
    } else if (exponent < 0 && exponent >= -22 && m < (1ull << 53)) {
        v /= exact_powers[-exponent];
    } else {
        while (exponent > 22) {
            v *= 1e22;
            exponent -= 22;
        }
        while (exponent < -22) {
            v /= 1e22;
            exponent += 22;
        }
        if (exponent >= 0)
            v *= exact_powers[exponent];
        else
            v /= exact_powers[-exponent];
    }
    if (v == __builtin_inf() || v == 0.0)
        errno = ERANGE;
    return sign * v;
}

float strtof(const char *s, char **end) {
    double v = strtod(s, end);
    if (v != v) {
        /* A NaN keeps its payload's low bits, as in glibc; converting the
           double would keep its high ones. */
        union {
            double d;
            unsigned long long u;
        } wide = {v};
        union {
            unsigned u;
            float f;
        } narrow = {(unsigned)(wide.u >> 32 & 0x80000000u) | 0x7fc00000u | (unsigned)(wide.u & 0x3fffffu)};
        return narrow.f;
    }
    return (float)v;
}

/* nan("n-char-sequence") is strtod("NAN(n-char-sequence)"), C99
   7.12.11.2; text receives that string. */
static void nan_text(char *text, size_t size, const char *tag) {
    size_t length = strlen(tag);
    if (length > size - 6)
        length = 0;
    memcpy(text, "nan(", 4);
    memcpy(text + 4, tag, length);
    text[4 + length] = ')';
    text[5 + length] = 0;
}

double nan(const char *tag) {
    char text[80];
    nan_text(text, sizeof text, tag);
    return strtod(text, 0);
}

float nanf(const char *tag) {
    char text[80];
    nan_text(text, sizeof text, tag);
    return strtof(text, 0);
}
long double strtold(const char *s, char **end) { return strtod(s, end); }
double atof(const char *s) { return strtod(s, 0); }

static unsigned long long random_state = 1;

int rand(void) {
    random_state = random_state * 6364136223846793005ull + 1442695040888963407ull;
    return (int)(random_state >> 33);
}

void srand(unsigned int seed) { random_state = seed; }

static void swap(unsigned char *a, unsigned char *b, size_t size) {
    while (size--) {
        unsigned char t = *a;
        *a++ = *b;
        *b++ = t;
    }
}

/* Insertion sort for short runs, else median-of-three quicksort on the
   smaller side recursively and the larger side iteratively. */
void qsort(void *base, size_t count, size_t size, int (*compare)(const void *, const void *)) {
    unsigned char *a = base;
    while (count > 8) {
        unsigned char *mid = a + (count / 2) * size, *last = a + (count - 1) * size;
        if (compare(mid, a) < 0)
            swap(mid, a, size);
        if (compare(last, mid) < 0) {
            swap(last, mid, size);
            if (compare(mid, a) < 0)
                swap(mid, a, size);
        }
        swap(mid, last, size);
        unsigned char *pivot = last;
        size_t store = 0;
        for (size_t i = 0; i + 1 < count; i++) {
            if (compare(a + i * size, pivot) < 0) {
                swap(a + i * size, a + store * size, size);
                store++;
            }
        }
        swap(a + store * size, pivot, size);
        size_t left = store, right = count - store - 1;
        if (left < right) {
            qsort(a, left, size, compare);
            a += (store + 1) * size;
            count = right;
        } else {
            qsort(a + (store + 1) * size, right, size, compare);
            count = left;
        }
    }
    for (size_t i = 1; i < count; i++)
        for (size_t j = i; j > 0 && compare(a + (j - 1) * size, a + j * size) > 0; j--)
            swap(a + (j - 1) * size, a + j * size, size);
}

void *bsearch(const void *key, const void *base, size_t count, size_t size,
              int (*compare)(const void *, const void *)) {
    const unsigned char *a = base;
    while (count > 0) {
        size_t half = count / 2;
        const unsigned char *middle = a + half * size;
        int c = compare(key, middle);
        if (c == 0)
            return (void *)middle;
        if (c > 0) {
            a = middle + size;
            count -= half + 1;
        } else {
            count = half;
        }
    }
    return 0;
}

clock_t clock(void) {
    unsigned long long now = 0;
    if (__bcc_wasi_clock_time_get(2, 1000, &now))
        return -1;
    return (clock_t)(now / 1000);
}

time_t time(time_t *timer) {
    unsigned long long now = 0;
    time_t seconds = __bcc_wasi_clock_time_get(0, 1000000000, &now) ? -1 : (time_t)(now / 1000000000u);
    if (timer)
        *timer = seconds;
    return seconds;
}

double difftime(time_t a, time_t b) { return (double)(a - b); }

/* scanf: whitespace, literal characters, %%, and %d %i %u %o %x %p %c %s
   %[ %f %e %g %a %n with the hh, h, l, ll, j, z, t and L lengths and *. */
struct source {
    const char *string;
    FILE *file;
    int peeked;
    int count;
};

static int peek(struct source *in) {
    if (in->peeked == -2)
        in->peeked = in->string ? (*in->string ? (unsigned char)*in->string++ : EOF) : fgetc(in->file);
    return in->peeked;
}

static void take(struct source *in) {
    in->peeked = -2;
    in->count++;
}

/* Stores an integer conversion's value, or %n's count, in the object its
   length modifier names; L (3) means long long here, as in glibc. */
#define STORE_INTEGER(ap, length, value)                         \
    do {                                                         \
        if ((length) == 2 || (length) == 3)                      \
            *va_arg(ap, long long *) = (long long)(value);       \
        else if ((length) == 1)                                  \
            *va_arg(ap, long *) = (long)(value);                 \
        else if ((length) == -1)                                 \
            *va_arg(ap, short *) = (short)(value);               \
        else if ((length) == -2)                                 \
            *va_arg(ap, signed char *) = (signed char)(value);   \
        else                                                     \
            *va_arg(ap, int *) = (int)(value);                   \
    } while (0)

/* Whether c is a digit of base. */
static int base_digit(int c, int base) {
    if (c >= '0' && c <= '9')
        return c - '0' < base;
    c |= 32;
    return base == 16 && c >= 'a' && c <= 'f';
}

static int scan(struct source *in, const char *fmt, va_list ap) {
    int assigned = 0;
    for (; *fmt; fmt++) {
        if (isspace((unsigned char)*fmt)) {
            while (peek(in) != EOF && isspace(peek(in)))
                take(in);
            continue;
        }
        if (*fmt != '%' || fmt[1] == '%') {
            if (*fmt == '%') {
                /* %% skips white space first, as a conversion does. */
                fmt++;
                while (peek(in) != EOF && isspace(peek(in)))
                    take(in);
            }
            /* The end of input before a literal is an input failure
               (7.19.6.2p16), so scanf("(%d)") ends with EOF. */
            if (peek(in) == EOF)
                return assigned ? assigned : EOF;
            if (peek(in) != (unsigned char)*fmt)
                break;
            take(in);
            continue;
        }
        fmt++;
        int skip = 0, width = 0, length = 0;
        if (*fmt == '*') {
            skip = 1;
            fmt++;
        }
        while (isdigit((unsigned char)*fmt))
            width = width * 10 + (*fmt++ - '0');
        if (*fmt == 'h') {
            length = -1;
            if (*++fmt == 'h') {
                length = -2;
                fmt++;
            }
        } else if (*fmt == 'l') {
            length = 1;
            if (*++fmt == 'l') {
                length = 2;
                fmt++;
            }
        } else if (*fmt == 'j') {
            length = 2;
            fmt++;
        } else if (*fmt == 'z' || *fmt == 't') {
            length = 1;
            fmt++;
        } else if (*fmt == 'L') {
            length = 3;
            fmt++;
        }
        char c = *fmt;
        int pointer = c == 'p';
        if (pointer) {
            /* A pointer reads as %lx: void * and long are both 32 bits. */
            c = 'x';
            length = 1;
        }
        if (c == 'n') {
            if (!skip)
                STORE_INTEGER(ap, length, in->count);
            continue;
        }
        if (c != 'c' && c != '[') {
            while (peek(in) != EOF && isspace(peek(in)))
                take(in);
        }
        if (peek(in) == EOF)
            return assigned ? assigned : EOF;
        if (width == 0)
            width = c == 'c' ? 1 : INT_MAX;
        char buffer[512];
        int n = 0;
        if (pointer && peek(in) == '(') {
            /* "(nil)", which printf writes for a null pointer, as glibc. */
            const char *nil = "(nil)";
            while (*nil && n < width && peek(in) == (unsigned char)*nil) {
                take(in);
                nil++;
                n++;
            }
            if (*nil)
                break;
            if (!skip) {
                *va_arg(ap, void **) = 0;
                assigned++;
            }
            continue;
        }
        if (c == 'c') {
            char *out = skip ? 0 : va_arg(ap, char *);
            for (; n < width && peek(in) != EOF; n++) {
                if (out)
                    out[n] = (char)peek(in);
                take(in);
            }
            if (n == 0)
                break;
            assigned += !skip;
            continue;
        }
        if (c == 's' || c == '[') {
            int set[256];
            int negate = 0;
            if (c == '[') {
                fmt++;
                if (*fmt == '^') {
                    negate = 1;
                    fmt++;
                }
                memset(set, 0, sizeof set);
                if (*fmt == ']')
                    set[(unsigned char)*fmt++] = 1;
                for (; *fmt && *fmt != ']'; fmt++) {
                    if (fmt[1] == '-' && fmt[2] && fmt[2] != ']') {
                        for (int x = (unsigned char)fmt[0]; x <= (unsigned char)fmt[2]; x++)
                            set[x] = 1;
                        fmt += 2;
                    } else {
                        set[(unsigned char)*fmt] = 1;
                    }
                }
            }
            char *out = skip ? 0 : va_arg(ap, char *);
            for (; n < width && peek(in) != EOF; n++) {
                int ch = peek(in);
                int accept = c == 's' ? !isspace(ch) : set[ch] != negate;
                if (!accept)
                    break;
                if (out)
                    out[n] = (char)ch;
                take(in);
            }
            if (n == 0)
                break;
            if (out)
                out[n] = 0;
            assigned += !skip;
            continue;
        }
        /* Numbers: consume exactly the characters that can continue the
           number (C99 7.19.6.2p9: the longest prefix of a matching input
           sequence), leaving the next one unread. If that prefix is not a
           whole number ("0x", "1e+", "infin"), the directive fails
           (7.19.6.2p10), and the characters stay consumed. */
        int floating = c == 'f' || c == 'e' || c == 'g' || c == 'a' || c == 'E' || c == 'G' || c == 'F' ||
                       c == 'A';
        int limit = width < (int)sizeof buffer - 1 ? width : (int)sizeof buffer - 1;
#define NEXT_IS(test) (n < limit && peek(in) != EOF && (test))
#define TAKE()                          \
    do {                                \
        buffer[n++] = (char)peek(in);   \
        take(in);                       \
    } while (0)
        if (NEXT_IS(peek(in) == '+' || peek(in) == '-'))
            TAKE();
        if (floating) {
            int lead = peek(in) | 32;
            if (lead == 'i' || lead == 'n') {
                const char *word = lead == 'i' ? "infinity" : "nan";
                int k = 0;
                for (; word[k] && NEXT_IS((peek(in) | 32) == word[k]); k++)
                    TAKE();
                /* NAN(n-char-sequence), C99 7.20.1.3p3. */
                if (lead == 'n' && k == 3 && NEXT_IS(peek(in) == '(')) {
                    TAKE();
                    while (NEXT_IS(isalnum(peek(in)) || peek(in) == '_'))
                        TAKE();
                    if (NEXT_IS(peek(in) == ')'))
                        TAKE();
                }
            } else {
                int hex = 0, digits = 0;
                if (NEXT_IS(peek(in) == '0')) {
                    TAKE();
                    digits = 1;
                    if (NEXT_IS((peek(in) | 32) == 'x')) {
                        /* The prefix's 0 is no digit: "0xp1" stops at p. */
                        TAKE();
                        hex = 1;
                        digits = 0;
                    }
                }
                while (NEXT_IS(base_digit(peek(in), hex ? 16 : 10))) {
                    TAKE();
                    digits++;
                }
                if (NEXT_IS(peek(in) == '.')) {
                    TAKE();
                    while (NEXT_IS(base_digit(peek(in), hex ? 16 : 10))) {
                        TAKE();
                        digits++;
                    }
                }
                if (digits && NEXT_IS((peek(in) | 32) == (hex ? 'p' : 'e'))) {
                    TAKE();
                    if (NEXT_IS(peek(in) == '+' || peek(in) == '-'))
                        TAKE();
                    while (NEXT_IS(peek(in) >= '0' && peek(in) <= '9'))
                        TAKE();
                }
            }
        } else {
            int base = c == 'd' || c == 'u' ? 10 : c == 'o' ? 8 : c == 'x' || c == 'X' ? 16 : 0;
            int digits = 0;
            if ((base == 0 || base == 16) && NEXT_IS(peek(in) == '0')) {
                TAKE();
                if (NEXT_IS((peek(in) | 32) == 'x')) {
                    /* The prefix's 0 is no digit: "0x" alone fails. */
                    TAKE();
                    base = 16;
                } else {
                    digits = 1;
                    if (base == 0)
                        base = 8;
                }
            }
            if (base == 0)
                base = 10;
            while (NEXT_IS(base_digit(peek(in), base))) {
                TAKE();
                digits++;
            }
            if (!digits)
                break;
        }
#undef TAKE
#undef NEXT_IS
        buffer[n] = 0;
        char *end;
        if (floating) {
            /* strtof for a float keeps a NaN's payload. */
            double v = 0;
            float narrow = 0;
            if (length == 0)
                narrow = strtof(buffer, &end);
            else
                v = strtod(buffer, &end);
            if (n == 0 || end != buffer + n)
                break;
            if (!skip) {
                if (length == 1)
                    *va_arg(ap, double *) = v;
                else if (length == 3)
                    *va_arg(ap, long double *) = v;
                else
                    *va_arg(ap, float *) = narrow;
                assigned++;
            }
            continue;
        }
        int base = c == 'd' || c == 'u' ? 10 : c == 'o' ? 8 : c == 'x' || c == 'X' ? 16 : 0;
        unsigned long long v = strtoull(buffer, &end, base);
        if (c == 'd' || c == 'i')
            v = (unsigned long long)strtoll(buffer, &end, base);
        if (end != buffer + n)
            break;
        if (!skip) {
            STORE_INTEGER(ap, length, v);
            assigned++;
        }
    }
    return assigned;
}

int vsscanf(const char *s, const char *fmt, va_list ap) {
    struct source in = {s, 0, -2, 0};
    return scan(&in, fmt, ap);
}

int sscanf(const char *s, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int n = vsscanf(s, fmt, ap);
    va_end(ap);
    return n;
}

int fscanf(FILE *f, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    struct source in = {0, f, -2, 0};
    int n = scan(&in, fmt, ap);
    if (in.peeked >= 0)
        ungetc(in.peeked, f);
    va_end(ap);
    return n;
}

int scanf(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    struct source in = {0, stdin, -2, 0};
    int n = scan(&in, fmt, ap);
    if (in.peeked >= 0)
        ungetc(in.peeked, stdin);
    va_end(ap);
    return n;
}

intmax_t imaxabs(intmax_t j) { return j < 0 ? -j : j; }
intmax_t strtoimax(const char *s, char **end, int base) { return strtoll(s, end, base); }
uintmax_t strtoumax(const char *s, char **end, int base) { return strtoull(s, end, base); }
