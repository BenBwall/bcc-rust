//! The compiler driver: runs one job per input, in command-line order, up to
//! the stage the options select, as GCC and Clang do. `-E` preprocesses each
//! source, `-S` compiles each to assembly, `-c` to an object, and `-o`
//! without them links every object, archive, library and linker option,
//! with each C source compiled to a temporary object first, into one
//! executable or shared library. Without a stage the source is analyzed and
//! the inspection views or `--emit` and `--interpret` run on it.
//!
//! For `bcc-rust a.c b.o -lm -o app`, `a.c` compiles to an object beside
//! `app`, and the bundled clang links that object, `b.o` and `-lm`, in that
//! order, into `app`.
//!
//! Read [`run`], then [`inputs::ordered_inputs`], [`job::compile`] and
//! [`link::link`].
//!
//! Files by role:
//! - Inputs and output names: `driver/inputs.rs`.
//! - One translation unit's job: `driver/job.rs`.
//! - Dependency output (`-M`, `-MD`): `driver/dependencies.rs`.
//! - Linking and assembling: `driver/link.rs`.
//! - Options: `driver/options.rs`; the driver's own messages:
//!   `driver/messages.rs`.
//!
//! The driver is a host CLI policy, not a C constraint; each job translates
//! one translation unit, C99: §5.1.1.1 paragraph 1, p. 9; PDF p. 21.

// Inputs and jobs
mod inputs;
mod job;

// Outputs
mod dependencies;
mod link;

// Options and messages
mod messages;
mod options;

use clap::ArgMatches;
use dependencies::DependencyNames;
use inputs::{
    Input,
    Source,
    check_output,
    ordered_inputs,
    output_or_stem,
};
pub(super) use inputs::{
    InputKind,
    kind,
    kind_of_language,
};
use job::{
    Job,
    compile,
};
use link::{
    assemble,
    link,
    preprocess_assembly,
};
pub(super) use messages::{
    Severity,
    report,
    report_error,
};
pub(super) use options::DriverOptions;

use super::{
    Cli,
    MainError,
    code_generation::{
        Emit,
        FAILURE_STATUS,
    },
};
use crate::util::bump::Bump;

/// Runs the jobs the arguments ask for and returns the exit status.
pub(super) fn run(args: &Cli, matches: &ArgMatches) -> Result<i32, MainError> {
    // GCC's driver prints the commands it runs bare, Clang's quoted.
    crate::backend::llvm::print_commands(args.driver.verbose.then(|| {
        if args.compiler_identity == super::arguments::Identity::Gcc {
            crate::backend::llvm::CommandStyle::GccVerbose
        } else {
            crate::backend::llvm::CommandStyle::Clang
        }
    }));
    let inputs = ordered_inputs(args, matches)?;
    let generation = &args.code_generation;
    let output = generation.output_path.as_deref();
    if let Some(output) = output {
        check_output(&inputs, output)?;
    }
    let sources = inputs
        .iter()
        .filter(|input| !matches!(input, Input::Link(_)))
        .count();
    let stage = if args.driver.preprocess || args.driver.dependencies.is_some() {
        Stage::Preprocess
    } else if generation.assembly {
        Stage::Assemble
    } else if generation.object {
        Stage::Compile
    } else if generation.output_path.is_some() && generation.emit.is_none() {
        Stage::Link
    } else {
        Stage::Inspect
    };
    if stage == Stage::Compile && args.target.is_wasm() {
        return Err(MainError::Driver(
            "the wasm32 targets write linked modules only: use -o, -S or --emit=wasm instead of -c",
        ));
    }
    if stage != Stage::Link {
        // GCC lets `-M` and `-MM` without `-E`, `-c` or `-S` write each
        // unit's rules to `-o` in turn, every unit replacing the one before.
        let rules_only = args.driver.dependencies.is_some()
            && !args.driver.preprocess
            && !generation.object
            && !generation.assembly;
        if output.is_some() && sources > 1 && !rules_only {
            return Err(MainError::Driver(
                "cannot specify '-o' with '-c', '-S' or '-E' with multiple files",
            ));
        }
    }
    let mut status = 0;
    if stage != Stage::Link {
        for input in &inputs {
            if let Input::Link(name) = input
                && !name.to_string_lossy().starts_with('-')
            {
                report(
                    args,
                    Severity::Warning,
                    format_args!(
                        "{}: linker input file unused because linking not done",
                        name.to_string_lossy()
                    ),
                );
                // GCC also fails when the unused file does not exist.
                if let Err(error) = std::fs::metadata(name) {
                    let arena = Bump::new();
                    report(
                        args,
                        Severity::Error,
                        format_args!(
                            "{}: linker input file not found: {}",
                            name.to_string_lossy(),
                            job::os_reason(&error, &arena)
                        ),
                    );
                    status = combine(status, FAILURE_STATUS);
                }
            }
        }
    }
    if inputs.iter().any(|input| input.c_source().is_some()) {
        link::warn_unused_assembler_options(args);
    }
    match stage {
        | Stage::Inspect => {
            let views = args.output.tokens || generation.requested();
            if views && sources > 1 {
                return Err(MainError::Driver(
                    "--tokens, --emit and --interpret take one input file",
                ));
            }
            for input in &inputs {
                // Linker inputs were reported unused above.
                let Some(source) = input.c_source() else {
                    continue;
                };
                status = combine(status, compile(args, source, Job::Inspect, None)?.status);
            }
        },
        | Stage::Preprocess =>
            for input in &inputs {
                // The bundled clang preprocesses assembly, and writes its
                // rules: alone for a dependency job, otherwise beside the
                // text for `-MD`.
                if let Input::Assembly(assembly) = *input
                    && assembly.preprocessed()
                {
                    let only = args.driver.dependencies.is_some();
                    let names = if only {
                        DependencyNames::default()
                    } else {
                        DependencyNames {
                            file:   output,
                            target: None,
                        }
                    };
                    let rules = dependencies::request_at(args, Some(assembly.path), only, names)?
                        .map(|rules| rules.clang_arguments(args, only))
                        .unwrap_or_default();
                    let text = if only { None } else { output };
                    status = combine(status, preprocess_assembly(args, assembly, text, &rules));
                    continue;
                }
                match input.c_source() {
                    // GCC skips preprocessed input before its preprocessor;
                    // Clang also warns.
                    | Some(Source::Preprocessed(path)) => report(
                        args,
                        Severity::Warning,
                        format_args!(
                            "{}: previously preprocessed input, not preprocessed again",
                            path.display()
                        ),
                    ),
                    | Some(source) if args.driver.dependencies.is_some() => {
                        let dependencies =
                            dependencies::request(args, source, true, DependencyNames::default())?;
                        let scanned = compile(args, source, Job::Scan, dependencies)?;
                        status = combine(status, scanned.status);
                    },
                    | Some(source) => {
                        let names = DependencyNames {
                            file:   output,
                            target: None,
                        };
                        let dependencies = dependencies::request(args, source, false, names)?;
                        let preprocessed =
                            compile(args, source, Job::Preprocess(output), dependencies)?;
                        status = combine(status, preprocessed.status);
                    },
                    | None => {},
                }
            },
        | Stage::Assemble | Stage::Compile =>
            for input in &inputs {
                let extension = if stage == Stage::Assemble { "s" } else { "o" };
                match *input {
                    | Input::Link(_) => {},
                    | Input::Assembly(assembly) =>
                        if stage == Stage::Compile {
                            let object = output_or_stem(output, assembly.path, extension)?;
                            let names = DependencyNames {
                                file:   Some(&object),
                                target: Some(&object),
                            };
                            let rules =
                                dependencies::request_at(args, Some(assembly.path), false, names)?
                                    .map(|rules| rules.clang_arguments(args, false))
                                    .unwrap_or_default();
                            status = combine(status, assemble(args, assembly, &object, &rules));
                        },
                    | Input::C(source) => {
                        let path = output_or_stem(output, source.path(), extension)?;
                        // GCC names an `-S` rule's target after the object
                        // `-c` would write, unless `-o` names the output.
                        let names = DependencyNames {
                            file:   Some(&path),
                            target: if stage == Stage::Compile {
                                Some(&path)
                            } else {
                                output
                            },
                        };
                        let dependencies = dependencies::request(args, source, false, names)?;
                        let mut job = generation.clone();
                        job.output_path = Some(path.clone());
                        if stage == Stage::Assemble {
                            job.emit = Some(Emit::Asm);
                            job.emit_path = Some(path);
                            job.output_path = None;
                        }
                        let generated = compile(args, source, Job::Generate(&job), dependencies)?;
                        status = combine(status, generated.status);
                    },
                }
            },
        | Stage::Link => status = link(args, &inputs)?,
    }
    Ok(status)
}

/// Folds one job's exit status into the run's: the first job that fails
/// decides it, and later jobs still run, as GCC's driver reports a failure
/// whichever input failed. Statuses are not ordered by severity: an
/// interpreted program's is the `int` it passed to `exit`, which may be
/// negative or a multiple of 256, and `main` hands it to the OS unchanged,
/// so a POSIX shell sees its low byte as for a native build.
///
/// C99: §5.1.2.2.3 paragraph 1, p. 13; PDF p. 25.
const fn combine(status: i32, job: i32) -> i32 {
    if status == 0 { job } else { status }
}

/// The last stage a run reaches.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    /// Analysis, the inspection views, `--emit` or `--interpret`.
    Inspect,
    /// `-E`.
    Preprocess,
    /// `-S`.
    Assemble,
    /// `-c`.
    Compile,
    /// `-o` alone.
    Link,
}
