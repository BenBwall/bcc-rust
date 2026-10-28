/* <stdio.h> for wasm32-wasip1: buffered streams over WASI file
   descriptors and the printf family.
   C99: §7.19, pp. 262-314.

   Floating conversions are exact: the value's binary significand and
   exponent are expanded to every decimal digit with big integers and then
   rounded to nearest, ties to even, as glibc prints them. stdout is line
   buffered and stderr unbuffered, as on a terminal. */
#include <errno.h>
#include <limits.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include "wasi.h"

struct __bcc_file {
    int fd;
    int mode;
    int error;
    int eof;
    int unget;
    unsigned char *buffer;
    size_t size;
    size_t length;
    size_t read_position;
    size_t read_length;
};

static unsigned char stdout_buffer[BUFSIZ];
static unsigned char stdin_buffer[BUFSIZ];
static struct __bcc_file files[3] = {
    {0, _IOFBF, 0, 0, EOF, stdin_buffer, BUFSIZ, 0, 0, 0},
    {1, _IOLBF, 0, 0, EOF, stdout_buffer, BUFSIZ, 0, 0, 0},
    {2, _IONBF, 0, 0, EOF, 0, 0, 0, 0, 0},
};

#undef stdin
#undef stdout
#undef stderr
FILE *const stdin = &files[0];
FILE *const stdout = &files[1];
FILE *const stderr = &files[2];

int __bcc_write_all(int fd, const void *buf, size_t len) {
    const unsigned char *p = buf;
    while (len > 0) {
        struct __bcc_ciovec iov = {p, len};
        size_t written = 0;
        int error = __bcc_wasi_fd_write(fd, &iov, 1, &written);
        if (error)
            return error;
        if (written == 0)
            return 29;
        p += written;
        len -= written;
    }
    return 0;
}

ssize_t write(int fd, const void *buf, size_t count) {
    struct __bcc_ciovec iov = {buf, count};
    size_t written = 0;
    int error = __bcc_wasi_fd_write(fd, &iov, 1, &written);
    if (error) {
        errno = error;
        return -1;
    }
    return (ssize_t)written;
}

ssize_t read(int fd, void *buf, size_t count) {
    struct __bcc_iovec iov = {buf, count};
    size_t got = 0;
    int error = __bcc_wasi_fd_read(fd, &iov, 1, &got);
    if (error) {
        errno = error;
        return -1;
    }
    return (ssize_t)got;
}

int fflush(FILE *f) {
    if (!f) {
        __bcc_flush_all();
        return 0;
    }
    if (f->length > 0 && f->fd != 0) {
        int error = __bcc_write_all(f->fd, f->buffer, f->length);
        f->length = 0;
        if (error) {
            f->error = 1;
            return EOF;
        }
    }
    return 0;
}

void __bcc_flush_all(void) {
    fflush(&files[1]);
    fflush(&files[2]);
}

static int put_bytes(FILE *f, const void *data, size_t n) {
    const unsigned char *p = data;
    if (f->fd == 0) {
        f->error = 1;
        return EOF;
    }
    if (f->mode == _IONBF || f->size == 0) {
        if (__bcc_write_all(f->fd, p, n)) {
            f->error = 1;
            return EOF;
        }
        return 0;
    }
    int newline = 0;
    for (size_t i = 0; i < n; i++) {
        if (f->length == f->size && fflush(f))
            return EOF;
        f->buffer[f->length++] = p[i];
        newline |= p[i] == '\n';
    }
    if (newline && f->mode == _IOLBF)
        return fflush(f);
    return 0;
}

int setvbuf(FILE *f, char *buf, int mode, size_t size) {
    fflush(f);
    f->mode = mode;
    if (buf && size > 0) {
        f->buffer = (unsigned char *)buf;
        f->size = size;
    } else if (mode != _IONBF && !f->buffer) {
        f->buffer = malloc(BUFSIZ);
        f->size = f->buffer ? BUFSIZ : 0;
    }
    return 0;
}

void setbuf(FILE *f, char *buf) {
    setvbuf(f, buf, buf ? _IOFBF : _IONBF, BUFSIZ);
}

int fputc(int c, FILE *f) {
    unsigned char byte = (unsigned char)c;
    return put_bytes(f, &byte, 1) ? EOF : byte;
}

int putc(int c, FILE *f) { return fputc(c, f); }
int putchar(int c) { return fputc(c, stdout); }

int fputs(const char *s, FILE *f) {
    return put_bytes(f, s, strlen(s)) ? EOF : 1;
}

int puts(const char *s) {
    if (put_bytes(stdout, s, strlen(s)) || put_bytes(stdout, "\n", 1))
        return EOF;
    return 1;
}

size_t fwrite(const void *p, size_t size, size_t count, FILE *f) {
    if (size == 0 || count == 0)
        return 0;
    return put_bytes(f, p, size * count) ? 0 : count;
}

int fgetc(FILE *f) {
    if (f->unget != EOF) {
        int c = f->unget;
        f->unget = EOF;
        return c;
    }
    if (f->fd != 0) {
        f->error = 1;
        return EOF;
    }
    if (f->read_position == f->read_length) {
        fflush(stdout);
        struct __bcc_iovec iov = {f->buffer, f->size};
        size_t got = 0;
        if (__bcc_wasi_fd_read(f->fd, &iov, 1, &got)) {
            f->error = 1;
            return EOF;
        }
        if (got == 0) {
            f->eof = 1;
            return EOF;
        }
        f->read_position = 0;
        f->read_length = got;
    }
    return f->buffer[f->read_position++];
}

int getc(FILE *f) { return fgetc(f); }
int getchar(void) { return fgetc(stdin); }

int ungetc(int c, FILE *f) {
    if (c == EOF)
        return EOF;
    f->unget = (unsigned char)c;
    f->eof = 0;
    return f->unget;
}

char *fgets(char *s, int n, FILE *f) {
    int i = 0;
    while (i + 1 < n) {
        int c = fgetc(f);
        if (c == EOF)
            break;
        s[i++] = (char)c;
        if (c == '\n')
            break;
    }
    if (i == 0 && n > 1)
        return 0;
    if (n > 0)
        s[i] = 0;
    return s;
}

size_t fread(void *p, size_t size, size_t count, FILE *f) {
    unsigned char *out = p;
    size_t total = size * count, i = 0;
    for (; i < total; i++) {
        int c = fgetc(f);
        if (c == EOF)
            break;
        out[i] = (unsigned char)c;
    }
    return size ? i / size : 0;
}

int feof(FILE *f) { return f->eof; }
int ferror(FILE *f) { return f->error; }
void clearerr(FILE *f) { f->eof = f->error = 0; }

int fclose(FILE *f) {
    fflush(f);
    return 0;
}

FILE *fopen(const char *name, const char *mode) {
    (void)name;
    (void)mode;
    errno = 52;
    return 0;
}

int remove(const char *name) {
    (void)name;
    errno = 52;
    return -1;
}

int rename(const char *old, const char *new) {
    (void)old;
    (void)new;
    errno = 52;
    return -1;
}

void perror(const char *s) {
    if (s && *s) {
        fputs(s, stderr);
        fputs(": ", stderr);
    }
    fputs(strerror(errno), stderr);
    fputs("\n", stderr);
}

/* Formatted output. */

struct sink {
    FILE *file;
    char *buffer;
    size_t capacity;
    size_t count;
};

static void emit(struct sink *sink, const char *p, size_t n) {
    if (sink->file) {
        put_bytes(sink->file, p, n);
    } else {
        for (size_t i = 0; i < n; i++) {
            if (sink->count + i + 1 < sink->capacity)
                sink->buffer[sink->count + i] = p[i];
        }
    }
    sink->count += n;
}

static void pad(struct sink *sink, char c, int n) {
    char chunk[32];
    memset(chunk, c, sizeof chunk);
    while (n > 0) {
        int step = n < 32 ? n : 32;
        emit(sink, chunk, (size_t)step);
        n -= step;
    }
}

enum { LEFT = 1, PLUS = 2, SPACE = 4, ALT = 8, ZERO = 16 };

struct spec {
    int flags;
    int width;
    int precision;
    char conversion;
};

/* Writes prefix, then body padded to the width with zeros after the prefix
   or spaces around everything. */
static void field(struct sink *sink, const struct spec *s, const char *prefix, size_t prefix_len,
                  int zeros, const char *body, size_t body_len, const char *tail, int tail_zeros) {
    size_t tail_len = tail ? strlen(tail) : 0;
    long total = (long)(prefix_len + (size_t)zeros + body_len + (size_t)tail_zeros + tail_len);
    long padding = s->width > total ? s->width - total : 0;
    if (!(s->flags & LEFT) && !(s->flags & ZERO))
        pad(sink, ' ', (int)padding);
    emit(sink, prefix, prefix_len);
    if (!(s->flags & LEFT) && (s->flags & ZERO))
        pad(sink, '0', (int)padding);
    pad(sink, '0', zeros);
    emit(sink, body, body_len);
    pad(sink, '0', tail_zeros);
    if (tail)
        emit(sink, tail, tail_len);
    if (s->flags & LEFT)
        pad(sink, ' ', (int)padding);
}

static void format_integer(struct sink *sink, struct spec *s, unsigned long long value, int negative) {
    char digits[32];
    int n = 0;
    unsigned base = 10;
    const char *alphabet = "0123456789abcdef";
    switch (s->conversion) {
    case 'o': base = 8; break;
    case 'x': base = 16; break;
    case 'X': base = 16; alphabet = "0123456789ABCDEF"; break;
    case 'p': base = 16; break;
    }
    unsigned long long v = value;
    while (v) {
        digits[31 - n++] = alphabet[v % base];
        v /= base;
    }
    int precision = s->precision < 0 ? 1 : s->precision;
    if (s->precision >= 0)
        s->flags &= ~ZERO;
    int zeros = precision > n ? precision - n : 0;
    char prefix[3];
    size_t prefix_len = 0;
    if (negative)
        prefix[prefix_len++] = '-';
    else if (s->flags & PLUS)
        prefix[prefix_len++] = '+';
    else if (s->flags & SPACE)
        prefix[prefix_len++] = ' ';
    if ((s->flags & ALT) && base == 8 && zeros == 0 && (value != 0 || precision == 0)) {
        if (n == 0 || digits[32 - n] != '0')
            zeros = 1;
    }
    if (((s->flags & ALT) && base == 16 && value != 0) || s->conversion == 'p') {
        prefix[prefix_len++] = '0';
        prefix[prefix_len++] = s->conversion == 'X' ? 'X' : 'x';
    }
    field(sink, s, prefix, prefix_len, zeros, digits + 32 - n, (size_t)n, 0, 0);
}

/* Big integers in 32-bit limbs, least significant first, for the exact
   decimal expansion of floating values up to binary128. */
#define LIMBS 520
static uint32_t big_integer[LIMBS];
static uint32_t big_fraction[LIMBS];

/* Decimal digits: the integer part's, then the fraction's. */
#define DIGITS 12000
static char digit_buffer[DIGITS];

struct decimal {
    int integer_digits;  /* digits before the point, at least one */
    int fraction_bits;   /* K: the fraction is big_fraction / 2^K */
    int fraction_limbs;
};

/* Sets big_integer to the integer part and big_fraction to the fraction
   numerator of m * 2^e, for m = (hi << 64) | lo, and writes the integer
   part's decimal digits to digit_buffer. */
static void expand(struct decimal *d, uint64_t hi, uint64_t lo, int e) {
    memset(big_integer, 0, sizeof big_integer);
    memset(big_fraction, 0, sizeof big_fraction);
    uint32_t m[4] = {(uint32_t)lo, (uint32_t)(lo >> 32), (uint32_t)hi, (uint32_t)(hi >> 32)};
    d->fraction_bits = 0;
    d->fraction_limbs = 0;
    if (e >= 0) {
        int limb = e / 32, bit = e % 32;
        for (int i = 0; i < 4; i++) {
            uint64_t v = (uint64_t)m[i] << bit;
            if (limb + i < LIMBS)
                big_integer[limb + i] |= (uint32_t)v;
            if (limb + i + 1 < LIMBS)
                big_integer[limb + i + 1] |= (uint32_t)(v >> 32);
        }
    } else {
        int k = -e;
        d->fraction_bits = k;
        d->fraction_limbs = k / 32 + 2;
        /* bits of m at positions >= k form the integer part */
        for (int bit = 0; bit < 128; bit++) {
            if (!((m[bit / 32] >> (bit % 32)) & 1))
                continue;
            if (bit >= k) {
                int at = bit - k;
                big_integer[at / 32] |= 1u << (at % 32);
            } else {
                big_fraction[bit / 32] |= 1u << (bit % 32);
            }
        }
    }
    /* Integer digits by repeated division by 10^9. */
    int top = LIMBS - 1;
    while (top >= 0 && big_integer[top] == 0)
        top--;
    char chunks[DIGITS];
    int count = 0;
    while (top >= 0) {
        uint64_t remainder = 0;
        for (int i = top; i >= 0; i--) {
            uint64_t current = (remainder << 32) | big_integer[i];
            big_integer[i] = (uint32_t)(current / 1000000000u);
            remainder = current % 1000000000u;
        }
        while (top >= 0 && big_integer[top] == 0)
            top--;
        for (int i = 0; i < 9 && count < DIGITS; i++) {
            chunks[count++] = (char)('0' + remainder % 10);
            remainder /= 10;
        }
    }
    while (count > 1 && chunks[count - 1] == '0')
        count--;
    if (count == 0)
        chunks[count++] = '0';
    for (int i = 0; i < count; i++)
        digit_buffer[i] = chunks[count - 1 - i];
    d->integer_digits = count;
}

/* The next fraction digit, or -1 when the fraction is exhausted (zero). */
static int next_fraction_digit(struct decimal *d) {
    int limbs = d->fraction_limbs;
    if (limbs == 0)
        return -1;
    uint64_t carry = 0;
    int nonzero = 0;
    for (int i = 0; i < limbs; i++) {
        uint64_t t = (uint64_t)big_fraction[i] * 10 + carry;
        big_fraction[i] = (uint32_t)t;
        carry = t >> 32;
        nonzero |= big_fraction[i] != 0;
    }
    if (!nonzero)
        return -1;
    int k = d->fraction_bits;
    int limb = k / 32, bit = k % 32;
    uint64_t window = big_fraction[limb] | ((uint64_t)big_fraction[limb + 1] << 32);
    int digit = (int)(window >> bit);
    window &= ((uint64_t)1 << bit) - 1;
    big_fraction[limb] = (uint32_t)window;
    big_fraction[limb + 1] = 0;
    return digit;
}

/* Whether any fraction bits remain. */
static int fraction_left(const struct decimal *d) {
    for (int i = 0; i < d->fraction_limbs; i++)
        if (big_fraction[i])
            return 1;
    return 0;
}

/* Rounds digits[0..n) up by one unit in the last place, returning whether
   a new leading digit was needed (written at digits[-1]). */
static int round_up(char *digits, int n) {
    for (int i = n - 1; i >= 0; i--) {
        if (digits[i] == '9') {
            digits[i] = '0';
        } else {
            digits[i]++;
            return 0;
        }
    }
    digits[-1] = '1';
    return 1;
}

/* Whether to round up given the next digit and whether anything nonzero
   follows it, ties to even on the last kept digit. */
static int should_round(int next, int sticky, char last) {
    if (next > 5)
        return 1;
    if (next < 5)
        return 0;
    return sticky || ((last - '0') & 1);
}

struct floating {
    int negative;
    int kind; /* 0 finite, 1 infinity, 2 NaN */
    uint64_t hi, lo;  /* significand */
    int exponent;     /* value = significand * 2^exponent */
    int zero;
    int mantissa_bits; /* for %a: fraction bits of the format */
    int bias;
};

static void decode_double(struct floating *f, double value) {
    union {
        double d;
        uint64_t u;
    } bits = {value};
    uint64_t u = bits.u;
    f->negative = (int)(u >> 63);
    int biased = (int)((u >> 52) & 0x7FF);
    uint64_t fraction = u & 0xFFFFFFFFFFFFFull;
    f->hi = 0;
    f->mantissa_bits = 52;
    f->bias = 1023;
    f->kind = biased == 0x7FF ? (fraction ? 2 : 1) : 0;
    f->zero = biased == 0 && fraction == 0;
    if (biased == 0) {
        f->lo = fraction;
        f->exponent = -1074;
    } else {
        f->lo = fraction | (1ull << 52);
        f->exponent = biased - 1075;
    }
}

static void decode_long_double(struct floating *f, long double value) {
    union {
        long double ld;
        struct {
            uint64_t lo, hi;
        } w;
    } bits;
    bits.ld = value;
    uint64_t hi = bits.w.hi, lo = bits.w.lo;
    f->negative = (int)(hi >> 63);
    int biased = (int)((hi >> 48) & 0x7FFF);
    uint64_t fraction_hi = hi & 0xFFFFFFFFFFFFull;
    f->mantissa_bits = 112;
    f->bias = 16383;
    f->kind = biased == 0x7FFF ? ((fraction_hi | lo) ? 2 : 1) : 0;
    f->zero = biased == 0 && fraction_hi == 0 && lo == 0;
    f->lo = lo;
    if (biased == 0) {
        f->hi = fraction_hi;
        f->exponent = -16494;
    } else {
        f->hi = fraction_hi | (1ull << 48);
        f->exponent = biased - 16495;
    }
}

static void format_special(struct sink *sink, struct spec *s, const struct floating *f, int upper) {
    const char *text = f->kind == 1 ? (upper ? "INF" : "inf") : (upper ? "NAN" : "nan");
    char prefix[1];
    size_t prefix_len = 0;
    if (f->negative)
        prefix[prefix_len++] = '-';
    else if (s->flags & PLUS)
        prefix[prefix_len++] = '+';
    else if (s->flags & SPACE)
        prefix[prefix_len++] = ' ';
    s->flags &= ~ZERO;
    field(sink, s, prefix, prefix_len, 0, text, 3, 0, 0);
}

/* %f: the integer digits and `precision` fraction digits, rounded. */
static void format_fixed(struct sink *sink, struct spec *s, struct floating *f, int precision, int strip) {
    struct decimal d;
    expand(&d, f->hi, f->lo, f->exponent);
    char *digits = digit_buffer + 1;
    memmove(digits, digit_buffer, (size_t)d.integer_digits);
    int n = d.integer_digits;
    int stored = precision;
    if (n + stored + 2 > DIGITS)
        stored = DIGITS - n - 2;
    for (int i = 0; i < stored; i++) {
        int digit = next_fraction_digit(&d);
        digits[n++] = (char)('0' + (digit < 0 ? 0 : digit));
    }
    int next = stored == precision ? next_fraction_digit(&d) : 0;
    if (next < 0)
        next = 0;
    int sticky = fraction_left(&d);
    int integer_digits = d.integer_digits;
    if (should_round(next, sticky, digits[n - 1])) {
        if (round_up(digits, n)) {
            digits--;
            n++;
            integer_digits++;
        }
    }
    /* Only `stored` fraction digits are in the buffer; the rest of the
       precision (all zeros for every binary64 value) is written as
       padding, never copied. */
    int extra_zeros = precision - stored;
    int fraction = stored;
    if (strip && !(s->flags & ALT)) {
        extra_zeros = 0;
        while (fraction > 0 && digits[integer_digits + fraction - 1] == '0')
            fraction--;
    }
    char prefix[1];
    size_t prefix_len = 0;
    if (f->negative)
        prefix[prefix_len++] = '-';
    else if (s->flags & PLUS)
        prefix[prefix_len++] = '+';
    else if (s->flags & SPACE)
        prefix[prefix_len++] = ' ';
    /* Assemble integer digits, the point and the fraction in place. */
    char *body = digits;
    size_t body_len = (size_t)integer_digits;
    int point = fraction > 0 || extra_zeros > 0 || (s->flags & ALT);
    if (point) {
        memmove(body + integer_digits + 1, body + integer_digits, (size_t)fraction);
        body[integer_digits] = '.';
        body_len += 1 + (size_t)fraction;
    }
    field(sink, s, prefix, prefix_len, 0, body, body_len, 0, extra_zeros);
}

/* The significant digits of f: `count` of them rounded, and the decimal
   exponent of the first. */
static int significant(struct floating *f, char *out, int count) {
    struct decimal d;
    expand(&d, f->hi, f->lo, f->exponent);
    int exponent;
    int n = 0;
    int next = 0;
    if (!(d.integer_digits == 1 && digit_buffer[0] == '0')) {
        exponent = d.integer_digits - 1;
        int i = 0;
        for (; i < d.integer_digits && n < count; i++)
            out[n++] = digit_buffer[i];
        if (n < count) {
            while (n < count) {
                int digit = next_fraction_digit(&d);
                out[n++] = (char)('0' + (digit < 0 ? 0 : digit));
            }
            next = next_fraction_digit(&d);
        } else if (i < d.integer_digits) {
            next = digit_buffer[i++] - '0';
            int sticky = 0;
            for (; i < d.integer_digits; i++)
                sticky |= digit_buffer[i] != '0';
            sticky |= fraction_left(&d);
            if (should_round(next, sticky, out[n - 1])) {
                if (round_up(out, n)) {
                    memmove(out + 1, out, (size_t)n - 1);
                    out[0] = '1';
                    exponent++;
                }
            }
            return exponent;
        } else {
            next = next_fraction_digit(&d);
        }
    } else {
        exponent = -1;
        int digit;
        while ((digit = next_fraction_digit(&d)) == 0)
            exponent--;
        out[n++] = (char)('0' + (digit < 0 ? 0 : digit));
        while (n < count) {
            digit = next_fraction_digit(&d);
            out[n++] = (char)('0' + (digit < 0 ? 0 : digit));
        }
        next = next_fraction_digit(&d);
    }
    if (next < 0)
        next = 0;
    int sticky = fraction_left(&d);
    if (should_round(next, sticky, out[n - 1])) {
        if (round_up(out, n)) {
            memmove(out + 1, out, (size_t)n - 1);
            out[0] = '1';
            exponent++;
        }
    }
    return exponent;
}

static char exponent_text[16];

static const char *exponent_suffix(char letter, int exponent, int min_digits) {
    char *p = exponent_text;
    *p++ = letter;
    *p++ = exponent < 0 ? '-' : '+';
    unsigned e = exponent < 0 ? (unsigned)-exponent : (unsigned)exponent;
    char digits[8];
    int n = 0;
    do {
        digits[n++] = (char)('0' + e % 10);
        e /= 10;
    } while (e);
    while (n < min_digits)
        digits[n++] = '0';
    while (n)
        *p++ = digits[--n];
    *p = 0;
    return exponent_text;
}

static char significant_buffer[DIGITS];

/* %e with `precision` digits after the point. */
static void format_exponent(struct sink *sink, struct spec *s, struct floating *f, int precision, int strip,
                            int upper) {
    int count = precision + 1;
    if (count > DIGITS - 2)
        count = DIGITS - 2;
    char *digits = significant_buffer + 1;
    int exponent = 0;
    if (f->zero) {
        memset(digits, '0', (size_t)count);
    } else {
        exponent = significant(f, digits, count);
    }
    int fraction = count - 1;
    /* A precision beyond the buffer is padded with zeros. */
    int extra_zeros = precision - fraction;
    if (strip && !(s->flags & ALT)) {
        extra_zeros = 0;
        while (fraction > 0 && digits[fraction] == '0')
            fraction--;
    }
    char prefix[1];
    size_t prefix_len = 0;
    if (f->negative)
        prefix[prefix_len++] = '-';
    else if (s->flags & PLUS)
        prefix[prefix_len++] = '+';
    else if (s->flags & SPACE)
        prefix[prefix_len++] = ' ';
    char *body = digits - 1;
    body[0] = digits[0];
    size_t body_len = 1;
    if (fraction > 0 || extra_zeros > 0 || (s->flags & ALT)) {
        body[1] = '.';
        body_len = 2 + (size_t)fraction;
    }
    field(sink, s, prefix, prefix_len, 0, body, body_len, exponent_suffix(upper ? 'E' : 'e', exponent, 2),
          extra_zeros);
}

/* %g: %e or %f by the decimal exponent, trailing zeros removed. */
static void format_general(struct sink *sink, struct spec *s, struct floating *f, int precision, int upper) {
    int p = precision == 0 ? 1 : precision;
    int exponent = 0;
    if (!f->zero) {
        char *digits = significant_buffer + 1;
        int count = p < DIGITS - 2 ? p : DIGITS - 2;
        exponent = significant(f, digits, count);
    }
    if (p > exponent && exponent >= -4)
        format_fixed(sink, s, f, p - 1 - exponent, 1);
    else
        format_exponent(sink, s, f, p - 1, 1, upper);
}

/* %a: hexadecimal significand and binary exponent. */
static void format_hex(struct sink *sink, struct spec *s, struct floating *f, int precision, int upper) {
    const char *alphabet = upper ? "0123456789ABCDEF" : "0123456789abcdef";
    int bits = f->mantissa_bits;
    int nibbles = (bits + 3) / 4;
    /* the fraction bits aligned to whole nibbles */
    uint64_t hi = f->hi, lo = f->lo;
    int lead;
    int exponent;
    uint64_t frac_hi, frac_lo;
    if (bits == 52) {
        lead = (int)(lo >> 52);
        frac_hi = 0;
        frac_lo = lo & 0xFFFFFFFFFFFFFull;
        exponent = f->zero ? 0 : (lead ? f->exponent + 52 : -1022);
    } else {
        lead = (int)(hi >> 48);
        frac_hi = hi & 0xFFFFFFFFFFFFull;
        frac_lo = lo;
        exponent = f->zero ? 0 : (lead ? f->exponent + 112 : -16382);
    }
    char hex[32];
    for (int i = 0; i < nibbles; i++) {
        int shift = (nibbles - 1 - i) * 4;
        uint64_t nibble;
        if (shift >= 64)
            nibble = (frac_hi >> (shift - 64)) & 15;
        else if (shift + 4 > 64)
            nibble = ((frac_lo >> shift) | (frac_hi << (64 - shift))) & 15;
        else
            nibble = (frac_lo >> shift) & 15;
        hex[i] = alphabet[nibble];
    }
    int count = nibbles;
    if (precision < 0) {
        while (count > 0 && hex[count - 1] == '0')
            count--;
    } else if (precision < nibbles) {
        /* round to nearest, ties to even, on the hexadecimal digits */
        int next = 0;
        for (int i = 0; i < 16; i++)
            if (alphabet[i] == hex[precision])
                next = i;
        int sticky = 0;
        for (int i = precision + 1; i < nibbles; i++)
            sticky |= hex[i] != '0';
        int last = precision > 0 ? 0 : lead;
        if (precision > 0)
            for (int i = 0; i < 16; i++)
                if (alphabet[i] == hex[precision - 1])
                    last = i;
        int up = next > 8 || (next == 8 && (sticky || (last & 1)));
        count = precision;
        if (up) {
            int i = count - 1;
            for (; i >= 0; i--) {
                int v = 0;
                for (int j = 0; j < 16; j++)
                    if (alphabet[j] == hex[i])
                        v = j;
                if (v == 15) {
                    hex[i] = '0';
                } else {
                    hex[i] = alphabet[v + 1];
                    break;
                }
            }
            if (i < 0)
                lead++;
        }
    } else {
        count = nibbles;
    }
    int zeros = precision > count ? precision - count : 0;
    char prefix[4];
    size_t prefix_len = 0;
    if (f->negative)
        prefix[prefix_len++] = '-';
    else if (s->flags & PLUS)
        prefix[prefix_len++] = '+';
    else if (s->flags & SPACE)
        prefix[prefix_len++] = ' ';
    prefix[prefix_len++] = '0';
    prefix[prefix_len++] = upper ? 'X' : 'x';
    char body[40];
    size_t body_len = 0;
    body[body_len++] = alphabet[lead];
    if (count > 0 || zeros > 0 || (s->flags & ALT))
        body[body_len++] = '.';
    memcpy(body + body_len, hex, (size_t)count);
    body_len += (size_t)count;
    (void)hi;
    field(sink, s, prefix, prefix_len, 0, body, body_len, exponent_suffix(upper ? 'P' : 'p', exponent, 1),
          zeros);
}

static void format_float(struct sink *sink, struct spec *s, struct floating *f) {
    char c = s->conversion;
    int upper = c == 'F' || c == 'E' || c == 'G' || c == 'A';
    if (f->kind) {
        format_special(sink, s, f, upper);
        return;
    }
    int precision = s->precision < 0 ? 6 : s->precision;
    switch (c) {
    case 'f':
    case 'F':
        format_fixed(sink, s, f, precision, 0);
        break;
    case 'e':
    case 'E':
        format_exponent(sink, s, f, precision, 0, upper);
        break;
    case 'g':
    case 'G':
        format_general(sink, s, f, precision, upper);
        break;
    default:
        format_hex(sink, s, f, s->precision, upper);
        break;
    }
}

static int format(struct sink *sink, const char *fmt, va_list ap) {
    while (*fmt) {
        if (*fmt != '%') {
            const char *start = fmt;
            while (*fmt && *fmt != '%')
                fmt++;
            emit(sink, start, (size_t)(fmt - start));
            continue;
        }
        fmt++;
        struct spec s = {0, 0, -1, 0};
        for (;; fmt++) {
            if (*fmt == '-')
                s.flags |= LEFT;
            else if (*fmt == '+')
                s.flags |= PLUS;
            else if (*fmt == ' ')
                s.flags |= SPACE;
            else if (*fmt == '#')
                s.flags |= ALT;
            else if (*fmt == '0')
                s.flags |= ZERO;
            else
                break;
        }
        if (*fmt == '*') {
            s.width = va_arg(ap, int);
            if (s.width < 0) {
                s.flags |= LEFT;
                s.width = -s.width;
            }
            fmt++;
        } else {
            while (*fmt >= '0' && *fmt <= '9')
                s.width = s.width * 10 + (*fmt++ - '0');
        }
        if (*fmt == '.') {
            fmt++;
            s.precision = 0;
            if (*fmt == '*') {
                s.precision = va_arg(ap, int);
                if (s.precision < 0)
                    s.precision = -1;
                fmt++;
            } else {
                while (*fmt >= '0' && *fmt <= '9')
                    s.precision = s.precision * 10 + (*fmt++ - '0');
            }
        }
        int length = 0; /* -2 hh, -1 h, 1 l, 2 ll/j, 3 z/t, 4 L */
        switch (*fmt) {
        case 'h':
            length = -1;
            if (*++fmt == 'h') {
                length = -2;
                fmt++;
            }
            break;
        case 'l':
            length = 1;
            if (*++fmt == 'l') {
                length = 2;
                fmt++;
            }
            break;
        case 'j':
        case 'q':
            length = 2;
            fmt++;
            break;
        case 'z':
        case 't':
            length = 3;
            fmt++;
            break;
        case 'L':
            length = 4;
            fmt++;
            break;
        }
        if (s.flags & LEFT)
            s.flags &= ~ZERO;
        s.conversion = *fmt;
        if (*fmt)
            fmt++;
        switch (s.conversion) {
        case 'd':
        case 'i': {
            long long v;
            if (length == 2)
                v = va_arg(ap, long long);
            else if (length == 1 || length == 3)
                v = va_arg(ap, long);
            else
                v = va_arg(ap, int);
            if (length == -1)
                v = (short)v;
            else if (length == -2)
                v = (signed char)v;
            unsigned long long magnitude = v < 0 ? 0ull - (unsigned long long)v : (unsigned long long)v;
            format_integer(sink, &s, magnitude, v < 0);
            break;
        }
        case 'u':
        case 'o':
        case 'x':
        case 'X': {
            unsigned long long v;
            if (length == 2)
                v = va_arg(ap, unsigned long long);
            else if (length == 1 || length == 3)
                v = va_arg(ap, unsigned long);
            else
                v = va_arg(ap, unsigned int);
            if (length == -1)
                v = (unsigned short)v;
            else if (length == -2)
                v = (unsigned char)v;
            s.flags &= ~(PLUS | SPACE);
            format_integer(sink, &s, v, 0);
            break;
        }
        case 'p': {
            void *p = va_arg(ap, void *);
            if (!p) {
                s.flags &= ~ZERO;
                s.precision = -1;
                field(sink, &s, "", 0, 0, "(nil)", 5, 0, 0);
            } else {
                s.flags &= ~(PLUS | SPACE);
                format_integer(sink, &s, (uintptr_t)p, 0);
            }
            break;
        }
        case 'c': {
            char c = (char)va_arg(ap, int);
            s.flags &= ~ZERO;
            field(sink, &s, "", 0, 0, &c, 1, 0, 0);
            break;
        }
        case 's': {
            const char *str = va_arg(ap, const char *);
            if (!str)
                str = (s.precision < 0 || s.precision >= 6) ? "(null)" : "";
            size_t n = s.precision < 0 ? strlen(str) : strnlen(str, (size_t)s.precision);
            s.flags &= ~ZERO;
            field(sink, &s, "", 0, 0, str, n, 0, 0);
            break;
        }
        case 'n': {
            if (length == 2)
                *va_arg(ap, long long *) = (long long)sink->count;
            else if (length == 1 || length == 3)
                *va_arg(ap, long *) = (long)sink->count;
            else if (length == -1)
                *va_arg(ap, short *) = (short)sink->count;
            else if (length == -2)
                *va_arg(ap, signed char *) = (signed char)sink->count;
            else
                *va_arg(ap, int *) = (int)sink->count;
            break;
        }
        case 'f':
        case 'F':
        case 'e':
        case 'E':
        case 'g':
        case 'G':
        case 'a':
        case 'A': {
            struct floating f;
            if (length == 4)
                decode_long_double(&f, va_arg(ap, long double));
            else
                decode_double(&f, va_arg(ap, double));
            format_float(sink, &s, &f);
            break;
        }
        case '%':
            emit(sink, "%", 1);
            break;
        default:
            break;
        }
    }
    return (int)sink->count;
}

int vfprintf(FILE *f, const char *fmt, va_list ap) {
    struct sink sink = {f, 0, 0, 0};
    int count = format(&sink, fmt, ap);
    return f->error ? -1 : count;
}

int vprintf(const char *fmt, va_list ap) { return vfprintf(stdout, fmt, ap); }

int vsnprintf(char *buffer, size_t n, const char *fmt, va_list ap) {
    struct sink sink = {0, buffer, n, 0};
    int count = format(&sink, fmt, ap);
    if (n > 0)
        buffer[sink.count < n ? sink.count : n - 1] = 0;
    return count;
}

int vsprintf(char *buffer, const char *fmt, va_list ap) {
    return vsnprintf(buffer, (size_t)INT_MAX, fmt, ap);
}

int printf(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int count = vfprintf(stdout, fmt, ap);
    va_end(ap);
    return count;
}

int fprintf(FILE *f, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int count = vfprintf(f, fmt, ap);
    va_end(ap);
    return count;
}

int sprintf(char *buffer, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int count = vsprintf(buffer, fmt, ap);
    va_end(ap);
    return count;
}

int snprintf(char *buffer, size_t n, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int count = vsnprintf(buffer, n, fmt, ap);
    va_end(ap);
    return count;
}
