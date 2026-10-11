//! Command-line interface: argument parsing, token and syntax-tree output,
//! and diagnostic reporting.

mod arguments;

mod diagnostic_reporter;

mod errors;

mod measured;

mod output;

mod token;

mod search;

#[cfg(test)]
use std::fmt::Write as _;
use std::{
    env::{
        split_paths,
        var_os,
    },
    ffi::OsStr,
    io::{
        self,
        Write,
    },
    ops::RangeInclusive,
    path::Path,
};

use arguments::{
    CliInput,
    SourceDateEpoch,
    SourceDateEpochParser,
    StandardParser,
    TargetParser,
    normalize_language_arguments,
    preprocessing_option,
};
use chrono::{
    DateTime,
    Utc,
};
use clap::{
    Args,
    ColorChoice,
    Parser,
};
use diagnostic_reporter::DiagnosticReporter;
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
#[cfg(test)]
use output::{
    TokenOutput,
    print_preprocessor_output_in,
};
use search::CliHeaderSearch;
use thiserror::Error;
pub(crate) use token::describe_token;

use crate::{
    configuration::{
        CompilerConfiguration,
        ExtensionPolicy,
        LanguageMode,
        MsvcFeature,
    },
    diagnostics::ColorChoice as RenderColor,
    headers::HeaderSearch,
    pipeline::{
        parse_translation_unit,
        preprocess_with_diagnostics,
        with_preprocessor,
    },
    translation_phases::{
        Context,
        parsing::{
            InspectionOptions,
            ParsedTranslationUnit,
        },
        preprocessing::{
            CharacterTokenType,
            IntegerTokenType,
            StringTokenType,
            Token,
            TokenType,
        },
    },
    util::bump::{
        ArenaString,
        Bump,
    },
};

#[doc(hidden)]
pub fn run() -> Result<(), MainError> {
    let args = Cli::try_parse_from(normalize_language_arguments(std::env::args_os()))?;
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

    if args.output.tokens {
        print_preprocessor_output(&mut context, source_filename, input_string, search);
    } else {
        print_parser_output(
            &mut context,
            source_filename,
            input_string,
            search,
            &args.output.parser,
            !args.no_repeated_specifier_warnings,
        );
    }
    Ok(())
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
    #[command(flatten)]
    input: CliInput,
    /// Select ISO C or a GNU dialect (also accepts GCC -std=VALUE).
    #[arg(long = "std", default_value = "gnu17", value_parser = StandardParser, action = clap::ArgAction::Set,
        overrides_with = "standard")]
    standard: LanguageMode,
    /// GCC language flags: -pedantic, -Wpedantic, -pedantic-errors,
    /// -ffreestanding, -fhosted, -fms-extensions, and
    /// -f[no-]ms-{declspec,int-types,calling-conventions,type-qualifiers,
    /// inline,seh,asm,pragma,anonymous-structs,va-args}.
    #[arg(long, hide = true)]
    language_option: Vec<String>,
    /// GCC startup preprocessing options, normalized with their operation.
    #[arg(long, hide = true, allow_hyphen_values = true, value_parser = preprocessing_option)]
    preprocessing_option: Vec<String>,
    #[command(flatten)]
    search: CliHeaderSearch,
    #[command(flatten)]
    output: CliOutput,
    /// Suppress the `repeated-specifiers` quality warning group.
    #[clap(long)]
    no_repeated_specifier_warnings: bool,
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

#[cfg(test)]
mod tests;
