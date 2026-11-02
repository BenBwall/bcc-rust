//! Ordinary bindings survive semantic dispatch and native code generation.
//! C99: §6.2.1p4, pp. 29-30; PDF pp. 41-42; GNU library builtin extensions.
use super::*;
use crate::configuration::{
    CStandard,
    ExtensionPolicy,
};

const ALLOCA: &str = "int alloca(int x){return x+1;} int main(void){return alloca(1)!=2;}";
const ASSEMBLER_ALLOCA: &str = r#"int calls; void *alloca(__SIZE_TYPE__) __asm__("my_alloca"); void *alloca(__SIZE_TYPE__ n){++calls;return &calls;} int main(void){void *p=alloca(7);return p!=&calls||calls!=1;}"#;
// Clang still substitutes compatible unlabeled alloca definitions by default.
// Disable that substitution for the ordinary-call contract requested here.
const DEFINED_ALLOCA: &str = "int calls; void *alloca(__SIZE_TYPE__ n){++calls;return &calls;} \
                              int main(void){void *p=alloca(7);return p!=&calls||calls!=1;}";
const FREESTANDING: &str =
    "int f(void){return strlen(7);} int strlen(int x){return x+1;} int main(void){return f()!=8;}";

fn cases(
    target: Target,
    shadows: &str,
) -> [(&str, CompilerConfiguration, &'static [&'static str]); 8] {
    let strict =
        CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Allow).with_target(target);
    let gnu = strict.with_gnu_extensions(true);
    [
        (shadows, gnu, &["-std=gnu99"]),
        (ALLOCA, strict, &["-std=c99"]),
        (ALLOCA, gnu, &["-std=gnu99"]),
        (ASSEMBLER_ALLOCA, strict, &["-std=c99"]),
        (ASSEMBLER_ALLOCA, gnu, &["-std=gnu99"]),
        (DEFINED_ALLOCA, strict, &["-std=c99", "-fno-builtin-alloca"]),
        (DEFINED_ALLOCA, gnu, &["-std=gnu99", "-fno-builtin-alloca"]),
        (
            FREESTANDING,
            CompilerConfiguration::new(CStandard::C89, ExtensionPolicy::Allow)
                .with_target(target)
                .with_hosted(false),
            &[
                "-std=c89",
                "-ffreestanding",
                "-Wno-implicit-function-declaration",
            ],
        ),
    ]
}

#[test]
fn undeclared_alloca_obeys_strict_and_gnu_modes() {
    use crate::{
        lowering::{
            LoweringError,
            LoweringErrorKind,
        },
        pipeline::MiddleEndError,
        translation_phases::{
            TranslationError,
            semantic_analysis::{
                SemanticError,
                SemanticErrorKind,
            },
        },
    };

    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        let strict =
            CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Allow).with_target(target);
        let tu = Bump::new();
        let mut context = Context::with_configuration(&tu, strict);
        let unit = parse_translation_unit(
            &mut context,
            Path::new("<test>"),
            "void f(void){alloca(4);}\n",
            HeaderSearch::default(),
        );
        let sema = analyze_translation_unit(&mut context, &unit);
        assert!(!sema.lowerable(&context));
        let errors = context.take_pending_errors();
        assert!(
            matches!(
                errors.as_slice(),
                [TranslationError::Semantic(SemanticError {
                    kind: SemanticErrorKind::ImplicitFunctionDeclaration,
                    ..
                })]
            ),
            "{target:?}: {errors:?}"
        );
        let ir = Bump::new();
        let result = lower_translation_unit(&context, &unit, &sema, &ir, None, Profile::PreAbi);
        assert!(
            matches!(result, Err(MiddleEndError::Lowering(errors)) if matches!(errors.errors, [LoweringError { kind: LoweringErrorKind::UnitHasErrors, .. }]))
        );

        for (source, configuration) in [
            (
                "void f(void){alloca(4);}\n",
                strict.with_gnu_extensions(true),
            ),
            ("void f(void){__builtin_alloca(4);}\n", strict),
        ] {
            with_configured_module(
                source,
                target,
                false,
                Profile::PreAbi,
                configuration,
                |module, _| {
                    assert!(
                        module.to_string().contains("dyn_alloca"),
                        "{source}\n{module}"
                    );
                },
            );
        }
    }
}

#[test]
fn builtin_bindings_and_language_modes_interpret_correctly() {
    let shadows = crate::test_support::builtin_binding_program();
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for (source, configuration, _) in cases(target, &shadows) {
            for (optimize, profile) in STAGES {
                with_configured_module(
                    source,
                    target,
                    optimize,
                    profile,
                    configuration,
                    |module, target| {
                        assert_eq!(interpret(module, target).0, 0, "{source}");
                        if source != shadows {
                            assert!(!module.to_string().contains("dyn_alloca"));
                        }
                    },
                );
            }
        }
    }
}

#[test]
fn builtin_spelling_function_declarations_remain_runtime_symbols() {
    let source = "int __builtin_trap(int); int __builtin_abs(int); void *alloca(__SIZE_TYPE__); \
                  int f(void){alloca(7);return __builtin_trap(7)+__builtin_abs(7);}";
    for gnu in [false, true] {
        let target = Target::LinuxGnu;
        with_configured_module(
            source,
            target,
            false,
            Profile::PreAbi,
            CompilerConfiguration::default()
                .with_target(target)
                .with_gnu_extensions(gnu),
            |module, _| {
                let text = module.to_string();
                for name in ["__builtin_trap", "__builtin_abs", "alloca"] {
                    assert!(text.contains(&format!("call @{name}")), "{text}");
                }
                assert!(!text.contains("dyn_alloca"));
            },
        );
    }
}

#[test]
fn builtin_bindings_and_language_modes_match_native_clang() {
    let shadows = crate::test_support::builtin_binding_program();
    let Some(target) = host() else {
        return;
    };
    let directory = TempDir::new("builtin-bindings");
    let toolchain = Toolchain::bundled(target).unwrap();
    for (index, (source, configuration, flags)) in cases(target, &shadows).into_iter().enumerate() {
        let input = directory.join(format!("case-{index}.c"));
        std::fs::write(&input, source).unwrap();
        for opt in [OptLevel::O0, OptLevel::O2] {
            let options = LlvmOptions {
                opt: Some(opt),
                disable_llvm_passes: false,
                target,
                // A freestanding strlen definition must not interpose on
                // MinGW's hosted startup; validate its native object instead.
                kind: if configuration.hosted() {
                    OutputKind::Executable
                } else {
                    OutputKind::Object
                },
                code: crate::target::CodeOptions::default(),
                fast_math_runtime: false,
            };
            let clang = directory.join(format!("clang-{index}-{opt:?}.exe"));
            let mut command = toolchain.command(&input, &clang, options);
            _ = command.args(flags);
            let output = crate::test_support::native::run_native(&mut command).unwrap();
            assert!(
                output.status.success(),
                "{source}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let expected = configuration.hosted().then(|| run_native(&clang));
            if let Some(expected) = &expected {
                assert_eq!(expected.0, 0, "Clang: {source}");
            }
            for optimize in [false, true] {
                let executable = directory.join(format!("bcc-{index}-{opt:?}-{optimize}.exe"));
                with_configured_module(
                    source,
                    target,
                    optimize,
                    Profile::PostAbi,
                    configuration,
                    |module, _| compile_module(module, &executable, options).unwrap(),
                );
                if let Some(expected) = &expected {
                    assert_eq!(&run_native(&executable), expected, "bcc: {source}");
                }
            }
        }
    }
}

#[test]
fn statement_expression_internal_and_outgoing_jumps_execute() {
    for source in [
        "int main(void){int i=0;int n=({goto a;i=100;a:i++;i;});return n!=1;}",
        "int main(void){int i=0;({i++;goto out;i=100;});return 1;out:return i!=1;}",
        "int main(void){return ({({goto out;0;});out:;0;});}",
    ] {
        for target in [Target::LinuxGnu, Target::WindowsGnu] {
            for (optimize, profile) in STAGES {
                with_module(source, target, optimize, profile, |module, target| {
                    assert_eq!(interpret(module, target).0, 0);
                });
            }
        }
    }
}

#[test]
fn gnu89_inline_bodies_reach_llvm_on_the_msvc_target() {
    for target in [Target::WindowsMsvc, Target::WindowsGnu, Target::LinuxGnu] {
        for standard in [CStandard::C89, CStandard::C99] {
            for attribute in [false, true] {
                let source = format!(
                    "extern inline {}int f(void){{return 7;}} int main(void){{return f()!=7;}}",
                    if attribute {
                        "__attribute__((gnu_inline)) "
                    } else {
                        ""
                    }
                );
                let configuration = CompilerConfiguration::new(standard, ExtensionPolicy::Allow)
                    .with_target(target)
                    .with_gnu_extensions(true)
                    .with_gnu89_inline(true);
                with_configured_module(
                    &source,
                    target,
                    false,
                    Profile::PostAbi,
                    configuration,
                    |module, _| {
                        let mut llvm = String::new();
                        crate::backend::llvm::emit_module(module, target, &mut llvm).unwrap();
                        let defined = target == Target::WindowsMsvc && !attribute;
                        let definition =
                            |line: &str| line.starts_with("define") && line.contains("@f(");
                        assert_eq!(
                            llvm.lines()
                                .any(|line| definition(line)
                                    && !line.contains("available_externally")),
                            defined,
                            "{llvm}"
                        );
                        // An inline-only body is the external function's
                        // `available_externally` definition, as Clang emits
                        // it when optimizing (#109).
                        assert_eq!(
                            llvm.lines()
                                .any(|line| definition(line)
                                    && line.contains("available_externally")),
                            !defined,
                            "{llvm}"
                        );
                        if defined {
                            assert_eq!(interpret(module, target).0, 0);
                        }
                    },
                );
            }
        }
    }
}
