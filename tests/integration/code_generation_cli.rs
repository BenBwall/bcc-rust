#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Test harnesses own process output and file paths outside compilation."
)]
mod tests {
    use std::{
        env,
        path::{
            Path,
            PathBuf,
        },
        process::{
            Command,
            Output,
        },
        sync::atomic::{
            AtomicU32,
            Ordering,
        },
    };

    use crate::{
        native::{
            run_native,
            run_native_with_timeout,
        },
        native_runner::run_until_timeout,
    };

    const SUM: &str = "int main(void) {\n  int s = 0;\n  for (int i = 1; i <= 4; i++)\n    s += \
                       i;\n  return s;\n}\n";

    /// Real glibc fortify wrappers must keep libc addresses in syscall tables.
    #[test]
    fn glibc_fortify_wrapper_addresses_match_clang_and_run() {
        if host() != Some("x86_64-unknown-linux-gnu")
            || !Path::new("/usr/include/unistd.h").is_file()
        {
            eprintln!("skipping glibc fortify test: no Linux GNU host headers");
            return;
        }
        let source = r"
#include <unistd.h>
#include <string.h>
#if __USE_FORTIFY_LEVEL != 2
#error fortify wrappers must be enabled
#endif
char *(*cat_value)(char *, const char *) = strcat;
char *(*cat_indirect)(char *, const char *) = *strcat;
char *(*cat_twice)(char *, const char *) = **strcat;
char *(*cat_conditional)(char *, const char *) = 1 ? strcat : strcpy;
char *(*cwd_value)(char *, size_t) = getcwd;
char *(*cwd_address)(char *, size_t) = &getcwd;
ssize_t (*read_value)(int, void *, size_t) = read;
ssize_t (*read_address)(int, void *, size_t) = &read;
void (*syscalls[])(void) = { (void (*)(void))getcwd, (void (*)(void))&read };
int main(void) {
    static char *(*cwd_static)(char *, size_t) = getcwd;
    char *(*cwd_local)(char *, size_t) = &getcwd;
    ssize_t (*read_local)(int, void *, size_t) = read;
    char buf[4096];
    if (cat_value != strcat || cat_indirect != strcat
        || cat_twice != strcat || cat_conditional != strcat) return 3;
    if (cwd_value != cwd_address || cwd_local != cwd_static || cwd_local != getcwd
        || read_value != read_address || read_local != read
        || syscalls[0] != (void (*)(void))getcwd || syscalls[1] != (void (*)(void))read)
        return 1;
    if (!cwd_local(buf, sizeof buf) || !getcwd(buf, sizeof buf)) return 2;
    return read_local(-1, buf, 0) != -1 || read(-1, buf, 0) != -1;
}
";
        let flags = [
            "--target=x86_64-unknown-linux-gnu",
            "-D_FORTIFY_SOURCE=2",
            "-D__OPTIMIZE__",
            "--idirafter",
            "/usr/include/x86_64-linux-gnu",
            "--idirafter",
            "/usr/include",
        ];
        let input = temporary("fortify.c");
        std::fs::write(&input, source).unwrap();
        let reference = Command::new("target/llvm/bin/clang")
            .arg(&input)
            .args([
                "-D_FORTIFY_SOURCE=2",
                "-D__OPTIMIZE__",
                "-S",
                "-emit-llvm",
                "-o",
                "-",
            ])
            .output()
            .unwrap();
        assert!(reference.status.success(), "{}", text(&reference.stderr));
        let output = bcc(source, &[&flags[..], &["--emit=llvm"]].concat());
        assert!(output.status.success(), "{}", text(&output.stderr));
        for llvm in [text(&reference.stdout), text(&output.stdout)] {
            for (global, symbol) in [
                ("cat_value", "strcat"),
                ("cat_indirect", "strcat"),
                ("cat_twice", "strcat"),
                ("cat_conditional", "strcat"),
                ("cwd_value", "getcwd"),
                ("cwd_address", "getcwd"),
                ("read_value", "read"),
                ("read_address", "read"),
            ] {
                let line = llvm
                    .lines()
                    .find(|line| line.starts_with(&format!("@{global} =")))
                    .unwrap();
                assert!(line.contains(&format!("ptr @{symbol}")), "{line}");
            }
            let table = llvm
                .lines()
                .find(|line| line.starts_with("@syscalls ="))
                .unwrap();
            assert!(
                table.contains("ptr @getcwd") && table.contains("ptr @read"),
                "{table}"
            );
        }
        let executable = temporary("fortify.exe");
        for options in [
            &["--opt=none", "--llvm-opt=O0"][..],
            &["--opt=bcc", "--llvm-opt=O2"][..],
        ] {
            let built = bcc(
                source,
                &[&flags[..], options, &["-o", executable.to_str().unwrap()]].concat(),
            );
            assert!(
                built.status.success(),
                "{options:?}: {}",
                text(&built.stderr)
            );
            let output = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(
                output.status.code(),
                Some(0),
                "{options:?}: {}",
                text(&output.stderr)
            );
        }
        std::fs::remove_file(input).unwrap();
        std::fs::remove_file(executable).unwrap();
    }

    #[test]
    fn interpreter_step_limit_applies_to_both_main_signatures() {
        for source in [
            SUM.to_owned(),
            SUM.replace("main(void)", "main(int argc, char **argv)"),
        ] {
            let limited = bcc(&source, &["--interpret", "--interpret-step-limit=1"]);
            assert_eq!(limited.status.code(), Some(70), "{}", text(&limited.stderr));
            assert!(text(&limited.stderr).contains("exceeded its step limit"));
            for limit in ["10000", "0"] {
                let result = bcc(
                    &source,
                    &["--interpret", &format!("--interpret-step-limit={limit}")],
                );
                assert_eq!(result.status.code(), Some(10), "{}", text(&result.stderr));
            }
        }
        for flags in [
            vec!["--interpret-step-limit=1"],
            vec!["--interpret", "--interpret-step-limit=-1"],
            vec!["--interpret", "--interpret-step-limit=none"],
        ] {
            assert_eq!(bcc(SUM, &flags).status.code(), Some(2));
        }
    }

    #[test]
    fn c99_host_math_families_match_native_output_in_each_precision() {
        let Some(target) = host() else {
            return;
        };
        let clang = if cfg!(windows) {
            "target/llvm/bin/clang.exe"
        } else {
            "target/llvm/bin/clang"
        };
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/programs/host_math.c");
        for long_double in [false, true] {
            let executable = temporary("host-math.exe");
            let mut compiler = Command::new(clang);
            _ = compiler
                .arg(&source)
                .args([
                    &format!("--target={target}"),
                    "-std=c99",
                    "-O0",
                    "-fno-builtin",
                    "-o",
                ])
                .arg(&executable);
            if !long_double {
                _ = compiler.arg("-DNO_LONG_DOUBLE");
            }
            if cfg!(unix) {
                _ = compiler.arg("-lm");
            }
            let built = compiler.output().unwrap();
            assert!(built.status.success(), "{}", text(&built.stderr));
            let reference = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(
                reference.status.code(),
                Some(0),
                "{}",
                text(&reference.stdout)
            );
            let expected = text(&reference.stdout);
            assert!(
                expected.lines().all(|line| line.ends_with(" 1")),
                "{expected}"
            );
            let targets = if long_double {
                vec!["x86_64-pc-windows-msvc"]
            } else {
                vec!["x86_64-w64-windows-gnu", "x86_64-unknown-linux-gnu"]
            };
            for target in targets {
                for optimize in ["--opt=none", "--opt=bcc"] {
                    let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
                    _ = command.arg(&source).args([
                        "--interpret",
                        &format!("--target={target}"),
                        optimize,
                    ]);
                    if !long_double {
                        _ = command.arg("-DNO_LONG_DOUBLE");
                    }
                    let result = run_native(&mut command).unwrap();
                    assert_eq!(
                        result.status.code(),
                        Some(0),
                        "{target} {optimize}: {}\n{}",
                        text(&result.stderr),
                        text(&result.stdout)
                    );
                    assert_eq!(text(&result.stdout), expected, "{target} {optimize}");
                }
            }
            std::fs::remove_file(executable).unwrap();
        }
    }

    #[test]
    fn host_math_preserves_the_existing_gnu_long_double_execution_limit() {
        for source in [
            "long double fabsl(long double); int main(void) { return fabsl(-1.5L)!=1.5L; }",
            "float nexttowardf(float,long double); int main(void) { return \
             nexttowardf(1.0f,2.0L)>1.0f; }",
        ] {
            let output = bcc(source, &["--interpret", "--target=x86_64-w64-windows-gnu"]);
            assert_eq!(output.status.code(), Some(70), "{}", text(&output.stderr));
            assert!(
                text(&output.stderr).contains("f80"),
                "{}",
                text(&output.stderr)
            );
        }
    }

    #[test]
    fn floating_output_formats_values_beyond_the_integer_digit_buffer() {
        let output = bcc(
            "int printf(const char *,...); int main(void) {
printf(\"%f %f %F\\n\",1e100,-0.0,1.0/0.0); return 0; }",
            &["--interpret"],
        );
        assert!(output.status.success(), "{}", text(&output.stderr));
        let output = text(&output.stdout);
        assert!(
            output.len() > 100 && output.ends_with(".000000 -0.000000 INF\n"),
            "{output}"
        );
    }

    #[test]
    fn aggregate_fetch_abi_refusal_has_owner_and_source_location() {
        let source = "struct Huge { char bytes[4294967296ULL]; };\nint fetch(int last, ...) \
                      {\n__builtin_va_list ap; \
                      __builtin_va_start(ap,last);\n__builtin_va_arg(ap,struct Huge); \
                      __builtin_va_end(ap); return 0;\n}\n";
        for (source, owner, line) in [
            (source.to_owned(), "fetch", 2),
            (
                source.replace(
                    "int fetch(int last, ...) {",
                    "int fetch(int last, ...) __asm__(\"alias\");\nint fetch(int last, ...) {",
                ),
                "alias",
                3,
            ),
        ] {
            for target in [
                "x86_64-unknown-linux-gnu",
                "x86_64-w64-windows-gnu",
                "x86_64-pc-windows-msvc",
            ] {
                for optimize in ["--opt=none", "--opt=bcc"] {
                    let output = bcc(
                        &source,
                        &[&format!("--target={target}"), optimize, "--emit=ir-abi"],
                    );
                    let stderr = text(&output.stderr);
                    assert_eq!(output.status.code(), Some(1), "{stderr}");
                    assert!(
                        stderr.contains(&format!("@{owner}: aggregate size 4294967296")),
                        "{stderr}"
                    );
                    assert!(stderr.contains(&format!("<input>:{line}:5")), "{stderr}");
                    assert_eq!(output.stdout.as_slice(), b"");
                }
            }
        }
    }

    /// glibc's Clang path implements `fpclassify`, `isfinite`, `isnormal`,
    /// `isnan` and `isinf` with these builtins; MinGW's passes the frame
    /// address to `_setjmp`. The interpreter has no x87 values, so `long
    /// double` runs natively only.
    #[test]
    fn classification_and_frame_address_builtins_run_in_both_back_ends() {
        let Some(target) = host() else {
            return;
        };
        let target = format!("--target={target}");
        let check = |ty: &str, subnormal: &str| {
            format!(
                "  {{ volatile {ty} zero = 0, one = 1, tiny = {subnormal};\n    {ty} nan = zero / \
                 zero, inf = one / zero, values[] = {{ nan, -inf, inf, one, -tiny, zero, -zero \
                 }};\n    int expected[] = {{ 0, 1, 1, 2, 3, 4, 4 }};\n    for (int i = 0; i < 7; \
                 i++) {{\n      {ty} v = values[i];\n      int c = __builtin_fpclassify(0, 1, 2, \
                 3, 4, v);\n      if (c != expected[i] || __builtin_isnan(v) != (c == 0)\n        \
                 || __builtin_isfinite(v) != (c > 1) || __builtin_isnormal(v) != (c == 2)\n        \
                 || __builtin_isinf_sign(v) != (c == 1 ? (v < 0 ? -1 : 1) : 0))\n        return \
                 failures + i + 1;\n    }}\n    failures += 10; }}\n"
            )
        };
        let program = |long_double: bool| {
            let mut source = String::from(
                "int main(void) {\n  int failures = -20;\n  void *frame = \
                 __builtin_frame_address(0);\n  void *ret = \
                 __builtin_extract_return_addr(__builtin_return_address(0));\n  if (!frame || \
                 !ret) return 100;\n",
            );
            source.push_str(&check("float", "1e-40f"));
            source.push_str(&check("double", "1e-310"));
            if long_double {
                source.push_str(&check("long double", "1e-4940L"));
                source.push_str("  failures -= 10;\n");
            }
            source.push_str("  return failures;\n}\n");
            source
        };
        for opt in ["--opt=none", "--opt=bcc"] {
            let output = bcc(&program(false), &[&target, opt, "--interpret"]);
            assert!(output.status.success(), "{opt}: {}", text(&output.stderr));
        }
        let executable = temporary("classify.exe");
        for opt in [&["--llvm-opt=O0"][..], &["--opt=bcc", "--llvm-opt=O2"][..]] {
            // MSVC's long double is binary64, where the x87 subnormal is 0.
            let built = bcc(
                &program(!target.ends_with("msvc")),
                &[&[&target[..], "-o", executable.to_str().unwrap()][..], opt].concat(),
            );
            assert!(built.status.success(), "{opt:?}: {}", text(&built.stderr));
            let output = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(output.status.code(), Some(0), "{opt:?}");
        }
        std::fs::remove_file(executable).unwrap();
    }

    /// `setjmp` and `longjmp` (#50) from `tests/fixtures/programs/setjmp.c`:
    /// MinGW-w64's declarations, whose `setjmp` passes `mingw_getsp()`, in
    /// the interpreter for the Windows target; the host's `<setjmp.h>` in the
    /// interpreter and in native executables built through LLVM. The native
    /// back end refuses returns-twice calls rather than miscompile them.
    #[test]
    fn setjmp_and_longjmp_run_in_the_interpreter_and_natively() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/programs/setjmp.c");
        let compile = |flags: &[&str]| {
            let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
            _ = command.arg(&source).args(flags).env("NO_COLOR", "1");
            run_native(&mut command).unwrap()
        };
        for opt in ["--opt=none", "--opt=bcc"] {
            let output = compile(&[
                "--interpret",
                "--target=x86_64-w64-windows-gnu",
                "-DMINGW_DECLARATIONS",
                opt,
            ]);
            assert_eq!(
                output.status.code(),
                Some(0),
                "{opt}: {}",
                text(&output.stderr)
            );
        }
        // bcc has no default C library directories; take the host's.
        let (target, include) = match host() {
            | Some(target @ "x86_64-unknown-linux-gnu")
                if Path::new("/usr/include/setjmp.h").is_file() =>
                (
                    target,
                    vec![
                        "/usr/include/x86_64-linux-gnu".to_owned(),
                        "/usr/include".to_owned(),
                    ],
                ),
            | Some(target @ "x86_64-w64-windows-gnu") => {
                let Some((_, include)) = mingw() else {
                    eprintln!("skipping native runs: no MinGW-w64 gcc on PATH");
                    return;
                };
                (target, vec![include.to_str().unwrap().to_owned()])
            },
            | _ => {
                eprintln!("skipping native runs: no supported host C library");
                return;
            },
        };
        let mut flags = vec![format!("--target={target}")];
        for directory in include {
            flags.extend(["--idirafter".to_owned(), directory]);
        }
        let flags: Vec<&str> = flags.iter().map(String::as_str).collect();
        let target = &flags[..];
        for opt in ["--opt=none", "--opt=bcc"] {
            let output = compile(&[&["--interpret", opt][..], target].concat());
            assert_eq!(
                output.status.code(),
                Some(0),
                "{opt}: {}",
                text(&output.stderr)
            );
        }
        let executable = temporary("setjmp.exe");
        let path = executable.to_str().unwrap();
        for opt in [&["--llvm-opt=O0"][..], &["--opt=bcc", "--llvm-opt=O2"][..]] {
            let built = compile(&[target, &["-o", path][..], opt].concat());
            assert!(built.status.success(), "{opt:?}: {}", text(&built.stderr));
            let output = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(output.status.code(), Some(0), "{opt:?}");
        }
        std::fs::remove_file(&executable).unwrap();
        let refused = compile(&[target, &["--backend=native", "-o", path][..]].concat());
        assert!(!refused.status.success());
        assert!(
            text(&refused.stderr).contains(
                "not yet supported by the native back end: a call to, or the address of, a \
                 function that may return twice"
            ),
            "{}",
            text(&refused.stderr)
        );
        assert!(!executable.exists());
    }

    #[test]
    fn inline_modes_emit_bodies_and_run_in_both_back_ends() {
        let Some(target) = host() else {
            return;
        };
        let target = format!("--target={target}");
        for (flags, definition) in [
            (
                vec!["-std=gnu17", "-fgnu89-inline"],
                "inline int f(int x) { return x + 1; }",
            ),
            (vec!["-std=gnu89"], "inline int f(int x) { return x + 1; }"),
            (
                vec!["-std=c89"],
                "__inline__ int f(int x) { return x + 1; }",
            ),
            (
                vec!["-std=c90"],
                "__inline__ int f(int x) { return x + 1; }",
            ),
            (
                vec!["-std=gnu17", "-fgnu89-inline", "-fno-gnu89-inline"],
                "extern inline int f(int x) { return x + 1; }",
            ),
            (
                vec!["-std=gnu89", "-fno-gnu89-inline"],
                "extern inline int f(int x) { return x + 1; }",
            ),
            (
                vec!["-std=c99"],
                "extern inline int f(int x) { return x + 1; }",
            ),
            (
                vec!["-std=c99"],
                "inline int f(int x) { return x + 1; } extern int f(int);",
            ),
            (
                vec!["-std=c99"],
                "inline __attribute__((gnu_inline)) int f(int x) { return x + 1; }",
            ),
            (
                vec!["-std=c99"],
                "extern inline __attribute__((gnu_inline)) int f(int x) { return 99; } int f(int \
                 x) { return x + 1; }",
            ),
            (
                vec!["-std=c99"],
                "static inline int f(int x) { return x + 1; }",
            ),
            (
                vec!["-std=c99", "-fgnu89-inline"],
                "static inline int f(int x) { return x + 1; }",
            ),
        ] {
            // Taking the address also verifies that a real callable body
            // exists.
            let source =
                format!("{definition} int main(void) {{ int (*p)(int) = f; return p(6) != 7; }}");
            let executable = temporary("inline.exe");
            let path = executable.to_str().unwrap();
            for opt in ["--opt=none", "--opt=bcc"] {
                let mut common = flags.clone();
                common.extend([&target, opt]);
                let mut interpret = common.clone();
                interpret.push("--interpret");
                let output = bcc(&source, &interpret);
                assert!(
                    output.status.success(),
                    "{flags:?}: {}",
                    text(&output.stderr)
                );
                common.extend(["--llvm-opt=O0", "-o", path]);
                let output = bcc(&source, &common);
                assert!(
                    output.status.success(),
                    "{flags:?}: {}",
                    text(&output.stderr)
                );
                let output = run_native(&mut Command::new(&executable)).unwrap();
                assert!(output.status.success(), "{flags:?}: {output:?}");
            }
            std::fs::remove_file(executable).unwrap();
        }
    }

    #[test]
    fn inline_only_calls_stay_external_and_w_suppresses_only_warnings() {
        for (flags, definition) in [
            (vec!["-std=c99"], "inline int f(void) { return 99; }"),
            (
                vec!["-std=c99", "-fgnu89-inline"],
                "extern inline int f(void) { return 99; }",
            ),
            (
                vec!["-std=c99"],
                "extern inline __attribute__((gnu_inline)) int f(void) { return 99; }",
            ),
        ] {
            let source = format!("{definition} int main(void) {{ return f(); }}");
            for opt in ["--opt=none", "--opt=bcc"] {
                let mut args = flags.clone();
                args.extend([opt, "--emit=llvm"]);
                let output = bcc(&source, &args);
                assert!(output.status.success(), "{}", text(&output.stderr));
                assert!(
                    text(&output.stdout).contains("declare i32 @f("),
                    "{}",
                    text(&output.stdout)
                );
                _ = args.pop();
                args.push("--interpret");
                let output = bcc(&source, &args);
                assert_eq!(output.status.code(), Some(70), "{}", text(&output.stderr));
            }
        }
        let source = "int main(void) { exit(0); }";
        assert!(text(&bcc(source, &["--interpret"]).stderr).contains("warning:"));
        let output = bcc(source, &["--interpret", "-w"]);
        assert!(output.status.success());
        assert!(output.stderr.is_empty(), "{}", text(&output.stderr));
        let output = bcc("int main(void) { return missing; }", &["--interpret", "-w"]);
        assert!(!output.status.success());
        assert!(text(&output.stderr).contains("error:"));
    }

    #[test]
    fn fold_finishes_many_noundef_parameters_and_volatile_bit_field_writes() {
        use std::fmt::Write as _;

        let mut source = String::from("struct B { unsigned b:1; }; void f(");
        for index in 0..1023 {
            write!(source, "int p{index}, ").unwrap();
        }
        source.push_str("volatile struct B *q) {\n");
        for _ in 0..1024 {
            source.push_str("q->b = p0;\n");
        }
        source.push_str("}\n");
        // The old nested parameter/attribute scans performed ~537M comparisons.
        // Allow ample process and front-end time while bounding that
        // regression.
        let output = run_native_with_timeout(
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(["--emit=ir", "--passes=fold", "--input", &source])
                .env("NO_COLOR", "1")
                .env_remove("CLICOLOR_FORCE"),
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        assert!(output.status.success(), "{}", text(&output.stderr));
        assert!(output.stderr.is_empty(), "{}", text(&output.stderr));
        let ir = text(&output.stdout);
        assert_eq!(ir.matches(" noundef").count(), 1024);
        assert_eq!(ir.matches("freeze.").count(), 1024);
    }

    #[test]
    fn noundef_scalar_argument_reports_ub_with_status_70() {
        let source =
            "static int ignore(int x) { return 0; } int main(void) { int x; return ignore(x); }";
        for target in [
            "--target=x86_64-unknown-linux-gnu",
            "--target=x86_64-w64-windows-gnu",
        ] {
            let output = bcc(source, &["--interpret", "--opt=none", target]);
            assert_eq!(output.status.code(), Some(70), "{}", text(&output.stderr));
            assert!(text(&output.stderr).contains("poison reached a noundef"));
        }
    }

    #[test]
    fn nested_overlapping_unions_finish_and_match_the_simple_union_abi() {
        let source = include_str!("../fixtures/lowering/nested_overlapping_unions.c");
        for target in [
            "--target=x86_64-unknown-linux-gnu",
            "--target=x86_64-w64-windows-gnu",
            "--target=x86_64-pc-windows-msvc",
        ] {
            for opt in ["--opt=none", "--opt=bcc"] {
                for mode in ["--emit=ir", "--emit=ir-abi", "--emit=llvm", "--interpret"] {
                    let output = run_native_with_timeout(
                        Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                            .args([mode, target, opt, "--input", source])
                            .env("NO_COLOR", "1")
                            .env_remove("CLICOLOR_FORCE"),
                        std::time::Duration::from_secs(10),
                    )
                    .unwrap();
                    assert!(
                        output.status.success(),
                        "{mode} {target} {opt}: {}",
                        text(&output.stderr)
                    );
                    assert!(output.stderr.is_empty(), "{}", text(&output.stderr));
                    let ir = text(&output.stdout);
                    match mode {
                        | "--emit=ir" =>
                            for name in ["f", "simple"] {
                                assert!(
                                    ir.contains(&format!(
                                        "function @{name}(ptr aggregate(size 4, align 4, [0: \
                                         i32]) noundef) -> i32"
                                    )),
                                    "{ir}"
                                );
                            },
                        | "--emit=ir-abi" =>
                            for name in ["f", "simple"] {
                                assert!(
                                    ir.contains(&format!("function @{name}(i32) -> i32")),
                                    "{ir}"
                                );
                            },
                        | "--emit=llvm" =>
                            for name in ["f", "simple"] {
                                assert!(ir.contains(&format!("define i32 @{name}(i32 %")), "{ir}");
                            },
                        | "--interpret" => assert!(ir.is_empty(), "{ir}"),
                        | _ => unreachable!("all tested modes are covered"),
                    }
                }
            }
        }
    }

    #[test]
    fn unresolved_external_objects_trap_on_access() {
        for source in [
            "extern int shared; int main(void){ return shared != 7; }",
            "extern int shared; int main(void){ shared = 7; return 0; }",
            "extern int shared; int main(void){ int *p = &shared; return *p; }",
            "extern int shared; int *p = &shared; int main(void){ return *p; }",
            "extern int shared[2]; int *p = shared; int main(void){ return p[1]; }",
        ] {
            for opt in ["--opt=none", "--opt=bcc"] {
                let output = bcc(&format!("{source}\n"), &["--interpret", opt]);
                assert_eq!(
                    output.status.code(),
                    Some(70),
                    "{source}: {}",
                    text(&output.stderr)
                );
                assert!(
                    text(&output.stderr).contains("unresolved external object @shared"),
                    "{source}: {}",
                    text(&output.stderr)
                );
                assert_eq!(output.stdout.as_slice(), b"");
            }
        }
    }

    #[test]
    fn tentative_object_definitions_remain_zero_initialized() {
        for source in [
            "int shared; int main(void){ return shared; }",
            "extern int shared; int shared; int main(void){ return shared; }",
            "int shared; int *p = &shared; int main(void){ return *p; }",
            "extern int shared; int shared = 7; int main(void){ return shared != 7; }",
            "extern int shared; int main(void){ return 0; }",
        ] {
            for opt in ["--opt=none", "--opt=bcc"] {
                let output = bcc(&format!("{source}\n"), &["--interpret", opt]);
                assert_eq!(
                    output.status.code(),
                    Some(0),
                    "{source}: {}",
                    text(&output.stderr)
                );
                assert_eq!(output.stderr.as_slice(), b"");
            }
        }
    }

    #[test]
    fn incompatible_host_signatures_trap_before_execution() {
        for source in [
            "double malloc(unsigned long); int main(void){return (int)malloc(1);}",
            "double putchar(int); int main(void){return (int)putchar(65);}",
            "double abs(int); int main(void){return (int)abs(-1);}",
            "double printf(const char *, ...); int main(void){return (int)printf(\"x\");}",
            "int fabs(double); int main(void){return fabs(-1.);}",
            "int free(void *); int main(void){return free(0);}",
            "int exit(int); int main(void){return exit(0);}",
            "int abort(void); int main(void){return abort();}",
            "void putchar(int); int main(void){putchar(65); return 0;}",
            "int putchar(double); int main(void){return putchar(65.);}",
            "void *memcpy(void *, const void *, double); int main(void){char a, b; memcpy(&a, &b, \
             1.); return 0;}",
            "int puts(void); int main(void){return puts();}",
            "int abs(int, int); int main(void){return abs(1, 2);}",
            "double putchar(int); int main(void){double (*p)(int)=putchar; return (int)p(65);}",
        ] {
            for opt in ["--opt=none", "--opt=bcc"] {
                let output = bcc(&format!("{source}\n"), &["--interpret", opt]);
                assert_eq!(
                    output.status.code(),
                    Some(70),
                    "{source}: {}",
                    text(&output.stderr)
                );
                assert!(
                    text(&output.stderr).contains("wrong signature"),
                    "{source}: {}",
                    text(&output.stderr)
                );
                assert!(
                    output.stdout.is_empty(),
                    "{source}: {}",
                    text(&output.stdout)
                );
            }
        }
    }

    #[test]
    fn poison_return_from_main_traps_like_exit() {
        for ending in ["return x;", "exit(x);"] {
            let source = format!("void exit(int); int main(void) {{ int x; {ending} }}\n");
            for flags in [["--interpret", "--opt=none"], ["--interpret", "--opt=bcc"]] {
                let output = bcc(&source, &flags);
                assert_eq!(
                    output.status.code(),
                    Some(70),
                    "{source}: {}",
                    text(&output.stderr)
                );
                assert!(text(&output.stderr).contains("undefined behaviour"));
                assert!(text(&output.stderr).contains("poison"));
            }
        }
    }

    const HELLO: &str = "int printf(const char *, ...);\nint main(void) {\n  printf(\"%s %d\\n\", \
                         \"answer\", 42);\n  return 3;\n}\n";

    /// Passes a structure by value both ways, which ABI lowering turns into
    /// two registers on System V and an `sret` pointer and a copy on Win64.
    const PAIR: &str = "struct pair { long long a, b; };\nstruct pair swap(struct pair p) {\n  \
                        struct pair r = { p.b, p.a };\n  return r;\n}\nint main(void) {\n  struct \
                        pair p = { 3, 40 };\n  return swap(p).a + 2;\n}\n";

    /// Runs the compiler on the inline `source` with `flags`.
    fn bcc(source: &str, flags: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(flags)
            .args(["--input", source])
            .env("NO_COLOR", "1")
            .env_remove("CLICOLOR_FORCE")
            .output()
            .unwrap()
    }

    /// Output bytes as text, with Windows line ends made plain.
    fn text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).replace("\r\n", "\n")
    }

    #[test]
    fn closed_stdout_exits_quietly() {
        use std::{
            fmt::Write as _,
            process::Stdio,
        };
        let mut source = String::from(
            "int putchar(int); int main(void) { for (int i = 0; i < 20000; ++i) putchar(65); \
             return 3; }\n",
        );
        for i in 0..300 {
            writeln!(source, "int f{i}(int x) {{ return x + {i}; }}").unwrap();
        }
        for flag in [
            "--emit=ir",
            "--emit=ir-abi",
            "--emit=llvm",
            "--print-default-passes",
        ] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
            _ = command
                .arg(flag)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if flag != "--print-default-passes" {
                _ = command.args(["--input", &source]);
            }
            let mut child = command.spawn().unwrap();
            // Close the read end before compilation finishes; enough output is
            // produced to reach the OS pipe even with stdout buffering.
            drop(child.stdout.take());
            let output = child.wait_with_output().unwrap();
            assert_eq!(
                output.status.code(),
                Some(0),
                "{flag}: {}",
                text(&output.stderr)
            );
            assert!(output.stderr.is_empty(), "{flag}: {}", text(&output.stderr));
        }
    }

    #[test]
    fn closed_stdout_preserves_interpreter_status_and_traps() {
        use std::process::Stdio;
        for (ending, status, diagnostic) in [
            ("return 42;", 42, ""),
            ("exit(23);", 23, ""),
            ("abort();", 134, ""),
            (
                "return 1 / zero;",
                70,
                "error: the interpreter stopped the program:",
            ),
        ] {
            let source = format!(
                "int putchar(int); void exit(int); void abort(void); int main(void) {{ int zero = \
                 0; for (int i = 0; i < 20000; ++i) putchar(65); {ending} }}\n"
            );
            let mut child = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(["--interpret", "--input", &source])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            drop(child.stdout.take());
            let output = child.wait_with_output().unwrap();
            let stderr = text(&output.stderr);
            assert_eq!(output.status.code(), Some(status), "{ending}: {stderr}");
            if diagnostic.is_empty() {
                assert!(stderr.is_empty(), "{ending}: {stderr}");
            } else {
                assert!(stderr.contains(diagnostic), "{ending}: {stderr}");
            }
        }
    }

    /// Opens a stdio handle whose writes fail. Rust's standard library
    /// treats EBADF as closed stdio and drops the output silently, so on
    /// Unix a read-only descriptor does not fail; `/dev/full` reports ENOSPC.
    fn failing_stdio(path: &Path) -> std::fs::File {
        if cfg!(unix) {
            std::fs::File::options()
                .write(true)
                .open("/dev/full")
                .unwrap()
        } else {
            std::fs::File::open(path).unwrap()
        }
    }

    #[test]
    fn read_only_stdout_reports_an_error_without_panicking() {
        let path = temporary("read-only-stdout.txt");
        std::fs::write(&path, b"unchanged").unwrap();
        for flag in [
            "--emit=ir",
            "--emit=ir-abi",
            "--emit=llvm",
            "--interpret",
            "--print-default-passes",
        ] {
            let file = failing_stdio(&path);
            let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
            _ = command.arg(flag).stdout(file);
            if flag != "--print-default-passes" {
                _ = command.args(["--input", HELLO]);
            }
            let output = command.output().unwrap();
            let stderr = text(&output.stderr);
            assert_eq!(output.status.code(), Some(1), "{flag}: {stderr}");
            assert!(
                stderr.starts_with("error: failed writing to stdout: "),
                "{flag}: {stderr}"
            );
            assert!(!stderr.contains("panicked"), "{flag}: {stderr}");
        }
        assert_eq!(std::fs::read(&path).unwrap(), b"unchanged");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn read_only_stderr_preserves_exit_status() {
        let path = temporary("read-only-stderr.txt");
        std::fs::write(&path, b"unchanged").unwrap();
        let missing_output = temporary("missing-parent").join("output.o");
        let missing_input = temporary("missing-input.c");
        for (source, flags, status, fail_stdout) in [
            (
                "int main(void){int z=0;return 1/z;}",
                vec!["--interpret"],
                70,
                false,
            ),
            (
                "int main(void){return unknown;}",
                vec!["--interpret"],
                1,
                false,
            ),
            (
                "int main(void){__asm__(\"nop\");return 0;}",
                vec!["--emit=ir"],
                1,
                false,
            ),
            (HELLO, vec!["--interpret"], 1, true),
            (SUM, vec!["--emit=llvm"], 1, true),
            (
                SUM,
                vec!["-c", "-o", missing_output.to_str().unwrap()],
                1,
                false,
            ),
            (
                "int main(void){return 42;}",
                vec!["--interpret", "--print-after-all"],
                42,
                false,
            ),
            ("int main(void){return 0;}", vec!["--interpret"], 0, false),
            (
                "void abort(void);int main(void){abort();}",
                vec!["--interpret"],
                134,
                false,
            ),
        ] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
            _ = command
                .args(&flags)
                .args(["--input", source])
                .stderr(failing_stdio(&path));
            if fail_stdout {
                _ = command.stdout(failing_stdio(&path));
            }
            let output = command.output().unwrap();
            assert_eq!(output.status.code(), Some(status), "{flags:?}: {source}");
        }
        let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .arg("--interpret")
            .arg(&missing_input)
            .stderr(failing_stdio(&path))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "input file error");
        assert_eq!(std::fs::read(&path).unwrap(), b"unchanged");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn interpreter_rejects_excessive_printf_widths_and_precisions() {
        for format in [
            "%18446744073709551616d",
            "%999999999999999999999999999999d",
            "%1048577d",
            "%.18446744073709551616d",
            "%.1048577d",
        ] {
            let source = format!(
                "int printf(const char *, ...); int main(void) {{ printf(\"{format}\", 1); return \
                 0; }}\n"
            );
            let output = bcc(&source, &["--interpret"]);
            let stderr = text(&output.stderr);
            assert_eq!(output.status.code(), Some(70), "{format}: {stderr}");
            assert!(
                stderr.starts_with("error: the interpreter stopped the program:"),
                "{format}: {stderr}"
            );
            assert!(
                output.stdout.is_empty(),
                "{format}: output was padded before rejecting the format"
            );
            assert!(!stderr.contains("panicked"), "{format}: {stderr}");
        }
    }

    #[test]
    fn print_default_passes_lists_a_pipeline_that_passes_accepts() {
        let run = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .arg("--print-default-passes")
            .output()
            .unwrap();
        assert!(run.status.success(), "{}", text(&run.stderr));
        let list = text(&run.stdout);
        let list = list.trim_end();
        assert!(!list.is_empty() && !list.contains('\n'), "{list:?}");
        // The list is in `--passes` syntax, so the optimizer accepts it.
        let passes = format!("--passes={list}");
        let optimized = bcc(SUM, &["--emit=ir", &passes]);
        assert!(optimized.status.success(), "{}", text(&optimized.stderr));
    }

    #[test]
    fn emit_ir_prints_the_module_and_the_optimizer_rewrites_it() {
        let lowered = bcc(SUM, &["--emit=ir"]);
        assert!(lowered.status.success(), "{}", text(&lowered.stderr));
        let lowered = text(&lowered.stdout);
        assert!(
            lowered.starts_with("target triple = \"x86_64-unknown-linux-gnu\"\n"),
            "{lowered}"
        );
        assert!(
            lowered.contains("function @main() -> i32 external {"),
            "{lowered}"
        );
        // The loop's increment has its own block until simplify-cfg.
        assert!(lowered.contains("block4:"), "{lowered}");

        for flags in [
            &["--emit=ir", "--opt=bcc"][..],
            &["--emit=ir", "--passes=simplify-cfg"][..],
        ] {
            let optimized = bcc(SUM, flags);
            assert!(optimized.status.success(), "{}", text(&optimized.stderr));
            let optimized = text(&optimized.stdout);
            assert!(!optimized.contains("block4:"), "{flags:?}: {optimized}");
        }
    }

    #[test]
    fn print_after_all_and_the_bisect_limit_control_the_optimizer() {
        let printed = bcc(
            SUM,
            &["--emit=ir", "--passes=fold,dce", "--print-after-all"],
        );
        assert!(printed.status.success());
        let stderr = text(&printed.stderr);
        assert!(
            stderr.contains("*** IR after fold of @main ***"),
            "{stderr}"
        );
        assert!(stderr.contains("*** IR after dce of @main ***"), "{stderr}");

        // No rewrite is allowed, so the module stays as lowered.
        let bisected = bcc(SUM, &["--emit=ir", "--opt=bcc", "--opt-bisect-limit=0"]);
        assert!(bisected.status.success());
        assert_eq!(
            text(&bisected.stdout),
            text(&bcc(SUM, &["--emit=ir"]).stdout)
        );

        // The per-pass report goes to stderr and leaves the module alone.
        let timed = bcc(SUM, &["--emit=ir", "--time-passes"]);
        assert!(timed.status.success());
        let report = text(&timed.stderr);
        assert!(
            report.contains(" rewrites") && report.contains("  sroa "),
            "{report}"
        );
        assert_eq!(
            text(&timed.stdout),
            text(&bcc(SUM, &["--emit=ir", "--opt=bcc"]).stdout)
        );

        let unknown = bcc(SUM, &["--emit=ir", "--passes=fold,unknown"]);
        assert_eq!(unknown.status.code(), Some(2));
        assert!(text(&unknown.stderr).contains("unknown pass `unknown`"));
    }

    #[test]
    fn emit_llvm_prints_llvm_ir_for_the_target() {
        let output = bcc(HELLO, &["--emit=llvm", "--target=x86_64-w64-windows-gnu"]);
        assert!(output.status.success(), "{}", text(&output.stderr));
        let stdout = text(&output.stdout);
        assert!(
            stdout.contains("target triple = \"x86_64-w64-windows-gnu\""),
            "{stdout}"
        );
        assert!(stdout.contains("define i32 @main()"), "{stdout}");
        assert!(
            stdout.contains("declare i32 @printf(ptr noundef, ...)"),
            "{stdout}"
        );
    }

    #[test]
    fn back_ends_receive_the_module_after_abi_lowering() {
        // `--emit=ir` prints the module before ABI lowering ...
        let pre = bcc(PAIR, &["--emit=ir", "--opt=bcc"]);
        assert!(pre.status.success(), "{}", text(&pre.stderr));
        let pre = text(&pre.stdout);
        assert!(!pre.contains("abi lowered"), "{pre}");
        assert!(
            pre.contains(
                "function @swap(ptr result(size 16, align 8, [0: i64, 8: i64]), ptr \
                 aggregate(size 16, align 8, [0: i64, 8: i64]) noundef) external"
            ),
            "{pre}"
        );
        // ... and `--emit=ir-abi` after it, for the target's convention.
        let post = bcc(PAIR, &["--emit=ir-abi", "--opt=bcc"]);
        assert!(post.status.success(), "{}", text(&post.stderr));
        let post = text(&post.stdout);
        assert!(post.contains("abi lowered\n"), "{post}");
        assert!(
            post.contains("function @swap(ptr registers(i64, i64, align 8), i64, i64) external"),
            "{post}"
        );
        let win64 = bcc(PAIR, &["--emit=ir-abi", "--target=x86_64-w64-windows-gnu"]);
        let win64 = text(&win64.stdout);
        assert!(
            win64.contains("function @swap(ptr sret(size 16, align 8), ptr noundef) external"),
            "{win64}"
        );

        // The LLVM back end prints the target's convention ...
        let llvm = bcc(PAIR, &["--emit=llvm"]);
        assert!(llvm.status.success(), "{}", text(&llvm.stderr));
        let llvm = text(&llvm.stdout);
        assert!(llvm.contains("define { i64, i64 } @swap(i64 %"), "{llvm}");
        let llvm = text(&bcc(PAIR, &["--emit=llvm", "--target=x86_64-w64-windows-gnu"]).stdout);
        assert!(llvm.contains("sret([16 x i8]) align 8"), "{llvm}");

        // ... and the interpreter runs the post-ABI module for each target.
        for target in [
            "--target=x86_64-unknown-linux-gnu",
            "--target=x86_64-w64-windows-gnu",
        ] {
            for opt in ["--opt=none", "--opt=bcc"] {
                let run = bcc(PAIR, &["--interpret", target, opt]);
                assert_eq!(
                    run.status.code(),
                    Some(42),
                    "{target} {opt}: {}",
                    text(&run.stderr)
                );
            }
        }
    }

    #[test]
    fn interpret_exits_with_the_status_of_main_and_forwards_output() {
        let output = bcc(HELLO, &["--interpret"]);
        assert_eq!(output.status.code(), Some(3), "{}", text(&output.stderr));
        assert_eq!(text(&output.stdout), "answer 42\n");

        let output = bcc(SUM, &["--interpret", "--opt=bcc"]);
        assert_eq!(output.status.code(), Some(10));

        let exited = bcc(
            "void exit(int);\nint main(void) {\n  exit(5);\n  return 1;\n}\n",
            &["--interpret"],
        );
        assert_eq!(exited.status.code(), Some(5));
    }

    #[test]
    fn interpret_supplies_modifiable_main_arguments() {
        let source = "int main(int argc, char **argv) {\nif (argc != 1 || argv == 0 || argv[0] == \
                      0 || argv[1] != 0) return 1;\nargv[0][0] = 'x'; argv[0] = 0; return 0;\n}\n";
        for target in [
            "--target=x86_64-w64-windows-gnu",
            "--target=x86_64-unknown-linux-gnu",
        ] {
            for opt in ["--opt=none", "--opt=bcc"] {
                let output = bcc(source, &["--interpret", target, opt]);
                assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
            }
        }
    }

    #[test]
    fn interpreter_traps_report_an_error_and_a_distinct_status() {
        let output = bcc(
            "int main(void) {\n  int a[2];\n  return a[5];\n}\n",
            &["--interpret"],
        );
        assert_eq!(output.status.code(), Some(70));
        let stderr = text(&output.stderr);
        assert!(
            stderr.contains("error: the interpreter stopped the program: undefined behaviour"),
            "{stderr}"
        );
    }

    #[test]
    fn unsupported_constructs_and_errors_fail_code_generation_only() {
        let source = "int main(void) {\n  __asm__(\"nop\");\n  return 0;\n}\n";
        for flags in [
            &["--interpret"][..],
            &["--emit=ir"][..],
            &["--emit=llvm"][..],
        ] {
            let output = bcc(source, flags);
            assert_eq!(output.status.code(), Some(1), "{flags:?}");
            assert!(output.stdout.is_empty(), "{flags:?}");
            let stderr = text(&output.stderr);
            assert!(
                stderr.contains("error: not yet supported by lowering: inline assembly"),
                "{stderr}"
            );
            assert!(stderr.contains("1 error generated."), "{stderr}");
        }
        // Without a back end, the program is valid C and nothing is lowered.
        let output = bcc(source, &[]);
        assert!(output.status.success());
        assert!(output.stderr.is_empty(), "{}", text(&output.stderr));

        let output = bcc("int main(void) {\n  return x;\n}\n", &["--interpret"]);
        assert_eq!(output.status.code(), Some(1));
        let stderr = text(&output.stderr);
        assert!(stderr.contains("no visible declaration"), "{stderr}");
        assert!(!stderr.contains("lowering"), "{stderr}");
    }

    #[test]
    fn back_end_options_need_a_back_end() {
        for flags in [
            &["-c"][..],
            &["--emit=ir", "--interpret"][..],
            &["--emit=ir", "--syntax-tree"][..],
        ] {
            let output = bcc(SUM, flags);
            assert_eq!(output.status.code(), Some(2), "{flags:?}");
        }
    }

    /// The target the tests run on, if the LLVM back end supports it.
    fn host() -> Option<&'static str> {
        if cfg!(all(windows, target_env = "gnu")) {
            Some("x86_64-w64-windows-gnu")
        } else if cfg!(all(windows, target_env = "msvc")) {
            Some("x86_64-pc-windows-msvc")
        } else if cfg!(all(target_os = "linux", target_env = "gnu")) {
            Some("x86_64-unknown-linux-gnu")
        } else if cfg!(all(target_os = "linux", target_env = "musl")) {
            Some("x86_64-unknown-linux-musl")
        } else {
            None
        }
    }

    /// A fresh path in the system temporary directory.
    fn temporary(name: &str) -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        env::temp_dir().join(format!(
            "bcc-cli-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn object_output_cannot_overwrite_input_source() {
        assert_output_preserves_input(true, false);
    }

    #[test]
    fn executable_output_cannot_overwrite_input_source() {
        assert_output_preserves_input(false, false);
    }

    #[test]
    fn object_output_cannot_overwrite_input_through_a_hard_link() {
        assert_output_preserves_input(true, true);
    }

    #[test]
    fn executable_output_cannot_overwrite_input_through_a_hard_link() {
        assert_output_preserves_input(false, true);
    }

    /// The same-file check must not open the output to read its identity:
    /// opening a FIFO for reading blocks until a writer appears.
    #[cfg(unix)]
    #[test]
    fn object_output_to_a_fifo_does_not_block() {
        let source = temporary("fifo.c");
        let fifo = temporary("fifo.o");
        std::fs::write(&source, SUM).unwrap();
        let made = Command::new("mkfifo").arg(&fifo).status().unwrap();
        assert!(made.success(), "mkfifo failed");
        let mut reader = Command::new("cat")
            .arg(&fifo)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let result = run_native_with_timeout(
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .arg(&source)
                .args(["-c", "-o"])
                .arg(&fifo),
            std::time::Duration::from_secs(60),
        );
        // A failed compile may never open the FIFO, leaving the reader
        // blocked in its own open; only a successful write ends it.
        if !result.as_ref().is_ok_and(|output| output.status.success()) {
            drop(reader.kill());
        }
        let reader_status = reader.wait().unwrap();
        std::fs::remove_file(&source).unwrap();
        std::fs::remove_file(&fifo).unwrap();
        let output = result.unwrap();
        assert!(output.status.success(), "{}", text(&output.stderr));
        assert!(reader_status.success());
    }

    fn assert_output_preserves_input(object: bool, hard_link: bool) {
        let source = temporary("preserve.c");
        let alias = temporary("alias.c");
        std::fs::write(&source, SUM).unwrap();
        let output = if hard_link {
            std::fs::hard_link(&source, &alias).unwrap();
            &alias
        } else {
            &source
        };
        let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
        _ = command.arg(&source).arg("-o").arg(output);
        if object {
            _ = command.arg("-c");
        }
        let result = command.output().unwrap();
        let contents = std::fs::read(&source).unwrap();
        std::fs::remove_file(&source).unwrap();
        if hard_link {
            std::fs::remove_file(&alias).unwrap();
        }
        assert!(contents == SUM.as_bytes(), "the C source was overwritten");
        assert_eq!(result.status.code(), Some(1), "{}", text(&result.stderr));
        assert_eq!(
            text(&result.stderr).trim(),
            format!(
                "error: output file '{}' is the same as input file '{}'",
                output.display(),
                source.display()
            )
        );
        assert_eq!(result.stdout.as_slice(), b"");
    }

    #[test]
    fn output_builds_executables_and_objects_through_clang() {
        let Some(target) = host() else {
            eprintln!("skipping: the host is not a supported target");
            return;
        };
        let clang = Path::new(env!("CARGO_MANIFEST_DIR")).join(if cfg!(windows) {
            "target/llvm/bin/clang.exe"
        } else {
            "target/llvm/bin/clang"
        });
        if !clang.is_file() {
            eprintln!("skipping: no bundled clang at {}", clang.display());
            return;
        }
        let target = format!("--target={target}");
        let executable = temporary("hello.exe");
        let object = temporary("hello.o");
        let path = |path: &Path| path.to_str().unwrap().to_owned();
        let built = bcc(
            HELLO,
            &[
                &target,
                "--opt=bcc",
                "--llvm-opt=O2",
                "--llvm-codegen-only",
                "-o",
                &path(&executable),
            ],
        );
        assert!(built.status.success(), "{}", text(&built.stderr));
        let run = run_native(&mut Command::new(&executable)).unwrap();
        assert_eq!(run.status.code(), Some(3));
        assert_eq!(text(&run.stdout), "answer 42\n");

        let compiled = bcc(HELLO, &[&target, "-c", "-o", &path(&object)]);
        assert!(compiled.status.success(), "{}", text(&compiled.stderr));
        assert!(object.is_file());

        // A structure passed and returned by value goes through ABI lowering.
        let pair = temporary("pair.exe");
        for flags in [&["--llvm-opt=O0"][..], &["--opt=bcc", "--llvm-opt=O2"][..]] {
            let built = bcc(
                PAIR,
                &[&[&target[..], "-o", &path(&pair)][..], flags].concat(),
            );
            assert!(built.status.success(), "{flags:?}: {}", text(&built.stderr));
            let run = run_native(&mut Command::new(&pair)).unwrap();
            assert_eq!(run.status.code(), Some(42), "{flags:?}");
        }
        drop(std::fs::remove_file(executable));
        drop(std::fs::remove_file(object));
        drop(std::fs::remove_file(pair));
    }

    #[test]
    fn native_back_end_builds_runs_and_refuses_clearly() {
        let Some(target) = host() else {
            eprintln!("skipping: the host is not a supported target");
            return;
        };
        let target = format!("--target={target}");
        let path = |path: &Path| path.to_str().unwrap().to_owned();
        let executable = temporary("native-hello.exe");
        for flags in [
            &[][..],
            &["--opt=bcc"][..],
            &["--native-object"][..],
            &["--native-object", "--opt=bcc"][..],
        ] {
            let built = bcc(
                HELLO,
                &[
                    &[&target[..], "--backend=native", "-o", &path(&executable)][..],
                    flags,
                ]
                .concat(),
            );
            if built.status.code() == Some(1) && text(&built.stderr).contains("cannot find") {
                eprintln!("skipping: {}", text(&built.stderr));
                return;
            }
            assert!(built.status.success(), "{flags:?}: {}", text(&built.stderr));
            let run = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(run.status.code(), Some(3), "{flags:?}");
            assert_eq!(text(&run.stdout), "answer 42\n", "{flags:?}");
        }
        let pair = temporary("native-pair.exe");
        let built = bcc(PAIR, &[&target, "--backend=native", "-o", &path(&pair)]);
        assert!(built.status.success(), "{}", text(&built.stderr));
        let run = run_native(&mut Command::new(&pair)).unwrap();
        assert_eq!(run.status.code(), Some(42));
        for flags in [&[][..], &["--native-object"][..]] {
            let object = temporary("native-hello.o");
            let linked = temporary("native-object-hello.exe");
            assert!(!object.exists());
            let compiled = bcc(
                HELLO,
                &[
                    &[&target[..], "--backend=native", "-c", "-o", &path(&object)][..],
                    flags,
                ]
                .concat(),
            );
            assert!(compiled.status.success(), "{}", text(&compiled.stderr));
            let bytes = std::fs::read(&object).unwrap();
            if target.contains("linux") {
                assert!(bytes.starts_with(b"\x7fELF\x02\x01\x01"));
                assert!(bytes.windows(6).any(|s| s == b".text\0"));
                assert!(bytes.windows(5).any(|s| s == b"main\0"));
            }
            let clang = Path::new(env!("CARGO_MANIFEST_DIR")).join(if cfg!(windows) {
                "target/llvm/bin/clang.exe"
            } else {
                "target/llvm/bin/clang"
            });
            let result = Command::new(clang)
                .arg(&target)
                .arg(&object)
                .arg("-o")
                .arg(&linked)
                .output()
                .unwrap();
            assert!(result.status.success(), "{}", text(&result.stderr));
            let run = run_native(&mut Command::new(&linked)).unwrap();
            assert_eq!(run.status.code(), Some(3));
            assert_eq!(text(&run.stdout), "answer 42\n");
            std::fs::remove_file(object).unwrap();
            std::fs::remove_file(linked).unwrap();
        }
        for file in [executable, pair] {
            drop(std::fs::remove_file(file));
        }

        // `long double` goes through the x87 unit.
        let float = temporary("native-float.exe");
        let built = bcc(
            "long double half(long double x) { return x / 2; }\nint main(void) { return half(4) \
             != 2; }\n",
            &[&target, "--backend=native", "-o", &path(&float)],
        );
        assert!(built.status.success(), "{}", text(&built.stderr));
        let run = run_native(&mut Command::new(&float)).unwrap();
        assert_eq!(run.status.code(), Some(0));
        drop(std::fs::remove_file(&float));

        // Unsupported code is refused before anything is written.
        let refused = bcc(
            "int pick(__int128 x) { switch (x) { case 1: return 2; default: return 3; } }\nint \
             main(void) { return pick(1) != 2; }\n",
            &[&target, "--backend=native", "-o", &path(&float)],
        );
        assert_eq!(refused.status.code(), Some(1));
        assert_eq!(
            text(&refused.stderr).trim(),
            "error: not yet supported by the native back end: a switch on an i128 value (in @pick)"
        );
        assert!(!float.exists());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn native_named_array_and_bss_sections_run() {
        let source = r#"int printf(const char *, ...);
static void hello(void) { printf("ctor\n"); }
static void goodbye(void) { printf("fini\n"); }
__attribute__((section(".init_array"), used)) static void (*p)(void) = hello;
__attribute__((section(".fini_array.101"), used)) static void (*q)(void) = goodbye;
__attribute__((section(".bss.foo"))) int zero;
int main(void) { printf("main\n"); return zero; }
"#;
        for flags in [
            &[][..],
            &["--opt=bcc"][..],
            &["--native-object"][..],
            &["--native-object", "--opt=bcc"][..],
        ] {
            let executable = temporary("native-named-sections.exe");
            let built = bcc(
                source,
                &[
                    &[
                        "--target=x86_64-unknown-linux-gnu",
                        "--backend=native",
                        "-o",
                        executable.to_str().unwrap(),
                    ][..],
                    flags,
                ]
                .concat(),
            );
            assert!(built.status.success(), "{flags:?}: {}", text(&built.stderr));
            let run = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(run.status.code(), Some(0));
            assert_eq!(text(&run.stdout), "ctor\nmain\nfini\n");
            std::fs::remove_file(executable).unwrap();
        }
    }

    #[test]
    fn native_assembly_and_option_conflicts() {
        let asm = bcc(
            SUM,
            &[
                "--target=x86_64-unknown-linux-gnu",
                "--backend=native",
                "--emit=asm",
            ],
        );
        assert!(asm.status.success(), "{}", text(&asm.stderr));
        let listing = text(&asm.stdout);
        assert!(listing.contains("\t.globl\tmain\n"), "{listing}");
        assert!(listing.contains(".cfi_startproc"), "{listing}");
        let asm = bcc(
            SUM,
            &[
                "--target=x86_64-w64-windows-gnu",
                "--backend=native",
                "--emit=asm",
            ],
        );
        assert!(text(&asm.stdout).contains(".seh_proc\tmain"));
        for (flags, message) in [
            (
                &["--native-object", "--emit=ir"][..],
                "error: --native-object requires --backend=native",
            ),
            (
                &["--emit=asm"][..],
                "error: --emit=asm prints the native back end's assembly; add --backend=native",
            ),
            (
                &["--backend=native", "--emit=llvm"][..],
                "error: --emit=llvm prints LLVM IR, which --backend=native does not use",
            ),
            (
                &["--backend=native", "--interpret"][..],
                "error: --backend=native needs -o or --emit=asm; --interpret runs the IR",
            ),
            (
                &["--backend=native", "--emit=asm", "--llvm-opt=O2"][..],
                "error: --llvm-opt and --llvm-codegen-only configure the LLVM back end, not \
                 --backend=native",
            ),
        ] {
            let output = bcc(SUM, flags);
            assert_eq!(output.status.code(), Some(1), "{flags:?}");
            assert_eq!(text(&output.stderr).trim(), message, "{flags:?}");
            assert!(output.stdout.is_empty(), "{flags:?}");
        }
    }

    #[test]
    fn llvm_diagnostic_errors_fail_cli_without_an_output_file() {
        // This static body is retained by a call, rather than `used`, so it
        // reaches LLVM's MC diagnostics instead of the early semantic check.
        let source = "static __attribute__((noinline,section(\".bss.z\"))) int f(int x) { return \
                      x+1; }\nstatic __attribute__((noinline,section(\".bss.second\"))) int h(int \
                      x) { return x+2; }\nint g(int x) { return f(x) + h(x); }\n";
        let object = temporary("llvm-diagnostic-refused.o");
        std::fs::write(&object, b"stale object").unwrap();
        let output = bcc(
            source,
            &[
                "--target=x86_64-unknown-linux-gnu",
                "--opt=none",
                "--llvm-opt=O0",
                "-c",
                "-o",
                object.to_str().unwrap(),
            ],
        );
        assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
        assert!(
            text(&output.stderr).contains("BSS section '.bss.z' cannot have non-zero bytes"),
            "{}",
            text(&output.stderr)
        );
        assert!(
            text(&output.stderr).contains("BSS section '.bss.second' cannot have non-zero bytes"),
            "{}",
            text(&output.stderr)
        );
        assert!(!object.exists(), "an LLVM error wrote an object");
        // Installing the diagnostic callback must preserve LLVM's filtering
        // of optimization remarks on ordinary successful compilations.
        let valid = bcc(
            "int value(void) { return 3; }",
            &[
                "--target=x86_64-unknown-linux-gnu",
                "--opt=none",
                "--llvm-opt=O0",
                "-c",
                "-o",
                object.to_str().unwrap(),
            ],
        );
        assert!(valid.status.success(), "{}", text(&valid.stderr));
        assert!(valid.stderr.is_empty(), "{}", text(&valid.stderr));
        assert!(object.exists());
        std::fs::remove_file(object).unwrap();
    }

    #[test]
    fn bss_initializer_errors_fail_all_backends_without_an_output_file() {
        let source = "int a __attribute__((section(\".bss.z\"))) = 4;\nint b \
                      __attribute__((section(\".gnu.linkonce.tb.z\"))) = 5;\n";
        for backend in [
            &[][..],
            &["--backend=native"][..],
            &["--backend=native", "--native-object"][..],
        ] {
            let object = temporary("bss-refused.o");
            std::fs::write(&object, b"stale object").unwrap();
            let output_path = object.to_str().unwrap();
            let mut flags = vec!["--target=x86_64-unknown-linux-gnu", "-c", "-o", output_path];
            flags.extend_from_slice(backend);
            let output = bcc(source, &flags);
            assert_eq!(
                output.status.code(),
                Some(1),
                "{backend:?}: {}",
                text(&output.stderr)
            );
            assert!(
                !object.exists(),
                "a failed compilation wrote an object: {backend:?}"
            );
            let stderr = text(&output.stderr);
            assert!(
                stderr.contains("BSS section '.bss.z' cannot have non-zero bytes"),
                "{stderr}"
            );
            assert!(
                stderr.contains("BSS section '.gnu.linkonce.tb.z' cannot have non-zero bytes"),
                "{stderr}"
            );
            assert!(stderr.contains(":1:") && stderr.contains(":2:"), "{stderr}");
        }
    }

    #[test]
    fn failed_compilation_removes_stale_object_files() {
        let cases = [
            ("preprocessing", "#error deliberate failure\n"),
            ("parsing", "int value = ;\n"),
            ("semantics", "int value = not_declared;\n"),
            ("lowering", "int value(void) { __asm__(\"\"); return 0; }\n"),
            (
                "backend",
                "static __attribute__((noinline,section(\".bss.fn\"))) int f(int x) { return x+1; \
                 } int g(int x) { return f(x); }\n",
            ),
        ];
        for backend in [
            &[][..],
            &["--backend=native"][..],
            &["--backend=native", "--native-object"][..],
        ] {
            for (kind, source) in cases {
                let object = temporary("stale-object.o");
                std::fs::write(&object, b"stale object").unwrap();
                let mut flags = vec![
                    "--target=x86_64-unknown-linux-gnu",
                    "-c",
                    "-o",
                    object.to_str().unwrap(),
                ];
                flags.extend_from_slice(backend);
                let output = bcc(source, &flags);
                assert_eq!(
                    output.status.code(),
                    Some(1),
                    "{kind} {backend:?}: {}",
                    text(&output.stderr)
                );
                assert!(!object.exists(), "{kind} {backend:?} left stale output");
            }
        }
    }

    #[test]
    fn front_end_failure_preserves_stale_executable_like_clang() {
        for backend in [
            &[][..],
            &["--backend=native"][..],
            &["--backend=native", "--native-object"][..],
        ] {
            let executable = temporary("stale-executable");
            std::fs::write(&executable, b"existing executable").unwrap();
            let mut flags = vec![
                "--target=x86_64-unknown-linux-gnu",
                "-o",
                executable.to_str().unwrap(),
            ];
            flags.extend_from_slice(backend);
            let output = bcc("int value = not_declared;", &flags);
            assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
            assert_eq!(std::fs::read(&executable).unwrap(), b"existing executable");
            std::fs::remove_file(executable).unwrap();
        }
    }

    #[test]
    fn null_pointer_arithmetic_emits_zero_data_without_fixups() {
        for backend in [
            &[][..],
            &["--backend=native"][..],
            &["--backend=native", "--native-object"][..],
        ] {
            let object = temporary("null-pointer-arithmetic.o");
            let mut flags = vec![
                "--target=x86_64-unknown-linux-gnu",
                "-c",
                "-o",
                object.to_str().unwrap(),
            ];
            flags.extend_from_slice(backend);
            let output = bcc(
                "char *a __attribute__((section(\".bss.a\"))) = (char *)0 + 0;\nint *b \
                 __attribute__((section(\".bss.b\"))) = 0 + (int *)0;\nchar *c = (char *)0 - 0;\n",
                &flags,
            );
            assert!(
                output.status.success(),
                "{backend:?}: {}",
                text(&output.stderr)
            );
            assert!(output.stderr.is_empty(), "{}", text(&output.stderr));
            assert!(object.exists());
            if cfg!(target_os = "linux") {
                let relocations = Command::new("readelf")
                    .arg("-r")
                    .arg(&object)
                    .output()
                    .unwrap();
                assert!(relocations.status.success());
                assert!(
                    text(&relocations.stdout).contains("There are no relocations"),
                    "{}",
                    text(&relocations.stdout)
                );
            }
            std::fs::remove_file(object).unwrap();
        }
    }

    #[test]
    fn windows_gnu_object_output_needs_no_mingw() {
        let object = temporary("no-mingw.o");
        let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(["--target=x86_64-pc-windows-gnu", "-c", "--input", SUM, "-o"])
            .arg(&object)
            .env_remove("MINGW_ROOT")
            .env("PATH", "")
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", text(&output.stderr));
        let bytes = std::fs::read(&object).unwrap();
        assert!(bytes.len() > 20, "object has no COFF header");
        assert_eq!(&bytes[..2], &[0x64, 0x86], "object is x86-64 COFF");
        std::fs::remove_file(object).unwrap();
    }

    /// The MinGW-w64 installation root and its C library include directory,
    /// from `MINGW_ROOT` or a `gcc.exe` on `PATH`, as the build finds them.
    fn mingw() -> Option<(PathBuf, PathBuf)> {
        let installation = |root: &Path| {
            ["include", "x86_64-w64-mingw32/include"]
                .into_iter()
                .map(|relative| root.join(relative))
                .find(|include| {
                    include.join("_mingw.h").is_file() && include.join("windows.h").is_file()
                })
                .map(|include| (root.to_path_buf(), include))
        };
        if let Some(root) = env::var_os("MINGW_ROOT") {
            return installation(Path::new(&root));
        }
        let path = env::var_os("PATH")?;
        env::split_paths(&path).find_map(|directory| {
            let root = directory.parent()?;
            directory
                .join("gcc.exe")
                .is_file()
                .then(|| installation(root))
                .flatten()
        })
    }

    /// GitHub issue #57: MinGW-w64's `_mingw.h` declares its `__CRT_INLINE`
    /// helpers `extern inline __attribute__((__gnu_inline__))` only for GCC
    /// 4.3 or later or for Clang. bcc claims Clang, so two translation units
    /// that include `<string.h>` emit no external `strncasecmp` and link.
    #[test]
    fn mingw_crt_inline_helpers_link_from_two_translation_units() {
        if host() != Some("x86_64-w64-windows-gnu") {
            eprintln!("skipping: the host is not Windows GNU");
            return;
        }
        let Some((root, include)) = mingw() else {
            eprintln!("skipping: no MinGW-w64 installation in MINGW_ROOT or PATH");
            return;
        };
        let units = [
            "#include <string.h>\nint one(void) { return strcasecmp(\"a\", \"A\"); }\n",
            "#include <stdio.h>\n#include <string.h>\nint one(void);\nint main(void) {\n  \
             printf(\"%d %d\\n\", one(), strncasecmp(\"ab\", \"AB\", 2));\n  return 0;\n}\n",
        ];
        let flags = [
            "--target=x86_64-w64-windows-gnu",
            "--idirafter",
            include.to_str().unwrap(),
        ];
        let mut objects = Vec::new();
        for (index, unit) in units.iter().enumerate() {
            let llvm = bcc(unit, &[&flags[..], &["--emit=llvm"]].concat());
            assert!(llvm.status.success(), "{}", text(&llvm.stderr));
            let module = text(&llvm.stdout);
            assert!(
                !module.lines().any(|line| line.starts_with("define")
                    && (line.contains("@strcasecmp(") || line.contains("@strncasecmp("))),
                "unit {index} defines a CRT inline helper:\n{module}"
            );
            let object = temporary(&format!("crt-inline-{index}.o"));
            let compiled = bcc(
                unit,
                &[&flags[..], &["-c", "-o", object.to_str().unwrap()]].concat(),
            );
            assert!(compiled.status.success(), "{}", text(&compiled.stderr));
            objects.push(object);
        }
        let executable = temporary("crt-inline.exe");
        let linked =
            Command::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("target/llvm/bin/clang.exe"))
                .arg("--target=x86_64-w64-windows-gnu")
                .arg(format!("--sysroot={}", root.display()))
                .arg("-fuse-ld=lld")
                .args(&objects)
                .arg("-o")
                .arg(&executable)
                .output()
                .unwrap();
        assert!(linked.status.success(), "{}", text(&linked.stderr));
        let run = run_native(&mut Command::new(&executable)).unwrap();
        assert_eq!(run.status.code(), Some(0));
        assert_eq!(text(&run.stdout), "0 0\n");
        for path in objects.into_iter().chain([executable]) {
            drop(std::fs::remove_file(path));
        }
    }

    #[test]
    fn looping_native_program_reports_timeout() {
        let Some(target) = host() else {
            return;
        };
        let executable = temporary("loop.exe");
        // A volatile access keeps the deliberate infinite loop defined in C.
        let built = bcc(
            "int printf(const char *, ...); int fflush(void *); int main(void) { volatile int x = \
             0; printf(\"loop started\\n\"); fflush(0); for (;;) x = 0; }",
            &[
                &format!("--target={target}"),
                "-o",
                executable.to_str().unwrap(),
            ],
        );
        assert!(built.status.success(), "{}", text(&built.stderr));
        let report = run_until_timeout(&mut Command::new(&executable), "loop started");
        assert!(report.contains("native execution timed out"), "{report}");
        assert!(report.contains("loop.exe"), "{report}");
        std::fs::remove_file(executable).unwrap();
    }

    #[test]
    fn native_output_exceeding_pipe_capacity_is_captured() {
        let Some(target) = host() else {
            return;
        };
        let executable = temporary("large-output.exe");
        let built = bcc(
            "int putchar(int); int main(void) { for (int i = 0; i < 200000; i++) putchar('x'); \
             return 7; }",
            &[
                &format!("--target={target}"),
                "-o",
                executable.to_str().unwrap(),
            ],
        );
        assert!(built.status.success(), "{}", text(&built.stderr));
        let output = run_native(&mut Command::new(&executable)).unwrap();
        assert_eq!(output.status.code(), Some(7));
        assert_eq!(output.stdout, vec![b'x'; 200_000]);
        assert!(output.stderr.is_empty(), "{}", text(&output.stderr));
        std::fs::remove_file(executable).unwrap();
    }
}
