//! Command-line interface: argument parsing, token and syntax-tree output,
//! and diagnostic reporting.

#[expect(
    clippy::disallowed_types,
    reason = "clap parses path arguments into `PathBuf`s; the compiler borrows them as `&Path`."
)]
use std::path::PathBuf;
use std::{
    env::{
        split_paths,
        var_os,
    },
    ffi::OsStr,
    fmt::Write as _,
    io::{
        self,
        Write,
    },
    ops::RangeInclusive,
    path::Path,
};

use chrono::{
    DateTime,
    Utc,
};
use clap::{
    Arg,
    Args,
    ColorChoice,
    Command,
    Parser,
    builder::{
        RangedI64ValueParser,
        TypedValueParser,
    },
    parser::ValueSource,
};
use rustc_hash::FxBuildHasher;
use thiserror::Error;

use crate::{
    configuration::{
        CompilerConfiguration,
        ExtensionPolicy,
        LanguageMode,
        MsvcFeature,
    },
    diagnostics::{
        ColorChoice as RenderColor,
        Diagnostic,
        Renderer,
        ToDiagnostic,
        count_of,
    },
    pipeline::{
        parse_translation_unit,
        preprocess_with_diagnostics,
        with_preprocessor,
    },
    translation_phases::{
        Context,
        ErrorSeverity,
        GetSourceVectors,
        SourceVector,
        TranslationError,
        parsing::{
            InspectionOptions,
            ParsedTranslationUnit,
        },
        preprocessing::{
            CharacterTokenType,
            IntegerTokenType,
            PreprocessorErrorType,
            StringTokenType,
            Token,
            TokenType,
        },
    },
    util::bump::{
        ArenaMap,
        ArenaString,
        ArenaVec,
        Bump,
    },
};

#[derive(Parser)]
#[command(author, version, about, long_about, color = ColorChoice::Always)]
#[expect(
    clippy::disallowed_types,
    reason = "clap's derived parser owns the repeated include directories as `Vec<PathBuf>`."
)]
struct Cli {
    #[command(flatten)]
    input: CliInput,
    /// Select ISO C or a GNU dialect (also accepts GCC -std=VALUE).
    #[arg(long = "std", default_value = "gnu17", value_parser = StandardParser, action = clap::ArgAction::Set,
        overrides_with = "standard")]
    standard: LanguageMode,
    /// GCC language flags: -pedantic, -Wpedantic, -pedantic-errors,
    /// -fms-extensions, and -f[no-]ms-{declspec,int-types,calling-conventions,
    /// type-qualifiers,inline,seh,asm,pragma,anonymous-structs,va-args}.
    #[arg(long, hide = true)]
    language_option: Vec<String>,
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

/// Exact clang-style explanation, deliberately omitting deprecated aliases.
const STANDARD_NOTES: &str =
    "note: use 'c89', 'c90', or 'iso9899:1990' for 'ISO C 1990' standard\nnote: use \
     'iso9899:199409' for 'ISO C 1990 with amendment 1' standard\nnote: use 'gnu89' or 'gnu90' \
     for 'ISO C 1990 with GNU extensions' standard\nnote: use 'c99' or 'iso9899:1999' for 'ISO C \
     1999' standard\nnote: use 'gnu99' for 'ISO C 1999 with GNU extensions' standard\nnote: use \
     'c11' or 'iso9899:2011' for 'ISO C 2011' standard\nnote: use 'gnu11' for 'ISO C 2011 with \
     GNU extensions' standard\nnote: use 'c17', 'iso9899:2017', 'c18', or 'iso9899:2018' for 'ISO \
     C 2017' standard\nnote: use 'gnu17' or 'gnu18' for 'ISO C 2017 with GNU extensions' \
     standard\nnote: use 'c23' or 'iso9899:2024' for 'ISO C 2023' standard\nnote: use 'gnu23' for \
     'ISO C 2023 with GNU extensions' standard\nnote: use 'c2y' for 'Working Draft for ISO C2y' \
     standard\nnote: use 'gnu2y' for 'Working Draft for ISO C2y with GNU extensions' standard\n";

#[derive(Clone)]
struct StandardParser;
impl TypedValueParser for StandardParser {
    type Value = LanguageMode;

    #[expect(
        clippy::disallowed_macros,
        reason = "Clap argument errors own their startup message."
    )]
    fn parse_ref(
        &self,
        _cmd: &Command,
        _arg: Option<&Arg>,
        value: &OsStr,
    ) -> Result<LanguageMode, clap::Error> {
        value.to_str().and_then(LanguageMode::parse).ok_or_else(|| {
            clap::Error::raw(
                clap::error::ErrorKind::InvalidValue,
                format!(
                    "invalid value '{}' in '-std={}'\n{STANDARD_NOTES}",
                    value.to_string_lossy(),
                    value.to_string_lossy()
                ),
            )
        })
    }
}

/// Normalize GCC's single-dash long options before clap. Values and tokens
/// after `--` are opaque, including an input string that looks like a flag.
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    reason = "Startup argv normalization owns OS strings beside clap; never a compilation buffer."
)]
fn normalize_language_arguments(
    arguments: impl IntoIterator<Item = std::ffi::OsString>,
) -> Vec<std::ffi::OsString> {
    let mut normalized = Vec::new();
    let mut opaque_value = true; // argv[0]
    let mut positional = false;
    for argument in arguments {
        if opaque_value || positional {
            opaque_value = false;
            normalized.push(argument);
            continue;
        }
        let Some(text) = argument.to_str() else {
            normalized.push(argument);
            continue;
        };
        if text == "--" {
            positional = true;
        }
        if let Some(value) = text.strip_prefix("-std=") {
            normalized.push(format!("--std={value}").into());
        } else if text == "-std" {
            normalized.push("--std".into());
            opaque_value = true;
        } else if matches!(text, "-pedantic" | "-Wpedantic" | "-pedantic-errors")
            || valid_msvc_flag(text)
        {
            normalized.push(format!("--language-option={text}").into());
        } else {
            opaque_value = matches!(
                text,
                "--std"
                    | "--input"
                    | "-i"
                    | "--iquote"
                    | "-q"
                    | "--isystem"
                    | "-s"
                    | "--source-date-epoch"
                    | "--language-option"
            );
            normalized.push(argument);
        }
    }
    normalized
}
fn valid_msvc_flag(text: &str) -> bool {
    text.strip_prefix("-fms-")
        .or_else(|| text.strip_prefix("-fno-ms-"))
        .is_some_and(|name| name == "extensions" || MsvcFeature::parse(name).is_some())
}
impl Cli {
    fn configuration(&self) -> CompilerConfiguration {
        let mut configuration =
            CompilerConfiguration::new(self.standard.standard, ExtensionPolicy::Allow)
                .with_gnu_extensions(self.standard.gnu);
        for option in &self.language_option {
            match option.as_str() {
                | "-pedantic" | "-Wpedantic" =>
                    configuration = configuration.with_extension_policy(ExtensionPolicy::Warn),
                | "-pedantic-errors" =>
                    configuration = configuration.with_extension_policy(ExtensionPolicy::Deny),
                | text => {
                    let (name, enabled) = if let Some(name) = text.strip_prefix("-fms-") {
                        (name, true)
                    } else if let Some(name) = text.strip_prefix("-fno-ms-") {
                        (name, false)
                    } else {
                        continue;
                    };
                    configuration = if name == "extensions" {
                        configuration.with_msvc_extensions(enabled)
                    } else if let Some(feature) = MsvcFeature::parse(name) {
                        configuration.with_msvc_feature(feature, enabled)
                    } else {
                        configuration
                    };
                },
            }
        }
        configuration.with_source_date_epoch(self.source_date_epoch.and_then(|epoch| epoch.0))
    }
}

/// The seconds since the Unix epoch that `__DATE__` and `__TIME__` spell,
/// or `None` for a malformed `SOURCE_DATE_EPOCH`, which is ignored.
#[derive(Debug, Clone, Copy)]
struct SourceDateEpoch(Option<i64>);

/// The seconds since the Unix epoch that a date can represent.
const SOURCE_DATE_EPOCH_RANGE: RangeInclusive<i64> =
    DateTime::<Utc>::MIN_UTC.timestamp()..=DateTime::<Utc>::MAX_UTC.timestamp();

/// Parses a [`SourceDateEpoch`]: a decimal integer with an optional sign and
/// surrounding whitespace, within [`SOURCE_DATE_EPOCH_RANGE`]. The
/// `SOURCE_DATE_EPOCH` variable ignores any other value, as reproducible
/// builds expect and as the preprocessor did when it read the variable
/// itself; the `--source-date-epoch` flag rejects it.
#[derive(Debug, Clone, Copy)]
struct SourceDateEpochParser;

impl TypedValueParser for SourceDateEpochParser {
    type Value = SourceDateEpoch;

    fn parse_ref(
        &self,
        cmd: &Command,
        arg: Option<&Arg>,
        value: &OsStr,
    ) -> Result<Self::Value, clap::Error> {
        self.parse_ref_(cmd, arg, value, ValueSource::CommandLine)
    }

    fn parse_ref_(
        &self,
        cmd: &Command,
        arg: Option<&Arg>,
        value: &OsStr,
        source: ValueSource,
    ) -> Result<Self::Value, clap::Error> {
        let seconds = value
            .to_str()
            .and_then(|value| value.trim().parse::<i64>().ok())
            .filter(|seconds| SOURCE_DATE_EPOCH_RANGE.contains(seconds));
        if seconds.is_some() || source == ValueSource::EnvVariable {
            return Ok(SourceDateEpoch(seconds));
        }
        // Clap's own integer parser explains why the value is invalid.
        RangedI64ValueParser::<i64>::new()
            .range(SOURCE_DATE_EPOCH_RANGE)
            .parse_ref(cmd, arg, value)
            .map(|seconds| SourceDateEpoch(Some(seconds)))
    }
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
    /// Print the raw syntax tree, in Rust debug form, for storage debugging.
    #[clap(long, conflicts_with = "tokens")]
    raw_syntax:       bool,
}

#[derive(Args)]
#[group(required = true, multiple = false)]
#[expect(
    clippy::disallowed_types,
    reason = "clap's derived parser owns the input string and the input file path."
)]
struct CliInput {
    /// Input string to be parsed.
    #[clap(short, long, conflicts_with = "input_file", allow_hyphen_values = true)]
    input:      Option<String>,
    /// Input file to be parsed.
    #[clap(conflicts_with = "input")]
    input_file: Option<PathBuf>,
}

#[doc(hidden)]
#[derive(Debug, Error)]
#[expect(
    clippy::disallowed_types,
    reason = "Carries the input path out of `run`, past its arenas, for `main`'s message."
)]
pub enum MainError {
    #[error("error: cannot read `{}`: {source}", path.display())]
    OpenInputFileError { path: PathBuf, source: io::Error },
    #[error(transparent)]
    ParseArgumentsError(#[from] clap::Error),
}

/// Returns the directories of a GCC-style search-path variable.
///
/// Elements use the platform separator (`;` on Windows, `:` elsewhere). As
/// in GCC and Clang, an empty element names the working directory, while an
/// unset or empty variable contributes nothing.
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "Startup reads CPATH-style variables beside clap's arguments; `run` borrows the \
              paths as `&Path`."
)]
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
    let mut args = Cli::try_parse_from(normalize_language_arguments(std::env::args_os()))?;
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
    // GCC searches `CPATH` like `-I` (before `-isystem`) and
    // `C_INCLUDE_PATH` like a trailing `-isystem`.
    let mut system_include = include_path_from_env("CPATH");
    system_include.append(&mut args.system_include);
    system_include.extend(include_path_from_env("C_INCLUDE_PATH"));
    let quote_include = tu.alloc_slice_fill_iter(args.quote_include.iter().map(Path::new));
    let system_include = tu.alloc_slice_fill_iter(system_include.iter().map(Path::new));
    let configuration = args.configuration();
    let mut context = Context::with_configuration(&tu, configuration);

    if args.output.tokens {
        print_preprocessor_output(
            &mut context,
            source_filename,
            input_string,
            quote_include,
            system_include,
        );
    } else {
        print_parser_output(
            &mut context,
            source_filename,
            input_string,
            quote_include,
            system_include,
            &args.output.parser,
            !args.no_repeated_specifier_warnings,
        );
    }
    Ok(())
}

/// Prints the preprocessed translation unit, each token after the
/// diagnostics its production reported.
fn print_preprocessor_output<'tu>(
    context: &mut Context<'tu>,
    source_filename: &Path,
    input_string: &'tu str,
    quote_include: &[&Path],
    system_include: &[&Path],
) {
    let mut reporter_arena = Bump::new();
    let mut stderr = io::stderr();
    print_preprocessor_output_in(
        context,
        source_filename,
        input_string,
        quote_include,
        system_include,
        TokenOutput {
            out:            &mut stderr,
            reporter_arena: &mut reporter_arena,
            color:          RenderColor::for_stderr(),
        },
    );
}

struct TokenOutput<'a> {
    out:            &'a mut dyn Write,
    reporter_arena: &'a mut Bump,
    color:          RenderColor,
}

/// Keeps only the diagnostic counts between token batches. Each batch's
/// folding state and copied locations are discarded before the next token.
fn print_preprocessor_output_in<'tu>(
    context: &mut Context<'tu>,
    source_filename: &Path,
    input_string: &'tu str,
    quote_include: &[&Path],
    system_include: &[&Path],
    TokenOutput {
        out,
        reporter_arena,
        color,
    }: TokenOutput<'_>,
) {
    let mut scratch = Bump::new();
    let items = with_preprocessor(
        context,
        source_filename,
        input_string,
        quote_include,
        system_include,
        |preprocessor, context, _pp| preprocess_with_diagnostics(preprocessor, context),
    );
    let mut items = items.into_iter();
    let (mut errors, mut warnings) = (0, 0);
    loop {
        let mut reporter = DiagnosticReporter::new(reporter_arena, context.tu_arena(), color);
        reporter.errors = errors;
        reporter.warnings = warnings;
        let token = loop {
            match items.next() {
                | Some(Err(error)) => reporter.report(&error, context),
                | Some(Ok(token)) => {
                    expect_stderr(reporter.flush(context, out));
                    break Some(token);
                },
                | None => {
                    expect_stderr(reporter.finish(context, out));
                    break None;
                },
            }
        };
        errors = reporter.errors;
        warnings = reporter.warnings;
        drop(reporter);
        reporter_arena.reset();
        match token {
            | Some(token) => {
                scratch.reset();
                expect_stderr(writeln!(
                    out,
                    "{}",
                    describe_token(token, context, &scratch)
                ));
            },
            | None => break,
        }
    }
}

/// Fails like `eprint!` when stderr cannot be written.
fn expect_stderr(result: io::Result<()>) {
    if let Err(error) = result {
        panic!("failed printing to stderr: {error}");
    }
}

/// One line per token: its location, kind, source spelling, and for
/// constants the value and type the preprocessor assigned. The line is
/// written in `scratch`.
pub(crate) fn describe_token<'a>(
    token: Token,
    context: &Context<'_>,
    scratch: &'a Bump,
) -> &'a str {
    let spelling = context
        .string_cache
        .at(token.contents)
        .trim_end_matches('\0');
    let literal = match token.kind {
        | TokenType::String(
            StringTokenType::String(contents) | StringTokenType::EncodedString(contents, _),
        ) => context.literal_spelling_in(scratch, scratch, contents, false),
        | TokenType::String(StringTokenType::WideString(contents)) =>
            context.literal_spelling_in(scratch, scratch, contents, true),
        | _ => "",
    };
    let mut line = ArenaString::new_in(scratch);
    if let Some(vector) = context.get_source_vectors(token.source_vectors).first() {
        _ = write!(
            line,
            "{}:{}:{}: ",
            context.get_source_file(vector.source_file_index).display(),
            vector.line,
            vector.column,
        );
    }
    _ = match token.kind {
        | TokenType::Identifier => write!(line, "identifier `{spelling}`"),
        | TokenType::Keyword(_) => write!(line, "keyword `{spelling}`"),
        | TokenType::Operator(operator) => write!(line, "punctuator `{}`", operator.spelling()),
        | TokenType::String(StringTokenType::EncodedString(_, encoding)) => write!(
            line,
            "{} string literal {}{literal}",
            encoding.type_name(),
            encoding.prefix()
        ),
        | TokenType::String(StringTokenType::String(_)) => write!(line, "string literal {literal}"),
        | TokenType::String(StringTokenType::WideString(_)) =>
            write!(line, "wide string literal {literal}"),
        | TokenType::Character(character) => {
            let (value, type_name) = match character {
                | CharacterTokenType::EncodedChar(c, encoding) =>
                    (i64::from(c), encoding.type_name()),
                | CharacterTokenType::Char(c) => (i64::from(u32::from(c)), "int"),
                | CharacterTokenType::WideChar(c) => (i64::from(c), "wchar_t"),
                | CharacterTokenType::MultiChar(value) => (i64::from(value), "int"),
            };
            write!(
                line,
                "character constant `{spelling}` = {value} ({type_name})"
            )
        },
        | TokenType::Integer(integer) => {
            let (value, type_name) = match integer {
                | IntegerTokenType::BitInt(value, width, unsigned) => {
                    _ = write!(
                        line,
                        "integer constant `{spelling}` = {} ({}_BitInt({width}))",
                        value.get(),
                        if unsigned { "unsigned " } else { "" }
                    );
                    return line.into_str();
                },
                | IntegerTokenType::Imaginary(value, component) => {
                    // GNU imaginary integer constants extend C99 §6.4.4.1
                    // under §4p6; preserve the imaginary component as for
                    // floats.
                    _ = write!(
                        line,
                        "integer constant `{spelling}` = {}i ({})",
                        value.get(),
                        component.type_name()
                    );
                    return line.into_str();
                },
                | IntegerTokenType::Int(value) => (i128::from(value), "int"),
                | IntegerTokenType::Long(value) => (i128::from(value.get()), "long"),
                | IntegerTokenType::LongLong(value) => (i128::from(value.get()), "long long"),
                | IntegerTokenType::UnsignedInt(value) => (i128::from(value), "unsigned int"),
                | IntegerTokenType::UnsignedLong(value) =>
                    (i128::from(value.get()), "unsigned long"),
                | IntegerTokenType::UnsignedLongLong(value) =>
                    (i128::from(value.get()), "unsigned long long"),
            };
            write!(
                line,
                "integer constant `{spelling}` = {value} ({type_name})"
            )
        },
        | TokenType::Float(float) => write!(
            line,
            "floating constant `{spelling}` = {float} ({})",
            float.type_name()
        ),
    };
    line.into_str()
}

fn print_parser_output<'tu>(
    context: &mut Context<'tu>,
    source_filename: &Path,
    input_string: &'tu str,
    quote_include: &[&Path],
    system_include: &[&Path],
    output: &ParserOutput,
    repeated_specifier_warnings: bool,
) {
    context.configuration = context
        .configuration
        .with_repeated_specifier_warnings(repeated_specifier_warnings);
    let unit = parse_translation_unit(
        context,
        source_filename,
        input_string,
        quote_include,
        system_include,
    );
    let reporter_arena = Bump::new();
    let mut reporter = DiagnosticReporter::new(
        &reporter_arena,
        context.tu_arena(),
        RenderColor::for_stderr(),
    );
    let stderr = &mut io::stderr();
    expect_stderr(reporter.report_pending(context, stderr));

    if output.syntax_tree {
        let inspection = Bump::new();
        eprint!(
            "{}",
            unit.inspect(
                &inspection,
                context,
                InspectionOptions {
                    show_locations: output.syntax_locations,
                },
            )
        );
    }
    if output.raw_syntax {
        print_raw_syntax(&unit);
    }
    expect_stderr(reporter.finish(context, stderr));
}

/// A step of [`compile_file_measured`].
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompileStep {
    /// Preprocessing and parsing, which record the diagnostics.
    Parse,
    /// Building, folding, ordering, and rendering the recorded diagnostics.
    Report,
}

/// Compiles the file at `path` in the library's default mode, writing its
/// diagnostics and their summary to `out` without color. Each step runs
/// inside `measure`, so a caller can observe one step alone; the allocation
/// tests count the global allocations each makes.
///
/// # Errors
///
/// Reading `path` or writing to `out` can fail.
#[doc(hidden)]
pub fn compile_file_measured(
    path: &Path,
    out: &mut dyn Write,
    measure: impl FnMut(CompileStep, &mut dyn FnMut()),
) -> io::Result<()> {
    compile_file_configured_measured(path, CompilerConfiguration::default(), out, measure)
}

/// Measures compilation with CLI standard, dialect and policy arguments.
/// Argument parsing runs before either measured interval. Include-path and
/// output options are not applied by this diagnostic measurement adapter.
///
/// # Errors
///
/// Invalid CLI arguments, reading `path`, or writing to `out` can fail.
#[doc(hidden)]
#[expect(
    clippy::disallowed_types,
    reason = "CLI argument normalization owns OS strings before measured compiler intervals."
)]
pub fn compile_file_with_arguments_measured(
    path: &Path,
    arguments: &[&str],
    out: &mut dyn Write,
    measure: impl FnMut(CompileStep, &mut dyn FnMut()),
) -> io::Result<()> {
    let args = Cli::try_parse_from(normalize_language_arguments(
        ["bcc-rust", "--input", ""]
            .into_iter()
            .chain(arguments.iter().copied())
            .map(std::ffi::OsString::from),
    ))
    .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    compile_file_configured_measured(path, args.configuration(), out, measure)
}

fn compile_file_configured_measured(
    path: &Path,
    configuration: CompilerConfiguration,
    out: &mut dyn Write,
    mut measure: impl FnMut(CompileStep, &mut dyn FnMut()),
) -> io::Result<()> {
    let tu = Bump::new();
    let source = tu.read_to_str_lossy(path)?;
    let mut context = Context::with_configuration(&tu, configuration);
    measure(CompileStep::Parse, &mut || {
        drop(parse_translation_unit(&mut context, path, source, &[], &[]));
    });
    let mut result = Ok(());
    measure(CompileStep::Report, &mut || {
        let reporter_arena = Bump::new();
        let mut reporter = DiagnosticReporter::new(&reporter_arena, &tu, RenderColor::Plain);
        result = reporter
            .report_pending(&mut context, out)
            .and_then(|()| reporter.finish(&context, out));
    });
    result
}

/// Stack reserved for rendering `--raw-syntax`. The derived `Debug` output
/// recurses once per nesting level of the tree, which the iterative parser
/// accepts far deeper than the main thread's stack allows. The stack is
/// reserved, not committed, so unused depth costs only address space.
const RAW_SYNTAX_STACK_BYTES: usize = 1 << 30;

/// Prints the whole syntax tree in Rust debug form to stderr, rendered in
/// an arena on a thread with a stack deep enough for deeply nested syntax.
#[expect(
    clippy::disallowed_methods,
    reason = "std's thread builder takes the name as a `String`; spawning the deep-stack thread \
              allocates in std anyway."
)]
fn print_raw_syntax(unit: &ParsedTranslationUnit<'_>) {
    let raw = unit.raw_debug();
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("raw-syntax".to_owned())
            .stack_size(RAW_SYNTAX_STACK_BYTES)
            .spawn_scoped(scope, move || {
                let arena = Bump::new();
                let mut text = ArenaString::new_in(&arena);
                _ = write!(text, "{raw:#?}");
                eprintln!("{text}");
            })
            .expect("the raw syntax thread starts")
            .join()
            .expect("raw syntax rendering finishes");
    });
}

/// Renders diagnostics to a caller's writer (stderr, for the CLI) and
/// summarizes them at the end, like `N errors and M warnings generated`.
///
/// An error reported at exactly the same place as an earlier error from the
/// same mistake is folded into it rather than printed again: a parser error
/// into the parser error just before it when no input was consumed between
/// them, otherwise into a lexing or preprocessing error at that place; a
/// preprocessing error into the preprocessing error just before it. The
/// parser and the preprocessor are considered separately.
///
/// Pending diagnostics are built in the translation-unit arena (`'tu`). The
/// pending list, the folding map, and the locations live in the reporter's
/// own arena (`'r`). Parser mode keeps it for the whole compilation; token
/// mode resets it between flushed batches.
struct DiagnosticReporter<'r, 'tu> {
    arena:        &'r Bump,
    diagnostics:  &'tu Bump,
    renderer:     Renderer,
    pending:      ArenaVec<'r, PendingDiagnostic<'r, 'tu>>,
    /// The parser diagnostic reported last: where it was folded or stored,
    /// and the input the parser had consumed by then.
    last_parser:  Option<(usize, usize)>,
    /// Where the preprocessing diagnostic reported last was folded or stored.
    last_other:   Option<usize>,
    /// The latest pending preprocessing error at each location.
    other_errors: ArenaMap<'r, &'r [SourceVector], usize>,
    errors:       usize,
    warnings:     usize,
}

/// One diagnostic awaiting rendering, with any later errors folded in.
struct PendingDiagnostic<'r, 'tu> {
    diagnostic: Diagnostic<'tu>,
    location: &'r [SourceVector],
    ordering_location: Option<(u32, u32)>,
    /// Its place among the pending diagnostics when it was reported, which
    /// keeps the order of diagnostics at one location stable.
    sequence: usize,
    /// Whether errors at the same place may be folded into this one.
    foldable: bool,
    /// An unclosed angle header and an empty translation unit remain two
    /// separate diagnostics even when they point at the same EOF position.
    preserve_empty_translation_unit: bool,
}

impl PendingDiagnostic<'_, '_> {
    fn absorbs(&self, location: &[SourceVector]) -> bool {
        self.foldable
            && self.diagnostic.severity == ErrorSeverity::Error
            && self.location == location
    }
}

impl<'r, 'tu> DiagnosticReporter<'r, 'tu> {
    fn new(arena: &'r Bump, diagnostics: &'tu Bump, color: RenderColor) -> Self {
        Self {
            arena,
            diagnostics,
            renderer: Renderer::new(color),
            pending: ArenaVec::new_in(arena),
            last_parser: None,
            last_other: None,
            other_errors: ArenaMap::with_hasher_in(FxBuildHasher, arena),
            errors: 0,
            warnings: 0,
        }
    }

    fn report(&mut self, error: &TranslationError<'_>, context: &mut Context<'_>) {
        let source = error.source_vectors(context);
        let diagnostic = error.diagnostic_in(context, source, self.diagnostics);
        let location = context.get_source_vectors(source);
        let ordering_location = match error {
            | TranslationError::Parsing(error) => error.ordering_location,
            // Preprocessing errors may still point into a macro definition.
            | _ if diagnostic.severity != ErrorSeverity::Warning => None,
            | _ => location
                .first()
                .map(|source| (source.source_file_index, source.index)),
        };
        let (parser, consumed, foldable) = match error {
            | TranslationError::Parsing(error) => (true, error.consumed_tokens, error.may_fold()),
            | _ => (false, 0, true),
        };
        let empty_translation_unit =
            matches!(error, TranslationError::Parsing(error) if error.is_empty_translation_unit());
        let target =
            if diagnostic.severity == ErrorSeverity::Error && foldable && !location.is_empty() {
                if parser {
                    self.last_parser
                        .filter(|&(index, last_consumed)| {
                            last_consumed == consumed && self.pending[index].absorbs(location)
                        })
                        .map(|(index, _)| index)
                        .or_else(|| {
                            self.other_errors.get(location).copied().filter(|&index| {
                                self.pending[index].absorbs(location)
                                    && !(empty_translation_unit
                                        && self.pending[index].preserve_empty_translation_unit)
                            })
                        })
                } else {
                    self.last_other
                        .filter(|&index| self.pending[index].absorbs(location))
                }
            } else {
                None
            };
        let index = if let Some(index) = target {
            self.pending[index].diagnostic.absorb(diagnostic, context);
            index
        } else {
            let location: &'r [SourceVector] =
                self.arena.alloc_slice_fill_iter(location.iter().cloned());
            if !parser && diagnostic.severity == ErrorSeverity::Error && !location.is_empty() {
                _ = self.other_errors.insert(location, self.pending.len());
            }
            self.pending.push(PendingDiagnostic {
                diagnostic,
                location,
                ordering_location,
                sequence: self.pending.len(),
                foldable,
                preserve_empty_translation_unit: matches!(
                    error,
                    TranslationError::Preprocessing(error)
                        if matches!(error.error_type, PreprocessorErrorType::UnterminatedHeaderName('>'))
                ),
            });
            self.pending.len() - 1
        };
        if parser {
            self.last_parser = Some((index, consumed));
        } else {
            self.last_other = Some(index);
        }
    }

    /// Reports every pending diagnostic of a parsed translation unit, in
    /// source order, and writes them to `out`.
    fn report_pending(&mut self, context: &mut Context<'_>, out: &mut dyn Write) -> io::Result<()> {
        while let Some(error) = context.pop_pending_error() {
            self.report(&error, context);
        }
        self.order_source_runs();
        self.flush(context, out)
    }

    /// Lookahead can fetch a warning beyond the current parser error. Order
    /// each file run only after folding; preprocessing errors, file transitions
    /// and unknown locations
    /// remain barriers, and macro diagnostics use their captured invocation.
    fn order_source_runs(&mut self) {
        for run in self.pending.chunk_by_mut(|left, right| {
            left.ordering_location.is_some()
                && left.ordering_location.map(|(file, _)| file)
                    == right.ordering_location.map(|(file, _)| file)
        }) {
            // Diagnostics at one location keep the order they were reported
            // in. An unstable sort with that tiebreak needs no buffer.
            run.sort_unstable_by_key(|diagnostic| {
                (diagnostic.ordering_location, diagnostic.sequence)
            });
        }
        self.last_parser = None;
        self.last_other = None;
        self.other_errors.clear();
    }

    fn flush(&mut self, context: &Context<'_>, out: &mut dyn Write) -> io::Result<()> {
        self.last_parser = None;
        self.last_other = None;
        self.other_errors.clear();
        for PendingDiagnostic { diagnostic, .. } in self.pending.drain(..) {
            match diagnostic.severity {
                | ErrorSeverity::Error => self.errors += 1,
                | ErrorSeverity::Warning => self.warnings += 1,
                | ErrorSeverity::Note => {},
            }
            out.write_all(self.renderer.render_text(&diagnostic, context).as_bytes())?;
        }
        Ok(())
    }

    fn finish(&mut self, context: &Context<'_>, out: &mut dyn Write) -> io::Result<()> {
        self.flush(context, out)?;
        let errors = count_of(self.errors, "error");
        let warnings = count_of(self.warnings, "warning");
        match (self.errors, self.warnings) {
            | (0, 0) => Ok(()),
            | (_, 0) => writeln!(out, "{errors} generated."),
            | (0, _) => writeln!(out, "{warnings} generated."),
            | _ => writeln!(out, "{errors} and {warnings} generated."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[expect(
        clippy::disallowed_types,
        reason = "Startup argv tests use owned OS strings as clap does."
    )]
    fn language_flags_apply_in_order_and_input_values_stay_opaque() {
        for (flags, seh) in [
            (["-fms-extensions", "-fno-ms-seh", "-fms-seh"], true),
            (["-fms-seh", "-fms-extensions", "-fno-ms-seh"], false),
        ] {
            let arguments = [
                "bcc-rust",
                "-std=c89",
                flags[0],
                flags[1],
                flags[2],
                "-pedantic-errors",
                "--input",
                "-std=c98",
            ];
            let cli = Cli::try_parse_from(normalize_language_arguments(
                arguments.map(std::ffi::OsString::from),
            ))
            .unwrap();
            let configuration = cli.configuration();
            assert_eq!(
                configuration.standard(),
                crate::configuration::CStandard::C89
            );
            assert_eq!(configuration.extension_policy(), ExtensionPolicy::Deny);
            assert_eq!(configuration.msvc_feature(MsvcFeature::Seh), seh);
            assert!(configuration.msvc_feature(MsvcFeature::Declspec));
            assert_eq!(cli.input.input.as_deref(), Some("-std=c98"));
        }
        let repeated =
            Cli::try_parse_from(["bcc-rust", "--std=c89", "--std=gnu23", "--input", "int x;"])
                .unwrap();
        assert_eq!(
            repeated.configuration().standard(),
            crate::configuration::CStandard::C23
        );
        assert!(repeated.configuration().gnu_extensions());
        let cli = Cli::try_parse_from(["bcc-rust", "--input", "int x;"]).unwrap();
        assert_eq!(
            cli.configuration().standard(),
            crate::configuration::CStandard::C17
        );
        assert!(cli.configuration().gnu_extensions());
    }

    struct CountDiagnostics(usize);

    impl Write for CountDiagnostics {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if buf.starts_with(b"error: #error bad") {
                self.0 += 1;
            }
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn token_reporter_high_water(repetitions: usize) -> usize {
        let tu = Bump::new();
        let mut source = ArenaString::new_in(&tu);
        for number in 0..repetitions {
            _ = write!(source, "#error bad\nint x{number};\n");
        }
        let source = source.into_str();
        let mut context = Context::new(&tu);
        let mut reporter_arena = Bump::new();
        let mut out = CountDiagnostics(0);
        print_preprocessor_output_in(
            &mut context,
            Path::new("<input>"),
            source,
            &[],
            &[],
            TokenOutput {
                out:            &mut out,
                reporter_arena: &mut reporter_arena,
                color:          RenderColor::Plain,
            },
        );
        assert_eq!(out.0, repetitions);
        reporter_arena.high_water()
    }

    #[test]
    fn token_reporter_arena_stays_bounded_across_diagnostic_batches() {
        let once = token_reporter_high_water(1_000);
        let twice = token_reporter_high_water(2_000);
        assert_eq!(once, twice);
    }
}
