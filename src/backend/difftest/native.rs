//! The native oracles: a kept case, compiled through the LLVM back end and the
//! bundled clang, or through bcc's own native back end, and run on the host,
//! must behave as the interpreter says the original program does.
//!
//! Two comparison arms run per case: A0 compiles the original module with
//! `-O0`, and A1 compiles the module the default pipeline produced with O2 code
//! generation without IR passes, so LLVM's code generator runs but none of
//! its IR passes. The status `main` returns is compared unless the
//! interpreter says it is poison (only its low eight bits off Windows), and
//! the standard output always is.

use std::{
    path::Path,
    process::Command,
    thread,
};

use super::oracle::{
    Pipeline,
    Reference,
    optimized,
    verify_errors,
};
use crate::{
    abi::lower_abi,
    backend::llvm::{
        LlvmOptions,
        OptLevel,
        OutputKind,
        Toolchain,
        compile_module,
        emit_module,
    },
    ir::{
        Profile,
        parse_module,
    },
    target::Target,
    test_support::TempDir,
    util::bump::Bump,
};

/// Compiles and runs every case under both arms, a few clang processes at
/// a time, and returns a report for each disagreement. Without a supported
/// host or the bundled clang it explains why and checks nothing.
pub(super) fn check_native(cases: &[Reference]) -> Vec<String> {
    let Some(target) = host() else {
        eprintln!("skipping the native oracle: the host is not a supported target");
        return Vec::new();
    };
    let toolchain = match Toolchain::bundled(target) {
        | Ok(toolchain) => toolchain,
        | Err(error) => {
            eprintln!("skipping the native oracle: {error}");
            return Vec::new();
        },
    };
    let directory = TempDir::new("difftest");
    let jobs: Vec<(&Reference, Arm)> = cases
        .iter()
        .flat_map(|case| [Arm::A0, Arm::A1].map(|arm| (case, arm)))
        .collect();
    let parallelism = thread::available_parallelism().map_or(4, usize::from);
    let mut failures = Vec::new();
    for chunk in jobs.chunks(parallelism) {
        thread::scope(|scope| {
            let runs: Vec<_> = chunk
                .iter()
                .map(|&(case, arm)| {
                    let (toolchain, directory) = (&toolchain, &directory);
                    scope.spawn(move || check_arm(case, arm, target, toolchain, directory.path()))
                })
                .collect();
            for run in runs {
                if let Err(report) = run.join().expect("a native check does not panic") {
                    failures.push(report);
                }
            }
        });
    }
    failures
}

/// A comparison arm of `middle-end.md`.
#[derive(Clone, Copy, Debug)]
enum Arm {
    /// The original module, `-O0`.
    A0,
    /// The optimized module, O2 code generation without IR passes.
    A1,
}

/// Compiles and runs one case under one arm.
fn check_arm(
    case: &Reference,
    arm: Arm,
    target: Target,
    _toolchain: &Toolchain,
    directory: &Path,
) -> Result<(), String> {
    let text = match arm {
        | Arm::A0 => case.text.clone(),
        | Arm::A1 => optimized(&case.text, Pipeline::Default, None)
            .text
            .map_err(|error| format!("seed {:#x}: {error}", case.seed))?,
    };
    let llvm =
        emit(&text, target).map_err(|error| format!("seed {:#x} {arm:?}: {error}", case.seed))?;
    let name = format!("case-{:x}-{arm:?}", case.seed);
    let arena = Bump::new();
    let module = parse_module(&arena, &text).map_err(|error| error.to_string())?;
    let executable = directory.join(format!("{name}.exe"));
    let (opt, disable_llvm_passes) = match arm {
        | Arm::A0 => (OptLevel::O0, false),
        | Arm::A1 => (OptLevel::O2, true),
    };
    let options = LlvmOptions {
        opt: Some(opt),
        disable_llvm_passes,
        target,
        kind: OutputKind::Executable,
        code: crate::target::CodeOptions::default(),
    };
    compile_module(&module, &executable, options)
        .map_err(|error| format!("seed {:#x} {arm:?}: {error}\n{text}\n{llvm}", case.seed))?;
    let output = crate::test_support::native::run_native(&mut Command::new(&executable)).map_err(
        |error| {
            format!(
                "seed {:#x}, arm {arm:?}: {error}\nrerun with BCC_DIFFTEST_SEED={:#x} \
                 BCC_DIFFTEST_NATIVE_CASES=1\n\nthe compiled module:\n{text}\nits LLVM IR:\n{llvm}",
                case.seed, case.seed
            )
        },
    )?;
    let stdout = if cfg!(windows) {
        String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
    } else {
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    let expected_stdout = String::from_utf8_lossy(&case.observation.output);
    let status = output.status.code();
    let expected_status = case
        .exit_value()
        .map(|value| if cfg!(windows) { value } else { value & 0xFF });
    let status_matches = case.returns_poison() || status == expected_status;
    if status_matches && stdout == expected_stdout {
        return Ok(());
    }
    Err(format!(
        "seed {:#x}, arm {arm:?}: the interpreter says the original {}, but the executable exits \
         with {status:?} and prints {stdout:?}\nrerun with BCC_DIFFTEST_SEED={:#x} \
         BCC_DIFFTEST_NATIVE_CASES=1\n\nthe compiled module:\n{text}\nits LLVM IR:\n{llvm}",
        case.seed, case.observation, case.seed
    ))
}

/// Parses `text` and prints it as LLVM IR for `target`.
fn emit(text: &str, target: Target) -> Result<String, String> {
    let arena = Bump::new();
    let module = parse_module(&arena, text).map_err(|error| error.to_string())?;
    let errors = verify_errors(&module, Profile::PostAbi);
    if !errors.is_empty() {
        return Err(format!("the module does not verify post-ABI:\n{errors}"));
    }
    let mut out = String::new();
    emit_module(&module, target, &mut out).map_err(|error| error.to_string())?;
    Ok(out)
}

/// The target the tests run on, if the back end supports it.
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

/// The native back end's oracle: every kept case it accepts, compiled as
/// generated (arm B0) and after the default pipeline (arm B1), must behave
/// as the interpreter says. Returns the disagreements and how many arms the
/// back end refused; a refusal is a capability gap, not a failure.
pub(super) fn check_native_backend(cases: &[Reference]) -> (Vec<String>, usize) {
    let Some(target) = host() else {
        eprintln!("skipping the native back end oracle: the host is not a supported target");
        return (Vec::new(), 0);
    };
    if let Err(error) = Toolchain::bundled(target) {
        eprintln!("skipping the native back end oracle: {error}");
        return (Vec::new(), 0);
    }
    let directory = TempDir::new("difftest-native-backend");
    let jobs: Vec<(&Reference, bool)> = cases
        .iter()
        .flat_map(|case| [false, true].map(|optimize| (case, optimize)))
        .collect();
    let parallelism = thread::available_parallelism().map_or(4, usize::from);
    let (mut failures, mut refused) = (Vec::new(), 0);
    for chunk in jobs.chunks(parallelism) {
        thread::scope(|scope| {
            let runs: Vec<_> = chunk
                .iter()
                .map(|&(case, optimize)| {
                    let directory = &directory;
                    scope.spawn(move || check_backend_arm(case, optimize, target, directory.path()))
                })
                .collect();
            for run in runs {
                match run.join().expect("a native check does not panic") {
                    | Ok(true) => {},
                    | Ok(false) => refused += 1,
                    | Err(report) => failures.push(report),
                }
            }
        });
    }
    (failures, refused)
}

/// Compiles one case with the native back end and runs it: `Ok(false)` if
/// the back end refused it.
fn check_backend_arm(
    case: &Reference,
    optimize: bool,
    target: Target,
    directory: &Path,
) -> Result<bool, String> {
    let arm = if optimize { "B1" } else { "B0" };
    let text = if optimize {
        optimized(&case.text, Pipeline::Default, None)
            .text
            .map_err(|error| format!("seed {:#x}: {error}", case.seed))?
    } else {
        case.text.clone()
    };
    // The native back end takes a module lowered for its target's ABI, as
    // the pipeline hands it one. The generator writes no aggregate or
    // extension, but a hand-written case's `i128` still changes: Win64
    // passes it by reference.
    let arena = Bump::new();
    let mut module = parse_module(&arena, &text).map_err(|error| error.to_string())?;
    lower_abi(&mut module, target, &Bump::new())
        .map_err(|error| format!("seed {:#x} {arm}: {error}", case.seed))?;
    let errors = verify_errors(&module, Profile::PostAbi);
    if !errors.is_empty() {
        return Err(format!(
            "seed {:#x}: not a post-ABI module:\n{errors}",
            case.seed
        ));
    }
    let mut assembly = String::new();
    if crate::backend::native::emit_assembly(&module, target, &mut assembly).is_err() {
        return Ok(false);
    }
    let executable = directory.join(format!("case-{:x}-{arm}.exe", case.seed));
    crate::backend::native::assemble(&assembly, &executable, target, OutputKind::Executable)
        .map_err(|error| format!("seed {:#x} {arm}: {error}\n{assembly}", case.seed))?;
    let output = crate::test_support::native::run_native(&mut Command::new(&executable))
        .map_err(|error| format!("seed {:#x} {arm}: {error}\n{module}\n{assembly}", case.seed))?;
    let stdout = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    let expected_stdout = String::from_utf8_lossy(&case.observation.output);
    let status = output.status.code();
    let expected_status = case
        .exit_value()
        .map(|value| if cfg!(windows) { value } else { value & 0xFF });
    if (case.returns_poison() || status == expected_status) && stdout == expected_stdout {
        return Ok(true);
    }
    Err(format!(
        "seed {:#x}, arm {arm}: the interpreter says the original {}, but the native back end's \
         executable exits with {status:?} and prints {stdout:?}\nrerun with \
         BCC_DIFFTEST_SEED={:#x}\n\nthe compiled module:\n{module}\nits assembly:\n{assembly}",
        case.seed, case.observation, case.seed
    ))
}
