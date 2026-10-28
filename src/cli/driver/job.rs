//! Translates one C source for one job: preprocessing it to text, analyzing
//! it for the inspection views, or generating code for it.

use std::{
    fmt::Write as _,
    io,
    path::Path,
};

use super::{
    super::{
        Cli,
        MainError,
        arguments::{
            Format,
            Identity,
        },
        code_generation::{
            CodeGeneration,
            FAILURE_STATUS,
            Generated,
            generate_code,
            report_stderr,
        },
        output::{
            print_parser_output,
            print_preprocessor_output,
        },
        output_file,
        preprocessed::print_preprocessed,
    },
    Source,
    dependencies::{
        self,
        DependencyRequest,
    },
};
use crate::{
    headers::HeaderSearch,
    pipeline::TextOptions,
    translation_phases::Context,
    util::bump::{
        ArenaString,
        ArenaVec,
        Bump,
    },
};

/// What one C job does with its translation unit.
#[derive(Clone, Copy)]
pub(super) enum Job<'a> {
    /// Analyze it and print what the inspection options ask for, or run
    /// the code generation they ask for.
    Inspect,
    /// Write its preprocessed text to the path, or stdout.
    Preprocess(Option<&'a Path>),
    /// Preprocess it for its dependencies alone (`-M`, `-MM`).
    Scan,
    /// Generate code with these options, which name the output.
    Generate(&'a CodeGeneration),
}

/// Translates `source` for `job` and returns its outcome, then writes the
/// make rules `dependencies` asks for.
pub(super) fn compile(
    args: &Cli,
    source: Source<'_>,
    job: Job<'_>,
    dependencies: Option<DependencyRequest>,
) -> Result<Generated, MainError> {
    let tu = Bump::new();
    let read: Result<(&str, &Path), _> = match source {
        | Source::Text(text) => Ok((tu.alloc_str(text), Path::new("<input>"))),
        | Source::File(path) | Source::Preprocessed(path) if path == Path::new("-") => tu
            .read_from_lossy(io::stdin().lock())
            .map(|text| (text, Path::new("<stdin>")))
            .map_err(|source| (path, source)),
        | Source::File(path) | Source::Preprocessed(path) => tu
            .read_to_str_lossy(path)
            .map(|text| (text, path))
            .map_err(|source| (path, source)),
    };
    let (input_string, source_filename) = match read {
        | Ok(read) => read,
        // An unreadable input fails its own job alone: GCC reports it and
        // still runs the others, then skips the link.
        | Err((path, source)) => {
            report_unreadable(args, path, &source, &tu);
            return Ok(Generated {
                status:         FAILURE_STATUS,
                atomic_runtime: false,
            });
        },
    };
    let output = match &job {
        | Job::Inspect => args.code_generation.output_path.as_deref(),
        | Job::Preprocess(output) => *output,
        | Job::Scan => None,
        | Job::Generate(options) => options
            .output_path
            .as_deref()
            .or(options.emit_path.as_deref()),
    };
    if let (Source::File(input) | Source::Preprocessed(input), Some(output)) = (source, output)
        && input != Path::new("-")
    {
        let same_file = output_file::is_input(input, output).map_err(|source| {
            MainError::InspectOutputFileError {
                path: output.into(),
                source,
            }
        })?;
        if same_file {
            return Err(MainError::OutputIsInput {
                input:  input.into(),
                output: output.into(),
            });
        }
    }
    let directories = args.search.directories(args.target);
    let search = HeaderSearch {
        quote:    tu.alloc_slice_fill_iter(directories.quote.iter().map(AsRef::as_ref)),
        angled:   tu.alloc_slice_fill_iter(directories.angled.iter().map(AsRef::as_ref)),
        system:   tu.alloc_slice_fill_iter(directories.system.iter().map(AsRef::as_ref)),
        resource: directories.resource,
        after:    tu.alloc_slice_fill_iter(directories.after.iter().map(AsRef::as_ref)),
    };
    if args.driver.verbose {
        dependencies::print_search(&search);
    }
    let mut context = Context::with_configuration(&tu, args.configuration());
    context.library_builtins = args.library_builtins(&tu);
    context.code_assumptions = args.code_assumptions();
    if args.driver.header_tree || args.driver.show_includes {
        context.include_entries = Some(ArenaVec::new_in(&tu));
    }
    context.missing_headers_generated =
        args.driver.generated_dependencies && matches!(job, Job::Scan);
    // GCC passes no preprocessing options to `-fpreprocessed` input.
    if matches!(source, Source::Preprocessed(_)) {
        context.preprocessed_input = true;
    } else {
        args.configure_preprocessing(&mut context);
    }
    context.configuration = context
        .configuration
        .with_repeated_specifier_warnings(!args.no_repeated_specifier_warnings);
    let generated = run_job(
        &mut context,
        args,
        (source_filename, input_string),
        search,
        job,
    )?;
    if let Some(dependencies) = dependencies {
        dependencies::write(&context, args, &dependencies)?;
    }
    if args.driver.header_tree {
        dependencies::header_tree(&context);
    }
    if args.driver.show_includes {
        dependencies::show_includes(&context);
    }
    Ok(generated)
}

/// Runs `job` on the translation unit read from `source_filename`.
fn run_job<'tu>(
    context: &mut Context<'tu>,
    args: &Cli,
    (source_filename, input_string): (&Path, &'tu str),
    search: HeaderSearch<'_>,
    job: Job<'_>,
) -> Result<Generated, MainError> {
    let options = match job {
        | Job::Scan => {
            let failed = print_preprocessed(
                context,
                source_filename,
                input_string,
                search,
                TextOptions::default(),
                &mut io::sink(),
            )
            .unwrap_or(true);
            return Ok(Generated {
                status:         if failed { FAILURE_STATUS } else { 0 },
                atomic_runtime: false,
            });
        },
        | Job::Preprocess(output) => {
            let text = TextOptions {
                no_line_markers: args.driver.no_line_markers,
                macros:          args.driver.dump_macros,
                as_written:      false,
            };
            let failed = match output.filter(|path| *path != Path::new("-")) {
                | Some(path) => std::fs::File::create(path)
                    .and_then(|mut file| {
                        print_preprocessed(
                            context,
                            source_filename,
                            input_string,
                            search,
                            text,
                            &mut file,
                        )
                    })
                    .map_err(|source| MainError::WriteOutputFileError {
                        path: path.into(),
                        source,
                    })?,
                | None => print_preprocessed(
                    context,
                    source_filename,
                    input_string,
                    search,
                    text,
                    &mut io::stdout().lock(),
                )
                .unwrap_or_else(|error| error.kind() != io::ErrorKind::BrokenPipe),
            };
            return Ok(Generated {
                status:         if failed { FAILURE_STATUS } else { 0 },
                atomic_runtime: false,
            });
        },
        | Job::Inspect if !args.code_generation.requested() => {
            let errors = if args.output.tokens {
                print_preprocessor_output(context, source_filename, input_string, search)
            } else {
                print_parser_output(
                    context,
                    source_filename,
                    input_string,
                    search,
                    &args.output.parser,
                )
            };
            // An error fails the command, as in any compiler, whichever
            // view printed it.
            return Ok(Generated {
                status:         if errors == 0 { 0 } else { FAILURE_STATUS },
                atomic_runtime: false,
            });
        },
        | Job::Inspect => &args.code_generation,
        | Job::Generate(options) => options,
    };
    let generated = generate_code(context, source_filename, input_string, search, options);
    // The failed compile job owns its requested object, even if it never
    // opened it. A front-end failure in link mode owns only a temporary
    // object; preserve the final executable as Clang does.
    if generated.status != 0
        && options.object
        && let Some(output) = &options.output_path
    {
        output_file::remove_failed_object(output, context.tu_arena());
    }
    Ok(generated)
}

/// Reports an input that cannot be read in the words of the compiler whose
/// diagnostic format is selected: GCC's `cc1`, Clang's front end (also for
/// clang-cl), `cl`'s `c1` with Microsoft's error C1083, or TCC; bcc's own
/// format adds the OS's error code. Clang's and clang-cl's drivers report a
/// missing file before any job runs, and so do their personalities, so this
/// reaches them for a file that exists but cannot be read.
fn report_unreadable(args: &Cli, path: &Path, error: &io::Error, arena: &Bump) {
    let reason = os_reason(error, arena);
    let path = path.display();
    match args.diagnostic_format {
        | Format::Bcc => report_stderr(format_args!("error: cannot read `{path}`: {error}")),
        // GCC's preprocessor reports a directory as a missing file.
        | Format::Gcc => report_stderr(format_args!(
            "cc1: fatal error: {path}: {}\ncompilation terminated.",
            if error.kind() == io::ErrorKind::IsADirectory {
                "No such file or directory"
            } else {
                reason
            }
        )),
        | Format::Msvc | Format::MsvcClassic | Format::MsvcCaret
            if args.compiler_identity != Identity::ClangCl =>
            report_stderr(format_args!(
                "c1: fatal error C1083: Cannot open source file: '{path}': {reason}"
            )),
        | Format::Clang | Format::Msvc | Format::MsvcClassic | Format::MsvcCaret => report_stderr(
            format_args!("error: error reading '{path}': {reason}\n1 error generated."),
        ),
        | Format::Tcc => report_stderr(format_args!(
            "{}: error: file '{path}' not found",
            args.driver.driver_name.as_deref().unwrap_or("tcc")
        )),
    }
}

/// The C library's `strerror` text for `error`, which the compilers print,
/// without the ` (os error N)` Rust appends to it.
pub(super) fn os_reason<'a>(error: &io::Error, arena: &'a Bump) -> &'a str {
    let mut text = ArenaString::new_in(arena);
    _ = write!(text, "{error}");
    let text = text.into_str();
    match error.raw_os_error() {
        | Some(_) => text
            .rsplit_once(" (os error ")
            .map_or(text, |(reason, _)| reason),
        | None => text,
    }
}
