/* <stdlib.h> conversions and utilities, <time.h>, and a small scanf for
   wasm32-wasip1. C99: §7.20, pp. 306-324; §7.19.6.2, pp. 282-289. The
   floating conversions are in floatscan.c.

   C23 reads a 0b or 0B prefix in strtol's base 0 and 2 and in scanf's %i.
   As in glibc 2.38 and later, the C23 functions have their own names
   (__isoc23_strtol, __isoc23_sscanf, ...), to which the headers redirect
   under -std=c23 or gnu23 and with _GNU_SOURCE, so other programs keep
   the C17 behaviour. */
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

/* Parses an integer: the magnitude, its sign, and how far it got. With
   binary set, base 0 and 2 also read a 0b prefix (C23). */
static unsigned long long parse_unsigned(const char *s, char **end, int base, int binary, int *negative,
                                         int *overflow) {
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
    } else if (binary && (base == 0 || base == 2) && p[0] == '0' && (p[1] | 32) == 'b' &&
               (p[2] == '0' || p[2] == '1')) {
        p += 2;
        base = 2;
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

static long long to_signed(const char *s, char **end, int base, int binary) {
    int negative, overflow;
    unsigned long long v = parse_unsigned(s, end, base, binary, &negative, &overflow);
    if (overflow || v > (negative ? (unsigned long long)LLONG_MAX + 1 : (unsigned long long)LLONG_MAX)) {
        errno = ERANGE;
        return negative ? LLONG_MIN : LLONG_MAX;
    }
    return negative ? (long long)(0 - v) : (long long)v;
}

static unsigned long long to_unsigned(const char *s, char **end, int base, int binary) {
    int negative, overflow;
    unsigned long long v = parse_unsigned(s, end, base, binary, &negative, &overflow);
    if (overflow) {
        errno = ERANGE;
        return ULLONG_MAX;
    }
    return negative ? 0 - v : v;
}

static long to_long(const char *s, char **end, int base, int binary) {
    long long v = to_signed(s, end, base, binary);
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

static unsigned long to_unsigned_long(const char *s, char **end, int base, int binary) {
    int negative, overflow;
    unsigned long long v = parse_unsigned(s, end, base, binary, &negative, &overflow);
    if (overflow || v > ULONG_MAX) {
        errno = ERANGE;
        return ULONG_MAX;
    }
    return negative ? (unsigned long)(0 - v) : (unsigned long)v;
}

long long strtoll(const char *s, char **end, int base) { return to_signed(s, end, base, 0); }
unsigned long long strtoull(const char *s, char **end, int base) { return to_unsigned(s, end, base, 0); }
long strtol(const char *s, char **end, int base) { return to_long(s, end, base, 0); }
unsigned long strtoul(const char *s, char **end, int base) { return to_unsigned_long(s, end, base, 0); }
long long __isoc23_strtoll(const char *s, char **end, int base) { return to_signed(s, end, base, 1); }
unsigned long long __isoc23_strtoull(const char *s, char **end, int base) { return to_unsigned(s, end, base, 1); }
long __isoc23_strtol(const char *s, char **end, int base) { return to_long(s, end, base, 1); }
unsigned long __isoc23_strtoul(const char *s, char **end, int base) { return to_unsigned_long(s, end, base, 1); }

int atoi(const char *s) { return (int)strtol(s, 0, 10); }
long atol(const char *s) { return strtol(s, 0, 10); }
long long atoll(const char *s) { return strtoll(s, 0, 10); }

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

/* scanf: whitespace, literal characters, %%, and %d %i %u %o %x %b %p %c
   %s %[ %f %e %g %a %n with the hh, h, l, ll, j, z, t and L lengths and *.
   %b (C23, and in glibc's every mode since 2.38) reads binary digits after
   an optional 0b prefix. */
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

/* A numeric field's characters, in local until they outgrow it. */
struct field {
    char *text;
    int length;
    int capacity;
    char local[512];
};

static int append(struct field *field, int c);
static void release(struct field *field);
static int convert(const char *text, int n, int c, int length, int base, int binary, va_list *ap);

/* The scanf family; with c23 set, %i reads a 0b prefix. */
static int scan(struct source *in, const char *fmt, va_list ap, int c23) {
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
           (7.19.6.2p10), and the characters stay consumed. A field has no
           length limit, as in glibc: it moves to the heap when it outgrows
           the stack buffer. When memory runs out, scanf returns EOF with
           errno ENOMEM, as glibc's does, whatever it has assigned. */
        int floating = c == 'f' || c == 'e' || c == 'g' || c == 'a' || c == 'E' || c == 'G' || c == 'F' ||
                       c == 'A';
        struct field field;
        field.text = field.local;
        field.length = 0;
        field.capacity = sizeof field.local;
#define NEXT_IS(test) (field.length < width && peek(in) != EOF && (test))
#define TAKE()                                       \
    do {                                             \
        if (!append(&field, peek(in)))               \
            return EOF;                              \
        take(in);                                    \
    } while (0)
        if (NEXT_IS(peek(in) == '+' || peek(in) == '-'))
            TAKE();
        int base = c == 'd' || c == 'u' ? 10 : c == 'o' ? 8 : c == 'x' || c == 'X' ? 16 : c == 'b' ? 2 : 0;
        /* %b, and %i in C23, read a 0b prefix (C23 7.23.6.2p12). */
        int binary = c == 'b' || (c == 'i' && c23);
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
            int digits = 0, radix = base;
            if ((radix == 0 || radix == 16 || radix == 2) && NEXT_IS(peek(in) == '0')) {
                TAKE();
                /* The prefix's 0 is no digit: "0x" alone fails. */
                if (radix != 2 && NEXT_IS((peek(in) | 32) == 'x')) {
                    TAKE();
                    radix = 16;
                } else if (binary && NEXT_IS((peek(in) | 32) == 'b')) {
                    TAKE();
                    radix = 2;
                } else {
                    digits = 1;
                    if (radix == 0)
                        radix = 8;
                }
            }
            if (radix == 0)
                radix = 10;
            while (NEXT_IS(base_digit(peek(in), radix))) {
                TAKE();
                digits++;
            }
            if (!digits) {
                release(&field);
                break;
            }
        }
#undef TAKE
#undef NEXT_IS
        if (!append(&field, 0))
            return EOF;
        int converted = field.length > 1 && convert(field.text, field.length - 1, c, length, base, binary,
                                                    skip ? 0 : &ap);
        release(&field);
        if (!converted)
            break;
        assigned += !skip;
    }
    return assigned;
}

/* Converts a numeric field of n characters, which must all be part of the
   number, and stores the value through the next argument of *ap unless ap
   is null. Returns whether the field was a whole number. */
static int convert(const char *text, int n, int c, int length, int base, int binary, va_list *ap) {
    char *end;
    if (c == 'f' || c == 'e' || c == 'g' || c == 'a' || c == 'E' || c == 'G' || c == 'F' || c == 'A') {
        /* Each type has its own correctly rounded conversion, so a float
           is not rounded twice and a long double keeps binary128's
           precision; strtof also keeps a NaN's low payload bits. */
        if (length == 1) {
            double v = strtod(text, &end);
            if (end != text + n)
                return 0;
            if (ap)
                *va_arg(*ap, double *) = v;
        } else if (length == 3) {
            long double v = strtold(text, &end);
            if (end != text + n)
                return 0;
            if (ap)
                *va_arg(*ap, long double *) = v;
        } else {
            float v = strtof(text, &end);
            if (end != text + n)
                return 0;
            if (ap)
                *va_arg(*ap, float *) = v;
        }
        return 1;
    }
    unsigned long long v = c == 'd' || c == 'i' ? (unsigned long long)to_signed(text, &end, base, binary)
                                                 : to_unsigned(text, &end, base, binary);
    if (end != text + n)
        return 0;
    if (ap)
        STORE_INTEGER(*ap, length, v);
    return 1;
}

/* Appends c to a numeric field, moving it to the heap when it outgrows
   its buffer. Fails, with errno ENOMEM, only when memory runs out. */
static int append(struct field *field, int c) {
    if (field->length == field->capacity) {
        int grown = field->capacity * 2;
        char *text = field->text == field->local ? malloc(grown) : realloc(field->text, grown);
        if (!text) {
            release(field);
            errno = ENOMEM;
            return 0;
        }
        if (field->text == field->local)
            memcpy(text, field->local, field->length);
        field->text = text;
        field->capacity = grown;
    }
    field->text[field->length++] = (char)c;
    return 1;
}

static void release(struct field *field) {
    if (field->text != field->local)
        free(field->text);
}

static int scan_string(const char *s, const char *fmt, va_list ap, int c23) {
    struct source in = {s, 0, -2, 0};
    return scan(&in, fmt, ap, c23);
}

/* Scans a stream and pushes back the character that ended the scan. */
static int scan_stream(FILE *f, const char *fmt, va_list ap, int c23) {
    struct source in = {0, f, -2, 0};
    int n = scan(&in, fmt, ap, c23);
    if (in.peeked >= 0)
        ungetc(in.peeked, f);
    return n;
}

int vsscanf(const char *s, const char *fmt, va_list ap) { return scan_string(s, fmt, ap, 0); }
int __isoc23_vsscanf(const char *s, const char *fmt, va_list ap) { return scan_string(s, fmt, ap, 1); }

int sscanf(const char *s, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int n = scan_string(s, fmt, ap, 0);
    va_end(ap);
    return n;
}

int __isoc23_sscanf(const char *s, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int n = scan_string(s, fmt, ap, 1);
    va_end(ap);
    return n;
}

int fscanf(FILE *f, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int n = scan_stream(f, fmt, ap, 0);
    va_end(ap);
    return n;
}

int __isoc23_fscanf(FILE *f, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int n = scan_stream(f, fmt, ap, 1);
    va_end(ap);
    return n;
}

int scanf(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int n = scan_stream(stdin, fmt, ap, 0);
    va_end(ap);
    return n;
}

int __isoc23_scanf(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int n = scan_stream(stdin, fmt, ap, 1);
    va_end(ap);
    return n;
}

intmax_t imaxabs(intmax_t j) { return j < 0 ? -j : j; }
intmax_t strtoimax(const char *s, char **end, int base) { return strtoll(s, end, base); }
uintmax_t strtoumax(const char *s, char **end, int base) { return strtoull(s, end, base); }
intmax_t __isoc23_strtoimax(const char *s, char **end, int base) { return to_signed(s, end, base, 1); }
uintmax_t __isoc23_strtoumax(const char *s, char **end, int base) { return to_unsigned(s, end, base, 1); }
