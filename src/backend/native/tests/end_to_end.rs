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

use super::{
    programs::{
        PROGRAMS,
        SSE_UNSIGNED_CONVERSIONS,
        SSE_UNSIGNED_CONVERSIONS_OUTPUT,
        WIDE_FLOAT_PROGRAMS,
    },
    spilled_assembly,
};
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
            OutputKind,
            compile_module,
        },
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

/// One compilation to check: a program, a target, whether the bcc
/// pipeline runs first, and whether every value is spilled (see
/// `spill_everything`).
struct Job<'a> {
    name:     &'a str,
    source:   &'a str,
    target:   Target,
    runner:   Runner,
    optimize: bool,
    spill:    bool,
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
        let assembly = if job.spill {
            spilled_assembly(module, job.target)
        } else {
            let mut text = String::new();
            emit_assembly(module, job.target, &mut text)
                .map(|()| text)
                .map_err(|error| error.to_string())
        };
        (expected, assembly)
    });
    let (expected, text) = match compiled {
        | (_, Err(refusal)) => return Verdict::Refused(refusal),
        | (Err(_), _) => return Verdict::NoOracle,
        | (Ok(expected), Ok(text)) => (expected, text),
    };
    let name = format!(
        "{}-{:?}-{}{}",
        job.name,
        job.target,
        if job.optimize { "bcc" } else { "none" },
        if job.spill { "-spilled" } else { "" }
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
                    spill: false,
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

/// Every program again with every value in a spill slot for its whole
/// life, so that every operand of every instruction the selector makes
/// goes through the spill code: an indexed store or ALU operation then
/// reads three spilled general registers, and its address goes through a
/// `lea` into a scratch register.
#[test]
fn programs_with_every_value_spilled_match_the_interpreter() {
    if crate::backend::llvm::Toolchain::bundled(Target::WindowsGnu).is_err() && cfg!(windows) {
        eprintln!("skipping: no bundled toolchain");
        return;
    }
    let directory = TempDir::new("native-spilled");
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
                    spill: true,
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

/// Every program the native back end accepts compiles for both calling
/// conventions with its own allocation and with every value spilled, and
/// the allocation checker (on in debug builds) proves each function: the
/// conventions this host cannot run are checked here too.
#[test]
fn every_value_spilled_passes_the_checker_under_both_conventions() {
    let programs = PROGRAMS
        .iter()
        .chain(WIDE_FLOAT_PROGRAMS)
        .chain([&SSE_UNSIGNED_CONVERSIONS])
        .map(|program| (program.name, program.source))
        .chain(
            LOWERING_PROGRAMS
                .iter()
                .chain(BY_VALUE_PROGRAMS)
                .map(|program| (program.name, program.source)),
        )
        .collect::<Vec<_>>();
    let parallelism = thread::available_parallelism()
        .map_or(4, usize::from)
        .min(8);
    let mut failures = Vec::new();
    for chunk in programs.chunks(parallelism.div_ceil(2).max(1)) {
        thread::scope(|scope| {
            let handles: Vec<_> = chunk
                .iter()
                .flat_map(|&(name, source)| {
                    [Target::LinuxGnu, Target::WindowsGnu].map(|target| {
                        scope.spawn(move || {
                            let mut failures = Vec::new();
                            for optimize in [false, true] {
                                with_module(source, target, optimize, |module| {
                                    let mut text = String::new();
                                    if emit_assembly(module, target, &mut text).is_err() {
                                        // Refused before allocation.
                                        return;
                                    }
                                    if let Err(error) = spilled_assembly(module, target) {
                                        failures
                                            .push(format!("{name} {target:?} {optimize}: {error}"));
                                    }
                                });
                            }
                            failures
                        })
                    })
                })
                .collect();
            for handle in handles {
                failures.extend(handle.join().expect("a compilation does not panic"));
            }
        });
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
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
                    spill: false,
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
/// overflow predicates, `i1` arithmetic, signed compares and selects, bit
/// counts of zero, narrow byte swaps, poison and freeze, and switch edges
/// with arguments.
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
    v15 = icmp.i1 slt v2, v3
    v16 = icmp.i1 sge v6, v2
    v17 = zext.i32 v15
    v18 = iconst.i32 16
    v19 = imul.i32 v17, v18
    v20 = zext.i32 v16
    v21 = iconst.i32 32
    v22 = imul.i32 v20, v21
    v23 = iadd.i32 v14, v19
    v24 = iadd.i32 v23, v22
    return v24
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

/// Block labels (GNU extension: GCC "Labels as Values"): `@walk` loops
/// through `indirect_br` edges whose arguments differ per target, swap
/// parameters and pass `i128` pairs and constants, so its labeled blocks
/// get landing blocks; a direct edge still enters a labeled block, and an
/// unreachable labeled block still has an address. `@pick` adds label
/// differences of every width, one with an addend, to a label.
const LABEL_PROGRAM: &str =
    "abi lowered
global @format internal constant size 6, align 1 = bytes \"256c6c640a00\"
global @table internal constant size 24, align 8 = bytes \
     \"000000000000000000000000000000000000000000000000\" relocs [0: blockaddress(@walk, label0), \
     8: blockaddress(@walk, label1), 16: blockaddress(@walk, label2)]
global @offsets internal constant size 16, align 8 = bytes \"00000000000000000000000000000000\" \
     relocs [0: labeldiff.i64(@pick, label1, label0), 8: labeldiff.i32(@pick, label2, label0) + \
     5, 12: labeldiff.i16(@pick, label1, label0), 14: labeldiff.i8(@pick, label0, label0)]
function @printf(ptr, ...) -> i32 external
function @walk(i64) -> i64 internal {
    label0 = block_label block1
    label1 = block_label block2
    label2 = block_label block3
    label3 = block_label block5
block0(v0: i64):
    v1 = global_addr @table
    v2 = load.ptr v1, align 8
    v3 = iconst.i64 0
    v4 = iconst.i128 5
    indirect_br v2, [block1(v0, v3, v4), block2(v3, v0), block3(v0)]
block1(v5: i64, v6: i64, v7: i128):
    v8 = icmp.i64 eq v5, v3
    brif v8, block4(v6, v7), block6
block6:
    v9 = iconst.i64 3
    v10 = imul.i64 v6, v9
    v11 = iadd.i64 v10, v5
    v12 = iconst.i64 1
    v13 = isub.i64 v5, v12
    v14 = sext.i128 v11
    v15 = iadd.i128 v7, v14
    v16 = urem.i64 v13, v9
    v17 = iconst.i64 8
    v18 = imul.i64 v16, v17
    v19 = ptr_add v1, v18
    v20 = load.ptr v19, align 8
    indirect_br v20, [block1(v13, v11, v15), block2(v11, v13), block3(v13)]
block2(v21: i64, v22: i64):
    v23 = iconst.i64 100
    v24 = iadd.i64 v21, v23
    v25 = block_addr label0
    v26 = sext.i128 v24
    v27 = iconst.i64 2
    v28 = urem.i64 v22, v27
    v29 = icmp.i64 eq v28, v3
    v30 = block_addr label2
    v31 = select.ptr v29, v25, v30
    indirect_br v31, [block1(v22, v24, v26), block3(v22)]
block3(v32: i64):
    v33 = iconst.i64 7
    v34 = iadd.i64 v32, v33
    v35 = iconst.i128 -3
    jump block1(v32, v34, v35)
block4(v36: i64, v37: i128):
    v38 = trunc.i64 v37
    v39 = iconst.i128 64
    v40 = lshr.i128 v37, v39
    v41 = trunc.i64 v40
    v42 = iadd.i64 v36, v38
    v43 = iadd.i64 v42, v41
    v44 = block_addr label3
    v45 = block_addr label0
    v46 = icmp.ptr eq v44, v45
    v47 = zext.i64 v46
    v48 = iadd.i64 v43, v47
    return v48
block5:
    unreachable
}

function @pick(i64) -> i64 internal {
    label0 = block_label block1
    label1 = block_label block2
    label2 = block_label block3
block0(v0: i64):
    v1 = block_addr label0
    v2 = ptr_add v1, v0
    indirect_br v2, [block1, block2, block3]
block1:
    v3 = iconst.i64 10
    return v3
block2:
    v4 = iconst.i64 20
    return v4
block3:
    v5 = iconst.i64 30
    return v5
}

function @main() -> i32 external {
block0:
    v0 = global_addr @format
    v1 = iconst.i64 9
    v2 = call @walk(v1)
    v3 = call @printf(v0, v2)
    v4 = iconst.i64 0
    v5 = call @walk(v4)
    v6 = call @printf(v0, v5)
    v7 = global_addr @offsets
    v8 = load.i64 v7, align 8
    v9 = call @pick(v8)
    v10 = call @printf(v0, v9)
    v11 = iconst.i64 8
    v12 = ptr_add v7, v11
    v13 = load.i32 v12, align 4
    v14 = sext.i64 v13
    v15 = iconst.i64 5
    v16 = isub.i64 v14, v15
    v17 = call @pick(v16)
    v18 = call @printf(v0, v17)
    v19 = iconst.i64 12
    v20 = ptr_add v7, v19
    v21 = load.i16 v20, align 2
    v22 = sext.i64 v21
    v23 = call @pick(v22)
    v24 = call @printf(v0, v23)
    v25 = iconst.i64 14
    v26 = ptr_add v7, v25
    v27 = load.i8 v26, align 1
    v28 = sext.i64 v27
    v29 = call @pick(v28)
    v30 = call @printf(v0, v29)
    v31 = iconst.i32 0
    return v31
}
";

#[test]
fn ir_programs_match_the_interpreter() {
    let directory = TempDir::new("native-ir");
    for (target, runner) in targets() {
        for (index, program) in [IR_PROGRAM, LABEL_PROGRAM].into_iter().enumerate() {
            let arena = Bump::new();
            let module = crate::ir::parse_module(&arena, program).unwrap();
            let expected = interpret(&module, target).unwrap();
            let mut text = String::new();
            emit_assembly(&module, target, &mut text).unwrap();
            let name = format!("ir{index}-{target:?}");
            let actual = run_assembly(&text, target, runner, directory.path(), &name).unwrap();
            assert_eq!(
                actual, expected,
                "{target:?}
{text}"
            );
            let text = spilled_assembly(&module, target).unwrap();
            let name = format!("ir{index}-{target:?}-spilled");
            let actual = run_assembly(&text, target, runner, directory.path(), &name).unwrap();
            assert_eq!(
                actual, expected,
                "{target:?}, every value spilled
{text}"
            );
        }
    }
}

/// #69: MinGW code that reads a DLL's data links through LLD's auto-import
/// without a 32-bit pseudo-relocation, which LLD warns "can fail at runtime
/// depending on memory layout": the `.refptr` stub's 64-bit address becomes
/// the import's address-table entry, so the link succeeds even with runtime
/// pseudo-relocations disabled. A direct `sym(%rip)` reference fails that
/// link. Linked freestanding, so it needs only the bundled toolchain.
#[test]
fn mingw_dll_data_links_without_narrow_pseudo_relocations() {
    let Ok(toolchain) = crate::backend::llvm::Toolchain::bundled(Target::WindowsGnu) else {
        eprintln!("skipping: no bundled toolchain");
        return;
    };
    let directory = TempDir::new("native-dll-data");
    let dll = directory.join("data.c");
    std::fs::write(
        &dll,
        "__declspec(dllexport) int shared_value = 42;\n__declspec(dllexport) int \
         shared_list[4];\nint DllMainCRTStartup(void *a, unsigned b, void *c) { return 1; }\n",
    )
    .unwrap();
    let clang = |args: &[&str]| {
        let output = run_native(
            Command::new(toolchain.clang())
                .args([
                    "--target=x86_64-w64-windows-gnu",
                    "-nostdlib",
                    "-fuse-ld=lld",
                ])
                .args(args)
                .current_dir(directory.path()),
        )
        .unwrap();
        (
            output.status.success(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };
    let (built, stderr) = clang(&[
        "-shared",
        "-o",
        "data.dll",
        "data.c",
        "-Wl,--out-implib,libdata.a",
        "-Wl,-e,DllMainCRTStartup",
    ]);
    assert!(built, "{stderr}");
    let source = "extern int shared_value; extern int shared_list[4];
int start(void) { return shared_value + shared_list[3]; }";
    for optimize in [false, true] {
        let text = with_module(source, Target::WindowsGnu, optimize, |module| {
            let mut text = String::new();
            emit_assembly(module, Target::WindowsGnu, &mut text).unwrap();
            text
        });
        std::fs::write(directory.join("start.s"), &text).unwrap();
        let (linked, stderr) = clang(&[
            "-o",
            "start.exe",
            "start.s",
            "-L.",
            "-ldata",
            "-Wl,-e,start",
            "-Wl,--disable-runtime-pseudo-reloc",
        ]);
        assert!(linked && !stderr.contains("too narrow"), "{stderr}\n{text}");
    }
    // The control: the direct reference this replaces does not link so.
    std::fs::write(
        directory.join("direct.s"),
        "\t.text\n\t.globl\tstart\nstart:\n\tmovl\tshared_value(%rip), %eax\n\tretq\n",
    )
    .unwrap();
    let (linked, stderr) = clang(&[
        "-o",
        "direct.exe",
        "direct.s",
        "-L.",
        "-ldata",
        "-Wl,-e,start",
        "-Wl,--disable-runtime-pseudo-reloc",
    ]);
    assert!(
        !linked && stderr.contains("requires pseudo relocations"),
        "{stderr}"
    );
}

/// `f80` and `f128` arithmetic, every comparison, conversions to and from
/// every integer width, `i128` and the other floats, block parameters, and
/// calls with more such arguments than registers. The interpreter does not
/// run `f80` or `f128`, so the module's executable from the LLVM back end
/// is the expected behaviour. The module is written before ABI lowering,
/// since Win64 passes its `i128` arguments by reference.
const WIDE_FLOAT_IR: &str = include_str!("wide_floats.ir");

/// The line of [`WIDE_FLOAT_IR`] that computes an `frem.f128`, and an
/// inline operation that replaces it where nothing links `fmodf128`. The
/// parser wants dense value numbers, so the line is replaced, not removed.
const WIDE_FLOAT_IR_FREM128: (&str, &str) = ("v10 = frem.f128 v0, v1", "v10 = fneg.f128 v1");

/// Whether `target`'s C library defines `fmodf128`, which both back ends
/// call for `frem.f128`, as Clang does for it and for `__builtin_fmodf128`.
/// glibc has it since 2.26. MinGW-w64's runtime has no binary128 functions,
/// and GCC's libquadmath names the routine `fmodq`.
fn has_fmodf128(target: Target) -> bool {
    !matches!(target, Target::WindowsGnu)
}

/// [`WIDE_FLOAT_IR`] lowered for `target`'s ABI and verified.
fn wide_float_ir(arena: &Bump, target: Target) -> Module<'_> {
    lowered_wide_float_ir(arena, target, WIDE_FLOAT_IR)
}

/// [`wide_float_ir`] without its `frem.f128` where nothing links it.
fn linkable_wide_float_ir(arena: &Bump, target: Target) -> Module<'_> {
    if has_fmodf128(target) {
        return wide_float_ir(arena, target);
    }
    let (frem, replacement) = WIDE_FLOAT_IR_FREM128;
    let count = WIDE_FLOAT_IR.matches(frem).count();
    assert_eq!(count, 1, "{target:?}: wide_floats.ir has {count} `{frem}`");
    lowered_wide_float_ir(arena, target, &WIDE_FLOAT_IR.replace(frem, replacement))
}

/// `text`, parsed, lowered for `target`'s ABI and verified.
fn lowered_wide_float_ir<'ir>(arena: &'ir Bump, target: Target, text: &str) -> Module<'ir> {
    let mut module = crate::ir::parse_module(arena, text).unwrap();
    crate::abi::lower_abi(&mut module, target, &Bump::new()).unwrap();
    let scratch = Bump::new();
    let errors = crate::ir::verify_module(&module, Profile::PostAbi, &scratch)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    assert!(errors.is_empty(), "{target:?}: {}", errors.join("\n"));
    module
}

/// Builds `module` with the LLVM back end at `-O0` and runs it.
fn run_llvm(
    module: &Module<'_>,
    target: Target,
    directory: &Path,
    name: &str,
) -> Result<Run, String> {
    let executable = directory.join(format!("{name}-llvm.exe"));
    let options = LlvmOptions {
        opt: None,
        disable_llvm_passes: false,
        target,
        kind: OutputKind::Executable,
        code: crate::target::CodeOptions::default(),
    };
    compile_module(module, &executable, options).map_err(|error| error.to_string())?;
    let output = run_native(&mut Command::new(&executable)).map_err(|error| error.to_string())?;
    Ok(Run {
        status: output.status.code().ok_or("the program was killed")?,
        stdout: String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"),
    })
}

/// Compiles `module` with both back ends, the native one also with every
/// value spilled, and compares the runs.
fn check_against_llvm(
    module: &Module<'_>,
    target: Target,
    directory: &Path,
    name: &str,
) -> Result<(), String> {
    let mut text = String::new();
    emit_assembly(module, target, &mut text).map_err(|error| format!("{name}: {error}"))?;
    let spilled =
        spilled_assembly(module, target).map_err(|error| format!("{name}, spilled: {error}"))?;
    let expected = run_llvm(module, target, directory, name)?;
    if expected.stdout.trim().is_empty() {
        return Err(format!("{name}: the program printed nothing: {expected:?}"));
    }
    for (text, name) in [
        (text, name.to_owned()),
        (spilled, format!("{name}-spilled")),
    ] {
        let actual = run_assembly(&text, target, Runner::Native, directory, &name)?;
        if actual != expected {
            return Err(format!(
                "{name}: the LLVM back end's executable did {expected:?}, the native one \
                 {actual:?}\n{text}"
            ));
        }
    }
    Ok(())
}

/// Every target's native back end compiles the wide-float programs,
/// whichever targets the host runs.
#[test]
fn wide_float_programs_compile_for_every_target() {
    for target in [Target::WindowsGnu, Target::LinuxGnu] {
        for program in WIDE_FLOAT_PROGRAMS {
            for optimize in [false, true] {
                with_module(program.source, target, optimize, |module| {
                    let mut text = String::new();
                    emit_assembly(module, target, &mut text).unwrap_or_else(|error| {
                        panic!("{}-{target:?}-{optimize}: {error}", program.name)
                    });
                });
            }
        }
        let arena = Bump::new();
        let module = wide_float_ir(&arena, target);
        let mut text = String::new();
        emit_assembly(&module, target, &mut text)
            .unwrap_or_else(|error| panic!("wide-ir-{target:?}: {error}"));
        // Clang's spelling, even where the C library lacks it.
        assert!(text.contains("fmodf128"), "wide-ir-{target:?}:\n{text}");
        let module = linkable_wide_float_ir(&arena, target);
        let mut text = String::new();
        emit_assembly(&module, target, &mut text)
            .unwrap_or_else(|error| panic!("linkable wide-ir-{target:?}: {error}"));
        assert_eq!(
            text.contains("fmodf128"),
            has_fmodf128(target),
            "linkable wide-ir-{target:?}:\n{text}"
        );
    }
}

#[test]
fn wide_float_programs_match_the_llvm_back_end() {
    let directory = TempDir::new("native-wide-floats");
    let mut failures = Vec::new();
    for (target, runner) in targets() {
        if runner != Runner::Native {
            eprintln!("skipping {target:?}: the LLVM back end's executables run only natively");
            continue;
        }
        for program in WIDE_FLOAT_PROGRAMS {
            for optimize in [false, true] {
                let name = format!("{}-{target:?}-{optimize}", program.name);
                let result = with_module(program.source, target, optimize, |module| {
                    check_against_llvm(module, target, directory.path(), &name)
                });
                if let Err(failure) = result {
                    failures.push(failure);
                }
            }
        }
        let arena = Bump::new();
        let module = linkable_wide_float_ir(&arena, target);
        let name = format!("wide-ir-{target:?}");
        if let Err(failure) = check_against_llvm(&module, target, directory.path(), &name) {
            failures.push(failure);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// `double` and `float` values at or above 2^63 convert to unsigned
/// without the "invalid" exception that converting them signed would
/// raise, with its own allocation and with every value spilled. The links
/// name `fetestexcept`, which WSL's link has no `-lm` for.
#[test]
fn sse_unsigned_conversions_raise_no_invalid_exception() {
    let directory = TempDir::new("native-unsigned-conversions");
    let program = SSE_UNSIGNED_CONVERSIONS;
    for (target, runner) in targets() {
        if runner != Runner::Native {
            eprintln!("skipping {target:?}: the program links with the C library's fenv");
            continue;
        }
        for (optimize, spill) in [(false, false), (true, false), (false, true), (true, true)] {
            let name = format!(
                "{}-{target:?}-{optimize}{}",
                program.name,
                if spill { "-spilled" } else { "" }
            );
            let run = with_module(program.source, target, optimize, |module| {
                let text = if spill {
                    spilled_assembly(module, target)
                } else {
                    let mut text = String::new();
                    emit_assembly(module, target, &mut text)
                        .map(|()| text)
                        .map_err(|error| error.to_string())
                }
                .unwrap_or_else(|error| panic!("{name}: {error}"));
                run_assembly(&text, target, runner, directory.path(), &name)
            })
            .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(
                run,
                Run {
                    status: 0,
                    stdout: SSE_UNSIGNED_CONVERSIONS_OUTPUT.to_owned(),
                },
                "{name}"
            );
        }
    }
}
