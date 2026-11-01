//! Checked C library builtins through the interpreter, the LLVM and native
//! back ends linked with the host C library, and Clang. Destination sizes
//! are written explicitly, as a fortified header computes them; a call that
//! would overrun its destination must end the program before writing.

use std::process::Output;

use super::*;

/// Every checked builtin on valid operands. `va_list` calls pass both
/// System V register areas (seven integers, nine doubles), and a counter
/// shows each operand is evaluated once.
const SUCCESS: &str = r#"
int printf(const char *, ...);
typedef __SIZE_TYPE__ size_t;
static int calls;
static char *once(char *p) { ++calls; return p; }
static int same(const char *a, const char *b) {
    while (*a && *a == *b) { ++a; ++b; }
    return *a == *b;
}
static int vformat(char *b, size_t size, const char *format, ...) {
    __builtin_va_list ap;
    __builtin_va_start(ap, format);
    int n = __builtin___vsprintf_chk(b, 1, size, format, ap);
    __builtin_va_end(ap);
    return n;
}
static int vnformat(char *b, size_t n, size_t size, const char *format, ...) {
    __builtin_va_list ap;
    __builtin_va_start(ap, format);
    int count = __builtin___vsnprintf_chk(b, n, 1, size, format, ap);
    __builtin_va_end(ap);
    return count;
}
static int vprint(const char *format, ...) {
    __builtin_va_list ap;
    __builtin_va_start(ap, format);
    int n = __builtin___vprintf_chk(1, format, ap);
    __builtin_va_end(ap);
    return n;
}
int main(void) {
    char a[128], b[16];
    if (__builtin___memset_chk(once(a), 'x', 8, sizeof a) != a || calls != 1 || a[7] != 'x') return 1;
    if (__builtin___memcpy_chk(once(b), "abc", 4, sizeof b) != b || calls != 2 || !same(b, "abc")) return 2;
    if (__builtin___memmove_chk(b + 1, b, 4, sizeof b - 1) != b + 1 || !same(b, "aabc")) return 3;
    if ((char *)__builtin___mempcpy_chk(a, "hi", 2, sizeof a) != a + 2 || a[1] != 'i') return 4;
    if (__builtin___strcpy_chk(once(b), "hello", sizeof b) != b || calls != 3 || !same(b, "hello")) return 5;
    if (__builtin___stpcpy_chk(a, "ab", sizeof a) != a + 2 || !same(a, "ab")) return 6;
    if (__builtin___strcat_chk(b, " you", sizeof b) != b || !same(b, "hello you")) return 7;
    if (__builtin___strncat_chk(b, "!!!", 1, sizeof b) != b || !same(b, "hello you!")) return 8;
    a[3] = 'q';
    if (__builtin___strncpy_chk(a, "xy", 4, sizeof a) != a || a[2] || a[3] || !same(a, "xy")) return 9;
    if (__builtin___stpncpy_chk(a, "xyz", 2, sizeof a) != a + 2 || a[1] != 'y') return 10;
    if (__builtin___strcpy_chk(b, "unknown", (size_t)-1) != b || !same(b, "unknown")) return 11;
    int n = vformat(once(a), sizeof a, "%d %ld %s %c %x %u %d|%f %f %f %f %f %f %f %f %f",
        1, -2L, "s", 'c', 255, 7u, 8, 0.5, 1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 8.5);
    if (calls != 4 || n != 96 || !same(a, "1 -2 s c ff 7 8|0.500000 1.500000 2.500000 3.500000 "
        "4.500000 5.500000 6.500000 7.500000 8.500000")) return 12;
    if (__builtin___sprintf_chk(once(b), 1, sizeof b, "%s-%d", "ab", 12) != 5 || calls != 5
        || !same(b, "ab-12")) return 13;
    if (__builtin___snprintf_chk(b, 4, 1, sizeof b, "%d", 123456) != 6 || !same(b, "123")) return 14;
    if (vnformat(b, 3, sizeof b, "%s", "abcdef") != 6 || !same(b, "ab")) return 15;
    if (vformat(b, 6, "%s", "abcde") != 5 || !same(b, "abcde")) return 16;
    if (vprint("%d-%s %f\n", 42, "ok", 0.25) != 15) return 17;
    /* Direct forms: the first variadic operand is the first converted, an
       extra one is ignored, and each operand is evaluated once. */
    if (__builtin___sprintf_chk(b, 0, sizeof b, "%d", 11, 22) != 2 || !same(b, "11")) return 18;
    if (__builtin___sprintf_chk(b, 0, sizeof b, "%d,%d", ++calls, 22) != 4 || calls != 6
        || !same(b, "6,22")) return 19;
    if (__builtin___snprintf_chk(b, sizeof b, 0, sizeof b, "%d", 11, 22) != 2 || !same(b, "11"))
        return 20;
    if (__builtin___snprintf_chk(b, 3, 0, sizeof b, "%d,%d", ++calls, 33) != 4 || calls != 7
        || !same(b, "7,")) return 21;
    printf("done %d\n", n);
    return 0;
}
"#;

/// Programs whose one checked call would write past its destination. Each
/// prints only if the call returns.
const OVERFLOWS: [(&str, &str); 9] = [
    ("vsprintf", "vformat(b, sizeof b, \"%d\", 12345)"),
    ("vsnprintf", "vnformat(b, 8, sizeof b, \"%d\", 1)"),
    (
        "sprintf",
        "__builtin___sprintf_chk(b, 1, sizeof b, \"%s\", \"abcd\")",
    ),
    (
        "snprintf",
        "__builtin___snprintf_chk(b, 8, 1, sizeof b, \"%d\", 1)",
    ),
    (
        "memcpy",
        "__builtin___memcpy_chk(b, \"abcdefgh\", 8, sizeof b)",
    ),
    ("memset", "__builtin___memset_chk(b, 0, 5, sizeof b)"),
    ("strcpy", "__builtin___strcpy_chk(b, \"abcd\", sizeof b)"),
    ("strcat", "__builtin___strcat_chk(b, \"cd\", sizeof b)"),
    (
        "strncat",
        "__builtin___strncat_chk(b, \"cdef\", 2, sizeof b)",
    ),
];

fn overflow_program(call: &str) -> String {
    let (prelude, _) = SUCCESS.split_once("int main(void)").unwrap();
    format!(
        "{prelude}int main(void) {{\n    char b[4] = \"ab\";\n    int n = ({call}) != 0;\n    \
         printf(\"returned %d\\n\", n);\n    return 3;\n}}\n"
    )
}

const TARGETS: [Target; 4] = [
    Target::LinuxGnu,
    Target::LinuxMusl,
    Target::WindowsGnu,
    Target::WindowsMsvc,
];

/// Runs `main`, returning how it ended and its output.
fn run(module: &Module<'_>, target: Target) -> (Termination, String) {
    let arena = Bump::new();
    let mut host = DefaultHost::for_target(&arena, target);
    let outcome = interpreter::run_main(module, &mut host)
        .unwrap_or_else(|trap| panic!("{}\n{module}", trap.display_in(module)));
    let output = String::from_utf8(host.output().to_vec()).unwrap();
    (outcome.termination, output)
}

#[test]
fn checked_builtins_lower_to_typed_library_calls() {
    for target in TARGETS {
        with_module(SUCCESS, target, false, Profile::PreAbi, |module, _| {
            // Parameters keep the C library's types; `noundef` is incidental.
            let text = module.to_string().replace(" noundef", "");
            for (name, signature) in [
                ("__vsprintf_chk", "(ptr, i32, i64, ptr, ptr) -> i32"),
                ("__vsnprintf_chk", "(ptr, i64, i32, i64, ptr, ptr) -> i32"),
                ("__vprintf_chk", "(i32, ptr, ptr) -> i32"),
                ("__sprintf_chk", "(ptr, i32, i64, ptr, ...) -> i32"),
                ("__snprintf_chk", "(ptr, i64, i32, i64, ptr, ...) -> i32"),
                ("__memcpy_chk", "(ptr, ptr, i64, i64) -> ptr"),
                ("__memset_chk", "(ptr, i32, i64, i64) -> ptr"),
                ("__strcpy_chk", "(ptr, ptr, i64) -> ptr"),
                ("__stpncpy_chk", "(ptr, ptr, i64, i64) -> ptr"),
            ] {
                assert!(
                    text.contains(&format!("function @{name}{signature} external")),
                    "{target:?} {name}:\n{text}"
                );
                assert!(text.contains(&format!("call @{name}(")), "{name}");
            }
            assert!(!text.contains("__builtin___"), "{text}");
        });
    }
}

#[test]
fn checked_builtins_execute_and_stop_overflows_in_the_interpreter() {
    for target in TARGETS {
        for (optimize, profile) in STAGES {
            let stage = format!("{target:?} {optimize} {profile:?}");
            let (termination, output) = with_module(SUCCESS, target, optimize, profile, run);
            assert!(
                matches!(
                    termination,
                    Termination::Returned(Some(RuntimeValue::Int(0)))
                ),
                "{stage}: {termination:?}"
            );
            assert_eq!(output, "42-ok 0.250000\ndone 96\n", "{stage}");
            for (name, call) in OVERFLOWS {
                let source = overflow_program(call);
                let (termination, output) = with_module(&source, target, optimize, profile, run);
                assert!(
                    matches!(termination, Termination::Aborted),
                    "{stage} {name}: {termination:?}"
                );
                assert_eq!(output, "", "{stage} {name}");
            }
        }
    }
}

/// How a native run ended: its exit status, or the signal that killed it.
fn ending(output: &Output) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        if let Some(signal) = output.status.signal() {
            return format!("signal {signal}");
        }
    }
    format!("status {}", output.status.code().unwrap())
}

/// The host's C library must provide the `__*_chk` entry points; glibc does,
/// musl and the Windows C runtimes do not.
#[test]
fn checked_builtins_match_clang_with_the_host_c_library() {
    let Some(target @ Target::LinuxGnu) = host() else {
        eprintln!("skipping: the host C library is not glibc");
        return;
    };
    let toolchain = match Toolchain::bundled(target) {
        | Ok(toolchain) => toolchain,
        | Err(error) => {
            eprintln!("skipping: {error}");
            return;
        },
    };
    let directory = TempDir::new("checked-builtins");
    let mut programs = vec![("success".to_string(), SUCCESS.to_string())];
    for (name, call) in OVERFLOWS {
        programs.push((name.to_string(), overflow_program(call)));
    }
    for (name, source) in &programs {
        let input = directory.join(format!("{name}.c"));
        std::fs::write(&input, source).unwrap();
        let options = |opt| LlvmOptions {
            opt: Some(opt),
            disable_llvm_passes: false,
            target,
            kind: OutputKind::Executable,
            code: crate::target::CodeOptions::default(),
            fast_math_runtime: false,
        };
        let mut endings = Vec::new();
        for opt in [OptLevel::O0, OptLevel::O2] {
            let executable = directory.join(format!("{name}-clang{opt:?}.exe"));
            let output = toolchain
                .command(&input, &executable, options(opt))
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            endings.push((format!("clang {opt:?}"), executable));
        }
        for arm in ARMS {
            let executable = directory.join(format!("{name}-{}.exe", arm.name));
            with_module(source, target, arm.bcc, Profile::PostAbi, |module, _| {
                compile_module(
                    module,
                    &executable,
                    LlvmOptions {
                        disable_llvm_passes: arm.disable_llvm_passes,
                        ..options(arm.opt)
                    },
                )
                .unwrap();
            });
            endings.push((arm.name.to_string(), executable));
        }
        for optimize in [false, true] {
            let object = directory.join(format!("{name}-native-{optimize}.o"));
            let executable = directory.join(format!("{name}-native-{optimize}.exe"));
            with_module(source, target, optimize, Profile::PostAbi, |module, _| {
                let mut assembly = String::new();
                crate::backend::native::emit_assembly(module, target, &mut assembly).unwrap();
                crate::backend::native::assemble(&assembly, &object, target, OutputKind::Object)
                    .unwrap();
            });
            let link = toolchain
                .command(&object, &executable, options(OptLevel::O0))
                .output()
                .unwrap();
            assert!(link.status.success(), "{link:?}");
            endings.push((format!("native {optimize}"), executable));
        }
        let mut expected = None;
        for (arm, executable) in endings {
            let output =
                crate::test_support::native::run_native(&mut Command::new(&executable)).unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            let observed = (
                ending(&output),
                String::from_utf8_lossy(&output.stdout).into_owned(),
            );
            if name == "success" {
                assert_eq!(
                    observed,
                    (
                        "status 0".to_string(),
                        "42-ok 0.250000\ndone 96\n".to_string()
                    ),
                    "{arm}: {stderr}"
                );
            } else {
                // glibc's __chk_fail reports the overflow and aborts.
                assert_eq!(
                    observed,
                    ("signal 6".to_string(), String::new()),
                    "{name} {arm}: {stderr}"
                );
                assert!(
                    stderr.contains("*** buffer overflow detected ***"),
                    "{name} {arm}: {stderr}"
                );
            }
            assert_eq!(
                expected.get_or_insert_with(|| observed.clone()),
                &observed,
                "{name} {arm}"
            );
        }
    }
}

/// The genuine glibc declarations: `FILE`, `stdout`, `va_list` and `size_t`
/// from the installed headers, without `_FORTIFY_SOURCE`, whose wrappers
/// need attributes this change does not add. The stream call writes to
/// standard output, which only native runs model.
const GLIBC: &str = r#"
#include <stdarg.h>
#include <stdio.h>
#include <string.h>
static int report(FILE *stream, const char *format, ...) {
    va_list ap;
    va_start(ap, format);
    int n = __builtin___vfprintf_chk(stream, 1, format, ap);
    va_end(ap);
    return n;
}
static int format(char *buffer, size_t size, const char *format, ...) {
    va_list ap;
    va_start(ap, format);
    int n = __builtin___vsprintf_chk(buffer, 1, size, format, ap);
    va_end(ap);
    return n;
}
int main(void) {
    char buffer[16];
    if (format(buffer, sizeof buffer, "%s=%d", "x", 7) != 3 || strcmp(buffer, "x=7")) return 1;
    if (report(stdout, "[%s] %ld\n", buffer, 12L) != 9) return 2;
    if (__builtin___memcpy_chk(buffer, "0123456789", 11, __builtin_object_size(buffer, 0)) != buffer)
        return 3;
    fflush(stdout);
    return format(buffer, sizeof buffer, "%s%s", "0123456789", buffer) == 20;
}
"#;

/// Linked with the host's glibc, every arm prints, then aborts on the last
/// call's 21-byte write into 16 bytes, as Clang's build does.
#[test]
fn checked_builtins_accept_glibc_headers_and_match_clang() {
    let Some(target @ Target::LinuxGnu) = host() else {
        eprintln!("skipping: the host C library is not glibc");
        return;
    };
    let includes = [
        Path::new("/usr/include/x86_64-linux-gnu"),
        Path::new("/usr/include"),
    ];
    if !includes[1].join("stdio.h").is_file() {
        eprintln!("skipping: no host stdio.h");
        return;
    }
    let toolchain = match Toolchain::bundled(target) {
        | Ok(toolchain) => toolchain,
        | Err(error) => {
            eprintln!("skipping: {error}");
            return;
        },
    };
    let headers = HeaderSearch {
        after: &includes,
        ..HeaderSearch::default()
    };
    let configuration = CompilerConfiguration::default()
        .with_target(target)
        .with_gnu_extensions(true);
    let directory = TempDir::new("checked-builtins-glibc");
    let input = directory.join("glibc.c");
    std::fs::write(&input, GLIBC).unwrap();
    let options = |opt| LlvmOptions {
        opt: Some(opt),
        disable_llvm_passes: false,
        target,
        kind: OutputKind::Executable,
        code: crate::target::CodeOptions::default(),
        fast_math_runtime: false,
    };
    let mut executables = Vec::new();
    for opt in [OptLevel::O0, OptLevel::O2] {
        let executable = directory.join(format!("clang{opt:?}.exe"));
        let output = toolchain
            .command(&input, &executable, options(opt))
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        executables.push((format!("clang {opt:?}"), executable));
    }
    for arm in ARMS {
        let executable = directory.join(format!("{}.exe", arm.name));
        with_configured_headers(
            GLIBC,
            target,
            arm.bcc,
            Profile::PostAbi,
            configuration,
            headers,
            |module, _| {
                compile_module(
                    module,
                    &executable,
                    LlvmOptions {
                        disable_llvm_passes: arm.disable_llvm_passes,
                        ..options(arm.opt)
                    },
                )
                .unwrap();
            },
        );
        executables.push((arm.name.to_string(), executable));
    }
    for (arm, executable) in executables {
        let output =
            crate::test_support::native::run_native(&mut Command::new(&executable)).unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            (
                ending(&output),
                String::from_utf8_lossy(&output.stdout).into_owned()
            ),
            ("signal 6".to_string(), "[x=7] 12\n".to_string()),
            "{arm}: {stderr}"
        );
        assert!(
            stderr.contains("*** buffer overflow detected ***"),
            "{arm}: {stderr}"
        );
    }
}
