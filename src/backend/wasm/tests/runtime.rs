//! The bundled runtime against glibc: the cases of review rounds 1 to 3
//! of the WebAssembly back end (scanf length modifiers, numeric prefixes,
//! incomplete numbers, input failures and `(nil)`, printf precisions past
//! its buffers and hexadecimal precision, floating to 128-bit conversions
//! at their limits, allocation size overflow and memory's last page,
//! trigonometric argument reduction, NaN payloads and nextafter, and
//! `__fp16` conversions against Clang), each
//! compiled for `wasm32-wasip1` at `-O0` and `-O2` and run under Node.
//! Expected outputs are glibc's, except where the 32-bit data model or
//! binary128 `long double` decides otherwise (marked).

use super::execution::{
    compile,
    node_available,
    run_with_arguments,
    run_with_input,
};
use crate::target::Target;

/// Runs `source` at both optimization levels and returns its stdout.
fn outputs(name: &str, source: &str) -> Option<[String; 2]> {
    outputs_with_input(name, source, "")
}

/// Runs `source` at both optimization levels with `input` on its standard
/// input and returns its stdout.
fn outputs_with_input(name: &str, source: &str, input: &str) -> Option<[String; 2]> {
    if !node_available() {
        eprintln!("note: node is not on the path; skipping {name}");
        return None;
    }
    Some([false, true].map(|optimize| {
        let bytes = compile(source, Target::Wasm32Wasi, optimize).unwrap();
        let run = run_with_input(name, &bytes, input.as_bytes());
        assert_eq!(
            run.status, 0,
            "{name} (optimize {optimize}): {}",
            run.stderr
        );
        run.stdout
    }))
}

/// `_start` reads a few short arguments into static storage and more, or
/// longer, ones into the heap; either way `argv[argc]` is a null pointer
/// and the strings are modifiable (C99 §5.1.2.2.1p2).
#[test]
fn arguments_reach_main_from_static_storage_or_the_heap() {
    if !node_available() {
        eprintln!("note: node is not on the path; skipping the argument test");
        return;
    }
    let source = r#"
#include <stdio.h>
#include <stdlib.h>
int main(int argc, char **argv) {
    void *first = malloc(1);
    unsigned long total = 0;
    for (int i = 1; i < argc; i++) {
        argv[i][0] = 'X';
        for (char *c = argv[i]; *c; c++)
            total += (unsigned char)*c;
    }
    printf("%d %d %lu %s\n", argc, argv[argc] == 0, total, argc > 1 ? argv[1] : "-");
    free(first);
    return 0;
}
"#;
    let long = "y".repeat(300);
    let many: Vec<String> = (0..20).map(|index| format!("a{index}")).collect();
    let many: Vec<&str> = many.iter().map(String::as_str).collect();
    let expected = |arguments: &[&str]| {
        let total: u64 = arguments
            .iter()
            .map(|argument| 88 + argument.bytes().skip(1).map(u64::from).sum::<u64>())
            .sum();
        let first = arguments
            .first()
            .map_or_else(|| "-".to_owned(), |argument| format!("X{}", &argument[1..]));
        format!("{} 1 {total} {first}\n", arguments.len() + 1)
    };
    for optimize in [false, true] {
        let bytes = compile(source, Target::Wasm32Wasi, optimize).unwrap();
        for arguments in [&[][..], &["one", "two", "three"], &many, &[long.as_str()]] {
            let run = run_with_arguments("arguments", &bytes, b"", arguments);
            assert_eq!(run.status, 0, "{}", run.stderr);
            assert_eq!(
                run.stdout,
                expected(arguments),
                "{} arguments",
                arguments.len()
            );
        }
    }
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

/// The end of input before a literal or `%%` is an input failure, so
/// scanf returns EOF (C99 7.19.6.2p16) and a loop over `"(%d)"` ends; `%p`
/// reads printf's `(nil)` as a null pointer; `%f` keeps a NaN's payload;
/// and `nan` keeps `errno` and agrees with `strtod` for long tags.
#[test]
fn scanf_input_failures_nil_pointers_and_nan_tags() {
    let source = r#"#include <errno.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
static unsigned long long dbits(double d) { unsigned long long u; memcpy(&u, &d, 8); return u; }
static unsigned fbits(float f) { unsigned u; memcpy(&u, &f, 4); return u; }
int main(void) {
    int a = -7, n = 0, r;
    printf("%d %d %d %d %d\n", sscanf("", "x%d", &a), sscanf("", "%%"), sscanf("  ", "%%"), sscanf("ab", "abc"), sscanf("abd", "abc"));
    r = sscanf(" %5", "%%%d", &a);
    printf("%d %d %d\n", r, a, sscanf("(5", "(%d)", &a));
    const char *s = "(1)(2)(3)";
    int total = 0, rounds = 0;
    while ((r = sscanf(s, "(%d)%n", &a, &n)) == 1 && rounds < 10) {
        total += a;
        s += n;
        rounds++;
    }
    printf("%d %d %d\n", r, total, rounds);
    void *p = &a;
    r = sscanf("(nil)", "%p", &p);
    printf("%d %d\n", r, p == 0);
    char text[32];
    snprintf(text, sizeof text, "%p", (void *)0);
    p = &a;
    r = sscanf(text, "%p", &p);
    printf("%s %d %d\n", text, r, p == 0);
    p = &a;
    r = sscanf("(nix)", "%p", &p);
    printf("%d %d\n", r, p == (void *)&a);
    float f = 0;
    r = sscanf("nan(123)", "%f", &f);
    printf("%d %08x", r, fbits(f));
    r = sscanf("-nan(0x7fffffff)", "%f", &f);
    printf(" %d %08x\n", r, fbits(f));
    char tag[81];
    memset(tag, '9', 80);
    tag[80] = 0;
    char digits[32];
    strcpy(digits, "999999999999999999999");
    errno = 0;
    double d = nan(digits);
    printf("%016llx %d", dbits(d), errno);
    errno = 0;
    d = strtod("nan(999999999999999999999)", 0);
    printf(" %016llx %d\n", dbits(d), errno);
    printf("%016llx %08x", dbits(nan(tag)), fbits(nanf(tag)));
    memset(tag, 'z', 80);
    printf(" %016llx %016llx\n", dbits(nan(tag)), dbits(nan("1)")));
    return 0;
}
"#;
    let expected = [
        "-1 -1 -1 -1 0",
        "1 5 1",
        "-1 6 3",
        "1 1",
        "(nil) 1 1",
        "0 1",
        "1 7fc0007b 1 ffffffff",
        "7fffffffffffffff 0 7fffffffffffffff 0",
        "7fffffffffffffff 7fffffff 7ff8000000000000 7ff8000000000000",
    ];
    for output in outputs("scanf-review-3", source).into_iter().flatten() {
        assert_eq!(output, expected.join("\n") + "\n");
    }
}

/// On a stream, a hexadecimal float without a significand digit stops
/// after the `0x` or `0x.` it read (the prefix's `0` is no digit), and a
/// scanf loop over parenthesized numbers ends with EOF.
#[test]
fn scanf_on_a_stream_stops_where_glibc_does() {
    let source = r#"#include <stdio.h>
int main(void) {
    double d = -1;
    char rest[16];
    for (int i = 0; i < 3; i++) {
        int r = scanf("%lf", &d);
        int s = scanf("%15s", rest);
        printf("%d %g %d %s\n", r, d, s, s == 1 ? rest : "-");
    }
    int v, total = 0, r;
    while ((r = scanf(" (%d)", &v)) == 1)
        total += v;
    printf("%d %d\n", r, total);
    return 0;
}
"#;
    let expected = ["0 -1 1 p1", "0 -1 1 p1z", "1 1.5 1 x", "-1 6"];
    let input = "0xp1 0x.p1z 1.5 x (1)(2) (3)";
    for output in outputs_with_input("scanf-stream-review-3", source, input)
        .into_iter()
        .flatten()
    {
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

/// `__fp16` on WebAssembly, binary16 storage whose values compute in
/// `float`, converts through the runtime's compiler-rt routines as Clang's
/// code does: to binary16 from `float`, `double` and binary128 with one
/// rounding (ties, subnormals, overflow and NaN payloads), from integers of
/// every width through `float` as Clang's lowering converts them, back
/// exactly, and in single-element records as the bits.
/// The expected output is Clang's on x86-64, which wasi-sdk's Clang and
/// compiler-rt give on wasm32 too.
#[test]
fn fp16_converts_as_clang_does() {
    let source = r#"#include <stdio.h>
#include <string.h>
static unsigned load_bits(const __fp16 *h) { unsigned short u; memcpy(&u, h, 2); return u; }
#define bits(x) ({ __fp16 t_ = (x); load_bits(&t_); })
#define from(u) ({ unsigned short u_ = (u); __fp16 h_; memcpy(&h_, &u_, 2); h_; })
struct one { __fp16 h; };
struct two { __fp16 a, b; };
__attribute__((noinline)) static float take(struct one s) { return s.h; }
__attribute__((noinline)) static struct one make(float x) { struct one s = {x}; return s; }
__attribute__((noinline)) static float sum(struct two t) { return t.a + t.b; }
__fp16 table[4] = {1.5, -2.25, 65504, 0x1p-24};
int main(void) {
    printf("%zu %zu %zu\n", sizeof(__fp16), _Alignof(__fp16), sizeof table);
    volatile double d[] = {1.0, 0.1, 65504.0, 65519.99, 65520.0, 1e10, -1e10, 0x1p-24, 0x1p-25, 0x1.000001p-25,
                           0x1.8p-24, 0x1.ffcp-15, 0x1p-14, 1.0 + 0x1p-11, 1.0 + 0x1.8p-11, -0.0, 3.14159265358979};
    for (int i = 0; i < (int)(sizeof d / sizeof d[0]); i++) {
        __fp16 h = d[i];
        float f = (float)d[i];
        __fp16 g = f;
        long double l = d[i];
        __fp16 k = l;
        printf("%04x %04x %04x %.9g %.17g\n", bits(h), bits(g), bits(k), (double)(float)h, (double)h);
    }
    volatile int iv[] = {0, 1, -1, 2049, 2051, 65504, 65519, 65520, 100000, -2147483647 - 1};
    for (int i = 0; i < (int)(sizeof iv / sizeof iv[0]); i++) {
        __fp16 h = iv[i];
        __fp16 u = (unsigned)iv[i];
        printf("%04x %04x %g\n", bits(h), bits(u), (double)h);
    }
    volatile long long big = 9007199254740993ll;
    volatile unsigned long long ubig = 18446744073709551615ull;
    volatile __int128 huge = (__int128)1 << 100;
    __fp16 a = big, b = ubig, c = huge, e = (__int128)-65519;
    printf("%04x %04x %04x %04x\n", bits(a), bits(b), bits(c), bits(e));
    __fp16 x = from(0x3c01), y = from(0x8001), z = from(0x7bff);
    printf("%lld %u %d %d\n", (long long)z, (unsigned)x, (int)(signed char)from(0xd250), (int)(__int128)z);
    long double lx = x, ly = y;
    printf("%.20Lg %.20Lg\n", lx, ly);
    volatile float nf = __builtin_nanf("0x123456");
    __fp16 n = nf, inf = 1e30f, ninf = -1e30;
    printf("%04x %04x %04x %d\n", bits(n), bits(inf), bits(ninf), (double)from(0x7c00) > 1e300);
    __fp16 p = 1.5, q = 2.25;
    p = p * q + 1;
    printf("%04x %g\n", bits(p), (double)(p / 3));
    printf("%g %g %04x\n", take(make(2.5f)), sum((struct two){0.5, 0.25}), bits(table[3]));
    printf("%f %.3f\n", table[0], table[1]);
    return 0;
}
"#;
    let expected = [
        "2 2 8",
        "3c00 3c00 3c00 1 1",
        "2e66 2e66 2e66 0.0999755859 0.0999755859375",
        "7bff 7bff 7bff 65504 65504",
        "7bff 7bff 7bff 65504 65504",
        "7c00 7c00 7c00 inf inf",
        "7c00 7c00 7c00 inf inf",
        "fc00 fc00 fc00 -inf -inf",
        "0001 0001 0001 5.96046448e-08 5.9604644775390625e-08",
        "0000 0000 0000 0 0",
        "0001 0000 0001 5.96046448e-08 5.9604644775390625e-08",
        "0002 0002 0002 1.1920929e-07 1.1920928955078125e-07",
        "0400 0400 0400 6.10351562e-05 6.103515625e-05",
        "0400 0400 0400 6.10351562e-05 6.103515625e-05",
        "3c00 3c00 3c00 1 1",
        "3c01 3c01 3c01 1.00097656 1.0009765625",
        "8000 8000 8000 -0 -0",
        "4248 4248 4248 3.140625 3.140625",
        "0000 0000 0",
        "3c00 3c00 1",
        "bc00 7c00 -1",
        "6800 6800 2048",
        "6802 6802 2052",
        "7bff 7bff 65504",
        "7bff 7bff 65504",
        "7c00 7c00 inf",
        "7c00 7c00 inf",
        "fc00 7c00 -inf",
        "7c00 7c00 7c00 fbff",
        "65504 1 -50 65504",
        "1.0009765625 -5.9604644775390625e-08",
        "7e91 7c00 fc00 1",
        "4460 1.45833",
        "2.5 0.75 0001",
        "1.500000 -2.250",
    ];
    for output in outputs("fp16", source).into_iter().flatten() {
        assert_eq!(output, expected.join("\n") + "\n");
    }
}

/// A signaling `__fp16` NaN widens as LLVM's code for Clang widens it on
/// wasm32: to `float` through `__extendhfsf2`, which keeps it signaling,
/// to `double` through that and `f64.promote_f32`, which quiets it, and to
/// binary128 through `__extendhftf2`. The expected output is wasi-sdk's
/// Clang with compiler-rt; Clang on x86-64 quiets the `float` too and has
/// x87 `long double`.
#[test]
fn fp16_signaling_nans_widen_as_wasm_clang_does() {
    let source = r#"#include <stdio.h>
#include <string.h>
int main(void) {
    unsigned short raw[] = {0x7d01, 0xfd01, 0x7e01, 0x7c00};
    for (int i = 0; i < 4; i++) {
        __fp16 h;
        memcpy(&h, &raw[i], 2);
        double d = h;
        float f = h;
        long double l = h;
        unsigned long long db, lb[2];
        unsigned fb;
        memcpy(&db, &d, 8);
        memcpy(&fb, &f, 4);
        memcpy(lb, &l, 16);
        printf("%04x %016llx %08x %016llx:%016llx\n", raw[i], db, fb, lb[1], lb[0]);
    }
    return 0;
}
"#;
    let expected = [
        "7d01 7ffc040000000000 7fa02000 7fff404000000000:0000000000000000",
        "fd01 fffc040000000000 ffa02000 ffff404000000000:0000000000000000",
        "7e01 7ff8040000000000 7fc02000 7fff804000000000:0000000000000000",
        "7c00 7ff0000000000000 7f800000 7fff000000000000:0000000000000000",
    ];
    for output in outputs("fp16-signaling", source).into_iter().flatten() {
        assert_eq!(output, expected.join("\n") + "\n");
    }
}

/// `nanl`, which `__builtin_nanl` calls, `strtold` of a NaN and
/// `nextafterl` on binary128 `long double`: a NaN keeps its sign and whole
/// payload in the low bits and `nextafterl` sets `ERANGE` as glibc does
/// where `long double` is binary128, the rules `nan` and `nextafter`
/// follow. Every other bit agrees with wasi-sdk's Clang and wasi-libc,
/// which drops payloads and signs and never sets `ERANGE`.
#[test]
fn nanl_strtold_and_nextafterl_follow_glibc_rules() {
    let source = r#"#include <errno.h>
#include <float.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
static void show(const char *what, long double v) {
    unsigned long long w[2];
    memcpy(w, &v, 16);
    printf("%s %016llx:%016llx\n", what, w[1], w[0]);
}
#define NEXT(x, y)                                                   \
    do {                                                             \
        volatile long double vx = (x), vy = (y);                     \
        errno = 0;                                                   \
        long double v = nextafterl(vx, vy);                          \
        int range = errno == ERANGE;                                 \
        show(#x " " #y, v);                                          \
        printf("  %d\n", range);                                     \
    } while (0)
int main(void) {
    char tag[8];
    strcpy(tag, "123");
    show("nanl", __builtin_nanl(""));
    show("nanl(123)", nanl(tag));
    show("strtold", strtold("nan(0x7b)", 0));
    show("strtold-", strtold("-nan(5)", 0));
    show("strtold1.5", strtold("1.5", 0));
    char *end;
    strtold("  nan(12x", &end);
    printf("end %s\n", end);
    NEXT(1.0L, 2.0L);
    NEXT(1.0L, 0.0L);
    NEXT(-1.0L, 0.0L);
    NEXT(0.0L, -1.0L);
    NEXT(LDBL_MAX, INFINITY);
    NEXT(INFINITY, 0.0L);
    NEXT(LDBL_MIN, 0.0L);
    NEXT(0x1p-16494L, 0.0L);
    printf("%d %d\n", isnan(nextafterl(NAN, 1.0L)), nextafterl(2.0L, 2.0L) == 2.0L);
    return 0;
}
"#;
    let expected = [
        "nanl 7fff800000000000:0000000000000000",
        "nanl(123) 7fff800000000000:000000000000007b",
        "strtold 7fff800000000000:000000000000007b",
        "strtold- ffff800000000000:0000000000000005",
        "strtold1.5 3fff800000000000:0000000000000000",
        "end (12x",
        "1.0L 2.0L 3fff000000000000:0000000000000001",
        "  0",
        "1.0L 0.0L 3ffeffffffffffff:ffffffffffffffff",
        "  0",
        "-1.0L 0.0L bffeffffffffffff:ffffffffffffffff",
        "  0",
        "0.0L -1.0L 8000000000000000:0000000000000001",
        "  0",
        "LDBL_MAX INFINITY 7fff000000000000:0000000000000000",
        "  1",
        "INFINITY 0.0L 7ffeffffffffffff:ffffffffffffffff",
        "  0",
        "LDBL_MIN 0.0L 0000ffffffffffff:ffffffffffffffff",
        "  1",
        "0x1p-16494L 0.0L 0000000000000000:0000000000000000",
        "  1",
        "1 1",
    ];
    for output in outputs("nanl", source).into_iter().flatten() {
        assert_eq!(output, expected.join("\n") + "\n");
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
