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
};
use crate::{
    configuration::{
        ExtensionPolicy,
        Feature,
        FeatureOrigin,
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
        self.report_extension_since(spelling, feature.origin(), source);
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
        let Some(severity) =
            self.extension_severity(origin, DiagnosticPolicy::Extension, source_vectors)
        else {
            return;
        };
        self.emit_extension(spelling, origin, severity, source_vectors);
    }

    fn extension_severity(
        &self,
        origin: FeatureOrigin,
        baseline: DiagnosticPolicy,
        source: SourceVectors,
    ) -> Option<ErrorSeverity> {
        if matches!(
            origin,
            FeatureOrigin::Standard(_) | FeatureOrigin::Removed { .. }
        ) && self.configuration.origin_is_native(origin)
        {
            return None;
        }
        let severity = policy_severity(self.configuration.extension_policy(), baseline)?;
        (!self.withholds(
            severity,
            !matches!(baseline, DiagnosticPolicy::Error),
            source,
        ))
        .then_some(severity)
    }

    /// Queues the diagnostic once the policy has asked for one. Callers
    /// inline only the policy checks above.
    #[cold]
    #[inline(never)]
    fn emit_extension(
        &mut self,
        spelling: &str,
        origin: FeatureOrigin,
        severity: ErrorSeverity,
        source_vectors: SourceVectors,
    ) {
        let spelling = self.diagnostic_text(spelling);
        self.pending_errors
            .push_back(TranslationError::Extension(ExtensionDiagnostic {
                suppressed: self.tu_arena().alloc(Cell::new(false)),
                spelling,
                origin,
                severity,
                source_vectors,
            }));
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
        if let Some(severity) = self.extension_severity(feature.origin(), baseline, source) {
            self.pending_errors.push_back(diagnostic(severity));
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

    /// Withdraws the pending preprocessing diagnostic at `index`. An already
    /// suppressed placeholder takes its place, so every other pending
    /// diagnostic keeps its queue position, which the parser records while
    /// it runs.
    pub(crate) fn withdraw_pending_error(&mut self, index: usize) {
        let suppressed = self.tu_arena().alloc(Cell::new(false));
        let Some(slot) = self.pending_errors.iter_mut().nth(index) else {
            return;
        };
        debug_assert!(
            matches!(slot, TranslationError::Preprocessing(_)),
            "only preprocessing diagnostics are withdrawn"
        );
        *slot = TranslationError::Extension(ExtensionDiagnostic {
            suppressed,
            spelling: "",
            origin: FeatureOrigin::Gnu,
            severity: ErrorSeverity::Warning,
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
    pub(super) source_vectors: SourceVectors,
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
        _context: &Context<'_>,
        source: SourceVectors,
        arena: &'d Bump,
    ) -> Diagnostic<'d> {
        Explanation::new(arena, format_in!(arena, "{self}")).at(self.severity, source)
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
