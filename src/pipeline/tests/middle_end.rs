//! End-to-end tests of the middle end. Every lowering test program runs in
//! the interpreter, as lowered and after the default bcc pipeline, and must
//! return its expected status. A subset is compiled through the bundled
//! clang in each comparison arm of `middle-end.md`, run natively, and
//! compared with the interpreter.
//!
//! Each clang run costs about 0.4 s, so the arms run side by side. Without
//! the bundled clang, or on a host the LLVM back end does not target, the
//! LLVM tests print why and pass.

use std::{
    path::Path,
    process::Command,
    thread,
};

use crate::{
    backend::{
        interpreter::{
            self,
            DefaultHost,
            RuntimeValue,
            Termination,
        },
        llvm::{
            LlvmOptions,
            OptLevel,
            OutputKind,
            Toolchain,
            emit_module,
        },
    },
    configuration::CompilerConfiguration,
    headers::HeaderSearch,
    ir::{
        Module,
        Profile,
        verify_module,
    },
    lowering::tests::programs::{
        PROGRAMS,
        Program,
    },
    optimizer::OptimizerOptions,
    pipeline::{
        analyze_translation_unit,
        lower_translation_unit,
        parse_translation_unit,
    },
    target::Target,
    test_support::TempDir,
    translation_phases::Context,
    util::bump::Bump,
};

/// Compiles `source` for `target` through lowering and, with `optimize`,
/// the default bcc pipeline, verifies the module and hands it to `then` with
/// the target.
fn with_module<R>(
    source: &str,
    target: Target,
    optimize: bool,
    then: impl FnOnce(&Module<'_>, Target) -> R,
) -> R {
    let tu = Bump::new();
    let source = tu.alloc_str(source);
    let configuration = CompilerConfiguration::default().with_target(target);
    let mut context = Context::with_configuration(&tu, configuration);
    let unit = parse_translation_unit(
        &mut context,
        Path::new("<test>"),
        source,
        HeaderSearch::default(),
    );
    let sema = analyze_translation_unit(&mut context, &unit);
    assert!(sema.lowerable(&context), "the front end accepts\n{source}");
    let ir = Bump::new();
    let options = OptimizerOptions::default();
    let module = lower_translation_unit(&context, &unit, &sema, &ir, optimize.then_some(&options))
        .unwrap_or_else(|errors| panic!("lowering failed: {errors:?}\n{source}"));
    let verifier = Bump::new();
    let errors = verify_module(&module, Profile::PreAbi, &verifier);
    assert!(errors.is_empty(), "{errors:?}\n{module}");
    then(&module, target)
}

/// Runs `main` of `module` in the interpreter with the C library of `target`,
/// and returns its exit status and output.
fn interpret(module: &Module<'_>, target: Target) -> (i32, String) {
    let arena = Bump::new();
    let mut host = DefaultHost::for_target(&arena, target);
    let outcome = interpreter::run(module, "main", &[], &mut host)
        .unwrap_or_else(|trap| panic!("{}\n{module}", trap.display_in(module)));
    #[expect(
        clippy::cast_possible_truncation,
        reason = "`main` returns an `int`, held in the low 32 bits."
    )]
    let status = match outcome.termination {
        | Termination::Returned(Some(RuntimeValue::Int(bits))) => bits as u32 as i32,
        | Termination::Exited(status) => status,
        | termination => panic!("main ended with {termination:?}"),
    };
    let output = String::from_utf8(host.output().to_vec()).expect("the output is UTF-8");
    (status, output)
}

/// The program called `name`.
fn program(name: &str) -> &'static Program {
    PROGRAMS
        .iter()
        .find(|program| program.name == name)
        .unwrap_or_else(|| panic!("no program {name}"))
}

#[test]
fn every_program_interprets_to_its_status() {
    for program in PROGRAMS {
        for optimize in [false, true] {
            let (status, _) = with_module(program.source, Target::LinuxGnu, optimize, interpret);
            assert_eq!(
                status, program.expected,
                "{} (optimized: {optimize})",
                program.name
            );
        }
    }
}

#[test]
fn printf_output_is_captured_by_the_interpreter() {
    for optimize in [false, true] {
        let (status, output) = with_module(
            program("printf_hello").source,
            Target::LinuxGnu,
            optimize,
            interpret,
        );
        assert_eq!((status, output.as_str()), (0, "answer 42\n"));
        // `%ld` reads a `long` at the target's width.
        for (target, expected) in [
            (Target::LinuxGnu, "-5 3000000000 8\n"),
            (Target::WindowsGnu, "-5 3000000000 4\n"),
        ] {
            let (_, output) =
                with_module(program("printf_long").source, target, optimize, interpret);
            assert_eq!(output, expected, "{target:?}");
        }
    }
}

/// A comparison arm of `middle-end.md`: whether the bcc pipeline runs, and
/// how clang is invoked.
#[derive(Clone, Copy, Debug)]
struct Arm {
    name:                &'static str,
    bcc:                 bool,
    opt:                 OptLevel,
    disable_llvm_passes: bool,
}

const ARMS: [Arm; 4] = [
    Arm {
        name:                "a0",
        bcc:                 false,
        opt:                 OptLevel::O0,
        disable_llvm_passes: false,
    },
    Arm {
        name:                "a1",
        bcc:                 true,
        opt:                 OptLevel::O2,
        disable_llvm_passes: true,
    },
    Arm {
        name:                "a2",
        bcc:                 false,
        opt:                 OptLevel::O2,
        disable_llvm_passes: false,
    },
    Arm {
        name:                "a3",
        bcc:                 true,
        opt:                 OptLevel::O2,
        disable_llvm_passes: false,
    },
];

/// The programs compiled through LLVM: loops, recursion, memory, globals,
/// control flow, indirect calls, and output through `printf`.
const LLVM_PROGRAMS: [&str; 8] = [
    "sum_to_ten",
    "recursive_fibonacci",
    "bubble_sort_local_array",
    "short_circuit_side_effects",
    "globals_and_static_locals",
    "function_pointers",
    "printf_hello",
    "printf_long",
];

/// The target the tests run on, if the LLVM back end supports it.
fn host() -> Option<Target> {
    if cfg!(all(windows, target_env = "gnu")) {
        Some(Target::WindowsGnu)
    } else if cfg!(all(windows, target_env = "msvc")) {
        Some(Target::WindowsMsvc)
    } else if cfg!(all(target_os = "linux", target_env = "gnu")) {
        Some(Target::LinuxGnu)
    } else if cfg!(all(target_os = "linux", target_env = "musl")) {
        Some(Target::LinuxMusl)
    } else {
        None
    }
}

/// Runs an executable, returning its exit status and standard output with
/// Windows line ends made plain.
fn run_native(executable: &Path) -> (i32, String) {
    let output = Command::new(executable).output().unwrap();
    let stdout = String::from_utf8(output.stdout)
        .unwrap()
        .replace("\r\n", "\n");
    (output.status.code().unwrap(), stdout)
}

#[test]
fn llvm_arms_match_the_interpreter() {
    let Some(target) = host() else {
        eprintln!("skipping: the host is not a supported target");
        return;
    };
    let toolchain = match Toolchain::bundled(target) {
        | Ok(toolchain) => toolchain,
        | Err(error) => {
            eprintln!("skipping: {error}");
            return;
        },
    };
    let directory = TempDir::new("middle-end-arms");
    let mut runs = Vec::new();
    for name in LLVM_PROGRAMS {
        let source = program(name).source;
        for arm in ARMS {
            let (expected, ll) = with_module(source, target, arm.bcc, |module, target| {
                let mut text = String::new();
                emit_module(module, target, &mut text)
                    .unwrap_or_else(|error| panic!("{name} {}: {error}", arm.name));
                (interpret(module, target), text)
            });
            let path = directory.join(format!("{name}-{}.ll", arm.name));
            std::fs::write(&path, ll).unwrap();
            runs.push((name, arm, expected, path));
        }
    }
    thread::scope(|scope| {
        let handles: Vec<_> = runs
            .iter()
            .map(|(name, arm, expected, ll)| {
                let toolchain = &toolchain;
                let directory = &directory;
                scope.spawn(move || {
                    let executable = directory.join(format!("{name}-{}.exe", arm.name));
                    let options = LlvmOptions {
                        opt: Some(arm.opt),
                        disable_llvm_passes: arm.disable_llvm_passes,
                        target,
                        kind: OutputKind::Executable,
                    };
                    toolchain
                        .compile_ll(ll, &executable, options)
                        .unwrap_or_else(|error| panic!("{name} {}: {error}", arm.name));
                    (name, arm, expected, run_native(&executable))
                })
            })
            .collect();
        for handle in handles {
            let (name, arm, expected, actual) = handle.join().unwrap();
            assert_eq!(&actual, expected, "{name} {}", arm.name);
            assert_eq!(actual.0, program(name).expected, "{name} {}", arm.name);
        }
    });
}
