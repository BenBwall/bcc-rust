//! End-to-end runs: C programs lowered for a target, run in the interpreter
//! and compiled by the native back end, with the same status and output.
//! Windows executables run natively; System V ones run under WSL, assembled
//! and linked by WSL's clang. Without a host for a target, its runs are
//! skipped with a note.

use std::{
    path::Path,
    process::Command,
    thread,
};

use super::programs::PROGRAMS;
use crate::{
    abi::tests::programs::PROGRAMS as BY_VALUE_PROGRAMS,
    backend::{
        interpreter::{
            self,
            DefaultHost,
            RuntimeValue,
            Termination,
        },
        llvm::OutputKind,
        native::{
            assemble,
            emit_assembly,
        },
    },
    configuration::CompilerConfiguration,
    headers::HeaderSearch,
    ir::{
        Module,
        Profile,
    },
    lowering::tests::programs::PROGRAMS as LOWERING_PROGRAMS,
    optimizer::OptimizerOptions,
    pipeline::{
        analyze_translation_unit,
        lower_translation_unit,
        parse_translation_unit,
    },
    target::Target,
    test_support::{
        TempDir,
        native::run_native,
    },
    translation_phases::Context,
    util::bump::Bump,
};

/// What a program did: its exit status and standard output.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Run {
    status: i32,
    stdout: String,
}

/// Lowers `source` for `target` to the post-ABI IR, optionally through the
/// bcc pipeline, and hands the module to `then`.
fn with_module<R>(
    source: &str,
    target: Target,
    optimize: bool,
    then: impl FnOnce(&Module<'_>) -> R,
) -> R {
    let tu = Bump::new();
    let source = tu.alloc_str(source);
    let mut context =
        Context::with_configuration(&tu, CompilerConfiguration::default().with_target(target));
    let unit = parse_translation_unit(
        &mut context,
        Path::new("<test>"),
        source,
        HeaderSearch::default(),
    );
    let sema = analyze_translation_unit(&mut context, &unit);
    assert!(
        sema.lowerable(&context),
        "the front end accepts\n{source}\n{:?}",
        context.take_pending_errors()
    );
    let ir = Bump::new();
    let options = OptimizerOptions::default();
    let module = lower_translation_unit(
        &context,
        &unit,
        &sema,
        &ir,
        optimize.then_some(&options),
        Profile::PostAbi,
    )
    .unwrap_or_else(|error| panic!("the middle end failed: {error:?}\n{source}"));
    then(&module)
}

/// Runs `main` in the interpreter with the target's C library.
fn interpret(module: &Module<'_>, target: Target) -> Result<Run, String> {
    let arena = Bump::new();
    let mut host = DefaultHost::for_target(&arena, target);
    let outcome = interpreter::run_main(module, &mut host)
        .map_err(|trap| trap.display_in(module).to_string())?;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "`main` returns an `int`, held in the low 32 bits."
    )]
    let status = match outcome.termination {
        | Termination::Returned(Some(RuntimeValue::Int(bits))) => bits as u32 as i32,
        | Termination::Returned(_) => 0,
        | Termination::Exited(status) => status,
        | Termination::Aborted => 134,
    };
    Ok(Run {
        status,
        stdout: String::from_utf8_lossy(host.output()).into_owned(),
    })
}

/// How this host runs executables for a target, if it can.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Runner {
    Native,
    Wsl,
}

fn runner(target: Target) -> Option<Runner> {
    match target {
        | Target::WindowsGnu if cfg!(all(windows, target_env = "gnu")) => Some(Runner::Native),
        | Target::LinuxGnu if cfg!(all(target_os = "linux", target_env = "gnu")) =>
            Some(Runner::Native),
        | Target::LinuxGnu if cfg!(windows) && wsl_has_clang() => Some(Runner::Wsl),
        | _ => None,
    }
}

/// Whether WSL is installed with a clang to assemble and link.
fn wsl_has_clang() -> bool {
    run_native(Command::new("wsl.exe").args(["-e", "sh", "-c", "command -v clang"]))
        .is_ok_and(|output| output.status.success())
}

/// Assembles, links and runs `text` as `name` in `directory`.
fn run_assembly(
    text: &str,
    target: Target,
    runner: Runner,
    directory: &Path,
    name: &str,
) -> Result<Run, String> {
    let output = match runner {
        | Runner::Native => {
            let executable = directory.join(format!("{name}.exe"));
            assemble(text, &executable, target, OutputKind::Executable)
                .map_err(|error| error.to_string())?;
            run_native(&mut Command::new(&executable)).map_err(|error| error.to_string())?
        },
        | Runner::Wsl => {
            std::fs::write(directory.join(format!("{name}.s")), text)
                .map_err(|error| error.to_string())?;
            let build = run_native(
                Command::new("wsl.exe")
                    .args(["-e", "clang", "-o", name, &format!("{name}.s")])
                    .current_dir(directory),
            )
            .map_err(|error| error.to_string())?;
            if !build.status.success() {
                return Err(format!(
                    "WSL clang failed:\n{}",
                    String::from_utf8_lossy(&build.stderr)
                ));
            }
            run_native(
                Command::new("wsl.exe")
                    .args(["-e", &format!("./{name}")])
                    .current_dir(directory),
            )
            .map_err(|error| error.to_string())?
        },
    };
    let status = output.status.code().ok_or("the program was killed")?;
    Ok(Run {
        status,
        stdout: String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"),
    })
}

/// One compilation to check: a program, a target, and whether the bcc
/// pipeline runs first.
struct Job<'a> {
    name:     &'a str,
    source:   &'a str,
    target:   Target,
    runner:   Runner,
    optimize: bool,
}

/// What one job found.
enum Verdict {
    Passed,
    /// The native back end refused the module.
    Refused(String),
    /// The interpreter could not run the program, so there is no oracle.
    NoOracle,
    Failed(String),
}

/// Runs a job: the interpreter's run against the native executable's.
fn check(job: &Job<'_>, directory: &Path) -> Verdict {
    let compiled = with_module(job.source, job.target, job.optimize, |module| {
        let expected = interpret(module, job.target);
        let mut text = String::new();
        let assembly =
            emit_assembly(module, job.target, &mut text).map_err(|error| error.to_string());
        (expected, assembly.map(|()| text))
    });
    let (expected, text) = match compiled {
        | (_, Err(refusal)) => return Verdict::Refused(refusal),
        | (Err(_), _) => return Verdict::NoOracle,
        | (Ok(expected), Ok(text)) => (expected, text),
    };
    let name = format!(
        "{}-{:?}-{}",
        job.name,
        job.target,
        if job.optimize { "bcc" } else { "none" }
    );
    match run_assembly(&text, job.target, job.runner, directory, &name) {
        | Ok(actual) => {
            // Exit statuses travel in eight bits off Windows.
            let status_matches = match job.runner {
                | Runner::Wsl => actual.status == expected.status & 0xFF,
                | Runner::Native => actual.status == expected.status,
            };
            if status_matches && actual.stdout == expected.stdout {
                Verdict::Passed
            } else {
                Verdict::Failed(format!(
                    "{name}: the interpreter says {expected:?}, the native executable did \
                     {actual:?}\n{text}"
                ))
            }
        },
        | Err(error) => Verdict::Failed(format!("{name}: {error}\n{text}")),
    }
}

/// Runs every job, several at a time, and returns the verdicts in order.
fn check_all(jobs: &[Job<'_>], directory: &Path) -> Vec<Verdict> {
    let parallelism = thread::available_parallelism()
        .map_or(4, usize::from)
        .min(8);
    let mut verdicts = Vec::with_capacity(jobs.len());
    for chunk in jobs.chunks(parallelism) {
        thread::scope(|scope| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|job| scope.spawn(move || check(job, directory)))
                .collect();
            for handle in handles {
                verdicts.push(handle.join().expect("a check does not panic"));
            }
        });
    }
    verdicts
}

/// The targets this host can run, with their runners.
fn targets() -> Vec<(Target, Runner)> {
    let mut targets = Vec::new();
    for target in [Target::WindowsGnu, Target::LinuxGnu] {
        match runner(target) {
            | Some(runner) => targets.push((target, runner)),
            | None => eprintln!("skipping {target:?}: this host cannot run its executables"),
        }
    }
    targets
}

#[test]
fn native_programs_match_the_interpreter() {
    if crate::backend::llvm::Toolchain::bundled(Target::WindowsGnu).is_err() && cfg!(windows) {
        eprintln!("skipping: no bundled toolchain");
        return;
    }
    let directory = TempDir::new("native-programs");
    let mut jobs = Vec::new();
    for (target, runner) in targets() {
        for program in PROGRAMS {
            for optimize in [false, true] {
                jobs.push(Job {
                    name: program.name,
                    source: program.source,
                    target,
                    runner,
                    optimize,
                });
            }
        }
    }
    let mut failures = Vec::new();
    for (job, verdict) in jobs.iter().zip(check_all(&jobs, directory.path())) {
        match verdict {
            | Verdict::Passed => {},
            | Verdict::Refused(refusal) =>
                failures.push(format!("{} {:?}: refused: {refusal}", job.name, job.target)),
            | Verdict::NoOracle => failures.push(format!(
                "{} {:?}: the interpreter traps",
                job.name, job.target
            )),
            | Verdict::Failed(failure) => failures.push(failure),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn accepted_lowering_programs_match_the_interpreter() {
    let directory = TempDir::new("native-lowering-programs");
    let mut jobs = Vec::new();
    for (target, runner) in targets() {
        for program in LOWERING_PROGRAMS.iter().chain(BY_VALUE_PROGRAMS) {
            for optimize in [false, true] {
                jobs.push(Job {
                    name: program.name,
                    source: program.source,
                    target,
                    runner,
                    optimize,
                });
            }
        }
    }
    if jobs.is_empty() {
        return;
    }
    let mut failures = Vec::new();
    let (mut passed, mut refused) = (0, 0);
    for (job, verdict) in jobs.iter().zip(check_all(&jobs, directory.path())) {
        match verdict {
            | Verdict::Passed => passed += 1,
            | Verdict::Refused(_) | Verdict::NoOracle => refused += 1,
            | Verdict::Failed(failure) =>
                failures.push(format!("{} {:?}: {failure}", job.name, job.target)),
        }
    }
    eprintln!(
        "{passed} native runs matched the interpreter; {refused} were refused or had no oracle"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    assert!(
        passed * 2 >= jobs.len(),
        "most lowering programs are integer-only: {passed} of {}",
        jobs.len()
    );
}

/// Instructions C lowering rarely produces, at every width: unsigned
/// overflow predicates, `i1` arithmetic and selects, bit counts of zero,
/// narrow byte swaps, poison and freeze, and switch edges with arguments.
const IR_PROGRAM: &str = "abi lowered
global @format internal constant size 4, align 1 = bytes \"25640a00\"
function @printf(ptr, ...) -> i32 external
function @overflow(i8, i8, i16, i16, i32, i32, i64, i64) -> i32 internal {
block0(v0: i8, v1: i8, v2: i16, v3: i16, v4: i32, v5: i32, v6: i64, v7: i64):
    v8 = uadd_overflow.i8 v0, v1
    v9 = umul_overflow.i8 v0, v1
    v10 = uadd_overflow.i16 v2, v3
    v11 = umul_overflow.i16 v2, v3
    v12 = uadd_overflow.i32 v4, v5
    v13 = umul_overflow.i32 v4, v5
    v14 = uadd_overflow.i64 v6, v7
    v15 = umul_overflow.i64 v6, v7
    v16 = zext.i32 v8
    v17 = zext.i32 v9
    v18 = zext.i32 v10
    v19 = zext.i32 v11
    v20 = zext.i32 v12
    v21 = zext.i32 v13
    v22 = zext.i32 v14
    v23 = zext.i32 v15
    v24 = iconst.i32 1
    v25 = shl.i32 v17, v24
    v26 = iconst.i32 2
    v27 = shl.i32 v18, v26
    v28 = iconst.i32 3
    v29 = shl.i32 v19, v28
    v30 = iconst.i32 4
    v31 = shl.i32 v20, v30
    v32 = iconst.i32 5
    v33 = shl.i32 v21, v32
    v34 = iconst.i32 6
    v35 = shl.i32 v22, v34
    v36 = iconst.i32 7
    v37 = shl.i32 v23, v36
    v38 = or.i32 v16, v25
    v39 = or.i32 v38, v27
    v40 = or.i32 v39, v29
    v41 = or.i32 v40, v31
    v42 = or.i32 v41, v33
    v43 = or.i32 v42, v35
    v44 = or.i32 v43, v37
    return v44
}

function @bits(i8, i16, i32, i64) -> i32 internal {
block0(v0: i8, v1: i16, v2: i32, v3: i64):
    v4 = popcount.i8 v0
    v5 = popcount.i16 v1
    v6 = clz_defined.i32 v2
    v7 = ctz_defined.i64 v3
    v8 = clz_defined.i8 v0
    v9 = ctz_defined.i16 v1
    v10 = bswap.i16 v1
    v11 = zext.i32 v4
    v12 = zext.i32 v5
    v13 = trunc.i32 v7
    v14 = zext.i32 v8
    v15 = zext.i32 v9
    v16 = zext.i32 v10
    v17 = iadd.i32 v11, v12
    v18 = iadd.i32 v17, v6
    v19 = iadd.i32 v18, v13
    v20 = iadd.i32 v19, v14
    v21 = iadd.i32 v20, v15
    v22 = xor.i32 v21, v16
    return v22
}

function @booleans(i32, i32) -> i32 internal {
block0(v0: i32, v1: i32):
    v2 = icmp.i32 slt v0, v1
    v3 = icmp.i32 eq v0, v1
    v4 = iadd.i1 v2, v3
    v5 = xor.i1 v4, v2
    v6 = trunc.i1 v0
    v7 = select.i1 v6, v5, v3
    v8 = sext.i32 v7
    v9 = select.i32 v2, v0, v1
    v10 = poison.i32
    v11 = freeze.i32 v10
    v12 = isub.i32 v11, v11
    v13 = iadd.i32 v8, v9
    v14 = iadd.i32 v13, v12
    return v14
}

function @route(i32) -> i32 internal {
block0(v0: i32):
    v1 = iconst.i32 100
    switch v0, block1(v0, v1), [1: block1(v1, v0), 2: block2, 3: block1(v0, v0)]
block1(v2: i32, v3: i32):
    v4 = isub.i32 v2, v3
    return v4
block2:
    v5 = iconst.i32 -7
    return v5
}

function @main() -> i32 external {
block0:
    v0 = global_addr @format
    v1 = iconst.i8 200
    v2 = iconst.i8 100
    v3 = iconst.i16 65000
    v4 = iconst.i16 600
    v5 = iconst.i32 4000000000
    v6 = iconst.i32 300000000
    v7 = iconst.i64 18446744073709551000
    v8 = iconst.i64 999
    v9 = call @overflow(v1, v2, v3, v4, v5, v6, v7, v8)
    v10 = call @printf(v0, v9)
    v11 = iconst.i8 3
    v12 = iconst.i8 4
    v13 = iconst.i16 255
    v14 = iconst.i16 255
    v15 = iconst.i32 65536
    v16 = iconst.i32 65535
    v17 = iconst.i64 4294967296
    v18 = iconst.i64 4294967295
    v19 = call @overflow(v11, v12, v13, v14, v15, v16, v17, v18)
    v20 = call @printf(v0, v19)
    v21 = iconst.i8 16
    v22 = iconst.i8 16
    v23 = iconst.i16 256
    v24 = iconst.i16 256
    v25 = iconst.i32 65536
    v26 = iconst.i32 65536
    v27 = iconst.i64 4294967296
    v28 = iconst.i64 4294967296
    v29 = call @overflow(v21, v22, v23, v24, v25, v26, v27, v28)
    v30 = call @printf(v0, v29)
    v31 = iconst.i8 0
    v32 = iconst.i16 0
    v33 = iconst.i32 0
    v34 = iconst.i64 0
    v35 = call @bits(v31, v32, v33, v34)
    v36 = call @printf(v0, v35)
    v37 = iconst.i8 129
    v38 = iconst.i16 4660
    v39 = iconst.i32 1
    v40 = iconst.i64 9223372036854775808
    v41 = call @bits(v37, v38, v39, v40)
    v42 = call @printf(v0, v41)
    v43 = iconst.i8 255
    v44 = iconst.i16 65535
    v45 = iconst.i32 4294967295
    v46 = iconst.i64 12
    v47 = call @bits(v43, v44, v45, v46)
    v48 = call @printf(v0, v47)
    v49 = iconst.i32 -5
    v50 = iconst.i32 3
    v51 = call @booleans(v49, v50)
    v52 = call @printf(v0, v51)
    v53 = iconst.i32 4
    v54 = iconst.i32 4
    v55 = call @booleans(v53, v54)
    v56 = call @printf(v0, v55)
    v57 = iconst.i32 7
    v58 = iconst.i32 -9
    v59 = call @booleans(v57, v58)
    v60 = call @printf(v0, v59)
    v61 = iconst.i32 0
    v62 = call @route(v61)
    v63 = call @printf(v0, v62)
    v64 = iconst.i32 1
    v65 = call @route(v64)
    v66 = call @printf(v0, v65)
    v67 = iconst.i32 2
    v68 = call @route(v67)
    v69 = call @printf(v0, v68)
    v70 = iconst.i32 3
    v71 = call @route(v70)
    v72 = call @printf(v0, v71)
    v73 = iconst.i32 0
    return v73
}
";

#[test]
fn ir_programs_match_the_interpreter() {
    let directory = TempDir::new("native-ir");
    for (target, runner) in targets() {
        let arena = Bump::new();
        let module = crate::ir::parse_module(&arena, IR_PROGRAM).unwrap();
        let expected = interpret(&module, target).unwrap();
        let mut text = String::new();
        emit_assembly(&module, target, &mut text).unwrap();
        let name = format!("ir-{target:?}");
        let actual = run_assembly(&text, target, runner, directory.path(), &name).unwrap();
        assert_eq!(
            actual, expected,
            "{target:?}
{text}"
        );
    }
}
