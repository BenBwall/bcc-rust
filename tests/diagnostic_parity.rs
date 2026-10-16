#![expect(
    missing_docs,
    unused_crate_dependencies,
    reason = "Integration tests exercise the compiler and its pinned Clang oracle."
)]

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "The process harness owns source strings, flags, and captured output."
)]
mod tests {
    use std::{
        io::Write,
        path::PathBuf,
        process::{
            Command,
            Output,
            Stdio,
        },
    };

    fn bcc(source: &str, flags: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(flags)
            .args(["--semantic-types", "--input", source])
            .env("NO_COLOR", "1")
            .env_remove("CPATH")
            .env_remove("C_INCLUDE_PATH")
            .output()
            .unwrap()
    }

    fn clang(source: &str, flags: &[&str]) -> Output {
        let executable = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/llvm/bin")
            .join(format!("clang{}", std::env::consts::EXE_SUFFIX));
        let mut child = Command::new(executable)
            .args(["-fsyntax-only", "-x", "c", "-"])
            .args(flags)
            .env_remove("CPATH")
            .env_remove("C_INCLUDE_PATH")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(source.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    #[test]
    fn dll_attribute_spelling_target_and_warning_groups_match_clang() {
        for target in [
            "--target=x86_64-unknown-linux-gnu",
            "--target=x86_64-unknown-linux-musl",
            "--target=x86_64-w64-windows-gnu",
            "--target=x86_64-pc-windows-msvc",
        ] {
            let windows = target.contains("windows");
            for name in ["dllimport", "dllexport", "__dllimport__", "__dllexport__"] {
                for (source, group, other) in [
                    (
                        format!("__declspec({name}) extern int value;\n"),
                        "ignored-attributes",
                        "unknown-attributes",
                    ),
                    (
                        format!("__attribute__(({name})) extern int value;\n"),
                        "unknown-attributes",
                        "ignored-attributes",
                    ),
                ] {
                    // Clang ignores wrapped declspec names on Windows;
                    // bcc's existing normalization there is separate policy.
                    if windows && name.starts_with("__") && group == "ignored-attributes" {
                        continue;
                    }
                    let tag = format!("-W{group}]");
                    for (option, warning, error) in [
                        (String::new(), !windows, false),
                        (format!("-Wno-{group}"), false, false),
                        (format!("-Wno-{other}"), !windows, false),
                        (format!("-Werror={group}"), false, !windows),
                    ] {
                        let mut flags = vec!["-std=c17", "-fms-extensions", target];
                        if !option.is_empty() {
                            flags.push(option.as_str());
                        }
                        for output in [bcc(&source, &flags), clang(&source, &flags)] {
                            let text = String::from_utf8_lossy(&output.stderr);
                            assert_eq!(
                                output.status.success(),
                                !error,
                                "{source} {flags:?}: {text}"
                            );
                            assert_eq!(
                                text.contains("warning:"),
                                warning,
                                "{source} {flags:?}: {text}"
                            );
                            assert_eq!(
                                text.contains("error:"),
                                error,
                                "{source} {flags:?}: {text}"
                            );
                            assert_eq!(
                                text.contains(&tag),
                                warning || error,
                                "{source} {flags:?}: {text}"
                            );
                        }
                    }
                }
            }
            // Policy for an unsupported declspec does not make GNU/query
            // spellings available outside Windows.
            let available = usize::from(windows);
            let source = format!(
                "#if __has_attribute(dllimport) != {available} || __has_attribute(dllexport) != \
                 {available} || __has_c_attribute(gnu::dllimport) != {available} || \
                 __has_c_attribute(gnu::dllexport) != {available}\n#error wrong attribute \
                 availability\n#endif\nint value;\n"
            );
            let flags = ["-std=c17", "-fms-extensions", target];
            for output in [bcc(&source, &flags), clang(&source, &flags)] {
                let text = String::from_utf8_lossy(&output.stderr);
                assert!(output.status.success(), "{target}: {text}");
                assert!(!text.contains("warning:"), "{target}: {text}");
            }
        }
    }

    #[test]
    fn issue54_constraints_and_warning_groups_match_pinned_clang() {
        for (source, standard, group, error) in [
            ("void f(void){int i; i=nullptr;}\n", "c23", "", true),
            ("void f(void *p){&*p; &(*p);}\n", "c17", "", false),
            (
                "void f(void){typeof(nullptr) a=0,b=(void*)0; nullptr==0;}\n",
                "c23",
                "",
                false,
            ),
            ("void f(void){int i; i==nullptr;}\n", "c23", "", true),
            ("void f(int *p){p<nullptr;}\n", "c23", "", true),
            ("void f(int *p){typeof(nullptr) n=p;}\n", "c23", "", true),
            (
                "void f(void){bool b=nullptr;}\n",
                "c23",
                "null-conversion",
                false,
            ),
            (
                "void f(int *p){p=(short)0;}\n",
                "c17",
                "non-literal-null-conversion",
                false,
            ),
            (
                "enum {ZERO=0}; void f(int *p){p=ZERO;}\n",
                "c17",
                "non-literal-null-conversion",
                false,
            ),
            (
                "void f(int *p){p=='\\0';}\n",
                "c17",
                "pointer-compare",
                false,
            ),
            (
                "void f(void){int i; _Bool b=&i;}\n",
                "c17",
                "pointer-bool-conversion",
                false,
            ),
            (
                "void f(int *p){(int)p;}\n",
                "c17",
                "pointer-to-int-cast",
                false,
            ),
            (
                "void f(void *p){*p;}\n",
                "c17",
                "void-ptr-dereference",
                false,
            ),
            (
                "void f(int *) __attribute__((nonnull(1))); void g(void){f(0);}\n",
                "c17",
                "nonnull",
                false,
            ),
            (
                "int kr(p) int *p;{return *p;} void f(unsigned u){kr(&u);}\n",
                "c17",
                "pointer-sign",
                false,
            ),
        ] {
            let standard = format!("-std={standard}");
            let flags = [standard.as_str()];
            let expected = clang(source, &flags);
            let actual = bcc(source, &flags);
            let expected = String::from_utf8_lossy(&expected.stderr);
            let actual = String::from_utf8_lossy(&actual.stderr);
            assert_eq!(actual.contains("error:"), error, "bcc: {source}: {actual}");
            assert_eq!(
                expected.contains("error:"),
                error,
                "Clang: {source}: {expected}"
            );
            if !group.is_empty() {
                let tag = format!("[-W{group}]");
                assert!(
                    actual.contains(&tag) && expected.contains(&tag),
                    "{source}\nbcc: {actual}\nClang: {expected}"
                );
                let suppressed = format!("-Wno-{group}");
                assert!(
                    !String::from_utf8_lossy(&bcc(source, &[&standard, &suppressed]).stderr)
                        .contains(&tag)
                );
                let promoted = format!("-Werror={group}");
                assert!(
                    String::from_utf8_lossy(&bcc(source, &[&standard, &promoted]).stderr)
                        .contains("error:")
                );
            }
        }
    }

    #[test]
    fn issue49_static_function_designator_initializer_matches_clang_policy() {
        let source = include_str!(
            "../docs/research/conformance-reproducers-2026-10-13/\
             function_to_void_pointer_initializer.c"
        );
        for (flags, error, tag) in [
            (vec!["-std=c17"], false, false),
            (vec!["-std=c17", "-pedantic"], false, true),
            (vec!["-std=c17", "-pedantic-errors"], true, true),
        ] {
            for output in [bcc(source, &flags), clang(source, &flags)] {
                let text = String::from_utf8_lossy(&output.stderr);
                assert_eq!(text.contains("error:"), error, "{flags:?}: {text}");
                // Clang uses -Wpedantic for the function/void conversion.
                assert_eq!(text.contains("-Wpedantic]"), tag, "{flags:?}: {text}");
            }
        }
    }

    #[test]
    fn c89_func_and_gnu_subtraction_match_clang_acceptance() {
        for (source, flags) in [
            (
                "int f(void){return __func__[0]!='f';}\n",
                vec!["-std=c89", "-pedantic-errors"],
            ),
            (
                "void f(void); long g(void *p){return (p-p)+(f-f);}\n",
                vec!["-std=c17"],
            ),
        ] {
            for output in [bcc(source, &flags), clang(source, &flags)] {
                let text = String::from_utf8_lossy(&output.stderr);
                assert!(
                    !text.contains("error:") && !text.contains("warning:"),
                    "{source}: {text}"
                );
            }
        }
    }

    #[test]
    fn embedded_directives_and_unknown_stdc_pragmas_match_clang() {
        let source = "#define ID(x) x\nint x=ID(\n#define VALUE 42\nVALUE);\n";
        for output in [bcc(source, &[]), clang(source, &[])] {
            let text = String::from_utf8_lossy(&output.stderr);
            assert!(
                !text.contains("error:") && !text.contains("warning:"),
                "{text}"
            );
        }
        for flags in [vec![], vec!["-Werror"], vec!["-Wno-unknown-pragmas"]] {
            let source = "#pragma STDC FOO\nint following;\n";
            let expected = clang(source, &flags);
            let actual = bcc(source, &flags);
            let expected = String::from_utf8_lossy(&expected.stderr);
            let actual = String::from_utf8_lossy(&actual.stderr);
            assert_eq!(
                actual.contains("error:"),
                expected.contains("error:"),
                "{flags:?}: {actual}\n{expected}"
            );
            assert_eq!(
                actual.contains("-Wunknown-pragmas]"),
                expected.contains("-Wunknown-pragmas]"),
                "{flags:?}: {actual}\n{expected}"
            );
        }
    }

    #[test]
    fn warning_options_are_accepted_and_unknown_options_warn() {
        for flag in ["-Wall", "-Wextra", "-Wconversion", "-Wnot-a-real-warning"] {
            let output = bcc("int x;\n", &[flag]);
            let expected = clang("int x;\n", &[flag]);
            assert!(output.status.success(), "{output:?}");
            assert_eq!(
                String::from_utf8_lossy(&output.stderr).contains("[-Wunknown-warning-option]"),
                String::from_utf8_lossy(&expected.stderr).contains("[-Wunknown-warning-option]")
            );
        }
        let output = bcc(
            "int x;\n",
            &["-Wnot-a-real-warning", "-Wno-unknown-warning-option"],
        );
        assert!(output.status.success());
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains("[-Wunknown-warning-option]"),
            "{output:?}"
        );
        assert_eq!(
            bcc("int x;\n", &["-Werror", "-Wnot-a-real-warning"])
                .status
                .code(),
            Some(1)
        );
    }
    #[test]
    fn aggregate_warning_controls_preserve_default_groups() {
        let source = "void f(int *p){p=(short)0; int x; _Bool b=&x;}\n";
        for flags in [
            vec!["-Wno-all"],
            vec!["-Wno-extra"],
            vec!["-Wno-conversion"],
        ] {
            let actual = bcc(source, &flags);
            let expected = clang(source, &flags);
            for group in ["non-literal-null-conversion", "pointer-bool-conversion"] {
                let tag = format!("[-W{group}]");
                assert_eq!(
                    String::from_utf8_lossy(&actual.stderr).contains(&tag),
                    String::from_utf8_lossy(&expected.stderr).contains(&tag),
                    "{flags:?}: {actual:?}"
                );
            }
        }
    }

    fn primary_location(text: &str, prefix: &str) -> (usize, usize) {
        let location = text
            .lines()
            .find_map(|line| line.trim().strip_prefix(prefix))
            .unwrap_or_else(|| panic!("no location {prefix}: {text}"));
        let mut parts = location.split(':');
        (
            parts.next().unwrap().parse().unwrap(),
            parts.next().unwrap().parse().unwrap(),
        )
    }

    #[test]
    fn assignment_initializer_and_macro_locations_match_clang() {
        for source in [
            "void f(char *p,float *q){p=q;}\n",
            "void f(float *q){char *p=q;}\n",
            "#define BAD *(1)\nvoid f(void){BAD;}\n",
            "#define ID(x) x\nvoid f(void){ID(*1);}\n",
            "#define BAD p=q\nvoid f(char *p,float *q){BAD;}\n",
        ] {
            let actual = bcc(source, &[]);
            let expected = clang(source, &[]);
            let actual = String::from_utf8_lossy(&actual.stderr);
            let expected = String::from_utf8_lossy(&expected.stderr);
            assert_eq!(
                primary_location(&actual, "--> <input>:"),
                primary_location(&expected, "<stdin>:"),
                "{source}\nbcc: {actual}\nClang: {expected}"
            );
        }
    }
}
