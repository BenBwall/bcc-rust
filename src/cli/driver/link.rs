//! Links the inputs of a multi-file build with the bundled clang, after
//! compiling each source and assembling each assembly file into a
//! temporary object.

use std::path::Path;

use super::{
    super::{
        Cli,
        MainError,
        code_generation::{
            FAILURE_STATUS,
            report_stderr,
        },
    },
    Input,
    Job,
    compile,
    dependencies,
};
use crate::{
    backend::llvm::{
        TemporaryObject,
        Toolchain,
    },
    util::bump::Bump,
};

/// Builds the `-o` output from every input: one C source alone links in
/// one step, as before the driver had several inputs; otherwise each C
/// source and assembly file becomes a temporary object beside the output,
/// and the bundled clang links them with the other inputs in order. No link
/// runs after a job failed.
#[expect(
    clippy::disallowed_types,
    reason = "The temporary objects own their paths and delete their files when dropped."
)]
pub(super) fn link(args: &Cli, inputs: &[Input<'_>]) -> Result<i32, MainError> {
    let generation = &args.code_generation;
    let output = generation
        .output_path
        .as_deref()
        .expect("the link stage has an output");
    if let [input] = inputs
        && let Some(source) = input.c_source()
    {
        let dependencies = dependencies::request(args, source, false, None)?;
        return compile(args, source, Job::Generate(generation), dependencies)
            .map(|generated| generated.status);
    }
    let arena = Bump::new();
    let mut temporaries = Vec::new();
    let mut status = 0;
    let mut atomic_runtime = false;
    let target = args.target;
    for input in inputs {
        let object = match input {
            | Input::Link(_) => continue,
            | _ => TemporaryObject::create(output, "o", &arena)
                .map(|(object, file)| {
                    drop(file);
                    object
                })
                .map_err(|source| MainError::WriteOutputFileError {
                    path: output.into(),
                    source,
                })?,
        };
        match input {
            | Input::Assembly(path) => status = status.max(assemble(args, path, object.path())),
            | _ => {
                let source = input.c_source().expect("a C input");
                let mut job = generation.clone();
                job.object = true;
                job.shared = false;
                job.pie = false;
                job.no_pie = false;
                job.output_path = Some(object.path().into());
                let dependencies = dependencies::request(args, source, false, None)?;
                let generated = compile(args, source, Job::Generate(&job), dependencies)?;
                status = status.max(generated.status);
                atomic_runtime |= generated.atomic_runtime;
            },
        }
        temporaries.push(object);
    }
    if status != 0 {
        return Ok(status);
    }
    let mut temporaries = temporaries.iter();
    let ordered = inputs.iter().map(|input| match input {
        | Input::Link(option) => *option,
        | _ => temporaries
            .next()
            .expect("each compiled input has an object")
            .path()
            .as_os_str(),
    });
    let options = generation.llvm_options(target, args.configuration().code_options());
    let linked = Toolchain::bundled(target)
        .and_then(|toolchain| toolchain.link_inputs(ordered, output, options, atomic_runtime));
    match linked {
        | Ok(()) => Ok(0),
        | Err(error) => {
            report_stderr(format_args!(
                "error: cannot build `{}`: {error}",
                output.display()
            ));
            Ok(FAILURE_STATUS)
        },
    }
}

/// Assembles `path` into the object `object` with the bundled clang and
/// returns the status.
pub(super) fn assemble(args: &Cli, path: &Path, object: &Path) -> i32 {
    match Toolchain::bundled(args.target)
        .and_then(|toolchain| toolchain.compile_foreign(path, object, args.target))
    {
        | Ok(()) => 0,
        | Err(error) => {
            report_stderr(format_args!(
                "error: cannot assemble `{}`: {error}",
                path.display()
            ));
            FAILURE_STATUS
        },
    }
}
