#![expect(
    missing_docs,
    unused_crate_dependencies,
    reason = "Integration tests exercise the compiled CLI."
)]

#[path = "support/native.rs"]
mod native;

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

    use super::native::{
        run_native,
        run_native_with_timeout,
    };

    const SUM: &str = "int main(void) {\n  int s = 0;\n  for (int i = 1; i <= 4; i++)\n    s += \
                       i;\n  return s;\n}\n";

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
                "int main(void){int n=2;int a[n];return 0;}",
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
        assert!(stdout.contains("declare i32 @printf(ptr, ...)"), "{stdout}");
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
                 aggregate(size 16, align 8, [0: i64, 8: i64])) external"
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
            win64.contains("function @swap(ptr sret(size 16, align 8), ptr) external"),
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
        let vla = "int main(void) {\n  int n = 3;\n  int a[n];\n  a[0] = 1;\n  return a[0];\n}\n";
        for flags in [
            &["--interpret"][..],
            &["--emit=ir"][..],
            &["--emit=llvm"][..],
        ] {
            let output = bcc(vla, flags);
            assert_eq!(output.status.code(), Some(1), "{flags:?}");
            assert!(output.stdout.is_empty(), "{flags:?}");
            let stderr = text(&output.stderr);
            assert!(
                stderr.contains("error: not yet supported by lowering: variable length arrays"),
                "{stderr}"
            );
            assert!(stderr.contains("1 error generated."), "{stderr}");
        }
        // Without a back end, the program is valid C and nothing is lowered.
        let output = bcc(vla, &[]);
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
            &["--opt=bcc"][..],
            &["--llvm-opt=O2"][..],
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
        let start = std::time::Instant::now();
        let error = run_native_with_timeout(
            &mut Command::new(&executable),
            std::time::Duration::from_millis(100),
        )
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(
            error.to_string().contains("native execution timed out"),
            "{error}"
        );
        assert!(error.to_string().contains("loop.exe"), "{error}");
        assert!(error.to_string().contains("loop started"), "{error}");
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "{error}"
        );
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
