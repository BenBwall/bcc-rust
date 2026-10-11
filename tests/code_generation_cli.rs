#![expect(
    missing_docs,
    unused_crate_dependencies,
    reason = "Integration tests exercise the compiled CLI."
)]

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

    const SUM: &str = "int main(void) {\n  int s = 0;\n  for (int i = 1; i <= 4; i++)\n    s += \
                       i;\n  return s;\n}\n";

    const HELLO: &str = "int printf(const char *, ...);\nint main(void) {\n  printf(\"%s %d\\n\", \
                         \"answer\", 42);\n  return 3;\n}\n";

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

        let unknown = bcc(SUM, &["--emit=ir", "--passes=fold,inline"]);
        assert_eq!(unknown.status.code(), Some(2));
        assert!(text(&unknown.stderr).contains("unknown pass `inline`"));
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
        assert!(stdout.contains("define dso_local i32 @main()"), "{stdout}");
        assert!(stdout.contains("declare i32 @printf(ptr, ...)"), "{stdout}");
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
        let float = "int main(void) {\n  float f = 1;\n  return f;\n}\n";
        for flags in [
            &["--interpret"][..],
            &["--emit=ir"][..],
            &["--emit=llvm"][..],
        ] {
            let output = bcc(float, flags);
            assert_eq!(output.status.code(), Some(1), "{flags:?}");
            assert!(output.stdout.is_empty(), "{flags:?}");
            let stderr = text(&output.stderr);
            assert!(
                stderr.contains("error: not yet supported by lowering: floating-point arithmetic"),
                "{stderr}"
            );
            assert!(stderr.contains("1 error generated."), "{stderr}");
        }
        // Without a back end, the program is valid C and nothing is lowered.
        let output = bcc(float, &[]);
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
        let run = Command::new(&executable).output().unwrap();
        assert_eq!(run.status.code(), Some(3));
        assert_eq!(text(&run.stdout), "answer 42\n");

        let compiled = bcc(HELLO, &[&target, "-c", "-o", &path(&object)]);
        assert!(compiled.status.success(), "{}", text(&compiled.stderr));
        assert!(object.is_file());
        drop(std::fs::remove_file(executable));
        drop(std::fs::remove_file(object));
    }
}
