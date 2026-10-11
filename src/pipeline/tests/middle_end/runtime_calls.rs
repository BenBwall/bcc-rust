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
                        },
                    )
                    .unwrap();
                },
            );
            assert_eq!(run_native(&exe).0, 0, "{name} {}", arm.name);
        }
    }
}
