//! Command-line interface: argument parsing, token and syntax-tree output,
//! and diagnostic reporting.

use std::{
    env::{
        split_paths,
        var_os,
    },
    path::{
        Path,
        PathBuf,
    },
};

use clap::{
    Args,
    ColorChoice,
    Parser,
};
use thiserror::Error;

use crate::{
    diagnostics::{
        ColorChoice as RenderColor,
        Diagnostic,
        Renderer,
        ToDiagnostic,
        c_quoted,
        count_of,
    },
    pipeline::PreprocessorIterator,
    translation_phases::{
        Context,
        ErrorSeverity,
        GetSourceVectors,
        SourceVector,
        TranslationError,
        parsing::{
            InspectionOptions,
            Parser as LanguageParser,
        },
        preprocessing::{
            CharacterTokenType,
            IntegerTokenType,
            Preprocessor,
            StringTokenType,
            Token,
            TokenType,
        },
    },
    util::{
        read_to_string_lossy,
        shared::{
            SharedString,
            SharedVec,
        },
    },
};

#[derive(Parser)]
#[command(author, version, about, long_about, color = ColorChoice::Always)]
struct Cli {
    #[command(flatten)]
    input: CliInput,
    /// Add directory to include search path.
    #[clap(short = 'q', long = "iquote")]
    quote_include: Vec<PathBuf>,
    /// Add directory to system include search path.
    #[clap(short = 's', long = "isystem")]
    system_include: Vec<PathBuf>,
    #[command(flatten)]
    output: CliOutput,
    /// Suppress the `repeated-specifiers` quality warning group.
    #[clap(long)]
    no_repeated_specifier_warnings: bool,
}

#[derive(Args)]
struct CliOutput {
    /// Print preprocessor tokens instead of parser output.
    #[clap(long)]
    tokens: bool,
    #[command(flatten)]
    parser: ParserOutput,
}

#[derive(Args)]
struct ParserOutput {
    /// Print a deterministic, source-oriented C syntax tree.
    #[clap(long, conflicts_with = "tokens")]
    syntax_tree:      bool,
    /// Include line and column locations in `--syntax-tree` output.
    #[clap(long, requires = "syntax_tree")]
    syntax_locations: bool,
    /// Print raw parser arenas for storage debugging.
    #[clap(long, conflicts_with = "tokens")]
    raw_syntax:       bool,
}

#[derive(Args)]
#[group(required = true, multiple = false)]
struct CliInput {
    /// Input string to be parsed.
    #[clap(short, long, conflicts_with = "input_file")]
    input:      Option<String>,
    /// Input file to be parsed.
    #[clap(conflicts_with = "input")]
    input_file: Option<PathBuf>,
}

#[doc(hidden)]
#[derive(Debug, Error)]
pub enum MainError {
    #[error("error: cannot read `{}`: {source}", path.display())]
    OpenInputFileError {
        path:   PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    ParseArgumentsError(#[from] clap::Error),
}

/// Returns the directories of a GCC-style search-path variable.
///
/// Elements use the platform separator (`;` on Windows, `:` elsewhere). As
/// in GCC and Clang, an empty element names the working directory, while an
/// unset or empty variable contributes nothing.
fn include_path_from_env(env_var: &str) -> Vec<PathBuf> {
    match var_os(env_var) {
        | Some(value) if !value.is_empty() => split_paths(&value)
            .map(|path| {
                if path.as_os_str().is_empty() {
                    PathBuf::from(".")
                } else {
                    path
                }
            })
            .collect(),
        | _ => Vec::new(),
    }
}

#[doc(hidden)]
pub fn run() -> Result<(), MainError> {
    let mut args = Cli::try_parse()?;
    let (input_string, source_filename) =
        match (args.input.input.take(), args.input.input_file.take()) {
            | (Some(input), None) => (
                SharedString::from(input),
                PathBuf::from("<input>").into_boxed_path(),
            ),
            | (None, Some(input_file)) => (
                SharedString::from(read_to_string_lossy(&input_file).map_err(|source| {
                    MainError::OpenInputFileError {
                        path: input_file.clone(),
                        source,
                    }
                })?),
                input_file.into_boxed_path(),
            ),
            | _ => unreachable!("clap requires exactly one input source"),
        };
    // GCC searches `CPATH` like `-I` (before `-isystem`) and
    // `C_INCLUDE_PATH` like a trailing `-isystem`.
    let mut system_include = include_path_from_env("CPATH");
    system_include.append(&mut args.system_include);
    system_include.extend(include_path_from_env("C_INCLUDE_PATH"));
    args.system_include = system_include;

    if args.output.tokens {
        print_preprocessor_output(
            source_filename,
            input_string,
            args.quote_include.into(),
            args.system_include.into(),
        );
    } else {
        print_parser_output(
            source_filename,
            input_string,
            args.quote_include.into(),
            args.system_include.into(),
            &args.output.parser,
            !args.no_repeated_specifier_warnings,
        );
    }
    Ok(())
}

fn print_preprocessor_output(
    source_filename: Box<Path>,
    input_string: SharedString,
    quote_include: SharedVec<PathBuf>,
    system_include: SharedVec<PathBuf>,
) {
    let mut reporter = DiagnosticReporter::new();
    let mut iterator =
        PreprocessorIterator::new(source_filename, input_string, quote_include, system_include);
    loop {
        // A deferred diagnostic's labels index the preprocessor arena, which
        // the next poll may compact; render it while its provenance is live.
        if iterator.compacts_on_next() {
            reporter.flush(&iterator.context);
        }
        let Some(item) = iterator.next() else {
            break;
        };
        match item {
            | Ok(token) => {
                reporter.flush(&iterator.context);
                eprintln!("{}", describe_token(token, &iterator.context));
            },
            | Err(error) => reporter.report(&error, &mut iterator.context),
        }
    }
    reporter.finish(&iterator.context);
}

/// One line per token: its location, kind, source spelling, and for
/// constants the value and type the preprocessor assigned.
fn describe_token(token: Token, context: &Context) -> String {
    let spelling = context
        .string_cache
        .at(token.contents)
        .trim_end_matches('\0');
    let description = match token.kind {
        | TokenType::Identifier => format!("identifier `{spelling}`"),
        | TokenType::Keyword(keyword) => format!("keyword `{}`", keyword.spelling()),
        | TokenType::Operator(operator) => format!("punctuator `{}`", operator.spelling()),
        | TokenType::String(StringTokenType::String(contents)) => format!(
            "string literal {}",
            c_quoted("", '"', context.string_cache.at(contents))
        ),
        | TokenType::String(StringTokenType::WideString(contents)) => format!(
            "wide string literal {}",
            c_quoted("L", '"', context.string_cache.at(contents))
        ),
        | TokenType::Character(character) => {
            let (value, type_name) = match character {
                | CharacterTokenType::Char(c) => (i64::from(u32::from(c)), "int"),
                | CharacterTokenType::WideChar(c) => (i64::from(u32::from(c)), "wchar_t"),
                | CharacterTokenType::MultiChar(value) => (i64::from(value), "int"),
            };
            format!("character constant `{spelling}` = {value} ({type_name})")
        },
        | TokenType::Integer(integer) => {
            let (value, type_name) = match integer {
                | IntegerTokenType::Int(value) => (i128::from(value), "int"),
                | IntegerTokenType::Long(value) => (i128::from(value), "long"),
                | IntegerTokenType::LongLong(value) => (i128::from(value), "long long"),
                | IntegerTokenType::UnsignedInt(value) => (i128::from(value), "unsigned int"),
                | IntegerTokenType::UnsignedLong(value) => (i128::from(value), "unsigned long"),
                | IntegerTokenType::UnsignedLongLong(value) =>
                    (i128::from(value), "unsigned long long"),
            };
            format!("integer constant `{spelling}` = {value} ({type_name})")
        },
        | TokenType::Float(float) => format!(
            "floating constant `{spelling}` = {float} ({})",
            float.type_name()
        ),
    };
    match context.get_source_vectors(token.source_vectors).first() {
        | Some(vector) => format!(
            "{}:{}:{}: {description}",
            context.get_source_file(vector.source_file_index).display(),
            vector.line,
            vector.column,
        ),
        | None => description,
    }
}

fn print_parser_output(
    source_filename: Box<Path>,
    input_string: SharedString,
    quote_include: SharedVec<PathBuf>,
    system_include: SharedVec<PathBuf>,
    output: &ParserOutput,
    repeated_specifier_warnings: bool,
) {
    let mut context = Context::new();
    context.configuration = context
        .configuration
        .with_repeated_specifier_warnings(repeated_specifier_warnings);
    let preprocessor = Preprocessor::new(
        &mut context,
        source_filename,
        input_string,
        quote_include,
        system_include,
    );
    let unit = LanguageParser::new(preprocessor).parse_translation_unit(&mut context);
    let mut reporter = DiagnosticReporter::new();
    while let Some(error) = context.pop_pending_error() {
        reporter.report(&error, &mut context);
    }
    reporter.flush(&context);

    if output.syntax_tree {
        eprint!(
            "{}",
            unit.syntax().inspect(
                unit.external_declarations(),
                &context,
                InspectionOptions {
                    show_locations: output.syntax_locations,
                },
            )
        );
    }
    if output.raw_syntax {
        eprintln!("{:#?}", unit.syntax().raw_debug());
    }
    reporter.finish(&context);
}

/// Renders diagnostics to stderr and summarizes them at the end, like
/// `N errors and M warnings generated`.
///
/// An error reported at exactly the same place as the error before it is a
/// cascade from the same mistake, so it is folded into that error rather
/// than printed again.
struct DiagnosticReporter {
    renderer: Renderer,
    pending:  Option<(Diagnostic, Vec<SourceVector>)>,
    errors:   usize,
    warnings: usize,
}

impl DiagnosticReporter {
    fn new() -> Self {
        Self {
            renderer: Renderer::new(RenderColor::for_stderr()),
            pending:  None,
            errors:   0,
            warnings: 0,
        }
    }

    fn report(&mut self, error: &TranslationError, context: &mut Context) {
        let source = error.source_vectors(context);
        let diagnostic = error.to_diagnostic(context, source);
        let location = context.get_source_vectors(source).to_vec();
        if let Some((pending, pending_location)) = &mut self.pending
            && diagnostic.severity == ErrorSeverity::Error
            && pending.severity == ErrorSeverity::Error
            && !location.is_empty()
            && *pending_location == location
        {
            pending.absorb(diagnostic, context);
            return;
        }
        self.flush(context);
        self.pending = Some((diagnostic, location));
    }

    fn flush(&mut self, context: &Context) {
        let Some((diagnostic, _)) = self.pending.take() else {
            return;
        };
        match diagnostic.severity {
            | ErrorSeverity::Error => self.errors += 1,
            | ErrorSeverity::Warning => self.warnings += 1,
            | ErrorSeverity::Note => {},
        }
        eprint!("{}", self.renderer.render(&diagnostic, context));
    }

    fn finish(&mut self, context: &Context) {
        self.flush(context);
        let counts: Vec<String> = [(self.errors, "error"), (self.warnings, "warning")]
            .into_iter()
            .filter(|&(count, _)| count > 0)
            .map(|(count, noun)| count_of(count, noun))
            .collect();
        if !counts.is_empty() {
            eprintln!("{} generated.", counts.join(" and "));
        }
    }
}
