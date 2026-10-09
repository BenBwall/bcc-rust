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
use thiserror::Error;

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

mod diagnostic_reporter;

use diagnostic_reporter::DiagnosticReporter;

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

#[derive(Clone)]
struct TargetParser;

#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "Clap owns startup arguments outside compilation arenas."
)]
fn preprocessing_option(value: &str) -> Result<String, &'static str> {
    if matches!(value.as_bytes().first(), Some(b'D' | b'U' | b'I')) {
        Ok(value.to_owned())
    } else {
        Err("invalid startup preprocessing operation")
    }
}

impl TypedValueParser for TargetParser {
    type Value = crate::target::Target;

    fn parse_ref(
        &self,
        _cmd: &Command,
        _arg: Option<&Arg>,
        value: &OsStr,
    ) -> Result<Self::Value, clap::Error> {
        value
            .to_str()
            .and_then(crate::target::Target::parse)
            .ok_or_else(|| {
                clap::Error::raw(
                    clap::error::ErrorKind::InvalidValue,
                    "unsupported target triple; supported targets: x86_64-unknown-linux-gnu, \
                     x86_64-unknown-linux-musl, x86_64-w64-windows-gnu (aliases: \
                     x86_64-w64-mingw32, x86_64-pc-windows-gnu), x86_64-pc-windows-msvc",
                )
            })
    }
}
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
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
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
        if matches!(text, "-D" | "-U" | "-include") {
            let operation = match text {
                | "-D" => "D",
                | "-U" => "U",
                | _ => "I",
            };
            if let Some(value) = arguments.next() {
                let mut value_with_operation = std::ffi::OsString::from(operation);
                value_with_operation.push(value);
                normalized.push("--preprocessing-option".into());
                normalized.push(value_with_operation);
            } else {
                // Let clap diagnose the missing operand.
                normalized.push("--preprocessing-option".into());
            }
        } else if text.starts_with("-D") || text.starts_with("-U") {
            normalized.push(format!("--preprocessing-option={}", &text[1..]).into());
        } else if let Some(value) = text.strip_prefix("-std=") {
            normalized.push(format!("--std={value}").into());
        } else if text == "-std" {
            normalized.push("--std".into());
            opaque_value = true;
        } else if let Some(flag) = SINGLE_DASH_SEARCH_FLAGS
            .iter()
            .find(|flag| text.strip_prefix('-') == Some(**flag))
        {
            normalized.push(format!("--{flag}").into());
            opaque_value = !flag.starts_with("no");
        } else if let Some((flag, value)) = SINGLE_DASH_SEARCH_FLAGS[..3]
            .iter()
            .find_map(|flag| Some((flag, text.strip_prefix('-')?.strip_prefix(flag)?)))
        {
            normalized.push(format!("--{flag}={value}").into());
        } else if LanguageFlag::parse(text).is_some() {
            normalized.push(format!("--language-option={text}").into());
        } else {
            opaque_value = matches!(
                text,
                "--std"
                    | "--target"
                    | "--input"
                    | "-i"
                    | "--iquote"
                    | "-q"
                    | "-I"
                    | "--include-directory"
                    | "--isystem"
                    | "-s"
                    | "--idirafter"
                    | "--sysroot"
                    | "--source-date-epoch"
                    | "--language-option"
                    | "--preprocessing-option"
            );
            normalized.push(argument);
        }
    }
    normalized
}
/// GCC's single-dash header search options, which clap receives with two
/// dashes. The first three take a directory, separately or joined.
const SINGLE_DASH_SEARCH_FLAGS: [&str; 6] = [
    "iquote",
    "isystem",
    "idirafter",
    "nostdinc",
    "nostdlibinc",
    "nobuiltininc",
];

/// A GCC language flag, which clap receives as `--language-option`.
#[derive(Clone, Copy)]
enum LanguageFlag {
    /// `-pedantic` or `-Wpedantic` (warn), or `-pedantic-errors` (deny).
    Pedantic(ExtensionPolicy),
    /// `-fms-extensions` or `-fno-ms-extensions`.
    MsvcExtensions(bool),
    /// `-fms-<feature>` or `-fno-ms-<feature>`.
    MsvcFeature(MsvcFeature, bool),
    /// `-fhosted` (true) or `-ffreestanding` (false).
    Hosted(bool),
}

impl LanguageFlag {
    fn parse(text: &str) -> Option<Self> {
        match text {
            | "-pedantic" | "-Wpedantic" => return Some(Self::Pedantic(ExtensionPolicy::Warn)),
            | "-pedantic-errors" => return Some(Self::Pedantic(ExtensionPolicy::Deny)),
            | "-fhosted" => return Some(Self::Hosted(true)),
            | "-ffreestanding" => return Some(Self::Hosted(false)),
            | _ => {},
        }
        let (name, enabled) = text
            .strip_prefix("-fms-")
            .map(|name| (name, true))
            .or_else(|| text.strip_prefix("-fno-ms-").map(|name| (name, false)))?;
        if name == "extensions" {
            Some(Self::MsvcExtensions(enabled))
        } else {
            MsvcFeature::parse(name).map(|feature| Self::MsvcFeature(feature, enabled))
        }
    }

    fn apply(self, configuration: CompilerConfiguration) -> CompilerConfiguration {
        match self {
            | Self::Pedantic(policy) => configuration.with_extension_policy(policy),
            | Self::MsvcExtensions(enabled) => configuration.with_msvc_extensions(enabled),
            | Self::MsvcFeature(feature, enabled) =>
                configuration.with_msvc_feature(feature, enabled),
            | Self::Hosted(hosted) => configuration.with_hosted(hosted),
        }
    }
}

impl Cli {
    fn configure_preprocessing(&self, context: &mut Context<'_>) {
        let tu = context.tu_arena();
        context.preprocessing_options =
            tu.alloc_slice_fill_iter(self.preprocessing_option.iter().map(|option| {
                use crate::configuration::PreprocessingOption;
                let value = tu.alloc_str(&option[1..]);
                match option.as_bytes()[0] {
                    | b'D' => PreprocessingOption::Define(value),
                    | b'U' => PreprocessingOption::Undefine(value),
                    | _ => PreprocessingOption::Include(value),
                }
            }));
    }

    fn configuration(&self) -> CompilerConfiguration {
        let configuration =
            CompilerConfiguration::new(self.standard.standard, ExtensionPolicy::Allow)
                .with_gnu_extensions(self.standard.gnu);
        let configuration = configuration.with_target(self.target);
        self.language_option
            .iter()
            .filter_map(|option| LanguageFlag::parse(option))
            .fold(configuration, |configuration, flag| {
                flag.apply(configuration)
            })
            .with_source_date_epoch(self.source_date_epoch.and_then(|epoch| epoch.0))
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

/// Header search options, in GCC and Clang spellings. Lookup order:
/// `"…"` names look beside the including file and in `-iquote`; both forms
/// then search `-I`, `CPATH`, `-isystem`, `C_INCLUDE_PATH`, the built-in
/// resource directory, the C library's directories, and `-idirafter`.
#[derive(Args)]
#[expect(
    clippy::disallowed_types,
    reason = "clap's derived parser owns the repeated directories as `Vec<PathBuf>`."
)]
struct CliHeaderSearch {
    /// Add directory to the search path for `"…"` includes only.
    #[clap(short = 'q', long = "iquote", value_name = "DIR")]
    quote_include:  Vec<PathBuf>,
    /// Add directory to the include search path.
    #[clap(short = 'I', long = "include-directory", value_name = "DIR")]
    include:        Vec<PathBuf>,
    /// Add directory to the system include search path, before the built-in
    /// headers.
    #[clap(short = 's', long = "isystem", value_name = "DIR")]
    system_include: Vec<PathBuf>,
    /// Add directory to the end of the search path, after the built-in
    /// headers and the C library's directories.
    #[clap(long = "idirafter", value_name = "DIR")]
    after_include:  Vec<PathBuf>,
    /// Use `DIR/usr/local/include` and `DIR/usr/include` as the C library's
    /// include directories.
    #[clap(long, value_name = "DIR")]
    sysroot:        Option<PathBuf>,
    /// Do not search the built-in headers or the C library's directories.
    #[clap(long)]
    nostdinc:       bool,
    /// Do not search the C library's directories.
    #[clap(long)]
    nostdlibinc:    bool,
    /// Do not search the built-in headers.
    #[clap(long)]
    nobuiltininc:   bool,
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
#[expect(
    clippy::struct_excessive_bools,
    reason = "Clap owns independent inspection flags with explicit conflicts."
)]
struct ParserOutput {
    /// Print resolved declarations, types, linkage and storage duration.
    #[clap(long, conflicts_with_all = ["tokens", "syntax_tree", "raw_syntax"])]
    semantic_types:   bool,
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

/// The directories of each header search group, owned beside clap's
/// arguments until `run` copies them into the translation-unit arena.
#[expect(
    clippy::disallowed_types,
    reason = "Startup owns the directories from argv and the environment as `Vec<PathBuf>`."
)]
struct SearchDirectories {
    quote:    Vec<PathBuf>,
    angled:   Vec<PathBuf>,
    system:   Vec<PathBuf>,
    resource: bool,
    after:    Vec<PathBuf>,
}

impl CliHeaderSearch {
    /// Groups the directories in search order. GCC searches `CPATH` like
    /// trailing `-I` directories and `C_INCLUDE_PATH` like trailing
    /// `-isystem` ones; `-isystem`, `C_INCLUDE_PATH`, the resource
    /// directory, the library's directories and `-idirafter` are system
    /// directories. C99: implementation-defined places, §6.10.2p2-3,
    /// pp. 149-150; PDF pp. 161-162.
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "Startup owns the directories from argv and the environment as `Vec<PathBuf>`."
    )]
    fn directories(&self) -> SearchDirectories {
        let mut angled = self.include.clone();
        angled.extend(include_path_from_env("CPATH"));
        let mut system = self.system_include.clone();
        system.extend(include_path_from_env("C_INCLUDE_PATH"));
        let mut after = Vec::new();
        if let Some(sysroot) = &self.sysroot
            && !self.nostdinc
            && !self.nostdlibinc
        {
            after.push(sysroot.join("usr").join("local").join("include"));
            after.push(sysroot.join("usr").join("include"));
        }
        after.extend(self.after_include.iter().cloned());
        SearchDirectories {
            quote: self.quote_include.clone(),
            angled,
            system,
            resource: !self.nostdinc && !self.nobuiltininc,
            after,
        }
    }
}

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

/// Prints the preprocessed translation unit, each token after the
/// diagnostics its production reported.
fn print_preprocessor_output<'tu>(
    context: &mut Context<'tu>,
    source_filename: &Path,
    input_string: &'tu str,
    search: HeaderSearch<'_>,
) {
    let mut reporter_arena = Bump::new();
    let mut stderr = io::stderr();
    print_preprocessor_output_in(
        context,
        source_filename,
        input_string,
        search,
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
    search: HeaderSearch<'_>,
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
        search,
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
        | TokenType::String(StringTokenType::EncodedString(contents, encoding)) =>
            context.literal_spelling_in(scratch, scratch, contents, encoding.prefix()),
        | TokenType::String(StringTokenType::String(contents)) =>
            context.literal_spelling_in(scratch, scratch, contents, ""),
        | TokenType::String(StringTokenType::WideString(contents)) =>
            context.literal_spelling_in(scratch, scratch, contents, "L"),
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
        | TokenType::String(StringTokenType::EncodedString(_, encoding)) =>
            write!(line, "{} string literal {literal}", encoding.type_name()),
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
    search: HeaderSearch<'_>,
    output: &ParserOutput,
    repeated_specifier_warnings: bool,
) {
    context.configuration = context
        .configuration
        .with_repeated_specifier_warnings(repeated_specifier_warnings);
    let unit = parse_translation_unit(context, source_filename, input_string, search);
    let semantic = (output.semantic_types || (!output.syntax_tree && !output.raw_syntax))
        .then(|| crate::pipeline::analyze_translation_unit(context, &unit));
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
    if let Some(semantic) = semantic
        && output.semantic_types
    {
        let inspection = Bump::new();
        eprint!("{}", semantic.inspect(context, &inspection));
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
    compile_file_configured_measured(path, CompilerConfiguration::default(), out, measure, |_| {})
}

/// Measures compilation with CLI language and startup preprocessing arguments.
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
    compile_file_configured_measured(path, args.configuration(), out, measure, |context| {
        args.configure_preprocessing(context);
    })
}

fn compile_file_configured_measured(
    path: &Path,
    configuration: CompilerConfiguration,
    out: &mut dyn Write,
    mut measure: impl FnMut(CompileStep, &mut dyn FnMut()),
    configure: impl FnOnce(&mut Context<'_>),
) -> io::Result<()> {
    let tu = Bump::new();
    let source = tu.read_to_str_lossy(path)?;
    let mut context = Context::with_configuration(&tu, configuration);
    configure(&mut context);
    measure(CompileStep::Parse, &mut || {
        let unit = parse_translation_unit(&mut context, path, source, HeaderSearch::default());
        let _semantic = crate::pipeline::analyze_translation_unit(&mut context, &unit);
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
            HeaderSearch::default(),
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
