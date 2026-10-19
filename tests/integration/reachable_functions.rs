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

    use crate::native::run_native;

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

    /// Windows DLL storage and inline linkage match the pinned Clang's
    /// emitted symbols: only evaluated references in emitted code fix an
    /// import, a constant `sizeof` does not use the entity, and a MinGW
    /// exported inline body is an external `weak_odr` definition that may
    /// call internal functions.
    #[test]
    fn dll_storage_and_inline_linkage_match_clang() {
        let clang = clang();
        let source_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/dll-linkage.c");
        for target in ["x86_64-w64-windows-gnu", "x86_64-pc-windows-msvc"] {
            let target_flag = format!("--target={target}");
            for (name, source, mode) in [
                (
                    "unused-static-reference",
                    "__declspec(dllimport) extern int x; int g(void) { return x; } extern int x; \
                     static int unused(void) { return x; }",
                    "c17",
                ),
                (
                    "sizeof-reference",
                    "__declspec(dllimport) extern int x; int g(void) { return x; } extern int x; \
                     int n = sizeof(x);",
                    "c17",
                ),
                (
                    "emitted-static-reference",
                    "__declspec(dllimport) extern int x; int g(void) { return x; } extern int x; \
                     static int called(void) { return x; } int h(void) { return called(); }",
                    "c17",
                ),
                (
                    "import-after-sizeof",
                    "extern int x; int n = sizeof(x); __declspec(dllimport) extern int x; int \
                     g(void) { return x; }",
                    "c17",
                ),
                (
                    "export-after-sizeof",
                    "int f(void); int n = sizeof(&f); __declspec(dllexport) int f(void) { return \
                     1; }",
                    "c17",
                ),
                (
                    "exported-inline-helper",
                    "static int helper(void) { return 7; } __declspec(dllexport) inline int \
                     f(void) { return helper(); }",
                    "gnu17",
                ),
                (
                    "constant-dead-if",
                    "__declspec(dllimport) extern int x; int g(void) { return x; } extern int x; \
                     int h(void) { if (0) return x; return (0 ? x : 0) + (0 && x); }",
                    "c17",
                ),
                (
                    "constant-false-loop",
                    "__declspec(dllimport) extern int x; int g(void) { return x; } extern int x; \
                     int h(void) { while (0) return x; return 0; }",
                    "c17",
                ),
                (
                    "after-jumps",
                    "__declspec(dllimport) extern int x; int g(void) { return x; } extern int x; \
                     int h(int a) { switch (a) { return x; case 1: break; } if (a) return 0; else \
                     return 1; return x; }",
                    "c17",
                ),
                (
                    "unused-inline",
                    "__declspec(dllimport) extern int x; int g(void) { return x; } extern int x; \
                     inline int h(void) { return x; }",
                    "c17",
                ),
                (
                    "referenced-inline",
                    "__declspec(dllimport) extern int x; int g(void) { return x; } extern int x; \
                     inline int h(void) { return x; } int k(void) { return h(); }",
                    "c17",
                ),
                (
                    "declared-export-inline-static-local",
                    "__declspec(dllexport) inline int f(void); static int helper(void) { return \
                     7; } inline int f(void) { static int n; return helper() + ++n; }",
                    "c17",
                ),
            ] {
                std::fs::write(&source_path, source).unwrap();
                let reference = Command::new(&clang)
                    .arg(&target_flag)
                    .args([
                        "-S",
                        "-emit-llvm",
                        "-O0",
                        "-fms-extensions",
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
                    source,
                    &[
                        "--emit=llvm",
                        "-fms-extensions",
                        &target_flag,
                        &format!("-std={mode}"),
                    ],
                );
                assert!(
                    output.status.success(),
                    "{target} {name}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert_eq!(
                    linkage(&output.stdout),
                    linkage(&reference.stdout),
                    "{target} {name}"
                );
            }
        }
        std::fs::remove_file(source_path).unwrap();
    }

    /// MSVC `__forceinline` matches the pinned Clang's LLVM IR before
    /// LLVM's passes (#89), so LLVM's always-inliner sees the same module:
    /// `linkonce_odr`, or `weak_odr` with `extern`, on MSVC; on GNU targets a
    /// C99 inline definition, an `available_externally` body that direct
    /// calls, recursive ones included, name by the external symbol. Each
    /// definition's linkage, inline control and direct callees must agree.
    /// `noinline` prevails. bcc's optimizer expands every direct call of a
    /// non-recursive `__forceinline` function.
    #[test]
    fn forceinline_linkage_and_inlining_match_clang() {
        const SOURCES: [(&str, &str); 3] = [
            (
                "specifiers",
                "__forceinline int k(int x) { return x; } static __forceinline int s(int x) { \
                 return x + 1; } extern __forceinline int e(int x) { return x + 2; } \
                 __forceinline int unused(int x) { return x + 3; } static __forceinline int \
                 sunused(int x) { return x + 4; } extern __forceinline int eunused(int x) { \
                 return x + 5; } int u(int x) { return k(x) + s(x) + e(x); } int (*p)(int) = k; \
                 int (*q)(int) = s; int (*r)(int) = e;",
            ),
            (
                "redeclarations",
                "__forceinline __attribute__((noinline)) int n1(int x) { return x; } \
                 __attribute__((noinline)) __forceinline int n2(int x) { return x; } \
                 __forceinline int d1(int x); int d1(int x) { return x; } int d2(int x); \
                 __forceinline int d2(int x) { return x; } __forceinline int d3(int x); \
                 __forceinline int d3(int x) { return x; } int u(int x) { return n1(x) + n2(x) + \
                 d1(x) + d2(x) + d3(x); }",
            ),
            (
                "recursion",
                "__forceinline int f(int x) { return x ? x * f(x - 1) : 1; } __forceinline int \
                 h(int); __forceinline int g(int x) { return x ? x * h(x - 1) : 1; } \
                 __forceinline int h(int x) { return x ? x + g(x - 1) : 0; } inline \
                 __attribute__((always_inline)) int a(int x) { return x ? a(x - 1) : 0; } static \
                 __forceinline int s(int x) { return x ? x * s(x - 1) : 1; } int u(int x) { \
                 return f(x) + g(x) + a(x) + s(x); }",
            ),
        ];
        let clang = clang();
        let source_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/forceinline.c");
        for target in [
            "x86_64-unknown-linux-gnu",
            "x86_64-w64-windows-gnu",
            "x86_64-pc-windows-msvc",
        ] {
            let target_flag = format!("--target={target}");
            for (name, source) in SOURCES {
                std::fs::write(&source_path, source).unwrap();
                let reference = Command::new(&clang)
                    .arg(&target_flag)
                    .args([
                        "-S",
                        "-emit-llvm",
                        "-O0",
                        "-Xclang",
                        "-disable-llvm-passes",
                        "-fms-extensions",
                        "-std=c17",
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
                let flags = ["--emit=llvm", "-fms-extensions", &target_flag, "-std=c17"];
                let output = bcc(source, &flags);
                assert!(
                    output.status.success(),
                    "{target} {name}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert_eq!(
                    inline_contract(&output.stdout),
                    inline_contract(&reference.stdout),
                    "{target} {name}:\n{}",
                    String::from_utf8_lossy(&output.stdout)
                );
                if name == "recursion" {
                    continue;
                }
                let optimized = bcc(source, &[&flags[..], &["--opt=bcc"]].concat());
                let text = String::from_utf8_lossy(&optimized.stdout);
                assert!(optimized.status.success(), "{target} {name}: {optimized:?}");
                let caller = text
                    .split("define i32 @u(")
                    .nth(1)
                    .and_then(|body| body.split("\n}").next())
                    .unwrap_or_else(|| panic!("{target} {name}: {text}"));
                let calls: Vec<_> = caller
                    .lines()
                    .filter_map(|line| line.split_once("call i32 @"))
                    .map(|(_, callee)| callee.split('(').next().unwrap())
                    .collect();
                let expected: &[&str] = if name == "redeclarations" {
                    &["n1", "n2"]
                } else {
                    &[]
                };
                assert_eq!(calls, expected, "{target} {name}: {text}");
            }
        }
        std::fs::remove_file(source_path).unwrap();
    }

    /// A recursive inline-only `__forceinline` function is not inline
    /// viable for LLVM, so Clang's caller calls the external definition
    /// another unit supplies; mutual recursion leaves the inner call
    /// external. bcc must run the same code at its default level, and plain
    /// `inline` already does (#89 review).
    /// C99: §6.7.4p6, pp. 112-113; PDF pp. 124-125.
    #[test]
    #[cfg(target_os = "linux")]
    fn recursive_forceinline_calls_match_clang_at_run_time() {
        const EXTERNAL: &str = "int f(int x) { return 100 + x; } int g(int x) { return 200 + x; } \
                                int h(int x) { return 300 + x; }";
        let root = std::env::temp_dir().join(format!("bcc-forceinline-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let external = root.join("external.c");
        std::fs::write(&external, EXTERNAL).unwrap();
        for (name, source) in [
            (
                "self",
                "__forceinline int f(int x) { return x ? x * f(x - 1) : 1; } int main(void) { \
                 return f(4) % 256; }",
            ),
            (
                "always-inline",
                "inline __attribute__((always_inline)) int f(int x) { return x ? x * f(x - 1) : \
                 1; } int main(void) { return f(4) % 256; }",
            ),
            (
                "plain",
                "inline int f(int x) { return x ? x * f(x - 1) : 1; } int main(void) { return \
                 f(4) % 256; }",
            ),
            (
                "mutual",
                "__forceinline int h(int); __forceinline int g(int x) { return x ? x * h(x - 1) : \
                 1; } __forceinline int h(int x) { return x ? x + g(x - 1) : 0; } int main(void) \
                 { return g(4) % 256; }",
            ),
            (
                "direct",
                "__forceinline int f(int x) { return x + 1; } int main(void) { return f(4); }",
            ),
        ] {
            let unit = root.join(format!("{name}.c"));
            std::fs::write(&unit, source).unwrap();
            let mut statuses = Vec::new();
            for compiler in ["clang", "bcc"] {
                let object = root.join(format!("{name}.{compiler}.o"));
                let compiled = if compiler == "bcc" {
                    bcc(
                        source,
                        &[
                            "--target=x86_64-unknown-linux-gnu",
                            "-fms-extensions",
                            "-std=c99",
                            "-c",
                            "-o",
                            object.to_str().unwrap(),
                        ],
                    )
                } else {
                    Command::new(clang())
                        .args([
                            "--target=x86_64-unknown-linux-gnu",
                            "-fms-extensions",
                            "-std=c99",
                            "-O0",
                            "-c",
                            "-o",
                        ])
                        .arg(&object)
                        .arg(&unit)
                        .output()
                        .unwrap()
                };
                assert!(compiled.status.success(), "{name} {compiler}: {compiled:?}");
                let program = root.join(format!("{name}.{compiler}"));
                let linked = Command::new(clang())
                    .args(["--target=x86_64-unknown-linux-gnu", "-fuse-ld=lld", "-o"])
                    .arg(&program)
                    .arg(&object)
                    .arg(&external)
                    .output()
                    .unwrap();
                assert!(linked.status.success(), "{name} {compiler}: {linked:?}");
                let ran = run_native(&mut Command::new(&program)).unwrap();
                statuses.push(ran.status.code());
            }
            assert_eq!(statuses[1], statuses[0], "{name}: bcc against clang");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Each defined function with its linkage, inline control and direct
    /// callees, and each declared function something references. Clang
    /// also gives a referenced inline-only function without `alwaysinline`
    /// an `available_externally` body, which LLVM's O0 pipeline never uses;
    /// it reads as the declaration bcc emits.
    fn inline_contract(bytes: &[u8]) -> BTreeSet<String> {
        const WORDS: [&str; 5] = [
            "define",
            "internal",
            "weak_odr",
            "linkonce_odr",
            "available_externally",
        ];
        let text = String::from_utf8_lossy(bytes);
        let control = |line: &str| {
            let groups: Vec<_> = line
                .split(' ')
                .filter(|word| word.starts_with('#'))
                .map(|group| format!("attributes {group} = {{"))
                .collect();
            // Clang -O0 marks every other function `noinline optnone`.
            let words: Vec<_> = text
                .lines()
                .filter(|attributes| groups.iter().any(|group| attributes.starts_with(group)))
                .flat_map(|attributes| attributes.split(' '))
                .collect();
            if words.contains(&"alwaysinline") {
                "alwaysinline"
            } else if words.contains(&"noinline") && !words.contains(&"optnone") {
                "noinline"
            } else {
                ""
            }
        };
        // `@name` followed by something that cannot continue a name.
        let mentions = |line: &str, name: &str| {
            let symbol = format!("@{name}");
            line.match_indices(&symbol).any(|(at, _)| {
                !line[at + symbol.len()..]
                    .starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '.')
            })
        };
        let mut contract = BTreeSet::new();
        let mut current = None;
        let mut callees = BTreeSet::new();
        for line in text.lines() {
            if line.starts_with("define ") || line.starts_with("declare ") {
                let name = line.split_once('@').unwrap().1.split('(').next().unwrap();
                if line.starts_with("declare ") {
                    if text
                        .lines()
                        .any(|other| other != line && mentions(other, name))
                    {
                        _ = contract.insert(format!("{name}: declare"));
                    }
                    continue;
                }
                let words: Vec<_> = line
                    .split([' ', '('])
                    .filter(|word| WORDS.contains(word))
                    .collect();
                let control = control(line);
                if words.contains(&"available_externally") && control != "alwaysinline" {
                    _ = contract.insert(format!("{name}: declare"));
                    continue;
                }
                current = Some(format!("{name}: {} {control}", words.join(" ")));
                callees.clear();
            } else if line == "}" {
                if let Some(definition) = current.take() {
                    let callees: Vec<_> = callees.iter().map(String::as_str).collect();
                    _ = contract.insert(format!("{definition} calls [{}]", callees.join(" ")));
                }
            } else if current.is_some()
                && let Some((_, callee)) = line.split_once(" call ")
                && let Some((_, callee)) = callee.split_once('@')
            {
                _ = callees.insert(callee.split('(').next().unwrap().to_owned());
            }
        }
        contract
    }

    /// Each emitted or declared global with its linkage and DLL storage.
    fn linkage(bytes: &[u8]) -> BTreeSet<String> {
        const WORDS: [&str; 7] = [
            "declare",
            "external",
            "internal",
            "weak_odr",
            "linkonce_odr",
            "dllimport",
            "dllexport",
        ];
        String::from_utf8_lossy(bytes)
            .lines()
            .filter(|line| {
                line.starts_with("define ")
                    || line.starts_with("declare ")
                    || (line.starts_with('@') && line.contains(" = "))
            })
            .map(|line| {
                let name = line
                    .split_once('@')
                    .unwrap()
                    .1
                    .split(['(', ' ', '='])
                    .next()
                    .unwrap()
                    .trim_matches('"');
                let words: Vec<_> = line
                    .split([' ', '('])
                    .filter(|word| WORDS.contains(word))
                    .collect();
                format!("{name}: {}", words.join(" "))
            })
            .collect()
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
