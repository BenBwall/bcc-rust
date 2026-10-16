#![expect(
    missing_docs,
    unused_crate_dependencies,
    reason = "CLI emission regressions."
)]

#[path = "support/native.rs"]
mod native;

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Tests own process output and temporary files."
)]
mod tests {
    use std::{
        collections::BTreeSet,
        path::PathBuf,
        process::Command,
    };

    use super::native::run_native;

    #[test]
    fn emitted_symbols_match_clang_o0() {
        let clang = clang();
        let source_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/reachable-symbols.c");
        // BCC defaults to Linux independently of its host; Clang defaults to
        // its native target. Compare explicit targets and their retention
        // metadata so Windows exercises the same symbol contract.
        for (target, used_symbol) in [
            ("x86_64-unknown-linux-gnu", "llvm.compiler.used"),
            ("x86_64-unknown-linux-musl", "llvm.compiler.used"),
            ("x86_64-w64-windows-gnu", "llvm.used"),
            ("x86_64-pc-windows-msvc", "llvm.used"),
        ] {
            let target_flag = format!("--target={target}");
            for (name, source, mode, expected) in [
                (
                    "static",
                    "static int f(void) { return 1; }",
                    "c99",
                    vec!["main"],
                ),
                (
                    "static-inline",
                    "static inline int f(void) { return 1; }",
                    "c99",
                    vec!["main"],
                ),
                (
                    "unused-chain",
                    "static int f(void) { return 1; } static int g(void) { return f(); }",
                    "c99",
                    vec!["main"],
                ),
                (
                    "unused-object",
                    "static int f(void) { return 1; } static int (*p)(void) = f;",
                    "c99",
                    vec!["main"],
                ),
                (
                    "emitted-object",
                    "static int f(void) { return 1; } int (*p)(void) = f;",
                    "c99",
                    vec!["f", "main", "p"],
                ),
                (
                    "gnu89-extern-inline",
                    "extern inline int f(void) { return 1; }",
                    "gnu89",
                    // MSVC keeps mode-selected GNU89 inline definitions;
                    // explicit gnu_inline has its own body-omission rule.
                    if target == "x86_64-pc-windows-msvc" {
                        vec!["f", "main"]
                    } else {
                        vec!["main"]
                    },
                ),
                (
                    "c99-inline",
                    "inline int f(void) { return 1; }",
                    "c99",
                    vec!["main"],
                ),
                (
                    "c99-inline-extern",
                    "inline int f(void) { return 1; } extern int f(void);",
                    "c99",
                    vec!["f", "main"],
                ),
                (
                    "sizeof",
                    "static int f(void) { return 1; } int n = sizeof(&f);",
                    "c99",
                    vec!["main", "n"],
                ),
                (
                    "typeof",
                    "static int f(void) { return 1; } __typeof__(&f) p;",
                    "gnu99",
                    vec!["main", "p"],
                ),
                (
                    "alignof",
                    "static int f(void) { return 1; } int n = __alignof__(f());",
                    "gnu99",
                    vec!["main", "n"],
                ),
                // GNU symbol attributes keep unreferenced internal definitions.
                (
                    "constructor",
                    "__attribute__((constructor)) static void init(void) {}                  \
                     __attribute__((destructor)) static void fini(void) {}",
                    "gnu99",
                    vec![
                        "fini",
                        "init",
                        "llvm.global_ctors",
                        "llvm.global_dtors",
                        "main",
                    ],
                ),
                (
                    "alias",
                    "static int target(void) { return 1; } int other(void)                  \
                     __attribute__((alias(\"target\")));",
                    "gnu99",
                    vec!["main", "other", "target"],
                ),
                (
                    "cleanup",
                    "static void done(int *p) { (void)p; } int run(void) { int x                  \
                     __attribute__((cleanup(done))) = 0; return x; }",
                    "gnu99",
                    vec!["done", "main", "run"],
                ),
                (
                    "used",
                    "__attribute__((used)) static int keep(void) { return 1; }",
                    "gnu99",
                    vec!["keep", used_symbol, "main"],
                ),
            ] {
                let source = format!("{source}\nint main(void) {{ return 0; }}\n");
                std::fs::write(&source_path, &source).unwrap();
                let reference = Command::new(&clang)
                    .arg(&target_flag)
                    .args([
                        "-S",
                        "-emit-llvm",
                        "-O0",
                        &format!("-std={mode}"),
                        "-o",
                        "-",
                    ])
                    .arg(&source_path)
                    .output()
                    .unwrap();
                assert!(
                    reference.status.success(),
                    "{target} {name}: {}",
                    String::from_utf8_lossy(&reference.stderr)
                );
                let output = bcc(
                    &source,
                    &["--emit=llvm", &target_flag, &format!("-std={mode}")],
                );
                assert!(
                    output.status.success(),
                    "{target} {name}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let actual = symbols(&output.stdout);
                assert_eq!(actual, symbols(&reference.stdout), "{target} {name}");
                assert_eq!(
                    actual,
                    expected.into_iter().map(str::to_owned).collect(),
                    "{target} {name}"
                );
            }
        }
        std::fs::remove_file(source_path).unwrap();
    }

    #[test]
    fn unreachable_unsupported_definitions_are_not_lowered() {
        let source = "typedef int V __attribute__((vector_size(16))); static V v(V x) { return x; \
                      }
static int dead(void) { __asm__(\"\"); return 0; }
static int (*unused)(void) = dead;
int main(void) { return 0; }";
        for flags in [&["--interpret"][..], &["--emit=llvm"][..]] {
            let result = bcc(source, flags);
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        for source in [
            source.replace("return 0; }", "return dead(); }"),
            source.replace("static int (*unused)", "int (*unused)"),
            source.replace("static V v", "V v"),
        ] {
            let result = bcc(&source, &["--emit=llvm"]);
            assert_eq!(result.status.code(), Some(1));
            assert!(
                String::from_utf8_lossy(&result.stderr).contains("not yet supported by lowering")
            );
        }
    }

    #[test]
    fn modifiers_in_unreachable_definitions_are_not_checked_by_lowering() {
        for declaration in [
            "static int __vectorcall dead(void) { return 0; }",
            "static int dead(void) { int (__vectorcall *p)(void); return 0; }",
        ] {
            let source = format!("{declaration}\nint main(void) {{ return 0; }}\n");
            let result = bcc(&source, &["--emit=llvm", "-fms-extensions"]);
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            let result = bcc(
                &source
                    .replace("static int dead", "int dead")
                    .replace("static int __vectorcall", "int __vectorcall"),
                &["--emit=llvm", "-fms-extensions"],
            );
            assert_eq!(result.status.code(), Some(1));
            assert!(
                String::from_utf8_lossy(&result.stderr).contains("not yet supported by lowering")
            );
        }
    }

    #[test]
    fn typeof_uses_only_reach_functions_when_the_operand_executes() {
        for body in [
            "__typeof__(&f) p=0;",
            "int x=__alignof__(f());",
            "__typeof__(int[sizeof(&f)]) a;",
        ] {
            let source = format!(
                "static int f(void) {{ __asm__(\"\"); return 0; }} int main(void) {{ {body} \
                 return 0; }}\n"
            );
            let result = bcc(&source, &["--emit=llvm", "-std=gnu99"]);
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        let source = "static int f(void) { return 7; } int main(void) { int n=2; int a[n]; \
                      __typeof__((f(), &a)) p=0; return 0; }\n";
        let result = bcc(source, &["--emit=llvm", "-std=gnu99"]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(symbols(&result.stdout).contains("f"));
    }

    /// An attribute meaning code generation lacks refuses the module only
    /// when the body that wrote it is emitted.
    #[test]
    fn attribute_refusals_in_unreachable_bodies_are_not_reported() {
        let body = "static int f(void) { typedef void (__attribute__((vectorcall)) *fp)(void);                     return 0; }";
        let unused = format!(
            "{body} int main(void) {{ return 0; }}
"
        );
        let result = bcc(&unused, &["--emit=llvm", "-std=gnu99"]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let used = format!(
            "{body} int main(void) {{ return f(); }}
"
        );
        let result = bcc(&used, &["--emit=llvm", "-std=gnu99"]);
        assert!(!result.status.success());
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("vectorcall")
                || String::from_utf8_lossy(&result.stderr).contains("convention"),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    #[test]
    fn forward_redeclarations_addresses_and_cycles_are_reachable() {
        let source = "static int a(int); static int b(int); static int leaf(void) { return 7; }
static int (*table[])(void) = {leaf};
static int a(int n) { return n ? b(n-1) : table[0](); }
static int b(int n) { return a(n); }
static int dead(int n) { return dead(n); }
int main(void) { return a(2)-7; }";
        let output = bcc(source, &["--emit=llvm"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            symbols(&output.stdout),
            ["a", "b", "leaf", "main", "table"]
                .into_iter()
                .map(str::to_owned)
                .collect()
        );
        assert!(bcc(source, &["--interpret"]).status.success());
    }

    #[test]
    fn static_local_initializers_only_reach_functions_when_the_object_is_emitted() {
        for (body, expected) in [
            ("static int (*p)(void)=f; return 0;", vec!["main"]),
            (
                "static int (*p)(void)=f; return p()-7;",
                vec!["f", "main", "main.p"],
            ),
        ] {
            let source = format!("static int f(void) {{ return 7; }} int main(void) {{ {body} }}");
            let output = bcc(&source, &["--emit=llvm"]);
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                symbols(&output.stdout),
                expected.into_iter().map(str::to_owned).collect()
            );
            assert!(bcc(&source, &["--interpret"]).status.success(), "{source}");
        }
    }

    #[test]
    fn parameter_type_site_references_belong_to_the_emitted_function() {
        let source = "static int bound(void) { return 2; } static int dead(void) { return 0; }
int f(int a[bound()]) { return a[0]; }
int main(void) { int a[2]={7,9}; return f(a)-7; }";
        let output = bcc(source, &["--emit=llvm"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            symbols(&output.stdout),
            ["bound", "f", "main"]
                .into_iter()
                .map(str::to_owned)
                .collect()
        );
        assert!(
            bcc(source, &["--interpret"]).status.success(),
            "parameter bound executes"
        );
    }

    #[test]
    fn fabs_builtin_siblings_have_their_declared_precision() {
        let source = "float (*f)(float)=__builtin_fabsf;
long double (*l)(long double)=__builtin_fabsl;
int main(void) { return f(-1.5f)!=1.5f || l(-2.5L)!=2.5L; }";
        let output = bcc(source, &["--interpret", "--target=x86_64-pc-windows-msvc"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn long_reference_chain_uses_an_explicit_worklist() {
        let mut source = String::from("static int f0(void) { return 7; }\n");
        for index in 1..2000 {
            use std::fmt::Write as _;
            writeln!(
                source,
                "static int f{index}(void) {{ return f{}(); }}",
                index - 1
            )
            .unwrap();
        }
        source.push_str("int main(void) { return f1999()-7; }\n");
        // Keep the depth test independent of Windows' command-line length.
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/reachable-chain.c");
        std::fs::write(&path, &source).unwrap();
        let output = run_native(
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .arg(&path)
                .arg("--emit=ir"),
        )
        .unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout)
                .matches("function @f")
                .count(),
            2000
        );
    }

    fn symbols(bytes: &[u8]) -> BTreeSet<String> {
        String::from_utf8_lossy(bytes)
            .lines()
            .filter(|line| {
                line.starts_with("define ") || (line.starts_with('@') && line.contains(" = "))
            })
            .map(|line| {
                line.split_once('@')
                    .unwrap()
                    .1
                    .split(['(', ' ', '='])
                    .next()
                    .unwrap()
                    .trim_matches('"')
                    .to_owned()
            })
            .collect()
    }

    fn bcc(source: &str, flags: &[&str]) -> std::process::Output {
        run_native(
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(flags)
                .args(["--input", source]),
        )
        .unwrap()
    }

    fn clang() -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        root.join(if cfg!(windows) {
            "target/llvm/bin/clang.exe"
        } else {
            "target/llvm/bin/clang"
        })
    }
}
