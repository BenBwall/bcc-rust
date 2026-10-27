//! Links the inputs of a multi-file build with the bundled clang, after
//! compiling each source and assembling each assembly file into a
//! temporary object.

#[expect(
    clippy::disallowed_types,
    reason = "Startup builds the clang command line beside clap's arguments."
)]
use std::ffi::OsString;
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
    combine,
    compile,
    dependencies::{
        self,
        DependencyNames,
    },
    inputs::Assembly,
};
use crate::{
    backend::llvm::{
        TemporaryObject,
        Toolchain,
    },
    util::bump::Bump,
};

/// Builds the `-o` output from every input: one C source alone links in
/// one step, as before the driver had several inputs; otherwise, or when
/// `--sysroot` selects the libraries, each C source and assembly file
/// becomes a temporary object beside the output, and the bundled clang
/// links them with the other inputs in order. No link runs after a job
/// failed.
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
    let sysroot = args.search.sysroot();
    let single = matches!(inputs, [Input::C(_)]);
    if single
        && sysroot.is_none()
        && let Some(source) = inputs[0].c_source()
    {
        let names = DependencyNames {
            file:   Some(output),
            target: Some(output),
        };
        let dependencies = dependencies::request(args, source, false, names)?;
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
        match *input {
            | Input::Assembly(assembly) =>
                status = combine(status, assemble(args, assembly, object.path())),
            | _ => {
                let source = input.c_source().expect("a C input");
                let mut job = generation.clone();
                job.object = true;
                job.shared = false;
                job.pie = false;
                job.no_pie = false;
                job.output_path = Some(object.path().into());
                // One source's rule names the output, as GCC writes it.
                // With several, each names its own object and `.d` file, as
                // Clang names them without `-o`; GCC would overwrite one
                // `.d` file per source.
                let names = if single {
                    DependencyNames {
                        file:   Some(output),
                        target: Some(output),
                    }
                } else {
                    DependencyNames::default()
                };
                let dependencies = dependencies::request(args, source, false, names)?;
                let generated = compile(args, source, Job::Generate(&job), dependencies)?;
                status = combine(status, generated.status);
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
    let linked = Toolchain::bundled(target).and_then(|toolchain| {
        toolchain
            .with_sysroot(sysroot)
            .link_inputs(ordered, output, options, atomic_runtime)
    });
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

/// Assembles `assembly` into the object `object` with the bundled clang
/// and returns the status. Clang reads the `-x` language the input was
/// given under, the assembler options, and, when the C preprocessor runs
/// first, the `-D`, `-U` and `-include` options and the header search.
#[expect(
    clippy::disallowed_types,
    reason = "The clang options are startup strings beside clap's arguments."
)]
pub(super) fn assemble(args: &Cli, assembly: Assembly<'_>, object: &Path) -> i32 {
    let options = assembler_arguments(args, assembly);
    let assembled = Toolchain::bundled(args.target).and_then(|toolchain| {
        toolchain
            .with_sysroot(args.search.sysroot())
            .compile_foreign(
                assembly.path,
                object,
                args.target,
                options.iter().map(OsString::as_os_str),
            )
    });
    match assembled {
        | Ok(()) => 0,
        | Err(error) => {
            report_stderr(format_args!(
                "error: cannot assemble `{}`: {error}",
                assembly.path.display()
            ));
            FAILURE_STATUS
        },
    }
}

/// The clang options before an assembly input.
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    reason = "Startup builds the clang command line beside clap's arguments."
)]
fn assembler_arguments(args: &Cli, assembly: Assembly<'_>) -> Vec<OsString> {
    let mut options: Vec<OsString> = Vec::new();
    if let Some(preprocessed) = assembly.language {
        options.push("-x".into());
        options.push(
            if preprocessed {
                "assembler-with-cpp"
            } else {
                "assembler"
            }
            .into(),
        );
    }
    options.extend(args.driver.assembler_option.iter().map(OsString::from));
    if assembly.preprocessed() {
        // The startup preprocessing operations, as
        // `Cli::configure_preprocessing` reads them.
        for option in &args.preprocessing_option {
            let value = &option[1..];
            match option.as_bytes()[0] {
                | b'D' => options.push(format!("-D{value}").into()),
                | b'U' => options.push(format!("-U{value}").into()),
                | _ => {
                    options.push("-include".into());
                    options.push(value.into());
                },
            }
        }
        options.extend(args.search.clang_arguments());
    }
    options
}

/// Warns that the assembler options do not reach C sources, which bcc
/// assembles itself, except `--noexecstack`: bcc's objects already mark
/// their stack non-executable.
pub(super) fn warn_unused_assembler_options(args: &Cli) {
    let mut operand = false;
    for option in &args.driver.assembler_option {
        let pieces = if std::mem::take(&mut operand) {
            // The operand of `-Xassembler` is one option.
            option.split('\0')
        } else if option == "-Xassembler" {
            operand = true;
            continue;
        } else {
            option.strip_prefix("-Wa,").unwrap_or(option).split(',')
        };
        for piece in pieces.filter(|piece| *piece != "--noexecstack") {
            report_stderr(format_args!(
                "warning: assembler option '{piece}' is not applied to C sources, which bcc \
                 assembles itself"
            ));
        }
    }
}
