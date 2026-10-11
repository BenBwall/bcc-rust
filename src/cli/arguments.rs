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
        ExtensionPolicy,
        LanguageMode,
        MsvcFeature,
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
            .with_source_date_epoch(self.source_date_epoch.and_then(|epoch| epoch.0))
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

/// A GCC language flag, which clap receives as `--language-option`.
#[derive(Clone, Copy)]
pub(super) enum LanguageFlag {
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
    pub(super) fn parse(text: &str) -> Option<Self> {
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

    pub(super) fn apply(self, configuration: CompilerConfiguration) -> CompilerConfiguration {
        match self {
            | Self::Pedantic(policy) => configuration.with_extension_policy(policy),
            | Self::MsvcExtensions(enabled) => configuration.with_msvc_extensions(enabled),
            | Self::MsvcFeature(feature, enabled) =>
                configuration.with_msvc_feature(feature, enabled),
            | Self::Hosted(hosted) => configuration.with_hosted(hosted),
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
    #[clap(short, long, conflicts_with = "input_file", allow_hyphen_values = true)]
    pub(super) input:      Option<String>,
    /// Input file to be parsed.
    #[clap(conflicts_with = "input")]
    pub(super) input_file: Option<PathBuf>,
}

/// The seconds since the Unix epoch that `__DATE__` and `__TIME__` spell,
/// or `None` for a malformed `SOURCE_DATE_EPOCH`, which is ignored.
#[derive(Debug, Clone, Copy)]
pub(super) struct SourceDateEpoch(Option<i64>);

/// Exact clang-style explanation, deliberately omitting deprecated aliases.
pub(super) const STANDARD_NOTES: &str =
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
pub(super) const SINGLE_DASH_SEARCH_FLAGS: [&str; 6] = [
    "iquote",
    "isystem",
    "idirafter",
    "nostdinc",
    "nostdlibinc",
    "nobuiltininc",
];

/// The seconds since the Unix epoch that a date can represent.
pub(super) const SOURCE_DATE_EPOCH_RANGE: RangeInclusive<i64> =
    DateTime::<Utc>::MIN_UTC.timestamp()..=DateTime::<Utc>::MAX_UTC.timestamp();

/// Parses a [`SourceDateEpoch`]: a decimal integer with an optional sign and
/// surrounding whitespace, within [`SOURCE_DATE_EPOCH_RANGE`]. The
/// `SOURCE_DATE_EPOCH` variable ignores any other value, as reproducible
/// builds expect and as the preprocessor did when it read the variable
/// itself; the `--source-date-epoch` flag rejects it.
#[derive(Debug, Clone, Copy)]
pub(super) struct SourceDateEpochParser;

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
