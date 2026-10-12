//! C99 §6.8p3 and §6.2.4p5: declarator order on repeated initialization.

use super::*;

const SOURCES: [&str; 3] = [
    r"
int main(void) {
    int *p = 0, n = 0;
again:;
    int a = p ? *p : 0, b[1] = {9};
    if (!n++) { p = b; goto again; }
    return a;
}
",
    r"
struct S { int value, zero; };
int main(void) {
    struct S *p = 0;
    int n = 0;
again:;
    struct S first = {p ? p->value : 0, 0}, second = {9};
    if (!n++) { p = &second; goto again; }
    if (first.zero || second.zero) return 1;
    return first.value;
}
",
    r"
int main(void) {
    volatile int *p = 0;
    int n = 0;
again:;
    volatile int a = p ? *p : 0, b = {9};
    if (!n++) { p = &b; goto again; }
    return a;
}
",
];

#[test]
fn declarator_initializers_run_in_order_in_all_stages() {
    for source in SOURCES {
        for target in [Target::LinuxGnu, Target::WindowsGnu] {
            for (optimize, profile) in STAGES {
                let actual = with_module(source, target, optimize, profile, interpret);
                assert_eq!(actual.0, 9, "{target:?} {optimize} {profile:?}\n{source}");
            }
        }
    }
}

#[test]
fn declarator_initializers_match_clang_and_native_execution() {
    let Some(target) = host() else { return };
    let toolchain = Toolchain::bundled(target).unwrap();
    let directory = TempDir::new("declarator-initializers");
    for (index, source) in SOURCES.into_iter().enumerate() {
        let input = directory.join(format!("case-{index}.c"));
        std::fs::write(&input, source).unwrap();
        for opt in [OptLevel::O0, OptLevel::O2] {
            let options = LlvmOptions {
                target,
                opt: Some(opt),
                disable_llvm_passes: false,
                kind: OutputKind::Executable,
            };
            let clang = directory.join(format!("clang-{index}-{opt:?}.exe"));
            let output = toolchain.command(&input, &clang, options).output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let expected = run_native(&clang);
            assert_eq!(expected.0, 9, "Clang: {source}");
            for optimize in [false, true] {
                for codegen_only in [false, true] {
                    if codegen_only && opt == OptLevel::O0 {
                        continue;
                    }
                    let executable = directory
                        .join(format!("bcc-{index}-{opt:?}-{optimize}-{codegen_only}.exe"));
                    with_module(source, target, optimize, Profile::PostAbi, |module, _| {
                        compile_module(
                            module,
                            &executable,
                            LlvmOptions {
                                disable_llvm_passes: codegen_only,
                                ..options
                            },
                        )
                        .unwrap();
                    });
                    assert_eq!(
                        run_native(&executable),
                        expected,
                        "{opt:?} {optimize} {codegen_only}\n{source}"
                    );
                }
            }
        }
    }
}
