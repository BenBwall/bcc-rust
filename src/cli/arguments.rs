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
        DiagnosticColor,
        DiagnosticFormat,
        DiagnosticGroup,
        DiagnosticPresentation,
        ExtensionPolicy,
        LanguageMode,
        MsvcFeature,
        RelaxedFloatingPoint,
        RuleSet,
        WarningOption,
    },
    target::{
        CodeOptions,
        Hardening,
        Relocation,
        StackProtector,
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
    let mut debug = DebugFlags::default();
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
        } else if debug.read(text, &mut normalized) {
            // Read in order: `-g0` undoes an earlier `-g`.
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
/// `-MP`, `-MG`, and `-MF`, `-MT` and `-MQ` with their operand joined or
/// next.
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
        | "-MG" => Some("--generated-dependencies"),
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
        // Warning options read the identity: Clang's `-Wno-X` forgets an
        // earlier `-Werror=X`, GCC's keeps it.
        let configuration = configuration
            .with_target(self.target)
            .with_compiler_identity(self.compiler_identity.into());
        // Clang reads `-pedantic` and `-pedantic-errors` before every `-W`
        // option, so `-Wno-pedantic` and `-Wno-error=pedantic` outrank them
        // wherever they appear; GCC reads all of them in order.
        let clang = configuration.compiler_identity().rules() == RuleSet::Clang;
        let first = |option: &str| clang && matches!(option, "-pedantic" | "-pedantic-errors");
        let options = self.language_option.iter().map(std::ops::Deref::deref);
        options
            .clone()
            .filter(|option| first(option))
            .chain(options.filter(|option| !first(option)))
            .filter_map(LanguageFlag::parse)
            .fold(configuration, |configuration, flag| {
                flag.apply(configuration)
            })
            .with_code_options(self.code_options())
            .with_optimization(self.code_generation.optimization_macros())
            .with_debug_info(self.code_generation.debug_options(self.target))
            .with_stack_locals(
                (self.code_generation.stack_locals || self.driver.driver_name.is_some())
                    && self.code_generation.stack_locals_apply(self.target),
            )
            .with_source_date_epoch(self.source_date_epoch.and_then(|epoch| epoch.0))
            .with_suppress_warnings(self.suppress_warnings)
            .with_syntax_only(self.driver.syntax_only)
            .with_compiler_identity(self.compiler_identity.into())
            .with_compiler_extensions(self.compiler_extensions)
            .with_default_math_errno()
            .with_diagnostic_format(self.diagnostic_format.into())
            .with_diagnostic_presentation(self.diagnostic_presentation())
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
            let unknown = match LanguageFlag::parse(option) {
                | Some(LanguageFlag::UnknownWarning) => true,
                | Some(LanguageFlag::WarningGroup(group, _)) =>
                    group.gcc_only()
                        && CompilerIdentity::from(self.compiler_identity) != CompilerIdentity::Gcc,
                | _ => false,
            };
            if unknown {
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

    /// The relocation model of the last `-fpic`-family flag, the TLS model,
    /// and the last of `-f[no-]strict-volatile-bitfields`,
    /// `-f[no-]function-sections` and `-f[no-]data-sections`.
    fn code_options(&self) -> CodeOptions {
        // WebAssembly has no such features: `report_x86_options_on_wasm`
        // refuses them.
        let x86_features = if self.target.is_wasm() {
            crate::target::X86Features::NONE
        } else {
            crate::target::X86Features::from_options(
                &self.language_option.iter().map(std::ops::Deref::deref),
                CompilerIdentity::from(self.compiler_identity) != CompilerIdentity::Gcc,
            )
        };
        CodeOptions {
            x86_features,
            hardening: self.hardening(),
            relocation: self
                .language_option
                .iter()
                .filter_map(|option| Relocation::parse(option))
                .next_back(),
            tls_model: self.tls_model.unwrap_or_default(),
            debug_info: self.code_generation.debug_options(self.target).is_some(),
            strict_volatile_bit_fields: self.last_of(
                "-fstrict-volatile-bitfields",
                "-fno-strict-volatile-bitfields",
            ),
            intel_syntax: self.last_of("-masm=intel", "-masm=att"),
            function_sections: self.last_of("-ffunction-sections", "-fno-function-sections"),
            data_sections: self.last_of("-fdata-sections", "-fno-data-sections"),
        }
    }

    /// The run-time checks the options ask for, each from its last option.
    /// Only Linux has them: the personalities warn about them on Windows.
    fn hardening(&self) -> Hardening {
        let mut hardening = Hardening::default();
        if !matches!(self.target, Target::LinuxGnu | Target::LinuxMusl) {
            return hardening;
        }
        let gcc = self.compiler_identity == Identity::Gcc;
        for option in &self.language_option {
            match option.as_str() {
                | "-fstack-protector" => hardening.stack_protector = StackProtector::Basic,
                | "-fstack-protector-strong" => hardening.stack_protector = StackProtector::Strong,
                | "-fstack-protector-all" => hardening.stack_protector = StackProtector::All,
                | "-fno-stack-protector" => hardening.stack_protector = StackProtector::Off,
                | "-fcf-protection" | "-fcf-protection=full" => {
                    hardening.branch_protection = true;
                    hardening.return_protection = true;
                },
                // GCC adds to what earlier options asked for, and Clang
                // replaces it.
                | "-fcf-protection=branch" => {
                    hardening.branch_protection = true;
                    hardening.return_protection &= gcc;
                },
                | "-fcf-protection=return" => {
                    hardening.branch_protection &= gcc;
                    hardening.return_protection = true;
                },
                | "-fcf-protection=none" => {
                    hardening.branch_protection = false;
                    hardening.return_protection = false;
                },
                | "-fstack-clash-protection" => hardening.stack_clash = true,
                | "-fno-stack-clash-protection" => hardening.stack_clash = false,
                | "-fsanitize=address" => hardening.sanitize_address = true,
                | "-fsanitize=thread" => hardening.sanitize_thread = true,
                | "-fno-sanitize=address" => hardening.sanitize_address = false,
                | "-fno-sanitize=thread" => hardening.sanitize_thread = false,
                | "-fno-sanitize=all" => {
                    hardening.sanitize_address = false;
                    hardening.sanitize_thread = false;
                },
                | "-pg" => hardening.profile = true,
                | "-fno-omit-frame-pointer" => hardening.keep_frame_pointer = true,
                | "-fomit-frame-pointer" => hardening.keep_frame_pointer = false,
                | _ => {},
            }
        }
        hardening
    }

    /// What GCC's and Clang's diagnostic formats write, from each option in
    /// order. `-fdiagnostics-plain-output` stands for the options it means, so
    /// a later one can take any of them back. GCC manual, "Diagnostic Message
    /// Formatting Options".
    fn diagnostic_presentation(&self) -> DiagnosticPresentation {
        let mut presentation = DiagnosticPresentation::default();
        for option in &self.language_option {
            match option.as_str() {
                | "-fdiagnostics-show-caret" | "-fcaret-diagnostics" => presentation.caret = true,
                | "-fno-diagnostics-show-caret" | "-fno-caret-diagnostics" =>
                    presentation.caret = false,
                | "-fshow-column" => presentation.column = true,
                | "-fno-show-column" => presentation.column = false,
                | "-fdiagnostics-show-line-numbers" => presentation.line_numbers = true,
                | "-fno-diagnostics-show-line-numbers" => presentation.line_numbers = false,
                | "-fdiagnostics-show-option" => presentation.option_names = true,
                | "-fno-diagnostics-show-option" => presentation.option_names = false,
                | "-fdiagnostics-color" | "-fdiagnostics-color=always" | "-fcolor-diagnostics" =>
                    presentation.color = DiagnosticColor::Always,
                | "-fno-diagnostics-color"
                | "-fdiagnostics-color=never"
                | "-fno-color-diagnostics" => presentation.color = DiagnosticColor::Never,
                | "-fdiagnostics-color=auto" => presentation.color = DiagnosticColor::Auto,
                | "-fdiagnostics-plain-output" => {
                    presentation.caret = false;
                    presentation.line_numbers = false;
                    presentation.color = DiagnosticColor::Never;
                },
                | _ => {},
            }
        }
        presentation
    }

    /// Whether `on` came after the last `off`, and at all.
    fn last_of(&self, on: &str, off: &str) -> bool {
        self.language_option
            .iter()
            .rfind(|option| *option == on || *option == off)
            .is_some_and(|option| option == on)
    }

    /// What code generation may assume about pointers and tentative
    /// definitions, each from the last flag that sets it, as GCC reads
    /// them.
    pub(super) fn code_assumptions(&self) -> crate::target::CodeAssumptions {
        let mut assumptions = crate::target::CodeAssumptions::default();
        for option in &self.language_option {
            match option.as_str() {
                | "-fwrapv-pointer" | "-fno-strict-overflow" => assumptions.wrapv_pointer = true,
                | "-fno-wrapv-pointer" | "-fstrict-overflow" => assumptions.wrapv_pointer = false,
                | "-fno-delete-null-pointer-checks" => assumptions.null_pointer_is_valid = true,
                | "-fdelete-null-pointer-checks" => assumptions.null_pointer_is_valid = false,
                | "-fcommon" => assumptions.common_symbols = true,
                | "-fno-common" => assumptions.common_symbols = false,
                | _ => {},
            }
        }
        assumptions
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

    /// Reports the x86 feature options a WebAssembly target has no use for,
    /// which leave no macros behind ([`Self::code_options`]). Clang refuses
    /// each in its own words, but for `-msse4.2` and `-msse4`, which it leaves
    /// unused, and reads `-mno-sse4` as `-mno-sse4.1`. GCC has no WebAssembly
    /// target; a GCC for a processor without these options says what it says of
    /// any option it doesn't know, and of a processor it doesn't know.
    /// Returns whether any was an error.
    pub(super) fn report_x86_options_on_wasm(&self) -> bool {
        use super::driver::{
            Severity,
            report,
        };
        if !self.target.is_wasm() {
            return false;
        }
        let gcc = CompilerIdentity::from(self.compiler_identity) == CompilerIdentity::Gcc;
        let triple = self.target.triple();
        let mut failed = false;
        for option in &self.language_option {
            let option = option.as_str();
            if !crate::target::is_feature_option(option) {
                continue;
            }
            if !gcc && matches!(option, "-msse4.2" | "-msse4") {
                report(
                    self,
                    Severity::Warning,
                    format_args!(
                        "argument unused during compilation: '{option}' \
                         [-Wunused-command-line-argument]"
                    ),
                );
                continue;
            }
            failed = true;
            match (gcc, option.strip_prefix("-march=")) {
                | (true, Some(architecture)) => report(
                    self,
                    Severity::Error,
                    format_args!("bad value '{architecture}' for '-march=' switch"),
                ),
                | (true, None) => report(
                    self,
                    Severity::Error,
                    format_args!("unrecognized command-line option '{option}'"),
                ),
                | (false, architecture) => {
                    let option = match option {
                        | _ if architecture.is_some() => "-march=",
                        | "-mno-sse4" => "-mno-sse4.1",
                        | other => other,
                    };
                    report(
                        self,
                        Severity::Error,
                        format_args!("unsupported option '{option}' for target '{triple}'"),
                    );
                },
            }
        }
        failed
    }

    /// The library functions code generation may treat as builtins, from
    /// `-fbuiltin`, `-fno-builtin` and `-fno-builtin-NAME` in order, as
    /// GCC reads them. `-ffreestanding` allows none, which lowering applies.
    pub(super) fn library_builtins<'tu>(
        &self,
        arena: &'tu crate::util::bump::Bump,
    ) -> crate::configuration::LibraryBuiltins<'tu> {
        use crate::configuration::LibraryBuiltins;
        let mut none = false;
        let mut excluded = crate::util::bump::ArenaVec::new_in(arena);
        for option in &self.language_option {
            match option.as_str() {
                // As in GCC, `-fbuiltin` undoes `-fno-builtin` but keeps
                // the functions `-fno-builtin-NAME` named.
                | "-fbuiltin" => none = false,
                | "-fno-builtin" => none = true,
                | _ =>
                    if let Some(name) = option.strip_prefix("-fno-builtin-") {
                        excluded.push(&*arena.alloc_str(name));
                    },
            }
        }
        if none {
            LibraryBuiltins::None
        } else if excluded.is_empty() {
            LibraryBuiltins::All
        } else {
            LibraryBuiltins::Except(excluded.leak())
        }
    }

    /// The header GCC reads before the main file on a glibc target,
    /// `<stdc-predef.h>`, as its `TARGET_C_PREINCLUDE` names it: under the
    /// GCC identity, hosted, without `-nostdinc`, and only when the search
    /// finds it, since GCC ignores a missing one. Clang reads none.
    /// GCC internals: "Target Macros", `TARGET_C_PREINCLUDE`.
    /// <https://gcc.gnu.org/onlinedocs/gccint/Misc.html>
    #[expect(
        clippy::disallowed_methods,
        reason = "Startup probes the header directories once, joining a path."
    )]
    pub(super) fn preinclude(
        &self,
        search: &crate::headers::HeaderSearch<'_>,
    ) -> Option<&'static str> {
        const HEADER: &str = "stdc-predef.h";
        let configuration = self.configuration();
        (configuration.compiler_identity() == CompilerIdentity::Gcc
            && configuration.target() == Target::LinuxGnu
            && configuration.hosted()
            && self.search.standard_includes()
            && search
                .angled
                .iter()
                .chain(search.system)
                .chain(search.after)
                .any(|directory| directory.join(HEADER).is_file()))
        .then_some(HEADER)
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

/// Whether `text` asks for a run-time check [`Cli::code_options`] gives Linux
/// alone, so that a personality warns about it on Windows.
pub(super) fn is_hardening_option(text: &str) -> bool {
    matches!(
        text,
        "-fstack-protector"
            | "-fstack-protector-strong"
            | "-fstack-protector-all"
            | "-fno-stack-protector"
            | "-fcf-protection"
            | "-fcf-protection=full"
            | "-fcf-protection=branch"
            | "-fcf-protection=return"
            | "-fcf-protection=none"
            | "-fstack-clash-protection"
            | "-fno-stack-clash-protection"
            | "-pg"
    ) || is_sanitizer_option(text)
}

/// Whether `text` turns a sanitizer bcc can give on or off. GCC's and Clang's
/// other sanitizers and lists of several stay refused.
pub(super) fn is_sanitizer_option(text: &str) -> bool {
    matches!(
        text,
        "-fsanitize=address"
            | "-fsanitize=thread"
            | "-fno-sanitize=address"
            | "-fno-sanitize=thread"
            | "-fno-sanitize=all"
    )
}

/// Whether `text` chooses what GCC's or Clang's format writes. Each compiler
/// knows only some of these, and its personality checks which.
pub(super) fn is_diagnostic_presentation(text: &str) -> bool {
    matches!(
        text,
        "-fdiagnostics-show-caret"
            | "-fno-diagnostics-show-caret"
            | "-fcaret-diagnostics"
            | "-fno-caret-diagnostics"
            | "-fshow-column"
            | "-fno-show-column"
            | "-fdiagnostics-show-line-numbers"
            | "-fno-diagnostics-show-line-numbers"
            | "-fdiagnostics-show-option"
            | "-fno-diagnostics-show-option"
            | "-fdiagnostics-color"
            | "-fno-diagnostics-color"
            | "-fdiagnostics-color=always"
            | "-fdiagnostics-color=never"
            | "-fdiagnostics-color=auto"
            | "-fcolor-diagnostics"
            | "-fno-color-diagnostics"
            | "-fdiagnostics-plain-output"
    )
}

/// Which of reassociation, reciprocals and approximate functions the
/// floating-point options among `options` leave given up, in that order:
/// the options Clang refuses beside `-ffp-eval-method=`.
pub(super) fn eval_method_conflicts<'a>(options: impl IntoIterator<Item = &'a str>) -> [bool; 3] {
    let configuration = options
        .into_iter()
        .filter_map(LanguageFlag::parse)
        .fold(CompilerConfiguration::default(), |configuration, flag| {
            flag.apply(configuration)
        });
    [
        configuration.associative_math(),
        configuration.relaxes_floating_point(RelaxedFloatingPoint::ReciprocalMath),
        configuration.relaxes_floating_point(RelaxedFloatingPoint::ApproxFunc),
    ]
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
    ExtraWarnings(bool),
    /// `-Wformat=0`, `-Wformat=1` or `-Wformat=2`.
    FormatLevel(u8),
    UnknownWarning,
    /// `-Wfatal-errors` or `-Wno-fatal-errors`.
    FatalErrors(bool),
    /// `-pedantic` or `-Wpedantic` (true), or `-Wno-pedantic` (false).
    Pedantic(bool),
    /// `-pedantic-errors`.
    PedanticErrors,
    /// `-Werror=pedantic` (true) or `-Wno-error=pedantic` (false).
    PedanticAsError(bool),
    /// `-fms-extensions` or `-fno-ms-extensions`.
    MsvcExtensions(bool),
    /// `-fms-<feature>` or `-fno-ms-<feature>`.
    MsvcFeature(MsvcFeature, bool),
    /// `-fhosted` (true) or `-ffreestanding` (false).
    Hosted(bool),
    /// `-fgnu89-inline` or `-fno-gnu89-inline`.
    Gnu89Inline(bool),
    /// `-fsigned-char` or `-fno-unsigned-char` (true), `-funsigned-char` or
    /// `-fno-signed-char` (false).
    CharSigned(bool),
    /// `-ffloatn-types` or `-fno-floatn-types`, bcc's own switch for the
    /// `_FloatN` keywords a GCC identity provides.
    InterchangeFloatTypes(bool),
    /// `-fno-signed-zeros`, `-freciprocal-math` and their relatives (true),
    /// or their opposites, such as `-fsigned-zeros`.
    RelaxedFloatingPoint(RelaxedFloatingPoint, bool),
    /// `-fsignaling-nans` (true) or `-fno-signaling-nans`.
    SignalingNans(bool),
    /// `--general-regs-only`, which GCC's `-mgeneral-regs-only` maps to.
    GeneralRegsOnly,
    /// `--user-interrupts`, which `-muintr` maps to.
    UserInterrupts,
    /// `-ffast-math`, `-funsafe-math-optimizations` or
    /// `-ffinite-math-only` (true), or their `-fno-` forms, which give up
    /// or restore the options they imply, in order with the others, as in
    /// GCC 13 and Clang.
    FloatingPointGroup(FloatingPointGroup, bool),
    /// `-fexcess-precision=standard` (true) or `=fast`.
    ExcessPrecisionStandard(bool),
    /// `-trigraphs` or `-ftrigraphs` (true), or `-fno-trigraphs`.
    Trigraphs(bool),
    /// `-fdollars-in-identifiers` (true) or `-fno-dollars-in-identifiers`.
    DollarIdentifiers(bool),
    /// `-fwrapv` (true), or `-fno-wrapv` and `-fstrict-overflow`.
    Wrapv(bool),
    /// `-ftrapv` (true) or `-fno-trapv`.
    Trapv(bool),
    /// `-fpack-struct=N`.
    PackStruct(u32),
    /// `-fpack-struct` (true) or `-fno-pack-struct`, which also clears N.
    PackAll(bool),
    /// `-fvisibility=default|hidden|protected|internal`.
    Visibility(crate::configuration::DefaultVisibility),
    /// `-fpermissive` (true) or `-fno-permissive`: GCC's C type-safety
    /// errors become warnings.
    Permissive(bool),
    /// `-fbuiltin`, `-fno-builtin` or `-fno-builtin-NAME`, which
    /// [`Cli::library_builtins`] reads in order.
    Builtins,
    /// A `-fpic`-family flag, which [`Cli::code_options`] reads in order.
    Relocation,
    /// `-f[no-]wrapv-pointer`, `-f[no-]delete-null-pointer-checks` or
    /// `-f[no-]common`, which [`Cli::code_assumptions`] reads in order, or
    /// `-f[no-]strict-volatile-bitfields`, `-f[no-]function-sections` or
    /// `-f[no-]data-sections`, which [`Cli::code_options`] does.
    Code,
    /// An option [`Cli::diagnostic_presentation`] reads.
    Presentation,
}

/// An option that gives up or restores several [`RelaxedFloatingPoint`]
/// options at once.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FloatingPointGroup {
    /// `-f[no-]fast-math`.
    Fast,
    /// `-f[no-]unsafe-math-optimizations`.
    Unsafe,
    /// `-f[no-]finite-math-only`.
    Finite,
}

impl FloatingPointGroup {
    /// Gives up (`relaxed`) or restores the group's options. `-ffast-math`
    /// and its `-fno-` form also set the complex arithmetic, unless a
    /// `-f[no-]cx-limited-range` was given, as GCC's `set_fast_math_flags`
    /// does (Clang's `-fno-fast-math` would restore it even then).
    fn apply(self, configuration: CompilerConfiguration, relaxed: bool) -> CompilerConfiguration {
        let options: &[RelaxedFloatingPoint] = match self {
            | Self::Fast => &RelaxedFloatingPoint::FAST_MATH,
            | Self::Unsafe => &RelaxedFloatingPoint::UNSAFE_MATH,
            | Self::Finite => &RelaxedFloatingPoint::FINITE_MATH,
        };
        let configuration = options
            .iter()
            .fold(configuration, |configuration, &option| {
                configuration.with_implied_floating_point(option, relaxed)
            });
        // GCC's `-ffast-math` also turns `-fsignaling-nans` off and makes
        // excess precision fast (`set_fast_math_flags`), unless a later
        // `-fno-fast-math` cancels it; Clang ignores both options.
        let configuration = match (self, relaxed) {
            | (Self::Fast, true) => configuration.with_fast_math_side_effects(),
            | (Self::Fast, false) => configuration.with_fast_math_cancelled(),
            | _ => configuration,
        };
        let configuration = match self {
            | Self::Fast => configuration.with_fast_math_given(false, relaxed),
            | Self::Unsafe => configuration.with_fast_math_given(true, relaxed),
            | Self::Finite => configuration,
        };
        if self == Self::Fast
            && !configuration.gives_floating_point(RelaxedFloatingPoint::CxLimitedRange)
        {
            configuration.with_implied_floating_point(RelaxedFloatingPoint::CxLimitedRange, relaxed)
        } else {
            configuration
        }
    }
}

impl LanguageFlag {
    fn parse(text: &str) -> Option<Self> {
        match text {
            | "-pedantic" | "-Wpedantic" => return Some(Self::Pedantic(true)),
            | "-pedantic-errors" => return Some(Self::PedanticErrors),
            | "-Werror=pedantic" => return Some(Self::PedanticAsError(true)),
            | "-Wno-error=pedantic" => return Some(Self::PedanticAsError(false)),
            | "-Werror" => return Some(Self::WarningsAsErrors(true)),
            | "-Wno-error" => return Some(Self::WarningsAsErrors(false)),
            | "-Wall" => return Some(Self::AllWarnings(true)),
            | "-Wno-all" => return Some(Self::AllWarnings(false)),
            | "-Wextra" | "-W" => return Some(Self::ExtraWarnings(true)),
            | "-Wno-extra" => return Some(Self::ExtraWarnings(false)),
            | "-Wno-pedantic" => return Some(Self::Pedantic(false)),
            | "-Wfatal-errors" => return Some(Self::FatalErrors(true)),
            | "-Wno-fatal-errors" => return Some(Self::FatalErrors(false)),
            | "-fhosted" => return Some(Self::Hosted(true)),
            | "-ffreestanding" => return Some(Self::Hosted(false)),
            | "-fgnu89-inline" => return Some(Self::Gnu89Inline(true)),
            | "-fno-gnu89-inline" => return Some(Self::Gnu89Inline(false)),
            | "-fsigned-char" | "-fno-unsigned-char" => return Some(Self::CharSigned(true)),
            | "-funsigned-char" | "-fno-signed-char" => return Some(Self::CharSigned(false)),
            | "-ffloatn-types" => return Some(Self::InterchangeFloatTypes(true)),
            | "-fno-signed-zeros" | "-fsigned-zeros" =>
                return Some(Self::RelaxedFloatingPoint(
                    RelaxedFloatingPoint::NoSignedZeros,
                    text == "-fno-signed-zeros",
                )),
            | "-freciprocal-math" | "-fno-reciprocal-math" =>
                return Some(Self::RelaxedFloatingPoint(
                    RelaxedFloatingPoint::ReciprocalMath,
                    text == "-freciprocal-math",
                )),
            | "-fno-trapping-math" | "-ftrapping-math" =>
                return Some(Self::RelaxedFloatingPoint(
                    RelaxedFloatingPoint::NoTrappingMath,
                    text == "-fno-trapping-math",
                )),
            | "-fno-math-errno" | "-fmath-errno" =>
                return Some(Self::RelaxedFloatingPoint(
                    RelaxedFloatingPoint::NoMathErrno,
                    text == "-fno-math-errno",
                )),
            | "-fassociative-math" | "-fno-associative-math" =>
                return Some(Self::RelaxedFloatingPoint(
                    RelaxedFloatingPoint::AssociativeMath,
                    text == "-fassociative-math",
                )),
            | "-fcx-limited-range" | "-fno-cx-limited-range" =>
                return Some(Self::RelaxedFloatingPoint(
                    RelaxedFloatingPoint::CxLimitedRange,
                    text == "-fcx-limited-range",
                )),
            | "-fcx-fortran-rules" | "-fno-cx-fortran-rules" =>
                return Some(Self::RelaxedFloatingPoint(
                    RelaxedFloatingPoint::CxFortranRules,
                    text == "-fcx-fortran-rules",
                )),
            | "-fsignaling-nans" => return Some(Self::SignalingNans(true)),
            | "-fno-signaling-nans" => return Some(Self::SignalingNans(false)),
            | "--general-regs-only" => return Some(Self::GeneralRegsOnly),
            | "--user-interrupts" => return Some(Self::UserInterrupts),
            | "-fno-honor-nans" | "-fhonor-nans" =>
                return Some(Self::RelaxedFloatingPoint(
                    RelaxedFloatingPoint::NoNans,
                    text == "-fno-honor-nans",
                )),
            | "-fno-honor-infinities" | "-fhonor-infinities" =>
                return Some(Self::RelaxedFloatingPoint(
                    RelaxedFloatingPoint::NoInfinities,
                    text == "-fno-honor-infinities",
                )),
            | "-fapprox-func" | "-fno-approx-func" =>
                return Some(Self::RelaxedFloatingPoint(
                    RelaxedFloatingPoint::ApproxFunc,
                    text == "-fapprox-func",
                )),
            | "-fexcess-precision=standard" | "-fexcess-precision=fast" =>
                return Some(Self::ExcessPrecisionStandard(
                    text == "-fexcess-precision=standard",
                )),
            | "-ffast-math" | "-fno-fast-math" =>
                return Some(Self::FloatingPointGroup(
                    FloatingPointGroup::Fast,
                    text == "-ffast-math",
                )),
            | "-funsafe-math-optimizations" | "-fno-unsafe-math-optimizations" =>
                return Some(Self::FloatingPointGroup(
                    FloatingPointGroup::Unsafe,
                    text == "-funsafe-math-optimizations",
                )),
            | "-ffinite-math-only" | "-fno-finite-math-only" =>
                return Some(Self::FloatingPointGroup(
                    FloatingPointGroup::Finite,
                    text == "-ffinite-math-only",
                )),
            | "-fno-floatn-types" => return Some(Self::InterchangeFloatTypes(false)),
            | "-trigraphs" | "-ftrigraphs" => return Some(Self::Trigraphs(true)),
            | "-fno-trigraphs" => return Some(Self::Trigraphs(false)),
            | "-fdollars-in-identifiers" => return Some(Self::DollarIdentifiers(true)),
            | "-fno-dollars-in-identifiers" => return Some(Self::DollarIdentifiers(false)),
            // `-f[no-]strict-overflow` also sets the pointer option, which
            // `Cli::code_assumptions` reads.
            | "-fwrapv" | "-fno-strict-overflow" => return Some(Self::Wrapv(true)),
            | "-fno-wrapv" | "-fstrict-overflow" => return Some(Self::Wrapv(false)),
            | "-ftrapv" => return Some(Self::Trapv(true)),
            | "-fno-trapv" => return Some(Self::Trapv(false)),
            | "-fwrapv-pointer"
            | "-fno-wrapv-pointer"
            | "-fdelete-null-pointer-checks"
            | "-fno-delete-null-pointer-checks"
            | "-fcommon"
            | "-fno-common"
            | "-fstrict-volatile-bitfields"
            | "-fno-strict-volatile-bitfields"
            | "-ffunction-sections"
            | "-fno-function-sections"
            | "-fdata-sections"
            | "-fno-data-sections"
            | "-masm=intel"
            | "-masm=att"
            | "-fno-omit-frame-pointer"
            | "-fomit-frame-pointer" => return Some(Self::Code),
            | _ if is_hardening_option(text) => return Some(Self::Code),
            | _ if is_diagnostic_presentation(text) => return Some(Self::Presentation),
            | _ if crate::target::is_feature_option(text)
                || text == crate::target::UNKNOWN_VECTOR_ABI =>
                return Some(Self::Code),
            | "-fbuiltin" | "-fno-builtin" => return Some(Self::Builtins),
            | "-fpermissive" => return Some(Self::Permissive(true)),
            | "-fno-permissive" => return Some(Self::Permissive(false)),
            | "-fpack-struct" => return Some(Self::PackAll(true)),
            | "-fno-pack-struct" => return Some(Self::PackAll(false)),
            | _ => {},
        }
        if text
            .strip_prefix("-fno-builtin-")
            .is_some_and(|name| !name.is_empty())
        {
            return Some(Self::Builtins);
        }
        if let Some(value) = text.strip_prefix("-fvisibility=") {
            use crate::configuration::DefaultVisibility as V;
            return match value {
                | "default" => Some(Self::Visibility(V::Default)),
                | "hidden" | "internal" => Some(Self::Visibility(V::Hidden)),
                | "protected" => Some(Self::Visibility(V::Protected)),
                | _ => None,
            };
        }
        if let Some(value) = text.strip_prefix("-fpack-struct=") {
            return match value {
                | "1" | "2" | "4" | "8" | "16" => value.parse().ok().map(Self::PackStruct),
                | _ => None,
            };
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
            } else if let Some(level) = text.strip_prefix("unused-const-variable=") {
                // GCC's level: 0 is off; level 1, which `-Wunused` gives C, and
                // level 2 differ in headers, which bcc does not report.
                return Some(if matches!(level, "0" | "1" | "2") {
                    Self::WarningGroup(
                        DiagnosticGroup::UnusedConstVariable,
                        WarningOption::Enable(level != "0"),
                    )
                } else {
                    Self::UnknownWarning
                });
            } else if let Some(level) = text.strip_prefix("format=") {
                // The levels of GCC's `-Wformat=` and Clang's groups:
                // `-Wformat=0` is `-Wno-format`, 1 is `-Wformat`, and 2 adds
                // `-Wformat-security` and `-Wformat-nonliteral`.
                return Some(match level {
                    | "0" | "1" | "2" => Self::FormatLevel(level.as_bytes()[0] - b'0'),
                    | _ => Self::UnknownWarning,
                });
            } else if let Some(level) = text.strip_prefix("normalized=") {
                // GCC's level: `none` turns the check off, and the others
                // all report the one case bcc detects.
                return Some(if matches!(level, "none" | "id" | "nfc" | "nfkc") {
                    Self::WarningGroup(
                        DiagnosticGroup::Normalized,
                        WarningOption::Enable(level != "none"),
                    )
                } else {
                    Self::UnknownWarning
                });
            } else {
                (text, WarningOption::Enable(true))
            };
            // GCC's `-Waddress` and Clang's group around
            // `-Wpointer-bool-conversion`, the part of them bcc reports.
            let group = if name == "address" {
                Some(DiagnosticGroup::PointerBoolConversion)
            } else {
                DiagnosticGroup::parse(name)
            };
            return Some(group.map_or(Self::UnknownWarning, |group| {
                Self::WarningGroup(group, option)
            }));
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
            | Self::UnknownWarning
            | Self::Relocation
            | Self::Code
            | Self::Presentation
            | Self::Builtins => configuration,
            // GCC's and Clang's `-Wall` hold `-Wimplicit` and
            // `-Wreturn-mismatch`, each in its own way.
            | Self::AllWarnings(enabled) => configuration.with_all_warnings(enabled),
            | Self::ExtraWarnings(enabled) => configuration.with_extra_warnings(enabled),
            | Self::WarningsAsErrors(enabled) => configuration.with_warnings_as_errors(enabled),
            | Self::WarningGroup(group, option) => configuration.with_warning_option(group, option),
            | Self::FormatLevel(level) => configuration.with_format_level(level),
            | Self::FatalErrors(enabled) => configuration.with_fatal_errors(enabled),
            | Self::Pedantic(enabled) => configuration.with_pedantic(enabled),
            | Self::PedanticErrors => configuration.with_pedantic_errors(),
            | Self::PedanticAsError(true) => configuration.with_werror_pedantic(),
            | Self::PedanticAsError(false) => configuration.with_pedantic_as_error(false),
            | Self::MsvcExtensions(enabled) => configuration.with_msvc_extensions(enabled),
            | Self::MsvcFeature(feature, enabled) =>
                configuration.with_msvc_feature(feature, enabled),
            | Self::Hosted(hosted) => configuration.with_hosted(hosted),
            | Self::Gnu89Inline(enabled) => configuration.with_gnu89_inline(enabled),
            | Self::CharSigned(signed) => configuration.with_char_signed(signed),
            | Self::InterchangeFloatTypes(enabled) =>
                configuration.with_interchange_float_types(enabled),
            | Self::RelaxedFloatingPoint(option, relaxed) =>
                configuration.with_relaxed_floating_point(option, relaxed),
            | Self::SignalingNans(enabled) => configuration.with_signaling_nans(enabled),
            | Self::GeneralRegsOnly => configuration.with_general_regs_only(true),
            | Self::UserInterrupts => configuration.with_user_interrupts(true),
            | Self::FloatingPointGroup(group, relaxed) => group.apply(configuration, relaxed),
            | Self::ExcessPrecisionStandard(standard) =>
                configuration.with_standard_excess_precision(standard),
            | Self::Trigraphs(enabled) => configuration.with_trigraphs(enabled),
            | Self::DollarIdentifiers(enabled) => configuration.with_dollar_identifiers(enabled),
            | Self::Wrapv(enabled) => configuration.with_wrapv(enabled),
            | Self::Trapv(enabled) => configuration.with_trapv(enabled),
            | Self::PackStruct(alignment) => configuration.with_default_pack(alignment),
            | Self::PackAll(true) => configuration.with_pack_structures(true),
            | Self::PackAll(false) => configuration
                .with_pack_structures(false)
                .with_default_pack(0),
            | Self::Visibility(visibility) => configuration.with_default_visibility(visibility),
            | Self::Permissive(enabled) => configuration.with_permissive(enabled),
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
const SINGLE_DASH_LINKER_OPTIONS: [&str; 9] = [
    "-r",
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

/// GCC's and Clang's `-g` options as the driver's, read in order as they do:
/// `-g0` turns debug information off and any other level turns it on, a plain
/// `-g`, `-ggdb` or `-gdwarf-N` keeps a level already asked for, and
/// `-gtoggle`, which the compilers apply last, flips the result.
#[derive(Default)]
struct DebugFlags {
    level:   u8,
    toggled: bool,
}

impl DebugFlags {
    /// Translates `text` if it is a debug option the driver acts on.
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_macros,
        reason = "Startup argv normalization owns OS strings beside clap."
    )]
    fn read(&mut self, text: &str, normalized: &mut Vec<std::ffi::OsString>) -> bool {
        match text {
            | "-g" | "-ggdb" => self.keep_or_default(normalized),
            | "-gdwarf" => {
                self.keep_or_default(normalized);
                normalized.push("--dwarf-version=5".into());
            },
            | "-gline-tables-only" | "-gmlt" => self.set(1, normalized),
            | "-gtoggle" => {
                self.toggled = !self.toggled;
                normalized.push("--debug-toggle".into());
            },
            | "-gcolumn-info" => normalized.push("--debug-columns".into()),
            | "-gno-column-info" => normalized.push("--no-debug-columns".into()),
            | _ =>
                if let Some(version) = text.strip_prefix("-gdwarf-") {
                    self.keep_or_default(normalized);
                    normalized.push(format!("--dwarf-version={version}").into());
                } else if let Some(level) = text
                    .strip_prefix("-ggdb")
                    .or_else(|| text.strip_prefix("-g"))
                    .and_then(|level| level.parse().ok())
                {
                    self.set(level, normalized);
                } else {
                    return false;
                },
        }
        true
    }

    #[expect(
        clippy::disallowed_types,
        reason = "Startup argv normalization owns OS strings beside clap."
    )]
    fn keep_or_default(&mut self, normalized: &mut Vec<std::ffi::OsString>) {
        if self.level == 0 {
            self.set(2, normalized);
        }
    }

    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_macros,
        reason = "Startup argv normalization owns OS strings beside clap."
    )]
    fn set(&mut self, level: u8, normalized: &mut Vec<std::ffi::OsString>) {
        self.level = level;
        normalized.push(format!("--debug-level={level}").into());
    }
}
