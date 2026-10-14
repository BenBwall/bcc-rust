//! GNU `cleanup`, `constructor`, `destructor`, `alias` and `weak` through the
//! interpreter, every LLVM arm and bundled Clang, and across translation
//! units linked with Clang-compiled code: natively for the Windows GNU host
//! and under WSL for System V. Expected outputs are bundled Clang 23's (and
//! GCC 15's, which agrees on these programs).
//! GNU extension: GCC manual, "Common Function Attributes" and "Common
//! Variable Attributes".

use super::*;

/// Single-TU programs: name, source, status and output.
const ATTRIBUTE_PROGRAMS: &[(&str, &str, i32, &str)] = &[
    (
        // The conformance reproducer of issue #56.
        "semantic_attributes",
        "int printf(const char *, ...);
static int order = 0, released = 0;
__attribute__((constructor)) static void early(void) { order = order * 10 + 1; }
__attribute__((constructor(200))) static void early2(void) { order = order * 10 + 2; }
__attribute__((destructor)) static void late(void) { printf(\"bye %d\\n\", order); }
static void release(int *p) { released += *p; }
int main(void) {
    { int a __attribute__((cleanup(release))) = 7; }
    { int b __attribute__((cleanup(release))) = 5; }
    printf(\"%d %d\\n\", order, released);
    order = 7;
    return 0;
}
",
        0,
        "21 12\nbye 7\n",
    ),
    (
        // The conformance reproducer of issue #56.
        "alias_weak",
        "int printf(const char *, ...);
int real_fn(int x) { return x * 3; }
int alias_fn(int) __attribute__((alias(\"real_fn\")));
__attribute__((weak)) int weak_fn(int x) { return x + 1; }
int main(void) { printf(\"%d %d\\n\", alias_fn(4), weak_fn(4)); return 0; }
",
        0,
        "12 5\n",
    ),
    (
        "init_order",
        "int putchar(int);
void exit(int);
static void p(int c) { putchar(c); }
__attribute__((constructor(300))) static void c300(void) { p('D'); }
__attribute__((constructor(200))) static void c200(void) { p('B'); }
__attribute__((constructor)) static void cdef(void) { p('A'); }
__attribute__((constructor(200))) static void c200b(void) { p('C'); }
__attribute__((destructor(300))) static void d300(void) { p('y'); }
__attribute__((destructor(200))) static void d200(void) { p('x'); }
__attribute__((destructor)) static void ddef(void) { p('z'); }
__attribute__((destructor(200))) static void d200b(void) { p('w'); }
void late(void) __attribute__((destructor(250)));
void late(void) { p('v'); }
int main(void) { p('m'); exit(3); }
",
        3,
        "BCDAmzyvwx",
    ),
    (
        "cleanup_exits",
        "int printf(const char *, ...);
static void out(int *p) { printf(\"out %d\\n\", *p); }
static void outl(long *p) { printf(\"outl %ld\\n\", *p); *p = 99; }
static void outv(void *p) { printf(\"outv %d\\n\", *(int *)p); }
static int ret_value(void) {
    int a __attribute__((cleanup(out))) = 1;
    int b __attribute__((cleanup(out))) = 2;
    return a + b;
}
static long ret_modified(void) {
    long x __attribute__((cleanup(outl))) = 5;
    return x;
}
static void loops(void) {
    for (int i = 0; i < 4; i++) {
        int v __attribute__((cleanup(out))) = i * 10;
        if (i == 1) continue;
        if (i == 3) break;
        printf(\"body %d\\n\", i);
    }
    int k = 0;
    while (k < 2) {
        int w __attribute__((cleanup(outv))) = 100 + k;
        k++;
    }
}
static void gotos(int n) {
again:
    {
        int g __attribute__((cleanup(out))) = n;
        if (n-- > 0) goto again;
        goto done;
    }
done:
    printf(\"done\\n\");
}
static int stmt_expr(void) {
    int r = ({ int t __attribute__((cleanup(out))) = 7; t + 1; });
    return r;
}
static void sw(int n) {
    switch (n) {
    case 1: {
        int c __attribute__((cleanup(out))) = 11;
        break;
    }
    default: {
        int d __attribute__((cleanup(out))) = 12;
        printf(\"default\\n\");
    }
    }
}
static void nested(void) {
    int a __attribute__((cleanup(out))) = 1;
    {
        int b __attribute__((cleanup(out))) = 2;
        {
            int c __attribute__((cleanup(out))) = 3;
        }
        int d __attribute__((cleanup(out))) = 4;
    }
    int e __attribute__((cleanup(out))) = 5;
}
static void vla(int n) {
    int arr[n];
    arr[0] = 42;
    int z __attribute__((cleanup(out))) = n;
    printf(\"vla %d\\n\", arr[0]);
}
int main(void) {
    printf(\"ret %d\\n\", ret_value());
    printf(\"mod %ld\\n\", ret_modified());
    loops();
    gotos(2);
    printf(\"se %d\\n\", stmt_expr());
    sw(1);
    sw(2);
    nested();
    vla(3);
    return 0;
}
",
        0,
        "out 2\nout 1\nret 3\noutl 5\nmod 5\nbody 0\nout 0\nout 10\nbody 2\nout 20\nout 30\noutv \
         100\noutv 101\nout 2\nout 1\nout 0\ndone\nout 7\nse 8\nout 11\ndefault\nout 12\nout \
         3\nout 4\nout 2\nout 5\nout 1\nvla 42\nout 3\n",
    ),
    (
        "undefined_weak",
        "extern int missing(void) __attribute__((weak));
extern int missing_object __attribute__((weak));
int main(void) { return (missing ? 1 : 40) + (&missing_object ? 1 : 2); }
",
        42,
        "",
    ),
];

#[test]
fn entity_attributes_interpret_at_every_stage() {
    for &(name, source, status, output) in ATTRIBUTE_PROGRAMS {
        for target in [Target::LinuxGnu, Target::WindowsGnu] {
            for (optimize, profile) in STAGES {
                let actual = with_module(source, target, optimize, profile, interpret);
                assert_eq!(
                    actual,
                    (status, output.to_string()),
                    "{name} {target:?} {optimize} {profile:?}"
                );
            }
        }
    }
}

#[test]
fn entity_attributes_match_native_arms_and_clang() {
    let Some(target) = host() else { return };
    let Ok(toolchain) = Toolchain::bundled(target) else {
        eprintln!("skipping: no bundled toolchain");
        return;
    };
    let directory = TempDir::new("entity-attributes");
    for &(name, source, status, output) in ATTRIBUTE_PROGRAMS {
        for arm in ARMS {
            let executable = directory.join(format!("{name}-{}.exe", arm.name));
            with_module(source, target, arm.bcc, Profile::PostAbi, |module, _| {
                compile_module(
                    module,
                    &executable,
                    LlvmOptions {
                        opt: Some(arm.opt),
                        disable_llvm_passes: arm.disable_llvm_passes,
                        target,
                        kind: OutputKind::Executable,
                    },
                )
                .unwrap();
            });
            assert_eq!(
                run_native(&executable),
                (status, output.to_string()),
                "{name} {}",
                arm.name
            );
        }
        let input = directory.join(format!("{name}.c"));
        std::fs::write(&input, source).unwrap();
        for opt in [OptLevel::O0, OptLevel::O2] {
            let executable = directory.join(format!("{name}-clang{opt:?}.exe"));
            let result = toolchain
                .command(
                    &input,
                    &executable,
                    LlvmOptions {
                        opt: Some(opt),
                        disable_llvm_passes: false,
                        target,
                        kind: OutputKind::Executable,
                    },
                )
                .output()
                .unwrap();
            assert!(result.status.success(), "{result:?}");
            assert_eq!(
                run_native(&executable),
                (status, output.to_string()),
                "{name} clang {opt:?}"
            );
        }
    }
}

/// One side of a two-unit program; the other side is compiled by Clang.
const BCC_UNIT: &str = "int printf(const char *, ...);
int clang_side(void);
__attribute__((weak)) int hook(void) { return 1; }
__attribute__((weak)) int other(void) { return 2; }
extern int optional(void) __attribute__((weak));
int counter = 5;
extern int counter_alias __attribute__((alias(\"counter\")));
int twice(int x) { return 2 * x; }
int twice_alias(int) __attribute__((alias(\"twice\")));
__attribute__((weak)) int weak_value = 7;
int strong(void) { return 30; }
int main(void) {
    printf(\"%d %d %d %d %d %d %d\\n\", hook(), other(), optional ? optional() : -1, counter_alias,
           twice_alias(4), weak_value, clang_side());
    return 0;
}
";

/// Clang's unit: strong definitions replace bcc's weak ones, and it uses
/// bcc's aliases and overrides nothing that bcc defines strongly.
const CLANG_UNIT: &str = "int hook(void) { return 10; }
int weak_value = 70;
extern int counter_alias;
int twice_alias(int);
__attribute__((weak)) int strong(void) { return 3; }
int clang_side(void) { return counter_alias == 5 && twice_alias(3) == 6 ? strong() : -1; }
";

const TWO_UNIT_OUTPUT: &str = "10 2 -1 5 8 70 30\n";

#[test]
fn weak_and_alias_link_with_clang_units_on_both_targets() {
    // Windows GNU natively, through the bundled toolchain.
    if host() == Some(Target::WindowsGnu)
        && let Ok(toolchain) = Toolchain::bundled(Target::WindowsGnu)
    {
        let directory = TempDir::new("weak-alias-windows");
        let clang = directory.join("clang.c");
        std::fs::write(&clang, CLANG_UNIT).unwrap();
        for arm in ARMS {
            let object = directory.join(format!("bcc-{}.o", arm.name));
            let executable = directory.join(format!("two-{}.exe", arm.name));
            let options = LlvmOptions {
                opt:                 Some(arm.opt),
                disable_llvm_passes: arm.disable_llvm_passes,
                target:              Target::WindowsGnu,
                kind:                OutputKind::Executable,
            };
            with_module(
                BCC_UNIT,
                Target::WindowsGnu,
                arm.bcc,
                Profile::PostAbi,
                |module, _| {
                    compile_module(
                        module,
                        &object,
                        LlvmOptions {
                            kind: OutputKind::Object,
                            ..options
                        },
                    )
                    .unwrap();
                },
            );
            let mut command = toolchain.command(&object, &executable, options);
            _ = command.args(["-x", "c"]).arg(&clang);
            let output = command.output().unwrap();
            assert!(output.status.success(), "{output:?}");
            assert_eq!(
                run_native(&executable),
                (0, TWO_UNIT_OUTPUT.to_string()),
                "windows {}",
                arm.name
            );
        }
    }
    // System V under WSL, linked by the distribution's clang with its C
    // library so that constructors and the dynamic loader behave as usual.
    let wsl_clang = cfg!(windows)
        && crate::test_support::native::run_native(Command::new("wsl.exe").args([
            "-e",
            "clang",
            "--version",
        ]))
        .is_ok_and(|output| output.status.success());
    if !wsl_clang {
        eprintln!("skipping System V: WSL clang is unavailable");
        return;
    }
    let directory = TempDir::new("weak-alias-sysv");
    std::fs::write(directory.join("clang.c"), CLANG_UNIT).unwrap();
    for arm in ARMS {
        let object = format!("bcc-{}.o", arm.name);
        with_module(
            BCC_UNIT,
            Target::LinuxGnu,
            arm.bcc,
            Profile::PostAbi,
            |module, _| {
                compile_module(
                    module,
                    &directory.join(&object),
                    LlvmOptions {
                        opt:                 Some(arm.opt),
                        disable_llvm_passes: arm.disable_llvm_passes,
                        target:              Target::LinuxGnu,
                        kind:                OutputKind::Object,
                    },
                )
                .unwrap();
            },
        );
        let program = format!("two-{}", arm.name);
        let link = crate::test_support::native::run_native(
            Command::new("wsl.exe")
                .args(["-e", "clang", &object, "clang.c", "-o", &program])
                .current_dir(directory.path()),
        )
        .unwrap();
        assert!(link.status.success(), "{link:?}");
        let run = crate::test_support::native::run_native(
            Command::new("wsl.exe")
                .args(["-e", &format!("./{program}")])
                .current_dir(directory.path()),
        )
        .unwrap();
        assert_eq!(
            (
                run.status.code(),
                String::from_utf8(run.stdout).unwrap().as_str()
            ),
            (Some(0), TWO_UNIT_OUTPUT),
            "System V {}",
            arm.name
        );
    }
}

#[test]
fn single_unit_programs_run_on_system_v_under_wsl() {
    let wsl_clang = cfg!(windows)
        && crate::test_support::native::run_native(Command::new("wsl.exe").args([
            "-e",
            "clang",
            "--version",
        ]))
        .is_ok_and(|output| output.status.success());
    if !wsl_clang {
        eprintln!("skipping System V: WSL clang is unavailable");
        return;
    }
    let directory = TempDir::new("entity-attributes-sysv");
    for &(name, source, status, output) in ATTRIBUTE_PROGRAMS {
        std::fs::write(directory.join(format!("{name}.c")), source).unwrap();
        let reference = crate::test_support::native::run_native(
            Command::new("wsl.exe")
                .args([
                    "-e",
                    "clang",
                    &format!("{name}.c"),
                    "-o",
                    &format!("{name}-clang"),
                ])
                .current_dir(directory.path()),
        )
        .unwrap();
        assert!(reference.status.success(), "{reference:?}");
        for arm in ARMS {
            let object = format!("{name}-{}.o", arm.name);
            with_module(
                source,
                Target::LinuxGnu,
                arm.bcc,
                Profile::PostAbi,
                |module, _| {
                    compile_module(
                        module,
                        &directory.join(&object),
                        LlvmOptions {
                            opt:                 Some(arm.opt),
                            disable_llvm_passes: arm.disable_llvm_passes,
                            target:              Target::LinuxGnu,
                            kind:                OutputKind::Object,
                        },
                    )
                    .unwrap();
                },
            );
            let program = format!("{name}-{}", arm.name);
            let link = crate::test_support::native::run_native(
                Command::new("wsl.exe")
                    .args(["-e", "clang", &object, "-o", &program])
                    .current_dir(directory.path()),
            )
            .unwrap();
            assert!(link.status.success(), "{link:?}");
            for executable in [program, format!("{name}-clang")] {
                let run = crate::test_support::native::run_native(
                    Command::new("wsl.exe")
                        .args(["-e", &format!("./{executable}")])
                        .current_dir(directory.path()),
                )
                .unwrap();
                assert_eq!(
                    (
                        run.status.code(),
                        String::from_utf8(run.stdout).unwrap().as_str()
                    ),
                    (Some(status), output),
                    "System V {executable}"
                );
            }
        }
    }
}
