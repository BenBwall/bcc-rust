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
                code: crate::target::CodeOptions::default(),
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

const FUNCTION_TABLE: &str = include_str!(
    "../../../../docs/research/conformance-reproducers-2026-10-13/\
     function_to_void_pointer_initializer.c"
);
const CALL_FUNCTION_TABLE: &str = "struct entry {int n; void *p;}; static double twice(double \
                                   x){return 2*x;} static const struct entry \
                                   table[]={{1,twice},{2,0}}; int main(void){return ((double \
                                   (*)(double))table[0].p)(21)!=42 || table[1].p!=0;}";

#[test]
fn function_designator_static_void_pointer_keeps_its_relocation() {
    use crate::ir::{
        GlobalInit,
        RelocationTarget,
        Symbol,
    };
    for source in [FUNCTION_TABLE, CALL_FUNCTION_TABLE] {
        // Keep a runtime table read: Clang folds FUNCTION_TABLE's original
        // condition and omits both the table and its function at O0.
        let source = format!(
            "volatile int slot; {}",
            source.replace("table[0]", "table[slot]")
        );
        for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
            for (optimize, profile) in STAGES {
                with_module(&source, target, optimize, profile, |module, target| {
                    assert!(module.globals().any(|(_, global)| matches!(global.init, Some(GlobalInit::Bytes {relocations, ..}) if relocations.iter().any(|relocation| matches!(relocation.target, RelocationTarget::Symbol(Symbol::Function(_)))))), "{module}");
                    assert_eq!(interpret(module, target), (0, String::new()));
                });
            }
        }
    }
}

#[test]
fn sqlite_function_table_initializer_matches_clang_native_execution() {
    let Some(target) = host() else {
        return;
    };
    let toolchain = Toolchain::bundled(target).unwrap();
    let directory = TempDir::new("static-function-void-pointer");
    for (index, source) in [FUNCTION_TABLE, CALL_FUNCTION_TABLE]
        .into_iter()
        .enumerate()
    {
        let input = directory.join(format!("table-{index}.c"));
        std::fs::write(&input, source).unwrap();
        for opt in [OptLevel::O0, OptLevel::O2] {
            let options = LlvmOptions {
                target,
                opt: Some(opt),
                disable_llvm_passes: false,
                kind: OutputKind::Executable,
                code: crate::target::CodeOptions::default(),
            };
            let executable = directory.join(format!("clang-{index}-{opt:?}.exe"));
            let output = toolchain
                .command(&input, &executable, options)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(run_native(&executable), (0, String::new()));
        }
        for arm in ARMS {
            let executable = directory.join(format!("bcc-{index}-{}.exe", arm.name));
            with_module(source, target, arm.bcc, Profile::PostAbi, |module, _| {
                compile_module(
                    module,
                    &executable,
                    LlvmOptions {
                        target,
                        opt: Some(arm.opt),
                        disable_llvm_passes: arm.disable_llvm_passes,
                        kind: OutputKind::Executable,
                        code: crate::target::CodeOptions::default(),
                    },
                )
                .unwrap();
            });
            assert_eq!(
                run_native(&executable),
                (0, String::new()),
                "{index} {}",
                arm.name
            );
        }
    }
}

#[test]
fn nullptr_objects_and_pointer_conversions_execute_in_every_stage() {
    let source = "typeof(nullptr) global=nullptr; int main(void){typeof(nullptr) n=global; int \
                  *p=n; bool b=n; return b || p!=nullptr || n!=nullptr || sizeof(n)!=sizeof(p);}";
    check_nullptr_program(source);
}

#[test]
fn nullptr_zero_comparisons_execute_in_every_stage() {
    check_nullptr_program(
        "int equal(typeof(nullptr) n){return n==0 && 0==n;} int different(typeof(nullptr) \
         n){return n!=0 || 0!=n;} int main(void){return !equal(nullptr)||different(nullptr);}",
    );
}

#[test]
fn nullptr_constant_casts_execute_in_every_stage() {
    check_nullptr_program(
        "typeof(nullptr) a=(typeof(nullptr))0,b=(typeof(nullptr))nullptr, \
         c=(typeof(nullptr))(void*)0; bool d=(bool)nullptr; int main(void){return \
         a!=nullptr||b!=nullptr||c!=nullptr||d;}",
    );
}

fn check_nullptr_program(source: &str) {
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        for (optimize, profile) in STAGES {
            with_configured_module(
                source,
                target,
                optimize,
                profile,
                CompilerConfiguration::new(
                    crate::configuration::CStandard::C23,
                    crate::configuration::ExtensionPolicy::Allow,
                )
                .with_target(target),
                |module, target| assert_eq!(interpret(module, target), (0, String::new())),
            );
        }
    }
}
