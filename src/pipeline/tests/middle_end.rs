//! End-to-end tests of the middle end. Every lowering test program and every
//! program passing structures by value runs in the interpreter, as lowered
//! and after the default bcc pipeline, before and after ABI lowering, and
//! must return its expected status. The structure programs and a subset of
//! the others are compiled through the bundled clang in each comparison arm
//! of `middle-end.md` after ABI lowering, as the driver does, run natively,
//! and compared with the interpreter.
//!
//! Each clang run costs about 0.4 s, so the arms run side by side. Without
//! the bundled clang, or on a host the LLVM back end does not target, the
//! LLVM tests print why and pass.

use std::{
    path::Path,
    process::Command,
    thread,
};

mod asm_labels;
mod braced_strings;
mod declaration_initializers;
mod runtime_calls;
mod union_initializers;

use crate::{
    abi::tests::programs::PROGRAMS as BY_VALUE_PROGRAMS,
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
            compile_module,
        },
    },
    configuration::CompilerConfiguration,
    headers::HeaderSearch,
    ir::{
        Module,
        Profile,
        parse_module,
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

/// Compiles `source` for `target` through lowering, with `optimize` the
/// default bcc pipeline, and for [`Profile::PostAbi`] ABI lowering, verifies
/// the module for `profile` and hands it to `then` with the target.
fn with_module<R>(
    source: &str,
    target: Target,
    optimize: bool,
    profile: Profile,
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
    let module = lower_translation_unit(
        &context,
        &unit,
        &sema,
        &ir,
        optimize.then_some(&options),
        profile,
    )
    .unwrap_or_else(|error| panic!("the middle end failed: {error:?}\n{source}"));
    assert_eq!(module.profile(), profile);
    let verifier = Bump::new();
    let errors = verify_module(&module, profile, &verifier);
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

#[test]
fn c_unicode_identifiers_round_trip_through_ir_text() {
    for source in [
        "int café(void){return 1;}",
        r"int caf\u00e9(void){return 1;}",
    ] {
        with_module(
            source,
            Target::LinuxGnu,
            false,
            Profile::PreAbi,
            |module, _| {
                assert!(module.symbol("café").is_some());
                let printed = module.to_string();
                let arena = Bump::new();
                let parsed = parse_module(&arena, &printed).unwrap();
                assert_eq!(parsed.to_string(), printed);
            },
        );
    }
}

/// The program called `name`.
fn program(name: &str) -> &'static Program {
    PROGRAMS
        .iter()
        .chain(BY_VALUE_PROGRAMS)
        .find(|program| program.name == name)
        .unwrap_or_else(|| panic!("no program {name}"))
}

/// Each combination of running the bcc pipeline and lowering for the ABI.
const STAGES: [(bool, Profile); 4] = [
    (false, Profile::PreAbi),
    (false, Profile::PostAbi),
    (true, Profile::PreAbi),
    (true, Profile::PostAbi),
];

const REINITIALIZATION_PROGRAMS: &[(&str, &str, i32)] = &[
    (
        "repeated_braces",
        "int main(void) { int n = 0; again:; int a[1][2] = {[0] = {n ? a[0][0] + 1 : 1}}; if \
         (!n++) goto again; return a[0][0] + a[0][1]; }",
        2,
    ),
    (
        "union_read",
        "union U { int old; struct { int x, y; } next; }; int main(void) { int n = 0; again:; \
         union U a = {.next = {n ? a.next.x + 1 : 1}}; if (!n++) goto again; return a.next.x + \
         a.next.y; }",
        2,
    ),
    (
        "array",
        "int main(void) { int n = 0; again:; int a[1] = {n ? a[0] + 1 : 1}; if (!n++) goto again; \
         return a[0]; }",
        2,
    ),
    (
        "struct",
        "struct S { int x, y; }; int main(void) { int n = 0; again:; struct S a = {n ? a.x + 1 : \
         1}; if (!n++) goto again; return a.x + a.y; }",
        2,
    ),
    (
        "nested",
        "struct S { int x[2]; int y; }; int main(void) { int n = 0; again:; struct S a = {{n ? \
         a.x[0] + 1 : 1}}; if (!n++) goto again; return a.x[0] + a.x[1] + a.y; }",
        2,
    ),
    (
        "designated",
        "int main(void) { int n = 0; again:; int a[4] = {[2] = n ? a[2] + 1 : 1, [0] = n ? a[0] + \
         2 : 3}; if (!n++) goto again; return a[0] + a[1] + a[2] + a[3]; }",
        7,
    ),
    (
        "declarator_order",
        "int main(void) { int *p = 0; int n = 0; again:; int a = p ? *p : 0, b[1] = {9}; p = b; \
         if (!n++) goto again; return a; }",
        9,
    ),
    (
        "union_switch",
        "union U { int old; struct { int x, y; } next; }; int main(void) { union U a = {.old = \
         -1, .next.x = 1}; return a.next.x + a.next.y; }",
        1,
    ),
    (
        "string_replacement",
        "int main(void) { char a[2][4] = {[0] = \"abc\", [0] = \"x\"}; return a[0][2]; }",
        0,
    ),
];

#[test]
fn aggregate_reinitialization_preserves_explicit_bytes() {
    for &(name, source, expected) in REINITIALIZATION_PROGRAMS {
        for target in [Target::LinuxGnu, Target::WindowsGnu] {
            for (optimize, profile) in STAGES {
                let actual = with_module(source, target, optimize, profile, interpret);
                assert_eq!(
                    actual.0, expected,
                    "{name} {target:?} {optimize} {profile:?}"
                );
            }
        }
    }
}

#[test]
fn aggregate_reinitialization_matches_native_arms_and_clang() {
    let Some(target) = host() else { return };
    let toolchain = Toolchain::bundled(target).unwrap();
    let directory = TempDir::new("aggregate-reinitialization");
    for &(name, source, expected) in REINITIALIZATION_PROGRAMS {
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
            assert_eq!(run_native(&executable).0, expected, "{name} {}", arm.name);
        }
        let input = directory.join(format!("{name}.c"));
        std::fs::write(&input, source).unwrap();
        for opt in [OptLevel::O0, OptLevel::O2] {
            let executable = directory.join(format!("{name}-clang{opt:?}.exe"));
            let output = toolchain
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
            assert!(output.status.success(), "{output:?}");
            assert_eq!(run_native(&executable).0, expected, "{name} clang {opt:?}");
        }
    }
}

#[test]
fn inline_handles_nested_calls_address_taken_locals_and_multiple_returns() {
    let source = "
static void write(int *p, int x) { *p = x; }
static int absolute(int x) {
    int y;
    write(&y, x);
    if (y < 0) return -y;
    return y;
}
static int nested(int x) { return absolute(x) + absolute(x + 1); }
int main(void) {
    int sum = 0;
    for (int i = -4; i < 5; ++i) sum += nested(i);
    return sum == 41 ? 0 : 1;
}";
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for optimize in [false, true] {
            with_module(
                source,
                target,
                optimize,
                Profile::PreAbi,
                |module, target| {
                    assert_eq!(interpret(module, target).0, 0);
                    if optimize {
                        let text = module.to_string();
                        assert!(!text.contains("call @write"), "{text}");
                        assert!(!text.contains("call @absolute"), "{text}");
                        assert!(!text.contains("call @nested"), "{text}");
                        assert!(!text.contains("stack_slot"), "{text}");
                    }
                },
            );
        }
    }
}

#[test]
fn every_program_interprets_to_its_status() {
    for program in PROGRAMS.iter().chain(BY_VALUE_PROGRAMS) {
        for (optimize, profile) in STAGES {
            let (status, _) = with_module(
                program.source,
                Target::LinuxGnu,
                optimize,
                profile,
                interpret,
            );
            assert_eq!(
                status, program.expected,
                "{} (optimized: {optimize}, {profile:?})",
                program.name
            );
        }
    }
}

#[test]
fn structures_by_value_interpret_to_their_status_on_win64() {
    for program in BY_VALUE_PROGRAMS {
        for (optimize, profile) in STAGES {
            let (status, _) = with_module(
                program.source,
                Target::WindowsGnu,
                optimize,
                profile,
                interpret,
            );
            assert_eq!(
                status, program.expected,
                "{} (optimized: {optimize}, {profile:?})",
                program.name
            );
        }
    }
}

#[test]
fn printf_output_is_captured_by_the_interpreter() {
    for (optimize, profile) in STAGES {
        let (status, output) = with_module(
            program("printf_hello").source,
            Target::LinuxGnu,
            optimize,
            profile,
            interpret,
        );
        assert_eq!((status, output.as_str()), (0, "answer 42\n"));
        // `%ld` reads a `long` at the target's width.
        for (target, expected) in [
            (Target::LinuxGnu, "-5 3000000000 8\n"),
            (Target::WindowsGnu, "-5 3000000000 4\n"),
        ] {
            let (_, output) = with_module(
                program("printf_long").source,
                target,
                optimize,
                profile,
                interpret,
            );
            assert_eq!(output, expected, "{target:?} {profile:?}");
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

/// The programs compiled through LLVM besides those passing structures by
/// value: loops, recursion, memory, globals, control flow, indirect calls,
/// and output through `printf`.
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
    let output = crate::test_support::native::run_native(&mut Command::new(executable))
        .unwrap_or_else(|error| panic!("{error}"));
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
    let _toolchain = match Toolchain::bundled(target) {
        | Ok(toolchain) => toolchain,
        | Err(error) => {
            eprintln!("skipping: {error}");
            return;
        },
    };
    let directory = TempDir::new("middle-end-arms");
    let mut runs = Vec::new();
    let names = LLVM_PROGRAMS
        .into_iter()
        .chain(BY_VALUE_PROGRAMS.iter().map(|program| program.name));
    for name in names {
        let source = program(name).source;
        for arm in ARMS {
            runs.push((name, arm, source));
        }
    }
    thread::scope(|scope| {
        let handles: Vec<_> = runs
            .iter()
            .map(|(name, arm, source)| {
                let directory = &directory;
                scope.spawn(move || {
                    let executable = directory.join(format!("{name}-{}.exe", arm.name));
                    let options = LlvmOptions {
                        opt: Some(arm.opt),
                        disable_llvm_passes: arm.disable_llvm_passes,
                        target,
                        kind: OutputKind::Executable,
                    };
                    let expected = with_module(
                        source,
                        target,
                        arm.bcc,
                        Profile::PostAbi,
                        |module, target| {
                            compile_module(module, &executable, options)
                                .unwrap_or_else(|error| panic!("{name} {}: {error}", arm.name));
                            interpret(module, target)
                        },
                    );
                    (name, arm, expected, run_native(&executable))
                })
            })
            .collect();
        for handle in handles {
            let (name, arm, expected, actual) = handle.join().unwrap();
            assert_eq!(actual, expected, "{name} {}", arm.name);
            assert_eq!(actual.0, program(name).expected, "{name} {}", arm.name);
        }
    });
}
