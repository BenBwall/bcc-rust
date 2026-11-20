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
        ExtensionGroup,
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
            let reach = self.reach(feature.origin(), Some(feature), ExtensionOption::Pedantic);
            self.report_extension_in(spelling, feature.origin(), source, reach);
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
            let reach = self.reach(feature.origin(), Some(feature), ExtensionOption::Everything);
            self.report_extension_in(spelling, feature.origin(), source, reach);
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
        let reach = self.reach(origin, None, ExtensionOption::Pedantic);
        self.report_extension_in(spelling, origin, source_vectors, reach);
    }

    /// [`Self::report_extension_since`] for syntax GCC files under an option
    /// of its own that `-Wpedantic` does not classify, which its tag names in
    /// place of `-Wpedantic`, as GCC 14 files a variable length array before
    /// C99 under `-Wvla` (`pedwarn_c90` in `c-errors.cc`).
    pub(crate) fn report_extension_under_gcc_option(
        &mut self,
        spelling: &str,
        origin: FeatureOrigin,
        source_vectors: SourceVectors,
        group: ExtensionGroup,
    ) {
        let mut reach = self.reach(origin, None, ExtensionOption::Pedantic);
        reach.gcc = GccFiling::Unclassified;
        if self.configuration.compiler_identity().rules() == RuleSet::Gcc {
            reach.group = group;
        }
        self.report_extension_in(spelling, origin, source_vectors, reach);
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
        let reach = ExtensionReach {
            gcc: GccFiling::Pedantic,
            ..self.reach(origin, None, ExtensionOption::Pedantic)
        };
        self.report_extension_in(spelling, origin, source_vectors, reach);
    }

    /// [`Self::report_extension_since`], for a diagnostic the options reach
    /// as `reach` says.
    fn report_extension_in(
        &mut self,
        spelling: &str,
        origin: FeatureOrigin,
        source_vectors: SourceVectors,
        reach: ExtensionReach,
    ) {
        let Some(severity) =
            self.extension_severity(origin, DiagnosticPolicy::Extension, source_vectors, reach)
        else {
            return;
        };
        self.emit_extension(spelling, origin, severity, source_vectors, reach);
    }

    /// How the warning options reach an extension diagnostic of syntax from
    /// `origin`, of `feature` where the caller names it, which a pragma
    /// names by `option`.
    fn reach(
        &self,
        origin: FeatureOrigin,
        feature: Option<Feature>,
        option: ExtensionOption,
    ) -> ExtensionReach {
        let gcc = match feature.and_then(Feature::gcc_filing) {
            | Some(filing) => filing,
            | None if self.parsing_started => GccFiling::Pedantic,
            | None => GccFiling::Preprocessor,
        };
        let clang = ExtensionClass::of(origin, feature, option == ExtensionOption::Everything);
        let group = match self.configuration.compiler_identity().rules() {
            | RuleSet::Clang => match feature.and_then(Feature::clang_group) {
                | Some(group) => group,
                | None => clang.clang_group(),
            },
            | RuleSet::Gcc => match feature.and_then(Feature::gcc_group) {
                | Some(group) => group,
                | None => ExtensionGroup::Pedantic,
            },
        };
        ExtensionReach {
            option,
            clang,
            gcc,
            group,
            feature,
        }
    }

    /// What the settings at `source` make of an extension diagnostic the
    /// options reach as `reach` says, if they report it.
    fn extension_severity(
        &self,
        origin: FeatureOrigin,
        baseline: DiagnosticPolicy,
        source: SourceVectors,
        reach: ExtensionReach,
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
        let option = reach.option;
        let severity = if self.command_line_classifies(reach, source) {
            self.command_line_extension_severity(baseline, reach)?
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
    /// diagnostic the options reach as `reach` says, where they classify it
    /// ([`Self::command_line_classifies`]): Clang's mapping of it
    /// ([`CompilerConfiguration::clang_extension_severity`]), or GCC's
    /// classification of a pedantic warning
    /// ([`CompilerConfiguration::gcc_pedantic_severity`]). The severity is
    /// final: `-w` and `-Werror` have had their say.
    /// C99: §5.1.1.3 paragraph 1, p. 11; PDF p. 23.
    ///
    /// [`CompilerConfiguration::clang_extension_severity`]: crate::configuration::CompilerConfiguration::clang_extension_severity
    /// [`CompilerConfiguration::gcc_pedantic_severity`]: crate::configuration::CompilerConfiguration::gcc_pedantic_severity
    fn command_line_extension_severity(
        &self,
        baseline: DiagnosticPolicy,
        reach: ExtensionReach,
    ) -> Option<ErrorSeverity> {
        let configuration = self.configuration;
        if configuration.compiler_identity().rules() == RuleSet::Clang {
            return configuration.clang_extension_severity(reach.clang);
        }
        // GCC files what Clang warns of by default under `-Wpedantic` too,
        // which enables it.
        let baseline = match reach.option {
            | ExtensionOption::Everything => DiagnosticPolicy::Extension,
            | _ => baseline,
        };
        let enabled = policy_severity(configuration.extension_policy(), baseline).is_some();
        configuration.gcc_pedantic_severity(enabled, reach.gcc)
    }

    /// Whether the command line alone classifies an extension diagnostic the
    /// options reach as `reach` says at `source`: where no diagnostic pragma
    /// governs it, under Clang's rules or GCC's. What GCC's preprocessor
    /// reports ([`GccFiling::Preprocessor`]) stays the extension policy's.
    fn command_line_classifies(&self, reach: ExtensionReach, source: SourceVectors) -> bool {
        reach.option != ExtensionOption::None
            && (self.configuration.compiler_identity().rules() == RuleSet::Clang
                || (self.configuration.compiler_identity() == CompilerIdentity::Gcc
                    && reach.gcc != GccFiling::Preprocessor))
            && !self.governed_by_pragma(source)
    }

    /// Queues an extension diagnostic the options reach as `reach` says: one
    /// whose severity the command line settled is final
    /// ([`Self::command_line_classifies`]); a pragma classifies the others.
    fn queue_extension(
        &mut self,
        source: SourceVectors,
        error: TranslationError<'tu>,
        severity: ErrorSeverity,
        reach: ExtensionReach,
    ) {
        if self.command_line_classifies(reach, source) {
            self.push_final_extension(error, severity);
        } else {
            self.classified_extension(source, error, reach.option);
        }
    }

    /// Whether GCC's `-Werror`, or its `-Werror=pedantic`, made the extension
    /// diagnostic at `source` the error `severity` says, which its format
    /// names, unlike one `-pedantic-errors` made
    /// (`print_option_information` in `diagnostic.cc`).
    fn gcc_promoted(
        &self,
        severity: ErrorSeverity,
        reach: ExtensionReach,
        source: SourceVectors,
    ) -> bool {
        severity == ErrorSeverity::Error
            && self.configuration.compiler_identity() == CompilerIdentity::Gcc
            && !self.configuration.pedantic_errors()
            && self.command_line_classifies(reach, source)
    }

    /// Queues an extension diagnostic at the `severity` the command line's
    /// options settled, which the reporter must not change: `-Werror` has had
    /// its say, and `-Wno-error=` may have kept it a warning. Only where the
    /// reporter would give it another severity does it carry this one
    /// ([`TranslationError::configured_severity`]). `-Wfatal-errors` still
    /// makes such an error fatal. The diagnostic keeps its own severity for
    /// its words, which may name the `-Werror` that promoted it.
    fn push_final_extension(&mut self, error: TranslationError<'tu>, severity: ErrorSeverity) {
        if error.configured_severity(self) == severity {
            self.pending_errors.push_back(error);
        } else {
            self.push_reclassified_extension(error, severity);
        }
    }

    #[cold]
    #[inline(never)]
    fn push_reclassified_extension(
        &mut self,
        mut error: TranslationError<'tu>,
        severity: ErrorSeverity,
    ) {
        if let TranslationError::Parsing(parsing) = &mut error {
            parsing.severity = severity;
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

    /// Queues the diagnostic the options reach as `reach` says once the policy
    /// has asked for one. Callers inline only the policy checks above.
    #[cold]
    #[inline(never)]
    fn emit_extension(
        &mut self,
        spelling: &str,
        origin: FeatureOrigin,
        severity: ErrorSeverity,
        source_vectors: SourceVectors,
        reach: ExtensionReach,
    ) {
        let spelling = self.diagnostic_text(spelling);
        let at = source_vectors;
        let source_vectors = self.file_location(source_vectors);
        let suppressed = self.tu_arena().alloc(Cell::new(false));
        let gcc_promotion = match reach.gcc {
            | _ if !self.gcc_promoted(severity, reach, at) => GccPromotion::None,
            | GccFiling::Unclassified if reach.group == ExtensionGroup::Pedantic =>
                GccPromotion::Werror,
            | _ => GccPromotion::WerrorGroup,
        };
        self.queue_extension(
            at,
            TranslationError::Extension(ExtensionDiagnostic {
                suppressed,
                spelling,
                origin,
                severity,
                group: reach.group,
                feature: reach.feature,
                gcc_promotion,
                source_vectors,
            }),
            severity,
            reach,
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
        let reach = self.reach(feature.origin(), Some(feature), option);
        if let Some(severity) = self.extension_severity(feature.origin(), baseline, source, reach) {
            let own = if self.gcc_promoted(severity, reach, source) {
                ErrorSeverity::Warning
            } else {
                severity
            };
            self.queue_extension(source, diagnostic(own), severity, reach);
        }
    }

    /// Whether semantic analysis reports a parameter without a name in a
    /// function definition at `source`, in its own words, rather than the
    /// parser as a C23 extension: where a diagnostic pragma governs it (or
    /// the identity's rules are neither Clang's nor GCC's), the extension
    /// policy leaves the extension unreported, and no pragma naming Clang's
    /// `-Weverything` reaches it. Where the command line alone classifies it
    /// ([`Self::command_line_classifies`]), the parser reports it as its
    /// compiler does, in order with the other C23 extensions.
    /// C99: §6.9.1 paragraph 5, p. 141; PDF p. 153.
    pub(crate) fn unnamed_parameter_left_to_analysis(&self, source: SourceVectors) -> bool {
        let reach = ExtensionReach {
            option:  ExtensionOption::Everything,
            clang:   ExtensionClass::C23ByDefault,
            gcc:     GccFiling::Pedantic,
            group:   ExtensionGroup::C23Extensions,
            feature: Some(Feature::UnnamedDefinitionParameters),
        };
        !self.command_line_classifies(reach, source)
            && self.extension_policy_at(reach.option, source) == ExtensionPolicy::Allow
            && self
                .everything_class_outside_pedantic(reach.option, source)
                .is_none()
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
            group: ExtensionGroup::Pedantic,
            feature: None,
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

/// How the warning options reach an extension diagnostic: by the option a
/// pragma names it by, the class Clang's command-line options map, and where
/// GCC files it; and the group its tag names under the identity's rules.
#[derive(Clone, Copy)]
struct ExtensionReach {
    option:  ExtensionOption,
    clang:   ExtensionClass,
    gcc:     GccFiling,
    group:   ExtensionGroup,
    feature: Option<Feature>,
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
    /// The group the tag names.
    group:                     ExtensionGroup,
    /// The feature, where the reporter named it.
    feature:                   Option<Feature>,
    gcc_promotion:             GccPromotion,
    pub(super) source_vectors: SourceVectors,
}

/// What GCC's format names in place of the group after an extension
/// diagnostic that its `-Werror` or `-Werror=pedantic` made an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GccPromotion {
    None,
    /// `-Werror`, for a warning GCC files under no option.
    Werror,
    /// `-Werror=` and the group.
    WerrorGroup,
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
        use crate::configuration::DiagnosticFormat as F;
        // GCC names the option that made the warning an error; Clang names
        // `-Werror` before the group on every extension diagnostic that is
        // an error (`printDiagnosticOptions` in `TextDiagnosticPrinter.cpp`).
        let format = context.configuration.diagnostic_format();
        let group = self.group.name();
        let option = match (format, self.gcc_promotion) {
            | (F::Gcc, GccPromotion::Werror) => "-Werror",
            | (F::Gcc, GccPromotion::WerrorGroup) => format_in!(arena, "-Werror={group}"),
            | (F::Clang, _) if self.severity == ErrorSeverity::Error =>
                format_in!(arena, "-Werror,-W{group}"),
            | _ => format_in!(arena, "-W{group}"),
        };
        let explanation =
            Explanation::new(arena, format_in!(arena, "{self} [{option}]")).option_suffix();
        // bcc's own format keeps the label and note semantic analysis gives
        // the same requirement where the parser leaves it to analysis.
        // C99: §6.9.1 paragraph 5, p. 141; PDF p. 153.
        let explanation = match (format, self.feature) {
            | (F::Bcc, Some(Feature::UnnamedDefinitionParameters)) => {
                let kind = super::semantic_analysis::SemanticErrorKind::UnnamedDefinitionParameter;
                explanation.label(kind.label()).note(kind.explanation().1)
            },
            | _ => explanation,
        };
        explanation.at(self.severity, source)
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
