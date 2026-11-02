//! Reduced execute-corpus regressions for library-call contracts.

use super::*;

const OPEN_MEMCPY: &str = r"
void *memcpy();
int builtin_copy(void) {
    int x = 1, y = 2;
    __builtin_memcpy(&x, &y, sizeof x);
    return x;
}
int main(void) {
    int x = 0, y = 3;
    builtin_copy();
    void *result = memcpy((void *)&x, (const void *)&y, sizeof x);
    return x != 3 || result != &x;
}
";

#[test]
fn open_calls_to_prototyped_host_symbols_accept_compatible_arguments() {
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for (optimize, profile) in STAGES {
            let actual = with_module(OPEN_MEMCPY, target, optimize, profile, interpret);
            assert_eq!(actual.0, 0, "{target:?} {optimize} {profile:?}");
        }
    }
}

#[test]
fn open_calls_to_prototyped_host_symbols_reject_incompatible_arguments() {
    for call in [
        "memcpy((void *)&x, (const void *)&y, (int)sizeof x)",
        "memcpy((void *)&x, (const void *)&y)",
        "memcpy(1.0, (const void *)&y, sizeof x)",
    ] {
        let source = OPEN_MEMCPY.replace("memcpy((void *)&x, (const void *)&y, sizeof x)", call);
        for target in [Target::LinuxGnu, Target::WindowsGnu] {
            for (optimize, profile) in STAGES {
                with_module(&source, target, optimize, profile, |module, target| {
                    let arena = Bump::new();
                    let mut host = DefaultHost::for_target(&arena, target);
                    let result = interpreter::run(module, "main", &[], &mut host);
                    assert!(
                        matches!(
                            result,
                            Err(interpreter::Trap::UndefinedBehavior(
                                interpreter::UbKind::SignatureMismatch,
                                _
                            ))
                        ),
                        "{call} {target:?} {optimize} {profile:?}: {result:?}"
                    );
                });
            }
        }
    }
}

#[test]
fn torture_library_builtin_prototypes_reject_invalid_calls() {
    for source in [
        "void f(void) { __builtin_memcpy(0, 0); }",
        "void f(void) { __builtin_abort(1); }",
        "void f(void) { __builtin_strlen(3.0); }",
    ] {
        let tu = Bump::new();
        let mut context = Context::new(&tu);
        let unit = parse_translation_unit(
            &mut context,
            Path::new("<test>"),
            source,
            HeaderSearch::default(),
        );
        let sema = analyze_translation_unit(&mut context, &unit);
        assert!(
            !sema.lowerable(&context),
            "invalid builtin call accepted: {source}"
        );
    }
}

const PROGRAMS: &[(&str, &str)] = &[
    ("builtin_alias_unprototyped", OPEN_MEMCPY),
    (
        "bounded_strings",
        "extern void *memchr(const void *, int, unsigned long long); extern char *strchr(const \
         char *, int); extern char *strrchr(const char *, int); extern char *strncpy(char *, \
         const char *, unsigned long long); int main(void) { char a[3] = {1, 2, 3}, b[4] = {9, 9, \
         9, 9}; const char *s = \"aba\"; if (memchr(a, 258, 3) != a + 1 || memchr(a + 3, 0, 0) != \
         0 || memchr(a, 0, 3) != 0) return 1; if (strchr(s, 'a') != s || strrchr(s, 'a') != s + 2 \
         || strchr(s, 0) != s + 3 || strrchr(s, 'x') != 0) return 2; if (strncpy(b, s, 2) != b || \
         b[2] != 9) return 3; strncpy(b, \"x\", 4); return b[0] != 'x' || b[1] != 0 || b[2] != 0 \
         || b[3] != 0; }",
    ),
    (
        "block_scope_old_style",
        "float f(x) float x; { return x; } int main(void) { float f(); return f(4.) != 4.; }",
    ),
    (
        "unprototyped_exit",
        "void exit(); int main(void) { exit(0); return 1; }",
    ),
    (
        "indirect_unprototyped_exit",
        "void exit(); int main(void) { void (*p)() = exit; p(0); return 1; }",
    ),
    (
        "unprototyped_strlen",
        "unsigned long long strlen(); int main(void) { return strlen(\"abc\") != 3; }",
    ),
    (
        "builtin_abort",
        "int main(void) { if (0) __builtin_abort(); return 0; }",
    ),
    (
        "builtin_memory",
        "int main(void) { char a[4], b[4]; __builtin_memset(a, 42, 4); __builtin_memcpy(b, a, 4); \
         return b[3] != 42; }",
    ),
    (
        "builtin_alias",
        "void *memcpy(void *, const void *, unsigned long long); int main(void) { char a[2] = {3, \
         7}, b[2]; memcpy(b, a, 2); __builtin_memcpy(b, a, 2); return b[1] != 7; }",
    ),
];

#[test]
fn torture_runtime_calls_match_in_all_stages() {
    for &(name, source) in PROGRAMS {
        // size_t differs between LP64 and LLP64; these explicit declarations
        // use the Win64 size type, while builtin-only programs cover both.
        let targets: &[Target] = if matches!(name, "builtin_memory" | "builtin_abort") {
            &[Target::LinuxGnu, Target::WindowsGnu]
        } else {
            &[Target::WindowsGnu]
        };
        for &target in targets {
            for (optimize, profile) in STAGES {
                let actual = with_module(source, target, optimize, profile, interpret);
                assert_eq!(actual.0, 0, "{name} {target:?} {optimize} {profile:?}");
            }
        }
    }
}

#[test]
fn torture_runtime_calls_match_native_arms() {
    let Some(target @ Target::WindowsGnu) = host() else {
        return;
    };
    let _toolchain = Toolchain::bundled(target).expect("the bundled LLVM toolchain");
    let directory = TempDir::new("torture-runtime-calls");
    for &(name, source) in PROGRAMS {
        for arm in ARMS {
            let exe = directory.join(format!("{name}-{}.exe", arm.name));
            with_module(
                source,
                target,
                arm.bcc,
                Profile::PostAbi,
                |module, _target| {
                    compile_module(
                        module,
                        &exe,
                        LlvmOptions {
                            opt: Some(arm.opt),
                            disable_llvm_passes: arm.disable_llvm_passes,
                            target,
                            kind: OutputKind::Executable,
                            code: crate::target::CodeOptions::default(),
                            fast_math_runtime: false,
                        },
                    )
                    .unwrap();
                },
            );
            assert_eq!(run_native(&exe).0, 0, "{name} {}", arm.name);
        }
    }
}

const SOURCE: &str = r#"
int sprintf(char *, const char *, ...);
int strcmp(const char *, const char *);
int main(void) {
    char buf[64];
    if (sprintf(buf, "%#hho/%#hhx/%#hhX", 0, 0, 0) != 5
        || strcmp(buf, "0/0/0")) return 1;
    if (sprintf(buf, "%#hho/%#hhx/%#hhX", 127, 127, 127) != 14
        || strcmp(buf, "0177/0x7f/0X7F")) return 2;
    if (sprintf(buf, "[%#08x][%#06o][%-#6X]", 1, 1, 1) != 26
        || strcmp(buf, "[0x000001][000001][0X1   ]")) return 3;
    if (sprintf(buf, "%#hho/%#hhx", 256, 257) != 5
        || strcmp(buf, "0/0x1")) return 4;
    return 0;
}
"#;

#[test]
fn alternate_integer_formats_match_in_all_stages() {
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for (optimize, profile) in STAGES {
            let actual = with_module(SOURCE, target, optimize, profile, interpret);
            assert_eq!(actual.0, 0, "{target:?} {optimize} {profile:?}");
            assert!(actual.1.is_empty(), "sprintf must not write stdout");
        }
    }
}

#[test]
fn alternate_integer_formats_match_native_arms() {
    let Some(target) = host() else { return };
    let directory = TempDir::new("alternate-integer-formats");
    for arm in ARMS {
        let exe = directory.join(format!("{}.exe", arm.name));
        with_module(SOURCE, target, arm.bcc, Profile::PostAbi, |module, _| {
            compile_module(
                module,
                &exe,
                LlvmOptions {
                    opt: Some(arm.opt),
                    disable_llvm_passes: arm.disable_llvm_passes,
                    target,
                    kind: OutputKind::Executable,
                    code: crate::target::CodeOptions::default(),
                    fast_math_runtime: false,
                },
            )
            .expect("compile the formatted output regression");
        });
        assert_eq!(run_native(&exe).0, 0, "{}", arm.name);
    }
}

const IMPLICIT_PROGRAMS: &[(&str, &str)] = &[
    ("implicit_exit", "int main(void) { exit(0); }"),
    (
        "implicit_strings",
        "int main(void) { char b[4]; void *p = memcpy(b, \"abc\", 4); return p != b || strcmp(b, \
         \"abc\") || strlen(b) != 3; }",
    ),
];

#[test]
fn implicit_library_calls_match_interpreter_and_native_arms() {
    let directory = TempDir::new("implicit-library-calls");
    for &(name, source) in IMPLICIT_PROGRAMS {
        for target in [Target::LinuxGnu, Target::WindowsGnu] {
            let configuration = CompilerConfiguration::default()
                .with_target(target)
                .with_gnu_extensions(true)
                .with_warning_option(
                    crate::configuration::DiagnosticGroup::ImplicitFunctionDeclaration,
                    crate::configuration::WarningOption::Error(false),
                );
            for (optimize, profile) in STAGES {
                let actual = with_configured_module(
                    source,
                    target,
                    optimize,
                    profile,
                    configuration,
                    interpret,
                );
                assert_eq!(actual.0, 0, "{name} {target:?} {optimize} {profile:?}");
            }
            if host() == Some(target) {
                for arm in ARMS {
                    let exe = directory.join(format!("{name}-{}.exe", arm.name));
                    with_configured_module(
                        source,
                        target,
                        arm.bcc,
                        Profile::PostAbi,
                        configuration,
                        |module, _| {
                            compile_module(
                                module,
                                &exe,
                                LlvmOptions {
                                    opt: Some(arm.opt),
                                    disable_llvm_passes: arm.disable_llvm_passes,
                                    target,
                                    kind: OutputKind::Executable,
                                    code: crate::target::CodeOptions::default(),
                                    fast_math_runtime: false,
                                },
                            )
                            .unwrap();
                        },
                    );
                    assert_eq!(run_native(&exe).0, 0, "{name} {}", arm.name);
                }
            }
        }
    }
}

#[test]
fn implicit_library_abi_and_noundef_match_clang() {
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        let size = if target == Target::LinuxGnu {
            "unsigned long"
        } else {
            "unsigned long long"
        };
        let source = format!(
            "void f(void *p, const void *q, {size} n, const char *a, const char *b, int status) \
             {{ memcpy(p,q,n); void *(*copy)(void *, const void *, {size}) = memcpy; copy(p,q,n); \
             strcmp(a,b); strlen(a); printf(\"%d\", status); exit(status); }}"
        );
        let directory = TempDir::new("implicit-library-abi");
        let path = directory.join("source.c");
        std::fs::write(&path, &source).unwrap();
        let toolchain = Toolchain::bundled(Target::LinuxGnu).unwrap();
        let clang = Command::new(toolchain.clang())
            .args([
                "-std=gnu17",
                "-Wno-error=implicit-function-declaration",
                "-O0",
                "-S",
                "-emit-llvm",
                "-o",
                "-",
            ])
            .arg(format!("--target={}", target.triple()))
            .arg(path)
            .output()
            .unwrap();
        assert!(
            clang.status.success(),
            "{}",
            String::from_utf8_lossy(&clang.stderr)
        );
        let clang = String::from_utf8(clang.stdout).unwrap();
        with_configured_module(
            &source,
            target,
            false,
            Profile::PostAbi,
            CompilerConfiguration::default()
                .with_target(target)
                .with_gnu_extensions(true)
                .with_warning_option(
                    crate::configuration::DiagnosticGroup::ImplicitFunctionDeclaration,
                    crate::configuration::WarningOption::Error(false),
                ),
            |module, _| {
                let mut bcc = String::new();
                crate::backend::llvm::emit_module(module, target, &mut bcc).unwrap();
                for (name, result, args) in [
                    ("memcpy", "ptr", vec!["ptr", "ptr", "i64"]),
                    ("strcmp", "i32", vec!["ptr", "ptr"]),
                    ("strlen", "i64", vec!["ptr"]),
                    ("printf", "i32", vec!["ptr", "..."]),
                    ("exit", "void", vec!["i32"]),
                ] {
                    let marker = format!("@{name}(");
                    for text in [&bcc, &clang] {
                        let declaration = text
                            .lines()
                            .find(|l| l.starts_with("declare ") && l.contains(&marker))
                            .unwrap();
                        assert!(
                            declaration
                                .split_once('@')
                                .unwrap()
                                .0
                                .split_whitespace()
                                .any(|w| w == result),
                            "{declaration}"
                        );
                        assert!(
                            !declaration.split_once('@').unwrap().0.contains("noundef"),
                            "{declaration}"
                        );
                        let parameters = declaration
                            .split_once(&marker)
                            .unwrap()
                            .1
                            .split_once(')')
                            .unwrap()
                            .0;
                        let actual: Vec<_> = parameters.split(',').map(str::trim).collect();
                        assert_eq!(actual.len(), args.len(), "{declaration}");
                        for (actual, expected) in actual.iter().zip(&args) {
                            assert_eq!(
                                actual.split_whitespace().next().unwrap(),
                                *expected,
                                "{declaration}"
                            );
                            assert_eq!(
                                actual.contains("noundef"),
                                *expected != "...",
                                "{declaration}"
                            );
                        }
                    }
                    if name != "memcpy" {
                        for text in [&bcc, &clang] {
                            let call = text
                                .lines()
                                .find(|l| l.contains("call ") && l.contains(&marker))
                                .unwrap();
                            let count = if name == "printf" { 2 } else { args.len() };
                            assert_eq!(call.matches("noundef").count(), count, "{call}");
                        }
                    }
                }
                // Clang rewrites direct memcpy to an intrinsic. The indirect
                // call preserves its C ABI and all three scalar contracts.
                for text in [&bcc, &clang] {
                    let call = text.lines().rfind(|l| l.contains("call ptr ")).unwrap();
                    assert_eq!(call.matches("noundef").count(), 3, "{call}");
                }
            },
        );
    }
}
