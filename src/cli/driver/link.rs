//! Links the inputs of a multi-file build with the bundled clang, after
//! compiling each source and assembling each assembly file into a
//! temporary object.

#[expect(
    clippy::disallowed_types,
    reason = "Startup builds the clang command line beside clap's arguments."
)]
use std::ffi::OsString;
use std::path::Path;
#[expect(
    clippy::disallowed_types,
    reason = "The driver names one dependency file per input as an owned path."
)]
use std::path::PathBuf;

use super::{
    super::{
        Cli,
        MainError,
        code_generation::FAILURE_STATUS,
    },
    Input,
    Job,
    Severity,
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
    configuration::CompilerIdentity,
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
        let base = default_output_base(args, source.path());
        let names = rule_names(args, output, base.as_deref());
        let dependencies = dependencies::request(args, source, false, names)?;
        return compile(args, source, Job::Generate(generation), dependencies)
            .map(|generated| generated.status);
    }
    let target = args.target;
    if target.is_wasm() {
        return Err(MainError::Driver(
            "the wasm32 targets link one C source into a module; pass a single source with -o",
        ));
    }
    let arena = Bump::new();
    let mut temporaries = Vec::new();
    let mut status = 0;
    let mut atomic_runtime = false;
    let mut executable_stack = false;
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
            | Input::Assembly(assembly) => {
                // A lone assembly input's rule names the output, as a lone
                // C source's does and as GCC and Clang write it.
                let base = default_output_base(args, assembly.path);
                let names = rule_names(args, output, base.as_deref());
                let rules = dependencies::request_at(args, Some(assembly.path), false, names)?
                    .map(|rules| rules.clang_arguments(args, false))
                    .unwrap_or_default();
                status = combine(status, assemble(args, assembly, object.path(), &rules));
            },
            | _ => {
                let source = input.c_source().expect("a C input");
                let mut job = generation.clone();
                job.object = true;
                job.shared = false;
                job.pie = false;
                job.no_pie = false;
                job.output_path = Some(object.path().into());
                let base = default_output_base(args, source.path());
                let names = rule_names(args, output, base.as_deref());
                let dependencies = dependencies::request(args, source, false, names)?;
                let generated = compile(args, source, Job::Generate(&job), dependencies)?;
                status = combine(status, generated.status);
                atomic_runtime |= generated.atomic_runtime;
                executable_stack |= generated.executable_stack;
            },
        }
        temporaries.push(object);
    }
    if status != 0 {
        return Ok(status);
    }
    // As GCC's driver leaves it to `ld.bfd`, which gives the executable an
    // executable stack when an input object's note asks for one, though
    // LLD refuses such an object unless told: trampolines of GNU nested
    // functions run on the stack.
    executable_stack |= inputs.iter().any(|input| match input {
        | Input::Link(option) => needs_executable_stack(Path::new(option)),
        | _ => false,
    }) || temporaries
        .iter()
        .any(|object| needs_executable_stack(object.path()));
    let mut temporaries = temporaries.iter();
    let ordered = inputs.iter().map(|input| match input {
        | Input::Link(option) => *option,
        | _ => temporaries
            .next()
            .expect("each compiled input has an object")
            .path()
            .as_os_str(),
    });
    let configuration = args.configuration();
    let options = generation.llvm_options(
        target,
        configuration.code_options(),
        configuration.links_fast_math_runtime(),
    );
    let linked = Toolchain::bundled(target).and_then(|toolchain| {
        toolchain.with_sysroot(sysroot).link_inputs(
            ordered,
            output,
            options,
            (atomic_runtime, executable_stack),
        )
    });
    match linked {
        | Ok(()) => Ok(0),
        | Err(error) => {
            super::report(
                args,
                Severity::Error,
                format_args!("cannot build `{}`: {error}", output.display()),
            );
            Ok(FAILURE_STATUS)
        },
    }
}

/// The names of a compiled input's dependency rules in a link: each
/// input's file and target are the `-o` output's (`prog.d`, `prog:`), so
/// with several the last one's rule stands, as GCC and Clang write them.
/// Without `-o`, the sources name them (`f.o:`), in GCC's `.d` file after
/// its default output's dump base (`base`, as `a-f.d`), in Clang's after
/// the source (`f.d`).
fn rule_names<'a>(args: &Cli, output: &'a Path, base: Option<&'a Path>) -> DependencyNames<'a> {
    if args.driver.default_output {
        DependencyNames {
            file:   base,
            target: None,
        }
    } else {
        DependencyNames {
            file:   Some(output),
            target: Some(output),
        }
    }
}

/// GCC's dump base for `source` when it links to its default output:
/// `a-STEM`, whose `.d` file `-MD` writes, as GCC 11 and later name it.
/// `None` under the other identities, with `-o`, or for stdin.
/// GCC manual, "Overall Options", `-dumpbase`.
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    reason = "The driver names one file per input as an owned path."
)]
fn default_output_base(args: &Cli, source: &Path) -> Option<PathBuf> {
    let gcc = CompilerIdentity::from(args.compiler_identity) == CompilerIdentity::Gcc;
    let name = source.file_name().filter(|_| source != Path::new("-"))?;
    (gcc && args.driver.default_output)
        .then(|| PathBuf::from(format!("a-{}", name.to_string_lossy())))
}

/// Assembles `assembly` into the object `object` with the bundled clang
/// and returns the status. Clang reads the `-x` language the input was
/// given under, the assembler options, and, when the C preprocessor runs
/// first, the `-D`, `-U` and `-include` options and the header search.
#[expect(
    clippy::disallowed_types,
    reason = "The clang options are startup strings beside clap's arguments."
)]
pub(super) fn assemble(
    args: &Cli,
    assembly: Assembly<'_>,
    object: &Path,
    rules: &[OsString],
) -> i32 {
    let mut options = assembler_arguments(args, assembly);
    options.extend_from_slice(rules);
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
            super::report(
                args,
                Severity::Error,
                format_args!("cannot assemble `{}`: {error}", assembly.path.display()),
            );
            FAILURE_STATUS
        },
    }
}

/// Preprocesses an assembly input that the C preprocessor reads first
/// (`.S`, `.sx`, `-x assembler-with-cpp`) under `-E`, with the options its
/// assembly would use, into `output` or stdout. bcc assembles such an input
/// with the bundled clang, so the text is what that assembly reads, as
/// GCC's and Clang's `-E` print it; `-P` and `-dM` apply as for C.
#[expect(
    clippy::disallowed_types,
    reason = "Startup builds the clang command line beside clap's arguments."
)]
pub(super) fn preprocess_assembly(
    args: &Cli,
    assembly: Assembly<'_>,
    output: Option<&Path>,
    rules: &[OsString],
) -> i32 {
    let mut options = assembler_arguments(args, assembly);
    options.extend_from_slice(rules);
    if args.driver.no_line_markers {
        options.push("-P".into());
    }
    if args.driver.dump_macros {
        options.push("-dM".into());
    }
    let preprocessed = Toolchain::bundled(args.target).and_then(|toolchain| {
        let toolchain = toolchain.with_sysroot(args.search.sysroot());
        let options = options.iter().map(OsString::as_os_str);
        match output.filter(|path| *path != Path::new("-")) {
            | Some(path) => std::fs::File::create(path).and_then(|mut file| {
                toolchain.preprocess_foreign(assembly.path, args.target, options, &mut file)
            }),
            | None => toolchain.preprocess_foreign(
                assembly.path,
                args.target,
                options,
                &mut std::io::stdout().lock(),
            ),
        }
    });
    match preprocessed {
        | Ok(()) => 0,
        | Err(error) => {
            super::report(
                args,
                Severity::Error,
                format_args!("cannot preprocess `{}`: {error}", assembly.path.display()),
            );
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
    // Clang knows `.S` but not `.sx`, which it would take for a linker
    // input, so a preprocessed input always names its language.
    if let Some(preprocessed) = assembly
        .language
        .or(assembly.preprocessed().then_some(true))
    {
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
    // Clang's warnings reach the user, so `-w` and `-Werror` apply to them.
    let configuration = args.configuration();
    if configuration.suppress_warnings() {
        options.push("-w".into());
    } else if configuration.warnings_are_errors() {
        options.push("-Werror".into());
    }
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
            super::report(
                args,
                Severity::Warning,
                format_args!(
                    "assembler option '{piece}' is not applied to C sources, which bcc assembles \
                     itself"
                ),
            );
        }
    }
}

/// Whether `path` is an ELF64 little-endian object whose
/// `.note.GNU-stack` section is executable (`SHF_EXECINSTR`), as GCC marks
/// an object whose trampolines run on the stack. Anything unreadable or
/// otherwise, such as an archive or a linker option, is not.
/// GNU extension: GCC manual, "Nested Functions".
fn needs_executable_stack(path: &Path) -> bool {
    use std::io::{
        Read,
        Seek,
        SeekFrom,
    };
    const NAME: &[u8; 16] = b".note.GNU-stack\0";
    const SHF_EXECINSTR: u64 = 4;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut read_at = |offset: u64, buffer: &mut [u8]| {
        file.seek(SeekFrom::Start(offset))
            .and_then(|_| file.read_exact(buffer))
            .is_ok()
    };
    let mut header = [0_u8; 64];
    if !read_at(0, &mut header) || header[..6] != *b"\x7fELF\x02\x01" {
        return false;
    }
    let word = |bytes: &[u8], at: usize| {
        u64::from_le_bytes(bytes[at..at + 8].try_into().expect("eight bytes"))
    };
    let half = |bytes: &[u8], at: usize| u64::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]));
    let (sections, count, names) = (
        word(&header, 0x28),
        half(&header, 0x3C),
        half(&header, 0x3E),
    );
    if half(&header, 0x3A) != 64 || names >= count {
        return false;
    }
    let mut section = [0_u8; 64];
    if !read_at(sections + 64 * names, &mut section) {
        return false;
    }
    let strings = word(&section, 0x18);
    (0..count).any(|index| {
        let mut name = [0_u8; 16];
        read_at(sections + 64 * index, &mut section)
            && word(&section, 8) & SHF_EXECINSTR != 0
            && read_at(
                strings
                    + u64::from(u32::from_le_bytes(
                        section[..4].try_into().expect("four bytes"),
                    )),
                &mut name,
            )
            && name == *NAME
    })
}
