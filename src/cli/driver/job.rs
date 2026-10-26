//! Translates one C source for one job: preprocessing it to text, analyzing
//! it for the inspection views, or generating code for it.

use std::{
    io,
    path::Path,
};

use super::{
    super::{
        Cli,
        MainError,
        code_generation::{
            CodeGeneration,
            FAILURE_STATUS,
            Generated,
            generate_code,
        },
        output::{
            print_parser_output,
            print_preprocessor_output,
        },
        output_file,
        preprocessed::{
            TextOptions,
            print_preprocessed,
        },
    },
    Source,
    dependencies::{
        self,
        DependencyRequest,
    },
};
use crate::{
    headers::HeaderSearch,
    translation_phases::Context,
    util::bump::Bump,
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
    let (input_string, source_filename): (&str, &Path) = match source {
        | Source::Text(text) => (tu.alloc_str(text), Path::new("<input>")),
        | Source::File(path) | Source::Preprocessed(path) if path == Path::new("-") => {
            let text = tu
                .read_from_lossy(io::stdin().lock())
                .map_err(|source| open_error(path, source))?;
            (text, Path::new("<stdin>"))
        },
        | Source::File(path) | Source::Preprocessed(path) => match tu.read_to_str_lossy(path) {
            | Ok(text) => (text, path),
            | Err(source) => return Err(open_error(path, source)),
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
    let mut context = Context::with_configuration(&tu, args.configuration());
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
            if args.output.tokens {
                print_preprocessor_output(context, source_filename, input_string, search);
            } else {
                let errors = print_parser_output(
                    context,
                    source_filename,
                    input_string,
                    search,
                    &args.output.parser,
                );
                // The inspection views exit with zero; a syntax check fails.
                if args.driver.syntax_only && errors != 0 {
                    return Ok(Generated {
                        status:         FAILURE_STATUS,
                        atomic_runtime: false,
                    });
                }
            }
            return Ok(Generated {
                status:         0,
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

fn open_error(path: &Path, source: io::Error) -> MainError {
    MainError::OpenInputFileError {
        path: path.into(),
        source,
    }
}
