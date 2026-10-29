//! Programs linked against the Microsoft C runtime: each is built by
//! `cl`, by `bcc --msvc-mode` and by bcc's native back end, with the DLL
//! runtime (`/MD`), the static one (`/MT`) and the debug DLL one (`/MDd`),
//! and must print and return what `cl`'s build does.

use std::{
    fs,
    path::Path,
};

use super::toolchain::{
    Compiler,
    Differences,
    build,
    msvc,
    outcome,
    scratch,
};

/// `printf` (whose UCRT inline definition starts its list with
/// `__va_start`), `malloc`, file input and output, `qsort` with a callback,
/// `atexit`, and `long double`, which is `double` under MSVC.
const STDIO: &str = r#"
#include <float.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int compare(const void *a, const void *b) {
    return *(const int *)a - *(const int *)b;
}

static int forward(char *buffer, size_t size, const char *format, ...) {
    va_list list;
    va_start(list, format);
    int written = vsnprintf(buffer, size, format, list);
    va_end(list);
    return written;
}

static void goodbye(void) { puts("atexit"); }

int main(void) {
    printf("%d %u %ld %lld %I64d %zu %x %o %c %s %%\n", -7, 7u, -70L, -700LL,
           (__int64)1 << 40, sizeof(void *), 255, 8, 'q', "text");
    printf("%e %f %g %.3f %a\n", 1.5e-3, 3.25, 1e20, 2.0 / 3.0, 1.0);
    long double wide = 1.0L / 3.0L;
    printf("long double %zu %d %d %.17Lg\n", sizeof(long double), LDBL_MANT_DIG,
           LDBL_DIG, wide);
    printf("double %s\n", sizeof(long double) == sizeof(double) ? "same" : "differs");

    int *values = malloc(5 * sizeof *values);
    for (int i = 0; i < 5; i++) values[i] = (i * 7) % 5;
    values = realloc(values, 10 * sizeof *values);
    for (int i = 5; i < 10; i++) values[i] = 10 - i;
    qsort(values, 10, sizeof *values, compare);
    for (int i = 0; i < 10; i++) printf("%d%c", values[i], i == 9 ? '\n' : ' ');
    free(values);
    int *zeroed = calloc(4, sizeof *zeroed);
    printf("calloc %d\n", zeroed[0] + zeroed[3]);
    free(zeroed);

    char buffer[64];
    printf("forward %d [%s]\n", forward(buffer, sizeof buffer, "%s-%d-%.1f", "x", 5, 2.5),
           buffer);
    printf("snprintf %d\n", snprintf(buffer, 4, "%d", 123456));

    FILE *file = fopen("crt-file.txt", "w+b");
    if (!file) return 10;
    fprintf(file, "line one\nline two %d\n", 2);
    fwrite("raw", 1, 3, file);
    long size = ftell(file);
    rewind(file);
    char line[32];
    fgets(line, sizeof line, file);
    printf("size %ld first %s", size, line);
    fseek(file, -3, SEEK_END);
    size_t got = fread(line, 1, sizeof line, file);
    printf("tail %zu %.3s eof %d\n", got, line, feof(file) != 0);
    fclose(file);
    printf("remove %d\n", remove("crt-file.txt"));

    printf("strtod %.2f strtol %ld\n", strtod("12.5e1", NULL), strtol("-0x1F", NULL, 16));
    atexit(goodbye);
    return 3;
}
"#;

/// `setjmp` and `longjmp` through several frames, a `longjmp` of 0, and
/// a `volatile` local changed between them.
const SETJMP: &str = r#"
#include <setjmp.h>
#include <stdio.h>

static jmp_buf env;
static int depth;

static void dive(int n) {
    volatile char pad[64];
    pad[0] = (char)n;
    if (n == 0) longjmp(env, 42);
    dive(n - 1);
    depth += pad[0];
}

int main(void) {
    volatile int stage = 0;
    int result = setjmp(env);
    if (result == 0) {
        stage = 1;
        dive(5);
        puts("not reached");
    }
    printf("%d %d %d\n", result, stage, depth);
    if (setjmp(env) == 0)
        longjmp(env, 0);
    else
        puts("zero became one");
    return 0;
}
"#;

/// Code that raises no structured exception: integer and floating
/// arithmetic, wide characters, environment and `exit` from a nested call.
const PLAIN: &str = r#"
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <wchar.h>

static void leave(int status) { fflush(stdout); exit(status); }

int main(void) {
    volatile int a = 7, b = -2;
    printf("%d %d %d %d\n", a / b, a % b, a << 3, b >> 1);
    volatile double x = 2.0;
    printf("%.6f %.6f %.6f\n", sqrt(x), pow(x, 0.5), floor(-x / 3));
    wchar_t wide[16];
    swprintf(wide, 16, L"%ls-%d", L"w", 9);
    printf("%zu %d\n", wcslen(wide), wcscmp(wide, L"w-9"));
    printf("%s\n", getenv("BCC_MSVC_NOT_SET") ? "set" : "unset");
    char text[] = "a,b,,c";
    int fields = 0;
    for (char *t = strtok(text, ","); t; t = strtok(NULL, ",")) fields++;
    printf("fields %d\n", fields);
    leave(5);
}
"#;

/// The runtime options compared, as `cl` spells them.
const RUNTIMES: [&str; 3] = ["/MD", "/MT", "/MDd"];

/// Every program, built every way, prints and returns what `cl`'s does.
#[test]
fn programs_match_cl_with_each_runtime() {
    let Some(msvc) = msvc() else {
        eprintln!("skipping: no Visual Studio C compiler on this host");
        return;
    };
    let root = scratch("crt");
    let mut differences = Differences::default();
    for (name, source) in [("stdio", STDIO), ("setjmp", SETJMP), ("plain", PLAIN)] {
        for runtime in RUNTIMES {
            let mut expected = None;
            for compiler in Compiler::ALL {
                let directory = root.join(format!("{name}-{}-{}", &runtime[1..], compiler.label()));
                fs::create_dir_all(&directory).unwrap();
                fs::write(directory.join(format!("{name}.c")), source).unwrap();
                let label = format!("{name} {runtime} {}", compiler.label());
                let result = build(
                    msvc.compiler(compiler)
                        .args(["/nologo", runtime])
                        .arg(format!("{name}.c")),
                    &directory,
                )
                .and_then(|()| outcome(&directory.join(format!("{name}.exe"))));
                match (compiler, result) {
                    | (Compiler::Cl, Ok(reference)) => expected = Some(reference),
                    | (Compiler::Cl, Err(error)) => panic!("cl cannot build {label}: {error}"),
                    | (_, Ok(actual)) =>
                        differences.compare(&label, expected.as_ref().unwrap(), &actual),
                    | (_, Err(error)) => differences.push(format!("{label}: {error}")),
                }
            }
        }
    }
    drop(fs::remove_dir_all(root));
    differences.assert_none("C runtime");
}

/// A DLL exporting functions and data with `__declspec(dllexport)`, and a
/// client importing them with `__declspec(dllimport)` through the DLL's
/// import library, built by every pair of compilers.
#[test]
fn dlls_and_import_libraries_link_across_compilers() {
    let Some(msvc) = msvc() else {
        eprintln!("skipping: no Visual Studio C compiler on this host");
        return;
    };
    let root = scratch("dll");
    let mut differences = Differences::default();
    let mut expected = None;
    for library in Compiler::ALL {
        for client in Compiler::ALL {
            let directory = root.join(format!("{}-{}", library.label(), client.label()));
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("boundary.c"), EXPORTER).unwrap();
            fs::write(directory.join("client.c"), IMPORTER).unwrap();
            let label = format!("DLL by {}, client by {}", library.label(), client.label());
            let result = build(
                msvc.compiler(library)
                    .args(["/nologo", "/LD", "/MD", "boundary.c"]),
                &directory,
            )
            .and_then(|()| expect_file(&directory, "boundary.dll"))
            .and_then(|()| expect_file(&directory, "boundary.lib"))
            .and_then(|()| {
                build(
                    msvc.compiler(client)
                        .args(["/nologo", "/MD", "client.c", "boundary.lib"]),
                    &directory,
                )
            })
            .and_then(|()| outcome(&directory.join("client.exe")));
            match (library, client, result) {
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
    differences.assert_none("DLL");
}

const EXPORTER: &str = r#"
#include <stdio.h>
struct Pair { int first, second; };
__declspec(dllexport) int counter = 5;
__declspec(dllexport) struct Pair pair = { 3, 4 };
__declspec(dllexport) int bump(int by) { counter += by; return counter; }
__declspec(dllexport) const char *name(void) { return "boundary"; }
__declspec(dllexport) int report(const char *what) { return printf("dll says %s\n", what); }
"#;

const IMPORTER: &str = r#"
#include <stdio.h>
struct Pair { int first, second; };
__declspec(dllimport) extern int counter;
__declspec(dllimport) extern struct Pair pair;
__declspec(dllimport) int bump(int);
__declspec(dllimport) const char *name(void);
__declspec(dllimport) int report(const char *);
int main(void) {
    int (*indirect)(int) = bump;
    int first = indirect(2);
    printf("%s %d %d %d\n", name(), first, counter, pair.first + pair.second);
    counter = 40;
    pair.second = 10;
    printf("%d %d\n", bump(2), pair.first + pair.second);
    fflush(stdout);
    report("hi");
    return first;
}
"#;

fn expect_file(directory: &Path, name: &str) -> Result<(), String> {
    if directory.join(name).is_file() {
        Ok(())
    } else {
        Err(format!("no {name} in {}", directory.display()))
    }
}
