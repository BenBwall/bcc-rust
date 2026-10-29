//! bcc's objects linked with `cl`'s in both directions: one translation
//! unit defines functions taking and returning structures by value,
//! variadic functions, `__int64` arithmetic and callbacks, and the other
//! calls them. `__vectorcall` is not covered: bcc refuses it.

use std::fs;

use super::toolchain::{
    Compiler,
    Differences,
    build,
    msvc,
    outcome,
    scratch,
};

const API: &str = r"
#include <stdarg.h>
#include <stddef.h>
typedef struct { char c; } S1;
typedef struct { char a, b, c; } S3;
typedef struct { int a, b; } S8;
typedef struct { double x; } SD;
typedef struct { float x, y; } SF2;
typedef struct { long long a, b; } S16;
typedef struct { int v[5]; } S20;
S1 make_s1(char);
int take_s1(S1);
S3 make_s3(char, char, char);
int take_s3(S3);
S8 make_s8(int, int);
int take_s8(S8);
SD make_sd(double);
double take_sd(SD);
SF2 make_sf2(float, float);
float take_sf2(SF2);
S16 make_s16(long long, long long);
long long take_s16(S16);
S20 make_s20(int);
int take_s20(S20);
double many(int, double, int, float, long long, double, char, S16, S8, S3, float);
int vsum(int count, ...);
double vmix(const char *format, ...);
int vformat(char *buffer, size_t size, const char *format, va_list list);
int format(char *buffer, size_t size, const char *format, ...);
__int64 mul64(__int64, __int64);
unsigned __int64 udiv64(unsigned __int64, unsigned __int64);
typedef int (*callback)(S8, int, double);
int apply(callback, S8, int);
int compare_ints(const void *, const void *);
_Bool is_odd(int);
signed char narrow(int);
unsigned short widen(unsigned char);
float halve(float);
long double third(long double);
";

const LIBRARY: &str = r#"
#include "api.h"
#include <stdio.h>
S1 make_s1(char c) { S1 s = { c }; return s; }
int take_s1(S1 s) { return s.c * 2; }
S3 make_s3(char a, char b, char c) { S3 s = { a, b, c }; return s; }
int take_s3(S3 s) { return s.a * 100 + s.b * 10 + s.c; }
S8 make_s8(int a, int b) { S8 s = { a, b }; return s; }
int take_s8(S8 s) { return s.a - s.b; }
SD make_sd(double x) { SD s = { x * 2 }; return s; }
double take_sd(SD s) { return s.x + 0.5; }
SF2 make_sf2(float x, float y) { SF2 s = { x, y }; return s; }
float take_sf2(SF2 s) { return s.x * s.y; }
S16 make_s16(long long a, long long b) { S16 s = { a, b }; return s; }
long long take_s16(S16 s) { return s.a * 3 + s.b; }
S20 make_s20(int base) { S20 s; for (int i = 0; i < 5; i++) s.v[i] = base + i; return s; }
int take_s20(S20 s) { int t = 0; for (int i = 0; i < 5; i++) t += s.v[i] * (i + 1); return t; }
double many(int a, double b, int c, float d, long long e, double f, char g, S16 h, S8 i,
            S3 j, float k) {
    return a + b + c + d + (double)e + f + g + (double)(h.a + h.b) + i.a + i.b + j.c + k;
}
int vsum(int count, ...) {
    va_list list;
    va_start(list, count);
    int total = 0;
    for (int i = 0; i < count; i++) total += va_arg(list, int);
    va_end(list);
    return total;
}
double vmix(const char *format, ...) {
    va_list list;
    va_start(list, format);
    double total = 0;
    for (; *format; format++) {
        switch (*format) {
        case 'i': total += va_arg(list, int); break;
        case 'd': total += va_arg(list, double); break;
        case 'l': total += (double)va_arg(list, long long); break;
        case 'p': total += *va_arg(list, int *); break;
        case 's': { S8 s = va_arg(list, S8); total += s.a * 10 + s.b; break; }
        case 'S': { S16 s = va_arg(list, S16); total += (double)(s.a - s.b); break; }
        }
    }
    va_end(list);
    return total;
}
int vformat(char *buffer, size_t size, const char *format, va_list list) {
    return vsnprintf(buffer, size, format, list);
}
int format(char *buffer, size_t size, const char *format, ...) {
    va_list list;
    va_start(list, format);
    int written = vformat(buffer, size, format, list);
    va_end(list);
    return written;
}
__int64 mul64(__int64 a, __int64 b) { return a * b; }
unsigned __int64 udiv64(unsigned __int64 a, unsigned __int64 b) { return a / b; }
int apply(callback f, S8 s, int k) { return f(s, k, 0.25) + f(make_s8(1, 2), -k, 4.0); }
int compare_ints(const void *a, const void *b) {
    int x = *(const int *)a, y = *(const int *)b;
    return (x > y) - (x < y);
}
_Bool is_odd(int n) { return n & 1; }
signed char narrow(int n) { return (signed char)n; }
unsigned short widen(unsigned char c) { return c * 300u; }
float halve(float x) { return x / 2; }
long double third(long double x) { return x / 3; }
"#;

const CALLER: &str = r#"
#include "api.h"
#include <stdio.h>
#include <stdlib.h>
static int combine(S8 s, int k, double scale) { return (int)((s.a * k + s.b) * scale * 4); }
int main(void) {
    printf("s1 %d %d\n", make_s1(9).c, take_s1(make_s1(-4)));
    S3 s3 = make_s3(1, 2, 3);
    printf("s3 %d %d %d %d\n", s3.a, s3.b, s3.c, take_s3(s3));
    S8 s8 = make_s8(40, 2);
    printf("s8 %d %d %d\n", s8.a, s8.b, take_s8(s8));
    printf("sd %.2f %.2f\n", make_sd(1.25).x, take_sd(make_sd(2)));
    SF2 f2 = make_sf2(1.5f, -2.0f);
    printf("sf2 %.2f %.2f %.2f\n", f2.x, f2.y, take_sf2(f2));
    S16 s16 = make_s16(1LL << 40, -5);
    printf("s16 %lld %lld %lld\n", s16.a, s16.b, take_s16(s16));
    S20 s20 = make_s20(10);
    printf("s20 %d %d %d\n", s20.v[0], s20.v[4], take_s20(s20));
    printf("many %.3f\n", many(1, 2.5, 3, 4.25f, 5LL << 33, 6.125, 7, s16, s8, s3, 0.5f));
    printf("vsum %d\n", vsum(5, 1, 2, 3, 4, 5));
    int target = 17;
    printf("vmix %.3f\n", vmix("idlpsS", 3, 0.5, 1LL << 35, &target, s8, s16));
    char buffer[64];
    int written = format(buffer, sizeof buffer, "%s|%5.2f|%lld|%c", "fmt", 3.14159,
                         -(1LL << 50), 'z');
    printf("format %d %s\n", written, buffer);
    printf("int64 %lld %llu\n", mul64(-3000000000LL, 7), udiv64(~0ULL, 3));
    printf("apply %d\n", apply(combine, s8, 3));
    int values[] = { 5, -1, 9, 0, 3 };
    qsort(values, 5, sizeof *values, compare_ints);
    printf("qsort %d %d %d %d %d\n", values[0], values[1], values[2], values[3], values[4]);
    printf("small %d %d %d %u\n", is_odd(7), is_odd(8), narrow(200), widen(200));
    printf("float %.3f %.6Lf\n", halve(5.0f), third(1.0L));
    return take_s8(s8) & 0xff;
}
"#;

/// Every pairing of compilers for the two units, linked by `cl` (and so by
/// `link.exe`), prints what `cl`'s own build prints.
#[test]
fn objects_link_with_cl_objects_in_both_directions() {
    let Some(msvc) = msvc() else {
        eprintln!("skipping: no Visual Studio C compiler on this host");
        return;
    };
    let root = scratch("interop");
    fs::write(root.join("api.h"), API).unwrap();
    fs::write(root.join("library.c"), LIBRARY).unwrap();
    fs::write(root.join("caller.c"), CALLER).unwrap();
    let mut differences = Differences::default();
    for compiler in Compiler::ALL {
        for unit in ["library", "caller"] {
            if let Err(error) = build(
                msvc.compiler(compiler).args([
                    "/nologo",
                    "/c",
                    "/MT",
                    &format!("{unit}.c"),
                    &format!("/Fo{unit}-{}.obj", compiler.label()),
                ]),
                &root,
            ) {
                assert!(compiler != Compiler::Cl, "{error}");
                differences.push(error);
            }
        }
    }
    let mut expected = None;
    for library in Compiler::ALL {
        for caller in Compiler::ALL {
            let label = format!(
                "library by {}, caller by {}",
                library.label(),
                caller.label()
            );
            let executable = format!("{}-{}.exe", library.label(), caller.label());
            // `cl` links bcc's objects, which name no default libraries,
            // through the libraries its own object names; two bcc objects
            // link with bcc.
            let linker = if library == Compiler::Cl || caller == Compiler::Cl {
                Compiler::Cl
            } else {
                Compiler::Bcc
            };
            let result = build(
                msvc.compiler(linker).args([
                    "/nologo",
                    "/MT",
                    &format!("caller-{}.obj", caller.label()),
                    &format!("library-{}.obj", library.label()),
                    &format!("/Fe{executable}"),
                ]),
                &root,
            )
            .and_then(|()| outcome(&root.join(&executable)));
            match (library, caller, result) {
                | (Compiler::Cl, Compiler::Cl, Ok(reference)) => expected = Some(reference),
                | (Compiler::Cl, Compiler::Cl, Err(error)) =>
                    panic!("cl cannot build {label}: {error}"),
                | (.., Ok(actual)) =>
                    differences.compare(&label, expected.as_ref().unwrap(), &actual),
                | (.., Err(error)) => differences.push(format!("{label}: {error}")),
            }
        }
    }
    drop(fs::remove_dir_all(root));
    differences.assert_none("ABI");
}

/// Two units that both leave `int shared;` a tentative definition, built
/// by each pair of compilers and linked by `cl` (or bcc for two bcc
/// objects): whether the link succeeds tells whether each compiler makes
/// the definition a communal symbol, as `-fcommon` does.
#[test]
fn tentative_definitions_link_as_cl_links_them() {
    let Some(msvc) = msvc() else {
        eprintln!("skipping: no Visual Studio C compiler on this host");
        return;
    };
    let root = scratch("common");
    fs::write(
        root.join("left.c"),
        "int shared;\nint get(void) { return shared; }\n",
    )
    .unwrap();
    fs::write(
        root.join("right.c"),
        "int shared;\nint get(void);\nint main(void) { shared = 4; return get(); }\n",
    )
    .unwrap();
    let mut differences = Differences::default();
    let mut expected = None;
    for left in [Compiler::Cl, Compiler::Bcc] {
        for right in [Compiler::Cl, Compiler::Bcc] {
            for (unit, compiler) in [("left", left), ("right", right)] {
                build(
                    msvc.compiler(compiler).args([
                        "/nologo",
                        "/c",
                        &format!("{unit}.c"),
                        &format!("/Fo{unit}-{}.obj", compiler.label()),
                    ]),
                    &root,
                )
                .unwrap();
            }
            let label = format!("left by {}, right by {}", left.label(), right.label());
            let executable = format!("common-{}-{}.exe", left.label(), right.label());
            let linker = if left == Compiler::Cl || right == Compiler::Cl {
                Compiler::Cl
            } else {
                Compiler::Bcc
            };
            let result = build(
                msvc.compiler(linker).args([
                    "/nologo",
                    &format!("left-{}.obj", left.label()),
                    &format!("right-{}.obj", right.label()),
                    &format!("/Fe{executable}"),
                ]),
                &root,
            )
            .and_then(|()| outcome(&root.join(&executable)));
            println!("{label}: {result:?}");
            match (left, right, result) {
                | (Compiler::Cl, Compiler::Cl, result) => expected = Some(result.is_ok()),
                | (.., result) => differences.compare(&label, &expected, &Some(result.is_ok())),
            }
        }
    }
    drop(fs::remove_dir_all(root));
    differences.assert_none("tentative definition");
}
