//! Normalizes GCC-style options before clap parses them, then builds the
//! compiler configuration and startup preprocessing operations.

#[expect(
    clippy::disallowed_types,
    reason = "clap parses path arguments into `PathBuf`s; the compiler borrows them as `&Path`."
)]
use std::path::PathBuf;
use std::{
    ffi::OsStr,
    ops::RangeInclusive,
};

use chrono::{
    DateTime,
    Utc,
};
use clap::{
    Arg,
    Args,
    Command,
    builder::{
        RangedI64ValueParser,
        TypedValueParser,
    },
    parser::ValueSource,
};

use super::Cli;
use crate::{
    configuration::{
        CompilerConfiguration,
        CompilerIdentity,
        DiagnosticFormat,
        DiagnosticGroup,
        ExtensionPolicy,
        LanguageMode,
        MsvcFeature,
        WarningOption,
    },
    target::{
        CodeOptions,
        Relocation,
        Target,
        TlsModel,
    },
    translation_phases::Context,
};

/// Normalize GCC's single-dash long options before clap. Values and tokens
/// after `--` are opaque, including an input string that looks like a flag.
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    reason = "Startup argv normalization owns OS strings beside clap; never a compilation buffer."
)]
pub(super) fn normalize_language_arguments(
    arguments: impl IntoIterator<Item = std::ffi::OsString>,
) -> Result<Vec<std::ffi::OsString>, clap::Error> {
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
        } else if let Some(option) = dependency_option(text, &mut arguments) {
            normalized.push(option);
        } else if let Some(format) = text.strip_prefix("-fdiagnostics-format=") {
            normalized.push(format!("--diagnostic-format={format}").into());
        } else if text == "-fsyntax-only" {
            normalized.push("--syntax-only".into());
        } else if text == "-dM" {
            normalized.push("--dump-macros".into());
        } else if text == "-x" {
            normalized.push("--language".into());
            opaque_value = true;
        } else if let Some(language) = text.strip_prefix("-x") {
            normalized.push(format!("--language={language}").into());
        } else if text.starts_with("-Wa,") {
            normalized.push(format!("--assembler-option={text}").into());
        } else if text == "-Xassembler" {
            normalized.push("--assembler-option=-Xassembler".into());
            if let Some(operand) = arguments.next() {
                let mut option = std::ffi::OsString::from("--assembler-option=");
                option.push(operand);
                normalized.push(option);
            }
        } else if matches!(text, "-l" | "-L" | "-Xlinker") {
            // The operand is the next argument, which keeps its place
            // among the inputs.
            let operand = arguments.next();
            if text == "-Xlinker" {
                normalized.push("--link-option=-Xlinker".into());
                if let Some(operand) = operand {
                    let mut option = std::ffi::OsString::from("--link-option=");
                    option.push(operand);
                    normalized.push(option);
                }
            } else {
                let mut option = std::ffi::OsString::from(format!("--link-option={text}"));
                if let Some(operand) = operand {
                    option.push(operand);
                }
                normalized.push(option);
            }
        } else if text == "-pthread" {
            // GCC and Clang link the thread library and define `_REENTRANT`.
            normalized.push("--link-option=-pthread".into());
            normalized.push("--preprocessing-option=D_REENTRANT".into());
        } else if text.starts_with("-l")
            || text.starts_with("-L")
            || text.starts_with("-Wl,")
            || SINGLE_DASH_LINKER_OPTIONS.contains(&text)
        {
            normalized.push(format!("--link-option={text}").into());
        } else if text == "-O" {
            normalized.push("--optimization=O1".into());
        } else if let Some(value) = text.strip_prefix("-O") {
            normalized.push(format!("--optimization=O{}", optimization_level(value)?).into());
        } else if let Some(value) = text.strip_prefix("-ftls-model=") {
            normalized.push(format!("--tls-model={value}").into());
        } else if let Some(flag) = SINGLE_DASH_LINK_FLAGS
            .iter()
            .find(|flag| text.strip_prefix('-') == Some(**flag))
        {
            normalized.push(format!("--{flag}").into());
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
                    | "--max-nesting-depth"
                    | "--tls-model"
                    | "--opt"
                    | "--llvm-opt"
                    | "--optimization"
                    | "--passes"
                    | "--opt-bisect-limit"
                    | "--interpret-step-limit"
                    | "--language-option"
                    | "--preprocessing-option"
                    | "--language"
                    | "--link-option"
                    | "--assembler-option"
                    | "--dependency-file"
                    | "--dependency-target"
                    | "--compiler-identity"
                    | "--diagnostic-format"
                    | "-o"
                    | "--output"
            );
            normalized.push(argument);
        }
    }
    Ok(normalized)
}

/// GCC's dependency options as the driver's: `-M`, `-MM`, `-MD`, `-MMD`,
/// `-MP`, and `-MF`, `-MT` and `-MQ` with their operand joined or next.
/// `-MQ` quotes its target for make as GCC does.
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    reason = "Startup argv normalization owns OS strings beside clap."
)]
fn dependency_option(
    text: &str,
    arguments: &mut impl Iterator<Item = std::ffi::OsString>,
) -> Option<std::ffi::OsString> {
    let fixed = match text {
        | "-M" => Some("--dependencies=all"),
        | "-MM" => Some("--dependencies=user"),
        | "-MD" => Some("--write-dependencies=all"),
        | "-MMD" => Some("--write-dependencies=user"),
        | "-MP" => Some("--phony-dependencies"),
        | _ => None,
    };
    if let Some(fixed) = fixed {
        return Some(fixed.into());
    }
    let (flag, joined) = ["-MF", "-MT", "-MQ"]
        .into_iter()
        .find_map(|flag| Some((flag, text.strip_prefix(flag)?)))?;
    let operand = if joined.is_empty() {
        arguments.next()?.to_string_lossy().into_owned()
    } else {
        joined.into()
    };
    Some(
        match flag {
            | "-MF" => format!("--dependency-file={operand}"),
            | "-MT" => format!("--dependency-target={operand}"),
            | _ => format!("--dependency-target={}", quote_for_make(&operand)),
        }
        .into(),
    )
}

/// `target` with the characters make treats specially escaped, as GCC's
/// `-MQ` writes it: `$` doubled, `#` and blanks backslashed, and the
/// backslashes before a blank doubled.
#[expect(
    clippy::disallowed_types,
    reason = "Startup argv normalization owns its strings beside clap."
)]
fn quote_for_make(target: &str) -> String {
    let mut quoted = String::with_capacity(target.len());
    let mut backslashes = 0;
    for character in target.chars() {
        match character {
            | '\\' => backslashes += 1,
            | ' ' | '\t' => {
                for _ in 0..=backslashes {
                    quoted.push('\\');
                }
                backslashes = 0;
            },
            | '$' => {
                quoted.push('$');
                backslashes = 0;
            },
            | '#' => {
                quoted.push('\\');
                backslashes = 0;
            },
            | _ => backslashes = 0,
        }
        quoted.push(character);
    }
    quoted
}

/// Clang's numeric aliases are driver policy, before clap sees the internal
/// optimization option. Invalid values retain the spelling the user supplied.
#[expect(
    clippy::disallowed_macros,
    reason = "Startup option diagnostics format owned clap errors and warnings."
)]
fn optimization_level(value: &str) -> Result<&str, clap::Error> {
    if matches!(value, "s" | "z" | "g") {
        return Ok(value);
    }
    let digits = value.strip_prefix('-').unwrap_or(value);
    let level = (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse::<i32>().ok())
        .flatten()
        .ok_or_else(|| {
            clap::Error::raw(
                clap::error::ErrorKind::InvalidValue,
                format!("invalid integral value '{value}' in '-O{value}'"),
            )
        })?;
    Ok(match level {
        | 0 => "0",
        | 1 => "1",
        | 2 => "2",
        | 3 => "3",
        | 4 if value == "4" => {
            eprintln!("warning: -O4 is equivalent to -O3 [-Wdeprecated]");
            "3"
        },
        | _ => {
            eprintln!(
                "warning: optimization level '-O{value}' is not supported; using '-O3' instead"
            );
            "3"
        },
    })
}

impl Cli {
    pub(super) fn configuration(&self) -> CompilerConfiguration {
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
            .with_code_options(self.code_options())
            .with_optimization(self.code_generation.optimization_macros())
            .with_source_date_epoch(self.source_date_epoch.and_then(|epoch| epoch.0))
            .with_suppress_warnings(self.suppress_warnings)
            .with_compiler_identity(self.compiler_identity.into())
            .with_diagnostic_format(self.diagnostic_format.into())
            .with_max_nesting_depth(self.max_nesting_depth)
    }

    /// Clang diagnoses unsupported warning options at startup and continues.
    pub(super) fn report_unknown_warning_options(&self) -> bool {
        let Some(severity) = self
            .configuration()
            .diagnostic_severity(DiagnosticGroup::UnknownWarningOption)
        else {
            return false;
        };
        let mut failed = false;
        for option in &self.language_option {
            if matches!(
                LanguageFlag::parse(option),
                Some(LanguageFlag::UnknownWarning)
            ) {
                let label = if severity == crate::translation_phases::ErrorSeverity::Error {
                    failed = true;
                    "error"
                } else {
                    "warning"
                };
                eprintln!("{label}: unknown warning option '{option}' [-Wunknown-warning-option]");
            }
        }
        failed
    }

    /// The relocation model of the last `-fpic`-family flag, and the TLS
    /// model.
    fn code_options(&self) -> CodeOptions {
        CodeOptions {
            relocation: self
                .language_option
                .iter()
                .filter_map(|option| Relocation::parse(option))
                .next_back(),
            tls_model:  self.tls_model.unwrap_or_default(),
        }
    }

    /// The last `-fpic`-family flag if it asks for position independence
    /// on the MSVC target, which Clang rejects as unsupported there.
    #[expect(
        clippy::disallowed_types,
        reason = "Clap owns startup arguments as `String`s; the flag is borrowed from them."
    )]
    pub(super) fn unsupported_relocation_flag(&self) -> Option<&str> {
        if self.target != Target::WindowsMsvc {
            return None;
        }
        self.language_option
            .iter()
            .rfind(|option| Relocation::parse(option).is_some())
            .filter(|option| Relocation::parse(option) != Some(Relocation::Static))
            .map(String::as_str)
    }

    pub(super) fn configure_preprocessing(&self, context: &mut Context<'_>) {
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
}

/// Whether bcc implements the GCC-style `-W`, `-f`, `-pedantic` or
/// `-Werror` option `text` itself, rather than only accepting it for a
/// driver personality.
pub(super) fn implements_language_option(text: &str) -> bool {
    text.starts_with("-ftls-model=")
        || text.starts_with("-fdiagnostics-format=")
        || LanguageFlag::parse(text)
            .is_some_and(|flag| !matches!(flag, LanguageFlag::UnknownWarning))
}

/// `--compiler-identity`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub(super) enum Identity {
    Clang,
    Gcc,
    Msvc,
    ClangCl,
    Tcc,
}

impl From<Identity> for CompilerIdentity {
    fn from(identity: Identity) -> Self {
        match identity {
            | Identity::Clang => Self::Clang,
            | Identity::Gcc => Self::Gcc,
            | Identity::Msvc => Self::Msvc,
            | Identity::ClangCl => Self::ClangCl,
            | Identity::Tcc => Self::Tcc,
        }
    }
}

/// `--diagnostic-format`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub(super) enum Format {
    Bcc,
    Clang,
    Gcc,
    /// `file(line,col):`, as clang-cl writes and `cl /diagnostics:column`.
    Msvc,
    /// `file(line):`, `cl /diagnostics:classic`.
    MsvcClassic,
    /// `file(line,col):` and the snippet, `cl /diagnostics:caret`.
    MsvcCaret,
    /// `file:line:`, without a snippet.
    Tcc,
}

impl From<Format> for DiagnosticFormat {
    fn from(format: Format) -> Self {
        match format {
            | Format::Bcc => Self::Bcc,
            | Format::Clang => Self::Clang,
            | Format::Gcc => Self::Gcc,
            | Format::Msvc => Self::Msvc {
                column: true,
                caret:  false,
            },
            | Format::MsvcClassic => Self::Msvc {
                column: false,
                caret:  false,
            },
            | Format::MsvcCaret => Self::Msvc {
                column: true,
                caret:  true,
            },
            | Format::Tcc => Self::Tcc,
        }
    }
}

/// A GCC language flag, which clap receives as `--language-option`.
#[derive(Clone, Copy)]
enum LanguageFlag {
    WarningsAsErrors(bool),
    WarningGroup(DiagnosticGroup, WarningOption),
    AllWarnings(bool),
    ConversionWarnings(bool),
    ExtraWarnings,
    UnknownWarning,
    /// `-pedantic` or `-Wpedantic` (warn), or `-pedantic-errors` (deny).
    Pedantic(ExtensionPolicy),
    /// `-fms-extensions` or `-fno-ms-extensions`.
    MsvcExtensions(bool),
    /// `-fms-<feature>` or `-fno-ms-<feature>`.
    MsvcFeature(MsvcFeature, bool),
    /// `-fhosted` (true) or `-ffreestanding` (false).
    Hosted(bool),
    /// `-fgnu89-inline` or `-fno-gnu89-inline`.
    Gnu89Inline(bool),
    /// `-trigraphs` or `-ftrigraphs` (true), or `-fno-trigraphs`.
    Trigraphs(bool),
    /// `-fwrapv` (true), or `-fno-wrapv` and `-fstrict-overflow`.
    Wrapv(bool),
    /// `-fpack-struct[=N]`, or `-fno-pack-struct` (0).
    PackStruct(u32),
    /// A `-fpic`-family flag, which [`Cli::code_options`] reads in order.
    Relocation,
}

impl LanguageFlag {
    fn parse(text: &str) -> Option<Self> {
        match text {
            | "-pedantic" | "-Wpedantic" => return Some(Self::Pedantic(ExtensionPolicy::Warn)),
            | "-pedantic-errors" => return Some(Self::Pedantic(ExtensionPolicy::Deny)),
            | "-Werror" => return Some(Self::WarningsAsErrors(true)),
            | "-Wno-error" => return Some(Self::WarningsAsErrors(false)),
            | "-Wall" => return Some(Self::AllWarnings(true)),
            | "-Wno-all" => return Some(Self::AllWarnings(false)),
            | "-Wextra" | "-Wno-extra" => return Some(Self::ExtraWarnings),
            | "-Wconversion" => return Some(Self::ConversionWarnings(true)),
            | "-Wno-conversion" => return Some(Self::ConversionWarnings(false)),
            | "-Wno-pedantic" => return Some(Self::Pedantic(ExtensionPolicy::Allow)),
            | "-fhosted" => return Some(Self::Hosted(true)),
            | "-ffreestanding" => return Some(Self::Hosted(false)),
            | "-fgnu89-inline" => return Some(Self::Gnu89Inline(true)),
            | "-fno-gnu89-inline" => return Some(Self::Gnu89Inline(false)),
            | "-trigraphs" | "-ftrigraphs" => return Some(Self::Trigraphs(true)),
            | "-fno-trigraphs" => return Some(Self::Trigraphs(false)),
            | "-fwrapv" => return Some(Self::Wrapv(true)),
            | "-fno-wrapv" | "-fstrict-overflow" => return Some(Self::Wrapv(false)),
            | "-fpack-struct" => return Some(Self::PackStruct(1)),
            | "-fno-pack-struct" => return Some(Self::PackStruct(0)),
            | _ => {},
        }
        if let Some(value) = text.strip_prefix("-fpack-struct=") {
            return match value {
                | "1" | "2" | "4" | "8" | "16" => value.parse().ok().map(Self::PackStruct),
                | _ => None,
            };
        }
        match text {
            | _ => {},
        }
        if Relocation::parse(text).is_some() {
            return Some(Self::Relocation);
        }
        if let Some(text) = text.strip_prefix("-W") {
            let (name, option) = if let Some(name) = text.strip_prefix("error=") {
                (name, WarningOption::Error(true))
            } else if let Some(name) = text.strip_prefix("no-error=") {
                (name, WarningOption::Error(false))
            } else if let Some(name) = text.strip_prefix("no-") {
                (name, WarningOption::Enable(false))
            } else {
                (text, WarningOption::Enable(true))
            };
            return Some(
                DiagnosticGroup::parse(name).map_or(Self::UnknownWarning, |group| {
                    Self::WarningGroup(group, option)
                }),
            );
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
            | Self::UnknownWarning | Self::ExtraWarnings | Self::Relocation => configuration,
            | Self::AllWarnings(enabled) => configuration.with_warning_option(
                DiagnosticGroup::UnusedFunction,
                WarningOption::Enable(enabled),
            ),
            | Self::ConversionWarnings(enabled) => configuration
                .with_warning_option(
                    DiagnosticGroup::NullConversion,
                    WarningOption::Enable(enabled),
                )
                .with_warning_option(
                    DiagnosticGroup::NonLiteralNullConversion,
                    WarningOption::Enable(enabled),
                )
                .with_warning_option(
                    DiagnosticGroup::PointerBoolConversion,
                    WarningOption::Enable(enabled),
                ),
            | Self::WarningsAsErrors(enabled) => configuration.with_warnings_as_errors(enabled),
            | Self::WarningGroup(group, option) => configuration.with_warning_option(group, option),
            | Self::Pedantic(policy) => configuration.with_extension_policy(policy),
            | Self::MsvcExtensions(enabled) => configuration.with_msvc_extensions(enabled),
            | Self::MsvcFeature(feature, enabled) =>
                configuration.with_msvc_feature(feature, enabled),
            | Self::Hosted(hosted) => configuration.with_hosted(hosted),
            | Self::Gnu89Inline(enabled) => configuration.with_gnu89_inline(enabled),
            | Self::Trigraphs(enabled) => configuration.with_trigraphs(enabled),
            | Self::Wrapv(enabled) => configuration.with_wrapv(enabled),
            | Self::PackStruct(alignment) => configuration.with_default_pack(alignment),
        }
    }
}

#[derive(Args)]
#[group(required = true, multiple = false)]
#[expect(
    clippy::disallowed_types,
    reason = "clap's derived parser owns the input string and the input file path."
)]
pub(super) struct CliInput {
    /// Input string to be parsed.
    #[clap(
        short,
        long,
        conflicts_with = "input_files",
        allow_hyphen_values = true
    )]
    pub(super) input:       Option<String>,
    /// Input files: C sources, `-` for stdin, assembly, and the objects,
    /// archives and libraries to link.
    #[clap(conflicts_with = "input")]
    pub(super) input_files: Vec<PathBuf>,
}

/// The seconds since the Unix epoch that `__DATE__` and `__TIME__` spell,
/// or `None` for a malformed `SOURCE_DATE_EPOCH`, which is ignored.
#[derive(Debug, Clone, Copy)]
pub(super) struct SourceDateEpoch(Option<i64>);

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
pub(super) struct StandardParser;

#[derive(Clone)]
pub(super) struct TargetParser;

/// Parses `-ftls-model=MODEL`, which clap receives as `--tls-model`.
#[derive(Clone)]
pub(super) struct TlsModelParser;

#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "Clap owns startup arguments outside compilation arenas."
)]
pub(super) fn preprocessing_option(value: &str) -> Result<String, &'static str> {
    if matches!(value.as_bytes().first(), Some(b'D' | b'U' | b'I')) {
        Ok(value.to_owned())
    } else {
        Err("invalid startup preprocessing operation")
    }
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

/// GCC's linker options without an operand, which the driver passes to the
/// link in order with the inputs.
const SINGLE_DASH_LINKER_OPTIONS: [&str; 8] = [
    "-static",
    "-nostdlib",
    "-nostartfiles",
    "-nodefaultlibs",
    "-rdynamic",
    "-pthread",
    "-static-libgcc",
    "-shared-libgcc",
];

/// GCC's single-dash link options, which clap receives with two dashes:
/// `-shared` would otherwise read as `-s hared`.
const SINGLE_DASH_LINK_FLAGS: [&str; 3] = ["shared", "pie", "no-pie"];

/// The seconds since the Unix epoch that a date can represent.
const SOURCE_DATE_EPOCH_RANGE: RangeInclusive<i64> =
    DateTime::<Utc>::MIN_UTC.timestamp()..=DateTime::<Utc>::MAX_UTC.timestamp();

/// Parses a [`SourceDateEpoch`]: a decimal integer with an optional sign and
/// surrounding whitespace, within [`SOURCE_DATE_EPOCH_RANGE`]. The
/// `SOURCE_DATE_EPOCH` variable ignores any other value, as reproducible
/// builds expect and as the preprocessor did when it read the variable
/// itself; the `--source-date-epoch` flag rejects it.
#[derive(Debug, Clone, Copy)]
pub(super) struct SourceDateEpochParser;

impl TypedValueParser for TargetParser {
    type Value = Target;

    fn parse_ref(
        &self,
        _cmd: &Command,
        _arg: Option<&Arg>,
        value: &OsStr,
    ) -> Result<Self::Value, clap::Error> {
        value.to_str().and_then(Target::parse).ok_or_else(|| {
            clap::Error::raw(
                clap::error::ErrorKind::InvalidValue,
                "unsupported target triple; supported targets: x86_64-unknown-linux-gnu, \
                 x86_64-unknown-linux-musl, x86_64-w64-windows-gnu (aliases: x86_64-w64-mingw32, \
                 x86_64-pc-windows-gnu), x86_64-pc-windows-msvc",
            )
        })
    }
}

impl TypedValueParser for TlsModelParser {
    type Value = TlsModel;

    #[expect(
        clippy::disallowed_macros,
        reason = "Clap argument errors own their startup message."
    )]
    fn parse_ref(
        &self,
        _cmd: &Command,
        _arg: Option<&Arg>,
        value: &OsStr,
    ) -> Result<TlsModel, clap::Error> {
        value.to_str().and_then(TlsModel::parse).ok_or_else(|| {
            // Clang's wording, with the accepted models.
            clap::Error::raw(
                clap::error::ErrorKind::InvalidValue,
                format!(
                    "invalid value '{}' in '-ftls-model={}'\nnote: use 'global-dynamic', \
                     'local-dynamic', 'initial-exec', or 'local-exec'\n",
                    value.to_string_lossy(),
                    value.to_string_lossy()
                ),
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
