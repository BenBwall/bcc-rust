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
impl Context<'_> {
    /// Shared policy emitter. GNU/MSVC extensions remain non-ISO even when
    /// enabled; standard features cease being extensions in their native mode.
    pub(crate) fn report_extension_since(
        &mut self,
        spelling: &str,
        origin: FeatureOrigin,
        source_vectors: SourceVectors,
    ) {
        if matches!(origin, FeatureOrigin::Standard(_))
            && self.configuration.origin_is_native(origin)
        {
            return;
        }
        let severity = match self.configuration.extension_policy() {
            | ExtensionPolicy::Allow => return,
            | ExtensionPolicy::Warn => ErrorSeverity::Warning,
            | ExtensionPolicy::Deny => ErrorSeverity::Error,
        };
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

    /// Feature-based adapter for lexer, preprocessor, and parser consumers.
    pub(crate) fn report_extension(
        &mut self,
        feature: Feature,
        spelling: &str,
        source: SourceVectors,
    ) {
        if self.configuration.is_native(feature)
            && (matches!(feature.origin(), FeatureOrigin::Standard(_))
                || feature == Feature::ImplicitInt)
        {
            return;
        }
        self.report_extension_since(spelling, feature.origin(), source);
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
                let source = context.push_source_vectors(&[]);
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
