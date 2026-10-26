//! The command-line interface parses options, loads the input, and builds a
//! translation context with the selected language mode and target. [`run`]
//! chooses token output, parsing and semantic analysis, or code generation,
//! then prints the requested view and diagnostics and returns the exit
//! status.
//!
//! For `bcc-rust --input "int x;" --syntax-tree`, arguments select an inline
//! source and syntax output. The driver preprocesses and parses that source,
//! reports diagnostics, and prints the declaration tree.
//!
//! Read [`run`], then [`Cli`], [`Cli::configuration`], and
//! [`output::print_parser_output`]. Token output starts at
//! [`output::print_preprocessor_output`], and code generation (`--emit`,
//! `--interpret`, `-o`) at [`code_generation::generate_code`].
//!
//! Files by role:
//! - Invocation: `arguments.rs`, `search.rs`, `errors.rs`.
//! - Output: `output.rs`, `output_file.rs`; the token view and the diagnostic
//!   reporter it shares with the explorer live in `pipeline/views.rs` and
//!   `diagnostics/reporter.rs`.
//! - Code generation: `code_generation.rs`.
//! - Measurement entry points: `measured.rs`.
//! - CLI fixtures: `tests.rs`.
//!
//! C99: translation phases 1-7, §5.1.1.2 paragraph 1, pp. 9-10;
//! PDF pp. 21-22. Required diagnostics: §5.1.1.3 paragraph 1, p. 11;
//! PDF p. 23.

// Arguments and input
mod arguments;
mod errors;
mod search;

// Output and diagnostic reporting
mod code_generation;
mod output;
mod output_file;

// Measurements
mod measured;

use std::path::Path;

use arguments::{
    CliInput,
    SourceDateEpoch,
    SourceDateEpochParser,
    StandardParser,
    TargetParser,
    TlsModelParser,
    normalize_language_arguments,
    preprocessing_option,
};
use clap::{
    ColorChoice,
    Parser,
};
use code_generation::{
    CodeGeneration,
    generate_code,
    print_default_passes,
};
pub use errors::MainError;
pub use measured::{
    CompileStep,
    compile_file_measured,
    compile_file_with_arguments_measured,
};
use output::{
    CliOutput,
    print_parser_output,
    print_preprocessor_output,
};
use search::CliHeaderSearch;

use crate::{
    configuration::LanguageMode,
    headers::HeaderSearch,
    translation_phases::Context,
    util::bump::Bump,
};

/// Runs the compiler on the command line's arguments and returns the
/// process's exit status. The inspection views exit with zero after
/// printing their diagnostics; code generation exits nonzero after errors,
/// and `--interpret` with the program's status.
#[doc(hidden)]
pub fn run() -> Result<i32, MainError> {
    crate::with_compilation_stack(COMPILATION_STACK_BYTES, run_on_current_thread)
        .map_err(MainError::CompilationStack)?
}

/// Address space reserved for the compilation worker's stack. Pages are
/// committed only as recursion touches them, so the size costs no memory.
pub(crate) const COMPILATION_STACK_BYTES: usize = 100 * 1024 * 1024 * 1024;

/// Keeps all compilation arenas, phases and output on the same thread.
fn run_on_current_thread() -> Result<i32, MainError> {
    let arguments = normalize_language_arguments(std::env::args_os())?;
    // Clap would demand an input for this flag, so it is answered before
    // parsing; the declaration below only documents it in `--help`.
    if arguments.len() == 2 && arguments[1] == "--print-default-passes" {
        return Ok(print_default_passes());
    }
    let args = Cli::try_parse_from(arguments)?;
    if args.report_unknown_warning_options() {
        return Ok(1);
    }
    if let Some(option) = args.unsupported_relocation_flag() {
        #[expect(
            clippy::disallowed_methods,
            reason = "The error carries the option out of `run`, past every arena."
        )]
        return Err(MainError::UnsupportedOption {
            option: option.to_owned(),
            target: args.target.triple(),
        });
    }
    if let Some(conflict) = args.code_generation.back_end_conflict() {
        code_generation::report_stderr(format_args!("error: {conflict}"));
        return Ok(1);
    }
    let tu = Bump::new();
    let (input_string, source_filename): (&str, &Path) =
        match (&args.input.input, &args.input.input_file) {
            | (Some(input), None) => (tu.alloc_str(input), Path::new("<input>")),
            | (None, Some(input_file)) => match tu.read_to_str_lossy(input_file) {
                | Ok(text) => (text, input_file),
                | Err(source) =>
                    return Err(MainError::OpenInputFileError {
                        path: input_file.clone(),
                        source,
                    }),
            },
            | _ => unreachable!("clap requires exactly one input source"),
        };
    if let (Some(input), Some(output)) = (&args.input.input_file, &args.code_generation.output_path)
    {
        let same_file = output_file::is_input(input, output).map_err(|source| {
            MainError::InspectOutputFileError {
                path: output.clone(),
                source,
            }
        })?;
        if same_file {
            return Err(MainError::OutputIsInput {
                input:  input.clone(),
                output: output.clone(),
            });
        }
    }
    let directories = args.search.directories();
    let search = HeaderSearch {
        quote:    tu.alloc_slice_fill_iter(directories.quote.iter().map(AsRef::as_ref)),
        angled:   tu.alloc_slice_fill_iter(directories.angled.iter().map(AsRef::as_ref)),
        system:   tu.alloc_slice_fill_iter(directories.system.iter().map(AsRef::as_ref)),
        resource: directories.resource,
        after:    tu.alloc_slice_fill_iter(directories.after.iter().map(AsRef::as_ref)),
    };
    let configuration = args.configuration();
    let mut context = Context::with_configuration(&tu, configuration);
    args.configure_preprocessing(&mut context);

    context.configuration = context
        .configuration
        .with_repeated_specifier_warnings(!args.no_repeated_specifier_warnings);

    if args.output.tokens {
        print_preprocessor_output(&mut context, source_filename, input_string, search);
    } else if args.code_generation.requested() {
        let status = generate_code(
            &mut context,
            source_filename,
            input_string,
            search,
            &args.code_generation,
        );
        // The failed compile job owns its requested object, even if it never
        // opened it. A front-end failure in link mode owns only a temporary
        // object; preserve the final executable as Clang does.
        if status != 0
            && args.code_generation.object
            && let Some(output) = &args.code_generation.output_path
        {
            output_file::remove_failed_object(output, &tu);
        }
        return Ok(status);
    } else {
        print_parser_output(
            &mut context,
            source_filename,
            input_string,
            search,
            &args.output.parser,
        );
    }
    Ok(0)
}

#[derive(Parser)]
#[command(author, version, about, long_about, color = ColorChoice::Always,
    after_help = "Preprocessing: -D NAME[=VALUE] or -DNAME[=VALUE], -U NAME or -UNAME, -include FILE.\nDefinitions default to 1. -D/-U run in order before forced includes.")]
#[expect(
    clippy::disallowed_types,
    reason = "clap's derived parser owns the repeated include directories as `Vec<PathBuf>`."
)]
struct Cli {
    /// Select the C target ABI (independent of the compiler host).
    #[arg(long, default_value = "x86_64-unknown-linux-gnu", value_parser = TargetParser)]
    target: crate::target::Target,
    /// Print the default bcc pass list, such as `inline,promote,fold,...`, for
    /// `--passes` ablation scripts, and exit.
    #[arg(long = "print-default-passes", exclusive = true)]
    _print_default_passes: bool,
    #[command(flatten)]
    input: CliInput,
    /// Select ISO C or a GNU dialect (also accepts GCC -std=VALUE).
    #[arg(long = "std", default_value = "gnu17", value_parser = StandardParser, action = clap::ArgAction::Set,
        overrides_with = "standard")]
    standard: LanguageMode,
    /// GCC language flags: -pedantic, -Wpedantic, -pedantic-errors,
    /// -ffreestanding, -fhosted, -f[no-]gnu89-inline, -fms-extensions, and
    /// -f[no-]ms-{declspec,int-types,calling-conventions,type-qualifiers,
    /// inline,seh,asm,pragma,anonymous-structs,va-args}; and the relocation
    /// models -fpic, -fPIC, -fpie, -fPIE and -fno-{pic,PIC,pie,PIE}.
    #[arg(long, hide = true)]
    language_option: Vec<String>,
    /// Select the ELF thread-local storage model (also accepts GCC
    /// -ftls-model=MODEL): global-dynamic (the default), local-dynamic,
    /// initial-exec or local-exec. The compiler may still choose a more
    /// specific model, as Clang does.
    #[arg(long, value_name = "MODEL", value_parser = TlsModelParser,
        action = clap::ArgAction::Set, overrides_with = "tls_model")]
    tls_model: Option<crate::target::TlsModel>,
    /// GCC startup preprocessing options, normalized with their operation.
    #[arg(long, hide = true, allow_hyphen_values = true, value_parser = preprocessing_option)]
    preprocessing_option: Vec<String>,
    #[command(flatten)]
    search: CliHeaderSearch,
    #[command(flatten)]
    output: CliOutput,
    #[command(flatten)]
    code_generation: CodeGeneration,
    /// Suppress the `repeated-specifiers` quality warning group.
    #[clap(long)]
    no_repeated_specifier_warnings: bool,
    /// Suppress all warning diagnostics (GCC -w).
    #[arg(short = 'w', long)]
    suppress_warnings: bool,
    /// Stop parsing with a diagnostic once this many recursive grammar
    /// productions are active at once. Each nested expression operand,
    /// statement, block, declarator and initializer brace counts one level.
    /// Raise it for machine-generated input that nests deeper.
    #[arg(long, value_name = "N",
        default_value_t = crate::configuration::DEFAULT_MAX_NESTING_DEPTH,
        value_parser = clap::value_parser!(u32).range(1..))]
    max_nesting_depth: u32,
    /// Spell `__DATE__` and `__TIME__` from seconds since the Unix epoch, in
    /// UTC.
    ///
    /// The flag overrides `SOURCE_DATE_EPOCH`. Without either, `__DATE__` and
    /// `__TIME__` spell the local time of translation. A malformed
    /// `SOURCE_DATE_EPOCH` is ignored; a malformed flag value is an error.
    #[arg(
        long,
        env = "SOURCE_DATE_EPOCH",
        value_name = "SECONDS",
        allow_negative_numbers = true,
        value_parser = SourceDateEpochParser
    )]
    source_date_epoch: Option<SourceDateEpoch>,
}

// Tests
#[cfg(test)]
mod tests;
