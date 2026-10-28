//! The bundled runtime against glibc: the cases of review rounds 1 and 2
//! of the WebAssembly back end (scanf length modifiers, numeric prefixes
//! and incomplete numbers, printf precisions past its buffers and
//! hexadecimal precision, floating to 128-bit conversions at their limits,
//! allocation size overflow and memory's last page, trigonometric argument
//! reduction, NaN payloads and nextafter), each compiled for
//! `wasm32-wasip1` at `-O0` and `-O2` and run under Node. Expected outputs are
//! glibc's, except where the 32-bit data model or binary128 `long double`
//! decides otherwise (marked).

use super::execution::{
    compile,
    node_available,
    run,
};
use crate::target::Target;

/// Runs `source` at both optimization levels and returns its stdout.
fn outputs(name: &str, source: &str) -> Option<[String; 2]> {
    if !node_available() {
        eprintln!("note: node is not on the path; skipping {name}");
        return None;
    }
    Some([false, true].map(|optimize| {
        let bytes = compile(source, Target::Wasm32Wasi, optimize).unwrap();
        let run = run(name, &bytes);
        assert_eq!(
            run.status, 0,
            "{name} (optimize {optimize}): {}",
            run.stderr
        );
        run.stdout
    }))
}

#[test]
fn scanf_honours_length_modifiers_and_stops_after_the_number() {
    let source = r#"#include <stdio.h>
#include <stddef.h>
#include <stdint.h>
int main(void) {
    struct { signed char n; unsigned char guard[3]; } s = {-1, {17, 34, 51}};
    int r = sscanf("abc", "abc%hhn", &s.n);
    printf("%d %d %u %u %u\n", r, s.n, s.guard[0], s.guard[1], s.guard[2]);
    long long ll = -1; short h = -1; long l = -1; intmax_t j = -1; size_t z = 99; ptrdiff_t t = 99;
    r = sscanf("abcdef", "ab%llnc%hnd%lne%jnf%zn%tn", &ll, &h, &l, &j, &z, &t);
    printf("%d %lld %d %ld %lld %zu %td\n", r, ll, h, l, (long long)j, z, t);
    struct { short v; unsigned char guard[2]; } w = {0, {7, 8}};
    signed char c = 0; unsigned long long u = 0;
    r = sscanf("300 -2 18446744073709551615", "%hd %hhd %llu", &w.v, &c, &u);
    printf("%d %d %u %u %d %llu\n", r, w.v, w.guard[0], w.guard[1], c, u);
    int a = 0, n = -1; char str[20] = "untouched";
    r = sscanf("123abc", "%d%n%19s", &a, &n, str);
    printf("%d %d %d %s\n", r, a, n, str);
    char ch = '?'; a = 0;
    r = sscanf("42q", "%d%c", &a, &ch);
    printf("%d %d %c\n", r, a, ch);
    unsigned x = 0; ch = '?';
    r = sscanf("0x1fg", "%x%c", &x, &ch);
    printf("%d %u %c\n", r, x, ch);
    double d = 0; ch = '?';
    r = sscanf("2.5e3x", "%lf%c", &d, &ch);
    printf("%d %g %c\n", r, d, ch);
    r = sscanf("-17 0777 0x10", "%i %i %i", &a, &n, (int *)&x);
    printf("%d %d %d %u\n", r, a, n, x);
    float fl = 0; ch = '?';
    r = sscanf("inf,", "%f%c", &fl, &ch);
    printf("%d %g %c\n", r, (double)fl, ch);
    r = sscanf("12345", "%3d%d", &a, &n);
    printf("%d %d %d\n", r, a, n);
    return 0;
}
"#;
    let expected = [
        "0 3 17 34 51",
        "0 2 3 4 5 6 6",
        "3 300 7 8 -2 18446744073709551615",
        "2 123 3 abc",
        "2 42 q",
        "2 31 g",
        "2 2500 x",
        "3 -17 511 16",
        "2 inf ,",
        "2 123 45",
    ];
    for output in outputs("scanf-review", source).into_iter().flatten() {
        assert_eq!(output, expected.join("\n") + "\n");
    }
}

/// A numeric conversion whose longest matching prefix is not a whole
/// number is a matching failure (C99 7.19.6.2p10); `%i` reads hexadecimal
/// after `0x`; `%p` reads a pointer; `L` means `long long` for `%n` and
/// `%d`, as in glibc.
#[test]
fn scanf_rejects_incomplete_numbers_and_reads_hexadecimal() {
    let source = r#"#include <stdio.h>
#include <string.h>
static void ints(const char *in, const char *fmt) {
    int a = -77, n = -1; char rest[16] = "";
    int r = sscanf(in, fmt, &a, &n, rest);
    printf("%s|%s: %d %d %d %s\n", in, fmt, r, a, n, rest);
}
static void flts(const char *in, const char *fmt) {
    double a = -77; int n = -1; char rest[16] = "";
    int r = sscanf(in, fmt, &a, &n, rest);
    printf("%s|%s: %d %g %d %s\n", in, fmt, r, a, n, rest);
}
int main(void) {
    ints("0x1A rest", "%i%n%15s");
    ints("0XfF", "%i%n%15s");
    ints("0xff", "%x%n%15s");
    ints("-0x10 z", "%i%n%15s");
    ints("0xg", "%x%n%15s");
    ints("0xg", "%i%n%15s");
    ints("0x", "%x%n%15s");
    ints("0x123", "%2i%n%15s");
    ints("09", "%i%n%15s");
    ints("-", "%d%n%15s");
    flts("1e", "%lf%n%15s");
    flts("1e+", "%lf%n%15s");
    flts("1ex", "%lf%n%15s");
    flts("1e+x", "%lf%n%15s");
    flts("infin", "%lf%n%15s");
    flts("0x1p", "%lf%n%15s");
    flts("0xz", "%lf%n%15s");
    flts("1e5", "%2lf%n%15s");
    flts("nan(123)x", "%lf%n%15s");
    flts("nan(12x", "%lf%n%15s");
    flts("0x1.8p1z", "%lf%n%15s");
    flts(".5x", "%lf%n%15s");
    flts(".x", "%lf%n%15s");
    long long buf[2] = {-1, -1};
    int r = sscanf("abcde", "abc%Ln", &buf[0]);
    printf("%d %lld %lld\n", r, buf[0], buf[1]);
    r = sscanf("-77", "%Ld", &buf[1]);
    printf("%d %lld\n", r, buf[1]);
    void *p = 0; int n = -1;
    r = sscanf("0x1234abcd", "%p%n", &p, &n);
    printf("%d %p %d\n", r, p, n);
    return 0;
}
"#;
    let expected = [
        "0x1A rest|%i%n%15s: 2 26 4 rest",
        "0XfF|%i%n%15s: 1 255 4 ",
        "0xff|%x%n%15s: 1 255 4 ",
        "-0x10 z|%i%n%15s: 2 -16 5 z",
        "0xg|%x%n%15s: 0 -77 -1 ",
        "0xg|%i%n%15s: 0 -77 -1 ",
        "0x|%x%n%15s: 0 -77 -1 ",
        "0x123|%2i%n%15s: 0 -77 -1 ",
        "09|%i%n%15s: 2 0 1 9",
        "-|%d%n%15s: 0 -77 -1 ",
        "1e|%lf%n%15s: 0 -77 -1 ",
        "1e+|%lf%n%15s: 0 -77 -1 ",
        "1ex|%lf%n%15s: 0 -77 -1 ",
        "1e+x|%lf%n%15s: 0 -77 -1 ",
        "infin|%lf%n%15s: 0 -77 -1 ",
        "0x1p|%lf%n%15s: 0 -77 -1 ",
        "0xz|%lf%n%15s: 0 -77 -1 ",
        "1e5|%2lf%n%15s: 0 -77 -1 ",
        "nan(123)x|%lf%n%15s: 2 nan 8 x",
        "nan(12x|%lf%n%15s: 0 -77 -1 ",
        "0x1.8p1z|%lf%n%15s: 2 3 7 z",
        ".5x|%lf%n%15s: 2 0.5 2 x",
        ".x|%lf%n%15s: 0 -77 -1 ",
        "0 3 -1",
        "1 -77",
        "1 0x1234abcd 10",
    ];
    for output in outputs("scanf-review-2", source).into_iter().flatten() {
        assert_eq!(output, expected.join("\n") + "\n");
    }
}

#[test]
fn printf_precisions_beyond_the_buffers_are_padded() {
    let source = r#"#include <stdio.h>
#include <string.h>
static char big[1100000];
int main(void) {
    char one[1];
    int n = snprintf(one, sizeof one, "%.1000000f", 1.25);
    printf("%d %d\n", n, one[0]);
    n = snprintf(big, sizeof big, "%.20000e", 1.25);
    printf("%d %zu %.12s %s\n", n, strlen(big), big, big + n - 8);
    n = snprintf(big, sizeof big, "%.30000f", 1e-300);
    printf("%d %c %c\n", n, big[302], big[n - 1]);
    n = snprintf(big, sizeof big, "%#.5000g", 2.0);
    printf("%d %.8s\n", n, big + n - 8);
    volatile double x = 1.5;
    printf("[%.20a] [%#.16a] [%30.16a] [%-24.14A] [%.0a] [%.1a]\n", x, x, x, x, x, 1.96875);
    n = snprintf(one, sizeof one, "%.40a", x);
    printf("%d\n", n);
    return 0;
}
"#;
    let expected = [
        "1000002 0",
        "20006 20006 1.2500000000 0000e+00",
        "30002 0 0",
        "5001 00000000",
        "[0x1.80000000000000000000p+0] [0x1.8000000000000000p+0] [       0x1.8000000000000000p+0] \
         [0X1.80000000000000P+0   ] [0x2p+0] [0x2.0p+0]",
        "47",
    ];
    for output in outputs("printf-review", source).into_iter().flatten() {
        assert_eq!(output, expected.join("\n") + "\n");
    }
}

#[test]
fn floating_to_128_bit_conversions_follow_compiler_rt() {
    let source = r#"#include <stdio.h>
#define HALVES(v) (unsigned long long)((unsigned __int128)(v) >> 64), (unsigned long long)(v)
int main(void) {
    volatile double a = 0x1p127, b = 0x1.8p127, c = -0x1p127, d = 0x1p128, e = -1.5, f = 0x1.fffffffffffffp126;
    unsigned __int128 x = (unsigned __int128)a, y = (unsigned __int128)b, w = (unsigned __int128)f;
    __int128 z = (__int128)c, k = (__int128)e;
    printf("%llx:%016llx %llx:%016llx %llx:%016llx %llx:%016llx %llx:%016llx\n", HALVES(x), HALVES(y), HALVES(z),
           HALVES(w), HALVES(k));
    volatile long double p = 0x1p127L, q = 0x1.8p127L, r = -0x1p127L, s = 0x1.ffffffffffffffffffffffffffffp126L;
    unsigned __int128 px = (unsigned __int128)p, qx = (unsigned __int128)q, sx = (unsigned __int128)s;
    __int128 rx = (__int128)r;
    printf("%llx:%016llx %llx:%016llx %llx:%016llx %llx:%016llx\n", HALVES(px), HALVES(qx), HALVES(rx), HALVES(sx));
    volatile long double m = -0x1p63L, big = 0x1p40L;
    printf("%lld %llu %d %u\n", (long long)m, (unsigned long long)big, (int)(long double)-0x1p31L, (unsigned)(long double)4000000000.0L);
    (void)d;
    return 0;
}
"#;
    // glibc on x86-64, except the fourth long double: binary128 holds
    // 0x1.ff...fp126 exactly, where the x87 format rounds it to 2^127.
    let expected = [
        "8000000000000000:0000000000000000 c000000000000000:0000000000000000 \
         8000000000000000:0000000000000000 7ffffffffffffc00:0000000000000000 \
         ffffffffffffffff:ffffffffffffffff",
        "8000000000000000:0000000000000000 c000000000000000:0000000000000000 \
         8000000000000000:0000000000000000 7fffffffffffffff:ffffffffffffc000",
        "-9223372036854775808 1099511627776 -2147483648 4000000000",
    ];
    for output in outputs("convert-review", source).into_iter().flatten() {
        assert_eq!(output, expected.join("\n") + "\n");
    }
}

#[test]
fn allocation_sizes_that_overflow_fail_with_enomem() {
    let source = r#"#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
int main(void) {
    errno = 0;
    void *p = aligned_alloc(0x100000u, SIZE_MAX - 0xfffffu);
    printf("%d %d\n", p == 0, errno == ENOMEM);
    free(p);
    errno = 0;
    p = aligned_alloc(0x100000u, (size_t)0xfff00000u);
    printf("%d %d\n", p == 0, errno == ENOMEM);
    errno = 0;
    p = calloc(SIZE_MAX / 2, 4);
    printf("%d %d\n", p == 0, errno == ENOMEM);
    errno = 0;
    p = malloc(SIZE_MAX - 8);
    printf("%d %d\n", p == 0, errno == ENOMEM);
    char *q = malloc(16);
    errno = 0;
    char *r = realloc(q, SIZE_MAX - 4);
    printf("%d %d\n", r == 0, errno == ENOMEM);
    free(q);
    void *aligned = aligned_alloc(4096, 100);
    printf("%d\n", ((uintptr_t)aligned & 4095) == 0);
    free(aligned);
    return 0;
}
"#;
    // On wasm32 every one of these sizes is too large, so each request
    // fails with ENOMEM (glibc on x86-64 can satisfy the second).
    for output in outputs("alloc-review", source).into_iter().flatten() {
        assert_eq!(output, "1 1\n1 1\n1 1\n1 1\n1 1\n1\n");
    }
}

/// The heap never grows memory to all 65536 pages, whose end would wrap to
/// 0 as a pointer, but takes every page before it: a request that needs
/// the last page fails, one that fills memory up to it succeeds, and the
/// heap keeps working after both.
#[test]
fn the_heap_takes_every_page_but_the_last() {
    let source = r#"#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
size_t __bcc_wasm_memory_size(void);
int main(void) {
    size_t big = 0x70000000u;
    char *a = malloc(big), *b = malloc(big);
    size_t pages = __bcc_wasm_memory_size();
    uintptr_t end = (uintptr_t)b - 16 + ((big + 31) & ~(size_t)15);
    size_t rest = (size_t)(pages * 65536ull - end);
    /* Exactly what all of memory up to its last page holds. */
    size_t most = rest + (65536u - pages - 1) * 65536u - 16;
    char *over = malloc(most + 16);
    char *c = malloc(most);
    printf("%d %d %d %d %d\n", a != 0, b != 0, over == 0, c != 0, __bcc_wasm_memory_size() == 65535);
    char *full = malloc(100);
    printf("%d %d\n", full == 0, (uintptr_t)c + most == 0xffff0000u);
    free(c);
    char *d = malloc(100), *e = malloc(100);
    free(a);
    char *f = malloc(1u << 20);
    printf("%d %d\n", d != 0 && e != 0 && d != e, f != 0);
    return 0;
}
"#;
    for output in outputs("alloc-last-page", source).into_iter().flatten() {
        assert_eq!(output, "1 1 1 1 1\n1 1\n1 1\n");
    }
}

/// NaN payloads from `nan`, `nanf`, `strtod` and `strtof`, and `nextafter`
/// with its `ERANGE` cases, as glibc gives them.
#[test]
fn nan_payloads_and_nextafter_match_glibc() {
    let source = r#"#include <errno.h>
#include <float.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
static unsigned long long dbits(double d) { unsigned long long u; memcpy(&u, &d, 8); return u; }
static unsigned fbits(float f) { unsigned u; memcpy(&u, &f, 4); return u; }
#define NEXT(f, x, y) do { volatile double vx = (x), vy = (y); errno = 0; double v = f(vx, vy); printf("%a %d\n", v, errno == ERANGE); } while (0)
int main(void) {
    printf("%016llx %016llx %016llx %016llx\n", dbits(nan("123")), dbits(nan("0x7b")), dbits(nan("")), dbits(nan("abc")));
    printf("%08x %08x\n", fbits(nanf("123")), fbits(nanf("")));
    printf("%016llx %016llx %08x\n", dbits(strtod("nan(42)", 0)), dbits(strtod("-nan(0x10)", 0)), fbits(strtof("nan(42)", 0)));
    NEXT(nextafter, 1.0, 2.0);
    NEXT(nextafter, 1.0, 0.0);
    NEXT(nextafter, -1.0, 0.0);
    NEXT(nextafter, 0.0, -1.0);
    NEXT(nextafter, DBL_MAX, INFINITY);
    NEXT(nextafter, INFINITY, 0.0);
    NEXT(nextafter, DBL_MIN, 0.0);
    NEXT(nextafter, 0x1p-1074, 0.0);
    NEXT(nextafter, -0.0, 0.0);
    NEXT(nextafterf, 1.0f, 2.0f);
    NEXT(nextafterf, 0.0f, -1.0f);
    NEXT(nextafterf, FLT_MAX, INFINITY);
    NEXT(nextafterf, -FLT_MIN, -INFINITY);
    printf("%d %d\n", isnan(nextafter(NAN, 1.0)), isnan(nextafterf(1.0f, NAN)));
    return 0;
}
"#;
    let expected = [
        "7ff800000000007b 7ff800000000007b 7ff8000000000000 7ff8000000000000",
        "7fc0007b 7fc00000",
        "7ff800000000002a fff8000000000010 7fc0002a",
        "0x1.0000000000001p+0 0",
        "0x1.fffffffffffffp-1 0",
        "-0x1.fffffffffffffp-1 0",
        "-0x0.0000000000001p-1022 0",
        "inf 1",
        "0x1.fffffffffffffp+1023 0",
        "0x0.fffffffffffffp-1022 1",
        "0x0p+0 1",
        "0x0p+0 0",
        "0x1.000002p+0 0",
        "-0x1p-149 0",
        "inf 1",
        "-0x1.000002p-126 0",
        "1 1",
    ];
    for output in outputs("math-review-2", source).into_iter().flatten() {
        assert_eq!(output, expected.join("\n") + "\n");
    }
}

#[test]
fn trigonometry_reduces_every_finite_argument() {
    let source = r#"#include <stdio.h>
#include <math.h>
int main(void) {
    volatile double a[] = {0.5, -3.0, 1e9, 1e12, 1e20, 1e22, 1e100, 1e300, 0x1p1023, -0x1.fffffffffffffp1023,
                           3.141592653589793, 1.5707963267948966, 710.0, 1e-300, 0.0};
    for (int i = 0; i < (int)(sizeof a / sizeof a[0]); i++)
        printf("%.17g %.17g %.17g\n", sin(a[i]), cos(a[i]), tan(a[i]));
    volatile float f = 1e20f;
    printf("%.9g %.9g %.9g\n", (double)sinf(f), (double)cosf(f), (double)tanf(f));
    return 0;
}
"#;
    // glibc's results; fdlibm's agree within an ulp on these inputs (of
    // float for the last line), though not on every input.
    let expected = [
        "0.47942553860420301 0.87758256189037276 0.54630248984379048",
        "-0.14112000805986721 -0.98999249660044542 0.1425465430742778",
        "0.54584344944869956 0.83788718136390239 0.65145220214514132",
        "-0.61123870237688949 0.79144630185289022 -0.77230596813187613",
        "-0.64525128526578079 0.76397040444172826 -0.84460246301988429",
        "-0.85220084976718879 0.52321478539513899 -1.6287782256068988",
        "-0.38063773100502868 0.92472423875193377 -0.41162296288324979",
        "-0.81788191211590855 -0.57538611195754907 1.4214488238747245",
        "0.56312777985088402 -0.82636983461414804 -0.68144764760662146",
        "-0.004961954789184062 -0.99998768942655991 0.0049620158744448951",
        "1.2246467991473532e-16 -1 -1.2246467991473532e-16",
        "1 6.123233995736766e-17 16331239353195370",
        "6.0288706691585265e-05 0.99999999818263596 6.0288706801151796e-05",
        "1e-300 1 1e-300",
        "0 1 0",
        "0.656576693 0.754259288 0.870491982",
    ];
    for output in outputs("trig-review", source).into_iter().flatten() {
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), expected.len(), "{output}");
        for (index, (line, want)) in lines.iter().zip(expected).enumerate() {
            for (got, want) in line.split(' ').zip(want.split(' ')) {
                // The distance in units in the last place, read in the
                // format each line prints.
                let ulps = if index + 1 == expected.len() {
                    let bits = |text: &str| i64::from(text.parse::<f32>().unwrap().to_bits());
                    (bits(got) - bits(want)).unsigned_abs()
                } else {
                    let bits = |text: &str| i128::from(text.parse::<f64>().unwrap().to_bits());
                    u64::try_from((bits(got) - bits(want)).unsigned_abs()).unwrap_or(u64::MAX)
                };
                assert!(ulps <= 1, "{line} against glibc's {want}");
            }
        }
    }
}
