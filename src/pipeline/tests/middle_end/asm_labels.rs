//! Assembler names must agree through the interpreter, LLVM and Clang.
use super::*;

#[test]
fn builtin_spelling_in_asm_label_is_exact() {
    let source = "int renamed(int) __asm__(\"__builtin_abs\");\nint renamed(int x) { return x + \
                  100; }\nextern int abs(int);\nint main(void) { return renamed(1) + abs(-42); }\n";
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for (optimize, profile) in STAGES {
            with_module(source, target, optimize, profile, |module, target| {
                assert!(module.symbol("__builtin_abs").is_some(), "{module}");
                assert_eq!(interpret(module, target), (143, String::new()));
            });
        }
    }
    let Some(target) = host() else {
        return;
    };
    let directory = TempDir::new("builtin-asm-label");
    let input = directory.join("input.c");
    std::fs::write(&input, source).unwrap();
    let toolchain = Toolchain::bundled(target).unwrap();
    for opt in [OptLevel::O0, OptLevel::O2] {
        let options = LlvmOptions {
            target,
            opt: Some(opt),
            disable_llvm_passes: false,
            kind: OutputKind::Executable,
            code: crate::target::CodeOptions::default(),
            fast_math_runtime: false,
        };
        let clang = directory.join(format!("clang-{opt:?}.exe"));
        let output = toolchain.command(&input, &clang, options).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(run_native(&clang), (143, String::new()));
        for optimize in [false, true] {
            let executable = directory.join(format!("bcc-{opt:?}-{optimize}.exe"));
            with_module(source, target, optimize, Profile::PostAbi, |module, _| {
                compile_module(module, &executable, options).unwrap();
            });
            assert_eq!(run_native(&executable), (143, String::new()));
        }
    }
}

const PROGRAMS: &[&str] = &[
    "extern int f(void) __asm__(\"g\"); int g(void){return 42;} int main(void){return f();}\n",
    "int f(void) __asm__(\"g\"); int f(void){return 42;} int main(void){return f();}\n",
    "extern int a __asm__(\"b\"); int b=42; int main(void){return a;}\n",
    "extern int a[] __asm__(\"b\"); int *p=a; int b[1]={42}; int main(void){return p[0];}\n",
    "int a __asm__(\"b\")=42; int main(void){return a;}\n",
    "int main(void){static int a __asm__(\"b\")=42; return a;}\n",
];

#[test]
fn asm_labels_interpret_before_and_after_abi_lowering() {
    for source in PROGRAMS {
        for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
            for (optimize, profile) in STAGES {
                let actual = with_module(source, target, optimize, profile, interpret);
                assert_eq!(
                    actual,
                    (42, String::new()),
                    "{source} {target:?} {optimize} {profile:?}"
                );
            }
        }
    }
}

#[test]
fn asm_labels_native_and_clang_return_42() {
    let Some(target) = host() else {
        return;
    };
    let toolchain = Toolchain::bundled(target).unwrap();
    let directory = TempDir::new("asm-labels");
    for (index, source) in PROGRAMS.iter().enumerate() {
        let input = directory.join(format!("case-{index}.c"));
        std::fs::write(&input, source).unwrap();
        let options = LlvmOptions {
            target,
            opt: Some(OptLevel::O0),
            disable_llvm_passes: false,
            kind: OutputKind::Executable,
            code: crate::target::CodeOptions::default(),
            fast_math_runtime: false,
        };
        let clang = directory.join(format!("clang-{index}.exe"));
        let output = toolchain.command(&input, &clang, options).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(run_native(&clang), (42, String::new()), "Clang: {source}");
        for optimize in [false, true] {
            let executable = directory.join(format!("bcc-{index}-{optimize}.exe"));
            with_module(
                source,
                target,
                optimize,
                Profile::PostAbi,
                |module, target| {
                    assert_eq!(interpret(module, target), (42, String::new()));
                    compile_module(module, &executable, options).unwrap();
                },
            );
            assert_eq!(
                run_native(&executable),
                (42, String::new()),
                "bcc: {source}"
            );
        }
    }
}
