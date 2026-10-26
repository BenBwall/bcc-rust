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
//! - Options: `driver/options.rs`.
//!
//! The driver is a host CLI policy, not a C constraint; each job translates
//! one translation unit, C99: §5.1.1.1 paragraph 1, p. 9; PDF p. 21.

// Inputs and jobs
mod inputs;
mod job;

// Outputs
mod dependencies;
mod link;

// Options
mod options;

use clap::ArgMatches;
use inputs::{
    Input,
    Source,
    ordered_inputs,
    output_or_stem,
};
use job::{
    Job,
    compile,
};
use link::{
    assemble,
    link,
};
pub(super) use options::DriverOptions;

use super::{
    Cli,
    MainError,
    code_generation::{
        Emit,
        report_stderr,
    },
};

/// Runs the jobs the arguments ask for and returns the exit status.
pub(super) fn run(args: &Cli, matches: &ArgMatches) -> Result<i32, MainError> {
    let inputs = ordered_inputs(args, matches)?;
    let generation = &args.code_generation;
    let output = generation.output_path.as_deref();
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
    if stage != Stage::Link {
        if output.is_some() && sources > 1 {
            return Err(MainError::Driver(
                "cannot specify '-o' with '-c', '-S' or '-E' with multiple files",
            ));
        }
        for input in &inputs {
            if let Input::Link(name) = input
                && !name.to_string_lossy().starts_with('-')
            {
                report_stderr(format_args!(
                    "warning: {}: linker input file unused because linking not done",
                    name.to_string_lossy()
                ));
            }
        }
    }
    let mut status = 0;
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
                status = status.max(compile(args, source, Job::Inspect, None)?.status);
            }
        },
        | Stage::Preprocess =>
            for input in &inputs {
                if let Some(source) = input.c_source() {
                    let (job, dependencies) = if args.driver.dependencies.is_some() {
                        (Job::Scan, dependencies::request(args, source, true, None)?)
                    } else {
                        (
                            Job::Preprocess(output),
                            dependencies::request(args, source, false, None)?,
                        )
                    };
                    status = status.max(compile(args, source, job, dependencies)?.status);
                }
            },
        | Stage::Assemble | Stage::Compile =>
            for input in &inputs {
                let extension = if stage == Stage::Assemble { "s" } else { "o" };
                match input {
                    | Input::Link(_) => {},
                    | Input::Assembly(path) =>
                        if stage == Stage::Compile {
                            let object = output_or_stem(output, path, extension)?;
                            status = status.max(assemble(args, path, &object));
                        },
                    | _ => {
                        let source = input.c_source().expect("a C input");
                        let path = output_or_stem(output, source.path(), extension)?;
                        let object = (stage == Stage::Compile).then_some(path.as_path());
                        let dependencies = dependencies::request(args, source, false, object)?;
                        let mut job = generation.clone();
                        job.output_path = Some(path.clone());
                        if stage == Stage::Assemble {
                            job.emit = Some(Emit::Asm);
                            job.emit_path = Some(path);
                            job.output_path = None;
                        }
                        status = status
                            .max(compile(args, source, Job::Generate(&job), dependencies)?.status);
                    },
                }
            },
        | Stage::Link => status = link(args, &inputs)?,
    }
    Ok(status)
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
