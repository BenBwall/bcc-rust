//! Shared extension diagnostics for translation phases 1-7.
//!
//! C99: §4p6, p. 7; PDF p. 19 permits extensions that preserve conforming
//! programs. Diagnostics follow §5.1.1.3p1, p. 11; PDF p. 23.

use std::{
    cell::Cell,
    fmt::{
        Display,
        Formatter,
        Result as FmtResult,
    },
};

use super::{
    Context,
    ErrorSeverity,
    GetPosition,
    GetSeverity,
    GetSourceVectors,
    SourcePosition,
    SourceVectors,
    TranslationError,
    errors::ClassifiedError,
};
use crate::{
    configuration::{
        CompilerIdentity,
        ExtensionClass,
        ExtensionPolicy,
        Feature,
        FeatureOrigin,
        GccFiling,
        PragmaClass,
        RuleSet,
    },
    diagnostics::{
        Diagnostic,
        Explanation,
        ToDiagnostic,
        format_in,
    },
    util::bump::Bump,
};

impl<'tu> Context<'tu> {
    /// Feature-based adapter for lexer, preprocessor, and parser consumers.
    /// A standard feature native to the selected revision has a native
    /// origin, so the origin check alone decides whether it is reported.
    pub(crate) fn report_extension(
        &mut self,
        feature: Feature,
        spelling: &str,
        source: SourceVectors,
    ) {
        if !self.configuration.silent_extension(feature) {
            let filing = self.gcc_filing(Some(feature));
            self.report_extension_in(
                spelling,
                feature.origin(),
                source,
                ExtensionOption::Pedantic,
                filing,
            );
        }
    }

    /// [`Self::report_extension`] for syntax Clang warns of by default (an
    /// `ExtWarn` of `-Wc23-extensions`, such as a label at the end of a
    /// block), which its `-Wpedantic` does not hold (`InferPedantic` in
    /// `ClangDiagnosticsEmitter.cpp`): a pragma naming that option maps none
    /// of it, so `-Wfatal-errors` makes fatal the error `-pedantic-errors`
    /// made of it. Only a pragma naming `-Weverything` maps it
    /// ([`ExtensionOption::Everything`]); otherwise the command line's
    /// extension policy decides it, which such a pragma can only turn on as
    /// a warning ([`Self::extension_policy_at`]).
    /// C99: §5.1.1.3 paragraph 1, p. 11; PDF p. 23.
    pub(crate) fn report_extension_outside_pedantic(
        &mut self,
        feature: Feature,
        spelling: &str,
        source: SourceVectors,
    ) {
        if !self.configuration.silent_extension(feature) {
            self.report_extension_in(
                spelling,
                feature.origin(),
                source,
                ExtensionOption::Everything,
                self.gcc_filing(Some(feature)),
            );
        }
    }

    /// Reports syntax from `origin` under the extension policy. GNU/MSVC
    /// extensions remain non-ISO even when enabled; standard features cease
    /// being extensions in a revision that has them.
    pub(crate) fn report_extension_since(
        &mut self,
        spelling: &str,
        origin: FeatureOrigin,
        source_vectors: SourceVectors,
    ) {
        self.report_extension_in(
            spelling,
            origin,
            source_vectors,
            ExtensionOption::Pedantic,
            self.gcc_filing(None),
        );
    }

    /// [`Self::report_extension_since`] for a keyword, which GCC's C front
    /// end reports under `-Wpedantic`, though bcc reads keywords while it
    /// converts preprocessing tokens.
    pub(crate) fn report_keyword_extension(
        &mut self,
        spelling: &str,
        origin: FeatureOrigin,
        source_vectors: SourceVectors,
    ) {
        self.report_extension_in(
            spelling,
            origin,
            source_vectors,
            ExtensionOption::Pedantic,
            GccFiling::Pedantic,
        );
    }

    /// [`Self::report_extension_since`], for a diagnostic of the warning
    /// `option`, which GCC files as `filing` says.
    fn report_extension_in(
        &mut self,
        spelling: &str,
        origin: FeatureOrigin,
        source_vectors: SourceVectors,
        option: ExtensionOption,
        filing: GccFiling,
    ) {
        let Some(severity) = self.extension_severity(
            origin,
            DiagnosticPolicy::Extension,
            source_vectors,
            option,
            filing,
        ) else {
            return;
        };
        self.emit_extension(spelling, origin, severity, source_vectors, option, filing);
    }

    /// What the settings at `source` make of an extension diagnostic of the
    /// warning `option`, if they report it.
    fn extension_severity(
        &self,
        origin: FeatureOrigin,
        baseline: DiagnosticPolicy,
        source: SourceVectors,
        option: ExtensionOption,
        filing: GccFiling,
    ) -> Option<ErrorSeverity> {
        if matches!(
            origin,
            FeatureOrigin::Standard(_) | FeatureOrigin::Removed { .. }
        ) && self.configuration.origin_is_native(origin)
        {
            return None;
        }
        // Clang's `-w` silences every diagnostic that is not an error by
        // default, whatever an option or a pragma made of it
        // (`DiagnosticIDs::getDiagnosticSeverity`), and GCC's every pedantic
        // warning before `-pedantic-errors` can make it an error
        // (`diagnostic_report_diagnostic` in `diagnostic.cc`). Only syntax the
        // mode does not accept stays an error.
        if self.configuration.suppress_warnings() && !matches!(baseline, DiagnosticPolicy::Error) {
            return None;
        }
        let severity = if let Some(decided) =
            self.command_line_extension_severity(origin, baseline, source, option, filing)
        {
            decided?
        } else {
            match self.everything_class_outside_pedantic(option, source) {
                | Some(PragmaClass::Ignored) => return None,
                | Some(PragmaClass::Warning) => ErrorSeverity::Warning,
                | Some(PragmaClass::Error) => ErrorSeverity::Error,
                | None => policy_severity(self.extension_policy_at(option, source), baseline)?,
            }
        };
        (!self.withholds(
            severity,
            !matches!(baseline, DiagnosticPolicy::Error),
            source,
        ))
        .then_some(severity)
    }

    /// What the command line's warning options alone make of an extension
    /// diagnostic of the warning `option` at `source`, where no diagnostic
    /// pragma governs it (`None` otherwise, and for an identity whose rules
    /// bcc does not model this way): Clang's mapping of it
    /// ([`CompilerConfiguration::clang_extension_severity`]), or GCC's
    /// classification of its `-Wpedantic`, which bcc files it under
    /// ([`CompilerConfiguration::gcc_pedantic_severity`]). The severity is
    /// final: `-w` and `-Werror` have had their say.
    /// C99: §5.1.1.3 paragraph 1, p. 11; PDF p. 23.
    ///
    /// [`CompilerConfiguration::clang_extension_severity`]: crate::configuration::CompilerConfiguration::clang_extension_severity
    /// [`CompilerConfiguration::gcc_pedantic_severity`]: crate::configuration::CompilerConfiguration::gcc_pedantic_severity
    fn command_line_extension_severity(
        &self,
        origin: FeatureOrigin,
        baseline: DiagnosticPolicy,
        source: SourceVectors,
        option: ExtensionOption,
        filing: GccFiling,
    ) -> Option<Option<ErrorSeverity>> {
        if !self.command_line_classifies(option, source, filing) {
            return None;
        }
        let configuration = self.configuration;
        Some(
            if configuration.compiler_identity().rules() == RuleSet::Clang {
                configuration.clang_extension_severity(ExtensionClass::of(
                    origin,
                    option == ExtensionOption::Everything,
                ))
            } else {
                // GCC files what Clang warns of by default under `-Wpedantic`
                // too, which enables it.
                let baseline = match option {
                    | ExtensionOption::Everything => DiagnosticPolicy::Extension,
                    | _ => baseline,
                };
                let policy = configuration.extension_policy();
                configuration
                    .gcc_pedantic_severity(policy_severity(policy, baseline).is_some(), filing)
            },
        )
    }

    /// Whether the command line alone classifies an extension diagnostic of
    /// the warning `option` at `source`
    /// ([`Self::command_line_extension_severity`]). What GCC's preprocessor
    /// reports ([`GccFiling::Preprocessor`]) stays the extension policy's.
    pub(crate) fn command_line_classifies(
        &self,
        option: ExtensionOption,
        source: SourceVectors,
        filing: GccFiling,
    ) -> bool {
        option != ExtensionOption::None
            && (self.configuration.compiler_identity().rules() == RuleSet::Clang
                || (self.configuration.compiler_identity() == CompilerIdentity::Gcc
                    && filing != GccFiling::Preprocessor))
            && !self.governed_by_pragma(source)
    }

    /// Queues an extension diagnostic whose severity the command line's
    /// options settled, which the reporter must not promote again: `-Werror`
    /// has had its say, and `-Wno-error=` may have kept it a warning.
    /// `-Wfatal-errors` still makes such an error fatal.
    #[cold]
    #[inline(never)]
    fn push_final_extension(&mut self, mut error: TranslationError<'tu>) {
        let severity = error.severity();
        if let TranslationError::Parsing(parsing) = &mut error {
            parsing.severity_is_final = true;
            self.pending_errors.push_back(error);
            return;
        }
        let error = self.tu_arena().alloc(error);
        self.pending_errors
            .push_back(TranslationError::Classified(ClassifiedError {
                error,
                severity,
                fatal: false,
                mapped: false,
            }));
    }

    /// How GCC files a diagnostic about `feature`, or about syntax whose
    /// feature the caller does not name, reported now.
    fn gcc_filing(&self, feature: Option<Feature>) -> GccFiling {
        if !self.parsing_started {
            if feature.is_some_and(Feature::gcc_preprocessor_names_no_option) {
                GccFiling::NoOption
            } else {
                GccFiling::Preprocessor
            }
        } else if feature.is_some_and(Feature::gcc_files_under_own_option) {
            GccFiling::OwnOption
        } else {
            GccFiling::Pedantic
        }
    }

    /// Queues an extension diagnostic of the warning `option`: one whose
    /// severity the command line settled is final
    /// ([`Self::command_line_classifies`]); a pragma classifies the others.
    fn queue_extension(
        &mut self,
        source: SourceVectors,
        error: TranslationError<'tu>,
        option: ExtensionOption,
        filing: GccFiling,
    ) {
        if self.command_line_classifies(option, source, filing) {
            self.push_final_extension(error);
        } else {
            self.classified_extension(source, error, option);
        }
    }

    /// Queues the diagnostic of the warning `option` once the policy has asked
    /// for one. Callers inline only the policy checks above.
    #[cold]
    #[inline(never)]
    fn emit_extension(
        &mut self,
        spelling: &str,
        origin: FeatureOrigin,
        severity: ErrorSeverity,
        source_vectors: SourceVectors,
        option: ExtensionOption,
        filing: GccFiling,
    ) {
        let spelling = self.diagnostic_text(spelling);
        let at = source_vectors;
        let source_vectors = self.file_location(source_vectors);
        let suppressed = self.tu_arena().alloc(Cell::new(false));
        // GCC names `-Werror` where it, or `-Werror=pedantic`, made a warning
        // an error, but not where `-pedantic-errors` made it one
        // (`print_option_information` in `diagnostic.cc`).
        let promoted = severity == ErrorSeverity::Error
            && self.configuration.compiler_identity() == CompilerIdentity::Gcc
            && !self.configuration.pedantic_errors()
            && self.command_line_classifies(option, at, filing);
        let gcc_promotion = match filing {
            | _ if !promoted => GccPromotion::None,
            | GccFiling::NoOption => GccPromotion::Werror,
            | _ => GccPromotion::WerrorPedantic,
        };
        self.queue_extension(
            at,
            TranslationError::Extension(ExtensionDiagnostic {
                suppressed,
                spelling,
                origin,
                severity,
                gcc_promotion,
                source_vectors,
            }),
            option,
            filing,
        );
    }

    /// Retains a phase's typed explanation while sharing suppression and
    /// severity. Non-accepted features always diagnose as errors.
    /// C99: §4p6, p. 7; PDF p. 19; §5.1.1.3p1, p. 11; PDF p. 23.
    pub(crate) fn report_extension_diagnostic(
        &mut self,
        feature: Feature,
        baseline: DiagnosticPolicy,
        source: SourceVectors,
        diagnostic: impl FnOnce(ErrorSeverity) -> TranslationError<'tu>,
    ) {
        let baseline = if self.configuration.accepts(feature) {
            baseline
        } else {
            DiagnosticPolicy::Error
        };
        // Clang's `-Wpedantic` holds what it reports only on request, not
        // what it warns of by default (`InferPedantic` in
        // `ClangDiagnosticsEmitter.cpp`), and no option holds an error.
        let option = match baseline {
            | DiagnosticPolicy::Extension => ExtensionOption::Pedantic,
            | DiagnosticPolicy::Warning => ExtensionOption::Everything,
            | DiagnosticPolicy::WarningOnly | DiagnosticPolicy::Error => ExtensionOption::None,
        };
        let filing = self.gcc_filing(Some(feature));
        if let Some(severity) =
            self.extension_severity(feature.origin(), baseline, source, option, filing)
        {
            self.queue_extension(source, diagnostic(severity), option, filing);
        }
    }

    /// Typed preprocessor adapter; payloads keep their labels, notes and help.
    /// C99: §5.1.1.3p1, p. 11; PDF p. 23.
    pub(crate) fn preprocessor_extension(
        &mut self,
        feature: Feature,
        baseline: DiagnosticPolicy,
        source: SourceVectors,
        payload: impl FnOnce(ErrorSeverity) -> super::preprocessing::PreprocessorErrorType<'tu>,
    ) {
        self.report_extension_diagnostic(feature, baseline, source, |severity| {
            TranslationError::Preprocessing(super::preprocessing::PreprocessorError {
                error_type:     payload(severity),
                source_vectors: source,
            })
        });
    }

    /// Withdraws the pending preprocessing or extension diagnostic at
    /// `index`. An already suppressed placeholder takes its place, so every
    /// other pending diagnostic keeps its queue position, which the parser
    /// records while it runs.
    pub(crate) fn withdraw_pending_error(&mut self, index: usize) {
        let suppressed = self.tu_arena().alloc(Cell::new(false));
        let Some(slot) = self.pending_errors.iter_mut().nth(index) else {
            return;
        };
        debug_assert!(
            matches!(
                slot,
                TranslationError::Preprocessing(_)
                    | TranslationError::Extension(_)
                    | TranslationError::Classified(_)
            ),
            "only preprocessing and extension diagnostics are withdrawn"
        );
        if slot.is_suppressed() {
            return;
        }
        *slot = TranslationError::Extension(ExtensionDiagnostic {
            suppressed,
            spelling: "",
            origin: FeatureOrigin::Gnu,
            severity: ErrorSeverity::Warning,
            gcc_promotion: GccPromotion::None,
            source_vectors: SourceVectors::default(),
        });
        self.suppress_extension(suppressed);
    }
}

/// Shared severity for extensions and policy-governed constraints or quality.
/// C99: §4p6, p. 7; PDF p. 19; §5.1.1.3p1, p. 11; PDF p. 23.
pub(crate) fn policy_severity(
    policy: ExtensionPolicy,
    baseline: DiagnosticPolicy,
) -> Option<ErrorSeverity> {
    if matches!(baseline, DiagnosticPolicy::Error) {
        return Some(ErrorSeverity::Error);
    }
    if matches!(baseline, DiagnosticPolicy::WarningOnly) {
        return Some(ErrorSeverity::Warning);
    }
    match policy {
        | ExtensionPolicy::Allow =>
            matches!(baseline, DiagnosticPolicy::Warning).then_some(ErrorSeverity::Warning),
        | ExtensionPolicy::Warn => Some(ErrorSeverity::Warning),
        | ExtensionPolicy::Deny => Some(ErrorSeverity::Error),
    }
}

/// The warning option that holds an extension diagnostic, which decides the
/// diagnostic pragmas that reach it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExtensionOption {
    /// `-Wpedantic`, and Clang's `-Weverything`, which holds it.
    Pedantic,
    /// Only Clang's `-Weverything`, for what Clang warns of by default outside
    /// `-Wpedantic`, such as its `-Wc23-extensions`. GCC files it under
    /// `-Wpedantic`.
    Everything,
    /// One this code does not track, whose own group, if any, classifies it.
    None,
}

/// Baseline for a diagnostic controlled by the extension policy.
/// C99: §5.1.1.3p1, p. 11; PDF p. 23 leaves severity to the implementation.
#[derive(Clone, Copy)]
pub(crate) enum DiagnosticPolicy {
    Extension,
    Warning,
    /// Clang's object-like -Wexpansion-to-defined is not pedantic-promoted.
    WarningOnly,
    Error,
}

#[derive(Debug)]
pub(crate) struct ExtensionDiagnostic<'tu> {
    /// Arena-stable suppression marker; FIFO diagnostics keep their order.
    pub(crate) suppressed:     &'tu Cell<bool>,
    spelling:                  &'tu str,
    origin:                    FeatureOrigin,
    severity:                  ErrorSeverity,
    gcc_promotion:             GccPromotion,
    pub(super) source_vectors: SourceVectors,
}

/// What GCC's format names in place of `-Wpedantic` after an extension
/// diagnostic that its `-Werror` or `-Werror=pedantic` made an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GccPromotion {
    None,
    /// `-Werror`, for a warning GCC files under no option.
    Werror,
    WerrorPedantic,
}

impl<'tu> ExtensionDiagnostic<'tu> {
    pub(crate) fn spelling(&self) -> &'tu str {
        self.spelling
    }
}

impl Display for ExtensionDiagnostic<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self.origin {
            | FeatureOrigin::Standard(standard) =>
                write!(f, "'{}' is a {} extension", self.spelling, standard.name()),
            | FeatureOrigin::Removed { since, removed } => write!(
                f,
                "'{}' is a {} feature removed in {}",
                self.spelling,
                since.name(),
                removed.name()
            ),
            | FeatureOrigin::Gnu => write!(f, "'{}' is a GNU extension", self.spelling),
            | FeatureOrigin::Msvc(_) => write!(f, "'{}' is an MSVC extension", self.spelling),
        }
    }
}

impl std::error::Error for ExtensionDiagnostic<'_> {}

impl GetSeverity for ExtensionDiagnostic<'_> {
    fn severity(&self) -> ErrorSeverity {
        self.severity
    }
}

impl GetPosition for ExtensionDiagnostic<'_> {
    fn position(&self, context: &Context<'_>) -> SourcePosition {
        self.source_vectors.position(context)
    }
}

impl GetSourceVectors for ExtensionDiagnostic<'_> {
    fn source_vectors(&self, _context: &mut Context<'_>) -> SourceVectors {
        self.source_vectors
    }
}

impl ToDiagnostic for ExtensionDiagnostic<'_> {
    fn diagnostic_in<'d>(
        &self,
        context: &Context<'_>,
        source: SourceVectors,
        arena: &'d Bump,
    ) -> Diagnostic<'d> {
        let gcc = context.configuration.diagnostic_format()
            == crate::configuration::DiagnosticFormat::Gcc;
        let option = match self.gcc_promotion {
            | GccPromotion::Werror if gcc => "-Werror",
            | GccPromotion::WerrorPedantic if gcc => "-Werror=pedantic",
            | _ => "-Wpedantic",
        };
        Explanation::new(arena, format_in!(arena, "{self} [{option}]"))
            .option_suffix()
            .at(self.severity, source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::{
        CStandard,
        CompilerConfiguration,
        MsvcFeature,
    };
    #[test]
    fn shared_policy_covers_extension_warning_and_hard_error_baselines() {
        for (policy, extension, warning) in [
            (ExtensionPolicy::Allow, None, ErrorSeverity::Warning),
            (
                ExtensionPolicy::Warn,
                Some(ErrorSeverity::Warning),
                ErrorSeverity::Warning,
            ),
            (
                ExtensionPolicy::Deny,
                Some(ErrorSeverity::Error),
                ErrorSeverity::Error,
            ),
        ] {
            assert_eq!(
                policy_severity(policy, DiagnosticPolicy::Extension),
                extension
            );
            assert_eq!(
                policy_severity(policy, DiagnosticPolicy::Warning),
                Some(warning)
            );
            assert_eq!(
                policy_severity(policy, DiagnosticPolicy::WarningOnly),
                Some(ErrorSeverity::Warning)
            );
            assert_eq!(
                policy_severity(policy, DiagnosticPolicy::Error),
                Some(ErrorSeverity::Error)
            );
        }
    }

    #[test]
    fn shared_emitter_applies_policy_and_suppresses_native_standard_features() {
        for standard in [CStandard::C89, CStandard::C11] {
            for policy in [
                ExtensionPolicy::Allow,
                ExtensionPolicy::Warn,
                ExtensionPolicy::Deny,
            ] {
                let tu = Bump::new();
                let configuration = CompilerConfiguration::new(standard, policy)
                    .with_gnu_extensions(true)
                    .with_msvc_extensions(true);
                let mut context = Context::with_configuration(&tu, configuration);
                let file = context.add_synthetic_source_file(
                    std::path::Path::new("<extension>"),
                    "_Static_assert",
                );
                let source = context.create_source_vectors(
                    SourcePosition::default(),
                    file,
                    "_Static_assert".len(),
                );
                context.report_extension(Feature::StaticAssert, "_Static_assert", source);
                context.report_extension(Feature::ImplicitInt, "implicit int", source);
                context.report_extension(Feature::GnuAttribute, "__attribute__", source);
                context.report_extension_since(
                    "__declspec",
                    FeatureOrigin::Msvc(MsvcFeature::Declspec),
                    source,
                );
                let expected = if policy == ExtensionPolicy::Allow {
                    0
                } else {
                    2 + usize::from(standard < CStandard::C11)
                        + usize::from(standard >= CStandard::C99)
                };
                assert_eq!(context.pending_error_count(), expected);
                while let Some(error) = context.pop_pending_error() {
                    assert_eq!(
                        error.severity(),
                        if policy == ExtensionPolicy::Warn {
                            ErrorSeverity::Warning
                        } else {
                            ErrorSeverity::Error
                        }
                    );
                    let diagnostic = error.diagnostic_in(&context, source, &tu);
                    assert_ne!(diagnostic.message, "");
                }
            }
        }
    }
}
