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
    reason = "Test harnesses own inputs, paths and process output outside compilation."
)]
mod tests {
    use std::{
        path::Path,
        process::{
            Command,
            Output,
        },
    };

    const ACCEPTED: &str = "int x=7; unsigned *u; const int **q; int *p;\nint main(void) { int \
                            same; p=&x; u=p; same=(p==u); q=&p; return !same || **q!=7; }\n";
    const ARGUMENT: &str = "int inspect(char *p) { return (unsigned char)p[2]==128 && (unsigned \
                            char)p[3]==63; }\nint main(void) { float x=1.0f; return \
                            inspect(&x)-1; }\n";

    /// Conditional mismatches, enum signs and array-element qualifiers that
    /// Clang accepts with warnings (or silently) and bcc used to reject.
    const MISMATCHES: &str = concat!(
        "enum N { M = -1 }; int x = 7; unsigned ux = 9; int a[2][3] = {{1,2,3},{4,5,6}};\n",
        "static int sum(const int (*m)[3], int n) {\n",
        "    int s = 0;\n",
        "    for (int i = 0; i < n; i++) for (int j = 0; j < 3; j++) s += m[i][j];\n",
        "    return s;\n",
        "}\n",
        "int main(void) {\n",
        "    int *ip = &x; unsigned *up = &ux; enum N n = M; enum N *pn = &n; unsigned *pu = pn;\n",
        "    int c = x == 7; int *chosen = c ? ip : up; long l = 0; int *q = c ? ip : l;\n",
        "    const int (*cpa)[3] = a;\n",
        "    return !(*chosen == 7 && q == ip && *pu == 0xffffffffu\n",
        "        && sum(a, 2) == 21 && cpa == a);\n",
        "}\n",
    );
    /// Addresses in pointer-wide integers of static storage, accepted when
    /// int-conversion is not an error, as in Clang.
    const STATIC_ADDRESSES: &str = concat!(
        "int x = 5; static int arr[4] = {1, 2, 3, 4};\n",
        "long long s = &x; long long t = (long long)&x;\n",
        "unsigned long long u[2] = { &arr[1], &arr[3] };\n",
        "int main(void) {\n",
        "    static long long local = &arr[2];\n",
        "    return !(s == (long long)&x && t == s && (int *)u[1] - (int *)u[0] == 2\n",
        "        && *(int *)s == 5 && *(int *)local == 3);\n",
        "}\n",
    );

    fn bcc(source: &str, flags: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(flags)
            .args(["--input", source])
            .env("NO_COLOR", "1")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("CPATH")
            .env_remove("C_INCLUDE_PATH")
            .output()
            .unwrap()
    }

    fn check(source: &str, flags: &[&str]) -> Output {
        // Inspection views historically return success even with diagnostics.
        // Code generation pins the compiler's error status and lowering gate.
        bcc(source, &[flags, &["--emit=ir"]].concat())
    }

    #[test]
    fn warning_groups_control_acceptance_and_keep_default_errors() {
        for (source, group, warning) in [
            (
                "int f(int *p,unsigned *q){return p==q;}\n",
                "compare-distinct-pointer-types",
                true,
            ),
            ("void f(int *p,unsigned *q){p=q;}\n", "pointer-sign", true),
            (
                "void f(const int **p,int **q){p=q;}\n",
                "incompatible-pointer-types-discards-qualifiers",
                true,
            ),
            (
                "void f(char *p,float *q){p=q;}\n",
                "incompatible-pointer-types",
                false,
            ),
            (
                "void f(int (*p)(int),int (*q)(double)){p=q;}\n",
                "incompatible-function-pointer-types",
                false,
            ),
            ("void f(int *p,int q){p=q;}\n", "int-conversion", false),
            (
                "int f(int *p,int q){return p==q;}\n",
                "pointer-integer-compare",
                true,
            ),
            (
                "int f(int (*p)(int),int (*q)(int)){return p<q;}\n",
                "ordered-compare-function-pointers",
                true,
            ),
        ] {
            for (flags, success, diagnostic) in [
                (vec![], warning, true),
                (vec!["-Werror".to_owned()], false, true),
                (vec!["-pedantic-errors".to_owned()], false, true),
                (vec![format!("-Werror={group}")], false, true),
                (vec![format!("-Wno-{group}")], true, false),
                (
                    vec![format!("-Wno-{group}"), format!("-W{group}")],
                    true,
                    true,
                ),
                (
                    vec![
                        "-Werror".to_owned(),
                        format!("-Wno-{group}"),
                        format!("-W{group}"),
                    ],
                    false,
                    true,
                ),
                (
                    vec!["-Werror".to_owned(), format!("-Wno-error={group}")],
                    true,
                    true,
                ),
                (
                    vec![format!("-Wno-error={group}"), "-Werror".to_owned()],
                    true,
                    true,
                ),
                (
                    vec!["-w".to_owned(), "-Werror".to_owned()],
                    warning,
                    !warning,
                ),
            ] {
                let flags: Vec<_> = flags.iter().map(String::as_str).collect();
                let output = check(source, &flags);
                let stderr = String::from_utf8_lossy(&output.stderr);
                assert_eq!(
                    output.status.success(),
                    success,
                    "{group} {flags:?}: {stderr}"
                );
                assert_eq!(
                    stderr.contains(&format!("[-W{group}]")),
                    diagnostic,
                    "{flags:?}: {stderr}"
                );
            }
        }
    }

    #[test]
    fn parent_group_controls_children_and_later_child_options_win() {
        let source = "void f(const int **p,int **q){p=q;}\n";
        assert_eq!(
            check(source, &["-Wno-incompatible-pointer-types"]).stderr,
            b""
        );
        let output = check(
            source,
            &[
                "-Wno-incompatible-pointer-types",
                "-Werror=incompatible-pointer-types-discards-qualifiers",
            ],
        );
        assert_eq!(output.status.code(), Some(1));
        let output = check(
            source,
            &[
                "-Werror=incompatible-pointer-types-discards-qualifiers",
                "-Wno-incompatible-pointer-types",
            ],
        );
        assert!(output.status.success());
        assert_eq!(output.stderr, b"");
    }

    #[test]
    fn distinct_function_order_preserves_both_group_controls() {
        let source = "int f(int (*p)(int),int (*q)(double)){return p<q;}\n";
        for (suppressed, promoted) in [
            (
                "compare-distinct-pointer-types",
                "ordered-compare-function-pointers",
            ),
            (
                "ordered-compare-function-pointers",
                "compare-distinct-pointer-types",
            ),
        ] {
            let suppressed = format!("-Wno-{suppressed}");
            let promoted_flag = format!("-Werror={promoted}");
            let output = check(source, &[&suppressed, &promoted_flag]);
            assert_eq!(output.status.code(), Some(1), "{output:?}");
            assert_eq!(output.stdout, b"");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.contains(&format!("[-W{promoted}]")), "{stderr}");
            assert!(
                !stderr.contains(suppressed.trim_start_matches("-Wno-")),
                "{stderr}"
            );
        }
    }

    #[test]
    fn global_werror_promotes_other_phases_and_blocks_lowering() {
        for source in [
            "#warning test\nint main(void){return 0;}\n",
            "int main(void){ int x; return; }\n",
        ] {
            let output = bcc(source, &["-std=gnu17", "-Werror", "--emit=ir"]);
            assert_eq!(output.status.code(), Some(1), "{output:?}");
            assert!(output.stdout.is_empty(), "{output:?}");
            assert!(String::from_utf8_lossy(&output.stderr).contains("error:"));
        }
    }

    #[test]
    fn accepted_conversions_execute_in_both_interpreters_and_native_arms() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let target = if cfg!(windows) {
            "--target=x86_64-w64-windows-gnu"
        } else {
            "--target=x86_64-unknown-linux-gnu"
        };
        let executable = root.join(format!(
            "target/pointer-conversions-{}.exe",
            std::process::id()
        ));
        for (source, extra) in [
            (ACCEPTED, &[][..]),
            (ARGUMENT, &["-Wno-error=incompatible-pointer-types"][..]),
            (MISMATCHES, &[][..]),
            (STATIC_ADDRESSES, &["-Wno-int-conversion"][..]),
        ] {
            for flags in [&["--interpret"][..], &["--interpret", "--opt=bcc"][..]] {
                let output = bcc(source, &[&[target][..], extra, flags].concat());
                assert!(output.status.success(), "{output:?}");
            }
            for flags in [
                &["--llvm-opt=O0"][..],
                &["--opt=bcc", "--llvm-opt=O2", "--llvm-codegen-only"][..],
                &["--llvm-opt=O2"][..],
            ] {
                let output = bcc(
                    source,
                    &[
                        &[target, "-o", executable.to_str().unwrap()][..],
                        extra,
                        flags,
                    ]
                    .concat(),
                );
                assert!(output.status.success(), "{output:?}");
                let mut command = if cfg!(windows) {
                    let mut command = Command::new("python");
                    _ = command
                        .arg(root.join("scripts/cargo_runner.py"))
                        .arg(&executable);
                    command
                } else {
                    Command::new(&executable)
                };
                let output = super::native::run_native(&mut command).unwrap();
                assert_eq!(output.status.code(), Some(0), "{output:?}");
            }
        }
        std::fs::remove_file(executable).unwrap();
    }
}
